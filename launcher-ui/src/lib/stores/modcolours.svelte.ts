import { assignModColours } from "../patches/colors";
import { app } from "./app.svelte";
import { mods } from "./mods.svelte";

/**
 * The colour number (1 to 8) of each mod, the same in every view: worked out over all installed mods
 * (plus `extra`, for ids that are not installed), with the colours picked in a mod's Overview on top.
 */
export function modColours(extra: string[] = []): Map<string, number> {
  const installed = mods.list.status === "ready" ? mods.list.data.map((m) => m.id) : [];
  const map = assignModColours([...installed, ...extra]);
  for (const [id, c] of Object.entries(app.settings?.mod_colors ?? {})) {
    if (c) map.set(id, c);
  }
  return map;
}
