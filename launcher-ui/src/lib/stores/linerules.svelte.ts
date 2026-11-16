import { call } from "../api";
import { commands, type LineRulesDiff } from "../bindings";
import { loadOrder } from "./loadorder.svelte";
import { profiles } from "./profiles.svelte";

/** The line rules notice's key in `ModProfile.notices_seen` (D51). */
export const NOTICE_LINE_RULES = "stacking_v2";

/**
 * The newer line rules (D50, D51) for a profile from before them. The background simulation
 * compares both rules once; the notice shows only when the newer ones change this profile's
 * output and the profile has no answer yet. Nothing changes until the user picks.
 */
class LineRulesStore {
  /** What the newer rules change in the active profile, or null when there is nothing to ask. */
  get change(): LineRulesDiff | null {
    const p = profiles.active;
    const summary = profiles.activeSummary;
    if (!p || !summary || summary.is_builtin || p.id !== summary.id) return null;
    if (p.line_rules_v2 !== undefined && p.line_rules_v2 !== null) return null;
    const sim = loadOrder.simulation;
    const c = sim.status === "ready" ? (sim.data.line_rules_change ?? null) : null;
    // Only a change in the game text asks; stack and rung lines are the Review detail.
    if (!c || c.files.length === 0) return null;
    return c;
  }

  get showNotice(): boolean {
    return this.change !== null;
  }

  /** How many places change, for the notice ("2 places", or files when no place is listed). */
  get countText(): string {
    const c = this.change;
    if (!c) return "";
    const places = c.stacks.length + c.rungs.length;
    if (places > 0) return places === 1 ? "1 place" : `${places} places`;
    return c.files.length === 1 ? "1 file" : `${c.files.length} files`;
  }

  /** The answer: true uses the newer rules, false keeps the older ones. The notice counts as seen. */
  async set(on: boolean): Promise<void> {
    await call(commands.lineRulesSet(on));
    const p = profiles.active;
    if (!p) return;
    const seen = [...new Set([...(p.notices_seen ?? []), NOTICE_LINE_RULES])];
    profiles.active = { ...p, line_rules_v2: on, notices_seen: seen };
  }
}

export const lineRules = new LineRulesStore();
