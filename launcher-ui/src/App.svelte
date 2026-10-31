<script lang="ts">
  import GameRunningBanner from "./components/GameRunningBanner.svelte";
  import Sidebar from "./components/Sidebar.svelte";
  import TitleBar from "./components/TitleBar.svelte";
  import Toasts from "./components/Toasts.svelte";
  import { connectStores, loadStores } from "./lib/stores";
  import { app } from "./lib/stores/app.svelte";
  import Diagnostics from "./pages/Diagnostics.svelte";
  import ModEngine from "./pages/ModEngine.svelte";
  import Mods from "./pages/Mods.svelte";
  import Play from "./pages/Play.svelte";
  import Saves from "./pages/Saves.svelte";
  import Settings from "./pages/Settings.svelte";

  const pages = { play: Play, mods: Mods, saves: Saves, diagnostics: Diagnostics, settings: Settings };
  const Current = $derived(app.mode === "engine" ? ModEngine : pages[app.page]);

  $effect(() => {
    const disconnect = connectStores();
    loadStores();
    return disconnect;
  });
</script>

<div class="frame">
  <TitleBar />
  <div class="body">
    <Sidebar />
    <main>
      <GameRunningBanner />
      <Current />
    </main>
  </div>
  <Toasts />
</div>

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
