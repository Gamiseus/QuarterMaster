<script lang="ts">
  import { mods } from "../lib/stores/mods.svelte";
</script>

<section class="qm-page">
  <h1>Mods</h1>

  {#if mods.list.status === "loading"}
    <p class="qm-muted">Loading mods...</p>
  {:else if mods.list.data.length === 0}
    <p class="qm-muted">No mods installed.</p>
  {:else}
    <table class="qm-card">
      <thead>
        <tr><th>Name</th><th>Version</th><th>Status</th></tr>
      </thead>
      <tbody>
        {#each mods.list.data as m (m.id)}
          <tr>
            <td>{m.name}</td>
            <td class="mono">{m.version}</td>
            <td class:on={m.enabled}>{m.enabled ? "Enabled" : "Disabled"}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</section>

<style>
  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    text-align: left;
    font-weight: 600;
    color: var(--qm-text-muted);
  }

  th,
  td {
    padding: var(--qm-space-2) var(--qm-space-3);
    border-bottom: 1px solid var(--qm-border);
  }

  tr:last-child td {
    border-bottom: 0;
  }

  .mono {
    font-family: var(--qm-font-mono);
  }

  td {
    color: var(--qm-text-muted);
  }

  td:first-child,
  td.on {
    color: var(--qm-text);
  }

  td.on {
    color: var(--qm-ok);
  }
</style>
