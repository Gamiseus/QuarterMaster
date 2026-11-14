<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Screen } from "../../components/ui";
  import { diagnostics } from "../../lib/stores/diagnostics.svelte";
  import { jobs } from "../../lib/stores/jobs.svelte";
  import CrashList from "./CrashList.svelte";
  import HealthCard from "./HealthCard.svelte";
  import LogConsole from "./LogConsole.svelte";
  import TraceDialog from "./TraceDialog.svelte";

  let building = $state(false);
  let traceOpen = $state(false);
  const bundleRunning = $derived(building || jobs.list.some((j) => j.kind === "diagnostic_bundle"));

  // Fresh health and crashes each time the screen opens; events keep them current afterwards.
  $effect(() => untrack(() => diagnostics.load()));

  async function bugReport() {
    building = true;
    try {
      await diagnostics.buildBundle();
    } catch {
      // jobs.track or call() already reported it.
    } finally {
      building = false;
    }
  }
</script>

<Screen title="Diagnostics" scroll={false}>
  {#snippet actions()}
    <Button onclick={() => (traceOpen = true)}>Trace globals...</Button>
    <Button disabled={bundleRunning} onclick={bugReport}>Create bug report</Button>
  {/snippet}
  <div class="diag">
    <div class="top">
      <HealthCard />
      <CrashList />
    </div>
    <LogConsole />
  </div>
</Screen>
<TraceDialog bind:open={traceOpen} />

<style>
  .diag {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--qm-space-4);
    min-height: 0;
  }

  /* Health and crashes sit side by side at the same height, each capped at 40% of the screen (half in a tall window). */
  .top {
    flex: none;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--qm-space-4);
    /* Both cards are as tall as the taller one: no gap under the shorter. */
    align-items: stretch;
  }

  .top :global(:is(.health, .crashes)) {
    display: flex;
    flex-direction: column;
    max-height: 40vh;
    min-height: 0;
  }

  /* A taller window has the room: the log below still keeps more than a third of it. */
  @media (min-height: 780px) {
    .top :global(:is(.health, .crashes)) {
      max-height: 50vh;
    }
  }

  .top :global(.card-scroll) {
    flex: 0 1 auto;
    min-height: 0;
  }
</style>
