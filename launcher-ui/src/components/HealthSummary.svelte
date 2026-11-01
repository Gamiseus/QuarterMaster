<script lang="ts">
  import { loadOrder } from "../lib/stores/loadorder.svelte";
  import { Badge } from "./ui";

  const sim = $derived(loadOrder.simulation);
  const conflicts = $derived(sim.status === "ready" ? sim.data.conflicts.length : 0);
</script>

{#if sim.status !== "ready"}
  <Badge data-testid="health-summary">Checking...</Badge>
{:else if conflicts > 0}
  <Badge tone="warn" data-testid="health-summary">{conflicts} {conflicts === 1 ? "conflict" : "conflicts"}</Badge>
{:else}
  <Badge tone="ok" data-testid="health-summary">{sim.data.applied_patches}/{sim.data.total_patches} patches ok</Badge>
{/if}
