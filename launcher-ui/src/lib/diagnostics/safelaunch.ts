import type { BisectCandidate, BisectStatus, BisectView, PatchReportSummary, RunEntry, RunOutcome, TrialSuspect } from "../bindings";

/** What a finished test run says about the problem: it came back, it did not, or the user is asked. */
export type TestReading = "crashed" | "fine" | "ask";

/** Whether the game's report says it did not load the profile as planned. */
function loadProblem(r: PatchReportSummary): boolean {
  return r.launch_config_error !== null || r.plan_error !== null || r.load_errors.length > 0 || r.dll_degraded !== null;
}

/**
 * Read from the run log entry against the crash the test is for (D68), as the service reads a
 * search test (`problem_came_back`). For a Lua error or a native crash, a crash report or a non-zero
 * exit code is the problem coming back, and a clean quit is fine even when some mod did not load.
 * For a load error, only a load problem counts. A run closed from outside the game's quit, or one
 * the launcher could not follow, is asked about.
 */
export function readTest(run: RunEntry, original: RunOutcome | null): TestReading {
  const report = run.report;
  if (original === "load_error") {
    if (run.outcome === "load_error" || (report !== null && loadProblem(report))) return "crashed";
    return report !== null || run.outcome === "clean" ? "fine" : "ask";
  }
  switch (run.outcome) {
    case "lua_error":
    case "native_crash":
      return "crashed";
    case "clean":
      return "fine";
    case "load_error": {
      // Only some mods failed to load, the game quit cleanly and no DLL problem: the crash is gone.
      const onlyMods =
        report !== null && report.launch_config_error === null && report.plan_error === null && report.dll_degraded === null;
      return onlyMods && report.clean_exit && run.crashes.length === 0 ? "fine" : "ask";
    }
    default:
      return "ask";
  }
}

/** The newest finished test run in `runs` (newest first, as `runs_list` gives them) that is not `before`. */
export function finishedTest(runs: RunEntry[], before: string | null): RunEntry | null {
  const run = runs.find((r) => r.trial !== null && r.ended_at !== null);
  return run && run.run_id !== before ? run : null;
}

/** The button that keeps the suspect off for good, through the preview the test showed. */
export function keepOffLabel(s: TrialSuspect): string {
  switch (s.kind) {
    case "fix":
      return "Pause the fix";
    case "patch":
      return "Turn off the patch";
    case "mod":
      return `Turn off ${s.label}`;
  }
}

/** The steps the dialog lists before a test. */
export function testSteps(s: TrialSuspect): string[] {
  return [
    "QuarterMaster backs up your save.",
    `The game starts without ${s.label}. Nothing in your profile changes.`,
    "Play until the problem would show, then quit the game.",
    "Come back here to see what the test found.",
  ];
}

/** "Test 2 of about 4": the step of the next or current test and the expected count (never below the step). */
export function testCount(v: BisectView): string {
  const n = v.next?.step ?? v.trials.length + 1;
  return `test ${n} of about ${Math.max(n, v.expected_trials)}`;
}

/** The candidate the search found, if it ended with one. */
export function foundCandidate(v: BisectView): BisectCandidate | null {
  return v.end === "found" && v.found !== null ? (v.candidates[v.found] ?? null) : null;
}

/** The labels of the candidates that may still be the cause. */
export function remainingLabels(v: BisectView): string[] {
  return v.remaining.flatMap((i) => (v.candidates[i] ? [v.candidates[i].suspect.label] : []));
}

/** A search that is still going for `profileId` (the dialog picks it up again when it opens). */
export function openSearch(status: BisectStatus, profileId: string | null): BisectView | null {
  const v = status.bisect;
  return v && v.end === null && v.profile_id === profileId ? v : null;
}

/** What an ended search says, apart from a found cause. */
export function searchEndText(v: BisectView): string {
  switch (v.end) {
    case "not_found":
    case "could_not_tell":
      return v.message ?? "The search could not find the cause.";
    case "stopped":
      return "Stopped. Nothing in your profile changed.";
    default:
      return "";
  }
}

/** The steps the dialog lists before a search. */
export const SEARCH_STEPS = [
  "QuarterMaster backs up your save before each test.",
  "Each test leaves out about half of the suspects that are left. Nothing in your profile changes.",
  "Play until the problem would show, then quit the game.",
  "Come back here: QuarterMaster reads the result and picks the next test.",
];
