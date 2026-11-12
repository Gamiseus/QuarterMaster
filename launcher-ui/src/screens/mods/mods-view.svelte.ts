import type { FilterChip } from "../../lib/modlist/filter";
import { app } from "../../lib/stores/app.svelte";
import { catalog } from "../../lib/stores/catalog.svelte";
import { patchView } from "../../lib/stores/patchview.svelte";
import { selection } from "../../lib/stores/selection.svelte";

/** Which Mods tab and detail tab are showing; other screens jump here through `openBrowse` / `openMod`. */
export const modsView = $state({
  tab: "installed" as "installed" | "browse" | "patches",
  detailTab: "overview",
  /** Filter chips the Installed tab applies once on its next render, then clears. */
  showChips: null as FilterChip[] | null,
});

function showMods(): void {
  app.mode = "launcher";
  app.page = "mods";
}

/** Opens the Browse tab, optionally searching for `text`. */
export function openBrowse(text?: string): void {
  showMods();
  modsView.tab = "browse";
  if (text !== undefined) catalog.setText(text, true);
}

/** Opens the Installed tab with `id` selected and its detail panel on `tab`. */
export function openMod(id: string, tab: string): void {
  showMods();
  modsView.tab = "installed";
  selection.selected = new Set([id]);
  selection.focused = id;
  modsView.detailTab = tab;
}

/** Opens the Installed tab showing only mods with updates. */
export function openUpdates(): void {
  showMods();
  modsView.tab = "installed";
  modsView.showChips = ["updates"];
}

/** Opens the Patches tab with `patchId` and the first file it changes selected. */
export function openPatch(patchId: string): void {
  showMods();
  modsView.tab = "patches";
  void patchView.openPatch(patchId);
}

/** Opens the Patches tab as it was. */
export function openPatches(): void {
  showMods();
  modsView.tab = "patches";
}

/** Opens the Patches tab on `file`'s patch order. */
export function openPatchOrder(file: string): void {
  showMods();
  modsView.tab = "patches";
  patchView.selectFile(file);
  patchView.view = "order";
  patchView.orderScope = "file";
}

/** Opens the Patches tab on the Fixes view. */
export function openFixes(): void {
  showMods();
  modsView.tab = "patches";
  patchView.view = "fixes";
}
