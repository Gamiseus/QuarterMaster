<script lang="ts">
  import { game } from "../lib/stores/game.svelte";
  import { IconClose } from "./icons";
  import { IconButton } from "./ui";

  const message = $derived(
    game.banner ?? (game.isRunning ? "The game is running. Changes to mods and saves wait until it closes." : null),
  );
</script>

{#if message}
  <div class="banner" role="status">
    <span>{message}</span>
    {#if game.banner}
      <IconButton icon={IconClose} label="Dismiss" onclick={() => game.dismissBanner()} />
    {/if}
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-1) var(--qm-space-3) var(--qm-space-1) var(--qm-space-5);
    background: var(--qm-raised);
    border-bottom: 1px solid var(--qm-border);
    box-shadow: inset 3px 0 0 var(--qm-warn);
    color: var(--qm-text);
  }
</style>
