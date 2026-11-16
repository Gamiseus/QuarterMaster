import type {
  BisectStatus,
  BisectView,
  DiagnosticRunPreview,
  DiagnosticRunRequest,
  DiagnosticRunResult,
  Diagnostics,
  GameStatus,
  GlobalChange,
  HookCalls,
  HealthReport,
  LaunchMethod,
  LogSource,
  ModProfile,
  PreviewResult,
  RevealKind,
  RunEntry,
  StagedTestPlan,
  TrialChanges,
  TrialRequest,
  TrialResult,
  TrialSuspect,
} from "../../bindings";
import { emitMock } from "../../events";
import { readTest } from "../../diagnostics/safelaunch";
import { MockBisect, MSG_NOTHING, NOTE_RESTART, candidatesFor } from "../bisect";
import { attentionReport, crashes, healthyReport, logLines, mismatchReport, moddedReport, sampleRun } from "../diagnostics-data";
import { mods, profileDocs, ready } from "../fixtures";
import { runMockJob } from "../jobs";
import * as ledger from "../ledger";
import { GLUE_MOD, MOCK_PATCHES, disabledIn, previewFor } from "../patchworld";
import { activeProfile, ensureNotRunning, fail, isGameBusy, log, profileChanged, setGame, state, type Handlers } from "../state";
import { backup } from "./saves";

const UNKNOWN_DLL = "The winmm.dll in the game folder is not a QuarterMaster build. It was left in place.";

let health: HealthReport | null = null;

/** Built on first use, so `__qmMockInit.health` (read at install time) picks the starting report. */
function current(): HealthReport {
  const start = { modded: moddedReport, mismatch: mismatchReport, attention: attentionReport, healthy: healthyReport };
  health ??= start[state.init.health ?? "healthy"]();
  return health;
}

let runs: Record<string, RunEntry[]> | null = null;

/** Run logs by profile, oldest first; `__qmMockInit.lastRun` seeds one run of the active profile. */
function runLog(): Record<string, RunEntry[]> {
  if (runs === null) {
    runs = {};
    const outcome = state.init.lastRun;
    if (outcome) runs[activeProfile().id] = [sampleRun(outcome)];
  }
  return runs;
}

/** Like `runs_list`: newest first, only for a profile that exists. */
function runsList(profileId: string): RunEntry[] {
  if (!profileDocs[profileId]) fail("not_found", `Profile '${profileId}' was not found.`);
  return [...(runLog()[profileId] ?? [])].reverse();
}

/** The test plan staged while a mock test run runs, by profile (D68). */
const testPlans: Record<string, StagedTestPlan> = {};

/** The health report with the active profile's last run and staged test plan, as `check_system_health` reads it. */
function healthNow(): HealthReport {
  const id = activeProfile().id;
  return { ...current(), last_run: runLog()[id]?.at(-1) ?? null, test_plan: testPlans[id] ?? null };
}

function healthChanged(): void {
  setTimeout(() => emitMock({ type: "invalidated", scope: "health" }), 0);
}

/** Like the service: an unknown DLL is left in place and reported as `integrity` after the folder is cleaned. */
function restoreClean(): null {
  ensureNotRunning();
  const h = current();
  const unknown = state.init.restore === "integrity" || (h.dll_present && !h.dll_is_known);
  h.is_game_dir_modded = false;
  if (!unknown) {
    h.dll_present = false;
    h.dll_matches_build = false;
    h.dll_is_known = false;
  }
  healthChanged();
  if (unknown) fail("integrity", UNKNOWN_DLL);
  return null;
}

function repairDll(): string {
  ensureNotRunning();
  const h = current();
  if (!h.dll_present || !h.dll_is_known) fail("integrity", UNKNOWN_DLL);
  h.dll_matches_build = true;
  h.dll_sha256 = h.source_dll_sha256;
  healthChanged();
  return h.dll_sha256 ?? "";
}

function overwriteClear(): number {
  ensureNotRunning();
  const h = current();
  const n = h.overwrite_files.length;
  h.overwrite_files = [];
  healthChanged();
  return n;
}

function acknowledgeUpdate(): null {
  current().game_update_detected = null;
  healthChanged();
  return null;
}

function disableSuspect(crashId: string): null {
  ensureNotRunning();
  const c = crashes.find((x) => x.id === crashId) ?? fail("not_found", `Crash report '${crashId}' was not found.`);
  // An adjusted copy is the profile's: its fix is paused, its base mod stays on (D58).
  if (c.suspected_adjusted_copy) {
    const fix = c.suspected_fix_ids?.[0] ?? fail("not_found", "The crash names an adjusted copy but not the fix that made it.");
    const p = activeProfile();
    ledger.setPaused(p, fix, true);
    profileChanged(p);
    return null;
  }
  const id = c.suspected_mod_id ?? fail("not_found", `Crash report '${crashId}' does not name a suspected mod.`);
  const p = activeProfile();
  p.enabled_mods = p.enabled_mods.filter((m) => m !== id);
  profileChanged(p);
  return null;
}

const NO_CHANGES: TrialChanges = { pause_fixes: [], disable_patches: [], disable_mods: [] };

const modName = (id: string) => mods.find((m) => m.id === id)?.name ?? id;

/** Like `crash_suspect`: an adjusted copy's fix (D58), else the patch, else the mod. */
function suspectOf(crashId: string): TrialSuspect | null {
  const c = crashes.find((x) => x.id === crashId) ?? fail("not_found", `Crash report '${crashId}' was not found.`);
  if (c.suspected_adjusted_copy) {
    const fix = c.suspected_fix_ids?.[0];
    if (!fix) return null;
    const label = (activeProfile().fixes ?? []).find((f) => f.id === fix)?.label ?? c.suspected_patch_name ?? fix;
    return { kind: "fix", id: fix, label: `the fix "${label}"`, name: label, changes: { ...NO_CHANGES, pause_fixes: [fix] } };
  }
  if (c.suspected_patch_id) {
    const name = c.suspected_patch_name ?? c.suspected_patch_id;
    const label = c.suspected_mod_id ? `the patch "${name}" of ${modName(c.suspected_mod_id)}` : `the patch "${name}"`;
    return { kind: "patch", id: c.suspected_patch_id, label, name, changes: { ...NO_CHANGES, disable_patches: [c.suspected_patch_id] } };
  }
  if (c.suspected_mod_id) {
    const id = c.suspected_mod_id;
    return { kind: "mod", id, label: modName(id), name: modName(id), changes: { ...NO_CHANGES, disable_mods: [id] } };
  }
  return null;
}

/** Like `trial_preview`: the active profile with the test's changes, as a preview (nothing is saved). */
function trialPreview(c: TrialChanges): PreviewResult {
  const p = activeProfile();
  const copy = structuredClone(p) as ModProfile;
  copy.enabled_mods = copy.enabled_mods.filter((m) => !c.disable_mods.includes(m));
  // Pausing on the copy is not a ledger write, so it stays out of the log.
  const logged = log.fixes.length;
  for (const id of c.pause_fixes) ledger.setPaused(copy, id, true);
  log.fixes.length = logged;
  const changes = { disable: c.disable_patches, enable: [], order_rules: [], glue_patches: [], stacking: null, add_fixes: [], remove_fixes: [], focus_file: null };
  return previewFor(p, changes, copy);
}

let trialCount = 0;

/** `__qmMockInit.bisectCulprit`: the problem is gone only in a test that leaves the culprit out. */
function trialOutcome(c: TrialChanges): RunEntry["outcome"] {
  const culprit = state.init.bisectCulprit;
  if (!culprit) return state.init.trialOutcome ?? "clean";
  const left = [...c.pause_fixes, ...c.disable_patches, ...c.disable_mods];
  return left.includes(culprit) ? "clean" : "lua_error";
}

/** Like `game_launch_trial`: backs up the save, runs the game for a moment and logs a test run with `__qmMockInit.trialOutcome`. */
function launchTrial(req: TrialRequest): GameStatus {
  if (isGameBusy()) fail("game_running", "The game is already running.");
  const p = activeProfile();
  const empty = req.changes.pause_fixes.length + req.changes.disable_patches.length + req.changes.disable_mods.length === 0;
  if (empty) fail("invalid_input", "Choose what to leave out of the test.");
  const step = req.step ?? 1;
  if (state.init.trialBackupFails) fail("io", "Your save could not be backed up, so the test was not started.");
  const slot = p.linked_save_profile_id ?? "default";
  const saved = backup(slot, req.bisect_id ? `trial ${req.bisect_id} ${step}` : `trial ${step}`);
  log.trials.push(structuredClone(req));
  trialCount++;
  const run: RunEntry = {
    ...sampleRun("unknown"),
    run_id: `mock-trial-${trialCount}`,
    started_at: new Date().toISOString(),
    ended_at: null,
    exit_code: null,
    crashes: [],
    message: null,
    active_fixes: [],
    trial: { bisect_id: req.bisect_id ?? null, step, label: req.label, ...req.changes, save_slot_id: slot, save_backup_id: saved.id },
  };
  (runLog()[p.id] ??= []).push(run);
  testPlans[p.id] = { label: req.label, staged_at: run.started_at };
  healthChanged();
  const status: GameStatus = { stage: { stage: "deploying" }, profile_id: p.id, is_modded: true };
  state.game = status;
  setTimeout(() => setGame({ ...status, stage: { stage: "running", pid: 4242 } }), 150);
  setTimeout(() => {
    const outcome = trialOutcome(req.changes);
    run.outcome = outcome;
    run.ended_at = new Date().toISOString();
    run.exit_code = outcome === "native_crash" ? 0xc0000005 : 0;
    run.message = outcome === "lua_error" ? "attempt to index a nil value (field 'target')" : null;
    bisect?.afterRun(run);
    if (bisect) markEnded(bisect);
    // The profile's own plan is staged again after the run.
    delete testPlans[p.id];
    setGame({ stage: { stage: "idle" }, profile_id: null, is_modded: false });
    healthChanged();
  }, state.init.trialMs ?? 600);
  return status;
}

/** Like `diagnostic_run_preview`: the names the code checks flagged (scenario `traceSuggestions`). */
function diagnosticPreview(): DiagnosticRunPreview {
  if (!state.init.traceSuggestions) return { globals: [], hooks: [] };
  return {
    globals: [
      { name: "loadModSettings", checks: ["capture_before_define"] },
      { name: "saveModSettings", checks: ["redefined_global"] },
    ],
    hooks: [{ name: "onDeath", checks: ["double_hook"] }],
  };
}

const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
let diagnosticRun: DiagnosticRunResult | null = null;
let diagnosticCount = 0;

/** What the mock game reports for the watched names: two mods set the first global. */
function traceFor(watched: Diagnostics): Pick<DiagnosticRunResult, "globals_trace" | "hook_calls"> {
  const [first, ...rest] = watched.globals ?? [];
  const globals_trace: GlobalChange[] = [];
  const change = (name: string, mod_id: string, patch_id: string, line: number) =>
    globals_trace.push({ name, mod_id, patch_id, source: "@main.lua", line, value_type: "function", identity: `function: 0x${line}`, seq: globals_trace.length + 1 });
  if (first) {
    change(first, "tactical_overhaul", "tactical_overhaul:main_lua:3", 210);
    change(first, "gcore", "gcore:main_lua:1", 3120);
  }
  if (rest[0]) change(rest[0], "", "", 88);
  const hook_calls: HookCalls[] = (watched.hooks ?? []).map((name, i) =>
    i === 0
      ? {
          name,
          total: 3,
          missing: false,
          callers: [
            { source: "@includes/urb_engine.lua", line: 42133, calls: 2, patch_id: "g_special_events:urb:1", mod_id: "g_special_events" },
            { source: "@includes/urb_engine.lua", line: 42756, calls: 1, patch_id: null, mod_id: null },
          ],
        }
      : { name, total: 0, missing: true, callers: [] },
  );
  return { globals_trace, hook_calls };
}

/** Like `game_launch_diagnostic`: backs up the save, runs the game for a moment and keeps what it reported. */
function launchDiagnostic(req: DiagnosticRunRequest): GameStatus {
  if (isGameBusy()) fail("game_running", "The game is already running.");
  const p = activeProfile();
  if (p.enabled_mods.length === 0) fail("invalid_input", "The Vanilla profile loads no mods, so there is nothing to trace.");
  const clean = (names: string[], dotted: boolean) => {
    const out: string[] = [];
    for (const n of names.map((x) => x.trim()).filter((x) => x)) {
      const parts = n.split(".");
      const ok = parts.every((part) => NAME.test(part)) && parts.length <= (dotted ? 2 : 1);
      if (!ok) fail("invalid_input", dotted ? `'${n}' is not a function name. Use a name, or Table.name.` : `'${n}' is not a global name. Use letters, digits and _ only.`);
      if (!out.includes(n)) out.push(n);
    }
    return out;
  };
  const names = { globals: clean(req.globals, false), hooks: clean(req.hooks, true) };
  const watched: Diagnostics = names;
  if (names.globals.length + names.hooks.length === 0) fail("invalid_input", "Name at least one global or function to watch.");
  const slot = p.linked_save_profile_id ?? "default";
  backup(slot, "Before diagnostic run");
  diagnosticCount++;
  const run: DiagnosticRunResult = {
    run_id: `mock-diagnostic-${diagnosticCount}`,
    started_at: new Date().toISOString(),
    running: true,
    outcome: "unknown",
    reported: false,
    watched,
    globals_trace: [],
    hook_calls: [],
  };
  diagnosticRun = run;
  const status: GameStatus = { stage: { stage: "deploying" }, profile_id: p.id, is_modded: true };
  state.game = status;
  setTimeout(() => setGame({ ...status, stage: { stage: "running", pid: 4242 } }), 150);
  setTimeout(() => {
    diagnosticRun = { ...run, running: false, outcome: "clean", reported: true, ...traceFor(watched) };
    setGame({ stage: { stage: "idle" }, profile_id: null, is_modded: false });
    healthChanged();
  }, state.init.trialMs ?? 600);
  return status;
}

const CRASHED: RunEntry["outcome"][] = ["lua_error", "load_error", "native_crash"];

/** The search for a crash's cause (D68 step 2); in memory, like the service. */
let bisect: MockBisect | null = null;
let bisectCount = 0;

function noBisect(): never {
  fail("not_found", "No search for the cause is running. Start one from Find the cause.");
}

/** Like `crash_file_patches`: the enabled patches of the crash's files, the snippet's first. */
function filePatches(fileNames: string[], snippet: string[]): TrialSuspect[] {
  const p = activeProfile();
  const off = disabledIn(p);
  const list = MOCK_PATCHES.filter((x) => fileNames.includes(x.file) && x.mod !== GLUE_MOD && p.enabled_mods.includes(x.mod) && !off.has(x.id));
  const rank = (id: string) => (snippet.includes(id) ? snippet.indexOf(id) : Number.MAX_SAFE_INTEGER);
  return [...list]
    .sort((a, b) => rank(a.id) - rank(b.id))
    .map((x) => ({
      kind: "patch",
      id: x.id,
      label: `the patch "${x.name}" of ${modName(x.mod)}`,
      name: x.name,
      changes: { ...NO_CHANGES, disable_patches: [x.id] },
    }));
}

function bisectStart(runId: string): BisectView {
  if (isGameBusy()) fail("game_running", "The game is already running.");
  const p = activeProfile();
  if (p.id === "vanilla") fail("invalid_input", "The Vanilla profile has no mods to test.");
  const runs = runLog()[p.id] ?? [];
  const at = runs.findIndex((r) => r.run_id === runId);
  if (at < 0) fail("not_found", `Run '${runId}' of this profile was not found.`);
  const run = runs[at];
  if (!CRASHED.includes(run.outcome)) fail("invalid_input", "That run did not crash.");
  const lastClean = [...runs.slice(0, at)].reverse().find((r) => r.outcome === "clean" && r.trial === null) ?? null;
  const cleared: TrialChanges = { pause_fixes: [], disable_patches: [], disable_mods: [] };
  for (const r of runs.slice(at)) {
    if (!r.trial || readTest(r, run.outcome) !== "crashed") continue;
    cleared.pause_fixes.push(...r.trial.pause_fixes);
    cleared.disable_patches.push(...r.trial.disable_patches);
    cleared.disable_mods.push(...r.trial.disable_mods);
  }
  const reports = run.crashes.flatMap((id) => crashes.filter((c) => c.id === id));
  const suspect = run.crashes[0] && reports.length > 0 ? suspectOf(run.crashes[0]) : null;
  const fileNames = reports.flatMap((c) => (c.crashing_file ? [c.crashing_file] : []));
  const snippet = reports.flatMap((c) => c.code_snippet.flatMap((l) => (l.attributed_patch_id ? [l.attributed_patch_id] : [])));
  const list = candidatesFor({ suspect, fixes: p.fixes ?? [], lastClean, filePatches: filePatches(fileNames, snippet), cleared });
  if (list.length === 0) fail("not_found", MSG_NOTHING);
  bisectCount++;
  if (bisect) markEnded(bisect, true);
  bisect = new MockBisect(`b${String(bisectCount).padStart(7, "0")}`, p.id, run.run_id, list, run.outcome);
  return bisect.view();
}

/** Like `note_search_end`: the run log records that the search ended, so a restart says nothing. */
function markEnded(b: MockBisect, replaced = false): void {
  if ((!replaced && b.open) || b.trials.length === 0) return;
  for (const r of runLog()[b.profileId] ?? []) if (r.trial?.bisect_id === b.id) r.trial.search_ended = true;
}

function bisectGet(): BisectStatus {
  if (bisect) return { bisect: bisect.view(), note: null };
  const last = runLog()[activeProfile().id]?.at(-1);
  return { bisect: null, note: last?.trial?.bisect_id && !last.trial.search_ended ? NOTE_RESTART : null };
}

function bisectPreview(): { bisect: BisectView; preview: PreviewResult | null } {
  const b = bisect ?? noBisect();
  for (;;) {
    const pending = b.running ?? b.ask;
    const split = !b.open ? null : pending ? pending.leftOut : b.nextSplit();
    if (!split) return { bisect: b.view(), preview: null };
    const preview = trialPreview(b.changesFor(split));
    if (preview.impact.breaks_file && b.idle) {
      b.skip(split);
      markEnded(b);
      continue;
    }
    return { bisect: b.view(), preview };
  }
}

function bisectLaunch(method: LaunchMethod | null): GameStatus {
  const b = bisect ?? noBisect();
  if (isGameBusy()) fail("game_running", "The game is already running.");
  if (!b.open) fail("invalid_input", "This search for the cause has ended. Start a new one.");
  if (!b.idle) fail("conflict", "A test is still running or waiting for your answer.");
  if (b.profileId !== activeProfile().id) fail("conflict", "The search is for another profile. Switch back to it to go on.");
  const split = b.nextSplit() ?? fail("invalid_input", "This search for the cause has ended. Start a new one.");
  const step = b.step();
  b.running = { step, leftOut: split, runId: null };
  try {
    return launchTrial({ changes: b.changesFor(split), label: b.labelFor(split), method, bisect_id: b.id, step });
  } catch (e) {
    if (b.running?.step === step) b.running = null;
    throw e;
  }
}

function bisectAnswer(result: TrialResult): BisectView {
  const b = bisect ?? noBisect();
  const p = b.ask ?? fail("invalid_input", "No test is waiting for an answer.");
  b.ask = null;
  b.record(p.step, p.leftOut, p.runId, result);
  markEnded(b);
  return b.view();
}

function bisectStop(): BisectView {
  const b = bisect ?? noBisect();
  if (b.open) b.finish("stopped", null);
  markEnded(b);
  return b.view();
}

export const handlers: Handlers = {
  bisect_start: (args) => bisectStart(String(args.runId)),
  bisect_get: () => bisectGet(),
  bisect_preview: () => bisectPreview(),
  bisect_launch: (args) => bisectLaunch((args.method ?? null) as LaunchMethod | null),
  bisect_answer: (args) => bisectAnswer(args.result as TrialResult),
  bisect_stop: () => bisectStop(),
  health_get: () => ready(healthNow()),
  crash_suspect: (args) => suspectOf(String(args.crashId)),
  trial_preview: (args) => trialPreview(args.changes as TrialChanges),
  game_launch_trial: (args) => launchTrial(args.req as TrialRequest),
  diagnostic_run_preview: () => diagnosticPreview(),
  game_launch_diagnostic: (args) => launchDiagnostic(args.req as DiagnosticRunRequest),
  diagnostic_run_result: () => diagnosticRun,
  crashes_list: () => crashes,
  runs_list: (args) => runsList(String(args.profileId)),
  crash_disable_suspect: (args) => disableSuspect(String(args.crashId)),
  dll_repair: () => repairDll(),
  game_restore_clean: () => restoreClean(),
  overwrite_clear: () => overwriteClear(),
  game_update_acknowledge: () => acknowledgeUpdate(),
  diagnostic_bundle_build: () =>
    runMockJob("diagnostic_bundle", "Building the bug report", () => ({
      output: "C:\\QuarterMaster\\cache\\diagnostics\\bundle-20261106-101500.json",
    })),
  logs_tail: (args) => {
    const lines = logLines[args.source as LogSource] ?? [];
    return lines.slice(-Number(args.lines ?? lines.length));
  },
  reveal: (args) => {
    log.reveals.push(args.kind as RevealKind);
    return null;
  },
};
