<script lang="ts">
  import { Badge, Button, Callout, Skeleton, StencilHeading, Switch } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type CompatChangesView, type DetectedConflict, type ModPatchStatus, type PatchState } from "../../../lib/bindings";
  import { changedBy, changedMods, idleLines, ignoredLines } from "../../../lib/patches/compat";
  import { explainConflict, isNote, isRecordless } from "../../../lib/patches/explain";
  import { foldConflicts } from "../../../lib/patches/groups";
  import FailureGroupRow from "../patches/FailureGroupRow.svelte";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { mods } from "../../../lib/stores/mods.svelte";
  import { patches } from "../../../lib/stores/patches.svelte";
  import { openPatch } from "../mods-view.svelte";
  import { resolver } from "../../../lib/stores/resolver.svelte";

  interface Props {
    modId: string;
    canMutate: boolean;
  }

  let { modId, canMutate }: Props = $props();

  const STATE: Record<PatchState, { label: string; tone: "ok" | "warn" | "danger" | "neutral" }> = {
    applied: { label: "Applied", tone: "ok" },
    broken: { label: "Broken", tone: "danger" },
    disabled: { label: "Disabled", tone: "neutral" },
    skipped: { label: "Skipped", tone: "neutral" },
    not_run: { label: "Does not run in the game", tone: "warn" },
  };

  const entry = $derived(patches.get(modId));
  const list = $derived(entry.status === "ready" ? entry.data : []);
  const named = $derived.by(() => {
    const byId = new Map<string, DetectedConflict>();
    for (const p of list) for (const c of p.conflicts) byId.set(c.id, c);
    return [...byId.values()];
  });
  /** Info notes (`multi_match`) are not conflicts: shown as notes, with nothing to resolve. */
  const conflicts = $derived(named.filter((c) => !isNote(c)));
  /** Failures caused by another patch fold under it (G10). */
  const rows = $derived(
    foldConflicts(conflicts, loadOrder.simulation.status === "ready" ? (loadOrder.simulation.data.failure_groups ?? []) : []),
  );
  const notes = $derived(named.filter(isNote));
  /** This mod's conflicts that no patch row has (a module, a `[vars]` name). */
  const modConflicts = $derived(
    loadOrder.simulation.status === "ready"
      ? loadOrder.simulation.data.conflicts.filter(
          (c) => isRecordless(c) && (c.mod_a_id === modId || c.mod_b_id === modId) && !named.some((n) => n.id === c.id),
        )
      : [],
  );

  /** Toggles in flight; dropped when fresh patch data arrives or the write fails. */
  let overrides = $state<Record<string, boolean>>({});
  /** Patch ids whose toggle is still being written. */
  let pending = $state<Set<string>>(new Set());

  $effect(() => {
    void entry;
    overrides = {};
  });

  /** What this mod's compatibility overrides did, and its rungs that are never used (§11.6). */
  let compat = $state<CompatChangesView | null>(null);
  let compatSeq = 0;
  async function loadCompat(id: string) {
    const mine = ++compatSeq;
    try {
      const r = await call(commands.compatChangesGet(id), { expect: ["not_found"] });
      if (mine === compatSeq) compat = r.status === "ready" ? r.data : null;
    } catch {
      if (mine === compatSeq) compat = null;
    }
  }

  // Refetched with the patch rows: both follow the load order and the mod set.
  $effect(() => {
    void entry;
    void loadCompat(modId);
  });

  const modNames = $derived(Object.fromEntries((mods.list.status === "ready" ? mods.list.data : []).map((m) => [m.id, m.name])));
  const modName = (id: string) => modNames[id] ?? id;
  const simOverrides = $derived(loadOrder.simulation.status === "ready" ? (loadOrder.simulation.data.overrides ?? []) : []);
  const changes = $derived(compat ? changedMods(compat) : []);
  const ignored = $derived(compat ? ignoredLines(compat) : []);
  const idle = $derived(compat ? idleLines(compat) : []);

  const nameOf = (p: ModPatchStatus) => p.name ?? p.patch_id;

  async function toggle(p: ModPatchStatus, enabled: boolean) {
    const id = p.patch_id;
    overrides = { ...overrides, [id]: enabled };
    pending = new Set([...pending, id]);
    try {
      await call(commands.patchSetEnabled(id, enabled));
    } catch {
      const next = { ...overrides };
      delete next[id];
      overrides = next;
    } finally {
      const next = new Set(pending);
      next.delete(id);
      pending = next;
    }
  }
</script>

<div class="patches">
  {#if ignored.length > 0}
    <Callout tone="warn" title="QuarterMaster ignores some of this mod's changes.">
      <ul class="lines" aria-label="Ignored changes">
        {#each ignored as line, i (`${line}:${i}`)}<li>{line}</li>{/each}
      </ul>
    </Callout>
  {/if}
  {#if entry.status === "loading"}
    <div aria-busy="true" aria-label="Loading patches"><Skeleton lines={3} /></div>
  {:else if entry.status === "error"}
    <div class="error" role="alert">
      <p>{entry.message}</p>
      <Button size="sm" onclick={() => patches.load(modId).catch(() => {})}>Retry</Button>
    </div>
  {:else if list.length === 0}
    <p class="qm-muted">This mod has no patches.</p>
  {:else}
    <ul class="rows">
      {#each list as p (p.patch_id)}
        <li class="row">
          <div class="main">
            <span class="name">{nameOf(p)}</span>
            <code class="file">{p.target_file}</code>
          </div>
          <Badge tone={STATE[p.state].tone}>{STATE[p.state].label}</Badge>
          {#if fixes.forPatch(p.patch_id, modId).length > 0}<Badge tone="accent">Fixed by you</Badge>{/if}
          {#each changedBy(simOverrides, modId, p.patch_id, p) as m (m)}<Badge tone="info">Changed by {modName(m)}</Badge>{/each}
          {#if p.target_file}
            <Button size="sm" variant="ghost" aria-label={`Show ${nameOf(p)} in patch view`} onclick={() => openPatch(p.patch_id)}>Show</Button>
          {/if}
          <Switch
            checked={overrides[p.patch_id] ?? p.state !== "disabled"}
            label={`Enable ${nameOf(p)}`}
            hideLabel
            disabled={!canMutate || pending.has(p.patch_id)}
            onCheckedChange={(v: boolean) => toggle(p, v)}
          />
        </li>
      {/each}
    </ul>
  {/if}

  {#if changes.length > 0 || idle.length > 0}
    <section class="compat" aria-label="Changes to other mods">
      <StencilHeading level={3}>{changes.length > 0 ? `Changes patches of: ${changes.map(modName).join(", ")}` : "Changes patches of other mods"}</StencilHeading>
      {#if compat && compat.applied.length > 0}
        <ul class="lines" aria-label="Changes it makes">
          {#each compat.applied as o, i (`${o.target}:${i}`)}<li>{o.detail}.</li>{/each}
        </ul>
      {/if}
      {#if idle.length > 0}
        <ul class="lines qm-muted" aria-label="Changes not in use now">
          {#each idle as line, i (`${line}:${i}`)}<li>{line}</li>{/each}
        </ul>
      {/if}
    </section>
  {/if}

  {#if conflicts.length > 0 || modConflicts.length > 0}
    <section class="conflicts">
      <StencilHeading level={3}>Conflicts</StencilHeading>
      {#each rows as r (r.kind === "one" ? r.conflict.id : `group:${r.group.target_file}:${r.group.root}`)}
        {#if r.kind === "one"}
          {@const c = r.conflict}
          <div class="conflict">
            <p>{c.description}</p>
            <code class="file">{c.target_file}</code>
            <!-- Every fix goes through the resolver, which previews its impact before Apply. -->
            <div class="actions">
              <Button size="sm" variant="primary" onclick={() => resolver.show(c.id)}>Resolve...</Button>
            </div>
          </div>
        {:else}
          <div class="conflict">
            <FailureGroupRow row={r} {modName} onresolve={(id) => resolver.show(id)} />
            <code class="file">{r.group.target_file}</code>
          </div>
        {/if}
      {/each}
      {#each modConflicts as c (c.id)}
        <div class="conflict" data-testid="mod-conflict">
          <p>{explainConflict(c, { records: [], modName })}</p>
          {#if c.target_file}<code class="file">{c.target_file}</code>{/if}
        </div>
      {/each}
    </section>
  {/if}

  {#if notes.length > 0}
    <section class="conflicts" aria-label="Notes">
      <StencilHeading level={3}>Notes</StencilHeading>
      {#each notes as c (c.id)}
        <div class="conflict">
          <p>{c.description}</p>
          <code class="file">{c.target_file}</code>
        </div>
      {/each}
    </section>
  {/if}
</div>

<style>
  .patches {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-4);
  }

  p {
    margin: 0;
  }

  .error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-2);
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
  }

  .main {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    overflow-wrap: anywhere;
  }

  .file {
    color: var(--qm-text-muted);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-11);
    overflow-wrap: anywhere;
  }

  .lines {
    margin: 0;
    padding-left: var(--qm-space-4);
  }

  .compat {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .conflicts {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .conflict {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-1);
  }
</style>
