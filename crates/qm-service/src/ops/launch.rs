//! Game launch state machine and the exit watcher that cleans the game folder.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::{QmEvent, Scope};
use crate::game::{GameStage, GameStatus};
use crate::ops::profiles::MSG_VANILLA_SLOT;
use crate::service::Service;
use qm_manager::runs::{RunEntry, TrialChanges, TrialInfo};
use qm_manager::{LaunchMethod, ModProfile, VANILLA_ID};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::MutexGuard;
use std::time::Instant;

const MSG_STEAM_TIMEOUT: &str = "Mud and Blood did not start within 60 seconds.";
const MSG_LOCKED_SLOT: &str =
    "This save slot is locked as vanilla. Pick another slot or create a new one for this profile.";
const BACKUP_LABEL: &str = "Auto-backup before game launch";
const MSG_LOST_TRACK: &str =
    "Lost track of the running game. Restart QuarterMaster after closing it.";
const MSG_RUN_ID: &str = "Could not write the run id into the launch settings.";
const MSG_TRIAL_EMPTY: &str = "Choose what to leave out of the test.";
const MSG_TRIAL_VANILLA: &str = "The Vanilla profile has no mods to test.";
const MSG_TRIAL_BACKUP: &str = "Your save could not be backed up, so the test was not started.";
const MSG_TEST_PLAN: &str =
    "Could not note that a test plan is staged, so the run was not started.";
/// The end of the refusal for a trial that breaks a file; a bisect skips that split.
pub(crate) const MSG_TRIAL_BREAKS_TAIL: &str = "from compiling, so it was not started.";

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LaunchRequest {
    pub profile_id: String,
    pub method: Option<LaunchMethod>,
    /// Start the game with no mods for this run (the pre-launch dialog's "Launch without
    /// mods" after a plan error, D16). The DLL still runs with its built-in edits, and the
    /// profile and its save slot stay active.
    #[serde(default)]
    #[specta(optional)]
    pub without_mods: bool,
}

/// A trial run of the active profile (D68): one launch with `changes` applied to a copy of
/// the profile. The profile file is never written.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TrialRequest {
    pub changes: TrialChanges,
    /// What the trial leaves out, as the user reads it.
    pub label: String,
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub method: Option<LaunchMethod>,
    /// The bisect this trial belongs to; none for a single suspect test.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub bisect_id: Option<String>,
    /// 1 for the first trial (the default).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub step: Option<u32>,
}

/// A diagnostic test run of the active profile (Phase E D91).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct DiagnosticRunRequest {
    /// Global names whose changes are noted while the game loads.
    pub globals: Vec<String>,
    /// Functions (`name` or `Table.name`) whose callers are counted.
    pub hooks: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[specta(optional)]
    pub method: Option<LaunchMethod>,
}

/// A name a diagnostic test run would watch, and the code checks that flagged it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WatchSuggestion {
    pub name: String,
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiagnosticRunPreview {
    pub globals: Vec<WatchSuggestion>,
    pub hooks: Vec<WatchSuggestion>,
}

/// What a diagnostic test run found.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct DiagnosticRunResult {
    pub run_id: String,
    pub started_at: String,
    /// The game has not closed yet (or the launcher did not see it close).
    pub running: bool,
    pub outcome: qm_manager::runs::RunOutcome,
    /// The game wrote a report for this run.
    pub reported: bool,
    pub watched: qm_core::handshake::Diagnostics,
    pub globals_trace: Vec<qm_core::handshake::GlobalChange>,
    pub hook_calls: Vec<qm_core::handshake::HookCalls>,
}

/// The code checks whose names a diagnostic run watches by default.
const WATCHED_CHECKS: [&str; 4] = [
    "redefined_global",
    "capture_before_define",
    "double_hook",
    "name_collision",
];
/// The most names of one kind a diagnostic run watches.
const MAX_WATCHED: usize = 50;
/// The label of a diagnostic run in the run log.
pub const DIAGNOSTIC_LABEL: &str = "Diagnostic test run";
const DIAGNOSTIC_BACKUP_LABEL: &str = "Before diagnostic run";
const MSG_DIAGNOSTIC_VANILLA: &str =
    "The Vanilla profile loads no mods, so there is nothing to trace.";
const MSG_DIAGNOSTIC_EMPTY: &str = "Name at least one global or function to watch.";
const MSG_DIAGNOSTIC_MANY: &str = "A diagnostic run watches at most 50 globals and 50 functions.";
const MSG_BAD_GLOBAL_TAIL: &str = "is not a global name. Use letters, digits and _ only.";
const MSG_BAD_HOOK_TAIL: &str = "is not a function name. Use a name, or Table.name.";

/// Whether `name` is a Lua identifier, or (with `dotted`) also `Table.name`.
fn is_watchable(name: &str, dotted: bool) -> bool {
    let ident = |s: &str| {
        let mut chars = s.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    match name.split_once('.') {
        Some((table, key)) => dotted && ident(table) && ident(key),
        None => ident(name),
    }
}

/// What a trial leaves out for a suspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SuspectKind {
    Fix,
    Patch,
    Mod,
}

/// What a crash report blames, as a trial run can leave it out (D68 step 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TrialSuspect {
    pub kind: SuspectKind,
    /// The fix, patch or mod id.
    pub id: String,
    /// As the user reads it: `the fix "<label>"`, `the patch "<name>" of <mod>` or the mod name.
    pub label: String,
    /// The fix label, patch name or mod name alone.
    pub name: String,
    pub changes: TrialChanges,
}

impl Service {
    fn lock_status(&self) -> MutexGuard<'_, GameStatus> {
        self.inner.game.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Current status; a game found running while nothing tracks it is attached first.
    pub fn game_status(&self) -> GameStatus {
        let st = self.lock_status().clone();
        if st.is_busy() {
            return st;
        }
        match self.inner.probe.find_game() {
            Some(pid) => {
                self.attach_to_running(pid);
                self.lock_status().clone()
            }
            None => st,
        }
    }

    /// Deploy (or restore for vanilla), spawn the game and start the exit watcher.
    pub fn game_launch(&self, req: LaunchRequest) -> QmResult<GameStatus> {
        self.launch(req, None)
    }

    /// Launches the active profile once with `req.changes` (D68). The trial's plan is the one
    /// its preview shows; a trial that stops a file from compiling is refused. The profile's
    /// save slot is always backed up first, whatever `auto_backup_saves` says, and a failed
    /// backup refuses the trial before anything is staged. The profile's own plan is staged
    /// again after the run.
    pub fn game_launch_trial(&self, req: TrialRequest) -> QmResult<GameStatus> {
        self.ensure_not_running()?;
        if req.changes.is_empty() {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_TRIAL_EMPTY));
        }
        let profile_id = self.paths().load_settings().active_profile_id;
        if profile_id == VANILLA_ID {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_TRIAL_VANILLA));
        }
        let check = self.trial_check(&req.changes)?;
        if check.impact.breaks_file {
            let files = check.impact.compile_errors_new.join(", ");
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("This test would stop {files} {MSG_TRIAL_BREAKS_TAIL}"),
            ));
        }
        let trial = TrialInfo {
            bisect_id: req.bisect_id.clone(),
            step: req.step.unwrap_or(1).max(1),
            label: req.label.clone(),
            pause_fixes: req.changes.pause_fixes.clone(),
            disable_patches: req.changes.disable_patches.clone(),
            disable_mods: req.changes.disable_mods.clone(),
            save_slot_id: None,
            save_backup_id: None,
            search_ended: false,
            diagnostics: None,
        };
        let launch = LaunchRequest {
            profile_id,
            method: req.method,
            without_mods: false,
        };
        self.launch(launch, Some(trial))
    }

    /// The names a diagnostic test run would watch by default (Phase E D91): the globals
    /// and functions the code checks flagged in the last simulation (`redefined_global`,
    /// `capture_before_define`, `double_hook`, `name_collision`). Empty while there is no
    /// simulation.
    pub fn diagnostic_run_preview(&self) -> DiagnosticRunPreview {
        let mut globals: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut hooks: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if let Some(report) = self.inner.caches.simulation.get() {
            for c in &report.conflicts {
                let Some(rest) = c.id.strip_prefix("semantic:") else {
                    continue;
                };
                let (Some(check), Some((_, name))) =
                    (rest.split(':').next(), rest.rsplit_once('@'))
                else {
                    continue;
                };
                if !WATCHED_CHECKS.contains(&check) {
                    continue;
                }
                let add = |map: &mut BTreeMap<String, Vec<String>>| {
                    let checks = map.entry(name.to_string()).or_default();
                    if !checks.iter().any(|c| c == check) {
                        checks.push(check.to_string());
                    }
                };
                if is_watchable(name, false) {
                    add(&mut globals);
                }
                if check == "double_hook" && is_watchable(name, true) {
                    add(&mut hooks);
                }
            }
        }
        let list = |map: BTreeMap<String, Vec<String>>| {
            map.into_iter()
                .take(MAX_WATCHED)
                .map(|(name, checks)| WatchSuggestion { name, checks })
                .collect()
        };
        DiagnosticRunPreview {
            globals: list(globals),
            hooks: list(hooks),
        }
    }

    /// Launches the active profile once as a diagnostic test run (Phase E D91): the game
    /// notes every change of the globals in `req.globals` while it loads and counts who
    /// calls the functions in `req.hooks`. Nothing is left out of the profile. As for a
    /// trial, the save slot is backed up first and the profile is staged again afterwards.
    pub fn game_launch_diagnostic(&self, req: DiagnosticRunRequest) -> QmResult<GameStatus> {
        self.ensure_not_running()?;
        let profile_id = self.paths().load_settings().active_profile_id;
        if profile_id == VANILLA_ID {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                MSG_DIAGNOSTIC_VANILLA,
            ));
        }
        let clean = |names: &[String], dotted: bool| -> QmResult<Vec<String>> {
            let mut out: Vec<String> = Vec::new();
            for n in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
                if !is_watchable(n, dotted) {
                    return Err(QmError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "'{n}' {}",
                            if dotted {
                                MSG_BAD_HOOK_TAIL
                            } else {
                                MSG_BAD_GLOBAL_TAIL
                            }
                        ),
                    ));
                }
                if !out.iter().any(|o| o == n) {
                    out.push(n.to_string());
                }
            }
            Ok(out)
        };
        let diagnostics = qm_core::handshake::Diagnostics {
            globals: clean(&req.globals, false)?,
            hooks: clean(&req.hooks, true)?,
        };
        if diagnostics.globals.is_empty() && diagnostics.hooks.is_empty() {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_DIAGNOSTIC_EMPTY));
        }
        if diagnostics.globals.len() > MAX_WATCHED || diagnostics.hooks.len() > MAX_WATCHED {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_DIAGNOSTIC_MANY));
        }
        let trial = TrialInfo {
            step: 1,
            label: DIAGNOSTIC_LABEL.to_string(),
            diagnostics: Some(diagnostics),
            ..TrialInfo::default()
        };
        let launch = LaunchRequest {
            profile_id,
            method: req.method,
            without_mods: false,
        };
        self.launch(launch, Some(trial))
    }

    /// The active profile's newest diagnostic test run with what the game reported, or None
    /// when the profile has none in its run log.
    pub fn diagnostic_run_result(&self) -> Option<DiagnosticRunResult> {
        let paths = self.paths();
        let profile_id = paths.load_settings().active_profile_id;
        let run = paths
            .list_runs(&profile_id)
            .into_iter()
            .rev()
            .find(|r| r.trial.as_ref().is_some_and(|t| t.diagnostics.is_some()))?;
        let watched = run.trial.as_ref().and_then(|t| t.diagnostics.clone())?;
        let report = run.report.clone().unwrap_or_default();
        Some(DiagnosticRunResult {
            run_id: run.run_id.clone(),
            started_at: run.started_at.clone(),
            running: run.is_open(),
            outcome: run.outcome,
            reported: run.report.is_some(),
            watched,
            globals_trace: report.globals_trace,
            hook_calls: report.hook_calls,
        })
    }

    fn launch(&self, req: LaunchRequest, trial: Option<TrialInfo>) -> QmResult<GameStatus> {
        self.ensure_not_running()?;
        let paths = self.paths();
        let prepared = self.prepare_launch(&req.profile_id);
        self.drain_profile_notices();
        let (profile, notices) = prepared?;
        for summary in notices {
            self.inner.sink.emit(QmEvent::Recovered { summary });
        }
        let settings = paths.load_settings();
        let method = match req
            .method
            .or(profile.launch_method)
            .unwrap_or(settings.default_launch_method)
        {
            // Old files only: vanilla is the Vanilla profile now, started directly.
            LaunchMethod::Vanilla => LaunchMethod::DirectExe,
            m => m,
        };
        let has_mods = profile.id != VANILLA_ID && !profile.enabled_mods.is_empty();
        let modded = !req.without_mods && has_mods;

        self.claim(GameStatus {
            stage: GameStage::Deploying,
            profile_id: Some(profile.id.clone()),
            is_modded: modded,
        })?;
        // After the claim, so an install job that starts now sees the game as busy.
        if let Err(e) = self.ensure_no_install() {
            self.set_status(GameStatus::idle());
            return Err(e);
        }
        // Before staging removes the staged report and crashes: a run left open (the launcher
        // closed while the game ran) is finished from them (D67).
        paths.finish_open_runs(&profile.id);
        // Before anything is staged: a trial that cannot back up the save does not start.
        let trial = match trial
            .map(|t| self.back_up_for_trial(&profile, t))
            .transpose()
        {
            Ok(t) => t,
            Err(e) => {
                self.set_status(GameStatus::idle());
                return Err(e);
            }
        };

        let spawned: QmResult<u32> = (|| {
            if req.without_mods {
                // A fresh plan with no mods: the profile's own plan is not used, so a bad one
                // does not stop this launch.
                self.mark_test_plan(&profile.id, None)?;
                self.stage_profile_without_mods(&profile.id)?;
            } else if let Some(t) = &trial {
                self.mark_test_plan(&profile.id, Some(&t.label))?;
                self.stage_profile_trial(&profile.id, &t.changes())?;
            } else {
                self.stage_profile(&profile.id)?;
            }
            // The run id goes into the staged launch-config.json before deploy (D67).
            self.begin_run(&profile.id, req.without_mods, trial.clone())?;
            if has_mods {
                // Without mods the DLL still runs, so its built-in safety edits and the save
                // identity apply (D16).
                self.tx_deploy(&profile.id)?;
                if modded {
                    // Sticky: this slot has now been played with mods.
                    paths
                        .mark_slot_modded(&profile.linked_save_profile_id)
                        .or_kind(ErrorKind::Io)?;
                    self.invalidate(Scope::Saves);
                }
            } else {
                self.tx_restore_clean()?;
            }
            // Failures are covered by fail_with_restore.
            self.invalidate(Scope::Health);
            // A trial backed up the slot before staging.
            if settings.auto_backup_saves && trial.is_none() {
                paths
                    .create_save_backup(&profile.linked_save_profile_id, BACKUP_LABEL)
                    .or_kind(ErrorKind::Io)?;
            }
            self.inner.probe.spawn(&|| {
                paths
                    .spawn_game_process(&profile, method)
                    .or_kind(ErrorKind::Io)
            })
        })();
        let spawned = spawned.inspect_err(|e| self.fail_with_restore(e.message.clone()))?;

        let (st, started) = if method == LaunchMethod::SteamUrl {
            let st = self.set_stage(GameStage::WaitingForSteam);
            let svc = self.clone();
            (
                st,
                spawn_thread("qm-steam-wait", move || svc.wait_for_steam()),
            )
        } else {
            let st = self.set_stage(GameStage::Running { pid: spawned });
            let svc = self.clone();
            (
                st,
                spawn_thread("qm-game-watch", move || svc.watch(spawned)),
            )
        };
        match started {
            Ok(()) => Ok(st),
            Err(e) => {
                // The game may be running; leave the folder alone.
                tracing::warn!("{e}");
                self.set_stage(GameStage::Failed {
                    reason: MSG_LOST_TRACK.to_string(),
                });
                Err(QmError::new(ErrorKind::Internal, MSG_LOST_TRACK).with_details(e.message))
            }
        }
    }

    /// The checks before a launch, on the files as they are on disk (hand edits cannot get past
    /// them). Returns the profile to launch and notices for the user. Refuses with `conflict`:
    /// - a profile other than Vanilla linked to the Vanilla slot (it is relinked to "New slot on
    ///   first launch", so the next launch works);
    /// - a profile with mods on a slot locked as vanilla.
    ///
    /// Edited built-in Vanilla files are put back and the launch goes ahead.
    fn prepare_launch(&self, profile_id: &str) -> QmResult<(ModProfile, Vec<String>)> {
        let paths = self.paths();
        let mut notices = Vec::new();
        let profile = paths.get_profile(profile_id).or_kind(ErrorKind::NotFound)?;
        if profile.id != VANILLA_ID && profile.linked_save_profile_id == VANILLA_ID {
            let mut relinked = profile.clone();
            relinked.linked_save_profile_id = String::new();
            self.save_profile(&relinked).or_kind(ErrorKind::Io)?;
            self.notify(Scope::Profiles);
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!(
                    "{MSG_VANILLA_SLOT} {} now gets a new save slot of its own when it next launches.",
                    profile.name
                ),
            ));
        }
        // An unreadable file counts as changed: the repair keeps a copy before rewriting it.
        let profile_tampered = match paths.vanilla_profile_on_disk() {
            Ok(p) => p.is_some_and(|p| {
                !p.enabled_mods.is_empty() || p.linked_save_profile_id != VANILLA_ID
            }),
            Err(_) => true,
        };
        let slot_tampered = match paths.vanilla_slot_on_disk() {
            Ok(s) => s.is_some_and(|s| !s.is_vanilla_clean || !s.locked_vanilla || s.tags.modded),
            Err(_) => true,
        };
        if profile_tampered || slot_tampered {
            paths.repair_vanilla_builtins().or_kind(ErrorKind::Io)?;
            notices.push(
                "The built-in Vanilla files were changed outside QuarterMaster and have been put back."
                    .to_string(),
            );
        }
        if let Some(name) = paths
            .ensure_linked_slot(&profile.id)
            .or_kind(ErrorKind::Io)?
        {
            notices.push(format!(
                "Created the save slot \"{name}\" for {}.",
                profile.name
            ));
            self.invalidate(Scope::Saves);
            self.invalidate(Scope::Profiles);
        }
        let profile = paths.get_profile(profile_id).or_kind(ErrorKind::NotFound)?;
        let locked = paths
            .list_save_profiles()
            .into_iter()
            .find(|s| s.id == profile.linked_save_profile_id)
            .is_some_and(|s| s.locked_vanilla);
        if locked && profile.id != VANILLA_ID && !profile.enabled_mods.is_empty() {
            return Err(QmError::new(ErrorKind::Conflict, MSG_LOCKED_SLOT));
        }
        Ok((profile, notices))
    }

    /// Restore the game folder to vanilla; refused while the game runs.
    pub fn game_restore_clean(&self) -> QmResult<()> {
        self.ensure_not_running()?;
        self.claim(GameStatus {
            stage: GameStage::CleaningUp,
            profile_id: None,
            is_modded: false,
        })?;
        let result = self.tx_restore_clean();
        self.set_status(GameStatus::idle());
        // Also on Err: an unknown DLL is left in place but the junction step still ran.
        self.invalidate(Scope::Health);
        result
    }

    /// `GameRunning` when a launch is in progress or an untracked game is found (which is attached).
    pub(crate) fn ensure_not_running(&self) -> QmResult<()> {
        if self.game_status().is_busy() {
            Err(QmError::game_running())
        } else {
            Ok(())
        }
    }

    /// `ensure_not_running`, but only when `slot_id` is the active save slot (the one the game writes).
    pub(crate) fn ensure_not_running_for(&self, slot_id: &str) -> QmResult<()> {
        if self.paths().load_settings().active_save_profile_id == slot_id {
            self.ensure_not_running()
        } else {
            Ok(())
        }
    }

    /// Track a game that was already running (e.g. found at startup) and clean up when it exits.
    pub(crate) fn attach_to_running(&self, pid: u32) {
        let settings = self.paths().load_settings();
        let is_modded = settings
            .game_exe_path
            .as_deref()
            .and_then(Path::parent)
            .is_some_and(|d| std::fs::symlink_metadata(d.join("Mods")).is_ok());
        let status = GameStatus {
            stage: GameStage::Running { pid },
            profile_id: Some(settings.active_profile_id),
            is_modded,
        };
        {
            let mut g = self.lock_status();
            if g.is_busy() {
                return;
            }
            *g = status.clone();
        }
        self.emit(status);
        let svc = self.clone();
        if let Err(e) = spawn_thread("qm-game-watch", move || svc.watch(pid)) {
            self.set_stage(GameStage::Failed { reason: e.message });
        }
    }

    /// Atomically move from a non-busy stage to `status`.
    fn claim(&self, status: GameStatus) -> QmResult<()> {
        {
            let mut g = self.lock_status();
            if g.is_busy() {
                return Err(QmError::game_running());
            }
            *g = status.clone();
        }
        self.emit(status);
        Ok(())
    }

    /// Poll for the game process Steam starts.
    fn wait_for_steam(&self) {
        let deadline = Instant::now() + self.timings.steam_timeout;
        loop {
            if let Some(pid) = self.inner.probe.find_game() {
                self.set_stage(GameStage::Running { pid });
                self.watch(pid);
                return;
            }
            if Instant::now() >= deadline {
                self.fail_with_restore(MSG_STEAM_TIMEOUT.to_string());
                return;
            }
            std::thread::sleep(self.timings.steam_poll);
        }
    }

    /// Block until `pid` exits, then finish the run and clean the game folder. Holds no lock
    /// while waiting.
    fn watch(&self, pid: u32) {
        let exit_code = match self.inner.probe.wait_for_exit(pid) {
            Ok(code) => code,
            Err(e) => {
                tracing::warn!("waiting for game process {pid} failed: {e}");
                // Only touch the folder when the game is really gone.
                if self.inner.probe.find_game().is_none() {
                    self.fail_with_restore(e.message);
                } else {
                    self.set_stage(GameStage::Failed { reason: e.message });
                }
                return;
            }
        };
        self.set_stage(GameStage::CleaningUp);
        // Before the junction goes, so the run log keeps the game's report (D67).
        if let Some(run) = self.finish_run(exit_code, None, true) {
            self.bisect_after_run(&run);
        }
        let result = self
            .recover()
            .map(|_| ())
            .and_then(|()| self.tx_restore_clean());
        self.restage_after_run();
        self.invalidate(Scope::Health);
        match result {
            Ok(()) => {
                self.set_status(GameStatus::idle());
            }
            Err(e) => {
                tracing::warn!("cleanup after game exit failed: {e}");
                self.set_stage(GameStage::Failed { reason: e.message });
            }
        }
    }

    /// Attempt a restore, then enter `Failed` (restore first so the folder is clean when observed).
    /// An unknown winmm.dll left in place (Integrity) does not count as a failed restore.
    fn fail_with_restore(&self, reason: String) {
        if let Some(run) = self.finish_run(None, Some(&reason), false) {
            self.bisect_after_run(&run);
        }
        let reason = match self.tx_restore_clean() {
            Ok(()) => reason,
            Err(e) if e.kind == ErrorKind::Integrity => {
                tracing::warn!("{e}");
                reason
            }
            Err(e) => {
                tracing::warn!("restore after failed launch did not complete: {e}");
                format!(
                    "{}. Restoring the game folder also failed: {}",
                    reason.trim_end_matches('.'),
                    e.message
                )
            }
        };
        self.restage_after_run();
        self.invalidate(Scope::Health);
        self.set_stage(GameStage::Failed { reason });
    }

    /// Backs up the profile's save slot for a trial (D68) and records the backup in `trial`.
    fn back_up_for_trial(&self, profile: &ModProfile, mut trial: TrialInfo) -> QmResult<TrialInfo> {
        let slot = &profile.linked_save_profile_id;
        let label = match &trial.bisect_id {
            _ if trial.diagnostics.is_some() => DIAGNOSTIC_BACKUP_LABEL.to_string(),
            Some(b) => format!("trial {b} {}", trial.step),
            None => format!("trial {}", trial.step),
        };
        let backup = self
            .paths()
            .create_save_backup_strict(slot, &label)
            .map_err(|m| QmError::new(ErrorKind::Io, MSG_TRIAL_BACKUP).with_details(m))?;
        self.invalidate(Scope::Saves);
        trial.save_slot_id = Some(slot.clone());
        trial.save_backup_id = Some(backup.id);
        Ok(trial)
    }

    /// Before a test plan (a trial or a run without mods) is staged for `profile_id`: the
    /// profile's own plan is staged again after the run. The mark on disk keeps that across a
    /// launcher restart (D68). Set first, so a staging that fails half way is put back too.
    fn mark_test_plan(&self, profile_id: &str, label: Option<&str>) -> QmResult<()> {
        *self
            .inner
            .restage_after_run
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(profile_id.to_string());
        // Vanilla has no mods to leave out and a restart never restages it: no mark, so
        // Health does not warn about it.
        if profile_id == VANILLA_ID {
            return Ok(());
        }
        self.paths()
            .mark_test_plan_staged(profile_id, label)
            .map_err(|m| QmError::new(ErrorKind::Io, MSG_TEST_PLAN).with_details(m))?;
        self.invalidate(Scope::Health);
        Ok(())
    }

    /// Opens this launch's entry in the profile's run log (D67).
    fn begin_run(
        &self,
        profile_id: &str,
        without_mods: bool,
        trial: Option<TrialInfo>,
    ) -> QmResult<()> {
        let paths = self.paths();
        // The save fields each mod writes go into the save stamp (Phase E D85). They come
        // from the last simulation, which is of the active profile; without one the stamp
        // names the mods only.
        let save_fields = (paths.load_settings().active_profile_id == profile_id)
            .then(|| self.inner.caches.simulation.get())
            .flatten()
            .map(|r| r.save_fields.by_mod())
            .unwrap_or_default();
        let run = paths
            .begin_run_stamped(profile_id, without_mods, trial, &save_fields)
            .map_err(|m| QmError::new(ErrorKind::Io, MSG_RUN_ID).with_details(m))?;
        *self
            .inner
            .current_run
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((profile_id.to_string(), run.run_id));
        Ok(())
    }

    /// Finishes the launch's run with what the game left (D67) and returns it. With
    /// `attached`, a game found running finishes its profile's open run. `note` explains an
    /// unknown outcome.
    fn finish_run(
        &self,
        exit_code: Option<u32>,
        note: Option<&str>,
        attached: bool,
    ) -> Option<RunEntry> {
        let current = self
            .inner
            .current_run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let from_before = current.is_none();
        let run = current.or_else(|| {
            if !attached {
                return None;
            }
            let profile = self.lock_status().profile_id.clone()?;
            let open = self.paths().open_run(&profile)?;
            Some((profile, open.run_id))
        });
        let (profile, run_id) = run?;
        let done = match self.paths().finish_run(&profile, &run_id, exit_code, note) {
            Ok(done) => done,
            Err(e) => {
                tracing::warn!("could not finish run {run_id} of {profile}: {e}");
                None
            }
        }?;
        // A trial or a run without mods from before a launcher restart: nothing in memory
        // knows to put the profile's own plan back, the run entry does.
        if from_before && profile != VANILLA_ID && left_other_plan(&done) {
            self.inner
                .restage_after_run
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert(profile);
        }
        Some(done)
    }

    /// At startup with no game running, before recovery: finishes the runs a closed
    /// launcher left open, from the reports and crashes they left (D67).
    pub(crate) fn finish_runs_left_open(&self) {
        let paths = self.paths();
        for p in paths.list_profiles() {
            paths.finish_open_runs(&p.id);
        }
    }

    /// At startup with no game running: stages the active profile's own plan again when a
    /// trial or a run without mods left its plan staged (the launcher closed before it could
    /// put it back). The test plan mark says so; for a run from before the mark, the run log
    /// and the staged run id do.
    pub(crate) fn restage_left_trial(&self) {
        let paths = self.paths();
        let active = paths.load_settings().active_profile_id;
        if active == VANILLA_ID {
            // A mark from an older build: Vanilla is never restaged.
            if let Err(e) = paths.clear_test_plan_staged(VANILLA_ID) {
                tracing::warn!("could not clear the test plan mark of {VANILLA_ID}: {e}");
            }
            return;
        }
        if paths.staged_test_plan(&active).is_some() {
            // A staging that works clears the mark; one that fails keeps it for the next try.
            match self.stage_profile(&active) {
                Ok(()) => tracing::info!("staged {active} again after a test run"),
                Err(e) => tracing::warn!("could not stage {active} again after a test run: {e}"),
            }
            return;
        }
        let Some(last) = paths.last_run(&active).filter(left_other_plan) else {
            return;
        };
        // Only while the staged folder is still the one that run used.
        let staged = paths.profiles_dir.join(&active).join("Mods");
        let still_staged = qm_core::handshake::LaunchConfig::load_from_mod_dir(&staged)
            .ok()
            .flatten()
            .and_then(|c| c.run_id)
            .is_some_and(|id| id == last.run_id);
        if !still_staged {
            return;
        }
        match self.stage_profile(&active) {
            Ok(()) => tracing::info!("staged {active} again after a test run"),
            Err(e) => tracing::warn!("could not stage {active} again after a test run: {e}"),
        }
    }

    /// After a run without mods or a trial run, stages the profile's own plan again, so the
    /// simulation and the next launch see it. A plan that still does not stage is left as it is.
    /// A test plan staged before a launcher restart is found by its mark.
    fn restage_after_run(&self) {
        let id = self
            .inner
            .restage_after_run
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .or_else(|| {
                let paths = self.paths();
                let active = paths.load_settings().active_profile_id;
                (active != VANILLA_ID && paths.staged_test_plan(&active).is_some())
                    .then_some(active)
            });
        let Some(id) = id else {
            return;
        };
        match self.stage_profile(&id) {
            Ok(()) => self.after_active_profile_change(),
            Err(e) => tracing::warn!("could not stage {id} again after a run without mods: {e}"),
        }
    }

    fn set_stage(&self, stage: GameStage) -> GameStatus {
        let status = {
            let mut g = self.lock_status();
            g.stage = stage;
            g.clone()
        };
        self.emit(status.clone());
        status
    }

    fn set_status(&self, status: GameStatus) -> GameStatus {
        *self.lock_status() = status.clone();
        self.emit(status.clone());
        status
    }

    fn emit(&self, status: GameStatus) {
        self.inner.sink.emit(QmEvent::GameStatusChanged { status });
    }
}

/// The run staged a plan other than its profile's own: a trial, or a run without mods.
fn left_other_plan(run: &RunEntry) -> bool {
    run.trial.is_some() || run.mods.is_empty()
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> QmResult<()> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(f)
        .map(|_| ())
        .map_err(|e| QmError::new(ErrorKind::Internal, format!("Failed to start {name}: {e}")))
}
