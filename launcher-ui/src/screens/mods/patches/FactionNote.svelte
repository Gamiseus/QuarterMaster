<script lang="ts">
  import { Callout } from "../../../components/ui";
  import type { PreviewImpact } from "../../../lib/bindings";
  import { factionChanges } from "../../../lib/patches/impact";

  interface Props {
    impact: PreviewImpact | null;
    modNames: Record<string, string>;
  }

  let { impact, modNames }: Props = $props();

  /** Sides whose faction the change switches (D73). */
  const changes = $derived(factionChanges(impact, (id) => modNames[id] ?? modNames[id.toLowerCase()] ?? id));
</script>

{#if changes.length > 0}
  <Callout tone="warn" title="This changes which faction a side plays as.">
    <ul class="factions">
      {#each changes as c (c)}<li>{c}.</li>{/each}
    </ul>
    <p>Switching factions resets that side's unlocks in your save.</p>
  </Callout>
{/if}

<style>
  .factions {
    margin: 0;
    padding-left: var(--qm-space-4);
  }

  p {
    margin: var(--qm-space-1) 0 0;
  }
</style>
