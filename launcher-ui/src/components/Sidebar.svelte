<script lang="ts">
  import { app, type Page } from "../lib/stores/app.svelte";

  const items: { page: Page; label: string }[] = [
    { page: "play", label: "Play" },
    { page: "mods", label: "Mods" },
    { page: "saves", label: "Saves" },
    { page: "diagnostics", label: "Diagnostics" },
    { page: "settings", label: "Settings" },
  ];

  function go(page: Page) {
    app.mode = "launcher";
    app.page = page;
  }

  function toggleMode() {
    app.mode = app.mode === "engine" ? "launcher" : "engine";
  }
</script>

<nav class="sidebar" aria-label="Main">
  <ul>
    {#each items as item (item.page)}
      <li>
        <button
          type="button"
          class:active={app.mode === "launcher" && app.page === item.page}
          aria-current={app.mode === "launcher" && app.page === item.page ? "page" : undefined}
          onclick={() => go(item.page)}
        >
          {item.label}
        </button>
      </li>
    {/each}
  </ul>
  <button
    type="button"
    class="mode"
    role="switch"
    aria-checked={app.mode === "engine"}
    onclick={toggleMode}
  >
    Mod Engine
  </button>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    width: 200px;
    padding: var(--qm-space-3);
    background: var(--qm-surface);
    border-right: 1px solid var(--qm-border);
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  button {
    width: 100%;
    padding: var(--qm-space-2) var(--qm-space-3);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: transparent;
    text-align: left;
    transition: background var(--qm-dur-fast) var(--qm-ease);
  }

  button:hover {
    background: var(--qm-surface-raised);
  }

  button.active {
    background: var(--qm-surface-raised);
    border-color: var(--qm-border);
    box-shadow: inset 3px 0 0 var(--qm-accent);
  }

  .mode {
    border-color: var(--qm-border);
    color: var(--qm-text-muted);
  }

  .mode[aria-checked="true"] {
    background: var(--qm-accent);
    border-color: var(--qm-accent);
    color: var(--qm-accent-contrast);
  }
</style>
