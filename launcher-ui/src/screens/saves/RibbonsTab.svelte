<script lang="ts">
  import { untrack } from "svelte";
  import { EmptyState } from "../../components/ui";
  import type { SaveSummary } from "../../lib/bindings";
  import { saves } from "../../lib/stores/saves.svelte";
  import SpriteImage from "./SpriteImage.svelte";

  interface Props {
    summary: SaveSummary;
  }

  let { summary }: Props = $props();

  $effect(() => untrack(() => saves.loadRibbons()));

  const ribbons = $derived(
    summary.ribbons.map((id) => {
      const info = saves.ribbons?.get(id);
      return { id, name: info?.name ?? `Ribbon ${id}`, description: info?.description ?? "" };
    }),
  );
</script>

{#if ribbons.length === 0}
  <EmptyState title="No ribbons yet." body="Ribbons the game awards show up here." />
{:else}
  <ul class="grid" aria-label="Ribbons">
    {#each ribbons as r (r.id)}
      <li class="ribbon">
        <SpriteImage kind="ribbon" id={r.id} alt={r.name} class="image" />
        <span class="name">{r.name}</span>
        {#if r.description}<span class="qm-muted desc">{r.description}</span>{/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--qm-detail-w) * 0.5), 1fr));
    gap: var(--qm-space-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ribbon {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--qm-space-1);
    padding: var(--qm-space-3);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    text-align: center;
  }

  .ribbon :global(.image) {
    width: 96px;
    height: 32px;
  }

  .name {
    font-size: var(--qm-text-13);
    font-weight: 600;
  }

  .desc {
    font-size: var(--qm-text-12);
  }
</style>
