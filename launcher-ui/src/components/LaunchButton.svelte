<script lang="ts">
  import type { GameStage } from "../lib/bindings";
  import { game } from "../lib/stores/game.svelte";
  import { profiles } from "../lib/stores/profiles.svelte";

  const LABELS: Record<GameStage["stage"], string> = {
    idle: "Launch",
    deploying: "Preparing...",
    waiting_for_steam: "Waiting for Steam...",
    running: "Running",
    cleaning_up: "Cleaning up...",
    failed: "Launch",
  };

  const stage = $derived(game.status.stage);
  const label = $derived(LABELS[stage.stage]);
  const profileId = $derived(profiles.active?.id ?? null);
  let pending = $state(false);

  async function launch() {
    if (!profileId || game.isBusy) return;
    pending = true;
    try {
      await game.launch(profileId);
    } catch {
      // Routed to the UI by call().
    } finally {
      pending = false;
    }
  }
</script>

<div class="launch">
  <button
    type="button"
    class="launch-button"
    data-testid="launch-button"
    disabled={game.isBusy || pending || !profileId}
    onclick={launch}
  >
    {label}
  </button>
  {#if stage.stage === "failed"}
    <p class="reason" role="alert">{stage.reason}</p>
  {/if}
</div>

<style>
  .launch {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--qm-space-2);
  }

  .launch-button {
    min-width: 200px;
    padding: var(--qm-space-3) var(--qm-space-6);
    border: 0;
    border-radius: var(--qm-radius-md);
    background: var(--qm-accent);
    color: var(--qm-accent-contrast);
    font-size: 16px;
    font-weight: 600;
    letter-spacing: 0.02em;
    transition: filter var(--qm-dur-fast) var(--qm-ease), opacity var(--qm-dur-fast) var(--qm-ease);
  }

  .launch-button:hover:not(:disabled) {
    filter: brightness(1.08);
  }

  .launch-button:disabled {
    opacity: 0.6;
  }

  .reason {
    margin: 0;
    color: var(--qm-danger);
  }
</style>
