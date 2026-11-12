<script lang="ts">
  import { Badge, Button } from "../../../components/ui";
  import type { CatalogEntry } from "../../../lib/bindings";
  import { formatLicense, formatUpdated } from "../../../lib/catalog/format";
  import { openExternal } from "../../../lib/external";
  import { catalog } from "../../../lib/stores/catalog.svelte";
  import { installs } from "../../../lib/stores/installs.svelte";

  interface Props {
    entry: CatalogEntry;
  }

  let { entry }: Props = $props();

  const uid = $props.id();
  const nexus = $derived(entry.source_type === "NexusMods");
  const reason = $derived(entry.download_url ? installs.blockReason() : "No download for this entry.");
</script>

<article class="card" aria-labelledby="{uid}-name">
  <header class="head">
    <div class="title">
      <h3 class="name" id="{uid}-name">{entry.name}</h3>
      <span class="qm-muted owner">{entry.owner}</span>
    </div>
    {#if entry.installed}
      <Badge tone="ok">Installed {entry.installed_version ?? ""}</Badge>
    {/if}
  </header>
  {#if entry.description}
    <p class="description">{entry.description}</p>
  {/if}
  <dl class="meta">
    {#if !nexus}
      <div><dt>Stars</dt><dd>{entry.stargazers_count}</dd></div>
    {/if}
    <div><dt>Updated</dt><dd>{formatUpdated(entry.updated_at)}</dd></div>
    {#if !nexus}
      <div><dt>License</dt><dd>{formatLicense(entry.license)}</dd></div>
    {/if}
  </dl>
  {#if entry.topics.length > 0}
    <ul class="topics" aria-label="Topics">
      {#each entry.topics as topic (topic)}
        <li><Badge>{topic}</Badge></li>
      {/each}
    </ul>
  {/if}
  <footer class="actions">
    {#if nexus}
      <Button size="sm" variant="primary" onclick={() => openExternal(entry.html_url)}>Open page</Button>
    {:else}
      <Button size="sm" variant="ghost" onclick={() => openExternal(entry.html_url)}>Open page</Button>
      <Button size="sm" variant="primary" disabled={reason !== null} onclick={() => catalog.install(entry).catch(() => {})}>
        {entry.installed ? "Reinstall" : "Install"}
      </Button>
      {#if reason}
        <span class="qm-muted reason">{reason}</span>
      {/if}
    {/if}
  </footer>
</article>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    padding: var(--qm-space-4);
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-md);
    background: var(--qm-panel);
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--qm-space-2);
  }

  .title {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    margin: 0;
    font-size: var(--qm-text-14);
    font-weight: 600;
  }

  .owner,
  .reason {
    font-size: var(--qm-text-12);
  }

  .description {
    margin: 0;
    font-size: var(--qm-text-13);
  }

  /* Facts and actions each sit behind a hairline, apart from the description and topics. */
  .meta {
    padding-top: var(--qm-space-2);
    border-top: 1px solid var(--qm-border);
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-3);
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .meta div {
    display: flex;
    gap: var(--qm-space-1);
  }

  .meta dt {
    color: var(--qm-text-muted);
  }

  .meta dd {
    margin: 0;
  }

  .topics {
    display: flex;
    flex-wrap: wrap;
    gap: var(--qm-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .actions {
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--qm-space-2);
    margin-top: auto;
  }

  .reason {
    flex-basis: 100%;
    text-align: right;
  }
</style>
