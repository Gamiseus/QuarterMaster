import type { FixAction } from "../../lib/modlist/fixes";
import { mods } from "../../lib/stores/mods.svelte";
import { profiles } from "../../lib/stores/profiles.svelte";
import { updates } from "../../lib/stores/updates.svelte";
import { modList } from "./modlist-controller.svelte";
import { openBrowse, openMod } from "./mods-view.svelte";

/**
 * Runs a one-click fix. Enable and disable go through the mod list controller, so they are undoable
 * and run through `loadOrder.whileChanging`. Rejects when the change was refused (already reported).
 */
export async function runFix(action: FixAction): Promise<void> {
  switch (action.kind) {
    case "enable":
    case "disable": {
      // The controller only knows the mods while the Mods screen has synced it; sync from the stores first.
      if (mods.list.status === "ready" && profiles.active) {
        modList.sync($state.snapshot(mods.list.data), $state.snapshot(profiles.active));
      }
      await modList.setEnabled(action.ids, action.kind === "enable");
      return;
    }
    case "update":
      await updates.install(action.modId);
      return;
    case "browse":
      openBrowse(action.text);
      return;
    case "open":
      openMod(action.modId, action.tab);
      return;
  }
}
