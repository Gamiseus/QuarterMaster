<script lang="ts">
  import { Button, Panel, StencilHeading } from "../../components/ui";
  import type { PathKey } from "../../lib/settings/form";
  import { app } from "../../lib/stores/app.svelte";
  import { installs } from "../../lib/stores/installs.svelte";
  import PathField from "./PathField.svelte";

  /** The game and save folders can be found automatically; a modder's workspace never is. */
  const DETECTED: { key: Exclude<PathKey, "workspace_mods_dir">; label: string; kind: "exe" | "dir" }[] = [
    { key: "game_exe_path", label: "Game executable (mnb.exe)", kind: "exe" },
    { key: "save_dir_path", label: "Save folder", kind: "dir" },
  ];

  let syncing = $state(false);
  let dumping = $state(false);
  const workspace = $derived(app.settings?.workspace_mods_dir ?? null);
  /** The launcher is running on the workspace's mods: the folder cannot change under it. */
  const workspaceMode = $derived(app.settings?.workspace_mode ?? false);

  async function dump() {
    dumping = true;
    try {
      await app.dumpGame();
    } catch {
      // call() or jobs.track already reported it.
    } finally {
      dumping = false;
    }
  }
  const syncBlocked = $derived(installs.blockReason());

  // A game change while the game runs is refused by the backend; call() shows the banner.
  function save(key: PathKey, path: string) {
    app.updateSettings({ [key]: path }).catch(() => {});
  }

  async function detect(key: PathKey): Promise<string | null> {
    return (await app.autoDetect())[key];
  }

  async function sync() {
    syncing = true;
    try {
      await app.syncWorkspace();
    } catch {
      // call() or jobs.track already reported it.
    } finally {
      syncing = false;
    }
  }
</script>

<Panel class="settings-card">
  <StencilHeading level={2}>Paths</StencilHeading>
  <div class="paths">
    {#each DETECTED as f (f.key)}
      <PathField
        label={f.label}
        kind={f.kind}
        value={app.settings?.[f.key] ?? null}
        disabled={!app.startupDone}
        onchange={(p) => save(f.key, p)}
        detect={() => detect(f.key)}
      />
    {/each}
    <PathField
      label="Workspace mods folder"
      kind="dir"
      value={workspace}
      disabled={!app.startupDone || workspaceMode}
      onchange={(p) => save("workspace_mods_dir", p)}
    >
      {#snippet actions()}
        <Button
          size="sm"
          variant="ghost"
          disabled={!app.startupDone || workspace === null || workspaceMode}
          onclick={() => app.updateSettings({ clear_workspace_mods_dir: true }).catch(() => {})}
        >
          Clear
        </Button>
        <Button size="sm" disabled={workspace === null || syncBlocked !== null || syncing} onclick={sync}>Sync now</Button>
        <Button size="sm" variant="ghost" disabled={workspace === null} aria-label="Open the workspace mods folder" onclick={() => app.reveal("workspace").catch(() => {})}>Open</Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={workspace === null || dumping || !app.startupDone}
          title="Writes every file of the game (code, images, sounds, fonts) into _game_dump in this folder, to read while you make mods."
          onclick={dump}
        >
          Write the game's files here
        </Button>
      {/snippet}
    </PathField>
  </div>
  <p class="qm-muted note">
    {#if workspace === null}
      Optional, for mod making: a folder where each subfolder is a mod you are working on. Pick a folder (a new one
      is made if it is not there yet) to get the Library and Workspace switch beside the profile picker.
    {:else if workspaceMode}
      The launcher is using this folder's mods as they are, with their own profiles. Sync now reads the folder again.
      Switch back to Library beside the profile picker to change or clear the folder.
    {:else}
      Your own mods folder for mod making. Sync now copies its mods into the library. To run the folder's mods as
      they are, apart from your installed mods, switch to Workspace beside the profile picker.
    {/if}
  </p>
</Panel>

<style>
  /* A hairline between path rows, like the other settings rows. */
  .paths :global(.field + .field) {
    margin-top: var(--qm-space-3);
    padding-top: var(--qm-space-3);
    border-top: 1px solid var(--qm-border);
  }

  .note {
    margin: 0;
    font-size: var(--qm-text-12);
  }
</style>
