<script lang="ts">
  import type { Component } from "svelte";
  import { app, type Page } from "../lib/stores/app.svelte";
  import { IconDiagnostics, IconEngine, IconMods, IconSaves, IconSettings, IconSidebar } from "./icons";
  import { IconButton } from "./ui";

  const STORAGE_KEY = "qm.sidebar.expanded";

  const items: { page: Page; label: string; icon: Component }[] = [
    { page: "mods", label: "Mods", icon: IconMods },
    { page: "saves", label: "Saves", icon: IconSaves },
    { page: "diagnostics", label: "Diagnostics", icon: IconDiagnostics },
    { page: "settings", label: "Settings", icon: IconSettings },
  ];

  function readExpanded(): boolean {
    try {
      return localStorage.getItem(STORAGE_KEY) === "true";
    } catch {
      return false;
    }
  }

  let expanded = $state(readExpanded());

  function toggleExpanded() {
    expanded = !expanded;
    try {
      localStorage.setItem(STORAGE_KEY, String(expanded));
    } catch {
      // Storage can be blocked; the choice then lasts for this session only.
    }
  }

  function go(page: Page) {
    app.mode = "launcher";
    app.page = page;
  }

  function toggleMode() {
    app.mode = app.mode === "engine" ? "launcher" : "engine";
  }

  const engine = $derived(app.mode === "engine");
</script>

<nav class={["sidebar", { expanded }]} aria-label="Main">
  <div class="group">
    <div class="row">
      <IconButton
        class="nav-btn"
        icon={IconSidebar}
        label={expanded ? "Collapse sidebar" : "Expand sidebar"}
        aria-expanded={expanded}
        onclick={toggleExpanded}
      />
    </div>
    {#each items as item (item.page)}
      {@const current = app.mode === "launcher" && app.page === item.page}
      <div class="row" class:active={current}>
        <IconButton
          class="nav-btn"
          icon={item.icon}
          label={item.label}
          aria-current={current ? "page" : undefined}
          onclick={() => go(item.page)}
        />
        {#if expanded}<span class="label" aria-hidden="true">{item.label}</span>{/if}
      </div>
    {/each}
  </div>
  <div class="row" class:active={engine}>
    <IconButton
      class="nav-btn"
      icon={IconEngine}
      label="Mod Engine"
      role="switch"
      aria-checked={engine}
      onclick={toggleMode}
    />
    {#if expanded}<span class="label" aria-hidden="true">Mod Engine</span>{/if}
  </div>
</nav>

<style>
  .sidebar {
    display: flex;
    flex: none;
    flex-direction: column;
    justify-content: space-between;
    width: var(--qm-sidebar-w);
    padding: var(--qm-space-2);
    background: var(--qm-surface);
    border-right: 1px solid var(--qm-border);
    transition: width var(--qm-dur-fast) var(--qm-ease);
  }

  .sidebar.expanded {
    width: var(--qm-sidebar-w-open);
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
  }

  /* The visible label sits over the full-width button and lets clicks through to it. */
  .row {
    position: relative;
  }

  .expanded .row :global(.nav-btn) {
    justify-content: flex-start;
    width: 100%;
    padding-left: calc((var(--qm-row-h) - var(--qm-icon-size)) / 2);
  }

  .label {
    position: absolute;
    top: 50%;
    left: calc(var(--qm-row-h) + var(--qm-space-1));
    transform: translateY(-50%);
    color: var(--qm-text);
    font-size: var(--qm-text-13);
    white-space: nowrap;
    pointer-events: none;
  }

  .row.active :global(.nav-btn) {
    background: var(--qm-raised);
    border-color: var(--qm-accent);
    color: var(--qm-accent-text);
  }
</style>
