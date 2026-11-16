<script lang="ts">
  import { Screen, ScrollRegion, Tabs } from "../../components/ui";
  import { onFileDrop } from "../../lib/dragdrop";
  import BrowseTab from "./browse/BrowseTab.svelte";
  import ModpackDialog from "../profiles/ModpackDialog.svelte";
  import AddModDialog from "./install/AddModDialog.svelte";
  import InstalledTab from "./InstalledTab.svelte";
  import PatchesTab from "./patches/PatchesTab.svelte";
  import { modsView } from "./mods-view.svelte";

  const TABS = [
    { value: "installed", label: "Installed" },
    { value: "browse", label: "Browse" },
    { value: "patches", label: "Patches" },
  ];

  let addOpen = $state(false);
  let dropped = $state<{ paths: string[] } | null>(null);
  let dragging = $state(false);
  let modpackPath = $state<string | null>(null);
  let modpackOpen = $state(false);

  function addMod() {
    dropped = null;
    addOpen = true;
  }

  // Files dropped anywhere on the screen open the Add mod dialog with every path. Inspecting is read-only;
  // the dialog's Install button waits for startup.
  $effect(() =>
    onFileDrop((e) => {
      dragging = e.type === "enter";
      if (e.type !== "drop" || e.paths.length === 0) return;
      const isPack = (p: string) => p.toLowerCase().endsWith(".qmpack");
      const mods = e.paths.filter((p) => !isPack(p));
      // A modpack has its own dialog; it opens when nothing else was dropped with it.
      if (mods.length === 0) {
        modpackPath = e.paths[0];
        modpackOpen = true;
        return;
      }
      dropped = { paths: mods };
      addOpen = true;
    }),
  );
</script>

<!-- Each tab lays out its own scroll regions, so the screen body does not scroll. -->
<Screen title="Mods" scroll={false}>
  <Tabs
    bind:value={() => modsView.tab, (v) => (modsView.tab = v === "browse" || v === "patches" ? v : "installed")}
    tabs={TABS}
    fill
  >
    {#snippet content(value)}
      {#if value === "browse"}
        <ScrollRegion class="browse-scroll"><BrowseTab /></ScrollRegion>
      {:else if value === "patches"}
        <PatchesTab />
      {:else}
        <InstalledTab onaddmod={addMod} />
      {/if}
    {/snippet}
  </Tabs>
</Screen>

<AddModDialog bind:open={addOpen} {dropped} {dragging} />
<ModpackDialog bind:open={modpackOpen} path={modpackPath} />

<style>
  :global(.browse-scroll) {
    padding-top: var(--qm-space-4);
  }
</style>
