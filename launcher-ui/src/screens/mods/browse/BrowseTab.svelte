<script lang="ts">
  import { untrack } from "svelte";
  import { Button, Callout, EmptyState, Skeleton } from "../../../components/ui";
  import { formatTimestamp } from "../../../lib/saves/format";
  import { catalog } from "../../../lib/stores/catalog.svelte";
  import CatalogCard from "./CatalogCard.svelte";
  import CatalogFilters from "./CatalogFilters.svelte";

  // The catalog is fetched the first time Browse opens, not at startup.
  $effect(() => untrack(() => catalog.ensureLoaded()));
</script>

<div class="browse">
  <CatalogFilters />
  {#if catalog.result.status === "loading"}
    <div class="grid" aria-busy="true" aria-label="Loading the catalog">
      {#each [1, 2, 3, 4] as n (n)}
        <div class="placeholder"><Skeleton lines={4} /></div>
      {/each}
    </div>
  {:else}
    {@const r = catalog.result.data}
    <!-- The site with the mod list could not be reached: the last good copy is shown. -->
    {#if r.offline && r.copy_from}
      <Callout tone="info" title="Offline copy from {formatTimestamp(r.copy_from)}">
        <p>The mod list site could not be reached, so this is the last list QuarterMaster saw.</p>
        {#snippet action()}
          <Button size="sm" onclick={() => catalog.refresh().catch(() => {})}>Refresh</Button>
        {/snippet}
      </Callout>
    {:else if r.error_message}
      <Callout tone="warn" title="Could not load the mod list.">
        <p>{r.error_message}</p>
        {#snippet action()}
          <Button size="sm" onclick={() => catalog.refresh().catch(() => {})}>Refresh</Button>
        {/snippet}
      </Callout>
    {/if}
    {#if r.catalog.length > 0}
      <div class="grid">
        {#each r.catalog as entry (entry.id)}
          <CatalogCard {entry} />
        {/each}
      </div>
    {:else if catalog.filtering || !r.error_message}
      <EmptyState title="No mods match." body="Try another search, topic or source.">
        {#snippet action()}
          <Button onclick={() => catalog.clearFilters()}>Clear filters</Button>
        {/snippet}
      </EmptyState>
    {/if}
  {/if}
</div>

<style>
  .browse {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--qm-detail-w) * 0.8), 1fr));
    gap: var(--qm-space-3);
  }

  .placeholder {
    padding: var(--qm-space-4);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
  }
</style>
