<script lang="ts">
  import { Button, Callout, Checkbox, Dialog, EmptyState, Skeleton } from "../../../components/ui";
  import { call } from "../../../lib/api";
  import { commands, type FixDraft, type PreviewResult, type SharedStack } from "../../../lib/bindings";
  import { blockLabel } from "../../../lib/patches/stacks";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import ImpactNotes from "./ImpactNotes.svelte";
  import StackCard from "./StackCard.svelte";

  interface Props {
    modNames: Record<string, string>;
    canMutate: boolean;
  }

  let { modNames, canMutate }: Props = $props();

  // Every shared line in the profile, grouped by file. A block moves with a StackRule fix (D55),
  // always through the preview.
  let stacks = $state<{ status: "loading" } | { status: "ready"; data: SharedStack[] } | { status: "error"; message: string }>({ status: "loading" });
  let seq = 0;

  type Preview = { status: "loading" } | { status: "ready"; data: PreviewResult } | { status: "error"; message: string };
  /** A move waiting for Apply. */
  let pending = $state<{ stack: SharedStack; patchId: string; draft: FixDraft; preview: Preview } | null>(null);
  let confirmOpen = $state(false);
  /** "I understand" for a move that puts a mod's block above one it depends on (D56). */
  let understood = $state(false);
  let busy = $state(false);
  let previewSeq = 0;

  const modName = (id: string) => modNames[id] ?? id;

  async function load() {
    const mine = ++seq;
    try {
      const data = await call(commands.stacksGet(null), { expect: ["conflict"] });
      if (mine === seq) stacks = { status: "ready", data };
    } catch (e) {
      if (mine === seq) stacks = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  // Reload with every new simulation (a mode change, a mod change, a new fix).
  $effect(() => {
    if (loadOrder.simulation.status === "ready") void load();
  });

  const byFile = $derived.by(() => {
    const out = new Map<string, SharedStack[]>();
    if (stacks.status === "ready") for (const s of stacks.data) out.set(s.target_file, [...(out.get(s.target_file) ?? []), s]);
    return [...out];
  });

  async function preview(draft: FixDraft, file: string) {
    const mine = ++previewSeq;
    if (pending) pending.preview = { status: "loading" };
    try {
      const data = await call(commands.simulationPreview({ add_fixes: [draft], focus_file: file }), { expect: ["cancelled", "conflict", "invalid_input"] });
      if (mine === previewSeq && pending) pending.preview = { status: "ready", data };
    } catch (e) {
      if (mine === previewSeq && pending) pending.preview = { status: "error", message: e instanceof Error ? e.message : String(e) };
    }
  }

  function onmove(stack: SharedStack, patchId: string, draft: FixDraft) {
    if (!canMutate) return;
    pending = { stack, patchId, draft, preview: { status: "loading" } };
    understood = false;
    confirmOpen = true;
    void preview(draft, stack.target_file);
  }

  async function apply() {
    if (!pending || busy) return;
    busy = true;
    try {
      await fixes.create(pending.draft);
      confirmOpen = false;
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }

  /** A block's name in the pending stack: the mod, plus the patch when the mod has several there. */
  function blockName(s: SharedStack, id: string | null | undefined): string {
    const b = s.blocks.find((x) => x.patch_id === id);
    return b ? blockLabel(b, s.blocks, modName) : (id ?? "");
  }

  function moveText(p: NonNullable<typeof pending>): string {
    const c = p.draft.change;
    if (c.kind !== "stack_rule") return "";
    const where = c.below_patch_id ? `below ${blockName(p.stack, c.below_patch_id)}'s` : `above ${blockName(p.stack, c.above_patch_id)}'s`;
    return `${blockName(p.stack, p.patchId)}'s code goes ${where} ${p.stack.side} line ${p.stack.file_line} of ${p.stack.target_file}, in every stacking mode.`;
  }
</script>

<div class="stack-view">
  {#if stacks.status === "loading"}
    <div aria-busy="true" aria-label="Loading shared lines"><Skeleton lines={4} /></div>
  {:else if stacks.status === "error"}
    <Callout tone="danger" title="Could not load the shared lines.">
      {stacks.message}
      {#snippet action()}<Button size="sm" onclick={() => load()}>Retry</Button>{/snippet}
    </Callout>
  {:else if byFile.length === 0}
    <EmptyState title="No shared lines" body="No line has code from more than one patch on the same side." />
  {:else}
    <p class="qm-muted intro">Lines where several patches add code, in the order the game runs it. Drag a block or use Move... to change it.</p>
    {#each byFile as [file, list] (file)}
      <section aria-label={`Shared lines in ${file}`}>
        <h4 class="file">{file}</h4>
        {#each list as s (s.stack_key)}
          <StackCard stack={s} {modName} {canMutate} {onmove} />
        {/each}
      </section>
    {/each}
  {/if}
</div>

{#if pending}
  {@const p = pending}
  {@const breaks = p.preview.status === "ready" && p.preview.data.impact.breaks_file}
  {@const warnings = p.preview.status === "ready" ? (p.preview.data.impact.relation_warnings ?? []) : []}
  <Dialog
    bind:open={confirmOpen}
    title={`Move ${blockName(p.stack, p.patchId)}'s block?`}
    description={moveText(p)}
    initialFocus="content"
    size="lg"
    dismissible={!busy}
  >
    {#if p.preview.status === "loading"}
      <div aria-busy="true" aria-label="Checking what changes"><Skeleton lines={3} /></div>
    {:else if p.preview.status === "error"}
      <Callout tone="danger" title="Could not check what this changes.">
        {p.preview.message}
        {#snippet action()}<Button size="sm" onclick={() => preview(p.draft, p.stack.target_file)}>Retry</Button>{/snippet}
      </Callout>
    {:else}
      <ImpactNotes impact={p.preview.data.impact} {modNames} name={(id) => blockName(p.stack, id)} />
      {#if warnings.length > 0}
        <Callout tone="warn" title="This order may break a mod.">
          <ul class="warnings">
            {#each warnings as w (w.rule + w.text)}<li>{w.text}.</li>{/each}
          </ul>
          {#snippet action()}<Checkbox label="I understand" bind:checked={understood} disabled={busy} />{/snippet}
        </Callout>
      {/if}
    {/if}
    {#snippet footer()}
      <Button disabled={busy} onclick={() => (confirmOpen = false)}>Cancel</Button>
      <Button
        variant="primary"
        disabled={busy || p.preview.status !== "ready" || breaks || (warnings.length > 0 && !understood)}
        aria-busy={busy}
        onclick={apply}
      >
        Move it
      </Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .stack-view {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  p {
    margin: 0;
  }

  .intro {
    font-size: var(--qm-text-12);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  .file {
    margin: 0;
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    font-weight: 600;
  }

  .warnings {
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
  }
</style>
