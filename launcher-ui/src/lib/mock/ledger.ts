// The mock's fix ledger (§3.11): the same shape as the service, on `profileDocs`. The legacy
// fields stay what the mock engine reads; every ledger write regenerates them from the fixes.
import type {
  FixChange, FixDraft, FixesView, FixSource, FixStatus, FixView, GluePatchEntry, JournalClash, JournalEntry, JournalUndoChoice, JournalUndoPreview,
  ModProfile, OrderRule, PatchLabel, PatchSnapshot, PreviewChanges, ProfileFix, Provenance, UndoPreview, UndoRecord,
} from "../bindings";
import { emitMock } from "../events";
import { liveConflicts, mods } from "./fixtures";
import { anchorFallback, copyBaseMod, copySuperseded, findPatch, previewFor, snapshotOf, stackRuleState } from "./patchworld";
import * as assets from "./assets";
import { fail, log, state } from "./state";

const IMPORT_FAILED = "Your existing fixes could not be imported yet";

const now = () => new Date().toISOString();

function newId(): string {
  const bytes = new Uint8Array(6);
  crypto.getRandomValues(bytes);
  return `fx_${[...bytes].map((b) => b.toString(16).padStart(2, "0")).join("")}`;
}

const modName = (id: string) => mods.find((m) => m.id === id)?.name ?? id;
const patchName = (id: string) => findPatch(id)?.name ?? id;

export function isMigrated(p: ModProfile): boolean {
  return (p.fixes_version ?? 0) >= 1;
}

function provenance(source: FixSource, extra: Partial<Provenance> = {}): Provenance {
  return { created_at: now(), source, conflict_id: null, conflict_kind: null, option_key: null, mods: [], approved: null, history: [], ...extra };
}

/** The patch a fix changes, when it has one (`subject`); a copy's is its base. `slotOf` keys a copy by its id. */
function subjectOf(f: ProfileFix): string | null {
  const c = f.change;
  switch (c.kind) {
    case "disable":
    case "priority":
    case "anchor":
    case "apply_as":
    case "skip_hunk":
    case "stack_rule":
      return c.patch.id;
    case "order":
      return c.rule.patch;
    case "adjusted_copy":
      return c.glue.replaces_patch_id ?? c.glue.id;
    case "mod_order":
    case "asset_choice":
      return null;
  }
}

/**
 * What only one running fix may hold (`slot`): the patch of an order rule, a priority, an
 * anchor or an ApplyAs, one edit of a patch, or the id of an adjusted copy. Null for a disable
 * or a load order change.
 */
function slotOf(f: ProfileFix): string | null {
  const c = f.change;
  switch (c.kind) {
    case "order":
      return `order:${c.rule.patch}`;
    case "priority":
    case "anchor":
    case "apply_as":
      return `${c.kind}:${c.patch.id}`;
    case "skip_hunk":
      return `hunk:${c.patch.id}#${c.hunk.ordinal}`;
    case "stack_rule":
      return `stack:${c.patch.id}#${c.site.context}#${c.site.ordinal ?? 0}#${c.side}`;
    case "adjusted_copy":
      return `adjusted_copy:${c.glue.id}`;
    case "asset_choice":
      return `asset:${c.key}`;
    case "disable":
    case "mod_order":
      return null;
  }
}

/** The running fixes other than `f` that hold what `f` would hold if it ran (`running_clashes`). */
export function runningClashes(p: ModProfile, f: ProfileFix): string[] {
  const want = slotOf(f);
  if (want === null) return [];
  return (p.fixes ?? []).filter((x) => !x.paused && x.id !== f.id && (slotOf(x) === want || opposite(f, x))).map((x) => x.id);
}

/** Like the service's `same_lines`: a trimmed pattern line of one is a line of the other. */
export function sameLines(a: string, b: string): boolean {
  if (!a || !b) return true;
  const bl = b.split("\n");
  return a.split("\n").some((x) => bl.includes(x));
}

/** Keep A against Keep B on the same lines, or two blocks' stack rules naming each other at one line and side (`opposite`). */
function opposite(a: ProfileFix, b: ProfileFix): boolean {
  const x = a.change;
  const y = b.change;
  if (x.kind === "stack_rule" && y.kind === "stack_rule") {
    const names = (r: typeof x, id: string) => r.below?.id === id || r.above?.id === id;
    const sameSite = x.site.context === y.site.context && (x.site.ordinal ?? null) === (y.site.ordinal ?? null);
    return sameSite && x.side === y.side && names(x, y.patch.id) && names(y, x.patch.id);
  }
  if (x.kind !== "skip_hunk" || y.kind !== "skip_hunk") return false;
  return x.hunk.keep === y.patch.id && y.hunk.keep === x.patch.id && sameLines(x.hunk.vanilla, y.hunk.vanilla);
}

/** The id of the patch a copy carries, if any. */
function glueIdOf(patch: GluePatchEntry["patch"]): string | null {
  if ("pattern" in patch) return patch.pattern.id ?? null;
  if ("regex" in patch) return patch.regex.id ?? null;
  if ("copy" in patch) return patch.copy.id ?? null;
  return patch.module.id ?? null;
}

function setGlueId(patch: GluePatchEntry["patch"], id: string): void {
  if ("pattern" in patch) patch.pattern.id = id;
  else if ("regex" in patch) patch.regex.id = id;
  else if ("copy" in patch) patch.copy.id = id;
  else patch.module.id = id;
}

/** The text of every patch the fix names, for drift diffs (D41). */
function snapshots(c: FixChange, at: string): Record<string, PatchSnapshot> {
  const ids =
    c.kind === "order" ? [c.rule.patch, ...(c.rule.before ?? []), ...(c.rule.after ?? [])]
    : c.kind === "adjusted_copy" ? [c.glue.replaces_patch_id ?? ""]
    : c.kind === "mod_order" || c.kind === "asset_choice" ? []
    : c.kind === "stack_rule" ? [c.patch.id, ...[c.below?.id, c.above?.id].filter((x): x is string => !!x)]
    : [c.patch.id];
  const out: Record<string, PatchSnapshot> = {};
  for (const id of ids) {
    const s = snapshotOf(id, at);
    if (s) out[id] = s;
  }
  return out;
}

function ref(id: string) {
  return { id, mod_id: findPatch(id)?.mod ?? "", fp: null };
}

function make(label: string, change: FixChange, prov: Provenance, undo: UndoRecord = "remove"): ProfileFix {
  const f: ProfileFix = {
    id: newId(), label, change, provenance: { ...prov, history: [{ at: prov.created_at, what: "created", detail: "" }] },
    paused: false, undo, snapshots: snapshots(change, prov.created_at),
  };
  // A migrated fix is stamped only when accepted (D74).
  if (prov.source !== "migrated") stamp(f);
  return f;
}

/** Game files a game update changed (`MockInit.gameUpdated`); every other file keeps its hash. */
const GAME_UPDATED = new Set<string>();

/** A game file's hash, as the service's vanilla cache gives it (opaque in the mock). */
function gameHash(file: string): string {
  return `${GAME_UPDATED.has(file) ? "v2" : "v1"}:${file}`;
}

/** The game files the patches of a fix target (`game_files`); none for a load order change. */
function gameFiles(c: FixChange): string[] {
  const ids =
    c.kind === "order" ? [c.rule.patch, ...(c.rule.before ?? []), ...(c.rule.after ?? [])]
    : c.kind === "adjusted_copy" ? [c.glue.replaces_patch_id ?? ""]
    : c.kind === "mod_order" || c.kind === "asset_choice" ? []
    : c.kind === "stack_rule" ? [c.patch.id, ...[c.below?.id, c.above?.id].filter((x): x is string => !!x)]
    : [c.patch.id];
  const files = ids.map((id) => findPatch(id)?.file).filter((x): x is string => !!x);
  if (c.kind === "adjusted_copy" && "pattern" in c.glue.patch) files.push(...[c.glue.patch.pattern.target].flat());
  return [...new Set(files)].sort();
}

/** Stamps a fix with the hashes of its game files now (D74). */
function stamp(f: ProfileFix): void {
  const files = gameFiles(f.change);
  if (files.length === 0) delete f.vanilla_hashes;
  else f.vanilla_hashes = Object.fromEntries(files.map((x) => [x, gameHash(x)]));
}

/** The stamped game files of a fix that changed since (`game_drift`). */
function gameDrift(f: ProfileFix): NonNullable<FixView["game_drift"]> {
  return Object.entries(f.vanilla_hashes ?? {})
    .filter(([file, before]) => before !== undefined && before !== gameHash(file))
    .map(([file, before]) => ({ file, before: before ?? "", after: gameHash(file) }));
}

const GAME_CHANGED = "The game changed since you made this fix";

const orderLabel = (r: OrderRule) =>
  r.before?.[0] ? `Run ${patchName(r.patch)} before ${patchName(r.before[0])}`
  : r.after?.[0] ? `Run ${patchName(r.patch)} after ${patchName(r.after[0])}`
  : `Order rule for ${patchName(r.patch)}`;

/** Rewrites the legacy fields from the fixes that are not paused (`sync_legacy`). */
export function sync(p: ModProfile): void {
  const live = (p.fixes ?? []).filter((f) => !f.paused);
  const off: string[] = [];
  const order: OrderRule[] = [];
  const prios: Record<string, number> = {};
  const glue: GluePatchEntry[] = [];
  for (const f of live) {
    const c = f.change;
    if (c.kind === "disable" && !off.includes(c.patch.id)) off.push(c.patch.id);
    if (c.kind === "order") order.push(structuredClone(c.rule));
    if (c.kind === "priority") prios[c.patch.id] = c.priority;
    if (c.kind === "adjusted_copy") {
      glue.push(structuredClone(c.glue));
      const base = c.glue.replaces_patch_id;
      if (c.disables_base && base && !off.includes(base)) off.push(base);
    }
  }
  p.disabled_patches = off;
  p.patch_order = order;
  p.patch_priorities = prios;
  p.glue_patches = glue;
}

/** `migration_error` after the user undid the import: no import until "Retry import". */
export const IMPORT_UNDONE = "You undid the import. Use Retry import to import them again.";

/**
 * Imports the legacy fields as fixes, once (§7.4). With `MockInit.migrationError` it fails like an
 * aborted migration and returns the reason.
 */
export function migrate(p: ModProfile): string | null {
  if (isMigrated(p)) return null;
  if (p.migration_error === IMPORT_UNDONE) return IMPORT_UNDONE;
  if (state.init.migrationError) return state.init.migrationError;
  const at = now();
  const prov = provenance("migrated", { created_at: at });
  const fixes: ProfileFix[] = [];
  const off = new Set(p.disabled_patches ?? []);
  for (const g of p.glue_patches ?? []) {
    const base = g.replaces_patch_id;
    const disables = base !== null && base !== undefined && off.has(base);
    if (disables && base) off.delete(base);
    fixes.push(make(g.name, { kind: "adjusted_copy", glue: structuredClone(g), base: base ? ref(base) : null, disables_base: disables }, prov));
  }
  for (const id of off) fixes.push(make(`Turn off ${patchName(id)}`, { kind: "disable", patch: ref(id) }, prov));
  for (const r of p.patch_order ?? []) fixes.push(make(orderLabel(r), { kind: "order", rule: structuredClone(r) }, prov));
  for (const [id, n] of Object.entries(p.patch_priorities ?? {})) {
    if (n !== undefined) fixes.push(make(`Set the priority of ${patchName(id)} to ${n}`, { kind: "priority", patch: ref(id), priority: n }, prov));
  }
  for (const [before, moved] of Object.entries(p.user_load_after ?? {})) {
    for (const m of moved ?? []) {
      fixes.push(make(`Load ${modName(m)} before ${modName(before)}`, { kind: "mod_order", move_mod_id: m, before_mod_id: before, priority: null }, prov, {
        restore_mod_order: { prev_priority: null, added_load_after: true },
      }));
    }
  }
  p.fixes = fixes;
  p.fixes_version = 1;
  p.migration_error = null;
  p.fix_journal = p.fix_journal ?? [];
  if (fixes.length > 0) {
    p.fix_journal.push({ at, op: "migrated", label: "Imported your earlier fixes", removed: [], added: fixes.map((f) => f.id) });
  } else {
    p.notices_seen = [...new Set([...(p.notices_seen ?? []), "fixes_import_v1"])];
  }
  sync(p);
  return null;
}

/** The live conflict a resolver suggestion came from, like the service's lookup. */
export function conflictOf(p: ModProfile, suggestion: unknown): Partial<Provenance> {
  const want = JSON.stringify(suggestion);
  const c = liveConflicts(p).find((x) => x.suggestions.some((s) => JSON.stringify(s) === want));
  return c ? { conflict_id: c.id, conflict_kind: c.kind } : {};
}

export { provenance };

/** The kind of the live conflict `id`. */
export function conflictKind(p: ModProfile, id: string): string | null {
  return liveConflicts(p).find((x) => x.id === id)?.kind ?? null;
}

/** A write path: migrates first, or refuses with the reason (D38). */
export function ledgerOf(p: ModProfile): ModProfile {
  const why = migrate(p);
  if (why !== null) fail("invalid_input", `${IMPORT_FAILED}: ${why}`);
  p.fixes = p.fixes ?? [];
  return p;
}

function push(p: ModProfile, f: ProfileFix): string {
  p.fixes = [...(p.fixes ?? []), f];
  sync(p);
  return f.id;
}

function remove(p: ModProfile, id: string): ProfileFix | undefined {
  const f = (p.fixes ?? []).find((x) => x.id === id);
  p.fixes = (p.fixes ?? []).filter((x) => x.id !== id);
  return f;
}

// Legacy command writes (§10): on a migrated profile they record fixes; on one that cannot be
// migrated they change the legacy fields as before.

export function opDisable(p: ModProfile, id: string, prov: Provenance, label = ""): string | null {
  if (migrate(p) !== null) {
    p.disabled_patches = [...new Set([...(p.disabled_patches ?? []), id])];
    return null;
  }
  const isOff = (f: ProfileFix) => f.change.kind === "disable" && f.change.patch.id === id;
  const have = (p.fixes ?? []).find((f) => !f.paused && isOff(f));
  if (have) return have.id;
  // A paused disable of the patch runs again.
  const paused = (p.fixes ?? []).find((f) => f.paused && isOff(f));
  if (paused) {
    paused.paused = false;
    paused.provenance.history = [...(paused.provenance.history ?? []), { at: now(), what: "resumed", detail: "turned off again" }];
    sync(p);
    return paused.id;
  }
  return push(p, make(label || `Turn off ${patchName(id)}`, { kind: "disable", patch: ref(id) }, prov));
}

export function opEnable(p: ModProfile, id: string): void {
  if (migrate(p) !== null) {
    p.disabled_patches = (p.disabled_patches ?? []).filter((d) => d !== id);
    return;
  }
  p.fixes = (p.fixes ?? []).filter((f) => !(f.change.kind === "disable" && f.change.patch.id === id));
  // A copy of the patch no longer turns it off.
  for (const f of p.fixes) {
    if (f.change.kind === "adjusted_copy" && f.change.glue.replaces_patch_id === id) f.change.disables_base = false;
  }
  sync(p);
}

export function opOrderSet(p: ModProfile, rule: OrderRule, prov: Provenance, label = ""): string | null {
  log.orderRules.push(structuredClone(rule));
  if (migrate(p) !== null) {
    p.patch_order = [...(p.patch_order ?? []).filter((r) => r.patch !== rule.patch), rule];
    return null;
  }
  // A paused rule stays as it is (paused): undo of the new one must not turn it on.
  let previous: OrderRule | null = null;
  for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "order" && f.change.rule.patch === rule.patch)) {
    remove(p, f.id);
    if (f.change.kind === "order") previous = f.change.rule;
  }
  const undo: UndoRecord = previous ? { restore_order: previous } : "remove";
  return push(p, make(label || orderLabel(rule), { kind: "order", rule: structuredClone(rule) }, prov, undo));
}

export function opOrderReset(p: ModProfile, patch: string): void {
  if (migrate(p) !== null) {
    p.patch_order = (p.patch_order ?? []).filter((r) => r.patch !== patch);
    return;
  }
  p.fixes = (p.fixes ?? []).filter((f) => !(f.change.kind === "order" && f.change.rule.patch === patch));
  sync(p);
}

export function opGlueAdd(p: ModProfile, glue: GluePatchEntry, disablesBase: boolean, prov: Provenance): string | null {
  const base = glue.replaces_patch_id;
  if (migrate(p) !== null) {
    p.glue_patches = [...(p.glue_patches ?? []).filter((g) => g.id !== glue.id), glue];
    if (disablesBase && base) p.disabled_patches = [...new Set([...(p.disabled_patches ?? []), base])];
    return null;
  }
  const alreadyOff = base ? (p.disabled_patches ?? []).includes(base) : false;
  for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "adjusted_copy" && f.change.glue.id === glue.id)) remove(p, f.id);
  // A paused copy with this id stays (paused); the new copy gets an id of its own, and so does
  // its patch when it carried the copy's id.
  const inLedger = (id: string) => (p.fixes ?? []).some((f) => f.change.kind === "adjusted_copy" && f.change.glue.id === id);
  if (inLedger(glue.id)) {
    const old = glue.id;
    let n = 2;
    while (inLedger(`${old}:${n}`)) n++;
    glue.id = `${old}:${n}`;
    if (glueIdOf(glue.patch) === old) setGlueId(glue.patch, glue.id);
  }
  const change: FixChange = { kind: "adjusted_copy", glue, base: base ? ref(base) : null, disables_base: disablesBase };
  return push(p, make(glue.name, change, prov, disablesBase && alreadyOff ? "keep_disable" : "remove"));
}

export function opGlueRemove(p: ModProfile, glueId: string): boolean {
  if (migrate(p) !== null) {
    const glue = (p.glue_patches ?? []).find((g) => g.id === glueId);
    if (!glue) return false;
    p.glue_patches = (p.glue_patches ?? []).filter((g) => g.id !== glueId);
    if (!p.glue_patches.some((g) => g.replaces_patch_id === glue.replaces_patch_id)) {
      p.disabled_patches = (p.disabled_patches ?? []).filter((d) => d !== glue.replaces_patch_id);
    }
    return true;
  }
  const f = (p.fixes ?? []).find((x) => x.change.kind === "adjusted_copy" && x.change.glue.id === glueId);
  if (!f) return false;
  remove(p, f.id);
  sync(p);
  return true;
}

/** Tier 1 "Load X before Y" (D36). */
export function opModOrder(p: ModProfile, move: string, before: string, priority: number, prov: Provenance): void {
  const prev = p.mod_priorities?.[move] ?? null;
  const list = p.user_load_after?.[before] ?? [];
  const added = !list.includes(move);
  p.mod_priorities = { ...(p.mod_priorities ?? {}), [move]: priority };
  p.user_load_after = { ...(p.user_load_after ?? {}), [before]: added ? [...list, move] : list };
  if (migrate(p) !== null) return;
  push(p, make(`Load ${modName(move)} before ${modName(before)}`, { kind: "mod_order", move_mod_id: move, before_mod_id: before, priority }, prov, {
    restore_mod_order: { prev_priority: prev, added_load_after: added },
  }));
}

/** `fix_create` (§10): Disable, Order, Priority, Anchor, ApplyAs or SkipHunk. */
export function createFix(p: ModProfile, draft: FixDraft): string {
  ledgerOf(p);
  const prov = provenance(draft.source, {
    conflict_id: draft.conflict_id ?? null,
    conflict_kind: draft.conflict_id ? (liveConflicts(p).find((x) => x.id === draft.conflict_id)?.kind ?? null) : null,
    option_key: draft.option_key ?? null,
    approved: draft.approved ?? null,
  });
  const label = (draft.label ?? "").trim();
  const c = draft.change;
  let id: string | null;
  switch (c.kind) {
    case "disable":
      if (!c.patch_id.trim()) fail("invalid_input", "Pick a patch to turn off.");
      id = opDisable(p, c.patch_id.trim(), prov, label);
      break;
    case "order":
      if (!c.rule.patch.trim() || ((c.rule.before ?? []).length === 0 && (c.rule.after ?? []).length === 0)) {
        fail("invalid_input", "An order rule needs a patch and at least one patch to run before or after.");
      }
      id = opOrderSet(p, c.rule, prov, label);
      break;
    case "priority": {
      // A paused priority stays as it is (paused): undo of the new one must not turn it on.
      let previous: number | null = null;
      for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "priority" && f.change.patch.id === c.patch_id)) {
        remove(p, f.id);
        if (f.change.kind === "priority") previous = f.change.priority;
      }
      const undo: UndoRecord = previous === null ? "remove" : { restore_priority: previous };
      id = push(p, make(label || `Set the priority of ${patchName(c.patch_id)} to ${c.priority}`, { kind: "priority", patch: ref(c.patch_id), priority: c.priority }, prov, undo));
      break;
    }
    case "anchor": {
      if (!c.patch_id.trim() || c.anchors.length === 0) fail("invalid_input", "Pick a patch and a line to put its code next to.");
      // Like the service: rungs the engine would never use are refused.
      if (findPatch(c.patch_id)?.position === "at") {
        fail("invalid_input", `${patchName(c.patch_id)} replaces lines, so it cannot be moved this way. Make an adjusted copy instead.`);
      }
      if (c.anchors.some((r) => r.position === "at" || (r.edge && (r.pattern || !r.within)))) fail("invalid_input", "That place cannot be used for this patch.");
      // Replaces the earlier running Anchor fix on the patch; undo puts it back. A paused one
      // stays as it is (paused).
      let undo: UndoRecord = "remove";
      for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "anchor" && f.change.patch.id === c.patch_id)) {
        remove(p, f.id);
        if (f.change.kind === "anchor") {
          undo = { restore_anchor: { label: f.label, anchors: f.change.anchors, sites: f.change.sites ?? [], replace_author_anchors: f.change.replace_author_anchors ?? false } };
        }
      }
      const within = c.anchors[0]?.within;
      id = push(p, make(label || (within ? `Keep ${patchName(c.patch_id)} next to ${patchName(within)}` : `Move ${patchName(c.patch_id)} to another line`), {
        kind: "anchor", patch: ref(c.patch_id), anchors: structuredClone(c.anchors), sites: structuredClone(c.sites ?? []), replace_author_anchors: false,
      }, prov, undo));
      break;
    }
    case "apply_as": {
      if (!c.patch_id.trim()) fail("invalid_input", "Pick a patch.");
      // Like the service: an `at` patch has separate edits, and a before/after patch on several
      // lines uses them to find its lines around another patch's code (D48).
      const found = findPatch(c.patch_id);
      if (found && found.position !== "at" && !found.anchor.includes("\n")) {
        fail("invalid_input", `${patchName(c.patch_id)} does not replace lines, so it has no separate edits.`);
      }
      // Replaces the earlier running ApplyAs fix on the patch; undo puts it back.
      let undo: UndoRecord = "remove";
      for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "apply_as" && f.change.patch.id === c.patch_id)) {
        remove(p, f.id);
        if (f.change.kind === "apply_as") undo = { restore_apply_as: { label: f.label, mode: f.change.mode } };
      }
      const how = c.mode === "edits" ? "as separate edits" : "as one replacement";
      id = push(p, make(label || `Run ${patchName(c.patch_id)} ${how}`, { kind: "apply_as", patch: ref(c.patch_id), mode: c.mode }, prov, undo));
      break;
    }
    case "skip_hunk": {
      if (!c.patch_id.trim()) fail("invalid_input", "Pick a patch.");
      // Like the service: only an `at` patch has separate edits to skip.
      const found = findPatch(c.patch_id);
      if (found && found.position !== "at") fail("invalid_input", `${patchName(c.patch_id)} does not replace lines, so it has no separate edits.`);
      const keep = c.keep_patch_id?.trim() || null;
      // Replaces a running skip of the same edit, and the other patch's choice on the same
      // lines; undo puts every one back.
      // The edit's game line: the mock's patches change one line, their anchor.
      const vanilla = (found?.anchor ?? "").split("\n").map((l) => l.trim()).join("\n");
      const restores: UndoRecord[] = [];
      for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "skip_hunk")) {
        if (f.change.kind !== "skip_hunk") continue;
        const same = f.change.patch.id === c.patch_id && f.change.hunk.ordinal === c.ordinal;
        const other = keep === f.change.patch.id && f.change.hunk.keep === c.patch_id && sameLines(f.change.hunk.vanilla, vanilla);
        if (!same && !other) continue;
        remove(p, f.id);
        restores.push({ restore_hunk: { label: f.label, patch: f.change.patch, hunk: f.change.hunk } });
      }
      const undo = undoAll(restores);
      const hunk = { fp: { anchor: "", payload: "" }, vanilla, ordinal: c.ordinal, keep };
      const text = keep ? `Keep ${patchName(keep)}'s change to a line` : `Skip ${patchName(c.patch_id)}'s change to a line`;
      id = push(p, make(label || text, { kind: "skip_hunk", patch: ref(c.patch_id), hunk }, prov, undo));
      break;
    }
    case "stack_rule": {
      // Like the service: one other block, not the patch itself, at a line of the game's file.
      const below = c.below_patch_id?.trim() || null;
      const above = c.above_patch_id?.trim() || null;
      const other = below ?? above;
      if (!other || (below && above)) fail("invalid_input", "Pick one block to put it below or above.");
      if (!c.patch_id.trim() || other === c.patch_id) fail("invalid_input", "Pick two different blocks.");
      if (!c.site.context.trim()) fail("invalid_input", "Code can only be ordered at a line of the game's file.");
      const change: FixChange = {
        kind: "stack_rule", patch: ref(c.patch_id), site: c.site, side: c.side, below: below ? ref(below) : null, above: above ? ref(above) : null,
      };
      // Replaces the running rule for the same patch, line and side, and the other block's
      // rule there that names this patch; undo puts every one back.
      const restores: UndoRecord[] = [];
      const slot = slotOf(make("", change, prov));
      const otherSlot = slotOf(make("", { ...change, patch: ref(other!), below: null, above: null }, prov));
      const namesMe = (f: ProfileFix) => f.change.kind === "stack_rule" && [f.change.below?.id, f.change.above?.id].includes(c.patch_id);
      for (const f of (p.fixes ?? []).filter((f) => !f.paused && (slotOf(f) === slot || (slotOf(f) === otherSlot && namesMe(f))))) {
        remove(p, f.id);
        if (f.change.kind !== "stack_rule") continue;
        const patch = f.change.patch.id === other ? f.change.patch : null;
        restores.push({ restore_stack: { label: f.label, below: f.change.below, above: f.change.above, ...(patch ? { patch } : {}) } });
      }
      const undo = undoAll(restores);
      const where = below ? `below ${patchName(below)}'s` : `above ${patchName(other!)}'s`;
      id = push(p, make(label || `Put ${patchName(c.patch_id)}'s block ${where}`, change, prov, undo));
      break;
    }
    case "asset_choice": {
      // Like the service: one running choice per group or slot; undo puts the old one back.
      const key = c.key.trim();
      const winner = c.winner.trim();
      assets.checkChoice(p, key, winner);
      const excludes = assets.excludesOf(p, key, winner);
      let undo: UndoRecord = "remove";
      for (const f of (p.fixes ?? []).filter((f) => !f.paused && f.change.kind === "asset_choice" && f.change.key === key)) {
        remove(p, f.id);
        if (f.change.kind === "asset_choice") {
          undo = { restore_asset_choice: { label: f.label, winner: f.change.winner, excludes: f.change.excludes ?? {} } };
        }
      }
      id = push(p, make(label || assets.choiceLabel(key, winner), { kind: "asset_choice", key, winner, excludes }, prov, undo));
      break;
    }
    case "adjusted_copy_run_as": {
      // Like the service: the running copy's fix changes in place (D58).
      const glueId = c.glue_id.trim();
      const all = (p.fixes ?? []).filter((f) => f.change.kind === "adjusted_copy" && f.change.glue.id === glueId);
      const f = all.find((x) => !x.paused) ?? all[0] ?? fail("not_found", "That adjusted copy is already gone.");
      if (f.paused) fail("invalid_input", "The copy is paused. Resume it first.");
      if (f.change.kind !== "adjusted_copy") fail("internal", "That fix is not an adjusted copy.");
      const glue = f.change.glue;
      const mod = copyBaseMod(p, glue);
      if (c.run_as === "base" && !mod) {
        fail("invalid_input", "The patch this copy stands in for is not loaded, so the copy cannot run like its mod.");
      }
      // Absent means the older way, as old copies have it.
      const was = glue.run_as === "base";
      if (c.run_as === "base") glue.run_as = "base";
      else delete glue.run_as;
      if (c.run_as === "base" && !glue.base_mod_id && mod) glue.base_mod_id = mod;
      // Like the service: a switch is logged under an event kind older versions read.
      if (was !== (c.run_as === "base")) {
        const detail = c.run_as === "base" ? RUN_LIKE_EVENT : RUN_LAST_EVENT;
        f.provenance.history = [...(f.provenance.history ?? []), { at: now(), what: "rebound", detail }];
      }
      sync(p);
      id = f.id;
      break;
    }
  }
  if (id === null) fail("internal", "The profile's fixes are not imported yet.");
  log.fixes.push(`create:${c.kind}:${subjectOf((p.fixes ?? []).find((f) => f.id === id)!) ?? ""}`);
  return id;
}

/** Later fixes on the same patch, newest last; they go with an undo. */
export function laterOnSamePatch(p: ModProfile, id: string): string[] {
  const list = p.fixes ?? [];
  const at = list.findIndex((f) => f.id === id);
  const subject = at === -1 ? null : subjectOf(list[at]);
  if (subject === null) return [];
  return list.slice(at + 1).filter((f) => subjectOf(f) === subject).map((f) => f.id);
}

function modOrderIntact(p: ModProfile, f: ProfileFix): boolean {
  const c = f.change;
  if (c.kind !== "mod_order") return true;
  const pairThere = (p.user_load_after?.[c.before_mod_id] ?? []).includes(c.move_mod_id);
  const prioThere = c.priority == null || p.mod_priorities?.[c.move_mod_id] === c.priority;
  return pairThere && prioThere;
}

export function undoAsks(p: ModProfile, f: ProfileFix): string[] {
  if (f.change.kind === "mod_order" && !modOrderIntact(p, f)) {
    return [`The load order of ${modName(f.change.move_mod_id)} was changed in Load order since`];
  }
  return [];
}

/** Like the service: one record that puts back every replaced fix; the last carries the others. */
function undoAll(restores: UndoRecord[]): UndoRecord {
  const last = restores.pop();
  if (!last) return "remove";
  if (restores.length === 0) return last;
  if (typeof last === "object" && "restore_hunk" in last) return { restore_hunk: { ...last.restore_hunk, also: restores } };
  if (typeof last === "object" && "restore_stack" in last) return { restore_stack: { ...last.restore_stack, also: restores } };
  return last;
}

/** Puts back the stack rule or skipped edit `u` records, and those in its `also`, for `f`. */
function restoreReplaced(p: ModProfile, f: ProfileFix, u: UndoRecord): void {
  const c = f.change;
  if (typeof u === "object" && "restore_stack" in u && c.kind === "stack_rule") {
    const r = u.restore_stack;
    for (const a of r.also ?? []) restoreReplaced(p, f, a);
    push(p, make(r.label, { ...c, patch: r.patch ?? c.patch, below: r.below ?? null, above: r.above ?? null }, { ...f.provenance, history: [] }));
  } else if (typeof u === "object" && "restore_hunk" in u && c.kind === "skip_hunk") {
    const r = u.restore_hunk;
    for (const a of r.also ?? []) restoreReplaced(p, f, a);
    push(p, make(r.label, { kind: "skip_hunk", patch: r.patch, hunk: r.hunk }, { ...f.provenance, history: [] }));
  }
}

/** Undoes one fix (§7.5); the legacy fields follow. */
export function undoOne(p: ModProfile, id: string): void {
  const f = remove(p, id);
  if (!f) return;
  const u = f.undo ?? "remove";
  const c = f.change;
  if (typeof u === "object" && "restore_order" in u && u.restore_order) {
    push(p, make(orderLabel(u.restore_order), { kind: "order", rule: u.restore_order }, provenance("order_view")));
  } else if (typeof u === "object" && "restore_priority" in u && u.restore_priority != null && c.kind === "priority") {
    push(p, make(`Set the priority of ${patchName(c.patch.id)} to ${u.restore_priority}`, { kind: "priority", patch: c.patch, priority: u.restore_priority }, provenance("order_view")));
  } else if (typeof u === "object" && "restore_anchor" in u && c.kind === "anchor") {
    const r = u.restore_anchor;
    push(p, make(r.label, { kind: "anchor", patch: c.patch, anchors: r.anchors, sites: r.sites ?? [], replace_author_anchors: r.replace_author_anchors ?? false }, { ...f.provenance, history: [] }));
  } else if (typeof u === "object" && "restore_apply_as" in u && c.kind === "apply_as") {
    const r = u.restore_apply_as;
    push(p, make(r.label, { kind: "apply_as", patch: c.patch, mode: r.mode }, { ...f.provenance, history: [] }));
  } else if (typeof u === "object" && ("restore_stack" in u || "restore_hunk" in u)) {
    restoreReplaced(p, f, u);
  } else if (typeof u === "object" && "restore_asset_choice" in u && c.kind === "asset_choice") {
    const r = u.restore_asset_choice;
    push(p, make(r.label, { kind: "asset_choice", key: c.key, winner: r.winner, excludes: r.excludes ?? {} }, { ...f.provenance, history: [] }));
  } else if (typeof u === "object" && "restore_mod_order" in u && c.kind === "mod_order") {
    const r = u.restore_mod_order;
    if (c.priority != null && p.mod_priorities?.[c.move_mod_id] === c.priority) {
      const next = { ...(p.mod_priorities ?? {}) };
      if (r.prev_priority == null) delete next[c.move_mod_id];
      else next[c.move_mod_id] = r.prev_priority;
      p.mod_priorities = next;
    }
    if (r.added_load_after) {
      const list = (p.user_load_after?.[c.before_mod_id] ?? []).filter((m) => m !== c.move_mod_id);
      const next = { ...(p.user_load_after ?? {}) };
      if (list.length > 0) next[c.before_mod_id] = list;
      else delete next[c.before_mod_id];
      p.user_load_after = next;
    }
  }
  sync(p);
}

export function find(p: ModProfile, id: string): ProfileFix {
  return (p.fixes ?? []).find((f) => f.id === id) ?? fail("not_found", "That fix is already gone.");
}

export function undoFix(p: ModProfile, id: string, confirmed: boolean): void {
  ledgerOf(p);
  const f = find(p, id);
  const also = laterOnSamePatch(p, id);
  const asks = undoAsks(p, f);
  if (!confirmed && (also.length > 0 || asks.length > 0)) {
    const why = [...(also.length > 0 ? [`${also.length} later fix(es) on the same patch would be undone too`] : []), ...asks];
    fail("conflict", `Confirm first: ${why.join("; ")}.`);
  }
  for (const x of [...also].reverse()) undoOne(p, x);
  undoOne(p, id);
  log.fixes.push(`undo:${id}`);
}

/** The profile with `changes`' fixes added and left out, as a preview sees it (nothing is saved). */
export function withFixChanges(p: ModProfile, changes: PreviewChanges): ModProfile {
  const add = changes.add_fixes ?? [];
  const drop = changes.remove_fixes ?? [];
  if (add.length === 0 && drop.length === 0) return p;
  const q = structuredClone(p);
  const why = migrate(q);
  if (why !== null) fail("invalid_input", `${IMPORT_FAILED}: ${why}`);
  for (const id of drop) undoOne(q, id);
  const logged = log.fixes.length;
  const orders = log.orderRules.length;
  for (const d of add) createFix(q, d);
  // A preview writes nothing, the log included.
  log.fixes.length = logged;
  log.orderRules.length = orders;
  return q;
}

export function undoPreview(p: ModProfile, id: string): UndoPreview {
  const q = structuredClone(p);
  if (migrate(q) !== null) fail("invalid_input", `${IMPORT_FAILED}: ${migrate(q)}`);
  const f = find(q, id);
  const also = laterOnSamePatch(q, id);
  const asks = undoAsks(q, f);
  for (const x of [...also].reverse()) undoOne(q, x);
  undoOne(q, id);
  return { impact: previewFor(p, {}, q).impact, also_undoes: also, asks };
}

// Views

function label(id: string, modHint = ""): PatchLabel {
  const m = findPatch(id);
  const mod = m?.mod ?? modHint;
  return { patch_id: id, mod_id: mod, name: m?.name ?? null, mod_name: mod ? modName(mod) : null };
}

function sameText(a: PatchSnapshot, b: PatchSnapshot): boolean {
  return a.kind === b.kind && a.pattern === b.pattern && a.payload === b.payload && a.position === b.position && a.times === b.times;
}

function drift(f: ProfileFix): FixView["drift"] {
  const out: NonNullable<FixView["drift"]> = [];
  for (const [id, before] of Object.entries(f.snapshots ?? {})) {
    const after = before ? snapshotOf(id, now()) : null;
    if (!before || !after || sameText(before, after)) continue;
    out.push({ patch_id: id, role: subjectOf(f) === id ? "fix" : "anchor_target", before, after });
  }
  return out.length > 0 ? out : null;
}

/** Names a mod update split into several patches (`MockInit.ambiguousFix`): the patches each could mean. */
const AMBIGUOUS: Record<string, string[]> = {};

/** Like the service's `status`: Needs review when the game changed since the fix was stamped (D74). */
function statusOf(p: ModProfile, f: ProfileFix): FixStatus {
  const s = baseStatusOf(p, f);
  if (s.state === "paused" || s.state === "waiting" || s.state === "suspended" || gameDrift(f).length === 0) return s;
  const reasons = s.state === "needs_review" ? s.reasons : s.state === "fell_back" ? [s.reason] : [];
  return { state: "needs_review", reasons: [...reasons, GAME_CHANGED] };
}

function baseStatusOf(p: ModProfile, f: ProfileFix): FixStatus {
  if (f.paused) return { state: "paused" };
  if (f.change.kind === "mod_order") {
    return modOrderIntact(p, f) ? { state: "active" } : { state: "needs_review", reasons: ["Changed in Load order"] };
  }
  const subject = subjectOf(f);
  const could = subject ? AMBIGUOUS[subject] : undefined;
  if (subject && could) {
    return { state: "needs_review", reasons: [`${could.length} patches could be ${subject}; nothing is applied until you pick one`] };
  }
  const patch = subject ? findPatch(subject) : undefined;
  if (subject && !patch && f.change.kind !== "adjusted_copy") {
    return { state: "needs_review", reasons: [`${subject} is no longer in its mod`] };
  }
  if (patch && !p.enabled_mods.includes(patch.mod)) return { state: "waiting", mods: [patch.mod] };
  const d = drift(f);
  if (d) {
    const changed = [...new Set(d.map((x) => modName(findPatch(x.patch_id)?.mod ?? "")))];
    return { state: "needs_review", reasons: [`${changed.join(", ")} changed this patch since you made this fix`] };
  }
  if (f.change.kind === "anchor" && subject) {
    const why = anchorFallback(p, subject);
    if (why) return { state: "fell_back", sites: 1, reason: `Uses its own line: ${why}` };
  }
  // Like the service's `stack_status`: a rule whose blocks never meet at its line waits (D55);
  // one a stronger rule beat is suspended (D56).
  const c = f.change;
  if (c.kind === "stack_rule") {
    const s = stackRuleState(p, f.id);
    if (s.state === "overridden") return { state: "suspended", reason: s.reason };
    if (s.state === "waiting") {
      const other = findPatch(c.below?.id ?? c.above?.id ?? "");
      const name = other ? `${modName(other.mod)} › ${other.name}` : (c.below?.id ?? c.above?.id ?? "");
      return { state: "waiting", mods: [`${name} at this line`] };
    }
  }
  return { state: "active" };
}

function statusText(s: FixStatus): string {
  switch (s.state) {
    case "active": return "Working";
    case "rebound": return `Follows ${s.to} now`;
    case "followed": return "Follows the changed code";
    case "fell_back": return s.reason;
    case "waiting": return `Waiting for ${s.mods.map(modName).join(", ")}`;
    case "needs_review": return s.reasons.join("; ");
    case "suspended": return s.reason;
    case "paused": return "Paused";
  }
}

function viewOf(p: ModProfile, f: ProfileFix, legacy: boolean): FixView {
  const status = legacy ? ({ state: "active" } as FixStatus) : statusOf(p, f);
  const subject = subjectOf(f);
  const c = f.change;
  const related =
    c.kind === "order" ? [...(c.rule.before ?? []), ...(c.rule.after ?? [])].map((id) => label(id))
    : c.kind === "stack_rule" ? [c.below, c.above].flatMap((r) => (r ? [label(r.id, r.mod_id)] : []))
    : [];
  const file = subject ? (findPatch(subject)?.file ?? null) : null;
  const copyFile = c.kind === "adjusted_copy" && "pattern" in c.glue.patch ? [c.glue.patch.pattern.target].flat()[0] : null;
  const blocked =
    legacy ? "Import your earlier fixes first."
    : c.kind === "adjusted_copy" ? "An adjusted copy cannot be saved as a compatibility patch."
    : c.kind === "mod_order" ? "A load order change cannot be saved as a compatibility patch."
    : status.state === "waiting" ? "Turn on the mods this fix changes first."
    : f.paused ? "It is paused. Resume it first."
    : status.state === "suspended" || status.state === "needs_review" ? "Review this fix first."
    : null;
  const runLike = runLikeMod(p, f, legacy);
  return {
    id: f.id, label: f.label, kind: c.kind, status, status_text: statusText(status), target_file: file ?? copyFile,
    patch: subject ? label(subject, c.kind === "adjusted_copy" ? (c.glue.base_mod_id ?? "") : "") : null,
    related, source: f.provenance.source, provenance: f.provenance, paused: f.paused ?? false, fallback_text: null,
    drift: legacy ? null : drift(f), can_export: blocked === null, export_blocked_reason: blocked, legacy,
    ...(subject && AMBIGUOUS[subject] && !legacy ? { candidates: AMBIGUOUS[subject].map((id) => label(id)) } : {}),
    ...(c.kind === "adjusted_copy" ? { glue_id: c.glue.id } : {}),
    ...(runLike ? { run_like_mod: runLike } : {}),
    ...(canRunLast(p, f, legacy) ? { can_run_last: true } : {}),
    ...(!legacy && gameDrift(f).length > 0 ? { game_drift: gameDrift(f) } : {}),
  };
}

/** The service's history details of a copy's run-as switch (D58). */
const RUN_LIKE_EVENT = "Runs like its mod";
const RUN_LAST_EVENT = "Runs after every mod's patches";

/** A running copy "Run it like <mod>" switched to its mod, as the service's `can_run_last` says it. */
function canRunLast(p: ModProfile, f: ProfileFix, legacy: boolean): boolean {
  const c = f.change;
  if (legacy || f.paused || c.kind !== "adjusted_copy" || c.glue.run_as !== "base") return false;
  if (copySuperseded(p, c.glue.id)) return false;
  const last = (f.provenance.history ?? [])
    .filter((e) => e.what === "rebound" && (e.detail === RUN_LIKE_EVENT || e.detail === RUN_LAST_EVENT))
    .at(-1);
  return last?.detail === RUN_LIKE_EVENT;
}

/** The mod a running older copy can run as instead (D58), as the service's `run_like_mod` says it. */
function runLikeMod(p: ModProfile, f: ProfileFix, legacy: boolean): string | null {
  const c = f.change;
  if (legacy || f.paused || c.kind !== "adjusted_copy" || c.glue.run_as === "base") return null;
  if (copySuperseded(p, c.glue.id)) return null;
  return copyBaseMod(p, c.glue);
}

/** Read-only rows of an unmigrated profile, from its legacy fields (D38). */
function legacyRows(p: ModProfile): ProfileFix[] {
  const prov = provenance("migrated", { created_at: p.updated_at });
  const rows: ProfileFix[] = [];
  const row = (field: string, i: number, lbl: string, change: FixChange) => rows.push({ id: `legacy:${field}:${i}`, label: lbl, change, provenance: prov });
  (p.disabled_patches ?? []).forEach((id, i) => row("disabled_patches", i, `Turn off ${patchName(id)}`, { kind: "disable", patch: ref(id) }));
  (p.patch_order ?? []).forEach((r, i) => row("patch_order", i, orderLabel(r), { kind: "order", rule: r }));
  Object.entries(p.patch_priorities ?? {}).forEach(([id, n], i) =>
    row("patch_priorities", i, `Set the priority of ${patchName(id)} to ${n}`, { kind: "priority", patch: ref(id), priority: n ?? 0 }),
  );
  (p.glue_patches ?? []).forEach((g, i) => row("glue_patches", i, g.name, { kind: "adjusted_copy", glue: g }));
  return rows;
}

export function fixesView(p: ModProfile): FixesView {
  const migrated = isMigrated(p);
  const rows = migrated ? (p.fixes ?? []) : legacyRows(p);
  return {
    profile_id: p.id,
    fixes: rows.map((f) => viewOf(p, f, !migrated)),
    suggestions: [],
    suggestions_pending: false,
    suggestions_note: null,
    stacking: p.stacking ?? "legacy",
    stacking_acknowledged: (p.notices_seen ?? []).includes("stacking_v1"),
    plan_error: state.init.planError ?? null,
    load_errors: state.init.loadErrors ?? [],
    plan_note: null,
    unreadable_fixes: 0,
    journal: (p.fix_journal ?? []).map((e, index) => ({ index, at: e.at, op: e.op, label: e.label, removed: e.removed?.length ?? 0, added: e.added?.length ?? 0 })),
    migrated,
    migration_error: p.migration_error ?? null,
  };
}

/** The suggestions are ready (after `suggestionsDelayMs`), or a timer will make them so. */
let suggestionsReady = false;
let suggestionsTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * The suggestions part of `fixes_list` (§9.1): with `__qmMockInit.suggestions`, pending on the
 * first read; after the delay a `fixes` event follows and later reads list them, like the
 * backend's `FixSuggestions` job.
 */
export function suggestionsView(): Pick<FixesView, "suggestions" | "suggestions_pending" | "suggestions_note"> {
  const list = state.init.suggestions;
  if (!list || suggestionsReady) return { suggestions: list ?? [], suggestions_pending: false, suggestions_note: null };
  if (!state.init.suggestionsHeld) {
    suggestionsTimer ??= setTimeout(releaseSuggestions, state.init.suggestionsDelayMs ?? 0);
  }
  return { suggestions: [], suggestions_pending: true, suggestions_note: null };
}

/** The suggestions are made: later reads list them, after a `fixes` event. */
export function releaseSuggestions(): void {
  if (suggestionsTimer) clearTimeout(suggestionsTimer);
  suggestionsTimer = null;
  if (suggestionsReady) return;
  suggestionsReady = true;
  emitMock({ type: "invalidated", scope: "fixes" });
}

export function fixView(p: ModProfile, id: string): FixView {
  return fixesView(p).fixes.find((f) => f.id === id) ?? fail("not_found", "That fix is already gone.");
}

export function setPaused(p: ModProfile, id: string, paused: boolean): void {
  ledgerOf(p);
  const f = find(p, id);
  if (paused && f.change.kind === "mod_order") fail("invalid_input", "A load order change cannot be paused. Undo it, or change it in Load order.");
  if ((f.paused ?? false) === paused) return;
  // One running priority, order rule or anchor per patch, one copy per id.
  if (!paused) {
    const labels = runningClashes(p, f).map((id) => `'${find(p, id).label}'`);
    if (labels.length > 0) fail("conflict", `${labels.join(", ")} is on the same patch now. Undo or pause it first.`);
  }
  f.paused = paused;
  f.provenance.history = [...(f.provenance.history ?? []), { at: now(), what: paused ? "paused" : "resumed", detail: "" }];
  sync(p);
  log.fixes.push(`${paused ? "pause" : "resume"}:${id}`);
}

/** Pins the fix to the patches as they are now. */
export function accept(p: ModProfile, id: string): void {
  ledgerOf(p);
  const f = find(p, id);
  const at = now();
  f.snapshots = snapshots(f.change, at);
  stamp(f);
  f.provenance.history = [...(f.provenance.history ?? []), { at, what: "accepted", detail: "" }];
  log.fixes.push(`accept:${id}`);
}

export function rebind(p: ModProfile, id: string, patchId: string): void {
  ledgerOf(p);
  const f = find(p, id);
  if (!findPatch(patchId)) fail("invalid_input", `No loaded patch is called '${patchId}'.`);
  const c = f.change;
  if (c.kind === "disable" || c.kind === "priority" || c.kind === "anchor") c.patch = ref(patchId);
  else if (c.kind === "order") c.rule = { ...c.rule, patch: patchId };
  else fail("invalid_input", "This fix cannot be pointed at another patch.");
  f.snapshots = snapshots(f.change, now());
  stamp(f);
  f.provenance.history = [...(f.provenance.history ?? []), { at: now(), what: "rebound", detail: patchId }];
  sync(p);
  log.fixes.push(`rebind:${id}:${patchId}`);
}

/** The running fixes each fix of `e` would clash with once back (`journal_clashes`); `p` no longer holds the fixes `e` added. */
function journalClashes(p: ModProfile, e: JournalEntry): [ProfileFix, string][] {
  return (e.removed ?? []).filter((f) => !f.paused).flatMap((f) => runningClashes(p, f).map((id): [ProfileFix, string] => [f, id]));
}

function journalEntry(p: ModProfile, index: number): JournalEntry {
  return p.fix_journal?.[index] ?? fail("not_found", "That step is already undone.");
}

/**
 * Reverts a batch operation. A migration goes back to the legacy fields and stays unimported until
 * "Retry import"; refused while fixes made or changed since would be lost. An export brings its
 * fixes back and turns its mod off; when one would clash with a newer running fix, `choice` says
 * how to go on (D71), and without one it is refused.
 */
export function journalUndo(p: ModProfile, index: number, choice: JournalUndoChoice | null = null): void {
  ledgerOf(p);
  const e = journalEntry(p, index);
  if (e.op === "migrated") {
    const added = e.added ?? [];
    const later = (p.fixes ?? []).filter((f) => !added.includes(f.id)).length;
    if (later > 0) fail("conflict", `${later} fix(es) were made after the import. Undo them first; undoing the import would lose them.`);
    if ((p.fixes ?? []).some((f) => f.paused)) {
      fail("conflict", "Imported fixes were changed after the import. Undo or resume them first; undoing the import would lose the changes.");
    }
    // The legacy fields are the compiled imported fixes: they stay as they are.
    p.fixes = [];
    p.fix_journal = [];
    p.fixes_version = 0;
    p.migration_error = IMPORT_UNDONE;
    return;
  }
  // Checked before anything changes: a refusal leaves the profile as it was.
  const kept = (p.fixes ?? []).filter((f) => !(e.added ?? []).includes(f.id));
  const clashes = journalClashes({ ...p, fixes: kept }, e);
  const newer = [...new Set(clashes.map(([, id]) => id))];
  if (clashes.length > 0 && choice === null) {
    const labels = newer.map((id) => `'${find(p, id).label}'`);
    fail("conflict", `${newer.length} fix(es) made since change the same patches: ${labels.join(", ")}. Undo or pause them first; undoing the export would override them.`);
  }
  const at = now();
  const restored = structuredClone(e.removed ?? []);
  if (clashes.length > 0 && choice === "pause_newer") {
    for (const f of kept.filter((x) => newer.includes(x.id))) {
      f.paused = true;
      f.provenance.history = [...(f.provenance.history ?? []), { at, what: "paused", detail: `to bring back fixes of ${e.label}` }];
    }
  } else if (clashes.length > 0) {
    for (const f of restored) {
      const by = clashes.find(([old]) => old.id === f.id)?.[1];
      if (by === undefined) continue;
      f.paused = true;
      f.provenance.history = [...(f.provenance.history ?? []), { at, what: "paused", detail: `brought back paused: '${find(p, by).label}' is on the same patch` }];
    }
  }
  p.fixes = [...kept, ...restored];
  p.enabled_mods = p.enabled_mods.filter((m) => !(e.mods_enabled ?? []).includes(m));
  p.fix_journal = (p.fix_journal ?? []).filter((_, i) => i !== index);
  sync(p);
}

/** What undoing step `index` does (`fix_journal_undo_preview`): the lists, the clashes and each choice's impact. */
export function journalUndoPreview(p: ModProfile, index: number): JournalUndoPreview {
  const base = structuredClone(p);
  if (migrate(base) !== null) fail("invalid_input", `${IMPORT_FAILED}: ${migrate(base)}`);
  const e = journalEntry(base, index);
  const kept = (base.fixes ?? []).filter((f) => !(e.added ?? []).includes(f.id));
  const clashes: JournalClash[] =
    e.op === "migrated"
      ? []
      : journalClashes({ ...base, fixes: kept }, e).map(([old, id]) => ({
          restored_fix_id: old.id, restored_label: old.label, newer_fix_id: id, newer_label: find(base, id).label,
          slot: old.change.kind, patch_id: subjectOf(old) ?? "",
        }));
  const impact = (choice: JournalUndoChoice | null) => {
    const q = structuredClone(base);
    journalUndo(q, index, choice);
    return previewFor(p, {}, q).impact;
  };
  return {
    op: e.op, label: e.label,
    restores: (e.removed ?? []).map((f) => f.label),
    turns_off_mods: (e.mods_enabled ?? []).map(modName),
    clashes,
    impact_keep: impact(clashes.length > 0 ? "restore_paused" : null),
    impact_pause: clashes.length > 0 ? impact("pause_newer") : null,
  };
}

/** The drifted Disable of `MockInit.driftedFix`: GMenu changed Hotkeys after the user turned it off,
 *  plus a working order fix from a conflict. */
export function seedDriftedFixes(p: ModProfile): void {
  const made = "2026-10-20T09:00:00Z";
  const off: ProfileFix = {
    id: "fx_5e1d0c0ffee1", label: "Turn off Hotkeys",
    change: { kind: "disable", patch: { id: "menu-hotkeys", mod_id: "gmenu", fp: null } },
    provenance: provenance("patch_toggle", { created_at: made, history: [{ at: made, what: "created", detail: "" }] }),
    paused: false, undo: "remove",
    snapshots: {
      "menu-hotkeys": {
        kind: "pattern", pattern: 'keys.menu = "m"', payload: 'keys.mods = "m"', position: "after", times: 1, mod_version: "1.2.0", taken_at: made,
      },
    },
  };
  const rule: OrderRule = { patch: "ai-targeting", before: ["core-targets"], after: [] };
  const order: ProfileFix = {
    id: "fx_0a11c0ffee02", label: "Run Targeting before Target picker",
    change: { kind: "order", rule },
    provenance: provenance("resolver", {
      created_at: "2026-11-07T10:30:00Z", conflict_id: "broken-anchor:scripts/ai.lua:ai-targeting", conflict_kind: "broken_anchor_replaced",
      history: [{ at: "2026-11-07T10:30:00Z", what: "created", detail: "" }],
    }),
    paused: false, undo: { restore_order: null }, snapshots: snapshots({ kind: "order", rule }, "2026-11-07T10:30:00Z"),
  };
  p.fixes = [off, order];
  p.fixes_version = 1;
  p.fix_journal = [];
  p.notices_seen = [...new Set([...(p.notices_seen ?? []), "fixes_import_v1"])];
  sync(p);
}

/** The copy of `MockInit.legacyCopy`: an adjusted copy of Vehicle init made before copies ran as their base mod, so it
 *  runs as QuarterMaster's own after every mod's patches (D58). */
export function seedLegacyCopy(p: ModProfile): void {
  const made = "2026-10-12T09:00:00Z";
  const glue: GluePatchEntry = {
    id: "glue:vehicle-init", name: "Adjusted copy of Vehicle init", replaces_patch_id: "vehicle-init", priority: 50,
    patch: {
      pattern: {
        pattern: "  initGame()", position: "after", target: "main.lua", payload: "  vehicles.load()", times: 1,
        id: null, name: null, requires: [], priority: null, before: [], after: [], on_failure: "warn",
      },
    },
  };
  const copy: ProfileFix = {
    id: "fx_c0b1c0ffee04", label: glue.name,
    change: { kind: "adjusted_copy", glue, base: ref("vehicle-init"), disables_base: true },
    provenance: provenance("resolver", { created_at: made, history: [{ at: made, what: "created", detail: "" }] }),
    paused: false, undo: "remove", snapshots: snapshots({ kind: "adjusted_copy", glue }, made),
  };
  p.fixes = [...(p.fixes ?? []), copy];
  p.fixes_version = 1;
  p.fix_journal = p.fix_journal ?? [];
  p.notices_seen = [...new Set([...(p.notices_seen ?? []), "fixes_import_v1"])];
  sync(p);
}

/** The fix of `MockInit.ambiguousFix`: a GMenu update split "Menu keys" into its menu entry and its
 *  hotkeys, so the Disable could mean either (Ambiguous, §7.2). */
/**
 * `MockInit.gameUpdated`: a Disable of GMenu's Hotkeys made before a game update that changed its
 * file, so the fix needs review (D74).
 */
export function seedGameUpdate(p: ModProfile): void {
  const made = "2026-10-20T09:00:00Z";
  const file = findPatch("menu-hotkeys")?.file ?? "scripts/ui/keys.lua";
  const off: ProfileFix = {
    id: "fx_9a3e0c0ffee4", label: "Turn off Hotkeys",
    change: { kind: "disable", patch: { id: "menu-hotkeys", mod_id: "gmenu", fp: null } },
    provenance: provenance("patch_toggle", { created_at: made, history: [{ at: made, what: "created", detail: "" }] }),
    paused: false, undo: "remove", snapshots: snapshots({ kind: "disable", patch: ref("menu-hotkeys") }, made),
    vanilla_hashes: { [file]: gameHash(file) },
  };
  GAME_UPDATED.add(file);
  p.fixes = [...(p.fixes ?? []), off];
  p.fixes_version = 1;
  p.fix_journal = p.fix_journal ?? [];
  p.notices_seen = [...new Set([...(p.notices_seen ?? []), "fixes_import_v1"])];
  sync(p);
}

/** `MockInit.factionOrder`: a load order fix that gave Vehicle Pack (Red Army) the higher priority. */
export function seedFactionOrder(p: ModProfile): void {
  migrate(p);
  opModOrder(p, "vehicle-pack", "ai-tweaks", 70, provenance("resolver"));
}

export function seedAmbiguousFix(p: ModProfile): void {
  const made = "2026-10-20T09:00:00Z";
  AMBIGUOUS["gmenu:menu-keys"] = ["menu-entry", "menu-hotkeys"];
  const off: ProfileFix = {
    id: "fx_a3b1c0ffee03", label: "Turn off Menu keys",
    change: { kind: "disable", patch: { id: "gmenu:menu-keys", mod_id: "gmenu", fp: null } },
    provenance: provenance("patch_toggle", { created_at: made, history: [{ at: made, what: "created", detail: "" }] }),
    paused: false, undo: "remove", snapshots: {},
  };
  p.fixes = [...(p.fixes ?? []), off];
  p.fixes_version = 1;
  p.fix_journal = p.fix_journal ?? [];
  p.notices_seen = [...new Set([...(p.notices_seen ?? []), "fixes_import_v1"])];
  sync(p);
}
