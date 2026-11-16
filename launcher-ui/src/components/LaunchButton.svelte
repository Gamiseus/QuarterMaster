<script lang="ts">
  import type { GameStage } from "../lib/bindings";
  import { game } from "../lib/stores/game.svelte";
  import { loadOrder } from "../lib/stores/loadorder.svelte";
  import { prelaunch } from "../lib/stores/prelaunch.svelte";
  import { Button, Tooltip } from "./ui";

  const LABELS: Record<GameStage["stage"], string> = {
    idle: "Launch",
    deploying: "Preparing...",
    waiting_for_steam: "Waiting for Steam...",
    running: "Running",
    cleaning_up: "Cleaning up...",
    failed: "Launch",
  };

  const stage = $derived(game.status.stage);
  // The error gate cannot be checked: Launch stays off until a retry loads the resolution and the simulation.
  const checkFailed = $derived(
    !prelaunch.vanillaActive && (loadOrder.resolution.status === "error" || loadOrder.simulation.status === "error"),
  );
  const label = $derived(prelaunch.checking ? "Checking..." : LABELS[stage.stage]);
</script>

<div class="launch">
  <Button variant="primary" data-testid="launch-button" disabled={!prelaunch.canLaunch || prelaunch.checking} onclick={() => prelaunch.request()}>
    {label}
  </Button>
  {#if checkFailed}
    <Tooltip text="Could not check the load order. Retry before launching.">
      {#snippet children({ props })}
        <Button
          {...props}
          size="sm"
          variant="ghost"
          aria-label="Retry checking the load order"
          onclick={() => loadOrder.load().catch(() => {})}
        >
          Retry
        </Button>
      {/snippet}
    </Tooltip>
  {/if}
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
