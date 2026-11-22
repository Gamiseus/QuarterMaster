<script lang="ts">
  import { Button, Dialog } from "../../components/ui";
  import { engine, isDirty } from "../../lib/stores/engine.svelte";
  import { sprites } from "../../lib/stores/sprites.svelte";
  import { studio } from "../../lib/stores/studio.svelte";

  let busy = $state(false);
  const dirty = $derived([
    ...engine.files.filter(isDirty).map((f) => f.path),
    ...(studio.dirty && studio.draft ? [`The patch ${studio.draft.name || studio.draft.id || "being written"}`] : []),
    ...(sprites.dirty ? [`The place of ${sprites.mine?.item.name ?? "your sprite"}`] : []),
  ]);

  async function saveAll() {
    busy = true;
    try {
      await engine.saveAll();
    } finally {
      busy = false;
    }
    // A file that could not be saved (changed on disk, an error) keeps the screen where it is.
    engine.pendingLeave?.resolve(!engine.hasUnsaved);
  }

  function dontSave() {
    engine.discardAll();
    engine.pendingLeave?.resolve(true);
  }
</script>

<Dialog
  bind:open={() => engine.pendingLeave !== null, (v) => !v && engine.pendingLeave?.resolve(false)}
  title="Save your changes?"
  description={dirty.length === 1 ? `${dirty[0]} has changes that are not saved.` : `${dirty.length} files have changes that are not saved.`}
  dismissible={!busy}
>
  {#if dirty.length > 1}
    <ul class="files">
      {#each dirty as p (p)}<li>{p}</li>{/each}
    </ul>
  {/if}
  {#snippet footer()}
    <Button variant="ghost" disabled={busy} onclick={() => engine.pendingLeave?.resolve(false)}>Cancel</Button>
    <Button variant="secondary" disabled={busy} onclick={dontSave}>Don't save</Button>
    <Button variant="primary" disabled={busy} aria-busy={busy} data-autofocus="" onclick={saveAll}>Save all</Button>
  {/snippet}
</Dialog>

<style>
  .files {
    margin: 0;
    padding-left: var(--qm-space-5);
    font-family: var(--qm-font-mono);
    font-size: var(--qm-text-13);
  }
</style>
