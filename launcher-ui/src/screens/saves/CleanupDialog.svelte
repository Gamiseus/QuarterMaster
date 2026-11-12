<script lang="ts">
  import { Button, Checkbox, Dialog } from "../../components/ui";
  import type { LeftoverFields } from "../../lib/bindings";
  import { cleanedText, leftoverRows } from "../../lib/saves/leftover";
  import { saves } from "../../lib/stores/saves.svelte";
  import { toasts } from "../../lib/stores/toasts.svelte";

  interface Props {
    open: boolean;
    slotId: string;
    slotName: string;
    leftover: LeftoverFields;
    /** Why the save cannot be changed now; null when it can. */
    blocked: string | null;
  }

  let { open = $bindable(false), slotId, slotName, leftover, blocked }: Props = $props();

  const rows = $derived(leftoverRows(leftover));
  const removable = $derived(rows.filter((r) => r.cleanable));
  const kept = $derived(rows.filter((r) => !r.cleanable));
  /** Values the user unticked; everything removable is ticked at first. */
  let skipped = $state<Record<string, boolean>>({});
  let busy = $state(false);

  const chosen = $derived(removable.filter((r) => !skipped[r.field]).map((r) => r.field));

  $effect(() => {
    if (open) skipped = {};
  });

  async function cleanUp() {
    if (busy || chosen.length === 0) return;
    busy = true;
    try {
      const done = await saves.cleanup(slotId, chosen);
      toasts.info(cleanedText(done.removed.length, slotName));
      open = false;
    } catch {
      // call() showed the error; the save is unchanged.
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  bind:open
  title="Clean up {slotName}?"
  description="These values were written by mods that no profile on this slot loads any more. The game keeps them in the save until they are removed."
  initialFocus="content"
  dismissible={!busy}
>
  {#if removable.length > 0}
    <ul class="rows" data-testid="cleanup-fields">
      {#each removable as r (r.field)}
        <li>
          <Checkbox
            label={`${r.field} (from ${r.mods.join(", ")})`}
            bind:checked={() => !skipped[r.field], (v) => (skipped[r.field] = !v)}
            disabled={busy}
          />
        </li>
      {/each}
    </ul>
  {/if}
  {#if kept.length > 0}
    <p class="qm-muted">Not removed, because more than one mod wrote them:</p>
    <ul class="kept" data-testid="cleanup-kept">
      {#each kept as r (r.field)}
        <li>{r.field} (from {r.mods.join(", ")})</li>
      {/each}
    </ul>
  {/if}
  <p class="qm-muted">
    {removable.length > 0
      ? "A backup is made first. Only the ticked values are taken out; everything else in the save stays as it is."
      : "Nothing here can be removed safely."}
  </p>
  {#if blocked}<p class="qm-muted" role="note">{blocked}</p>{/if}
  {#snippet footer()}
    <Button disabled={busy} onclick={() => (open = false)}>{removable.length > 0 ? "Cancel" : "Close"}</Button>
    {#if removable.length > 0}
      <Button variant="primary" disabled={busy || blocked !== null || chosen.length === 0} aria-busy={busy} onclick={cleanUp}>
        Clean up
      </Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  p {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .rows,
  .kept {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .kept {
    gap: 2px;
    padding-left: var(--qm-space-4);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-12);
    list-style: disc;
  }
</style>
