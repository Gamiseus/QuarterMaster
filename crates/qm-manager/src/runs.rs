//! Run log (D67): one entry per launch in `profiles/<id>/runs.json`, oldest first, at most
//! `MAX_RUNS`. The launcher opens an entry before deploy and finishes it after the game exits,
//! before the Mods junction goes, so each profile keeps its last run after the game closes.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use qm_core::handshake::{
    write_file_atomic, CrashReport, Diagnostics, FixBinding, GlobalChange, HookCalls,
    LaunchConfig, LoadError, LoadOrderPlan, SaveStamp,
};
use qm_core::lenient::Lenient;
use qm_core::patch::table::PatchTable;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::storage::QuarterMasterPaths;
use crate::ModProfile;

pub const RUNS_FILE: &str = "runs.json";
/// Set while the profile's staged Mods folder holds a test plan (a trial or a run without
/// mods), so a launcher restart still stages the profile's own plan again.
pub const TEST_PLAN_FILE: &str = "test-plan.json";
/// Entries kept per profile; the oldest go first.
pub const MAX_RUNS: usize = 20;
/// Unknown note for a run that was still open when the next launch started.
const NOTE_LEFT_OPEN: &str = "QuarterMaster was not watching when the game closed.";

/// How a run ended.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// The game quit through love.quit (the footer's clean-exit mark).
    Clean,
    /// A Lua error reached QuarterMaster's crash handler.
    LuaError,
    /// The game could not use the plan or launch settings, or mods failed to load.
    LoadError,
    /// The process ended with a non-zero exit code and no clean-exit mark.
    NativeCrash,
    /// Exit code 0 without the clean-exit mark (closed from outside love.quit).
    Closed,
    /// Not known: no exit code and no mark, or the run is still open. Also any outcome a
    /// newer launcher wrote.
    #[default]
    #[serde(other)]
    Unknown,
}

/// What the game's patch-report.json said about the run, copied before the junction goes.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatchReportSummary {
    pub qm_version: String,
    pub timestamp: String,
    pub profile_id: Option<String>,
    pub run_id: Option<String>,
    pub clean_exit: bool,
    pub total_patches_applied: u32,
    pub total_warnings: u32,
    pub total_failures: u32,
    pub builtin_matches: u32,
    pub plan_error: Option<String>,
    pub plan_warnings: Vec<String>,
    pub load_errors: Vec<LoadError>,
    pub fix_bindings: Vec<FixBinding>,
    pub launch_config_error: Option<String>,
    pub dll_degraded: Option<String>,
    /// A diagnostic test run's trace of the watched globals (Phase E D89); else empty.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub globals_trace: Vec<GlobalChange>,
    /// A diagnostic test run's count of who called the watched functions (D90).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub hook_calls: Vec<HookCalls>,
}

impl PatchReportSummary {
    /// Reads the fields leniently: a field of the wrong type is left at its default, and a
    /// list keeps the entries that read.
    pub fn from_json(v: &Value) -> Self {
        Self {
            qm_version: str_of(v, "qm_version").unwrap_or_default(),
            timestamp: str_of(v, "timestamp").unwrap_or_default(),
            profile_id: str_of(v, "profile_id"),
            run_id: str_of(v, "run_id"),
            clean_exit: v
                .get("clean_exit")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            total_patches_applied: u32_of(v, "total_patches_applied"),
            total_warnings: u32_of(v, "total_warnings"),
            total_failures: u32_of(v, "total_failures"),
            builtin_matches: u32_of(v, "builtin_matches"),
            plan_error: str_of(v, "plan_error"),
            plan_warnings: list_of(v, "plan_warnings"),
            load_errors: list_of(v, "load_errors"),
            fix_bindings: list_of(v, "fix_bindings"),
            launch_config_error: str_of(v, "launch_config_error"),
            dll_degraded: str_of(v, "dll_degraded"),
            globals_trace: list_of(v, "globals_trace"),
            hook_calls: list_of(v, "hook_calls"),
        }
    }

    /// The first reason the game did not load the profile as planned.
    pub fn load_problem(&self) -> Option<String> {
        if let Some(e) = &self.launch_config_error {
            return Some(e.clone());
        }
        if let Some(e) = &self.plan_error {
            return Some(e.clone());
        }
        if let Some(e) = self.load_errors.first() {
            let name = if e.mod_id.is_empty() {
                &e.folder
            } else {
                &e.mod_id
            };
            return Some(format!("{name}: {}", e.message));
        }
        self.dll_degraded.clone()
    }

    /// Only some mods failed to load: the plan, the launch settings and the DLL were fine.
    fn only_mods_failed(&self) -> bool {
        self.launch_config_error.is_none()
            && self.plan_error.is_none()
            && self.dll_degraded.is_none()
    }
}

/// What a trial run leaves out (D68). It changes a copy of the profile in memory; the profile
/// file is never written.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrialChanges {
    /// Ledger fix ids to pause.
    pub pause_fixes: Vec<String>,
    /// Patch ids to turn off.
    pub disable_patches: Vec<String>,
    /// Mod ids to turn off.
    pub disable_mods: Vec<String>,
}

impl TrialChanges {
    pub fn is_empty(&self) -> bool {
        self.pause_fixes.is_empty()
            && self.disable_patches.is_empty()
            && self.disable_mods.is_empty()
    }

    /// `p` as the trial runs it: the mods turned off, the fixes paused (only their own legacy
    /// entries go, so the rest of the plan is the one a normal launch stages) and the patches
    /// turned off. The preview and the staging both use it, so the trial runs the plan the
    /// preview showed.
    pub fn apply(&self, p: &mut ModProfile) {
        p.enabled_mods
            .retain(|m| !self.disable_mods.iter().any(|d| d.eq_ignore_ascii_case(m)));
        crate::fixes::pause_in_place(p, &self.pause_fixes);
        for id in &self.disable_patches {
            if !p.disabled_patches.contains(id) {
                p.disabled_patches.push(id.clone());
            }
        }
    }
}

/// Set for a safe-launch trial run (D68).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrialInfo {
    /// The bisect this trial belongs to; None for a single suspect test.
    pub bisect_id: Option<String>,
    /// 1 for the first trial.
    pub step: u32,
    /// What the trial left out, as the user reads it ("the adjusted copy 'X'").
    pub label: String,
    pub pause_fixes: Vec<String>,
    pub disable_patches: Vec<String>,
    pub disable_mods: Vec<String>,
    /// The save slot the trial ran on and the backup made just before it.
    pub save_slot_id: Option<String>,
    pub save_backup_id: Option<String>,
    /// The search this trial belongs to has ended (found, not found, stopped or replaced).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub search_ended: bool,
    /// Set for a diagnostic test run (Phase E D91): what the game watches. Such a run
    /// leaves nothing out.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub diagnostics: Option<Diagnostics>,
}

impl TrialInfo {
    pub fn changes(&self) -> TrialChanges {
        TrialChanges {
            pause_fixes: self.pause_fixes.clone(),
            disable_patches: self.disable_patches.clone(),
            disable_mods: self.disable_mods.clone(),
        }
    }
}

/// A test plan staged in place of the profile's own (D68). Kept in `TEST_PLAN_FILE` until the
/// profile's own plan is staged again.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StagedTestPlan {
    /// What the test leaves out, as the user reads it; None for a run without mods.
    pub label: Option<String>,
    pub staged_at: String,
}

/// One launch of a profile.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunEntry {
    pub run_id: String,
    pub started_at: String,
    /// None while the run is open.
    pub ended_at: Option<String>,
    pub exit_code: Option<u32>,
    /// `PatchTable::stamp` of the staged plan, as hex. Equal within one launcher build means
    /// the same patches and plan.
    pub plan_stamp: Option<String>,
    /// The staged mods in load order: id, version.
    pub mods: Vec<(String, String)>,
    /// Ledger fixes that were not paused.
    pub active_fixes: Vec<String>,
    pub report: Option<PatchReportSummary>,
    /// Ids of the crash reports of this run.
    pub crashes: Vec<String>,
    pub outcome: RunOutcome,
    /// The first crash's error, the load problem, or why the outcome is unknown.
    pub message: Option<String>,
    pub trial: Option<TrialInfo>,
}

impl RunEntry {
    pub fn is_open(&self) -> bool {
        self.ended_at.is_none()
    }
}

/// A new run id: a random UUID.
pub fn new_run_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The outcome of a finished run and its message (see `RunOutcome` for the order).
pub fn outcome_of(
    exit_code: Option<u32>,
    report: Option<&PatchReportSummary>,
    crashes: &[&CrashReport],
) -> (RunOutcome, Option<String>) {
    // A DLL panic is caught and the game goes on with builtins only; that is a load problem.
    if let Some(c) = crashes.iter().find(|c| c.crash_kind != "rust_panic") {
        return (RunOutcome::LuaError, Some(c.error_message.clone()));
    }
    let clean = report.is_some_and(|r| r.clean_exit);
    if !clean && exit_code.is_some_and(|c| c != 0) {
        return (RunOutcome::NativeCrash, None);
    }
    if let Some(why) = report.and_then(PatchReportSummary::load_problem) {
        return (RunOutcome::LoadError, Some(why));
    }
    if let Some(c) = crashes.first() {
        return (RunOutcome::LoadError, Some(c.error_message.clone()));
    }
    if clean {
        return (RunOutcome::Clean, None);
    }
    match exit_code {
        Some(0) => (RunOutcome::Closed, None),
        _ => (RunOutcome::Unknown, None),
    }
}

/// Whether test run `run` brought back the problem of a crashed run whose outcome was
/// `original` (D68). Some(true): the same kind of failure came back. Some(false): it did not.
/// None: the run does not tell (ask the user). A test is read against the crash it tests: for a
/// Lua error or a native crash, a load problem of some mod in a run that quit cleanly does not
/// count; for a load error, only a load problem does.
pub fn problem_came_back(original: RunOutcome, run: &RunEntry) -> Option<bool> {
    let report = run.report.as_ref();
    if original == RunOutcome::LoadError {
        if run.outcome == RunOutcome::LoadError
            || report.and_then(PatchReportSummary::load_problem).is_some()
        {
            return Some(true);
        }
        // The game wrote its report with no load problem: the mods loaded.
        return (report.is_some() || run.outcome == RunOutcome::Clean).then_some(false);
    }
    match run.outcome {
        RunOutcome::LuaError | RunOutcome::NativeCrash => Some(true),
        RunOutcome::Clean => Some(false),
        // A clean quit with some mods not loaded: the crash did not come back. A plan, launch
        // settings or DLL problem (or a DLL panic) means the mods did not run as planned.
        RunOutcome::LoadError => report
            .filter(|r| r.clean_exit && r.only_mods_failed() && run.crashes.is_empty())
            .map(|_| false),
        RunOutcome::Closed | RunOutcome::Unknown => None,
    }
}

/// Parses a report or entry time: RFC 3339, with or without a time zone annotation.
fn parse_time(s: &str) -> Option<Timestamp> {
    s.parse::<Timestamp>().ok()
}

/// `at` lies within the run, from its start to `end`. The DLL takes its times from the same
/// clock after the launcher opened the run.
fn within(entry: &RunEntry, at: &str, end: Timestamp) -> bool {
    let (Some(start), Some(at)) = (parse_time(&entry.started_at), parse_time(at)) else {
        return false;
    };
    at >= start && at <= end
}

/// The game's report belongs to the run: the same run id, or (an old DLL without run ids)
/// the same profile and a time within the run.
pub fn report_belongs(
    entry: &RunEntry,
    profile_id: &str,
    report: &PatchReportSummary,
    end: Timestamp,
) -> bool {
    match &report.run_id {
        Some(id) => *id == entry.run_id,
        None => {
            report.profile_id.as_deref().is_none_or(|p| p == profile_id)
                && within(entry, &report.timestamp, end)
        }
    }
}

/// A crash report belongs to the run: the same run id, or (an old DLL) the same profile and
/// a time within the run.
pub fn crash_belongs(
    entry: &RunEntry,
    profile_id: &str,
    crash: &CrashReport,
    end: Timestamp,
) -> bool {
    match &crash.run_id {
        Some(id) => *id == entry.run_id,
        None => {
            crash
                .active_profile_id
                .as_deref()
                .is_none_or(|p| p == profile_id)
                && within(entry, &crash.timestamp, end)
        }
    }
}

/// Ends `entry` at `end`: links the report and crashes that belong to it and sets the
/// outcome. `note` is the message when nothing else explains an unknown outcome.
pub fn finish_entry(
    entry: &mut RunEntry,
    profile_id: &str,
    reports: &[PatchReportSummary],
    crashes: &[CrashReport],
    exit_code: Option<u32>,
    end: Timestamp,
    note: Option<&str>,
) {
    let report = reports
        .iter()
        .find(|r| report_belongs(entry, profile_id, r, end))
        .cloned();
    let mut linked: Vec<&CrashReport> = crashes
        .iter()
        .filter(|c| crash_belongs(entry, profile_id, c, end))
        .collect();
    linked.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    let (outcome, message) = outcome_of(exit_code, report.as_ref(), &linked);
    entry.ended_at = Some(end.to_string());
    entry.exit_code = exit_code;
    entry.crashes = linked.iter().map(|c| c.id.clone()).collect();
    entry.report = report;
    entry.outcome = outcome;
    entry.message = message.or_else(|| {
        (outcome == RunOutcome::Unknown)
            .then(|| note.map(str::to_string))
            .flatten()
    });
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn u32_of(v: &Value, key: &str) -> u32 {
    v.get(key)
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn list_of<T: DeserializeOwned>(v: &Value, key: &str) -> Vec<T> {
    match v.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| serde_json::from_value(x.clone()).ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// Why a runs file did not read.
enum ReadRunsError {
    /// The file is there but could not be read; it may be fine.
    Io(String),
    /// The file is not a JSON list.
    Invalid(String),
}

/// The entries of a runs file that read.
fn read_runs(path: &Path) -> Result<Vec<RunEntry>, ReadRunsError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(ReadRunsError::Io(format!(
                "{} could not be read: {e}",
                path.display()
            )))
        }
    };
    let items: Vec<Value> = serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|e| ReadRunsError::Invalid(format!("{} is not valid: {e}", path.display())))?;
    Ok(items
        .into_iter()
        .filter_map(|x| serde_json::from_value(x).ok())
        .collect())
}

/// A crash id as a file name: letters, digits, `-` and `_` only.
fn crash_file_name(id: &str) -> String {
    let stem: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}.json")
}

/// Reads `<mods_dir>/qm/patch-report.json` as a summary; None when it is missing or not JSON.
fn read_report(mods_dir: &Path) -> Option<PatchReportSummary> {
    let raw = std::fs::read_to_string(mods_dir.join("qm").join("patch-report.json")).ok()?;
    let v: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    Some(PatchReportSummary::from_json(&v))
}

impl QuarterMasterPaths {
    pub fn runs_file(&self, profile_id: &str) -> PathBuf {
        self.profiles_dir.join(profile_id).join(RUNS_FILE)
    }

    /// The profile's runs, oldest first. Entries that do not read are left out; an
    /// unreadable file is an empty log.
    pub fn list_runs(&self, profile_id: &str) -> Vec<RunEntry> {
        read_runs(&self.runs_file(profile_id)).unwrap_or_default()
    }

    /// The log to change and write back. A file that is not valid JSON is kept beside it
    /// under a new name and the log starts again; a file that could not be read is an Err,
    /// so it is never written over.
    fn runs_to_change(&self, profile_id: &str) -> Result<Vec<RunEntry>, String> {
        let path = self.runs_file(profile_id);
        match read_runs(&path) {
            Ok(runs) => Ok(runs),
            Err(ReadRunsError::Io(e)) => Err(e),
            Err(ReadRunsError::Invalid(e)) => {
                let stamp = Timestamp::now().as_millisecond();
                let mut bad = path.with_extension("json.bad");
                let mut n = 0;
                while bad.exists() {
                    n += 1;
                    bad = path.with_extension(format!("json.{stamp}-{n}.bad"));
                }
                std::fs::copy(&path, &bad)
                    .map_err(|c| format!("{e}; it could not be kept aside: {c}"))?;
                Ok(Vec::new())
            }
        }
    }

    /// Keeps a copy of the crash reports `ids` in the launcher's crash log, which outlives the
    /// staged Mods folder (every staging removes `qm/log/crashes`). A copy already there is
    /// left as it is.
    fn keep_crashes(&self, crashes: &[CrashReport], ids: &[String]) {
        let dir = self.logs_dir.join("crashes");
        for c in crashes.iter().filter(|c| ids.contains(&c.id)) {
            let file = dir.join(crash_file_name(&c.id));
            if file.exists() {
                continue;
            }
            let Ok(json) = serde_json::to_string_pretty(c) else {
                continue;
            };
            // A crash that is not kept is only lost from the log once the folder is staged again.
            let _ = std::fs::create_dir_all(&dir)
                .map_err(|e| e.to_string())
                .and_then(|()| write_file_atomic(&file, json.as_bytes()));
        }
    }

    /// The profile's newest run.
    pub fn last_run(&self, profile_id: &str) -> Option<RunEntry> {
        self.list_runs(profile_id).pop()
    }

    /// The profile's newest run that is still open (the game was running when the launcher
    /// last looked).
    pub fn open_run(&self, profile_id: &str) -> Option<RunEntry> {
        self.last_run(profile_id).filter(RunEntry::is_open)
    }

    /// Writes the log through a temp file, keeping the newest `MAX_RUNS`. `runs` comes from
    /// `runs_to_change`. When runs drop out, the crash copies no log links any more go too.
    fn save_runs(&self, profile_id: &str, mut runs: Vec<RunEntry>) -> Result<(), String> {
        let path = self.runs_file(profile_id);
        let dropped = runs.len() > MAX_RUNS;
        if dropped {
            runs.drain(..runs.len() - MAX_RUNS);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(&runs)
            .map_err(|e| format!("Failed to serialize {RUNS_FILE}: {e}"))?;
        write_file_atomic(&path, json.as_bytes())?;
        if dropped {
            self.prune_crash_copies();
        }
        Ok(())
    }

    /// Removes the crash copies in `logs_dir/crashes` that no run in any profile's log links.
    /// Only files that read as a crash report go, and nothing goes while some profile's log
    /// cannot be read. Returns how many copies were removed.
    pub fn prune_crash_copies(&self) -> usize {
        let Ok(profiles) = std::fs::read_dir(&self.profiles_dir) else {
            return 0;
        };
        let mut linked = std::collections::HashSet::new();
        for dir in profiles.filter_map(Result::ok).map(|e| e.path()) {
            if !dir.is_dir() {
                continue;
            }
            match read_runs(&dir.join(RUNS_FILE)) {
                Ok(runs) => linked.extend(runs.into_iter().flat_map(|r| r.crashes)),
                Err(_) => return 0,
            }
        }
        let Ok(files) = std::fs::read_dir(self.logs_dir.join("crashes")) else {
            return 0;
        };
        let mut removed = 0;
        for file in files.filter_map(Result::ok).map(|e| e.path()) {
            if file.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(c) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|raw| serde_json::from_str::<CrashReport>(&raw).ok())
            else {
                continue;
            };
            if !linked.contains(&c.id) && std::fs::remove_file(&file).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    pub fn test_plan_file(&self, profile_id: &str) -> PathBuf {
        self.profiles_dir.join(profile_id).join(TEST_PLAN_FILE)
    }

    /// The test plan staged for `profile_id`, if any. A mark that does not read still counts.
    pub fn staged_test_plan(&self, profile_id: &str) -> Option<StagedTestPlan> {
        let raw = match std::fs::read_to_string(self.test_plan_file(profile_id)) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => return Some(StagedTestPlan::default()),
        };
        Some(serde_json::from_str(raw.trim_start_matches('\u{feff}')).unwrap_or_default())
    }

    /// Marks that a test plan is about to be staged for `profile_id`. Call it before staging,
    /// so a staging that fails half way is put back too.
    pub fn mark_test_plan_staged(
        &self,
        profile_id: &str,
        label: Option<&str>,
    ) -> Result<(), String> {
        let path = self.test_plan_file(profile_id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
        }
        let plan = StagedTestPlan {
            label: label.map(str::to_string),
            staged_at: Timestamp::now().to_string(),
        };
        let json = serde_json::to_string_pretty(&plan)
            .map_err(|e| format!("Failed to serialize {TEST_PLAN_FILE}: {e}"))?;
        write_file_atomic(&path, json.as_bytes())
    }

    /// The profile's own plan is staged again: the test plan mark goes.
    pub fn clear_test_plan_staged(&self, profile_id: &str) -> Result<(), String> {
        let path = self.test_plan_file(profile_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Failed to remove {}: {e}", path.display())),
        }
    }

    /// Opens a run for a launch of `profile_id`, whose Mods folder is staged: a new run id in
    /// the staged launch-config.json (read back, as the game reads it), the staged mods, the
    /// active fixes and the plan stamp. Runs still open are finished first; the launcher
    /// finishes them before staging (`finish_open_runs`), while their report and crashes are
    /// still there. Err only when launch-config.json cannot take the run id; a log that
    /// cannot be written is left as it is.
    pub fn begin_run(&self, profile_id: &str, without_mods: bool) -> Result<RunEntry, String> {
        self.begin_run_with(profile_id, without_mods, None, &BTreeMap::new())
    }

    /// `begin_run` for a trial run (D68): the entry carries `trial`, and the fixes the trial
    /// pauses are not counted as active.
    pub fn begin_trial_run(&self, profile_id: &str, trial: TrialInfo) -> Result<RunEntry, String> {
        self.begin_run_with(profile_id, false, Some(trial), &BTreeMap::new())
    }

    /// `begin_run` or `begin_trial_run` with the save fields each mod writes (mod id ->
    /// fields, from the simulation's registry), which go into the save stamp (Phase E D85).
    pub fn begin_run_stamped(
        &self,
        profile_id: &str,
        without_mods: bool,
        trial: Option<TrialInfo>,
        save_fields: &BTreeMap<String, Vec<String>>,
    ) -> Result<RunEntry, String> {
        self.begin_run_with(profile_id, without_mods, trial, save_fields)
    }

    fn begin_run_with(
        &self,
        profile_id: &str,
        without_mods: bool,
        trial: Option<TrialInfo>,
        save_fields: &BTreeMap<String, Vec<String>>,
    ) -> Result<RunEntry, String> {
        self.finish_open_runs(profile_id);
        let mods_dir = self.profiles_dir.join(profile_id).join("Mods");
        let run_id = new_run_id();
        let mut cfg = LaunchConfig::load_from_mod_dir(&mods_dir)?.ok_or_else(|| {
            "launch-config.json is missing from the staged Mods folder".to_string()
        })?;
        let order = LoadOrderPlan::read(&mods_dir)
            .plan
            .map(|p| p.enabled_mods)
            .unwrap_or_default();
        let mods: Vec<(String, String)> = order
            .into_iter()
            .map(|id| {
                let version = cfg.active_mods.get(&id).cloned().unwrap_or_default();
                (id, version)
            })
            .collect();
        cfg.run_id = Some(run_id.clone());
        // Only a diagnostic test run watches anything (D91).
        cfg.diagnostics = trial
            .as_ref()
            .and_then(|t| t.diagnostics.clone())
            .map(Into::into);
        // Only a run with mods stamps the saves it loads: a run without mods leaves a save
        // as it found it.
        cfg.save_stamp = (!without_mods && !mods.is_empty()).then(|| {
            SaveStamp {
                profile_id: profile_id.to_string(),
                profile_name: cfg.profile_name.clone().unwrap_or_default(),
                mods: mods.iter().cloned().collect(),
                fields: save_fields
                    .iter()
                    .filter(|(id, _)| mods.iter().any(|(m, _)| m.eq_ignore_ascii_case(id)))
                    .map(|(id, fields)| (id.clone(), fields.clone()))
                    .collect(),
            }
            .into()
        });
        cfg.save_to_mod_dir(&mods_dir)?;
        match LaunchConfig::load_from_mod_dir(&mods_dir)? {
            Some(c) if c.run_id.as_deref() == Some(run_id.as_str()) => {}
            _ => return Err("launch-config.json did not keep the run id".to_string()),
        }

        let active_fixes = if without_mods {
            Vec::new()
        } else {
            self.get_profile(profile_id)
                .map(|p| {
                    p.fixes
                        .iter()
                        .filter_map(Lenient::ok)
                        .filter(|f| !f.paused)
                        .filter(|f| {
                            trial
                                .as_ref()
                                .is_none_or(|t| !t.pause_fixes.contains(&f.id))
                        })
                        .map(|f| f.id.clone())
                        .collect()
                })
                .unwrap_or_default()
        };
        let plan_stamp = PatchTable::load(&mods_dir)
            .ok()
            .map(|t| format!("{:016x}", t.stamp()));
        let entry = RunEntry {
            run_id,
            started_at: Timestamp::now().to_string(),
            plan_stamp,
            mods,
            active_fixes,
            trial,
            ..RunEntry::default()
        };
        let mut runs = match self.runs_to_change(profile_id) {
            Ok(runs) => runs,
            // The run id is in launch-config.json either way; the log is left as it is.
            Err(_) => return Ok(entry),
        };
        runs.push(entry.clone());
        // A log that does not save loses only this entry.
        let _ = self.save_runs(profile_id, runs);
        Ok(entry)
    }

    /// The reports the game may have written for a run of `profile_id`: the game folder's
    /// Mods (a copied folder when no junction could be made), then the staged folder.
    fn run_reports(&self, profile_id: &str) -> Vec<PatchReportSummary> {
        let mut dirs = Vec::new();
        if let Some(game_dir) = self
            .load_settings()
            .game_exe_path
            .as_deref()
            .and_then(Path::parent)
        {
            dirs.push(game_dir.join("Mods"));
        }
        dirs.push(self.profiles_dir.join(profile_id).join("Mods"));
        dirs.iter().filter_map(|d| read_report(d)).collect()
    }

    /// Finishes run `run_id` of `profile_id` after the game exited: the exit code, the
    /// game's report and the crash reports of this run (kept in the launcher's crash log),
    /// and the outcome. Call it before the Mods junction is removed. Ok(None) when the log
    /// has no such run.
    pub fn finish_run(
        &self,
        profile_id: &str,
        run_id: &str,
        exit_code: Option<u32>,
        note: Option<&str>,
    ) -> Result<Option<RunEntry>, String> {
        let mut runs = self.runs_to_change(profile_id)?;
        let Some(entry) = runs.iter_mut().find(|r| r.run_id == run_id) else {
            return Ok(None);
        };
        let reports = self.run_reports(profile_id);
        let crashes = self.list_crash_reports_for(profile_id);
        finish_entry(
            entry,
            profile_id,
            &reports,
            &crashes,
            exit_code,
            Timestamp::now(),
            note,
        );
        self.keep_crashes(&crashes, &entry.crashes);
        let done = entry.clone();
        self.save_runs(profile_id, runs)?;
        Ok(Some(done))
    }

    /// Finishes every open run of `profile_id` with no exit code (the launcher did not see
    /// the game close), from what the runs left in the game and staged folders. Call it
    /// before the profile is staged again, which removes the staged report and crashes.
    pub fn finish_open_runs(&self, profile_id: &str) {
        if !self.list_runs(profile_id).iter().any(RunEntry::is_open) {
            return;
        }
        let Ok(mut runs) = self.runs_to_change(profile_id) else {
            return;
        };
        let reports = self.run_reports(profile_id);
        let crashes = self.list_crash_reports_for(profile_id);
        let now = Timestamp::now();
        for entry in runs.iter_mut().filter(|r| r.is_open()) {
            finish_entry(
                entry,
                profile_id,
                &reports,
                &crashes,
                None,
                now,
                Some(NOTE_LEFT_OPEN),
            );
            self.keep_crashes(&crashes, &entry.crashes);
        }
        let _ = self.save_runs(profile_id, runs);
    }

    /// Marks the trial runs of search `bisect_id` of `profile_id` as from a search that
    /// ended. Returns whether any entry changed.
    pub fn mark_search_ended(&self, profile_id: &str, bisect_id: &str) -> bool {
        let Ok(mut runs) = self.runs_to_change(profile_id) else {
            return false;
        };
        let mut changed = false;
        for t in runs.iter_mut().filter_map(|r| r.trial.as_mut()) {
            if t.bisect_id.as_deref() == Some(bisect_id) && !t.search_ended {
                t.search_ended = true;
                changed = true;
            }
        }
        changed && self.save_runs(profile_id, runs).is_ok()
    }
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
