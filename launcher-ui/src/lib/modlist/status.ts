import type { ModPatchStatus, ModSummary, SimulationOverview, UpdatesReport } from "../bindings";

export type ModStatus = {
  patches: number;
  broken: number;
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
      broken: states ? states.filter((p) => p.state === "broken").length : 0,
      conflicts: conflicts.get(mod.id) ?? 0,
      updateAvailable: updatable.has(mod.id),
    });
  }
  return result;
}
