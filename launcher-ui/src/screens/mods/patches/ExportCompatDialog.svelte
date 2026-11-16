<script lang="ts">
  import { untrack } from "svelte";
  import { Badge, Button, Callout, Dialog, KeyValueList, Section, Skeleton, TextField } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type CompatExportPreview } from "../../../lib/bindings";
  import { equivalenceBadge, requirementText } from "../../../lib/patches/compat";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { toasts } from "../../../lib/stores/toasts.svelte";

  interface Props {
    open?: boolean;
    /** The fixes to save, in any order. */
    fixIds: string[];
    canMutate: boolean;
  }

  let { open = $bindable(false), fixIds, canMutate }: Props = $props();

  const MAX_NAME = 64;

  let name = $state("");
  let preview = $state<{ status: "loading" } | { status: "ready"; data: CompatExportPreview } | { status: "error"; message: string }>({ status: "loading" });
  let busy = $state(false);
  /** A rename is being checked; the last answer stays shown and Save stays on. */
  let checking = $state(false);
  let seq = 0;

  async function load(ids: string[], wanted: string, keep = false) {
    const mine = ++seq;
    // Untracked: the opening effect calls this and must not rerun on these.
    const shown = keep && untrack(() => preview.status) === "ready";
    checking = shown;
    if (!shown) preview = { status: "loading" };
    try {
      const data = await call(commands.compatExportPreview({ fix_ids: ids, name: wanted }), { expect: ["invalid_input", "conflict", "not_found"] });
      if (mine !== seq) return;
      preview = { status: "ready", data };
      // The first preview names the mod; the user can change it.
      if (name.trim() === "") name = data.name;
    } catch (e) {
      if (mine === seq) preview = { status: "error", message: e instanceof Error ? e.message : String(e) };
    } finally {
      if (mine === seq) checking = false;
    }
  }

  // Each opening checks again: the profile may have changed since.
  $effect(() => {
    if (!open) return;
    name = "";
    void load(fixIds, "");
  });

  /**
   * A changed name shows in the files; the check runs again with it. The name changes neither
   * the id nor the verdict, so Save stays on meanwhile and saves with the new name.
   */
  function renamed() {
    if (preview.status === "ready" && name.trim() !== preview.data.name) void load(fixIds, name.trim(), true);
  }

  const data = $derived(preview.status === "ready" ? preview.data : null);
  const badge = $derived(data ? equivalenceBadge(data.equivalence) : null);
  const canSave = $derived(
    canMutate && !busy && data !== null && data.equivalence.kind === "equivalent" && data.refused.length === 0 && data.included.length > 0,
  );
  const label = (id: string) => fixes.list.find((f) => f.id === id)?.label ?? id;

  async function save() {
    if (!canSave) return;
    busy = true;
    try {
      const done = await call(commands.compatExport({ fix_ids: fixIds, name: name.trim() }));
      open = false;
      await fixes.load().catch(() => {});
      const step = journal().filter((j) => j.op === "compat_export").at(-1);
      const message = `Saved to your mod library. It replaces ${done.removed_fix_ids.length === 1 ? "this fix" : "these fixes"} in this profile.`;
      if (step) toasts.notice(message, [{ label: "Undo", run: () => void undo(step.at) }]);
      else toasts.info(message);
    } catch {
      // call() showed the error; the dialog stays open.
    } finally {
      busy = false;
    }
  }

  const journal = () => fixes.journal;

  /** Asks to undo the export saved at `at`: the dialog previews it first (D71). */
  async function undo(at: string) {
    try {
      await fixes.askJournalUndo(at);
    } catch {
      // call() showed the error.
    }
  }
</script>

<Dialog
  bind:open
  title="Save as compatibility patch"
  description="Makes a small mod from your fixes that you can share. QuarterMaster installs it and turns it on in this profile in place of the fixes."
  size="lg"
  dismissible={!busy}
>
  <div class="export" aria-live="polite">
    <TextField label="Name" bind:value={name} maxlength={MAX_NAME} disabled={busy} onchange={renamed} />
    {#if checking}<p class="checking" aria-busy="true">Checking the new name...</p>{/if}
    {#if preview.status === "loading"}
      <div aria-busy="true" aria-label="Checking the compatibility patch"><Skeleton lines={4} /></div>
    {:else if preview.status === "error"}
      <Callout tone="danger" title="Could not check the compatibility patch.">
        {preview.message}
        {#snippet action()}<Button size="sm" onclick={() => load(fixIds, name.trim())}>Retry</Button>{/snippet}
      </Callout>
    {:else if data}
      {#if data.mod_id}
        <KeyValueList
          label="Compatibility patch"
          items={[
            { label: "Id", value: data.mod_id, mono: true },
            { label: "Version", value: data.version },
          ]}
        />
      {/if}
      {#if badge}
        <div class="verdict">
          <Badge tone={badge.tone}>{badge.text}</Badge>
          {#if data.equivalence.kind === "differs"}
            <span>With the compatibility patch, these files would change: {data.equivalence.files.join(", ")}. Keep the fixes instead.</span>
          {:else if data.equivalence.kind === "not_checked"}
            <span>{data.equivalence.reason}</span>
          {:else}
            <span>The game gets the same code with the compatibility patch as with your fixes.</span>
          {/if}
        </div>
      {/if}
      {#if data.refused.length > 0}
        <Callout tone="warn" title="Some fixes cannot be saved this way.">
          <ul aria-label="Fixes it cannot include">
            {#each data.refused as r (r.fix_id)}<li>{label(r.fix_id)}: {r.reason}</li>{/each}
          </ul>
        </Callout>
      {/if}
      {#if data.requires.length > 0}
        <Section title="Requires">
          <ul class="list" aria-label="Requires">
            {#each data.requires as r (r.mod_id)}<li>{requirementText(r)}</li>{/each}
          </ul>
        </Section>
      {/if}
      {#if data.included.length > 0}
        <Section title="Fixes it includes">
          <ul class="list" aria-label="Fixes it includes">
            {#each data.included as id (id)}<li>{label(id)}</li>{/each}
          </ul>
        </Section>
      {/if}
      {#if data.files.length > 0}
        <Section title="Files">
          {#each data.files as f (f.name)}
            <details class="file">
              <summary><code>{f.name}</code></summary>
              <pre aria-label={f.name}>{f.text}</pre>
            </details>
          {/each}
        </Section>
      {/if}
    {/if}
  </div>
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" disabled={!canSave} aria-busy={busy} onclick={save}>Save compatibility patch</Button>
  {/snippet}
</Dialog>

<style>
  .export {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .verdict {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--qm-space-2);
    font-size: var(--qm-text-13);
  }

  .checking {
    margin: 0;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-13);
  }

  ul {
    margin: 0;
    padding-left: var(--qm-space-4);
  }

  .list li + li {
    margin-top: var(--qm-space-1);
  }

  .file + .file {
    margin-top: var(--qm-space-2);
  }

  summary {
    cursor: pointer;
  }

  code,
  pre {
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
  }

  pre {
    max-height: 16rem;
    margin: var(--qm-space-1) 0 0;
    padding: var(--qm-space-2);
    overflow: auto;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    background: var(--qm-raised);
    white-space: pre;
  }
</style>
