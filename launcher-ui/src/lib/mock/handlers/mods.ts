import * as assets from "../assets";
import type { CompatExportRequest, ConfigValue, ConflictResolutionSuggestion, FixChangeDraft, FixDraft, JournalUndoChoice, LoadOrderUpdate } from "../../bindings";
import { compatChanges, exportFixes, exportPreview } from "../compat";
import { detailFor, resetConfig, savedConfig } from "../detail";
import { loadOrderFor, mods, patchesFor, ready, simulationFor } from "../fixtures";
import * as ledger from "../ledger";
import { anchorCandidatesFor, fileDiffFor, glueFor, MOD_RELATIONS, patchDetailFor, previewFor, regionOptionsFor, regionVerdict, stacksFor } from "../patchworld";
import type { InsertPosition, ModProfile, OrderRule, PreviewChanges, RegionOptions, RegionRef, SimulationOverview, StackMode } from "../../bindings";
import { activeProfile, ensureNotRunning, fail, log, profileChanged, state, type Handlers } from "../state";

function setLoadOrder(update: LoadOrderUpdate) {
  ensureNotRunning();
  log.lastLoadOrderSet = structuredClone(update);
  const p = activeProfile();
  p.mod_priorities = { ...update.mod_priorities };
  p.user_load_after = { ...update.user_load_after };
  p.locked_mods = [...update.locked_mods];
  p.separators = update.separators.map((sep) => ({ ...sep }));
  profileChanged(p);
  return loadOrderFor(p);
}

function setEnabled(id: string, enabled: boolean) {
  ensureNotRunning();
  const m = mods.find((x) => x.id === id) ?? fail("not_found", `No mod ${id}.`);
  const p = activeProfile();
  p.enabled_mods = enabled ? [...new Set([...p.enabled_mods, id])] : p.enabled_mods.filter((x) => x !== id);
  profileChanged(p);
  return { ...m, enabled };
}

/** Legacy patch toggles: a Disable fix on a migrated profile, the legacy field otherwise. */
function setPatchesEnabled(ids: string[], enabled: boolean): null {
  ensureNotRunning();
  const p = activeProfile();
  for (const id of ids) {
    if (enabled) ledger.opEnable(p, id);
    else ledger.opDisable(p, id, ledger.provenance("patch_toggle"));
  }
  profileChanged(p);
  return null;
}

/** `MockInit.orderReports`: one of each order report the simulation gives (G15). */
const ORDER_REPORTS: Pick<SimulationOverview, "overridden_rules" | "unknown_ids" | "suspended_rungs" | "invalid_rungs"> = {
  overridden_rules: ["profile rule Hotkeys before Menu entry ignored: Hotkeys requires Menu entry"],
  unknown_ids: ["gmenu:old-entry"],
  suspended_rungs: [{
    patch_id: "Minimap", rung: 0, source: "author",
    reason: "Minimap and Compass would each have to run first, so Minimap uses its own line",
  }],
  invalid_rungs: [{
    patch_id: "ai-overlay", rung: 0, reason: "not_required", source: "author",
    detail: "AI Tweaks places code inside Tactical HUD's change but does not require Tactical HUD; QuarterMaster uses its own line",
  }],
};

/** Every conflict-driven write records its conflict, so "Resolved by" finds it (D36). */
function resolveConflict(s: ConflictResolutionSuggestion, winner: number | null): null {
  ensureNotRunning();
  const p = activeProfile();
  const prov = ledger.provenance("resolver", ledger.conflictOf(p, s));
  if (s.tier === "select_winner") {
    if (winner !== 0 && winner !== 1) fail("invalid_input", "Choose which patch wins.");
    ledger.opDisable(p, winner === 0 ? s.patch_b_id : s.patch_a_id, prov);
  } else if (s.tier === "swap_mod_order") {
    const needs = MOD_RELATIONS[s.move_mod_id];
    if (needs && [...needs.requires, ...needs.load_after].includes(s.before_mod_id)) {
      const name = (id: string) => mods.find((m) => m.id === id)?.name ?? id;
      fail("invalid_input", `${name(s.move_mod_id)} needs ${name(s.before_mod_id)} to load first, so it cannot load before it.`);
    }
    ledger.opModOrder(p, s.move_mod_id, s.before_mod_id, s.suggested_priority, prov);
  } else if (s.tier === "generate_glue_patch") {
    ledger.opGlueAdd(p, s.glue_patch, false, prov);
  } else {
    // As the service: the same drafts the resolver previews (D46, D49).
    const change: FixChangeDraft = s.tier === "apply_as"
      ? { kind: "apply_as", patch_id: s.patch_id, mode: "edits" }
      : { kind: "skip_hunk", patch_id: s.patch_id, ordinal: s.ordinal, keep_patch_id: s.keep_patch_id };
    ledger.createFix(p, { change, label: "", source: "resolver", conflict_id: prov.conflict_id ?? null, option_key: null, approved: null });
  }
  profileChanged(p);
  return null;
}

/** Region options with each usable one previewed on its file: one that would not place the code is greyed with why. */
function checkedRegionOptions(file: string, patchId: string, conflictId: string | null): RegionOptions {
  const p = activeProfile();
  const r = regionOptionsFor(file, patchId, p, conflictId) ?? fail("not_found", `No loaded patch is called '${patchId}'.`);
  for (const o of r.options) {
    if (!o.draft || !o.placement_ok) continue;
    let why: string | null;
    try {
      why = regionVerdict(file, o.draft.change.kind === "anchor" ? o.draft.change.patch_id : patchId, ledger.withFixChanges(p, { add_fixes: [o.draft] }));
    } catch (e) {
      why = (e as { message?: string }).message ?? "The preview is not ready yet.";
    }
    o.placement_ok = why === null;
    o.placement_reason = why;
  }
  return r;
}

/** A fix command on the active profile: refused while the game runs, saved and announced after. */
function fixWrite<T>(write: (p: ModProfile) => T): T {
  ensureNotRunning();
  const p = activeProfile();
  if (p.id === "vanilla") fail("invalid_input", "The Vanilla profile cannot be changed.");
  const out = write(p);
  profileChanged(p);
  return out;
}

export const handlers: Handlers = {
  mods_list: () => {
    const enabled = new Set(activeProfile().enabled_mods);
    return ready(mods.map((m) => ({ ...m, enabled: enabled.has(m.id) })));
  },
  mod_detail: (args) => {
    const id = String(args.id);
    return detailFor(id, activeProfile().enabled_mods.includes(id)) ?? fail("not_found", `No mod ${id}.`);
  },
  mod_config_set: (args) => {
    ensureNotRunning();
    const id = String(args.id);
    savedConfig[id] = { ...(savedConfig[id] ?? {}), ...(args.values as Record<string, ConfigValue>) };
    return savedConfig[id];
  },
  mod_config_reset: (args) => {
    ensureNotRunning();
    return resetConfig(String(args.id));
  },
  patch_set_enabled: (args) => setPatchesEnabled([String(args.patchId)], Boolean(args.enabled)),
  conflict_resolve: (args) =>
    resolveConflict(args.suggestion as ConflictResolutionSuggestion, (args.winnerIndex as number | null | undefined) ?? null),
  mod_set_enabled: (args) => setEnabled(String(args.id), Boolean(args.enabled)),
  load_order_set: (args) => setLoadOrder(args.update as LoadOrderUpdate),
  load_order_get: () => ready(loadOrderFor(activeProfile())),
  // Loading while the simulation a profile write started runs, as the service answers.
  simulation_get: () =>
    state.simulating
      ? { status: "loading" }
      : ready({
          ...simulationFor(activeProfile()),
          plan_error: state.init.planError ?? null,
          load_errors: state.init.loadErrors ?? [],
          ...(state.init.orderReports ? ORDER_REPORTS : {}),
        }),
  mod_patches_get: (args) => ready(patchesFor(String(args.modId), activeProfile())),
  compat_changes_get: (args) => {
    const id = String(args.modId);
    if (!mods.some((m) => m.id === id)) fail("not_found", `Mod '${id}' is not installed.`);
    return ready(compatChanges(id, activeProfile()));
  },
  compat_export_preview: (args) => exportPreview(activeProfile(), args.req as CompatExportRequest),
  compat_export: (args) => fixWrite((p) => exportFixes(p, args.req as CompatExportRequest)),
  patch_set_enabled_many: (args) => setPatchesEnabled((args.patchIds as string[]) ?? [], Boolean(args.enabled)),
  file_diff_get: (args) => {
    const file = String(args.targetFile);
    return ready(fileDiffFor(file, activeProfile()) ?? fail("not_found", `No patches touch '${file}'.`));
  },
  simulation_preview: (args) => {
    const p = activeProfile();
    const changes = args.changes as PreviewChanges;
    return previewFor(p, changes, ledger.withFixChanges(p, changes));
  },
  anchor_candidates: (args) =>
    anchorCandidatesFor(String(args.targetFile), Number(args.line), Number(args.radius), activeProfile(), (args.region as RegionRef | null | undefined) ?? null) ??
    fail("not_found", `No patches touch '${String(args.targetFile)}'.`),
  region_options: (args) =>
    checkedRegionOptions(String(args.targetFile), String(args.patchId), args.conflictId == null ? null : String(args.conflictId)),
  glue_patch_create: (args) => {
    ensureNotRunning();
    const p = activeProfile();
    const glue =
      glueFor(String(args.basePatchId), String(args.pattern), args.position as InsertPosition, p) ??
      fail("not_found", `No loaded patch is called '${String(args.basePatchId)}'.`);
    const conflictId = args.conflictId == null ? "" : String(args.conflictId).trim();
    const prov = ledger.provenance("resolver", conflictId ? { conflict_id: conflictId, conflict_kind: ledger.conflictKind(p, conflictId) } : {});
    ledger.opGlueAdd(p, glue, true, prov);
    log.glueCreates.push(glue.id);
    profileChanged(p);
    return glue;
  },
  glue_patch_remove: (args) => {
    ensureNotRunning();
    const p = activeProfile();
    if (!ledger.opGlueRemove(p, String(args.glueId))) fail("not_found", "That adjusted patch is already gone.");
    profileChanged(p);
    return null;
  },
  patch_order_set: (args) => {
    ensureNotRunning();
    const rule = JSON.parse(JSON.stringify(args.rule)) as OrderRule;
    if (!rule.patch.trim() || ((rule.before ?? []).length === 0 && (rule.after ?? []).length === 0)) {
      fail("invalid_input", "An order rule needs a patch and at least one patch to run before or after.");
    }
    const p = activeProfile();
    ledger.opOrderSet(p, rule, ledger.provenance("order_view"));
    profileChanged(p);
    return null;
  },
  patch_order_reset: (args) => {
    ensureNotRunning();
    const p = activeProfile();
    ledger.opOrderReset(p, String(args.patchId));
    profileChanged(p);
    return null;
  },
  fixes_list: () => ({ ...ledger.fixesView(activeProfile()), ...ledger.suggestionsView() }),
  fixes_import: () =>
    fixWrite((p) => {
      // Cleared first: an import the user undid runs again only from here.
      const before = p.migration_error;
      p.migration_error = null;
      try {
        ledger.ledgerOf(p);
      } catch (e) {
        p.migration_error = before;
        throw e;
      }
      return ledger.fixesView(p);
    }),
  fix_create: (args) => fixWrite((p) => ledger.fixView(p, ledger.createFix(p, args.draft as FixDraft))),
  fix_create_many: (args) =>
    fixWrite((p) => {
      // All or none: the drafts go into a copy that replaces the profile once every one is made.
      const work = structuredClone(p);
      const ids = ((args.drafts as FixDraft[] | undefined) ?? []).map((d) => ledger.createFix(work, d));
      Object.assign(p, work);
      return [...new Set(ids)].map((id) => ledger.fixView(p, id));
    }),
  fix_suggestion_apply: (args) =>
    fixWrite((p) => {
      // One write: each remove undone alone, then the drafts; all or none.
      const work = structuredClone(p);
      for (const id of (args.removes as string[] | undefined) ?? []) {
        ledger.find(work, id);
        ledger.undoOne(work, id);
      }
      const ids = ((args.drafts as FixDraft[] | undefined) ?? []).map((d) => ledger.createFix(work, d));
      Object.assign(p, work);
      return [...new Set(ids)].map((id) => ledger.fixView(p, id));
    }),
  fix_undo_preview: (args) => ledger.undoPreview(activeProfile(), String(args.fixId)),
  fix_undo: (args) =>
    fixWrite((p) => {
      ledger.undoFix(p, String(args.fixId), Boolean(args.confirmed));
      return null;
    }),
  fix_journal_undo_preview: (args) => ledger.journalUndoPreview(activeProfile(), Number(args.entryIndex)),
  fix_journal_undo: (args) =>
    fixWrite((p) => {
      ledger.journalUndo(p, Number(args.entryIndex), (args.choice as JournalUndoChoice | null | undefined) ?? null);
      return null;
    }),
  fix_set_paused: (args) =>
    fixWrite((p) => {
      ledger.setPaused(p, String(args.fixId), Boolean(args.paused));
      return ledger.fixView(p, String(args.fixId));
    }),
  fix_accept: (args) =>
    fixWrite((p) => {
      ledger.accept(p, String(args.fixId));
      return ledger.fixView(p, String(args.fixId));
    }),
  fix_rebind: (args) =>
    fixWrite((p) => {
      ledger.rebind(p, String(args.fixId), String(args.patchId));
      return ledger.fixView(p, String(args.fixId));
    }),
  patch_detail: (args) => {
    const id = String(args.patchId);
    return patchDetailFor(id, activeProfile()) ?? fail("not_found", `No loaded patch is called '${id}'.`);
  },
  stacks_get: (args) => stacksFor(activeProfile(), args.targetFile == null ? null : String(args.targetFile)),
  stacking_set: (args) => {
    ensureNotRunning();
    const p = activeProfile();
    if (p.id === "vanilla") fail("invalid_input", "The Vanilla profile cannot be changed.");
    p.stacking = args.mode as StackMode;
    log.stackingSets.push(p.stacking);
    p.notices_seen = [...new Set([...(p.notices_seen ?? []), "stacking_v1"])];
    profileChanged(p);
    return null;
  },
  assets_ledger: () => ready(assets.ledgerWith(assets.choicesOf(activeProfile()))),
  asset_choice_preview: (args) => assets.choicePreview(activeProfile(), String(args.key), String(args.winner)),
  asset_choice_clear: (args) =>
    fixWrite((p) => {
      const f = (p.fixes ?? []).find((x) => !x.paused && x.change.kind === "asset_choice" && x.change.key === String(args.key));
      if (!f) fail("not_found", "There is no choice for that any more.");
      ledger.undoFix(p, f!.id, true);
      return null;
    }),
  faction_choices: () => ready(assets.factionChoices(activeProfile())),
  faction_pick_set: (args) => {
    ensureNotRunning();
    const p = activeProfile();
    if (p.id === "vanilla") fail("invalid_input", "The Vanilla profile cannot be changed.");
    assets.pickFaction(p, String(args.side), args.factionId == null ? null : String(args.factionId));
    profileChanged(p);
    return null;
  },
  line_rules_set: (args) => {
    ensureNotRunning();
    const p = activeProfile();
    if (p.id === "vanilla") fail("invalid_input", "The Vanilla profile cannot be changed.");
    p.line_rules_v2 = Boolean(args.on);
    p.notices_seen = [...new Set([...(p.notices_seen ?? []), "stacking_v2"])];
    profileChanged(p);
    return null;
  },
  notice_dismiss: (args) => {
    const key = String(args.key);
    if (!["stacking_v1", "stacking_v2", "fixes_import_v1"].includes(key)) fail("invalid_input", `There is no notice called '${key}'.`);
    ensureNotRunning();
    const p = activeProfile();
    if (p.id !== "vanilla" && !(p.notices_seen ?? []).includes(key)) {
      log.noticeDismissals.push(key);
      p.notices_seen = [...(p.notices_seen ?? []), key];
      profileChanged(p);
    }
    return null;
  },
};
