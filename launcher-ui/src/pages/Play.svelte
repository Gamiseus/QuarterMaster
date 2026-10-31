<script lang="ts">
  import LaunchButton from "../components/LaunchButton.svelte";
  import { jobs } from "../lib/stores/jobs.svelte";
  import { profiles } from "../lib/stores/profiles.svelte";
</script>

<section class="qm-page">
  <h1>Play</h1>

  <div class="qm-card hero">
    <div>
      <p class="qm-muted label">Active profile</p>
      {#if profiles.active}
        <p class="profile">{profiles.active.name}</p>
        <p class="qm-muted">{profiles.active.enabled_count} mods enabled</p>
      {:else if profiles.loaded}
        <p class="profile">No active profile</p>
      {:else}
        <p class="qm-muted">Loading profiles...</p>
      {/if}
    </div>
    <LaunchButton />
  </div>

  {#if profiles.list.length > 0}
    <div class="qm-card">
      <h2>Profiles</h2>
      <ul class="profiles">
        {#each profiles.list as p (p.id)}
          <li class:active={p.is_active}>{p.name}</li>
        {/each}
      </ul>
    </div>
  {/if}

  {#if jobs.list.length > 0}
    <div class="qm-card">
      <h2>Background tasks</h2>
      <ul class="jobs">
        {#each jobs.list as job (job.id)}
          <li>
            <span>{job.label}</span>
            <progress max="1" value={job.progress}></progress>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</section>

<style>
  .hero {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-5);
  }

  p {
    margin: 0;
  }

  .label {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .profile {
    font-size: 18px;
    font-weight: 600;
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: var(--qm-space-3) 0 0;
    padding: 0;
    list-style: none;
  }

  .profiles li.active {
    color: var(--qm-accent);
    font-weight: 600;
  }

  .jobs li {
    display: grid;
    grid-template-columns: 1fr 200px;
    align-items: center;
    gap: var(--qm-space-3);
  }

  progress {
    width: 100%;
    accent-color: var(--qm-accent);
  }
</style>
