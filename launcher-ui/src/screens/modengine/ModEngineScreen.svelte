<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { Button, EmptyState, Screen, Skeleton } from "../../components/ui";
  import { app } from "../../lib/stores/app.svelte";
  import { game } from "../../lib/stores/game.svelte";
  import { engine, type EngineTarget } from "../../lib/stores/engine.svelte";
  import EditorPane from "./EditorPane.svelte";
  import FileTree from "./FileTree.svelte";
  import ProjectList from "./ProjectList.svelte";
  import ReleaseView from "./ReleaseView.svelte";
  import SpriteView from "./sprites/SpriteView.svelte";
  import StudioView from "./studio/StudioView.svelte";
  import TemplateDialog from "./TemplateDialog.svelte";
  import TestPanel from "./TestPanel.svelte";

  const workspace = $derived(engine.target?.kind === "workspace");
  const showTest = $derived(workspace && engine.showTest);
  const title = $derived(
    engine.target?.kind === "workspace"
      ? (engine.projects?.projects.find((p) => p.folder === engine.folder)?.name ?? engine.target.folder)
      : engine.target?.kind === "library"
        ? engine.target.name
        : null,
  );

  onMount(() => {
    app.leaveGuard = () => engine.confirmLeave();
    engine.load().catch(() => {});
  });

  onDestroy(() => {
    app.leaveGuard = null;
  });

  // What a test would run, read again when the mod or the mod set changes.
  $effect(() => {
    const folder = engine.folder;
    void engine.projects;
    if (folder) engine.loadTestPlan().catch(() => {});
  });

  async function test() {
    const plan = engine.testPlan;
    if (plan?.blocked) {
      engine.showTest = true;
      return;
    }
    await engine.startTest().catch(() => {});
  }

  async function select(target: EngineTarget) {
    const same =
      (target.kind === "workspace" && engine.target?.kind === "workspace" && target.folder === engine.target.folder) ||
      (target.kind === "library" && engine.target?.kind === "library" && target.id === engine.target.id);
    if (same) return;
    if (!(await engine.confirmLeave())) return;
    await engine.select(target).catch(() => {});
  }
</script>

<Screen title="Mod Engine" scroll={false}>
  {#snippet actions()}
    {#if workspace}
      <div class="views" role="group" aria-label="View">
        <button type="button" aria-pressed={engine.view === "files"} onclick={() => (engine.view = "files")}>Files</button>
        <button type="button" aria-pressed={engine.view === "studio"} onclick={() => (engine.view = "studio")}>Patch Studio</button>
        <button type="button" aria-pressed={engine.view === "sprites"} onclick={() => (engine.view = "sprites")}>Sprites</button>
        <button type="button" aria-pressed={engine.view === "release"} onclick={() => (engine.view = "release")}>Release</button>
      </div>
      <Button size="sm" variant="primary" disabled={game.isBusy} title={engine.testPlan?.message ?? undefined} onclick={test}>Test in game</Button>
      <Button size="sm" disabled={engine.linting} aria-busy={engine.linting} onclick={() => engine.runLint()}>
        {engine.linting ? "Checking..." : "Check mod"}
      </Button>
      <Button size="sm" variant="ghost" onclick={() => engine.openTemplates("add")}>Add example</Button>
      <Button size="sm" variant="ghost" onclick={() => engine.reveal().catch(() => {})}>Open folder</Button>
    {/if}
  {/snippet}

  {#if engine.projects === null}
    <div class="loading"><Skeleton lines={6} /></div>
  {:else if !engine.projects.workspace_set}
    <div class="center">
      <EmptyState
        title="Set a workspace mods folder"
        body="The Mod Engine edits the mods in your workspace mods folder, where you make and test your own mods. Choose that folder in Settings first."
      >
        {#snippet action()}
          <Button variant="primary" onclick={() => app.goTo("launcher", "settings")}>Open Settings</Button>
        {/snippet}
      </EmptyState>
    </div>
  {:else}
    <div class={["columns", { "with-test": showTest }]}>
      <ProjectList onselect={select} />
      {#if engine.target?.kind === "workspace" && engine.view === "studio"}
        <div class="studio-col">
          <StudioView folder={engine.target.folder} />
        </div>
      {:else if engine.target?.kind === "workspace" && engine.view === "sprites"}
        <div class="studio-col">
          <SpriteView folder={engine.target.folder} />
        </div>
      {:else if engine.target?.kind === "workspace" && engine.view === "release"}
        <div class="studio-col">
          <ReleaseView folder={engine.target.folder} />
        </div>
      {:else if engine.target}
        <div class="files-col">
          {#if title}<p class="mod-name" title={title}>{title}</p>{/if}
          <FileTree />
        </div>
        <EditorPane />
      {:else}
        <div class="pick">
          <EmptyState
            title="Pick a mod"
            body="Choose a mod from your workspace to edit it, or open a library mod to read it and copy it to the workspace. To start a new one, pick a template: each is a small working mod for one part of the modding kit."
          >
            {#snippet action()}
              <Button variant="primary" onclick={() => engine.openTemplates("new")}>New mod from a template</Button>
            {/snippet}
          </EmptyState>
        </div>
      {/if}
      {#if showTest}
        <div class="test-row"><TestPanel onclose={() => (engine.showTest = false)} /></div>
      {/if}
    </div>
    <TemplateDialog />
  {/if}
</Screen>

<style>
  .columns {
    display: grid;
    flex: 1;
    grid-template-columns: minmax(180px, 15rem) minmax(180px, 15rem) minmax(0, 1fr);
    /* One row the height of the screen: each column scrolls inside itself. */
    grid-template-rows: minmax(0, 1fr);
    gap: var(--qm-space-4);
    min-height: 0;
  }

  .columns.with-test {
    grid-template-rows: minmax(0, 1fr) minmax(0, auto);
  }

  .columns > :global(.projects) {
    grid-row: 1 / -1;
  }

  .test-row {
    grid-column: 2 / span 2;
    display: flex;
    min-height: 0;
    max-height: 18rem;
  }

  .studio-col {
    display: flex;
    flex-direction: column;
    grid-column: 2 / span 2;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    padding-left: var(--qm-space-4);
    border-left: 1px solid var(--qm-border);
  }

  .views {
    display: inline-flex;
    border: 1px solid var(--qm-border-strong);
    border-radius: var(--qm-radius-sm);
    overflow: hidden;
  }

  .views button {
    padding: var(--qm-space-1) var(--qm-space-3);
    border: 0;
    background: none;
    color: var(--qm-text-muted);
    font: inherit;
    font-size: var(--qm-text-13);
    cursor: pointer;
  }

  .views button[aria-pressed="true"] {
    background: color-mix(in srgb, var(--qm-accent) 18%, transparent);
    color: var(--qm-text);
  }

  .files-col {
    display: flex;
    flex-direction: column;
    gap: var(--qm-space-2);
    min-height: 0;
    padding-left: var(--qm-space-4);
    border-left: 1px solid var(--qm-border);
  }

  .mod-name {
    flex: none;
    margin: 0;
    overflow: hidden;
    font-weight: 700;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pick {
    grid-column: 2 / span 2;
    align-self: start;
    max-width: calc(var(--qm-detail-w) * 1.4);
    margin-top: var(--qm-space-6);
  }

  .center {
    max-width: calc(var(--qm-detail-w) * 1.6);
    margin: var(--qm-space-6) auto 0;
  }

  .loading {
    max-width: calc(var(--qm-detail-w) * 1.6);
  }
</style>
