import type { SaveProfile } from "../bindings";

export interface SlotTag {
  label: "Built in" | "Vanilla" | "Modded" | "Edited" | "Locked";
  tone: "neutral" | "ok" | "warn" | "info";
}

/**
 * The badges for a slot (docs/specs/save-tags.md): Vanilla until it has been played with mods,
 * then Modded for good; Edited (save editor, later); Locked when locked as vanilla.
 */
export function slotTags(p: SaveProfile): SlotTag[] {
  const tags = p.tags ?? { modded: false, edited: false };
  const out: SlotTag[] = [];
  if (p.builtin) out.push({ label: "Built in", tone: "neutral" });
  out.push(tags.modded ? { label: "Modded", tone: "warn" } : { label: "Vanilla", tone: "ok" });
  if (tags.edited) out.push({ label: "Edited", tone: "warn" });
  if (p.locked_vanilla) out.push({ label: "Locked", tone: "info" });
  return out;
}
