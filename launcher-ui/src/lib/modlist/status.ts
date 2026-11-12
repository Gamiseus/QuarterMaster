import type { FactionHeader, ModKind, ModPatchStatus, ModSummary, SimulationOverview, UpdatesReport } from "../bindings";

export type ModStatus = {
  patches: number;
  /** Broken patches; null while the mod's patch states are unknown (not loaded yet, or the load failed). */
  broken: number | null;
  conflicts: number;
  updateAvailable: boolean;
};

export function assembleStatus(
  mods: ModSummary[],
  sim: SimulationOverview | null,
  updates: UpdatesReport | null,
  patchStates?: Map<string, ModPatchStatus[]>,
): Map<string, ModStatus> {
  const conflicts = new Map<string, number>();
  for (const c of sim?.conflicts ?? []) {
    // Info notes are not conflicts.
    if (c.severity === "info") continue;
    conflicts.set(c.mod_a_id, (conflicts.get(c.mod_a_id) ?? 0) + 1);
    if (c.mod_b_id !== null && c.mod_b_id !== c.mod_a_id) {
      conflicts.set(c.mod_b_id, (conflicts.get(c.mod_b_id) ?? 0) + 1);
    }
  }
  const updatable = new Set(
    (updates?.mods ?? []).filter((u) => u.update_available).map((u) => u.mod_id),
  );
  const result = new Map<string, ModStatus>();
  for (const mod of mods) {
    const states = patchStates?.get(mod.id);
    result.set(mod.id, {
      patches: mod.patch_count,
      broken: states ? states.filter((p) => p.state === "broken").length : mod.patch_count === 0 ? 0 : null,
      conflicts: conflicts.get(mod.id) ?? 0,
      updateAvailable: updatable.has(mod.id),
    });
  }
  return result;
}

export type BadgeTone = "ok" | "warn" | "danger" | "info" | "neutral" | "accent";

/** "the Allies" or "the Germans": the side a faction mod takes over. */
export function factionSide(faction: FactionHeader): string {
  return faction.replaces === "allied" ? "the Allies" : "the Germans";
}

/** The status badges a mod row shows, most important first. */
export function rowBadges(
  status: ModStatus,
  enabled: boolean,
  kind: ModKind = "managed",
  faction: FactionHeader | null = null,
): { tone: BadgeTone; text: string }[] {
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  // An asset-only mod's one patch is the autoloader's, and a faction mod's patches are
  // QuarterMaster's own; name what the mod is instead.
  const patches =
    kind === "assets"
      ? "Assets only"
      : kind === "faction" && faction
        ? `Replaces ${factionSide(faction)}`
        : plural(status.patches, "patch", "patches");
  const out: { tone: BadgeTone; text: string }[] = [];
  if (!enabled) {
    // Nothing of a disabled mod is applied: no health claim either way.
    out.push({ tone: "neutral", text: patches });
  } else {
    if (status.broken !== null && status.broken > 0) out.push({ tone: "danger", text: `${status.broken} broken` });
    if (status.conflicts > 0) out.push({ tone: "warn", text: plural(status.conflicts, "conflict", "conflicts") });
    // Unknown patch states make no health claim until they load.
    if (status.broken === null && status.conflicts === 0) out.push({ tone: "neutral", text: patches });
    else if (status.broken === 0 && status.conflicts === 0) out.push({ tone: "ok", text: patches });
  }
  if (status.updateAvailable) out.push({ tone: "accent", text: "Update" });
  return out;
}

/** Rows show at most this many badges; the rest fold into a "+N" badge. */
export const MAX_ROW_BADGES = 2;

/** From this text size up, rows show one badge (plus "+N") so the mod name keeps its room. */
export const ONE_BADGE_SCALE = 1.375;

export function maxRowBadges(textScale: number): number {
  return textScale >= ONE_BADGE_SCALE ? 1 : MAX_ROW_BADGES;
}
