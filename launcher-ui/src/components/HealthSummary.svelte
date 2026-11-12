<script lang="ts">
  import { isNote } from "../lib/patches/explain";
  import { fixes } from "../lib/stores/fixes.svelte";
  import { loadOrder } from "../lib/stores/loadorder.svelte";
  import { openFixes, openPatches } from "../screens/mods/mods-view.svelte";
  import { Badge } from "./ui";

  const sim = $derived(loadOrder.simulation);
  const conflicts = $derived(sim.status === "ready" ? sim.data.conflicts.filter((c) => !isNote(c)).length : 0);
  /** Fixes that warn before a launch (D23); they are not conflicts, so they get their own badge. */
  const review = $derived(fixes.attention.total);
</script>

{#if sim.status === "error"}
  <button type="button" class="retry" data-testid="health-summary" onclick={() => loadOrder.load().catch(() => {})}>
    <Badge tone="warn">Could not check. Retry</Badge>
  </button>
{:else if sim.status !== "ready"}
  <Badge data-testid="health-summary">Checking...</Badge>
{:else if conflicts > 0}
  <button type="button" class="retry" aria-label={`${conflicts} ${conflicts === 1 ? "conflict" : "conflicts"}. Open the Patches tab`} onclick={openPatches}>
    <Badge tone="warn" data-testid="health-summary">{conflicts} {conflicts === 1 ? "conflict" : "conflicts"}</Badge>
  </button>
{:else}
  <Badge tone="ok" data-testid="health-summary">{sim.data.applied_patches}/{sim.data.total_patches} patches ok</Badge>
{/if}
{#if review > 0}
  <button type="button" class="retry" aria-label={`${review} ${review === 1 ? "fix needs" : "fixes need"} your attention. Open Fixes`} onclick={openFixes}>
    <Badge tone="warn" data-testid="fix-summary">{review} {review === 1 ? "fix" : "fixes"} to review</Badge>
  </button>
{/if}

<style>
  .retry {
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
  }
</style>
