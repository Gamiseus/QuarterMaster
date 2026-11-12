<script lang="ts">
  import { Button, Callout } from "../../components/ui";
  import { updates } from "../../lib/stores/updates.svelte";
</script>

<!-- One note for the whole check, never one per mod: github.com answered 429 to a fallback check. -->
{#if updates.busy}
  <Callout tone="warn" title="GitHub is busy. Some mods were not checked.">
    <p>Try again later.</p>
    {#snippet action()}
      <Button size="sm" onclick={() => updates.refresh().catch(() => {})}>Refresh</Button>
    {/snippet}
  </Callout>
{/if}
