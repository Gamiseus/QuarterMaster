import type { CrashReport, HealthReport, LogSource, RunEntry, RunOutcome } from "../bindings";

/** The hook installed and matching, the game folder clean: what a healthy idle setup looks like. */
export function healthyReport(): HealthReport {
  return {
    dll_present: true,
    is_game_dir_modded: false,
    source_dll_ready: true,
    dll_sha256: "a".repeat(64),
    source_dll_sha256: "a".repeat(64),
    dll_matches_build: true,
    dll_is_known: true,
    game_running_pid: null,
    game_update_detected: null,
    overwrite_files: [],
    steam_cloud_warning: null,
    patch_report: null,
    patch_report_error: null,
    recent_logs: [],
    last_run: null,
    test_plan: null,
  };
}

/** `__qmMockInit.lastRun`: a finished run of the active profile with this outcome (D67). */
export function sampleRun(outcome: RunOutcome): RunEntry {
  const crashed = outcome === "lua_error";
  return {
    run_id: "5b0e7c1e-6f43-4d6a-9a51-0d6c3f1e2a90",
    started_at: "2026-11-02T20:40:00+00:00[UTC]",
    ended_at: "2026-11-02T21:14:05+00:00[UTC]",
    exit_code: outcome === "native_crash" ? 0xc0000005 : 0,
    plan_stamp: "3f9a0c4d2b1e8f70",
    mods: [
      ["gcore", "1.4.0"],
      ["ai-tweaks", "0.9.2"],
    ],
    active_fixes: [],
    report: null,
    crashes: crashed ? ["crash-20261102-211400"] : [],
    outcome,
    message: crashed ? "attempt to index a nil value (field 'target')" : null,
    trial: null,
  };
}

/** `__qmMockInit.health = "modded"`: left set up after a launch, with a foreign winmm.dll. */
export function moddedReport(): HealthReport {
  return { ...healthyReport(), is_game_dir_modded: true, dll_is_known: false, dll_matches_build: false, dll_sha256: "f".repeat(64) };
}

/** `__qmMockInit.health = "attention"`: a game update to check and files left in the overwrite folder. */
export function attentionReport(): HealthReport {
  return {
    ...healthyReport(),
    game_update_detected: { previous_sha256: "c".repeat(64), current_sha256: "d".repeat(64) },
    overwrite_files: Array.from({ length: 12 }, (_, i) => `saves/autosave-${String(i + 1).padStart(2, "0")}.dat`),
  };
}

/** `__qmMockInit.health = "mismatch"`: a QuarterMaster DLL from another build, which Repair fixes. */
export function mismatchReport(): HealthReport {
  return { ...healthyReport(), dll_matches_build: false, dll_sha256: "b".repeat(64) };
}

export const crashes: CrashReport[] = [
  {
    id: "crash-20261102-211400",
    timestamp: "2026-11-02T21:14:00+00:00[UTC]",
    crash_kind: "lua_error",
    error_message: "attempt to index a nil value (field 'target')",
    traceback: "scripts/ai.lua:88: in function 'pickTarget'",
    crashing_file: "scripts/ai.lua",
    crashing_line: 88,
    suspected_mod_id: "ai-tweaks",
    suspected_patch_id: "ai-targeting",
    suspected_patch_name: "Targeting",
    code_snippet: [],
    active_profile_id: "default",
    active_mods: { gcore: "1.4.0", "ai-tweaks": "0.9.2" },
  },
  {
    id: "crash-20261030-180200",
    timestamp: "2026-10-30T18:02:00+00:00[UTC]",
    crash_kind: "out_of_memory",
    error_message: "not enough memory",
    traceback: "",
    crashing_file: null,
    crashing_line: null,
    suspected_mod_id: null,
    suspected_patch_id: null,
    suspected_patch_name: null,
    code_snippet: [],
    active_profile_id: "default",
    active_mods: { gcore: "1.4.0" },
  },
];

export const logLines: Record<LogSource, string[]> = {
  runtime: [
    "[qm] QuarterMaster hook loaded (5 mods)",
    "[qm] Patched scripts/ui/hud.lua (2 patches)",
    "[game] Mission 12 loaded",
    "[game] Reinforcements arrived",
  ],
  runtime_errors: ["[error] scripts/ui/hud.lua:41: attempt to index a nil value"],
  launcher: ["QuarterMaster started", "Settings loaded", "Mods indexed: 5"],
};
