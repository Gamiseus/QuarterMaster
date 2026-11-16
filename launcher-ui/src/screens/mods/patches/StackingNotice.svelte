<script lang="ts">
  import { Button, Callout } from "../../../components/ui";
  import { loadOrder } from "../../../lib/stores/loadorder.svelte";
  import { stacking } from "../../../lib/stores/stacking.svelte";
  import StackChangeList from "./StackChangeList.svelte";

  interface Props {
    modNames: Record<string, string>;
    canMutate: boolean;
  }

  let { modNames, canMutate }: Props = $props();

  let reviewing = $state(false);
  let busy = $state(false);
  const uid = $props.id();

  // The check runs once the simulation is ready (the notice may open before it is).
  $effect(() => {
    void stacking.ensure();
  });

  const changes = $derived(stacking.changes);
  const count = $derived(stacking.count);
  const countText = $derived(
    count !== null
      ? `This changes ${count === 1 ? "1 place" : `${count} places`} in this profile.`
      : changes?.status === "error" || (changes === null && loadOrder.simulation.status === "error")
        ? "QuarterMaster could not check which places this changes."
        : "Checking which places this changes...",
  );

  async function run(write: () => Promise<void>) {
    busy = true;
    try {
      await write();
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }
</script>

<Callout tone="info" title="Code at shared lines" class="stacking-notice">
  <p>
    QuarterMaster can put code from mods that depend on other mods below theirs at shared lines. {countText}
  </p>
  <div class="actions">
    <Button size="sm" variant="primary" disabled={!canMutate || busy} onclick={() => run(() => stacking.set("dependencies"))}>Use dependency order</Button>
    <Button
      size="sm"
      disabled={count === null || count === 0}
      aria-expanded={reviewing}
      aria-controls={`${uid}-review`}
      onclick={() => (reviewing = !reviewing)}
    >
      Review
    </Button>
    <Button size="sm" variant="ghost" disabled={!canMutate || busy} onclick={() => run(() => stacking.set("legacy"))}>Keep the older order</Button>
  </div>
  {#if reviewing && changes?.status === "ready"}
    <div id={`${uid}-review`} class="review"><StackChangeList changes={changes.data} {modNames} /></div>
  {/if}
</Callout>

<style>
  p {
    margin: 0;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-2);
    margin-top: var(--qm-space-2);
  }

  .review {
    max-height: 280px;
    margin-top: var(--qm-space-2);
    overflow: auto;
  }
</style>
