<script lang="ts">
  import { Tooltip } from "bits-ui";
  import { untrack } from "svelte";
  import BlockingBanner from "./components/BlockingBanner.svelte";
  import GameRunningBanner from "./components/GameRunningBanner.svelte";
  import PreLaunchDialog from "./components/PreLaunchDialog.svelte";
  import ConflictResolver from "./screens/mods/patches/ConflictResolver.svelte";
  import JournalUndoDialog from "./screens/mods/patches/JournalUndoDialog.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import Toasts from "./components/Toasts.svelte";
  import { connectStores, loadStores } from "./lib/stores";
  import { app } from "./lib/stores/app.svelte";
  import { updates } from "./lib/stores/updates.svelte";
  import { openUpdates } from "./screens/mods/mods-view.svelte";
  import OnboardingDialog from "./screens/onboarding/OnboardingDialog.svelte";
  import LegacyDataDialog from "./screens/setup/LegacyDataDialog.svelte";
  import ManualInstallDialog from "./screens/setup/ManualInstallDialog.svelte";
  import ReportIssueDialog from "./components/ReportIssueDialog.svelte";
  import { setup } from "./lib/stores/setup.svelte";
  import DiagnosticsScreen from "./screens/diagnostics/DiagnosticsScreen.svelte";
  import ModEngineScreen from "./screens/modengine/ModEngineScreen.svelte";
  import ModsScreen from "./screens/mods/ModsScreen.svelte";
  import SavesScreen from "./screens/saves/SavesScreen.svelte";
  import SettingsScreen from "./screens/settings/SettingsScreen.svelte";

  const screens = {
    mods: ModsScreen,
    saves: SavesScreen,
    diagnostics: DiagnosticsScreen,
    settings: SettingsScreen,
  };
  const Current = $derived(app.mode === "engine" ? ModEngineScreen : screens[app.page]);

  $effect(() => {
    const disconnect = connectStores();
    updates.onReview = openUpdates;
    loadStores();
    return disconnect;
  });

  // Updates found before startup finished (or before settings loaded) are handled once both are ready.
  $effect(() => {
    if (app.startupDone && app.settings) untrack(() => updates.announce());
  });

  // Once per start: data an earlier QuarterMaster left in AppData (asked before the first-run setup).
  $effect(() => {
    if (app.startupDone) untrack(() => void setup.checkLegacy());
  });

  // A manual install in the game folder, looked for once the game is set (and again when it changes).
  const gamePath = $derived(app.startupDone && app.settings?.first_run_completed ? (app.settings.game_exe_path ?? null) : null);
  $effect(() => {
    if (gamePath !== null) untrack(() => void setup.checkManual());
  });

  // Periodic checks while automatic checking is on; a derived flag, so other settings changes keep the timer.
  const autoCheck = $derived(app.startupDone && (app.settings?.auto_check_updates ?? false));
  $effect(() => {
    if (autoCheck) return untrack(() => updates.schedule());
  });
</script>

<Tooltip.Provider delayDuration={400} skipDelayDuration={200}>
  <div class="frame">
    <TitleBar />
    <div class="body">
      <Sidebar />
      <main>
        <BlockingBanner />
        <GameRunningBanner />
        <Current />
      </main>
    </div>
    <PreLaunchDialog />
    <ConflictResolver />
    <JournalUndoDialog />
    <LegacyDataDialog />
    <OnboardingDialog />
    <ManualInstallDialog />
    <ReportIssueDialog />
    <Toasts />
  </div>
</Tooltip.Provider>

<style>
  .frame {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  .body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  /* The window is the frame: main never scrolls; each screen fills it and scrolls inside its own regions. */
  main {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
</style>
