import type { FactionPick, PreviewImpact } from "../bindings";

/** Side names as the game shows them. */
const SIDE: Record<string, string> = { allied: "Allies", axis: "Germans" };

/**
 * Sides whose faction the change switches (D73), in words. Starting with another faction resets
 * that side's unlocks in the save, so previews warn.
 */
export function factionChanges(impact: PreviewImpact | null, modName: (modId: string) => string): string[] {
  return (impact?.faction_changes ?? []).map((c) => {
    const side = SIDE[c.side] ?? c.side;
    const what = (p: FactionPick | null) => (p ? `${p.name} (from ${modName(p.mod_id)})` : `the game's own ${side}`);
    return `The ${side} would play as ${what(c.after)} instead of ${what(c.before)}`;
  });
}

/** A patch outcome after a previewed change, as a verb phrase. */
export const OUTCOME: Record<string, string> = {
  applied: "applies",
  zero_matches: "no longer finds its line",
  count_mismatch: "matches a different number of lines",
  skipped_disabled: "is turned off",
  skipped_by_policy: "is skipped",
  aborted: "aborts",
};

/** Patches whose outcome the change alters, in words; `skip` lists patches the change itself names. */
export function outcomeChanges(impact: PreviewImpact | null, name: (patchId: string) => string, skip: string[] = []): string[] {
  return (impact?.patch_changes ?? [])
    .filter((p) => p.after !== p.before && !skip.includes(p.patch_id))
    .map((p) => `${name(p.patch_id)} ${p.after ? (OUTCOME[p.after] ?? p.after) : "no longer runs on " + p.target_file}`);
}

/** How many blocks of code at shared lines run in a different place after the change. */
export function movedBlocks(impact: PreviewImpact | null): number {
  let n = 0;
  for (const s of impact?.stack_changes ?? []) {
    const after = s.after.map((b) => b.patch_id);
    const before = s.before.map((b) => b.patch_id).filter((id) => after.includes(id));
    const kept = after.filter((id) => before.includes(id));
    n += kept.filter((id, i) => before[i] !== id).length;
  }
  return n;
}

/** Labels of the user's fixes the change replaces (they have a status now and none after). */
export function replacedFixes(impact: PreviewImpact | null, labelOf: (fixId: string) => string | null): string[] {
  return (impact?.fix_status_changes ?? [])
    .filter((s) => s.before != null && s.after == null)
    .map((s) => labelOf(s.fix_id))
    .filter((l): l is string => l !== null);
}
