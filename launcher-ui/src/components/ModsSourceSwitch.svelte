<script lang="ts">
  import { call } from "../lib/api";
  import { commands, type ModsSource } from "../lib/bindings";
  import { app } from "../lib/stores/app.svelte";
  import { game } from "../lib/stores/game.svelte";
  import { installs } from "../lib/stores/installs.svelte";
  import { ConfirmDialog } from "./ui";

  // Shown only once a workspace mods folder is set: mod makers switch between the mods they play with
  // and the mods they are making. Each side has its own mods and profiles.
  const shown = $derived(Boolean(app.settings?.workspace_mods_dir));
  const current = $derived<ModsSource>(app.settings?.workspace_mode ? "workspace" : "library");
  const blocked = $derived(
    !app.startupDone ? "Available once startup has finished." : game.isBusy ? "Close the game to switch." : installs.blockReason(),
  );
  let target = $state<ModsSource>("workspace");
  let open = $state(false);

  function ask(to: ModsSource) {
    if (to === current || blocked !== null) return;
    target = to;
    open = true;
  }

  async function go() {
    // The launcher starts again on the other set, so nothing more happens here.
    await call(commands.modsSourceSet(target));
    await app.load();
  }
</script>

{#if shown}
  <div class="source" role="group" aria-label="Mods in use" data-testid="mods-source">
    {#each [{ value: "library", label: "Library" }, { value: "workspace", label: "Workspace" }] as const as s (s.value)}
      <button
        type="button"
        class={{ on: current === s.value }}
        aria-pressed={current === s.value}
        disabled={blocked !== null && current !== s.value}
        title={current === s.value
          ? s.value === "library"
            ? "Using the mods installed in the library."
            : "Using the mods in your workspace folder."
          : (blocked ?? (s.value === "library" ? "Switch to the mods installed in the library." : "Switch to the mods in your workspace folder."))}
        onclick={() => ask(s.value)}
      >
        {s.label}
      </button>
    {/each}
  </div>
{/if}

<ConfirmDialog
  bind:open
  title={target === "workspace" ? "Switch to your workspace mods?" : "Switch to the library mods?"}
  body={target === "workspace"
    ? "The launcher starts again and shows only the mods in your workspace folder, with their own profiles. The game runs those folders as they are, so an edit is in the next launch. Your library mods and profiles are kept and come back when you switch back."
    : "The launcher starts again and shows your installed mods and their profiles. Your workspace profiles are kept and come back when you switch back."}
  confirmLabel="Switch and restart"
  danger={false}
  onconfirm={go}
/>

<style>
  .source {
    display: flex;
    flex: none;
    border: 1px solid var(--qm-border);
    border-radius: var(--qm-radius-sm);
    overflow: hidden;
  }

  button {
    padding: var(--qm-space-1) var(--qm-space-2);
    border: 0;
    background: transparent;
    color: var(--qm-text-muted);
    font-size: var(--qm-text-12);
    transition: background var(--qm-dur-fast) var(--qm-ease);
  }

  button:hover:not(:disabled):not(.on) {
    background: var(--qm-raised);
    color: var(--qm-text);
  }

  button.on {
    background: color-mix(in srgb, var(--qm-accent) 18%, transparent);
    color: var(--qm-text);
    font-weight: 600;
  }

  button:disabled {
    opacity: 0.5;
  }

  button:focus-visible {
    outline: none;
    box-shadow: inset var(--qm-focus-ring);
  }
</style>
