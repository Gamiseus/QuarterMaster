<script lang="ts">
  import type { GameStage } from "../lib/bindings";
  import { game } from "../lib/stores/game.svelte";
  import { prelaunch } from "../lib/stores/prelaunch.svelte";
  import { Button } from "./ui";

  const LABELS: Record<GameStage["stage"], string> = {
    idle: "Launch",
    deploying: "Preparing...",
    waiting_for_steam: "Waiting for Steam...",
    running: "Running",
    cleaning_up: "Cleaning up...",
    failed: "Launch",
  };

  const stage = $derived(game.status.stage);
  const label = $derived(prelaunch.vanilla && LABELS[stage.stage] === "Launch" ? "Launch vanilla" : LABELS[stage.stage]);
</script>

<div class="launch">
  <Button variant="primary" data-testid="launch-button" disabled={!prelaunch.canLaunch} onclick={() => prelaunch.request()}>
    {label}
  </Button>
  {#if stage.stage === "failed"}
    <span class="reason" role="alert" title={stage.reason}>{stage.reason}</span>
  {/if}
</div>

<style>
  .launch {
    display: flex;
    align-items: center;
    gap: var(--qm-space-2);
    min-width: 0;
  }

  .reason {
    overflow: hidden;
    max-width: calc(var(--qm-detail-w) / 2);
    color: var(--qm-danger);
    font-size: var(--qm-text-12);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
