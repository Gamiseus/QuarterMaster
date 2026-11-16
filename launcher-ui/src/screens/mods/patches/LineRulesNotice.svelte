<script lang="ts">
  import { Button, Callout } from "../../../components/ui";
  import { lineRules } from "../../../lib/stores/linerules.svelte";

  interface Props {
    canMutate: boolean;
  }

  let { canMutate }: Props = $props();

  let reviewing = $state(false);
  let busy = $state(false);
  const uid = $props.id();

  const change = $derived(lineRules.change);
  const lines = $derived(change ? [...change.stacks, ...change.rungs] : []);

  async function run(on: boolean) {
    busy = true;
    try {
      await lineRules.set(on);
    } catch {
      // call() showed the error.
    } finally {
      busy = false;
    }
  }
</script>

<Callout tone="info" title="Newer rules for shared lines" class="line-rules-notice">
  <p>
    QuarterMaster now keeps code a mod adds below a line it rewrites with that line, and follows
    a mod's lines after another mod edits them. This changes {lineRules.countText} in this profile.
  </p>
  <div class="actions">
    <Button size="sm" variant="primary" disabled={!canMutate || busy} onclick={() => run(true)}>Use the new order</Button>
    <Button
      size="sm"
      disabled={lines.length === 0}
      aria-expanded={reviewing}
      aria-controls={`${uid}-review`}
      onclick={() => (reviewing = !reviewing)}
    >
      Review
    </Button>
    <Button size="sm" variant="ghost" disabled={!canMutate || busy} onclick={() => run(false)}>Keep the older order</Button>
  </div>
  {#if reviewing && lines.length > 0}
    <ul id={`${uid}-review`} class="review" aria-label="What changes">
      {#each lines as line, i (`${line}:${i}`)}<li>{line}</li>{/each}
    </ul>
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
    margin: var(--qm-space-2) 0 0;
    padding: 0 0 0 var(--qm-space-4);
    overflow: auto;
    overflow-wrap: anywhere;
    color: var(--qm-text-muted);
  }
</style>
