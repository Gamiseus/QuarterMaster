<script lang="ts">
  import { call } from "../lib/api";
  import { commands, type SaveSlotSummary } from "../lib/bindings";
  import { onEvent } from "../lib/events";

  let saves = $state<SaveSlotSummary[] | null>(null);

  function load() {
    call(commands.savesList())
      .then((list) => (saves = list))
      .catch(() => {});
  }

  $effect(() => {
    load();
    return onEvent((e) => {
      if (e.type === "invalidated" && e.scope === "saves") load();
    });
  });
</script>

<section class="qm-page">
  <h1>Saves</h1>

  {#if saves === null}
    <p class="qm-muted">Loading saves...</p>
  {:else if saves.length === 0}
    <p class="qm-muted">No save slots.</p>
  {:else}
    <ul class="qm-card">
      {#each saves as s (s.profile.id)}
        <li class:active={s.is_active}>
          {s.profile.name}
          {#if s.is_active}<span class="qm-muted">Active</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  ul {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    margin: 0;
    list-style: none;
  }

  li {
    display: flex;
    justify-content: space-between;
  }

  li.active {
    font-weight: 600;
  }
</style>
