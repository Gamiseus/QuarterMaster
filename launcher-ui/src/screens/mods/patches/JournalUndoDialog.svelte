<script lang="ts">
  import { ConfirmDialog, RadioGroup, Skeleton } from "../../../components/ui";
  import { QmCommandError } from "../../../lib/api";
  import type { JournalUndoChoice, JournalUndoPreview } from "../../../lib/bindings";
  import { canUndo, choiceImpact, impactSummary, JOURNAL_CHOICES, journalBody, journalDone, journalTitle } from "../../../lib/patches/journal";
  import { fixes } from "../../../lib/stores/fixes.svelte";
  import { mods } from "../../../lib/stores/mods.svelte";
  import { toasts } from "../../../lib/stores/toasts.svelte";
  import ImpactNotes from "./ImpactNotes.svelte";

  // Mounted once in App, so the Undo of an export's toast works from any screen (D71).
  const step = $derived(fixes.journalStep);

  let preview = $state<JournalUndoPreview | null>(null);
  let error = $state<string | null>(null);
  /** The service refused the undo (for example a fix made meanwhile); shown inline. */
  let refused = $state<string | null>(null);
  let choice = $state<JournalUndoChoice | null>(null);
  let token = 0;

  const modNames = $derived(Object.fromEntries((mods.list.status === "ready" ? mods.list.data : []).map((m) => [m.id, m.name])));
  /** A mod by its name, as FactionNote names it. */
  const modName = (id: string) => modNames[id] ?? modNames[id.toLowerCase()] ?? id;
  const name = (patchId: string) => {
    for (const f of fixes.list) {
      for (const l of [f.patch, ...f.related]) if (l && l.patch_id === patchId) return l.name ?? patchId;
    }
    return patchId;
  };

  // Each opening previews the undo again; the profile may have changed since.
  $effect(() => {
    if (!fixes.journalOpen || !step) return;
    const mine = ++token;
    const index = step.index;
    preview = null;
    error = null;
    refused = null;
    choice = null;
    fixes
      .journalUndoPreview(index)
      .then((p) => {
        if (mine === token) preview = p;
      })
      .catch((e: unknown) => {
        if (mine === token) error = e instanceof Error ? e.message : String(e);
      });
  });

  const clashes = $derived(preview?.clashes ?? []);
  const items = $derived(
    preview
      ? JOURNAL_CHOICES.map((c) => {
          const impact = c.value === "pause_newer" ? preview!.impact_pause : preview!.impact_keep;
          // The notes under the option show it; screen readers get the summary.
          return { ...c, description: impactSummary(impact, modName, name), descriptionHidden: impact !== null };
        })
      : [],
  );
  const plain = $derived(preview && clashes.length === 0 ? choiceImpact(preview, null) : null);

  async function confirm() {
    if (!step) return;
    refused = null;
    try {
      if (await fixes.journalUndo(step.at, clashes.length > 0 ? choice : null)) toasts.info(journalDone(step.op));
    } catch (e) {
      if (e instanceof QmCommandError && e.kind === "conflict") refused = e.message;
      throw e;
    }
  }
</script>

<ConfirmDialog
  bind:open={fixes.journalOpen}
  title={step ? journalTitle(step.label) : "Undo this step?"}
  body={journalBody(step?.op ?? "")}
  confirmLabel="Undo"
  danger={false}
  confirmDisabled={!canUndo(preview, error !== null, choice)}
  onconfirm={confirm}
>
  <div class="journal-undo" aria-live="polite">
    {#if refused}
      <p class="error" role="alert">{refused}</p>
    {/if}
    {#if error}
      <p class="error" role="alert">Could not check what undo changes: {error}</p>
    {:else if !preview}
      <div aria-busy="true" aria-label="Checking what undo changes"><Skeleton lines={2} /></div>
    {:else}
      {#if preview.restores.length > 0}
        <p>Brings back:</p>
        <ul aria-label="Brought back">
          {#each preview.restores as label, i (i)}<li>{label}</li>{/each}
        </ul>
      {/if}
      {#if preview.turns_off_mods.length > 0}
        <p>Turns off: {preview.turns_off_mods.join(", ")}.</p>
      {/if}
      {#if clashes.length > 0}
        <p>Fixes made since change the same patches:</p>
        <ul aria-label="Made since">
          {#each clashes as c (`${c.restored_fix_id}:${c.newer_fix_id}`)}
            <li>{c.newer_label} <span class="qm-muted">(yours: {c.restored_label})</span></li>
          {/each}
        </ul>
        <RadioGroup label="How to undo" bind:value={choice} {items}>
          {#snippet extra(value)}
            {@const impact = value === "pause_newer" ? preview?.impact_pause : preview?.impact_keep}
            {#if impact}<ImpactNotes {impact} {modNames} {name} />{/if}
          {/snippet}
        </RadioGroup>
      {:else if plain}
        <ImpactNotes impact={plain} {modNames} {name} />
      {:else}
        <p class="qm-muted">{impactSummary(null)}</p>
      {/if}
    {/if}
  </div>
</ConfirmDialog>

<style>
  .journal-undo {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
  }

  p,
  ul {
    margin: 0;
  }

  ul {
    padding-left: var(--qm-space-4);
  }

  .error {
    color: var(--qm-danger);
  }
</style>
