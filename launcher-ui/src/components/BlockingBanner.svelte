<script lang="ts">
  import { loadOrder } from "../lib/stores/loadorder.svelte";
  import { prelaunch } from "../lib/stores/prelaunch.svelte";
  import { Button } from "./ui";

  const errors = $derived(loadOrder.errors);
  const more = $derived(errors.length - 1);
</script>

{#if errors.length > 0}
  <div class="banner" role="alert">
    <span class="message">
      {errors[0].message}{#if more > 0}<span class="more"> ({more} more)</span>{/if}
    </span>
    {#if !prelaunch.vanilla}
      <Button size="sm" onclick={() => prelaunch.review()}>Review</Button>
    {/if}
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-2) var(--qm-space-4) var(--qm-space-2) var(--qm-space-5);
    background: var(--qm-raised);
    border-bottom: 1px solid var(--qm-border);
    box-shadow: inset 3px 0 0 var(--qm-danger);
    color: var(--qm-text);
  }

  .message {
    min-width: 0;
  }

  .more {
    margin-left: var(--qm-space-1);
    color: var(--qm-text-muted);
  }
</style>
