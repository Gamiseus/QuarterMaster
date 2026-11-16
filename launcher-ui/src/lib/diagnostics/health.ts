import type { FixBinding, HealthPatchReport, HealthReport, RunEntry, StagedTestPlan } from "../bindings";

export type HealthTone = "ok" | "warn" | "danger" | "neutral";

/** A fix offered on the row itself. */
export type HealthAction = "clear_overwrite" | "acknowledge_update" | "find_cause";

export interface HealthRow {
  label: string;
  tone: HealthTone;
  detail: string;
  action?: HealthAction;
}

function hookRow(r: HealthReport): HealthRow {
  const label = "Hook DLL";
  if (!r.source_dll_ready) {
    return { label, tone: "danger", detail: "qm_winmm.dll is missing next to the launcher, so mods cannot be loaded." };
  }
  if (!r.dll_present) return { label, tone: "neutral", detail: "Not installed. QuarterMaster installs it at launch." };
  if (!r.dll_is_known) {
    return { label, tone: "danger", detail: "The winmm.dll in the game folder is not a QuarterMaster build." };
  }
  if (!r.dll_matches_build) {
    return { label, tone: "warn", detail: "Installed, but from a different QuarterMaster build. Repair it to match." };
  }
  return { label, tone: "ok", detail: "Installed and matches this build." };
}

function folderRow(r: HealthReport, busy: boolean): HealthRow {
  const label = "Game folder";
  if (!r.is_game_dir_modded) return { label, tone: "ok", detail: "Clean. QuarterMaster sets it up at launch." };
  if (busy) return { label, tone: "ok", detail: "Set up for the running game." };
  return { label, tone: "warn", detail: "Still set up from the last launch. Restore it if the game misbehaves without mods." };
}

/** A test run's plan is staged in place of the profile's own (D68), until the game is closed. */
export function testPlanRow(t: StagedTestPlan, busy: boolean): HealthRow {
  const what = t.label ? `The plan of the test without ${t.label} is staged.` : "The plan of the run without mods is staged.";
  if (busy) return { label: "Test plan", tone: "neutral", detail: `${what} Your own plan is staged again when the game closes.` };
  return { label: "Test plan", tone: "warn", detail: `${what} Your own plan is staged again at the next launch.` };
}

/** "Waiting for core: the adjusted copy 'X' is skipped while core is off." for each waiting fix. */
export function waitingNotes(bindings: FixBinding[]): string[] {
  return bindings
    .filter((b) => b.status.state === "waiting")
    .map((b) => (b.detail.endsWith(".") ? b.detail : `${b.detail}.`));
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Looks up a profile's display name by id. */
export type ProfileNameOf = (id: string) => string | undefined;

/** What the last-run rows read from a report: the game folder's report and the run log's copy both have it. */
type RunReport = Pick<HealthPatchReport, "launch_config_error" | "plan_error" | "plan_warnings" | "load_errors" | "fix_bindings">;

/** Exit codes from 0x80000000 up are Windows status codes, shown in hex. */
export function exitCodeText(code: number): string {
  return code >= 0x80000000 ? `0x${code.toString(16).toUpperCase()}` : String(code);
}

/** A run that ended like this offers "Find the cause" (D68). */
export function runCrashed(run: RunEntry): boolean {
  return run.ended_at !== null && (run.outcome === "lua_error" || run.outcome === "load_error" || run.outcome === "native_crash");
}

/** How the profile's last run ended, from its run log entry (D67). A crash offers "Find the cause" (D68). */
export function runOutcomeRow(run: RunEntry): HealthRow {
  const row = outcomeRow(run);
  const label = run.trial?.diagnostics ? "Diagnostic test run." : `Test without ${run.trial?.label}.`;
  const test = run.trial ? { ...row, detail: `${label} ${row.detail}` } : row;
  return runCrashed(run) ? { ...test, action: "find_cause" } : test;
}

function outcomeRow(run: RunEntry): HealthRow {
  const label = "Last run";
  if (run.ended_at === null) return { label, tone: "neutral", detail: "Not finished yet." };
  switch (run.outcome) {
    case "clean":
      return { label, tone: "ok", detail: "The game closed normally." };
    case "closed":
      return { label, tone: "neutral", detail: "The game closed without its normal quit." };
    case "lua_error":
      return { label, tone: "danger", detail: run.message ? `The game stopped with a Lua error: ${run.message}` : "The game stopped with a Lua error." };
    case "load_error":
      return { label, tone: "danger", detail: run.message ? `The game did not load everything: ${run.message}` : "The game did not load everything." };
    case "native_crash":
      return {
        label,
        tone: "danger",
        detail: run.exit_code === null ? "The game crashed." : `The game crashed (exit code ${exitCodeText(run.exit_code)}).`,
      };
    case "unknown":
      return { label, tone: "neutral", detail: run.message ? `Could not tell how the game closed. ${run.message}` : "Could not tell how the game closed." };
  }
}

/**
 * How the active profile's last run went and what the game reported about its plan and mods.
 * The report is the game folder's patch-report.json while it is there, else the copy in the
 * profile's run log (`last_run`, D72), which outlives the game folder. A game folder report of
 * another profile gives one neutral row when the active profile has no run of its own. A null
 * `profile_id` on either side keeps the rows, so a plan error the game could not tie to a profile
 * still shows. A game folder report that cannot be read gives one row, whatever the profile.
 */
export function lastRunRows(r: HealthReport, activeId: string | null, nameOf: ProfileNameOf = () => undefined): HealthRow[] {
  const live = r.patch_report;
  const run = r.last_run;
  const otherProfile = live !== null && live.profile_id !== null && activeId !== null && live.profile_id !== activeId;
  if (live && otherProfile && !run) {
    const id = live.profile_id ?? "";
    const name = nameOf(id) ?? id;
    return [{ label: "Last run", tone: "neutral", detail: `The last run used the profile "${name}".` }];
  }
  const rows: HealthRow[] = run ? [runOutcomeRow(run)] : [];
  if (!live && r.patch_report_error) {
    rows.push({ label: "Last run: report", tone: "warn", detail: `The last run's report could not be read: ${r.patch_report_error}` });
  }
  const p: RunReport | null = live && !otherProfile ? live : (run?.report ?? null);
  if (!p) return rows;
  if (p.launch_config_error) {
    rows.push({
      label: "Last run: launch settings",
      tone: "danger",
      detail: `The game started with no mods and its default save folder. ${p.launch_config_error}`,
    });
  }
  if (p.plan_error) {
    rows.push({ label: "Last run: load order", tone: "danger", detail: `The game started with no mods. ${p.plan_error}` });
  } else if (p.plan_warnings.length > 0) {
    rows.push({
      label: "Last run: load order",
      tone: "warn",
      detail: `The game skipped ${plural(p.plan_warnings.length, "entry", "entries")} of the load order file. ${p.plan_warnings[0]}`,
    });
  }
  if (p.load_errors.length > 0) {
    const names = p.load_errors.map((e) => e.mod_id || e.folder).join(", ");
    rows.push({
      label: "Last run: mods",
      tone: "danger",
      detail: `${plural(p.load_errors.length, "mod", "mods")} could not be loaded and ${p.load_errors.length === 1 ? "was" : "were"} skipped: ${names}.`,
    });
  }
  const waiting = waitingNotes(p.fix_bindings);
  if (waiting.length > 0) {
    rows.push({ label: "Last run: adjusted copies", tone: "neutral", detail: waiting.join(" ") });
  }
  return rows;
}

/**
 * The health card's rows, in display order. `busy` means the game is starting, running or closing.
 * The last-run rows show only for `activeProfileId` (see `lastRunRows`).
 */
export function healthRows(
  r: HealthReport,
  busy: boolean,
  activeProfileId: string | null,
  nameOf: ProfileNameOf = () => undefined,
): HealthRow[] {
  const files = r.overwrite_files.length;
  return [
    hookRow(r),
    folderRow(r, busy),
    r.game_update_detected
      ? {
          label: "Game update",
          tone: "warn",
          detail: "The game changed since the last launch. Check that your mods still work, then mark it as checked.",
          action: "acknowledge_update",
        }
      : { label: "Game update", tone: "ok", detail: "No change since the last launch." },
    r.steam_cloud_warning
      ? { label: "Steam Cloud", tone: "warn", detail: r.steam_cloud_warning }
      : { label: "Steam Cloud", tone: "ok", detail: "No conflict found." },
    files > 0
      ? {
          label: "Overwrite files",
          tone: "warn",
          detail: `${files} ${files === 1 ? "file" : "files"} in the overwrite folder.`,
          action: "clear_overwrite",
        }
      : { label: "Overwrite files", tone: "ok", detail: "None." },
    ...(r.test_plan ? [testPlanRow(r.test_plan, busy)] : []),
    ...lastRunRows(r, activeProfileId, nameOf),
  ];
}

/** Repair replaces a QuarterMaster DLL from another build; a foreign DLL is never touched. */
export function canRepairDll(r: HealthReport, busy: boolean): boolean {
  return r.dll_present && !r.dll_matches_build && r.dll_is_known && r.source_dll_ready && !busy;
}

export function canRestoreClean(r: HealthReport, busy: boolean): boolean {
  return (r.is_game_dir_modded || r.dll_present) && !busy;
}
