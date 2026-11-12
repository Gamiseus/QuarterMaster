<script lang="ts">
  import { ConfirmDialog, Skeleton } from "../../../components/ui";
  import type { FixView, UndoPreview } from "../../../lib/bindings";
  import { confirmReason, fixes } from "../../../lib/stores/fixes.svelte";
  import ImpactNotes from "./ImpactNotes.svelte";

  interface Props {
    open?: boolean;
    fix: FixView | null;
    modNames: Record<string, string>;
  }

  let { open = $bindable(false), fix, modNames }: Props = $props();

  let preview = $state<UndoPreview | null>(null);
  let error = $state<string | null>(null);
  /** The service asked to confirm (the preview could not run): the next click confirms. */
  let refused = $state<string | null>(null);
  let token = 0;

  const label = (id: string) => fixes.list.find((f) => f.id === id)?.label ?? id;
  const name = (patchId: string) => {
    for (const f of fixes.list) {
      for (const l of [f.patch, ...f.related]) if (l && l.patch_id === patchId) return l.name ?? patchId;
    }
    return patchId;
  };

  // Each opening previews the undo again; the profile may have changed since.
  $effect(() => {
    if (!open || !fix) return;
    const mine = ++token;
    const id = fix.id;
    preview = null;
    error = null;
    refused = null;
    fixes
      .undoPreview(id)
      .then((p) => {
        if (mine === token) preview = p;
      })
      .catch((e: unknown) => {
        if (mine === token) error = e instanceof Error ? e.message : String(e);
      });
  });

  const asks = $derived(preview ? preview.also_undoes.length > 0 || preview.asks.length > 0 : false);
  // Confirm waits for the preview; after a failed one the service says what needs a confirm.
  const checking = $derived(preview === null && error === null);

  async function confirm() {
    if (!fix) return;
    const confirmed = asks || refused !== null;
    try {
      await fixes.undo(fix.id, confirmed);
    } catch (e) {
      const reason = confirmed ? null : confirmReason(e);
      if (reason !== null) refused = reason;
      throw e;
    }
  }
</script>

<ConfirmDialog
  bind:open
  title={fix ? `Undo "${fix.label}"?` : "Undo fix?"}
  body="The fix is taken out of this profile. The game code goes back to how it was without it."
  confirmLabel={refused ? "Undo anyway" : "Undo fix"}
  danger={false}
  confirmDisabled={checking}
  onconfirm={confirm}
>
  <div class="undo" aria-live="polite">
    {#if refused}
      <p role="alert">Undo changes more than this fix: {refused}</p>
    {:else if error}
      <p class="error" role="alert">Could not check what undo changes: {error}</p>
    {:else if !preview}
      <div aria-busy="true" aria-label="Checking what undo changes"><Skeleton lines={2} /></div>
    {:else}
      <ImpactNotes impact={preview.impact} {modNames} {name} />
      {#if preview.also_undoes.length > 0}
        <p>Also undoes, because it was made later on the same patch:</p>
        <ul aria-label="Also undone">
          {#each preview.also_undoes as id (id)}<li>{label(id)}</li>{/each}
        </ul>
      {/if}
      {#if preview.asks.length > 0}
        <p>You changed this since; undo leaves it as it is:</p>
        <ul aria-label="Left as it is">
          {#each preview.asks as ask (ask)}<li>{ask}</li>{/each}
        </ul>
      {/if}
    {/if}
  </div>
</ConfirmDialog>

<style>
  .undo {
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
