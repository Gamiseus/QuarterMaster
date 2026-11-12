// A tiny patch engine for the mock: the fixture mods' patches applied to made-up game files, so
// file diffs, records, file stats, patch details, conflicts and previews all follow the profile
// (enabled mods, disabled patches, patch order rules and adjusted copies).
import type {
  AnchorCandidate, AnchorReport, ASite, AttributedCodeLine, ConsumerInfo, ConsumerKind, DetectedConflict, EdgeCheck, FallbackNotice,
  FileDiff, FileStat, FixDraft, GluePatchEntry, ModProfile, OrderRule, PatchChange, PatchDetail, PatchExecutionRecord,
  PatchOutcomeStatus, PatchSnapshot, PreviewChanges, PreviewImpact, PreviewResult, ProfileFix, RegionOption, RegionOptions,
  RegionRef, RegionSite, RelationWarning, Rung, SharedStack, StackBlock, StackChange, StackMode, StackMove, StackReason, StackSide,
  InvalidRung, OverrideReport, SiteKey, ApplyAs, HunkKey, RunsAsVerdict, ConflictResolutionSuggestion, FailureGroup, InferredEdge,
  FactionChange, FactionPick,
} from "../bindings";
import { liveConflicts, loadOrderFor, mods, patchesByMod, patchesFor } from "./fixtures";
import { state } from "./state";

/** Declared relations per mod, as mod.json has them (the mod detail shows the same). */
export const MOD_RELATIONS: Record<string, { requires: string[]; load_after: string[]; load_before: string[] }> = Object.fromEntries(
  ["gcore", "gmenu", "better-medics", "tactical-hud", "vehicle-pack", "ai-tweaks"].map((id) => [
    id,
    id === "gcore" ? { requires: [], load_after: [], load_before: [] } : { requires: ["gcore"], load_after: ["gcore"], load_before: [] },
  ]),
);

/** `deps(m)` among the enabled mods (D4): requires, load_after and others' load_before, transitively. */
function dependsOn(enabled: string[]): (a: string, b: string) => boolean {
  const direct = new Map<string, Set<string>>(enabled.map((m) => [m, new Set<string>()]));
  for (const m of enabled) {
    const r = MOD_RELATIONS[m];
    if (!r) continue;
    for (const d of [...r.requires, ...r.load_after]) if (direct.has(d) && d !== m) direct.get(m)!.add(d);
    for (const d of r.load_before) if (direct.has(d) && d !== m) direct.get(d)!.add(m);
  }
  const closure = new Map<string, Set<string>>();
  for (const m of enabled) {
    const seen = new Set<string>();
    const todo = [...(direct.get(m) ?? [])];
    while (todo.length > 0) {
      const d = todo.pop()!;
      if (seen.has(d)) continue;
      seen.add(d);
      todo.push(...(direct.get(d) ?? []));
    }
    closure.set(m, seen);
  }
  // Mutually dependent pairs count as unrelated.
  return (a, b) => a !== b && (closure.get(a)?.has(b) ?? false) && !(closure.get(b)?.has(a) ?? false);
}

/** `item` is a chain item, or for `rule` an index into the rules given. */
type Slot = { slot: number; legacy: number; reason: { kind: "below" | "above" | "load_order" | "unsatisfiable" | "rule"; item: number; above?: number } | null };

/** The slot rule of plan 2e §5.3, as `qm-core/src/patch/stack.rs` has it. `owners` are top to bottom. */
export function stackSlot(mode: StackMode, side: StackSide, me: string, owners: string[], deps: (a: string, b: string) => boolean): Slot {
  const k = owners.length;
  const legacy = side === "after" ? 0 : k;
  const at = (slot: number, reason: Slot["reason"]): Slot => ({ slot, legacy, reason: slot === legacy ? null : reason });
  if (mode === "legacy") return at(legacy, null);
  if (mode === "load_order") {
    // Last, but never across this mod's own items (D5).
    const own = owners.indexOf(me);
    return at(side === "after" && own !== -1 ? own : k, { kind: "load_order", item: -1 });
  }
  let loItem = -1;
  owners.forEach((o, i) => {
    if (o !== me && deps(me, o)) loItem = i;
  });
  const hiItem = owners.findIndex((o) => o !== me && deps(o, me));
  const lo = loItem + 1;
  const hi = hiItem === -1 ? k : hiItem;
  if (lo > hi) return { slot: lo, legacy, reason: { kind: "unsatisfiable", item: lo - 1, above: hi } };
  const slot = Math.min(Math.max(legacy, lo), hi);
  return at(slot, slot > legacy ? { kind: "below", item: loItem } : { kind: "above", item: hiItem });
}

/** One stack rule as the slot rule reads it: below or above chain item `item`; `rank` 0 is a profile rule. */
export type SlotRule = { item: number; below: boolean; rank: number };

/**
 * `stackSlot` with stack rules (D55, D56), as `stack_slot_with` has it: the rules act last, in
 * every mode, and the slot moves the least it must. Of two rules that cannot both hold, the
 * stronger (lower rank, then earlier) stays; `overridden` lists the others (indexes into `rules`).
 */
export function stackSlotRuled(
  mode: StackMode, side: StackSide, me: string, owners: string[], deps: (a: string, b: string) => boolean, rules: SlotRule[],
): { slot: Slot; overridden: number[] } {
  const base = stackSlot(mode, side, me, owners, deps);
  const k = owners.length;
  const order = rules.map((_, i) => i).sort((a, b) => rules[a].rank - rules[b].rank || a - b);
  let [lo, hi] = [0, k];
  const held: [number, number, number][] = [];
  const overridden: number[] = [];
  for (const i of order) {
    const r = rules[i];
    const [rlo, rhi] = r.below ? [r.item + 1, k] : [0, r.item];
    const [nlo, nhi] = [Math.max(lo, rlo), Math.min(hi, rhi)];
    if (nlo <= nhi && r.item < k) {
      [lo, hi] = [nlo, nhi];
      held.push([i, rlo, rhi]);
    } else overridden.push(i);
  }
  if (held.length === 0) return { slot: base, overridden };
  const slot = Math.min(Math.max(base.slot, lo), hi);
  let reason: Slot["reason"];
  if (slot !== base.slot) {
    const bound = held.find(([, rlo, rhi]) => (slot === lo && rlo === lo) || (slot === hi && rhi === hi));
    reason = { kind: "rule", item: (bound ?? held[0])[0] };
  } else reason = base.reason ?? { kind: "rule", item: held[0][0] };
  return { slot: { slot, legacy: base.legacy, reason }, overridden };
}

type MockPatch = {
  id: string;
  mod: string;
  name: string;
  file: string;
  /** One line, or several (`\n`): a window of lines, like the engine's multi-line patterns. */
  anchor: string;
  position: "after" | "before" | "at";
  payload: string[];
  requires?: string[];
  /** Set for an adjusted copy: the patch it stands in for. */
  replaces?: string;
  /** An adjusted copy that runs as its base mod at the base's place (D58); other copies run last. */
  atBase?: boolean;
  /** The line matches only its own text: `*` and `?` are not wildcards (D59). */
  exact?: boolean;
};

/** Whether a line matches a patch's line, like the engine: indentation aside, with `*` and `?` as wildcards unless the
 *  patch matches exactly (D59). */
export function lineMatcher(anchor: string, exact = false): (line: string) => boolean {
  const want = anchor.trim();
  if (exact || !/[*?]/.test(want)) return (line) => line.trim() === want;
  const re = new RegExp(`^${want.replace(/[.+^${}()|[\]\\]/g, "\\$&").replaceAll("*", ".*").replaceAll("?", ".")}$`, "s");
  return (line) => re.test(line.trim());
}

const lines = (text: string) => text.split("\n");

/** `main.lua` is 10,000 lines, like the real one is long: the view must stay quick on it. */
const MAIN = Array.from({ length: 10_000 }, (_, i) =>
  i === 119 ? "function love.load()" : i === 120 ? "  initGame()" : `-- main ${i + 1}`,
);

const ORIGINALS: Record<string, string[]> = {
  "main.lua": MAIN,
  "scripts/ui/menu.lua": lines("local menu = {}\n\nfunction menu.build()\n  addEntry(\"Play\")\n  addEntry(\"Options\")\nend\n\nreturn menu"),
  "scripts/ui/keys.lua": lines("local keys = {}\nkeys.pause = \"p\"\nreturn keys"),
  "scripts/unit.lua": lines("function unit.heal(u)\n  u.hp = u.hp + 1\nend"),
  "scripts/ui/hud.lua": lines(
    "local hud = {}\n\nfunction hud.draw()\n  drawHealth()\n  drawRadar()\n  drawUnits()\n  drawAmmo()\nend\n\nreturn hud",
  ),
  "scripts/vehicles.lua": lines("function spawnVehicles()\n  spawn(\"jeep\")\nend"),
  "scripts/ai.lua": lines("local ai = {}\n\nfunction ai.think(u)\n  scanArea(u)\n  pickNearest(u)\n  moveTo(u)\nend\n\nreturn ai"),
};

/** Patch ids match `patchesByMod` in fixtures. GMenu's Hotkeys anchor is missing from the game's file;
 *  AI Tweaks' Targeting loses its anchor to GCore's Target picker (a `broken_anchor_replaced` conflict). */
export const MOCK_PATCHES: MockPatch[] = [
  { id: "core-init", mod: "gcore", name: "Core init", file: "main.lua", anchor: "  initGame()", position: "after", payload: ["  gcore.init()", "  gcore.hooks()"] },
  // Two more mods add code after the same line: a shared line (§5.5).
  { id: "vehicle-init", mod: "vehicle-pack", name: "Vehicle init", file: "main.lua", anchor: "  initGame()", position: "after", payload: ["  vehicles.load()"] },
  { id: "ai-init", mod: "ai-tweaks", name: "AI init", file: "main.lua", anchor: "  initGame()", position: "after", payload: ["  aiTweaks.setup()"] },
  { id: "core-targets", mod: "gcore", name: "Target picker", file: "scripts/ai.lua", anchor: "  pickNearest(u)", position: "at", payload: ["  gcore.pickTarget(u)"] },
  { id: "menu-entry", mod: "gmenu", name: "Menu entry", file: "scripts/ui/menu.lua", anchor: "  addEntry(\"Options\")", position: "after", payload: ["  addEntry(\"Mods\")"] },
  { id: "menu-hotkeys", mod: "gmenu", name: "Hotkeys", file: "scripts/ui/keys.lua", anchor: "keys.menu = \"m\"", position: "after", payload: ["keys.mods = \"o\""] },
  { id: "medic-heal", mod: "better-medics", name: "Heal rate", file: "scripts/unit.lua", anchor: "  u.hp = u.hp + 1", position: "at", payload: ["  u.hp = u.hp + 3"] },
  { id: "hud-minimap", mod: "tactical-hud", name: "Minimap", file: "scripts/ui/hud.lua", anchor: "  drawRadar()", position: "at", payload: ["  drawMinimap()", "  drawObjectives()"] },
  { id: "hud-compass", mod: "tactical-hud", name: "Compass", file: "scripts/ui/hud.lua", anchor: "  drawMinimap()", position: "after", payload: ["  drawCompass()"], requires: ["hud-minimap"] },
  { id: "ai-overlay", mod: "ai-tweaks", name: "AI overlay", file: "scripts/ui/hud.lua", anchor: "  drawUnits()", position: "at", payload: ["  drawUnits()", "  drawThreats()"] },
  { id: "vehicle-spawn", mod: "vehicle-pack", name: "Vehicle spawn", file: "scripts/vehicles.lua", anchor: "  spawn(\"jeep\")", position: "after", payload: ["  spawn(\"halftrack\")", "  spawn(\"tank\")"] },
  { id: "ai-targeting", mod: "ai-tweaks", name: "Targeting", file: "scripts/ai.lua", anchor: "  pickNearest(u)", position: "at", payload: ["  pickWeakest(u)"] },
];

/** `MockInit.regionConflict`: GCore replaces the line AI Tweaks' Halt reset adds code after with
 *  lines that open an `if` (the real GCore change does the same), so Halt reset loses its line. */
export function addHaltScenario(): void {
  if (MOCK_PATCHES.some((p) => p.id === "core-halt")) return;
  ORIGINALS["scripts/orders.lua"] = lines(
    "local orders = {}\n\nfunction orders.halt(u)\n  u.moving = false\n  if u.target then\n    u.target = nil\n  end\nend\n\nreturn orders",
  );
  MOCK_PATCHES.push(
    { id: "core-halt", mod: "gcore", name: "Clear pathfinding on Halt", file: "scripts/orders.lua", anchor: "  if u.target then", position: "at", payload: ["  if u.spamTimer <= 0 then", "    u.path = nil"] },
    { id: "ai-halt", mod: "ai-tweaks", name: "Halt reset", file: "scripts/orders.lua", anchor: "  if u.target then", position: "after", payload: ["    u.aimTimer = 0"] },
  );
  patchesByMod.gcore.push({ patch_id: "core-halt", name: "Clear pathfinding on Halt", target_file: "scripts/orders.lua", state: "applied", conflicts: [] });
  patchesByMod["ai-tweaks"].push({ patch_id: "ai-halt", name: "Halt reset", target_file: "scripts/orders.lua", state: "broken", conflicts: [] });
}

/**
 * `MockInit.haltSplit`: the Halt pair of §10.1 in small. GCore adds a line after `u.ty = u.y`; AI Tweaks' Clear path
 * on Halt rewrites three lines, `u.ty = u.y` among them, adding two lines. GCore loads first, so its line splits AI
 * Tweaks' lines and Clear path on Halt loses its place until it runs as separate edits (D46, D48).
 */
export function addHaltSplitScenario(): void {
  if (MOCK_PATCHES.some((p) => p.id === "halt-path")) return;
  ORIGINALS["scripts/halt.lua"] = lines(
    "function halt(u)\n  u.tx = u.x\n  u.ty = u.y\n  if u.spamTimer <= 0 then\n    u.spamTimer = 1\n  end\nend",
  );
  MOCK_PATCHES.push(
    { id: "halt-timer", mod: "gcore", name: "Halt timer", file: "scripts/halt.lua", anchor: "  u.ty = u.y", position: "after", payload: ["  u.haltTimer = 0"] },
    {
      id: "halt-path", mod: "ai-tweaks", name: "Clear path on Halt", file: "scripts/halt.lua",
      anchor: "  u.tx = u.x\n  u.ty = u.y\n  if u.spamTimer <= 0 then", position: "at",
      payload: ["  u.tx = u.x", "  u.ty = u.y", "  u.path = nil", "  u.goal = nil", "  if u.spamTimer <= 0 then"],
    },
  );
  patchesByMod.gcore.push({ patch_id: "halt-timer", name: "Halt timer", target_file: "scripts/halt.lua", state: "applied", conflicts: [] });
  patchesByMod["ai-tweaks"].push({ patch_id: "halt-path", name: "Clear path on Halt", target_file: "scripts/halt.lua", state: "broken", conflicts: [] });
}

export const GLUE_MOD = "qm_glue";

function find(id: string): MockPatch | undefined {
  return MOCK_PATCHES.find((x) => x.id === id || `${x.mod}:${x.id}` === id);
}

/** A fixture patch by id or `mod:id`: its mod, name and file. */
export function findPatch(id: string): { id: string; mod: string; name: string; file: string; anchor: string; position: "after" | "before" | "at" } | undefined {
  return find(id);
}

/** A patch's text as a fix snapshots it (D41); the mod versions come from the fixtures. */
export function snapshotOf(id: string, takenAt: string): PatchSnapshot | null {
  const p = find(id);
  if (!p) return null;
  return {
    kind: "pattern", pattern: p.anchor, payload: p.payload.join("\n"), position: p.position, times: 1,
    mod_version: mods.find((m) => m.id === p.mod)?.version ?? null, taken_at: takenAt,
  };
}

/** The copy a chain of copies of copies starts from (D59); a loop ends at the copy it started from. */
function rootGlue(all: GluePatchEntry[], g: GluePatchEntry): GluePatchEntry {
  let at = g;
  for (let n = 0; n < all.length; n++) {
    const next = all.find((o) => o.id === at.replaces_patch_id);
    if (!next || next.id === g.id) return at;
    at = next;
  }
  return at;
}

/** Every copy of the profile, running or paused, as the ledger and the legacy field have them. */
function allCopies(profile: ModProfile): GluePatchEntry[] {
  const ledger = (profile.fixes ?? []).flatMap((f) => (f.change.kind === "adjusted_copy" ? [f.change.glue] : []));
  return [...ledger, ...(profile.glue_patches ?? []).filter((g) => !ledger.some((x) => x.id === g.id))];
}

/** `glue` as a patch: a copy of a copy stands in for the first copy's base (`root`). A copy that runs as its base mod
 *  takes that mod (D58); an older copy runs as QuarterMaster's own. */
function fromGlue(g: GluePatchEntry, root: GluePatchEntry = g): MockPatch | null {
  if (!("pattern" in g.patch)) return null;
  const p = g.patch.pattern;
  const base = root.replaces_patch_id ? find(root.replaces_patch_id) : undefined;
  const atBase = g.run_as === "base" && base !== undefined;
  return {
    id: g.id, mod: atBase && base ? base.mod : GLUE_MOD, name: g.name, file: Array.isArray(p.target) ? p.target[0] : p.target,
    anchor: p.pattern, position: p.position, payload: p.payload.split("\n"), replaces: base?.id ?? root.replaces_patch_id ?? undefined,
    atBase, exact: g.exact ?? false,
  };
}

/** The mod a copy runs as when it runs like its base mod (D58): the mod of the first copy's base, when that patch is
 *  loaded; else null (such a copy runs last either way). */
export function copyBaseMod(profile: ModProfile, glue: GluePatchEntry): string | null {
  const root = rootGlue(allCopies(profile), glue);
  const base = root.replaces_patch_id ? find(root.replaces_patch_id) : undefined;
  return base && profile.enabled_mods.includes(base.mod) ? base.mod : null;
}

/** Whether a running newer copy of the copy `glueId` stands in for it, so it does not run (D59). */
export function copySuperseded(profile: ModProfile, glueId: string): boolean {
  return (profile.glue_patches ?? []).some((g) => g.replaces_patch_id === glueId);
}

/** One `[[overrides]]` entry of a compatibility mod, as the mock has it (§3.10). */
export type MockOverride = {
  patch: string;
  disable?: boolean;
  priority?: number;
  before?: string[];
  after?: string[];
  anchors?: Rung[];
  sites?: SiteKey[];
  /** A stack rule (D55): at line `stack_site` on `stack_side`, the patch's block goes below or above this patch's block. */
  stack_below?: string;
  stack_above?: string;
  stack_site?: SiteKey;
  stack_side?: StackSide;
  /** Run the patch as separate edits or as one replacement (D46). */
  apply_as?: ApplyAs;
  /** Edits of the patch to skip, keeping another patch's change (D49). The mock engine does not skip them. */
  skip_hunks?: HunkKey[];
};

/** Compatibility mods' overrides, by mod id. */
export const COMPAT_OVERRIDES: Record<string, MockOverride[]> = {};
/** Author rungs of a mod's own patches that the engine never uses (D33), by mod id. */
export const SKIPPED_RUNGS: Record<string, InvalidRung[]> = {};

/** What the compatibility mods change in a profile, and a report per override (§8.1). */
export type CompatLayer = {
  reports: OverrideReport[];
  disabled: string[];
  rules: OrderRule[];
  /** Anchor overrides by patch id, with the mod that set them. */
  anchors: Map<string, { from: string; anchors: Rung[] }>;
  /** Compatibility mods that changed each patch (`records[].changed_by`). */
  changedBy: Map<string, string[]>;
  /** Stack rules that apply (D55): `patch` goes below `below` or above `above`, both patch ids. */
  stacks: { patch: string; below: string | null; above: string | null; site: SiteKey | null; side: StackSide | null; from: string }[];
  /** How compatibility mods ask patches to run (D46), by patch id. */
  applyAs: Map<string, ApplyAs>;
};

const modOfName = (name: string) => find(name)?.mod ?? (name.includes(":") ? name.split(":")[0] : name);

/** The gate, statuses and precedence of §8.1 (later compat mods winning is left out). */
export function compatLayer(profile: ModProfile): CompatLayer {
  const out: CompatLayer = { reports: [], disabled: [], rules: [], anchors: new Map(), changedBy: new Map(), stacks: [], applyAs: new Map() };
  const order = loadOrderFor(profile).ordered_mods.map((m) => m.mod_id);
  for (const c of order.filter((m) => COMPAT_OVERRIDES[m])) {
    const cn = modNameOf(c);
    const allowed = new Set([c, ...(MOD_RELATIONS[c]?.requires ?? [])]);
    for (const o of COMPAT_OVERRIDES[c]) {
      const report = (action: string, status: string, detail: string) =>
        out.reports.push({ from_mod: c, target: o.patch, action, status, detail });
      const actions = [
        ...(o.disable ? ["disable"] : []),
        ...(o.priority != null ? ["priority"] : []),
        ...((o.before ?? []).length + (o.after ?? []).length > 0 ? ["order"] : []),
        ...((o.anchors ?? []).length > 0 ? ["anchors"] : []),
        ...(o.stack_below || o.stack_above ? ["stack"] : []),
        ...(o.apply_as ? ["apply_as"] : []),
        ...((o.skip_hunks ?? []).length > 0 ? ["skip_hunks"] : []),
      ];
      if (actions.length === 0) {
        report("", "invalid", `${cn}: the override of ${o.patch} changes nothing; it is skipped`);
        continue;
      }
      const target = modOfName(o.patch);
      const refs = [
        target, ...(o.anchors ?? []).flatMap((r) => (r.within ? [modOfName(r.within)] : [])), ...[...(o.before ?? []), ...(o.after ?? [])].map(modOfName),
        ...[o.stack_below, o.stack_above].flatMap((n) => (n ? [modOfName(n)] : [])),
        ...(o.skip_hunks ?? []).flatMap((h) => (h.keep ? [modOfName(h.keep)] : [])),
      ];
      const missing = refs.find((m) => !allowed.has(m));
      if (missing) {
        const ml = modNameOf(missing);
        report(actions.join(", "), "rejected_not_required", missing === target
          ? `${cn} lists a change to ${ml}'s patch but does not require ${ml}; QuarterMaster ignores it`
          : `${cn} lists a change to ${o.patch} that refers to ${ml} but does not require ${ml}; QuarterMaster ignores it`);
        continue;
      }
      if (!profile.enabled_mods.includes(target)) {
        const ml = modNameOf(target);
        report(actions.join(", "), "waiting", `Waiting for ${ml}: ${cn}'s change to ${o.patch} does nothing while ${ml} is off`);
        continue;
      }
      const p = find(o.patch);
      if (!p) {
        report(actions.join(", "), "target_missing", `${o.patch} is not in ${modNameOf(target)}; ${cn}'s change to it does nothing`);
        continue;
      }
      // A profile fix on the same patch wins (D13); a disable always applies.
      const mine = (f: ProfileFix) => !f.paused && "patch" in f.change && (f.change.patch.id === p.id || f.change.patch.id === `${p.mod}:${p.id}`);
      const byProfile = new Set<string>();
      if (o.priority != null && (profile.fixes ?? []).some((f) => mine(f) && f.change.kind === "priority")) byProfile.add("priority");
      if ((o.anchors ?? []).length > 0 && (profile.fixes ?? []).some((f) => mine(f) && f.change.kind === "anchor")) byProfile.add("anchors");
      if (o.apply_as && (profile.fixes ?? []).some((f) => mine(f) && f.change.kind === "apply_as")) byProfile.add("apply_as");
      const applied = actions.filter((a) => !byProfile.has(a));
      if (applied.length > 0) {
        report(applied.join(", "), "applied", `${cn} changes ${o.patch}: ${applied.join(", ")}`);
        out.changedBy.set(p.id, [...new Set([...(out.changedBy.get(p.id) ?? []), c])]);
      }
      if (byProfile.size > 0) {
        const list = actions.filter((a) => byProfile.has(a)).join(", ");
        report(list, "superseded_by_profile", `Your fix on ${o.patch} replaces ${cn}'s change (${list})`);
      }
      if (o.disable) out.disabled.push(p.id);
      if (applied.includes("order")) out.rules.push({ patch: p.id, before: o.before ?? [], after: o.after ?? [] });
      if (applied.includes("anchors")) out.anchors.set(p.id, { from: c, anchors: o.anchors ?? [] });
      if (applied.includes("stack")) {
        const id = (n: string | undefined) => (n ? (find(n)?.id ?? n) : null);
        out.stacks.push({ patch: p.id, below: id(o.stack_below), above: id(o.stack_above), site: o.stack_site ?? null, side: o.stack_side ?? null, from: c });
      }
      if (applied.includes("apply_as") && o.apply_as) out.applyAs.set(p.id, o.apply_as);
    }
  }
  return out;
}

/** Patches the game skips in a profile: the profile's own and the ones compatibility mods turn off. */
export function disabledIn(profile: ModProfile): Set<string> {
  return new Set([...(profile.disabled_patches ?? []), ...compatLayer(profile).disabled]);
}

/** Moves patches to satisfy each rule, in rule order (the mock's stand-in for the engine's sort). */
function applyRules(list: MockPatch[], rules: OrderRule[]): MockPatch[] {
  const out = [...list];
  const at = (name: string) => out.findIndex((p) => p.id === name || `${p.mod}:${p.id}` === name);
  for (const r of rules) {
    const i = at(r.patch);
    if (i === -1) continue;
    const [p] = out.splice(i, 1);
    const firstBefore = Math.min(...(r.before ?? []).map(at).filter((k) => k !== -1), out.length);
    const lastAfter = Math.max(...(r.after ?? []).map(at).filter((k) => k !== -1), -1);
    out.splice(Math.max(Math.min(firstBefore, i), lastAfter + 1), 0, p);
  }
  return out;
}

/** The mod an adjusted copy waits for: its base patch's mod when that mod is off (D19). */
function waitingFor(g: GluePatchEntry, on: (mod: string) => boolean): string | null {
  const id = g.replaces_patch_id;
  if (!id) return null;
  const mod = find(id)?.mod ?? g.base_mod_id ?? (id.includes(":") ? id.split(":")[0] : undefined);
  return mod && !on(mod) ? mod : null;
}

/** The file's patches from enabled mods, in load order then patch order, with profile rules and
 *  adjusted copies. The loader drops a copy's base patch: a copy that runs as its base mod takes
 *  its place (D58), an older copy runs after every mod patch (D18). A copy whose base mod is off
 *  is skipped, and so is a copy that a newer copy of it replaces (D59). */
function ordered(file: string, profile: ModProfile, compatRules: OrderRule[] = compatLayer(profile).rules): MockPatch[] {
  const rank = new Map(loadOrderFor(profile).ordered_mods.map((m) => [m.mod_id, m.order_index]));
  const all = profile.glue_patches ?? [];
  const glue = all
    .filter((g) => !copySuperseded(profile, g.id))
    .map((g) => ({ g, root: rootGlue(all, g) }))
    .filter(({ root }) => waitingFor(root, (m) => rank.has(m)) === null)
    .map(({ g, root }) => fromGlue(g, root))
    .filter((g): g is MockPatch => g !== null && g.file === file);
  const base = MOCK_PATCHES.filter((p) => p.file === file && rank.has(p.mod)).sort(
    (a, b) => (rank.get(a.mod) ?? 0) - (rank.get(b.mod) ?? 0) || MOCK_PATCHES.indexOf(a) - MOCK_PATCHES.indexOf(b),
  );
  const list = base.flatMap((p) => {
    const copies = glue.filter((g) => g.replaces === p.id);
    return copies.length > 0 ? copies.filter((g) => g.atBase) : [p];
  });
  const last = glue.filter((g) => !list.includes(g));
  // Compatibility rules first: a profile rule on the same patch moves it last (D35).
  return applyRules([...list, ...last], [...compatRules, ...(profile.patch_order ?? [])]);
}

type Run = {
  lines: AttributedCodeLine[]; records: PatchExecutionRecord[]; order: MockPatch[]; conflicts: DetectedConflict[]; stacks: SharedStack[];
  /** Places where an Anchor fix's patch uses its own line, and why. */
  fallbacks: FallbackNotice[];
  /** Anchors on another patch's code, and the failures grouped under the code that did not apply (G10). */
  edges: InferredEdge[];
  groups: FailureGroup[];
};

/**
 * The file's anchors on other patches' code (G10), like the simulation: a patch with a line the game's file lacks uses
 * every other patch whose code has all such lines. A failed patch whose code is only in patches that did not apply
 * hangs under the first of them; the root of a chain is a patch that did not apply for a reason of its own.
 */
function chains(file: string, order: MockPatch[], records: PatchExecutionRecord[], conflicts: DetectedConflict[]): { edges: InferredEdge[]; groups: FailureGroup[] } {
  const original = ORIGINALS[file] ?? [];
  const uses = new Map<number, number[]>();
  records.forEach((rec, f) => {
    const p = order[f];
    if (!p || rec.status === "skipped_disabled") return;
    const missing = p.anchor.split("\n").filter((w) => !original.some(lineMatcher(w, p.exact)));
    if (missing.length === 0) return;
    const from = order.flatMap((q, g) => (g !== f && q.id !== p.id && missing.every((w) => q.payload.some(lineMatcher(w, p.exact))) ? [g] : []));
    if (from.length > 0) uses.set(f, from);
  });
  const edges: InferredEdge[] = [];
  for (const [f, from] of uses) {
    for (const g of from) {
      edges.push({ target_file: file, patch_id: records[f].patch_id, mod_id: records[f].mod_id, uses_patch_id: records[g].patch_id, uses_mod_id: records[g].mod_id });
    }
  }
  const applied = (r: PatchExecutionRecord) => r.status === "applied" || r.status === "count_mismatch";
  const parent = new Map<number, number>();
  for (const [f, from] of uses) {
    if (records[f].status === "zero_matches" && from.every((g) => !applied(records[g]))) parent.set(f, from[0]);
  }
  const rootOf = (f: number): number | null => {
    let at = f;
    for (let n = 0; n <= records.length; n++) {
      const up = parent.get(at);
      if (up === undefined) return at;
      if (up === f) return null;
      at = up;
    }
    return null;
  };
  const groups: FailureGroup[] = [];
  records.forEach((rec, f) => {
    if (!parent.has(f)) return;
    const r = rootOf(f);
    if (r === null) return;
    const root = records[r];
    const member = { patch_id: rec.patch_id, mod_id: rec.mod_id, name: rec.patch_name };
    const g = groups.find((x) => x.root === root.patch_id);
    if (g) g.members.push(member);
    else groups.push({
      target_file: file, root: root.patch_id, root_mod: root.mod_id, root_name: root.patch_name, members: [member],
      reason: root.status === "skipped_disabled" ? "turned_off" : "no_match",
    });
    // Like the simulation, every patch with no match has its own broken-anchor conflict.
    if (!conflicts.some((c) => c.patch_a_id === rec.patch_id)) {
      conflicts.push({
        id: `broken-anchor:${file}:${rec.patch_id}`, severity: "error", kind: "broken_anchor_replaced", target_file: file,
        mod_a_id: rec.mod_id, mod_b_id: null, patch_a_id: rec.patch_id, patch_b_id: null,
        description: `Patch '${rec.patch_name}' (from '${rec.mod_id}') resulted in 0 matches on '${file}'.`, suggestions: [],
      });
    }
  });
  return { edges, groups };
}

/** A line with its uid and the block that wrote it (-1: the game's own line). */
type Line = AttributedCodeLine & { uid: number; block: number };
/** One patch's inserted or replacing code; `anchor` is set for inserts (§5.1), `parent` is the block of the line it hangs on. */
type Block = { record: number; mod: string; anchor: { uid: number; side: StackSide } | null; parent: number };
/** A top-level item of a chain: its block and first and last line (0-based). */
type Item = { block: number; first: number; last: number };

/** The block in `l`'s ancestry anchored on (uid, side), or -1. */
function itemBlock(l: Line, blocks: Block[], uid: number, side: StackSide): number {
  for (let b = l.block, n = 0; b !== -1 && n <= blocks.length; b = blocks[b].parent, n++) {
    const a = blocks[b].anchor;
    if (a && a.uid === uid && a.side === side) return b;
  }
  return -1;
}

/** The items stacked on one side of line `at`, top to bottom (§5.2). */
function chain(lines: Line[], blocks: Block[], at: number, side: StackSide): Item[] {
  const uid = lines[at].uid;
  const items: Item[] = [];
  const step = side === "after" ? 1 : -1;
  for (let i = at + step; i >= 0 && i < lines.length; i += step) {
    const b = itemBlock(lines[i], blocks, uid, side);
    if (b === -1) break;
    const last = side === "after" ? items[items.length - 1] : items[0];
    if (last && last.block === b) {
      if (side === "after") last.last = i;
      else last.first = i;
    } else if (side === "after") items.push({ block: b, first: i, last: i });
    else items.unshift({ block: b, first: i, last: i });
  }
  return items;
}

function reasonOf(s: Slot, items: Item[], blocks: Block[], sources: StackReason[] = []): StackReason | null {
  if (!s.reason) return null;
  const modOf = (i: number) => (items[i] ? blocks[items[i].block].mod : "");
  switch (s.reason.kind) {
    case "below": return { kind: "below_dependency", of: modOf(s.reason.item) };
    case "above": return { kind: "above_dependent", of: modOf(s.reason.item) };
    case "load_order": return { kind: "load_order" };
    case "unsatisfiable": return { kind: "unsatisfiable", below: modOf(s.reason.item), above: modOf(s.reason.above ?? -1) };
    case "rule": return sources[s.reason.item] ?? { kind: "guard_clamped" };
  }
}

/** A stack rule held by one of the two patches it names (D55): this patch goes below `other` or above it. */
type BoundRule = { other: string; meBelow: boolean; site: SiteKey | null; side: StackSide | null; rank: number; reason: StackReason; label: string };

/** The stack rules that name `p`: the profile's running StackRule fixes first, then compatibility mods' (D56). */
function stackRulesOf(profile: ModProfile, layer: CompatLayer, p: MockPatch): BoundRule[] {
  const is = (id: string | null | undefined) => !!id && (id === p.id || id === `${p.mod}:${p.id}`);
  const plain = (id: string) => find(id)?.id ?? id;
  const out: BoundRule[] = [];
  const bind = (patch: string, below: string | null, above: string | null, site: SiteKey | null, side: StackSide | null, rank: number, reason: StackReason, label: string) => {
    const other = below ?? above;
    if (!other) return;
    if (is(patch)) out.push({ other: plain(other), meBelow: below !== null, site, side, rank, reason, label });
    else if (is(other)) out.push({ other: plain(patch), meBelow: below === null, site, side, rank, reason, label });
  };
  for (const f of profile.fixes ?? []) {
    const c = f.change;
    if (f.paused || c.kind !== "stack_rule") continue;
    bind(c.patch.id, c.below?.id ?? null, c.above?.id ?? null, c.site, c.side, 0, { kind: "user_rule", fix_id: f.id }, `Your fix ${f.id}`);
  }
  for (const r of layer.stacks) bind(r.patch, r.below, r.above, r.site, r.side, 1, { kind: "compat_rule", mod_id: r.from }, `${r.from.charAt(0).toUpperCase()}${r.from.slice(1)}'s rule`);
  return out;
}

/** The enclosing function's name, like the real report's "in love.load". */
function siteLabel(lines: Line[], at: number): string {
  for (let i = at; i >= 0; i--) {
    const t = lines[i].text;
    if (t === "end") return "outside any function";
    const m = /^(?:local\s+)?function\s+([\w.:]+)/.exec(t);
    if (m) return `in ${m[1]}`;
  }
  return "outside any function";
}

/** Lines with two or more items on one side (§5.5), from the final lines. */
function sharedStacks(file: string, mode: StackMode, lines: Line[], blocks: Block[], records: PatchExecutionRecord[]): SharedStack[] {
  const live = new Map(lines.map((l, i) => [l.uid, i]));
  const keys = new Map<string, { at: number; side: StackSide }>();
  for (const b of blocks) {
    const at = b.anchor ? live.get(b.anchor.uid) : undefined;
    if (b.anchor && at !== undefined) keys.set(`${String(at).padStart(8, "0")}:${b.anchor.side === "before" ? 0 : 1}`, { at, side: b.anchor.side });
  }
  const out: SharedStack[] = [];
  for (const [, { at, side }] of [...keys].sort(([a], [b]) => (a < b ? -1 : 1))) {
    const items = chain(lines, blocks, at, side);
    if (items.length < 2) continue;
    const list: StackBlock[] = items.map((it) => {
      const r = records[blocks[it.block].record];
      return {
        patch_id: r.patch_id, patch_name: r.patch_name, mod_id: r.mod_id, first_line: it.first + 1, last_line: it.last + 1,
        reason: r.stack_moves?.[0]?.reason ?? null,
        ...(r.stack_moves?.[0]?.reason.kind === "user_rule" ? { rule_fix_id: r.stack_moves[0].reason.fix_id } : {}),
      };
    });
    const applied = items.map((it) => blocks[it.block].record).sort((a, b) => a - b);
    if (side === "after") applied.reverse();
    const legacy = applied.map((i) => records[i].patch_id);
    out.push({
      target_file: file, file_line: at + 1, anchor_text: lines[at].text.trim(), side, site_label: siteLabel(lines, at), mode,
      blocks: list, legacy, moved: list.some((b, i) => b.patch_id !== legacy[i]),
      // The mock has no line contexts: the line's own text stands in for one.
      site: { context: `${file}:${lines[at].text.trim()}`, ordinal: 1 },
      stack_key: `${file}#${at + 1}#${side}`,
    });
  }
  return out;
}

/** The profile's Anchor fix on `p` that is not paused, when it has one. */
function anchorFixOf(profile: ModProfile, p: MockPatch): ProfileFix | undefined {
  return (profile.fixes ?? []).find(
    (f) => !f.paused && f.change.kind === "anchor" && (f.change.patch.id === p.id || f.change.patch.id === `${p.mod}:${p.id}`),
  );
}

const modNameOf = (id: string) => mods.find((m) => m.id === id)?.name ?? id;

/** The line where `rung` puts `p`'s code in `current`, or why it cannot (§6.1, without stacking). */
function placeByRung(rung: Rung, p: MockPatch, current: Line[], profile: ModProfile, file: string): { at: number; side: StackSide } | { why: string } {
  const side: StackSide = rung.position === "before" || rung.position === "after" ? rung.position : p.position === "before" ? "before" : "after";
  const pick = (hits: number[]) => {
    const n = rung.nth ?? null;
    if (n !== null) return hits[n - 1] ?? null;
    return hits.length === 1 ? hits[0] : null;
  };
  if (!rung.within) {
    const text = (rung.pattern ?? "").trim();
    const hits = current.flatMap((l, i) => (text && l.text.trim() === text ? [i] : []));
    const at = pick(hits);
    if (at === null) return { why: hits.length === 0 ? "the line it was moved to is gone" : "the line it was moved to is there more than once" };
    return { at, side };
  }
  const b = find(rung.within);
  if (!b) return { why: `${rung.within} is no longer in its mod` };
  if (!profile.enabled_mods.includes(b.mod)) return { why: `${modNameOf(b.mod)} is off` };
  if ((profile.disabled_patches ?? []).includes(b.id)) return { why: `${b.name} is turned off` };
  const mine = current.flatMap((l, i) => (l.patch_id === b.id ? [i] : []));
  if (mine.length === 0) return { why: `${b.name} made no change in ${file}` };
  if (rung.edge) {
    const [start, end] = edgeChecks(mine.map((i) => current[i].text), modNameOf(b.mod));
    const gate = rung.edge === "start" ? start : end;
    if (!gate.allowed) return { why: gate.reason ?? "that edge cannot be used" };
    return rung.edge === "start" ? { at: mine[0], side: "before" } : { at: mine[mine.length - 1], side: "after" };
  }
  const text = (rung.pattern ?? "").trim();
  const at = pick(mine.filter((i) => current[i].text.trim() === text));
  if (at === null) return { why: `${modNameOf(b.mod)} no longer has that line` };
  return { at, side };
}

/** How `p` runs: a running profile ApplyAs fix first, then a compatibility mod's override (D46). */
function applyAsOf(profile: ModProfile, layer: CompatLayer, p: MockPatch): ApplyAs {
  const fix = (profile.fixes ?? []).find(
    (f) => !f.paused && f.change.kind === "apply_as" && (f.change.patch.id === p.id || f.change.patch.id === `${p.mod}:${p.id}`),
  );
  if (fix && fix.change.kind === "apply_as") return fix.change.mode;
  return layer.applyAs.get(p.id) ?? "literal";
}

/**
 * Where the lines `want` are in `current`: their indexes, one after another, or with `skip` lines other patches added
 * between them (D48); the first and the last always match. Null when they are not there.
 */
function windowIn(current: Line[], want: string[], p: MockPatch, skip: boolean): number[] | null {
  const match = want.map((w) => lineMatcher(w, p.exact));
  const foreign = (l: Line) => l.block !== -1 && l.patch_id !== p.id;
  for (let i = 0; i < current.length; i++) {
    if (!match[0](current[i].text)) continue;
    const hit = [i];
    let j = i + 1;
    for (let k = 1; k < match.length; k++) {
      while (skip && j < current.length && !match[k](current[j].text) && foreign(current[j])) j++;
      if (j >= current.length || !match[k](current[j].text)) break;
      hit.push(j++);
    }
    if (hit.length === match.length) return hit;
  }
  return null;
}

/** The kept lines of an `at` patch's separate edits: pairs of pattern and payload lines, by longest common run (D47). */
function keptPairs(pattern: string[], payload: string[]): [number, number][] {
  const a = pattern.map((l) => l.trim());
  const b = payload.map((l) => l.trim());
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      // A wildcard line is never kept.
      lcs[i][j] = a[i] === b[j] && !/[*?]/.test(a[i]) ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const out: [number, number][] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j] && !/[*?]/.test(a[i])) out.push([i++, j++]);
    else if (lcs[i + 1][j] >= lcs[i][j + 1]) i++;
    else j++;
  }
  return out;
}

/** The separate edits between kept lines: pattern lines `p` replaced by payload lines `q` (either may be empty). */
type Gap = { p: [number, number]; q: [number, number]; after: number | null; before: number | null };

function gapsOf(pattern: string[], payload: string[], kept: [number, number][]): Gap[] {
  const out: Gap[] = [];
  let prev: [number, number] = [-1, -1];
  for (const pair of [...kept, [pattern.length, payload.length] as [number, number]]) {
    const p: [number, number] = [prev[0] + 1, pair[0]];
    const q: [number, number] = [prev[1] + 1, pair[1]];
    if (p[1] > p[0] || q[1] > q[0]) out.push({ p, q, after: prev[0] >= 0 ? prev[0] : null, before: pair[0] < pattern.length ? pair[0] : null });
    prev = pair;
  }
  return out;
}

/** "Runs as 3 insertions", as the service words it (§6.6). */
function runsAsText(inserted: number, changed: number, removed: number): string {
  const count = (n: number, one: string, many: string) => (n === 0 ? [] : [n === 1 ? `1 ${one}` : `${n} ${many}`]);
  const parts = [...count(inserted, "insertion", "insertions"), ...count(changed, "changed line", "changed lines"), ...count(removed, "removed line", "removed lines")];
  if (parts.length === 0) return "Runs as separate edits that change no line";
  if (parts.length === 1) return `Runs as ${parts[0]}`;
  return `Runs as ${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/** How an `at` patch runs (§6.6); null for other patches. */
export function runsAsFor(id: string, profile: ModProfile): RunsAsVerdict | null {
  const p = find(id);
  if (!p || p.position !== "at") return null;
  const pattern = p.anchor.split("\n");
  const kept = keptPairs(pattern, p.payload);
  let inserted = 0;
  let changed = 0;
  let removed = 0;
  for (const g of gapsOf(pattern, p.payload, kept)) {
    const pl = g.p[1] - g.p[0];
    const ql = g.q[1] - g.q[0];
    if (pl === 0) inserted += ql;
    else if (ql === 0) removed += pl;
    else changed += pl;
  }
  const asks = applyAsOf(profile, compatLayer(profile), p);
  return {
    patch_id: p.id, mod_id: p.mod, asks, runs_as: asks,
    edits: { kept: kept.length, inserted, changed, removed },
    text: asks === "edits" ? runsAsText(inserted, changed, removed) : "Runs as one replacement",
  };
}

/** How another patch took `p`'s line: what it wrote instead (§9.1). */
function consumerOf(file: string, p: MockPatch, culprit: MockPatch): ConsumerInfo {
  const old = p.anchor.trim();
  const kind: ConsumerKind =
    culprit.payload.length === 0 ? "deleted"
    : culprit.payload.some((l) => l.trim() === old) ? "reemitted"
    : culprit.payload.length === 1 ? "rewritten_1to1"
    : "replaced_by_block";
  const line = (ORIGINALS[file] ?? []).findIndex((l) => l.trim() === old) + 1;
  const vanilla = (ORIGINALS[file] ?? [])[line - 1] ?? p.anchor;
  return {
    patch_id: culprit.id, mod_id: culprit.mod, kind, site_line: line, old_text: vanilla,
    new_text: kind === "deleted" ? null : culprit.payload.join("\n"),
  };
}

function run(file: string, profile: ModProfile): Run {
  const layer = compatLayer(profile);
  const off = new Set([...(profile.disabled_patches ?? []), ...layer.disabled]);
  const mode: StackMode = profile.stacking ?? "legacy";
  const deps = dependsOn(profile.enabled_mods);
  let current: Line[] = (ORIGINALS[file] ?? []).map((text, i) => ({
    line_number: 0, text, mod_id: null, patch_id: null, patch_name: null, uid: i, block: -1,
  }));
  let nextUid = current.length;
  const blocks: Block[] = [];
  const records: PatchExecutionRecord[] = [];
  const conflicts: DetectedConflict[] = [];
  const fallbacks: FallbackNotice[] = [];
  /** Game lines a patch replaced, so a later patch that wanted them knows who took them. */
  const takenBy = new Map<string, MockPatch>();
  const order = ordered(file, profile, layer.rules);
  for (const p of order) {
    const base: PatchExecutionRecord = {
      patch_id: p.id, patch_name: p.name, mod_id: p.mod, source_file: p.mod === GLUE_MOD ? "qm/load-order.json" : `${p.mod}/qm.toml`,
      target_file: file, patch_type: "pattern", priority: 50, matched_count: 0, expected_count: 1, on_failure: "warn",
      status: "skipped_disabled", warnings: [], regions: [],
      ...(layer.changedBy.has(p.id) ? { changed_by: layer.changedBy.get(p.id) } : {}),
    };
    if (off.has(p.id)) {
      const by = (profile.disabled_patches ?? []).includes(p.id) ? "Disabled by user in profile" : `Turned off by ${(base.changed_by ?? []).map(modNameOf).join(", ")}`;
      records.push({ ...base, warnings: [by] });
      continue;
    }
    // A profile Anchor fix places the code first, then a compatibility mod's anchors; a rung that
    // cannot be used falls back to the patch's own line.
    const profileFix = p.position === "at" ? undefined : anchorFixOf(profile, p);
    const compat = p.position === "at" || profileFix ? undefined : layer.anchors.get(p.id);
    const fix = profileFix && profileFix.change.kind === "anchor"
      ? { source: `profile:${profileFix.id}`, anchors: profileFix.change.anchors, id: profileFix.id }
      : compat ? { source: `compat:${compat.from}`, anchors: compat.anchors, id: null } : undefined;
    let placed: { at: number; side: StackSide | null; rung: number | null } | null = null;
    let why: string | null = null;
    let anchoredOn: string | null = null;
    if (fix) {
      for (const [i, rung] of fix.anchors.entries()) {
        const r = placeByRung(rung, p, current, profile, file);
        if ("why" in r) {
          why ??= r.why;
          continue;
        }
        placed = { at: r.at, side: r.side, rung: i };
        anchoredOn = rung.within ? (find(rung.within) ? `${find(rung.within)!.mod}:${find(rung.within)!.id}` : rung.within) : "text";
        why = null;
        break;
      }
    }
    // Like the engine, the match ignores indentation; an exact copy's `*` is a plain `*` (D59). Several lines match
    // one after another; as separate edits they may have other patches' lines between them (D48).
    const want = p.anchor.split("\n");
    const edits = p.position === "at" && want.length > 1 && applyAsOf(profile, layer, p) === "edits";
    const win = windowIn(current, want, p, false) ?? (edits ? windowIn(current, want, p, true) : null);
    const own = win ? (p.position === "after" ? win[win.length - 1] : win[0]) : -1;
    if (!placed && own !== -1) placed = { at: own, side: p.position === "at" ? null : p.position, rung: null };
    const first = lineMatcher(want[0], p.exact);
    const vanillaLine = (ORIGINALS[file] ?? []).findIndex((l, i, all) => first(l) && want.every((w, k) => lineMatcher(w, p.exact)(all[i + k] ?? ""))) + 1;
    const anchoring: AnchorReport | null = fix
      ? {
          source: fix.source,
          sites: [{
            site: 0, vanilla_line: vanillaLine || null, rung: placed?.rung ?? null, anchored_on: placed?.rung != null ? anchoredOn : null,
            fallback: placed?.rung != null ? null : why, followed: false, b_drift: false, lost: false,
          }],
        }
      : null;
    const extra = fix ? { anchoring, ...(fix.id ? { fix_ids: [fix.id] } : {}) } : {};
    if (fix && placed?.rung == null && why) fallbacks.push({ file, patch_id: p.id, site_line: vanillaLine, reason: why });
    if (!placed) {
      records.push({ ...base, status: "zero_matches", warnings: [`Pattern not found: ${p.anchor}`], ...extra });
      // Other patches' lines split the window (§4.8): it finds its place again as separate edits.
      const split = want.length > 1 ? windowIn(current, want, p, true) : null;
      if (split) {
        const between = current.slice(split[0] + 1, split[split.length - 1]).filter((l) => l.block !== -1 && l.patch_id !== p.id);
        const splitter = between[0]?.patch_id ? find(between[0].patch_id) : undefined;
        if (splitter) {
          const offer: ConflictResolutionSuggestion = {
            tier: "apply_as", patch_id: p.id, mod_id: p.mod,
            explanation: p.position === "at" ? `Run '${p.name}' as separate edits instead of one replacement.` : `Run '${p.name}' as separate edits, so it finds its lines around the other mod's code.`,
          };
          conflicts.push({
            id: `broken-anchor:${file}:${p.id}`, severity: "error", kind: "broken_anchor_order", target_file: file,
            mod_a_id: p.mod, mod_b_id: splitter.mod, patch_a_id: p.id, patch_b_id: splitter.id,
            description: `Patch '${p.name}' (from '${p.mod}') failed to match on '${file}' because earlier mod '${splitter.mod}' modified the vanilla anchor.`,
            suggestions: [offer],
            consumer: {
              patch_id: splitter.id, mod_id: splitter.mod, kind: "split", site_line: vanillaLine,
              old_text: (ORIGINALS[file] ?? [])[vanillaLine - 1] ?? want[0], new_text: between.map((l) => l.text).join("\n"),
            },
          });
        }
        continue;
      }
      const culprit = takenBy.get(p.anchor.trim());
      if (culprit) {
        conflicts.push({
          id: `broken-anchor:${file}:${p.id}`, severity: "error", kind: "broken_anchor_replaced", target_file: file,
          mod_a_id: p.mod, mod_b_id: culprit.mod, patch_a_id: p.id, patch_b_id: culprit.id,
          description: `Patch '${p.name}' (from '${p.mod}') failed to match on '${file}' because earlier mod '${culprit.mod}' modified the vanilla anchor.`,
          suggestions: [], consumer: consumerOf(file, p, culprit),
        });
      }
      continue;
    }
    if (edits && win && placed.rung === null) {
      // Separate edits (D46): kept lines stay as the game has them, new lines go next to them; changed lines are
      // replaced where they are. The lines of other patches between them stay (D48).
      const kept = keptPairs(want, p.payload);
      const block = blocks.length;
      const above = kept.length > 0 ? current[win[kept[0][0]]] : current[win[0]];
      blocks.push({ record: records.length, mod: p.mod, anchor: { uid: above.uid, side: "after" }, parent: -1 });
      const made = (text: string): Line => ({ line_number: 0, text, mod_id: p.mod, patch_id: p.id, patch_name: p.name, uid: nextUid++, block });
      // From the last edit back, so the indexes before it stay.
      for (const g of gapsOf(want, p.payload, kept).reverse()) {
        const add = p.payload.slice(g.q[0], g.q[1]).map(made);
        if (g.p[1] > g.p[0]) {
          const from = win[g.p[0]];
          const to = win[g.p[1] - 1];
          current = [...current.slice(0, from), ...add, ...current.slice(to + 1)];
        } else {
          const at = g.after !== null ? win[g.after] + 1 : win[g.before ?? 0];
          current = [...current.slice(0, at), ...add, ...current.slice(at)];
        }
      }
      const mine = current.flatMap((l, i) => (l.block === block ? [i] : []));
      records.push({
        ...base, status: "applied", matched_count: 1,
        regions: mine.length > 0 ? [{ start_line: mine[0] + 1, end_line: mine[mine.length - 1] + 1 }] : [],
        ...extra,
      });
      continue;
    }
    const at = placed.at;
    const block = blocks.length;
    const side = placed.side;
    blocks.push({ record: records.length, mod: p.mod, anchor: side ? { uid: current[at].uid, side } : null, parent: current[at].block });
    const added: Line[] = p.payload.map((text) => ({ line_number: 0, text, mod_id: p.mod, patch_id: p.id, patch_name: p.name, uid: nextUid++, block }));
    let start = side === "after" ? at + 1 : at;
    const moves: StackMove[] = [];
    if (side) {
      // Legacy puts it next to the line (after) or right above it (before); other modes read the chain.
      const items = chain(current, blocks, at, side);
      // Stack rules whose other block is in this chain, at this line and side (D55); the mock's
      // line key is the line's text.
      const patchOf = (it: Item) => records[blocks[it.block].record].patch_id;
      const here = stackRulesOf(profile, layer, p).flatMap((r) => {
        const item = items.findIndex((it) => patchOf(it) === r.other);
        const siteOk = !r.site || r.site.context === `${file}:${current[at].text.trim()}`;
        return item !== -1 && siteOk && (!r.side || r.side === side) ? [{ ...r, item }] : [];
      });
      const ruled = stackSlotRuled(mode, side, p.mod, items.map((it) => blocks[it.block].mod), deps, here.map((r) => ({ item: r.item, below: r.meBelow, rank: r.rank })));
      const s = ruled.slot;
      const reason = reasonOf(s, items, blocks, here.map((r) => r.reason));
      const overridden = ruled.overridden.map((i) => {
        const r = here[i];
        return `${r.label} to put this block ${r.meBelow ? "below" : "above"} ${records[blocks[items[r.item].block].record].patch_name}'s is overridden by a stronger rule`;
      });
      if (side === "after") start = s.slot === 0 ? at + 1 : items[s.slot - 1].last + 1;
      else start = s.slot === items.length ? at : items[s.slot].first;
      if (reason) {
        const id = (i: number) => (items[i] ? records[blocks[items[i].block].record].patch_id : null);
        moves.push({
          site: 0, anchor_line: at + 1, side, legacy_slot: s.legacy, slot: s.slot, reason, below: id(s.slot - 1), above: id(s.slot),
          ...(overridden.length > 0 ? { overridden } : {}),
        });
      }
    }
    // A replacement takes the whole window.
    const end = side === null ? at + (win && placed.rung === null && p.position === "at" ? win.length : 1) : start;
    if (side === null) for (let k = at; k < end; k++) takenBy.set(current[k].text.trim(), p);
    current = [...current.slice(0, start), ...added, ...current.slice(end)];
    records.push({
      ...base, status: "applied", matched_count: 1, regions: [{ start_line: start + 1, end_line: start + added.length }],
      ...(moves.length > 0 ? { stack_moves: moves } : {}), ...extra,
    });
  }
  current.forEach((l, i) => (l.line_number = i + 1));
  const stacks = sharedStacks(file, mode, current, blocks, records);
  const lines = current.map(({ uid: _uid, block: _block, ...l }) => l);
  const { edges, groups } = chains(file, order, records, conflicts);
  return { lines, records, order, conflicts, stacks, fallbacks, edges, groups };
}

/** The patched text of every file the profile changes. */
export function profileTexts(profile: ModProfile): Map<string, string> {
  return new Map(touchedFiles(profile).map((f) => [f, run(f, profile).lines.map((l) => l.text).join("\n")]));
}

/** Every shared line in the profile, or in one file. */
export function stacksFor(profile: ModProfile, file: string | null): SharedStack[] {
  return touchedFiles(profile).filter((f) => file === null || f === file).flatMap((f) => run(f, profile).stacks);
}

/** Like the real simulation: over 1,200 lines, keep only patched lines and 4 around them. */
function trimmed(all: AttributedCodeLine[]): AttributedCodeLine[] {
  if (all.length <= 1200) return all;
  const keep = new Set<number>();
  all.forEach((l, i) => {
    if (l.patch_id) for (let k = i - 4; k <= i + 4; k++) keep.add(k);
  });
  return all.filter((_, i) => keep.has(i));
}

export function touchedFiles(profile: ModProfile): string[] {
  return [...new Set(MOCK_PATCHES.filter((p) => profile.enabled_mods.includes(p.mod)).map((p) => p.file))];
}

/** The simulated outcome of one fixture patch: applied, broken or disabled. */
export function patchState(id: string, profile: ModProfile): "applied" | "broken" | "disabled" | null {
  const p = find(id);
  if (!p) return null;
  if (!profile.enabled_mods.includes(p.mod)) return "disabled";
  const rec = run(p.file, profile).records.find((r) => r.patch_id === p.id);
  if (!rec || rec.status === "skipped_disabled") return "disabled";
  return rec.status === "applied" ? "applied" : "broken";
}

/** Broken anchors whose line another patch replaced. */
export function engineConflicts(profile: ModProfile): DetectedConflict[] {
  return touchedFiles(profile).flatMap((f) => run(f, profile).conflicts);
}

/** Patches that fail because code they anchor on did not apply, by root, and every anchor on another patch's code (G10). */
export function chainsFor(profile: ModProfile): { failure_groups: FailureGroup[]; inferred_edges: InferredEdge[] } {
  const runs = touchedFiles(profile).map((f) => run(f, profile));
  return { failure_groups: runs.flatMap((r) => r.groups), inferred_edges: runs.flatMap((r) => r.edges) };
}

export function fileDiffFor(file: string, profile: ModProfile): FileDiff | null {
  if (!touchedFiles(profile).includes(file)) return null;
  const r = run(file, profile);
  const original = ORIGINALS[file] ?? [];
  return {
    original_lines: original,
    result: {
      target_file: file,
      mods_touching: [...new Set(r.order.map((p) => p.mod))],
      records: r.records,
      attributed_lines: trimmed(r.lines),
      original_line_count: original.length,
      patched_line_count: r.lines.length,
      stacks: r.stacks,
    },
  };
}

export function fileStatsFor(profile: ModProfile): FileStat[] {
  const conflicts = liveConflicts(profile);
  return touchedFiles(profile).map((file) => {
    const r = run(file, profile);
    return {
      target_file: file,
      original_lines: ORIGINALS[file]?.length ?? 0,
      patched_lines: r.lines.length,
      patches: r.records.length,
      mods: [...new Set(r.order.map((p) => p.mod))],
      broken: r.records.filter((x) => x.status === "zero_matches").length,
      conflicts: conflicts.filter((c) => c.target_file === file).length,
    };
  });
}

export function patchDetailFor(id: string, profile: ModProfile): PatchDetail | null {
  const glue = (profile.glue_patches ?? []).find((g) => g.id === id);
  const p = glue ? fromGlue(glue, rootGlue(profile.glue_patches ?? [], glue)) : find(id);
  if (!p) return null;
  const order = ordered(p.file, profile);
  const index = order.findIndex((x) => x.id === p.id);
  const name = p.mod === GLUE_MOD ? "Your adjustments" : (mods.find((m) => m.id === p.mod)?.name ?? p.mod);
  return {
    patch_id: p.id,
    name: p.name,
    mod_id: p.mod,
    source_file: p.mod === GLUE_MOD ? "qm/load-order.json" : `${name}/qm.toml`,
    priority: loadOrderFor(profile).ordered_mods.find((m) => m.mod_id === p.mod)?.effective_priority ?? glue?.priority ?? 50,
    definition: {
      pattern: {
        pattern: p.anchor, position: p.position, target: p.file, payload: p.payload.join("\n"), times: 1,
        id: p.id, name: p.name, requires: p.requires ?? [], priority: null, before: [], after: [], on_failure: "warn",
      },
    },
    positions: index === -1 ? [] : [{ target_file: p.file, index, count: order.length }],
    declared_before: [],
    declared_after: [],
    profile_rules: (profile.patch_order ?? []).filter((r) => r.patch === p.id || r.patch === `${p.mod}:${p.id}`),
    requires: p.requires ?? [],
    required_by: MOCK_PATCHES.filter((q) => q.requires?.includes(p.id)).map((q) => q.id),
    disabled: disabledIn(profile).has(p.id),
    replaces_patch_id: glue?.replaces_patch_id ?? null,
    ...(glue?.base_mod_id ? { base_mod_id: glue.base_mod_id } : {}),
    ...(runsAsFor(p.id, profile) ? { runs_as: runsAsFor(p.id, profile) } : {}),
  };
}

/** The profile with `changes` applied, as `simulation_preview` sees it (nothing is saved). */
function previewProfile(profile: ModProfile, changes: PreviewChanges): ModProfile {
  const p = JSON.parse(JSON.stringify(profile)) as ModProfile;
  const off = new Set(p.disabled_patches ?? []);
  for (const id of changes.disable ?? []) off.add(id);
  for (const id of changes.enable ?? []) off.delete(id);
  p.disabled_patches = [...off];
  let rules = p.patch_order ?? [];
  for (const r of changes.order_rules ?? []) rules = [...rules.filter((x) => x.patch !== r.patch), r];
  p.patch_order = rules;
  p.glue_patches = [...(p.glue_patches ?? []), ...(changes.glue_patches ?? [])];
  if (changes.stacking) p.stacking = changes.stacking;
  return p;
}

/** `from`: the profile the changes start from, when fixes were added to or left out of it first. */
export function previewFor(profile: ModProfile, changes: PreviewChanges, from: ModProfile = profile): PreviewResult {
  const next = previewProfile(from, changes);
  const before = new Set(liveConflicts(profile).map((c) => c.id));
  const conflicts = liveConflicts(next);
  const after = new Set(conflicts.map((c) => c.id));
  return {
    file: changes.focus_file ? (fileDiffFor(changes.focus_file, next)?.result ?? null) : null,
    conflicts,
    resolved: [...before].filter((id) => !after.has(id)),
    new_conflicts: [...after].filter((id) => !before.has(id)),
    impact: impactFor(profile, next),
  };
}

/**
 * The files an Anchor fix new in `next` would stop from compiling: one with a rung on the line
 * `MockInit.breaksOn` names (the mock compiles nothing, §11.7).
 */
function brokenFiles(profile: ModProfile, next: ModProfile): string[] {
  const bad = state.init.breaksOn?.trim();
  if (!bad) return [];
  const had = new Set((profile.fixes ?? []).map((f) => JSON.stringify(f.change)));
  const files = (next.fixes ?? []).flatMap((f) => {
    const c = f.change;
    if (c.kind !== "anchor" || had.has(JSON.stringify(c)) || !c.anchors.some((r) => r.pattern?.trim() === bad)) return [];
    const file = find(c.patch.id)?.file;
    return file ? [file] : [];
  });
  return [...new Set(files)];
}

/** The mock's impact report: which files and patch outcomes change (nothing is compiled here). */
function impactFor(profile: ModProfile, next: ModProfile): PreviewImpact {
  const broken = brokenFiles(profile, next);
  const factions = factionChanges(profile, next);
  const files = [...new Set([...touchedFiles(profile), ...touchedFiles(next)])];
  const text = (f: string, p: ModProfile) => (touchedFiles(p).includes(f) ? run(f, p).lines.map((l) => l.text).join("\n") : null);
  const outcome = (p: ModProfile) =>
    new Map(p.enabled_mods.flatMap((id) => patchesFor(id, p).map((x) => ({ ...x, mod_id: id }))).map((x) => [`${x.target_file}|${x.patch_id}`, x]));
  const [a, b] = [outcome(profile), outcome(next)];
  const status = (s: string | undefined) => (s === "applied" ? "applied" : s === "broken" ? "zero_matches" : s === "disabled" ? "skipped_disabled" : s ? "skipped_by_policy" : null) as PatchOutcomeStatus | null;
  const patch_changes: PatchChange[] = [];
  for (const key of new Set([...a.keys(), ...b.keys()])) {
    const [x, y] = [a.get(key), b.get(key)];
    if (x?.state === y?.state) continue;
    const [target_file, patch_id] = key.split("|");
    patch_changes.push({ patch_id, mod_id: (x ?? y)!.mod_id, target_file, before: status(x?.state), after: status(y?.state), before_matches: x?.state === "applied" ? 1 : 0, after_matches: y?.state === "applied" ? 1 : 0 });
  }
  const moved = stackChanges(stacksFor(profile, null), stacksFor(next, null));
  return {
    files_changed: files.filter((f) => text(f, profile) !== text(f, next)),
    patch_changes,
    compile_errors_new: broken,
    compile_errors_existing: [],
    compile_errors_fixed: [],
    placement_new: [],
    breaks_file: broken.length > 0,
    stack_changes: moved,
    relation_warnings: relationWarnings(profile, next, moved),
    fallbacks_new: fallbacksOf(next).filter((f) => !fallbacksOf(profile).some((g) => sameFallback(f, g))),
    fallbacks_fixed: fallbacksOf(profile).filter((f) => !fallbacksOf(next).some((g) => sameFallback(f, g))),
    fix_status_changes: [],
    ...(factions.length > 0 ? { faction_changes: factions } : {}),
  };
}

/** The factions of `MockInit.factionOrder`, by mod id. */
const FACTIONS: Record<string, { side: string; id: string; name: string }> = {
  "vehicle-pack": { side: "allied", id: "red_army", name: "Red Army" },
  "ai-tweaks": { side: "allied", id: "blue_army", name: "Blue Army" },
};

/** The faction each side plays as (`choose_active`): the enabled faction mod with the highest priority, then the larger id. */
function activeFactions(p: ModProfile): Map<string, FactionPick> {
  const best = new Map<string, { prio: number; id: string; pick: FactionPick }>();
  for (const id of p.enabled_mods) {
    const f = FACTIONS[id];
    if (!f) continue;
    const prio = p.mod_priorities?.[id] ?? mods.find((m) => m.id === id)?.load_priority ?? 50;
    const cur = best.get(f.side);
    if (!cur || prio > cur.prio || (prio === cur.prio && id >= cur.id)) best.set(f.side, { prio, id, pick: { mod_id: id, faction_id: f.id, name: f.name } });
  }
  return new Map([...best].map(([side, v]) => [side, v.pick]));
}

/** Sides whose faction the change switches (D73); none without `MockInit.factionOrder`. */
export function factionChanges(profile: ModProfile, next: ModProfile): FactionChange[] {
  if (!state.init.factionOrder) return [];
  const [was, will] = [activeFactions(profile), activeFactions(next)];
  return ["allied", "axis"].flatMap((side) => {
    const [before, after] = [was.get(side) ?? null, will.get(side) ?? null];
    return JSON.stringify(before) === JSON.stringify(after) ? [] : [{ side, before, after }];
  });
}

/** Places where an Anchor fix's patch uses its own line, in every touched file. */
function fallbacksOf(profile: ModProfile): FallbackNotice[] {
  return touchedFiles(profile).flatMap((f) => run(f, profile).fallbacks);
}

const sameFallback = (a: FallbackNotice, b: FallbackNotice) => a.file === b.file && a.patch_id === b.patch_id && a.site_line === b.site_line;

/**
 * Order rules new in `next` that run a mod's patch before a mod it depends on (D39), and new
 * stack rules that put a mod's block above the block of a mod it depends on (D56): the pair the
 * rule names, and like the service every pair `moved` turns around.
 */
function relationWarnings(profile: ModProfile, next: ModProfile, moved: StackChange[]): RelationWarning[] {
  const was = new Set((profile.patch_order ?? []).map((r) => JSON.stringify(r)));
  const deps = dependsOn(next.enabled_mods);
  const modName = (id: string) => mods.find((m) => m.id === id)?.name ?? id;
  const out: RelationWarning[] = [];
  const warn = (rule: OrderRule, first: string | undefined, then: string | undefined) => {
    if (!first || !then || !deps(first, then)) return;
    const verb = MOD_RELATIONS[first]?.requires.includes(then) ? "requires" : "loads after";
    const text = `${modName(first)} ${verb} ${modName(then)}; this runs ${modName(first)}'s patch before ${modName(then)}'s`;
    if (!out.some((w) => w.rule === rule.patch && w.text === text)) out.push({ rule: rule.patch, mod_id: first, depends_on: then, text });
  };
  for (const rule of (next.patch_order ?? []).filter((r) => !was.has(JSON.stringify(r)))) {
    const subject = find(rule.patch)?.mod;
    for (const b of rule.before ?? []) warn(rule, subject, find(b)?.mod);
    for (const a of rule.after ?? []) warn(rule, find(a)?.mod, subject);
  }
  // D56: a new stack rule that puts a mod's block above the block of a mod it depends on.
  const stackWarn = (rule: string, upper: string | undefined, lower: string | undefined) => {
    if (!upper || !lower || upper === lower || !deps(upper, lower)) return;
    if (out.some((w) => w.mod_id === upper && w.depends_on === lower && w.text.includes("'s block above"))) return;
    const verb = MOD_RELATIONS[upper]?.requires.includes(lower) ? "requires" : "loads after";
    const text = `${modName(upper)} ${verb} ${modName(lower)}; putting ${modName(upper)}'s block above ${modName(lower)}'s may break it`;
    out.push({ rule, mod_id: upper, depends_on: lower, text });
  };
  const rules: string[] = [];
  const had = new Set((profile.fixes ?? []).map((f) => f.id));
  for (const f of (next.fixes ?? []).filter((x) => !had.has(x.id) && !x.paused)) {
    const c = f.change;
    if (c.kind !== "stack_rule") continue;
    const me = find(c.patch.id)?.mod;
    const pairs: [string | undefined, string | undefined][] = [];
    if (c.below) pairs.push([find(c.below.id)?.mod, me]);
    if (c.above) pairs.push([me, find(c.above.id)?.mod]);
    for (const [upper, lower] of pairs) stackWarn(c.patch.id, upper, lower);
    rules.push(c.patch.id);
  }
  // Each pair of blocks a new stack rule turns around: the upper one did not run first before.
  if (rules.length > 0) {
    for (const c of moved) {
      const at = (list: StackBlock[], id: string) => list.findIndex((b) => b.patch_id === id);
      c.after.forEach((upper, i) => {
        for (const lower of c.after.slice(i + 1)) {
          const bu = at(c.before, upper.patch_id);
          const bl = at(c.before, lower.patch_id);
          if (bu < 0 || bl < 0 || bu < bl) continue;
          const rule = rules.find((r) => r === upper.patch_id || r === lower.patch_id) ?? rules[0];
          stackWarn(rule, upper.mod_id, lower.mod_id);
        }
      });
    }
  }
  return out;
}

/**
 * What the simulation says about StackRule fix `id` (D55, D56): it placed or held a block
 * (`held`), a stronger rule beat it (`overridden`, with the reason), or its two blocks never
 * meet at its line (`waiting`).
 */
export function stackRuleState(profile: ModProfile, id: string): { state: "held" } | { state: "overridden"; reason: string } | { state: "waiting" } {
  const moves = touchedFiles(profile).flatMap((f) => run(f, profile).records.flatMap((r) => r.stack_moves ?? []));
  if (moves.some((m) => m.reason.kind === "user_rule" && m.reason.fix_id === id)) return { state: "held" };
  const beaten = moves.flatMap((m) => m.overridden ?? []).find((o) => o.startsWith(`Your fix ${id} to put`));
  return beaten ? { state: "overridden", reason: beaten } : { state: "waiting" };
}

/** Stacks in both runs whose shared patches run in a different order, matched by file, side,
 *  line text and place among stacks with the same text (like the service). */
function stackChanges(was: SharedStack[], will: SharedStack[]): StackChange[] {
  const index = (list: SharedStack[]) => {
    const seen = new Map<string, number>();
    return new Map(
      list.map((s) => {
        const k = `${s.target_file}|${s.side}|${s.anchor_text}`;
        const n = seen.get(k) ?? 0;
        seen.set(k, n + 1);
        return [`${k}|${n}`, s] as const;
      }),
    );
  };
  const [a, b] = [index(was), index(will)];
  const out: StackChange[] = [];
  for (const [key, x] of a) {
    const y = b.get(key);
    if (!y) continue;
    const [ix, iy] = [x.blocks.map((v) => v.patch_id), y.blocks.map((v) => v.patch_id)];
    const common = (v: string[], o: string[]) => v.filter((id) => o.includes(id)).join(",");
    if (common(ix, iy) !== common(iy, ix)) {
      out.push({ file: y.target_file, line: y.file_line, side: y.side, anchor_text: y.anchor_text, before: x.blocks, after: y.blocks });
    }
  }
  return out.sort((p, q) => (p.file < q.file ? -1 : p.file > q.file ? 1 : p.line - q.line));
}

export function anchorCandidatesFor(
  file: string, line: number, radius: number, profile: ModProfile, region: RegionRef | null = null,
): AnchorCandidate[] | null {
  if (!touchedFiles(profile).includes(file)) return null;
  const r = run(file, profile);
  if (region) {
    const b = find(region.patch_id);
    const site = b ? bSites(r, b).find((s) => s.site === region.site) : undefined;
    return site ? site.lines.filter((l) => l.text.trim() !== "") : null;
  }
  const lines = r.lines;
  const all = lines.map((l) => l.text);
  const lo = Math.max(1, line - radius);
  const hi = Math.min(all.length, line + radius);
  const out: AnchorCandidate[] = [];
  for (let n = lo; n <= hi; n++) {
    const text = all[n - 1];
    if (text.trim() === "") continue;
    const by = lines[n - 1];
    out.push({
      line: n, text, matches_in_file: all.filter((t) => t.trim() === text.trim()).length, raw_text: text,
      origin: by.patch_id && by.mod_id ? { kind: "inserted", patch_id: by.patch_id, mod_id: by.mod_id } : null,
      matches_in_region: null,
    });
  }
  return out;
}

/**
 * Whether code can go at each edge of `own` (B's lines), like `placement::edge_checks`: a change
 * that leaves a block open has no usable edge, and the end edge says which block.
 */
export function edgeChecks(own: string[], bName: string): [EdgeCheck, EdgeCheck] {
  const open: string[] = [];
  let below = false;
  for (const raw of own) {
    const t = raw.trim();
    if (/^end\b/.test(t)) {
      if (open.length === 0) below = true;
      else open.pop();
      continue;
    }
    if (/^(elseif|else)\b/.test(t)) continue;
    if (/\b(then|do)$/.test(t) || /\bfunction\b.*\)$/.test(t)) open.push(t);
  }
  if (open.length === 0 && !below) return [{ allowed: true, reason: null }, { allowed: true, reason: null }];
  const top = open[open.length - 1];
  const cond = top ? /^(?:if|elseif|while)\s+(.+?)\s+(?:then|do)$/.exec(top)?.[1] : undefined;
  const end: EdgeCheck = {
    allowed: false,
    reason: cond ? `would run only when ${cond}` : top ? `would land inside a block ${bName} leaves open` : `would land outside the block around ${bName}'s change`,
  };
  return [{ allowed: false, reason: `${bName} does not close every block it opens, so its edges cannot be used` }, end];
}

/** The function a line is in ("" at the top level). */
function functionAt(lines: AttributedCodeLine[], at: number): string {
  for (let i = at; i >= 0; i--) {
    const t = lines[i].text;
    if (t === "end") return "";
    const m = /^(?:local\s+)?function\s+([\w.:]+)/.exec(t);
    if (m) return m[1];
  }
  return "";
}

/** B's places in a run: one per contiguous stretch of its lines (the mock's patches match once). */
function bSites(r: Run, b: MockPatch): RegionSite[] {
  const mine = r.lines.flatMap((l, i) => (l.patch_id === b.id ? [i] : []));
  if (mine.length === 0) return [];
  const first = mine[0];
  const last = mine[mine.length - 1];
  const own = mine.map((i) => r.lines[i].text);
  const count = (t: string) => own.filter((x) => x.trim() === t.trim()).length;
  const [edge_start, edge_end] = edgeChecks(own, modNameOf(b.mod));
  const lines: AnchorCandidate[] = [];
  for (let i = first; i <= last; i++) {
    const l = r.lines[i];
    const inB = l.patch_id === b.id;
    lines.push({
      line: i + 1, text: l.text, raw_text: l.text,
      matches_in_file: r.lines.filter((x) => x.text.trim() === l.text.trim()).length,
      origin: l.patch_id && l.mod_id ? { kind: "inserted", patch_id: l.patch_id, mod_id: l.mod_id } : null,
      matches_in_region: inB ? count(l.text) : null,
    });
  }
  return [{ site: 0, first_line: first + 1, last_line: last + 1, function: functionAt(r.lines, first), lines, edge_start, edge_end }];
}

/** A patch's own places in the game's file (D25). */
function aSites(p: MockPatch): ASite[] {
  return (ORIGINALS[p.file] ?? []).flatMap((text, i) =>
    text.trim() === p.anchor.trim() ? [{ key: { context: p.anchor.trim(), ordinal: 1 }, line: i + 1, anchor_line: i + 1, text }] : [],
  );
}

/**
 * Region options (§9.2) from the conflict's consumer. Options are not previewed here: the handler
 * greys the ones whose preview fails. An edge the other patch leaves open is greyed already.
 */
export function regionOptionsFor(file: string, patchId: string, profile: ModProfile, conflictId: string | null = null): RegionOptions | null {
  const p = find(patchId);
  if (!p || p.file !== file) return null;
  const at = p.position === "at";
  const r = touchedFiles(profile).includes(file) ? run(file, profile) : null;
  const c = r?.conflicts.find((x) => x.patch_a_id === p.id && x.consumer && (conflictId === null || x.id === conflictId));
  const consumer = c?.consumer ?? null;
  const b = consumer ? find(consumer.patch_id) : undefined;
  const sites = b && r ? bSites(r, b) : [];
  const site = sites[0];
  const bName = b ? modNameOf(b.mod) : "";
  const options: RegionOption[] = [];
  if (consumer?.kind === "split" && b) {
    // The whole patch runs as separate edits and skips the other mod's lines (§4.8).
    const key = `region:split:${b.id}:edits:${site ? site.site : "-"}`;
    const label = `Run ${p.name} as separate edits`;
    const pattern = p.anchor.split("\n");
    const gaps = gapsOf(pattern, p.payload, keptPairs(pattern, p.payload));
    const g = gaps.length === 1 && gaps[0].p[1] === gaps[0].p[0] ? gaps[0] : null;
    const n = g ? g.q[1] - g.q[0] : 0;
    const where = g ? (g.after !== null ? `after "${pattern[g.after].trim()}"` : `before "${pattern[0].trim()}"`) : "";
    const note = !at
      ? `${modNameOf(p.mod)}'s lines find their place again around ${bName}'s lines, which stay`
      : g
        ? `${modNameOf(p.mod)}'s ${n === 1 ? "1 new line goes" : `${n} new lines go`} in ${where}; ${bName}'s lines next to ${n === 1 ? "it" : "them"} stay`
        : `${modNameOf(p.mod)}'s lines the game already has stay as they are; ${bName}'s lines between them stay`;
    options.push({
      key, label, kind: "split",
      draft: { change: { kind: "apply_as", patch_id: p.id, mode: "edits" }, label, source: "resolver", conflict_id: c?.id ?? null, option_key: key, approved: null },
      placement_ok: true, placement_reason: null, suggested: true, note, old_text: null, new_text: null,
    });
  }
  const draft = (key: string, label: string, anchors: Rung[]): FixDraft => ({
    change: { kind: "anchor", patch_id: p.id, anchors, sites: [{ context: p.anchor.trim(), ordinal: 1 }] },
    label, source: "resolver", conflict_id: c?.id ?? null, option_key: key, approved: null,
  });
  const option = (variant: string, label: string, anchors: Rung[] | null, gate?: EdgeCheck): RegionOption => {
    const key = `region:${consumer?.kind}:${b?.id}:${variant}:${site ? site.site : "-"}`;
    return {
      key, label, kind: consumer?.kind ?? "", draft: anchors ? draft(key, label, anchors) : null,
      placement_ok: gate?.allowed ?? true, placement_reason: gate && !gate.allowed ? gate.reason : null,
      suggested: false, note: null, old_text: null, new_text: null,
    };
  };
  if (consumer && b && !at) {
    const within = b.id;
    const bSite = site ? site.site : null;
    switch (consumer.kind) {
      case "reemitted":
        options.push({ ...option("within", `Keep it next to ${bName}'s copy of that line`, [{ within, pattern: consumer.old_text.trim(), exact: true, b_site: bSite }]), suggested: true });
        break;
      case "rewritten_1to1": {
        const next = (consumer.new_text ?? "").trim();
        if (next) {
          options.push({
            ...option("follow", `Follow ${bName}'s new version of that line`, [{ within, pattern: next, exact: true, b_site: bSite }]),
            old_text: consumer.old_text, new_text: next,
          });
        }
        break;
      }
      case "replaced_by_block":
      case "deleted": {
        const note = consumer.kind === "deleted" ? `${bName} removed this line; placing next to its change is a guess` : null;
        const none: EdgeCheck = { allowed: false, reason: `${bName}'s change is no longer in ${file}` };
        options.push({ ...option("edge_end", `Put it after ${bName}'s change`, [{ within, edge: "end", b_site: bSite }], site?.edge_end ?? none), note });
        options.push({ ...option("edge_start", `Put it before ${bName}'s change`, [{ within, edge: "start", b_site: bSite }], site?.edge_start ?? none), note });
        if (consumer.kind === "replaced_by_block") options.push(option("pick", `Pick a line inside ${bName}'s change...`, null));
        break;
      }
      case "split":
        break;
    }
  }
  options.sort((x, y) => Number(y.suggested) - Number(x.suggested));
  return {
    target_file: file, patch_id: p.id, consumer, a_is_at: at, move_by_copy: at,
    b_patch_id: b?.id ?? null, b_mod_id: b?.mod ?? null, options, sites, a_sites: aSites(p),
  };
}

/** Why an option's preview fails, like the service's region verdict; null when it places the code. */
export function regionVerdict(file: string, patchId: string, next: ModProfile): string | null {
  if (!touchedFiles(next).includes(file)) return "its code would not be placed there";
  const rec = run(file, next).records.find((x) => x.patch_id === patchId);
  const s = rec?.anchoring?.sites[0];
  // Separate edits have no rung: the patch must apply.
  if (rec && !rec.anchoring) return rec.status === "applied" ? null : "it would still not find its lines";
  if (!rec || !s) return "its code would not be placed there";
  return s.rung === null ? (s.fallback ?? "it would use its own line") : null;
}

/** Why an Anchor fix's patch uses its own line now, or null. */
export function anchorFallback(profile: ModProfile, patchId: string): string | null {
  const p = find(patchId);
  if (!p || !touchedFiles(profile).includes(p.file)) return null;
  return run(p.file, profile).fallbacks.find((f) => f.patch_id === p.id)?.reason ?? null;
}

/** `glue_patch_create`: a copy of `baseId` on another line. A copy of a copy keeps the older copy's name and base mod
 *  and replaces the older copy, as the service does (D59). */
export function glueFor(baseId: string, pattern: string, position: "at" | "before" | "after", profile: ModProfile): GluePatchEntry | null {
  const copies = allCopies(profile);
  const older = copies.find((g) => g.id === baseId) ?? null;
  const root = older ? rootGlue(copies, older) : null;
  const base = find(root ? (root.replaces_patch_id ?? "") : baseId);
  if (!base) return null;
  const stem = older ? (older.replaces_patch_id ?? baseId) : base.id;
  let id = `glue:${stem}`;
  for (let n = 2; copies.some((g) => g.id === id); n++) id = `glue:${stem}:${n}`;
  const payload = older && "pattern" in older.patch ? older.patch.pattern.payload : base.payload.join("\n");
  const baseMod = older ? (older.base_mod_id ?? null) : base.mod;
  return {
    id,
    name: older ? older.name : `Adjusted copy of ${base.name}`,
    replaces_patch_id: older ? older.id : base.id,
    priority: 50,
    patch: {
      pattern: {
        pattern, position, target: base.file, payload, times: 1,
        id: null, name: null, requires: base.requires ?? [], priority: null, before: [], after: [], on_failure: "warn",
      },
    },
    ...(baseMod ? { base_mod_id: baseMod } : {}),
    // New copies run as their base mod and match their line exactly (D58, D59).
    run_as: "base",
    exact: true,
  };
}
