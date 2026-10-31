<script lang="ts">
  import { call } from "../lib/api";
  import { commands, type HealthReport, type Loadable } from "../lib/bindings";
  import { onEvent } from "../lib/events";
  import { logs } from "../lib/stores/logs.svelte";

  let health = $state<Loadable<HealthReport>>({ status: "loading" });

  function load() {
    call(commands.healthGet())
      .then((h) => (health = h))
      .catch(() => {});
  }

  $effect(() => {
    load();
    return onEvent((e) => {
      if (e.type === "invalidated" && e.scope === "health") load();
    });
  });

  const yesNo = (v: boolean) => (v ? "Yes" : "No");
</script>

<section class="qm-page">
  <h1>Diagnostics</h1>

  <div class="qm-card">
    <h2>Health</h2>
    {#if health.status === "loading"}
      <p class="qm-muted">Checking...</p>
    {:else}
      <dl>
        <dt>Loader DLL present</dt>
        <dd>{yesNo(health.data.dll_present)}</dd>
        <dt>Loader DLL matches build</dt>
        <dd>{yesNo(health.data.dll_matches_build)}</dd>
        <dt>Game folder modded</dt>
        <dd>{yesNo(health.data.is_game_dir_modded)}</dd>
        <dt>Game running</dt>
        <dd>{health.data.game_running_pid === null ? "No" : `Yes (PID ${health.data.game_running_pid})`}</dd>
      </dl>
      {#if health.data.steam_cloud_warning}
        <p class="warn">{health.data.steam_cloud_warning}</p>
      {/if}
    {/if}
  </div>

  <div class="qm-card">
    <h2>Live log</h2>
    {#if logs.lines.length === 0}
      <p class="qm-muted">No log lines yet.</p>
    {:else}
      <ol class="log">
        {#each logs.lines as entry (entry.seq)}
          <li><span class="qm-muted">{entry.source}</span> {entry.line}</li>
        {/each}
      </ol>
    {/if}
  </div>
</section>

<style>
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--qm-space-2) var(--qm-space-5);
    margin: var(--qm-space-3) 0 0;
  }

  dt {
    color: var(--qm-text-muted);
  }

  dd {
    margin: 0;
  }

  .warn {
    color: var(--qm-warn);
  }

  .log {
    max-height: 320px;
    margin: var(--qm-space-3) 0 0;
    padding: 0;
    overflow: auto;
    list-style: none;
    font-family: var(--qm-font-mono);
    font-size: 12px;
  }
</style>
