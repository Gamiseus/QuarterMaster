// Compatibility mods in the mock (§8, §11.7): the AI Compat mod with one applied and one
// rejected override and a skipped author rung, and "Save as compatibility patch".
import type {
  CompatChangesView, CompatEquivalence, CompatExportPreview, CompatExportRequest, CompatExportResult, CompatRefusal,
  CompatRequirement, ModProfile, ModSummary, ProfileFix,
} from "../bindings";
import { mods, patchesByMod } from "./fixtures";
import * as ledger from "./ledger";
import { COMPAT_OVERRIDES, compatLayer, findPatch, MOCK_PATCHES, MOD_RELATIONS, profileTexts, SKIPPED_RUNGS, type MockOverride } from "./patchworld";
import { fail, log, state } from "./state";

/** The mock compatibility mod of `MockInit.compatMod`. */
export const COMPAT_MOD = "ai-compat";

const summary = (id: string, name: string, priority: number, patches: number): ModSummary => ({
  id, name, version: "1.0.0", authors: ["Unknown Author"], category: "Other", enabled: true, patch_count: patches, installed_versions: ["1.0.0"],
  is_workspace_linked: false, has_config: false, load_priority: priority, kind: "managed", faction: null,
});

/**
 * AI Compat requires GCore and AI Tweaks. Its override that runs AI init after Core init applies; the
 * one that turns off Vehicle Pack's patch is ignored, because it does not require Vehicle Pack. Its
 * own Threat legend patch names Tactical HUD's Minimap without requiring it, so it uses its own line.
 */
export function addCompatScenario(): boolean {
  if (mods.some((m) => m.id === COMPAT_MOD)) return false;
  mods.push(summary(COMPAT_MOD, "AI Compat", 70, 1));
  MOD_RELATIONS[COMPAT_MOD] = { requires: ["gcore", "ai-tweaks"], load_after: ["gcore", "ai-tweaks"], load_before: [] };
  COMPAT_OVERRIDES[COMPAT_MOD] = [
    { patch: "ai-tweaks:ai-init", after: ["gcore:core-init"] },
    { patch: "vehicle-pack:vehicle-init", disable: true },
  ];
  MOCK_PATCHES.push({ id: "threat-legend", mod: COMPAT_MOD, name: "Threat legend", file: "scripts/ui/hud.lua", anchor: "  drawAmmo()", position: "after", payload: ["  drawThreatLegend()"] });
  patchesByMod[COMPAT_MOD] = [{ patch_id: "threat-legend", name: "Threat legend", target_file: "scripts/ui/hud.lua", state: "applied", conflicts: [] }];
  SKIPPED_RUNGS[COMPAT_MOD] = [{
    patch_id: "threat-legend", rung: 0, reason: "not_required", source: "author",
    detail: "AI Compat places code inside Tactical HUD's change but does not require Tactical HUD; QuarterMaster uses its own line",
  }];
  return true;
}

/** `compat_changes_get`: what the mod's overrides did, and its rungs that are never used. */
export function compatChanges(modId: string, profile: ModProfile): CompatChangesView {
  const mine = compatLayer(profile).reports.filter((r) => r.from_mod === modId);
  return {
    applied: mine.filter((r) => r.status === "applied"),
    rejected: mine.filter((r) => r.status !== "applied"),
    rejected_rungs: profile.enabled_mods.includes(modId) ? (SKIPPED_RUNGS[modId] ?? []) : [],
  };
}

const modName = (id: string) => mods.find((m) => m.id === id)?.name ?? id;
const modOf = (name: string) => findPatch(name)?.mod ?? (name.includes(":") ? name.split(":")[0] : name);

type Why = "changes" | "places_next" | "orders" | "stacks" | "keeps_change";
type Part = { fix: ProfileFix; entry: MockOverride; target: string; refs: [string, Why][] };

/** One fix as an override entry, or why it cannot be one. */
function partOf(p: ModProfile, f: ProfileFix): Part | string {
  const v = ledger.fixView(p, f.id);
  if (!v.can_export) return v.export_blocked_reason ?? "This fix cannot be saved as a compatibility patch.";
  const c = f.change;
  const subject = c.kind === "order" ? c.rule.patch : "patch" in c ? c.patch.id : "";
  const patch = findPatch(subject);
  if (!patch) return "The patch it changes is not loaded.";
  const entry: MockOverride = { patch: `${patch.mod}:${patch.id}` };
  const refs: [string, Why][] = [[patch.mod, "changes"]];
  if (c.kind === "disable") entry.disable = true;
  else if (c.kind === "priority") entry.priority = c.priority;
  else if (c.kind === "order") {
    entry.before = c.rule.before ?? [];
    entry.after = c.rule.after ?? [];
    for (const n of [...entry.before, ...entry.after]) refs.push([modOf(n), "orders"]);
  } else if (c.kind === "anchor") {
    entry.anchors = structuredClone(c.anchors);
    entry.sites = structuredClone(c.sites ?? []);
    for (const r of c.anchors) if (r.within) refs.push([modOf(r.within), "places_next"]);
  } else if (c.kind === "stack_rule") {
    // The other block's patch, named the way overrides name patches.
    const named = (id: string | undefined) => {
      const o = id ? findPatch(id) : undefined;
      return o ? `${o.mod}:${o.id}` : undefined;
    };
    if ((c.below && !named(c.below.id)) || (c.above && !named(c.above.id))) return "The other patch it names is not loaded.";
    entry.stack_below = named(c.below?.id);
    entry.stack_above = named(c.above?.id);
    entry.stack_site = structuredClone(c.site);
    entry.stack_side = c.side;
    for (const n of [entry.stack_below, entry.stack_above]) if (n) refs.push([modOf(n), "stacks"]);
  } else if (c.kind === "apply_as") {
    entry.apply_as = c.mode;
  } else if (c.kind === "skip_hunk") {
    // The patch whose change stays, named the way overrides name patches (D49).
    const keep = c.hunk.keep ? findPatch(c.hunk.keep) : undefined;
    if (c.hunk.keep && !keep) return "The other patch it names is not loaded.";
    entry.skip_hunks = [{ ...structuredClone(c.hunk), keep: keep ? `${keep.mod}:${keep.id}` : null }];
    if (keep) refs.push([keep.mod, "keeps_change"]);
  }
  return { fix: f, entry, target: patch.mod, refs };
}

/** `compat_<a>_<b>`, sanitized to `^[a-z0-9_]{3,48}$`, with `_2`, `_3`, ... when taken (like the service). */
export function uniqueId(a: string, b: string | null, taken: Set<string>): string {
  const base = (b ? `compat_${a}_${b}` : `compat_${a}`).toLowerCase().replace(/[^a-z0-9]/g, "_").replace(/_+/g, "_");
  const fit = (s: string, room: number) => s.slice(0, room).replace(/_+$/, "");
  const first = fit(base, 48);
  if (!taken.has(first)) return first;
  for (let n = 2; ; n++) {
    const id = `${fit(base, 48 - String(n).length - 1)}_${n}`;
    if (!taken.has(id)) return id;
  }
}

/** The user's name for the mod: one line, no path characters, at most 64 characters. */
export function cleanName(s: string): string | null {
  const kept = s.replace(/[\\/:*?"<>|]/g, "").replace(/[\u0000-\u001f]/g, " ").split(/\s+/).filter(Boolean).join(" ");
  const name = kept.replace(/^[. ]+/, "").slice(0, 64).trim();
  return name || null;
}

/** A TOML value; strings in JSON form are valid basic TOML strings. */
function toml(v: unknown): string {
  if (Array.isArray(v)) return `[${v.map(toml).join(", ")}]`;
  if (v !== null && typeof v === "object") {
    const kv = Object.entries(v).filter(([, x]) => x !== null && x !== undefined);
    return `{ ${kv.map(([k, x]) => `${k} = ${toml(x)}`).join(", ")} }`;
  }
  return typeof v === "string" ? JSON.stringify(v) : String(v);
}

function tomlFor(name: string, profile: string, parts: Part[]): string {
  const out = [
    `# ${name}`,
    `# A compatibility patch QuarterMaster made from fixes in the profile "${profile}".`,
    "# Each [[overrides]] entry is one fix. QuarterMaster applies an entry only while this",
    "# mod requires every mod the entry refers to.",
    "",
    "[manifest]",
    'version = "1.0.0"',
  ];
  for (const x of parts) {
    out.push("", `# ${x.fix.label}`, "[[overrides]]");
    for (const [k, v] of Object.entries(x.entry)) if (v !== undefined && !(Array.isArray(v) && v.length === 0)) out.push(`${k} = ${toml(v)}`);
  }
  return `${out.join("\n")}\n`;
}

function readmeFor(name: string, profile: string, parts: Part[], requires: CompatRequirement[]): string {
  const changes = parts.map((x) => (x.fix.label.includes(x.entry.patch) ? `- ${x.fix.label}` : `- ${x.fix.label} (${x.entry.patch})`));
  const needs = requires.map((r) => `- ${r.name}${r.range !== "*" && r.version ? ` ${r.version} or newer` : ""}: ${r.reasons.join("; ")}`);
  return [
    name, "", `A compatibility patch QuarterMaster made from fixes in the profile "${profile}".`, "", "It changes:", ...changes, "", "It needs:", ...needs,
    "", "Turn it off in QuarterMaster to stop using it.", "",
  ].join("\n");
}

/** Registers a compatibility mod with the mock engine; returns a function that takes it out again. */
function register(id: string, name: string, requires: string[], overrides: MockOverride[]): () => void {
  mods.push(summary(id, name, 100, 0));
  MOD_RELATIONS[id] = { requires: [...requires], load_after: [...requires], load_before: [] };
  COMPAT_OVERRIDES[id] = overrides;
  patchesByMod[id] = [];
  return () => {
    mods.splice(mods.findIndex((m) => m.id === id), 1);
    delete MOD_RELATIONS[id];
    delete COMPAT_OVERRIDES[id];
    delete patchesByMod[id];
  };
}

type Work = { preview: CompatExportPreview; parts: Part[]; next: ModProfile | null };

/** What saving `req` makes, checked the way activation would run it (§8.2). Nothing is kept. */
function exportWork(profile: ModProfile, req: CompatExportRequest): Work {
  if (req.fix_ids.length === 0) fail("invalid_input", "Pick at least one fix to save.");
  const p = structuredClone(profile);
  const why = ledger.migrate(p);
  if (why !== null) fail("invalid_input", `Your existing fixes could not be imported yet: ${why}`);
  const wanted = new Set(req.fix_ids);
  const refused: CompatRefusal[] = [...wanted].filter((id) => !(p.fixes ?? []).some((f) => f.id === id)).map((fix_id) => ({ fix_id, reason: "That fix is already gone." }));
  const parts: Part[] = [];
  for (const f of (p.fixes ?? []).filter((x) => wanted.has(x.id))) {
    const r = partOf(p, f);
    if (typeof r === "string") refused.push({ fix_id: f.id, reason: r });
    else parts.push(r);
  }
  const first = parts[0];
  if (!first) {
    const reason = "None of these fixes can be saved as a compatibility patch.";
    return {
      preview: { mod_id: "", name: "", version: "1.0.0", requires: [], load_after: [], files: [], equivalence: { kind: "not_checked", reason }, included: [], refused },
      parts, next: null,
    };
  }

  // The changed mods first, then the others, each with why.
  const requires: CompatRequirement[] = [];
  const refs = parts.flatMap((x) => x.refs);
  for (const [m, w] of [...refs.filter(([, w]) => w === "changes"), ...refs.filter(([, w]) => w !== "changes")]) {
    const l = modName(m);
    const reason =
      w === "changes" ? `it changes ${l}'s patch`
      : w === "places_next" ? `it places code next to ${l}'s change`
      : w === "stacks" ? `it puts its code next to ${l}'s code at a shared line`
      : w === "keeps_change" ? `it keeps ${l}'s change to a line`
      : `it runs a patch before or after ${l}'s patch`;
    const have = requires.find((r) => r.mod_id === m);
    if (have) {
      if (!have.reasons.includes(reason)) have.reasons.push(reason);
      continue;
    }
    const version = mods.find((x) => x.id === m)?.version ?? null;
    requires.push({ mod_id: m, name: l, version, range: version ? `>=${version}` : "*", reasons: [reason] });
  }
  const other = requires.map((r) => r.mod_id).find((m) => m !== first.target) ?? null;
  const modId = uniqueId(first.target, other, new Set(mods.map((m) => m.id.toLowerCase())));
  const name = cleanName(req.name ?? "") ?? `Compatibility patch for ${requires.slice(0, 2).map((r) => r.name).join(" and ")}`.slice(0, 64).trim();
  const manifest = {
    id: modId, name, version: "1.0.0", authors: [], description: "A compatibility patch QuarterMaster made from fixes in a profile.",
    category: "Other", tags: ["compatibility"],
    relationships: { requires: Object.fromEntries(requires.map((r) => [r.mod_id, r.range])), load_after: requires.map((r) => r.mod_id) },
  };

  // The profile after the export, with the mod in place of the fixes.
  const next = structuredClone(p);
  const included = new Set(parts.map((x) => x.fix.id));
  next.fixes = (next.fixes ?? []).filter((f) => !included.has(f.id));
  ledger.sync(next);
  next.enabled_mods = [...next.enabled_mods, modId];
  const before = profileTexts(profile);
  const unregister = register(modId, name, requires.map((r) => r.mod_id), parts.map((x) => x.entry));
  let after: Map<string, string>;
  try {
    after = profileTexts(next);
  } finally {
    unregister();
  }
  let files = [...new Set([...before.keys(), ...after.keys()])].filter((f) => before.get(f) !== after.get(f)).sort();
  if (state.init.exportDiffers && files.length === 0) {
    files = [...new Set(parts.map((x) => findPatch(x.entry.patch)?.file ?? ""))].filter(Boolean);
  }
  const equivalence: CompatEquivalence = files.length === 0 ? { kind: "equivalent" } : { kind: "differs", files };
  return {
    preview: {
      mod_id: modId, name, version: "1.0.0", requires, load_after: requires.map((r) => r.mod_id),
      files: [
        { name: "mod.json", text: JSON.stringify(manifest, null, 2) },
        { name: "qm.toml", text: tomlFor(name, profile.name, parts) },
        { name: "README.txt", text: readmeFor(name, profile.name, parts, requires) },
      ],
      equivalence, included: parts.map((x) => x.fix.id), refused,
    },
    parts, next,
  };
}

export function exportPreview(profile: ModProfile, req: CompatExportRequest): CompatExportPreview {
  return exportWork(profile, req).preview;
}

/** `compat_export` (D14): installs the mod, turns it on and takes out the fixes, as one undoable step. */
export function exportFixes(profile: ModProfile, req: CompatExportRequest): CompatExportResult {
  ledger.ledgerOf(profile);
  const { preview, parts, next } = exportWork(profile, req);
  if (preview.refused.length > 0) {
    fail("invalid_input", `Some fixes cannot be saved as a compatibility patch: ${preview.refused.map((r) => r.reason).join(" ")}`);
  }
  const eq = preview.equivalence;
  if (eq.kind === "differs") fail("conflict", `The compatibility patch would change the game code in ${eq.files.join(", ")}. Keep the fixes instead.`);
  if (eq.kind === "not_checked" || !next) fail("invalid_input", eq.kind === "not_checked" ? eq.reason : "Nothing to save.");
  register(preview.mod_id, preview.name, preview.requires.map((r) => r.mod_id), parts.map((x) => x.entry));
  const removed = (profile.fixes ?? []).filter((f) => preview.included.includes(f.id));
  profile.fixes = (profile.fixes ?? []).filter((f) => !preview.included.includes(f.id));
  ledger.sync(profile);
  profile.enabled_mods = [...profile.enabled_mods, preview.mod_id];
  profile.fix_journal = [
    ...(profile.fix_journal ?? []),
    { at: new Date().toISOString(), op: "compat_export", label: `Saved as compatibility patch "${preview.name}"`, removed, added: [], mods_enabled: [preview.mod_id] },
  ];
  log.compatExports.push(preview.mod_id);
  return {
    mod_id: preview.mod_id, version: preview.version, shown_path: `C:\\Users\\Player\\AppData\\Roaming\\QuarterMaster\\library\\${preview.mod_id}\\1.0.0`,
    removed_fix_ids: removed.map((f) => f.id),
  };
}

/**
 * `MockInit.exportClash` (D71): the working order fix of `driftedFix` saved as a compatibility
 * patch, then a newer rule for the same patch made since, so undoing the export asks.
 */
export function seedExportClash(p: ModProfile): void {
  if (!(p.fixes ?? []).some((f) => f.id === "fx_0a11c0ffee02")) ledger.seedDriftedFixes(p);
  exportFixes(p, { fix_ids: ["fx_0a11c0ffee02"], name: "Targeting fix" });
  ledger.createFix(p, {
    change: { kind: "order", rule: { patch: "ai-targeting", before: [], after: ["core-targets"] } },
    label: "Run Targeting after Target picker", source: "order_view", conflict_id: null,
  });
  // Seeded, not done by the user.
  log.compatExports.length = 0;
  log.fixes.length = 0;
  log.orderRules.length = 0;
}
