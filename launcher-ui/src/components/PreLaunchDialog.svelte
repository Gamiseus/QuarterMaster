<script lang="ts">
  import { loadOrder } from "../lib/stores/loadorder.svelte";
  import { prelaunch } from "../lib/stores/prelaunch.svelte";
  import { Button, Dialog } from "./ui";

  const errors = $derived(loadOrder.errors);

  // The dialog only makes sense while errors exist (they may clear while it is open).
  $effect(() => {
    if (errors.length === 0) prelaunch.open = false;
  });

  function launchAnyway() {
    prelaunch.open = false;
    void prelaunch.launch();
  }
</script>

<Dialog
  bind:open={prelaunch.open}
  title="Fix before launching"
  description="The load order has errors. The game may crash or mods may not work."
>
  <ul class="errors">
    {#each errors as d, i (i)}
      <li>{d.message}</li>
    {/each}
  </ul>
  {#snippet footer()}
    <Button disabled={!prelaunch.canLaunch} onclick={launchAnyway}>Launch anyway</Button>
    <Button variant="primary" onclick={() => (prelaunch.open = false)}>Cancel</Button>
  {/snippet}
</Dialog>

<style>
  .errors {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    padding: 0 0 0 var(--qm-space-4);
    color: var(--qm-danger);
  }

  .errors li {
    color: var(--qm-text);
  }

  .errors li::marker {
    color: var(--qm-danger);
  }
</style>
