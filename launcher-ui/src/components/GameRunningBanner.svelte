<script lang="ts">
  import { game } from "../lib/stores/game.svelte";

  const message = $derived(
    game.banner ?? (game.isRunning ? "The game is running. Changes to mods and saves wait until it closes." : null),
  );
</script>

{#if message}
  <div class="banner" role="status">
    <span>{message}</span>
    {#if game.banner}
      <button type="button" class="dismiss" aria-label="Dismiss" onclick={() => game.dismissBanner()}>
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7" /></svg>
      </button>
    {/if}
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-3);
    padding: var(--qm-space-2) var(--qm-space-4) var(--qm-space-2) var(--qm-space-6);
    background: var(--qm-surface-raised);
    border-bottom: 1px solid var(--qm-border);
    box-shadow: inset 3px 0 0 var(--qm-warn);
    color: var(--qm-text);
  }

  .dismiss {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: var(--qm-radius-sm);
    background: transparent;
  }

  .dismiss:hover {
    background: var(--qm-surface);
  }

  svg {
    width: 10px;
    height: 10px;
    stroke: currentColor;
  }
</style>
