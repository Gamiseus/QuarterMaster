<script lang="ts">
  import { Button, Select, TextField } from "../../../components/ui";
  import { catalog } from "../../../lib/stores/catalog.svelte";

  const SOURCES: { value: string | null; label: string }[] = [
    { value: null, label: "All sources" },
    { value: "GitHub", label: "GitHub" },
    { value: "NexusMods", label: "Nexus Mods" },
  ];

  const topicItems = $derived([
    { value: null as string | null, label: "All topics" },
    ...catalog.topics.map((t) => ({ value: t as string | null, label: t })),
  ]);
</script>

<div class="filters">
  <TextField
    class="search"
    label="Search the catalog"
    placeholder="Name, author or description"
    value={catalog.text}
    oninput={(e) => catalog.setText(e.currentTarget.value)}
  />
  <Select
    class="pick"
    label="Topic"
    items={topicItems}
    bind:value={() => catalog.query.topic, (v) => catalog.setTopic(v ?? null)}
  />
  <Select
    class="pick"
    label="Source"
    items={SOURCES}
    bind:value={() => catalog.query.source, (v) => catalog.setSource(v ?? null)}
  />
  <Button onclick={() => catalog.refresh().catch(() => {})}>Refresh</Button>
</div>

<style>
  .filters {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--qm-space-3);
  }

  .filters :global(.search) {
    flex: 1;
    min-width: calc(var(--qm-detail-w) / 2);
  }

  .filters :global(.pick) {
    width: calc(var(--qm-detail-w) / 2);
  }
</style>
