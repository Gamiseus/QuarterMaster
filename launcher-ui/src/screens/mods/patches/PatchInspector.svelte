<script lang="ts">
  import { Badge, Button, Inset, KeyValueList, Section } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type PatchDetail, type PatchExecutionRecord } from "../../../lib/bindings";
  import { dependentsToDisable, requirementsToEnable } from "../../../lib/patches/cascade";
  import { isNote, recordNotes, runsAsValue } from "../../../lib/patches/explain";
  import { changedBy, modOfName, namesPatch, tiedTo } from "../../../lib/patches/compat";
  import { badgeOf } from "../../../lib/patches/fixlist";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { patchView } from "../../../lib/stores/patchview.svelte";
  import { resolver } from "../../../lib/stores/resolver.svelte";
  import CascadeDialog from "./CascadeDialog.svelte";
  import LuaCode from "./LuaCode.svelte";

  interface Props {
    detail: PatchDetail;
    modNames: Record<string, string>;
    canMutate: boolean;
    /** Selects another patch (a Requires or Required by link). */
    onselect: (patchId: string) => void;
    /** Why stacking put this patch's code where it is at shared lines in the shown file. */
    stackNotes?: string[];
    /** The patch's record in the shown file, when it touches that file. */
    record?: PatchExecutionRecord | null;
  }

  let { detail, modNames, canMutate, onselect, stackNotes = [], record = null }: Props = $props();

  const POSITION = { at: "Replaces the matched line", before: "Before the matched line", after: "After the matched line" };
  const ON_FAILURE = { warn: "Warn and go on", skip_patch: "Skip this patch", skip_mod: "Skip the whole mod", abort: "Stop loading" };

  const title = $derived(detail.name ?? detail.patch_id);
  const named = $derived(
    loadOrder.simulation.status === "ready"
      ? loadOrder.simulation.data.conflicts.filter((c) => c.patch_a_id === detail.patch_id || c.patch_b_id === detail.patch_id)
      : [],
  );
  /** Info notes (`multi_match`) are not conflicts: shown as notes, with nothing to resolve. */
  const conflicts = $derived(named.filter((c) => !isNote(c)));
  const notes = $derived(named.filter(isNote));
  /** The profile's fixes that change or name this patch. */
  const myFixes = $derived(fixes.forPatch(detail.patch_id, detail.mod_id));
  /** Compatibility mods that change this patch (§11.6). */
  const overrides = $derived(loadOrder.simulation.status === "ready" ? (loadOrder.simulation.data.overrides ?? []) : []);
  const compatMods = $derived(changedBy(overrides, detail.mod_id, detail.patch_id, record));
  /** What each compatibility mod's override does to this patch, when the simulation says. */
  const compatChanges = $derived(overrides.filter((o) => o.status === "applied" && namesPatch(o.target, detail.mod_id, detail.patch_id)));
  /** The other patches whose change this patch's code sits in or next to. */
  const tied = $derived(tiedTo(record));
  const tiedName = (within: string) => {
    const m = modOfName(within);
    return m ? (modNames[m] ?? m) : within;
  };
  const def = $derived(detail.definition);
  const kind = $derived("pattern" in def ? "Pattern" : "regex" in def ? "Regex" : "copy" in def ? "Copy" : "Module");
  const body = $derived("pattern" in def ? def.pattern : "regex" in def ? def.regex : null);
  const target = $derived.by(() => {
    const t = "pattern" in def ? def.pattern.target : "regex" in def ? def.regex.target : "copy" in def ? def.copy.target : def.module.before;
    return Array.isArray(t) ? t.join(", ") : (t ?? "");
  });
  /** How an `at` patch runs: one replacement or separate edits (§6.6). */
  const runsAs = $derived(runsAsValue(detail));
  /** Why it ran as it did in the shown file: a patch it needs, edits it skipped (D49, D64). */
  const runNotes = $derived(recordNotes(record));
  const facts = $derived([
    { label: "Mod", value: modNames[detail.mod_id] ?? detail.mod_id },
    { label: "Type", value: kind },
    { label: "Target", value: target, mono: true },
    ...(body ? [{ label: "Position", value: POSITION[body.position] }] : []),
    ...(runsAs ? [{ label: "Runs as", value: runsAs }] : []),
    ...(body ? [{ label: "Times", value: body.times == null ? "Every match" : String(body.times) }] : []),
    { label: "On failure", value: ON_FAILURE[(body?.on_failure ?? "warn") as keyof typeof ON_FAILURE] ?? "Warn and go on" },
    { label: "Priority", value: String(detail.priority) },
    { label: "Source", value: detail.source_file, mono: true },
    { label: "Id", value: detail.patch_id, mono: true, copy: true },
  ]);

  let dialog = $state<null | { kind: "disable" | "enable"; others: PatchDetail[] }>(null);
  let dialogOpen = $state(false);
  let busy = $state(false);

  const fetch = (id: string) => call(commands.patchDetail(id), { expect: ["not_found"] }).catch(() => undefined);
  const nameOf = (d: PatchDetail) => d.name ?? d.patch_id;
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

  async function write(ids: string[], enabled: boolean) {
    await call(commands.patchSetEnabledMany(ids, enabled));
  }

  async function toggle() {
    busy = true;
    try {
      const disabling = !detail.disabled;
      const others = disabling
        ? await dependentsToDisable(detail.patch_id, fetch)
        : await requirementsToEnable(detail.patch_id, fetch);
      if (others.length === 0) {
        await write([detail.patch_id], !disabling).catch(() => {});
      } else {
        dialog = { kind: disabling ? "disable" : "enable", others };
        dialogOpen = true;
      }
    } finally {
      busy = false;
    }
  }

  const all = () => [detail.patch_id, ...(dialog?.others ?? []).map((d) => d.patch_id)];
</script>

<div class="inspector">
  <header class="head">
    <h3 class="title">{title}</h3>
    {#if detail.disabled}<Badge>Disabled for this profile</Badge>{/if}
    {#if detail.replaces_patch_id}<Badge tone="info">Adjusted copy</Badge>{/if}
    {#if myFixes.length > 0}<Badge tone="accent">Fixed by you</Badge>{/if}
    {#each compatMods as m (m)}<Badge tone="info">Changed by {modNames[m] ?? m}</Badge>{/each}
    {#each [...new Set(tied.map(tiedName))] as b (b)}<Badge>Tied to {b}</Badge>{/each}
  </header>

  <KeyValueList label="Patch facts" items={facts} />

  {#if conflicts.length > 0}
    <Section title="Conflicts">
      <ul class="conflicts">
        {#each conflicts as c (c.id)}
          <li>
            <span>{c.description}</span>
            <Button size="sm" onclick={() => resolver.show(c.id)}>Resolve...</Button>
          </li>
        {/each}
      </ul>
    </Section>
  {/if}

  {#if notes.length > 0}
    <Section title="Notes">
      <ul class="conflicts" aria-label="Notes on this patch">
        {#each notes as c (c.id)}
          <li><span>{c.description}</span></li>
        {/each}
      </ul>
    </Section>
  {/if}

  {#if runNotes.length > 0}
    <Section title="In this file">
      <ul class="order" aria-label="How it ran in this file">
        {#each runNotes as note, i (`${note}:${i}`)}<li>{note}</li>{/each}
      </ul>
    </Section>
  {/if}

  {#if myFixes.length > 0}
    <Section title="Fixes">
      <ul class="conflicts" aria-label="Fixes on this patch">
        {#each myFixes as f (f.id)}
          {@const badge = badgeOf(f)}
          <li>
            <span><Badge tone={badge.tone}>{badge.text}</Badge> {f.label}</span>
          </li>
        {/each}
      </ul>
      <div class="actions">
        <Button size="sm" onclick={() => (patchView.view = "fixes")}>See in Fixes</Button>
      </div>
    </Section>
  {/if}

  {#if compatChanges.length > 0}
    <Section title="Changed by other mods">
      <ul class="order" aria-label="Changes by compatibility patches">
        {#each compatChanges as o, i (`${o.from_mod}:${i}`)}<li>{o.detail}</li>{/each}
      </ul>
    </Section>
  {/if}

  {#if body}
    <Section title="Anchor">
      <Inset><LuaCode code={body.pattern} label="Anchor" /></Inset>
    </Section>
  {/if}
  {#if body?.payload}
    <Section title="Payload">
      <Inset><LuaCode code={body.payload} label="Payload" /></Inset>
    </Section>
  {/if}

  <Section title="Dependencies">
    {#each [["Requires", detail.requires], ["Required by", detail.required_by]] as const as [label, ids] (label)}
      <div class="deps">
        <span class="qm-muted">{label}</span>
        {#if ids.length === 0}
          <span>None</span>
        {:else}
          <ul aria-label={label}>
            {#each ids as id (id)}
              <li><button type="button" class="link" onclick={() => onselect(id)}>{id}</button></li>
            {/each}
          </ul>
        {/if}
      </div>
    {/each}
  </Section>

  <Section title="Order">
    <ul class="order">
      {#each detail.positions as p (p.target_file)}
        <li>{p.index + 1} of {p.count} in {p.target_file}</li>
      {/each}
      {#if detail.declared_before.length > 0}<li>Runs before {detail.declared_before.join(", ")} (set by the mod)</li>{/if}
      {#if detail.declared_after.length > 0}<li>Runs after {detail.declared_after.join(", ")} (set by the mod)</li>{/if}
      {#each detail.profile_rules as r, i (i)}
        <li>Moved by you: {[...(r.before ?? []).map((b) => `before ${b}`), ...(r.after ?? []).map((a) => `after ${a}`)].join(", ")}</li>
      {/each}
    </ul>
  </Section>

  {#if stackNotes.length > 0}
    <Section title="Shared lines">
      <ul class="order" aria-label="Shared lines">
        {#each stackNotes as note, i (`${note}:${i}`)}<li>{note}</li>{/each}
      </ul>
    </Section>
  {/if}

  <div class="actions">
    <Button variant={detail.disabled ? "primary" : "secondary"} disabled={!canMutate || busy} onclick={toggle}>
      {detail.disabled ? "Enable" : "Disable for this profile"}
    </Button>
  </div>
</div>

{#if dialog?.kind === "disable"}
  <CascadeDialog
    bind:open={dialogOpen}
    title={`Disable "${title}"?`}
    message={`Disabling "${title}" also stops ${plural(dialog.others.length, "patch that needs", "patches that need")} it:`}
    items={dialog.others.map(nameOf)}
    confirmLabel={`Disable all ${dialog.others.length + 1}`}
    danger
    onconfirm={() => write(all(), false)}
  />
{:else if dialog?.kind === "enable"}
  <CascadeDialog
    bind:open={dialogOpen}
    title={`Enable "${title}"?`}
    message={`"${title}" needs ${plural(dialog.others.length, "patch that is", "patches that are")} disabled:`}
    items={dialog.others.map(nameOf)}
    confirmLabel={`Enable ${plural(dialog.others.length, "required patch", "required patches")} too`}
    alternateLabel="Enable only this patch"
    onconfirm={() => write(all(), true)}
    onalternate={() => write([detail.patch_id], true)}
  />
{/if}

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .title {
    margin: 0;
    font-size: var(--qm-text-16);
  }

  .deps {
    display: grid;
    grid-template-columns: 11ch 1fr;
    gap: var(--qm-space-2);
    padding: var(--qm-space-1) 0;
  }

  .deps ul,
  .order {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--qm-accent-text);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    text-decoration: underline;
    cursor: pointer;
  }

  .link:focus-visible {
    outline: none;
    box-shadow: var(--qm-focus-ring);
  }

  .conflicts {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .conflicts li {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-1);
  }

  .actions {
    display: flex;
    gap: var(--qm-space-2);
  }
</style>
