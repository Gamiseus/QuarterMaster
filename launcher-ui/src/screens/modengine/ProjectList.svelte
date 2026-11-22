<script lang="ts">
  import { Badge, Button } from "../../components/ui";
  import type { EngineModKind, EngineProject } from "../../lib/bindings";
  import { engine, type EngineTarget } from "../../lib/stores/engine.svelte";

  interface Props {
    /** Opens a mod after the caller settled unsaved edits. */
    onselect: (target: EngineTarget) => void;
  }

  let { onselect }: Props = $props();

  const KIND: Record<EngineModKind, string> = {
    patch: "Patches",
    faction: "Faction",
    assets: "Assets",
    unmanaged: "No mod files",
  };

  let showLibrary = $state(false);
  const projects = $derived(engine.projects?.projects ?? []);
  const selected = $derived(engine.target);

  // A library mod being read keeps its list open, so it shows where it is.
  $effect(() => {
    if (selected?.kind === "library") showLibrary = true;
  });

  function checkBadge(p: EngineProject): { tone: "ok" | "warn" | "danger"; text: string } | null {
    const c = p.last_check;
    if (!c) return null;
    if (c.errors > 0) return { tone: "danger", text: `${c.errors} ${c.errors === 1 ? "error" : "errors"}` };
    if (c.warnings > 0) return { tone: "warn", text: `${c.warnings} ${c.warnings === 1 ? "warning" : "warnings"}` };
    return { tone: "ok", text: "Checked" };
  }
</script>

<nav class="projects" aria-label="Mods in the workspace">
  <div class="bar">
    <h2 class="qm-stencil head">Workspace</h2>
    <Button size="sm" variant="ghost" onclick={() => engine.openTemplates("new")}>New mod</Button>
  </div>
  {#if projects.length === 0}
    <p class="qm-muted empty">No mod folders in the workspace yet.</p>
  {:else}
    <ul class="list">
      {#each projects as p (p.folder)}
        {@const badge = checkBadge(p)}
        {@const current = selected?.kind === "workspace" && selected.folder === p.folder}
        <li>
          <button
            type="button"
            class={["item", { current }]}
            aria-current={current ? "true" : undefined}
            onclick={() => onselect({ kind: "workspace", folder: p.folder })}
          >
            <span class="name">{p.name}</span>
            <span class="meta qm-muted">
              {KIND[p.kind]}{#if p.version}<span class="version">{p.version}</span>{/if}
            </span>
            {#if badge}<Badge tone={badge.tone}>{badge.text}</Badge>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <div class="library">
    <button type="button" class="toggle" aria-expanded={showLibrary} onclick={() => (showLibrary = !showLibrary)}>
      <span class="qm-stencil">Library</span>
      <span class="qm-muted count">{engine.library.length}, read-only</span>
    </button>
    {#if showLibrary}
      <ul class="list" aria-label="Installed library mods">
        {#each engine.library as m (`${m.id}@${m.version}`)}
          {@const current = selected?.kind === "library" && selected.id === m.id}
          <li class="lib-row">
            <button
              type="button"
              class={["item", { current }]}
              aria-current={current ? "true" : undefined}
              onclick={() => onselect({ kind: "library", id: m.id, version: m.version, name: m.name })}
            >
              <span class="name">{m.name}</span>
              <span class="meta qm-muted"><span class="version">{m.version}</span></span>
            </button>
            {#if current}
              <Button size="sm" variant="ghost" aria-label={`Copy ${m.name} to the workspace`} onclick={() => engine.copyToWorkspace(m).catch(() => {})}>
                Copy to workspace
              </Button>
            {/if}
          </li>
        {:else}
          <li class="qm-muted empty">No mods are installed.</li>
        {/each}
      </ul>
    {/if}
  </div>
</nav>

<style>
  .projects {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-height: 0;
    overflow: auto;
    padding-right: var(--qm-space-1);
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--qm-space-2);
  }

  .head {
    font-size: var(--qm-text-13);
    color: var(--qm-text-muted);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 2px var(--qm-space-2);
    width: 100%;
    padding: var(--qm-space-2);
    border: 1px solid transparent;
    border-radius: var(--qm-radius-sm);
    background: none;
    color: var(--qm-text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .item:hover {
    background: var(--qm-raised);
  }

  .item.current {
    border-color: var(--qm-accent);
    background: color-mix(in srgb, var(--qm-accent) 10%, transparent);
  }

  .item :global(.badge) {
    grid-column: 2;
    grid-row: 1 / span 2;
  }

  .name {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    display: flex;
    gap: var(--qm-space-2);
    font-size: var(--qm-text-12);
  }

  .version {
    font-family: var(--qm-font-mono);
  }

  .empty {
    margin: 0;
    font-size: var(--qm-text-12);
  }

  .library {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-1);
    margin-top: var(--qm-space-3);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .toggle {
    display: flex;
    align-items: baseline;
    gap: var(--qm-space-2);
    padding: var(--qm-space-1) 0;
    border: 0;
    background: none;
    color: var(--qm-text-muted);
    font: inherit;
    font-size: var(--qm-text-13);
    cursor: pointer;
  }

  .count {
    font-size: var(--qm-text-12);
  }

  .lib-row {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  .lib-row :global(.btn) {
    margin: 0 0 var(--qm-space-1) var(--qm-space-1);
  }
</style>
