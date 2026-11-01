<script lang="ts">
  import { Tooltip } from "bits-ui";
  import BlockingBanner from "./components/BlockingBanner.svelte";
  import GameRunningBanner from "./components/GameRunningBanner.svelte";
  import PreLaunchDialog from "./components/PreLaunchDialog.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import Toasts from "./components/Toasts.svelte";
  import { connectStores, loadStores } from "./lib/stores";
  import { app } from "./lib/stores/app.svelte";
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
    loadStores();
    return disconnect;
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

  main {
    flex: 1;
    min-width: 0;
    overflow: auto;
  }
</style>
