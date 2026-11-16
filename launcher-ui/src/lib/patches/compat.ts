// Compatibility mods in the UI (§11.6): who changed a patch, what a compat mod changes, and the
// export dialog's text.
import type { CompatChangesView, CompatEquivalence, CompatRequirement, OverrideReport, PatchExecutionRecord } from "../bindings";

const lower = (s: string) => s.toLowerCase();

/** Whether an override's target names the patch `patchId` of `modId` (`mod:id` or the id itself). */
export function namesPatch(target: string, modId: string, patchId: string): boolean {
  const t = lower(target);
  return t === lower(patchId) || t === lower(`${modId}:${patchId}`);
}

/** The mod a patch name belongs to: the part before the first `:`, if any. */
export function modOfName(name: string): string | null {
  const i = name.indexOf(":");
  return i > 0 ? name.slice(0, i) : null;
}

/**
 * The compatibility mods that changed a patch: the engine's `changed_by` (its record, or its row
 * in a mod's patches), then overrides whose target names the patch as written.
 */
export function changedBy(overrides: OverrideReport[], modId: string, patchId: string, record?: Pick<PatchExecutionRecord, "changed_by"> | null): string[] {
  const out = new Set<string>(record?.changed_by ?? []);
  for (const o of overrides) if (o.status === "applied" && namesPatch(o.target, modId, patchId)) out.add(o.from_mod);
  return [...out];
}

/** The patches whose change this patch's code is placed in or next to (resolved rungs). */
export function tiedTo(record?: PatchExecutionRecord | null): string[] {
  const out = new Set<string>();
  for (const s of record?.anchoring?.sites ?? []) {
    if (s.rung !== null && s.anchored_on && s.anchored_on !== "text") out.add(s.anchored_on);
  }
  return [...out];
}

/** The mods whose patches a compat mod changes, in report order. */
export function changedMods(view: CompatChangesView): string[] {
  return [...new Set(view.applied.map((o) => modOfName(o.target)).filter((m): m is string => m !== null))];
}

/** Overrides that QuarterMaster ignores for a fault of the mod (warn), against those that wait or lost to another change. */
const IGNORED = new Set(["rejected_not_required", "invalid", "target_missing", "suspended_drift"]);

/** The sentences of the warn Callout: ignored overrides, then rungs that are never used. */
export function ignoredLines(view: CompatChangesView): string[] {
  const end = (s: string) => (/[.!?]$/.test(s) ? s : `${s}.`);
  return [...view.rejected.filter((o) => IGNORED.has(o.status)).map((o) => end(o.detail)), ...view.rejected_rungs.map((r) => end(r.detail))];
}

/** Overrides not used now for a reason outside the mod (a mod is off, a later change wins). */
export function idleLines(view: CompatChangesView): string[] {
  return view.rejected.filter((o) => !IGNORED.has(o.status)).map((o) => (/[.!?]$/.test(o.detail) ? o.detail : `${o.detail}.`));
}

/** "GCore 1.4.0 or newer (it places code next to GCore's change)". */
export function requirementText(r: CompatRequirement): string {
  const version = r.version && r.range !== "*" ? ` ${r.version} or newer` : "";
  return `${r.name}${version}${r.reasons.length > 0 ? ` (${r.reasons.join("; ")})` : ""}`;
}

/** The equivalence badge (§11.6). */
export function equivalenceBadge(e: CompatEquivalence): { tone: "ok" | "danger" | "neutral"; text: string } {
  switch (e.kind) {
    case "equivalent":
      return { tone: "ok", text: "Same game code as now" };
    case "differs":
      return { tone: "danger", text: "Changes the game code" };
    case "not_checked":
      return { tone: "neutral", text: "Not checked" };
  }
}
