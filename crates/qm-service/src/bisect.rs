//! Bisect (D68 step 2): finds which recent change brings a crash back, one trial run at a
//! time. Each trial leaves out part of the suspects; the user starts every trial. The state
//! lives in the service only, so a launcher restart ends the search with a note.
use crate::dto::PreviewResult;
use crate::error::{ErrorKind, QmError, QmResult};
use crate::game::GameStatus;
use crate::ops::launch::{SuspectKind, TrialRequest, TrialSuspect, MSG_TRIAL_BREAKS_TAIL};
use crate::ops::patch_tools::MSG_NOT_READY;
use crate::service::Service;
use jiff::Timestamp;
use qm_core::handshake::CrashReport;
use qm_core::patch::plan::GLUE_MOD;
use qm_manager::fixes::{self, FixChange, FixEventKind, ProfileFix};
use qm_manager::runs::{problem_came_back, RunEntry, RunOutcome, TrialChanges};
use qm_manager::{LaunchMethod, VANILLA_ID};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const MSG_NO_BISECT: &str = "No search for the cause is running. Start one from Find the cause.";
const MSG_ENDED: &str = "This search for the cause has ended. Start a new one.";
const MSG_BUSY: &str = "A test is still running or waiting for your answer.";
const MSG_NO_ANSWER: &str = "No test is waiting for an answer.";
const MSG_NOTHING: &str = "There is nothing to test: the crash names no fix, patch or mod, and no fix changed since the last run without problems.";
const MSG_NOT_CRASHED: &str = "That run did not crash.";
const MSG_TWO_UNKNOWN: &str = "Two tests could not tell whether the problem happened.";
const MSG_NONE_LEFT: &str =
    "The problem came back without each of them, so none of them is the cause.";
const MSG_ALL_BREAK: &str = "Every test that is left would stop a file from compiling.";
const NOTE_RESTART: &str = "The search for the cause ended when QuarterMaster closed. Start it again if the problem is still there.";

/// Where a candidate came from, in the order the search lists them (§7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CandidateSource {
    /// What the crash report blames.
    Suspect,
    /// A fix created, changed or resumed since the last run without problems.
    RecentFix,
    /// A patch of a file the crash names.
    CrashFile,
}

/// One thing the search can leave out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BisectCandidate {
    pub suspect: TrialSuspect,
    pub source: CandidateSource,
}

/// What one trial showed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum TrialResult {
    /// The problem came back without the left-out candidates.
    Crashed,
    /// The game ran without problems without them.
    Fine,
    /// Could not tell.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BisectTrial {
    pub step: u32,
    /// Indexes into `candidates`.
    pub left_out: Vec<u32>,
    pub run_id: Option<String>,
    pub result: TrialResult,
}

/// How a search ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BisectEnd {
    Found,
    NotFound,
    CouldNotTell,
    Stopped,
}

/// The trial to launch next, or the one running or waiting for an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NextTrial {
    pub step: u32,
    pub left_out: Vec<u32>,
    /// What it leaves out, as the user reads it.
    pub label: String,
    pub changes: TrialChanges,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BisectView {
    pub id: String,
    pub profile_id: String,
    /// The crashed run the search started from.
    pub run_id: String,
    pub candidates: Vec<BisectCandidate>,
    /// Indexes of the candidates that may still be the cause.
    pub remaining: Vec<u32>,
    pub trials: Vec<BisectTrial>,
    pub next: Option<NextTrial>,
    /// About how many trials the whole search takes.
    pub expected_trials: u32,
    /// The trial in `next` was launched and its run has not finished.
    pub running: bool,
    /// The trial in `next` finished and the launcher could not tell how: ask the user.
    pub ask: bool,
    /// Splits not tried because they would stop a file from compiling.
    pub skipped: u32,
    pub end: Option<BisectEnd>,
    /// With `Found`: the index of the cause.
    pub found: Option<u32>,
    pub message: Option<String>,
    /// The backup made before the first trial: "Restore the save from before the tests".
    pub save_slot_id: Option<String>,
    pub save_backup_id: Option<String>,
}

/// The search, or a note when a launcher restart ended one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BisectStatus {
    pub bisect: Option<BisectView>,
    pub note: Option<String>,
}

/// The next trial and its preview; no preview when the search ended instead.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BisectPreview {
    pub bisect: BisectView,
    pub preview: Option<PreviewResult>,
}

/// A trial that was launched or that waits for the user's answer.
#[derive(Debug, Clone)]
struct Pending {
    step: u32,
    left_out: Vec<usize>,
    run_id: Option<String>,
}

/// The search state (§7.3). Pure: the service feeds it results.
#[derive(Debug, Clone)]
pub(crate) struct Bisect {
    id: String,
    profile_id: String,
    run_id: String,
    /// How the crashed run failed: a test is read against it.
    origin: RunOutcome,
    candidates: Vec<BisectCandidate>,
    remaining: Vec<usize>,
    /// A trial without some of `remaining` ran fine, so the cause is among them.
    confirmed: bool,
    trials: Vec<BisectTrial>,
    skipped: Vec<Vec<usize>>,
    unknowns: u32,
    running: Option<Pending>,
    ask: Option<Pending>,
    end: Option<BisectEnd>,
    message: Option<String>,
    backup: Option<(String, String)>,
    /// Bumped on every change, so a check that ran without the lock sees a newer state.
    rev: u64,
}

fn to_u32(v: &[usize]) -> Vec<u32> {
    v.iter()
        .map(|&i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect()
}

/// Trials a halving search of `n` items takes.
fn halvings(n: usize) -> u32 {
    let mut steps = 0;
    let mut left = n;
    while left > 1 {
        left = left.div_ceil(2);
        steps += 1;
    }
    steps
}

fn push_new(to: &mut Vec<String>, from: &[String]) {
    for x in from {
        if !to.contains(x) {
            to.push(x.clone());
        }
    }
}

impl Bisect {
    pub(crate) fn new(
        id: String,
        profile_id: String,
        run_id: String,
        candidates: Vec<BisectCandidate>,
    ) -> Bisect {
        let mut b = Bisect {
            id,
            profile_id,
            run_id,
            origin: RunOutcome::LuaError,
            remaining: (0..candidates.len()).collect(),
            candidates,
            confirmed: false,
            trials: Vec::new(),
            skipped: Vec::new(),
            unknowns: 0,
            running: None,
            ask: None,
            end: None,
            message: None,
            backup: None,
            rev: 0,
        };
        b.settle();
        b
    }

    /// The splits to try for `remaining`, best first: the first half, the second half, then
    /// each candidate alone. Each leaves its candidates out.
    fn options(&self) -> Vec<Vec<usize>> {
        let r = &self.remaining;
        let mut out: Vec<Vec<usize>> = Vec::new();
        match r.len() {
            0 => {}
            1 if self.confirmed => {}
            1 => out.push(r.clone()),
            n => {
                let (a, b) = r.split_at(n.div_ceil(2));
                out.push(a.to_vec());
                out.push(b.to_vec());
                for &i in r {
                    if !out.iter().any(|o| o.as_slice() == [i]) {
                        out.push(vec![i]);
                    }
                }
            }
        }
        out
    }

    fn next_split(&self) -> Option<Vec<usize>> {
        self.options()
            .into_iter()
            .find(|o| !self.skipped.contains(o))
    }

    fn step(&self) -> u32 {
        u32::try_from(self.trials.len() + 1).unwrap_or(u32::MAX)
    }

    pub(crate) fn changes_for(&self, left_out: &[usize]) -> TrialChanges {
        let mut c = TrialChanges::default();
        for s in left_out
            .iter()
            .filter_map(|&i| self.candidates.get(i))
            .map(|x| &x.suspect.changes)
        {
            push_new(&mut c.pause_fixes, &s.pause_fixes);
            push_new(&mut c.disable_patches, &s.disable_patches);
            push_new(&mut c.disable_mods, &s.disable_mods);
        }
        c
    }

    pub(crate) fn label_for(&self, left_out: &[usize]) -> String {
        let names: Vec<&str> = left_out
            .iter()
            .filter_map(|&i| self.candidates.get(i))
            .map(|c| c.suspect.label.as_str())
            .collect();
        match names.as_slice() {
            [] => String::new(),
            [one] => (*one).to_string(),
            [a, b] => format!("{a} and {b}"),
            [a, b, c] => format!("{a}, {b} and {c}"),
            [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
        }
    }

    fn is_open(&self) -> bool {
        self.end.is_none()
    }

    fn idle(&self) -> bool {
        self.running.is_none() && self.ask.is_none()
    }

    /// Records what the trial without `left_out` showed.
    pub(crate) fn record(
        &mut self,
        step: u32,
        left_out: Vec<usize>,
        run_id: Option<String>,
        result: TrialResult,
    ) {
        match result {
            TrialResult::Fine => {
                self.remaining = left_out.clone();
                self.confirmed = true;
            }
            TrialResult::Crashed => self.remaining.retain(|i| !left_out.contains(i)),
            TrialResult::Unknown => self.unknowns += 1,
        }
        self.trials.push(BisectTrial {
            step,
            left_out: to_u32(&left_out),
            run_id,
            result,
        });
        self.rev += 1;
        self.settle();
    }

    /// A split that would stop a file from compiling is not tried.
    pub(crate) fn skip(&mut self, left_out: Vec<usize>) {
        if !self.skipped.contains(&left_out) {
            self.skipped.push(left_out);
        }
        self.rev += 1;
        self.settle();
    }

    fn finish(&mut self, end: BisectEnd, message: Option<&str>) {
        self.end = Some(end);
        self.message = message.map(str::to_string);
        self.running = None;
        self.ask = None;
        self.rev += 1;
    }

    /// Ends the search when the result is known.
    fn settle(&mut self) {
        if !self.is_open() || !self.idle() {
            return;
        }
        if self.unknowns >= 2 {
            self.finish(BisectEnd::CouldNotTell, Some(MSG_TWO_UNKNOWN));
        } else if self.remaining.is_empty() {
            self.finish(BisectEnd::NotFound, Some(MSG_NONE_LEFT));
        } else if self.remaining.len() == 1 && self.confirmed {
            self.finish(BisectEnd::Found, None);
        } else if self.next_split().is_none() {
            self.finish(BisectEnd::CouldNotTell, Some(MSG_ALL_BREAK));
        }
    }

    fn next_trial(&self, step: u32, left_out: &[usize]) -> NextTrial {
        NextTrial {
            step,
            left_out: to_u32(left_out),
            label: self.label_for(left_out),
            changes: self.changes_for(left_out),
        }
    }

    pub(crate) fn view(&self) -> BisectView {
        let pending = self.running.as_ref().or(self.ask.as_ref());
        let next = match pending {
            Some(p) => Some(self.next_trial(p.step, &p.left_out)),
            None if self.is_open() => self.next_split().map(|s| self.next_trial(self.step(), &s)),
            None => None,
        };
        let left = if !self.is_open() {
            0
        } else if self.remaining.len() == 1 {
            1
        } else {
            halvings(self.remaining.len())
        };
        let found = match (self.end, self.remaining.as_slice()) {
            (Some(BisectEnd::Found), [i]) => u32::try_from(*i).ok(),
            _ => None,
        };
        BisectView {
            id: self.id.clone(),
            profile_id: self.profile_id.clone(),
            run_id: self.run_id.clone(),
            candidates: self.candidates.clone(),
            remaining: to_u32(&self.remaining),
            trials: self.trials.clone(),
            next,
            expected_trials: u32::try_from(self.trials.len())
                .unwrap_or(u32::MAX)
                .saturating_add(left),
            running: self.running.is_some(),
            ask: self.ask.is_some(),
            skipped: u32::try_from(self.skipped.len()).unwrap_or(u32::MAX),
            end: self.end,
            found,
            message: self.message.clone(),
            save_slot_id: self.backup.as_ref().map(|b| b.0.clone()),
            save_backup_id: self.backup.as_ref().map(|b| b.1.clone()),
        }
    }
}

fn crashed(o: RunOutcome) -> bool {
    matches!(
        o,
        RunOutcome::LuaError | RunOutcome::LoadError | RunOutcome::NativeCrash
    )
}

fn parse_time(s: &str) -> Option<Timestamp> {
    s.parse::<Timestamp>().ok()
}

/// When `f` was last created, changed or resumed.
fn changed_at(f: &ProfileFix) -> Option<Timestamp> {
    let events = f.provenance.history.iter().filter(|e| {
        matches!(
            e.what,
            FixEventKind::Created
                | FixEventKind::Pinned
                | FixEventKind::Rebound
                | FixEventKind::Accepted
                | FixEventKind::Resumed
        )
    });
    std::iter::once(f.provenance.created_at.as_str())
        .chain(events.map(|e| e.at.as_str()))
        .filter_map(parse_time)
        .max()
}

/// `name` as a game file name: no leading `@` or `./`.
fn file_key(name: &str) -> &str {
    name.trim_start_matches('@').trim_start_matches("./")
}

/// The candidates in §7.3 order: the crash's suspect, the fixes changed since the last clean
/// run (newest first), then the patches of the files the crash names. `cleared` lists what a
/// crashed trial since already left out; those are not the cause.
pub(crate) struct CandidateInput<'a> {
    pub suspect: Option<TrialSuspect>,
    pub fixes: Vec<&'a ProfileFix>,
    pub last_clean: Option<&'a RunEntry>,
    pub file_patches: Vec<TrialSuspect>,
    pub cleared: &'a TrialChanges,
}

fn is_cleared(s: &TrialSuspect, cleared: &TrialChanges) -> bool {
    let list = match s.kind {
        SuspectKind::Fix => &cleared.pause_fixes,
        SuspectKind::Patch => &cleared.disable_patches,
        SuspectKind::Mod => &cleared.disable_mods,
    };
    list.iter().any(|x| x.eq_ignore_ascii_case(&s.id))
}

pub(crate) fn fix_suspect(f: &ProfileFix) -> TrialSuspect {
    TrialSuspect {
        kind: SuspectKind::Fix,
        id: f.id.clone(),
        label: format!("the fix \"{}\"", f.label),
        name: f.label.clone(),
        changes: TrialChanges {
            pause_fixes: vec![f.id.clone()],
            ..TrialChanges::default()
        },
    }
}

pub(crate) fn candidates(input: CandidateInput<'_>) -> Vec<BisectCandidate> {
    let mut out: Vec<BisectCandidate> = Vec::new();
    let mut add = |s: TrialSuspect, source: CandidateSource| {
        let seen = out
            .iter()
            .any(|c| c.suspect.kind == s.kind && c.suspect.id.eq_ignore_ascii_case(&s.id));
        if !seen && !is_cleared(&s, input.cleared) {
            out.push(BisectCandidate { suspect: s, source });
        }
    };
    if let Some(s) = input.suspect {
        add(s, CandidateSource::Suspect);
    }
    let since = input.last_clean.and_then(|r| parse_time(&r.started_at));
    let mut recent: Vec<(Option<Timestamp>, &ProfileFix)> = input
        .fixes
        .iter()
        // Pausing a mod-order record changes nothing the game runs.
        .filter(|f| !f.paused && !matches!(f.change, FixChange::ModOrder { .. }))
        .map(|f| (changed_at(f), *f))
        .filter(|(at, f)| match (input.last_clean, since) {
            (None, _) => true,
            (Some(clean), Some(since)) => {
                !clean.active_fixes.contains(&f.id) || at.is_some_and(|at| at > since)
            }
            (Some(clean), None) => !clean.active_fixes.contains(&f.id),
        })
        .collect();
    recent.sort_by_key(|x| std::cmp::Reverse(x.0));
    for (_, f) in recent {
        add(fix_suspect(f), CandidateSource::RecentFix);
    }
    for s in input.file_patches {
        add(s, CandidateSource::CrashFile);
    }
    out
}

impl Service {
    fn lock_bisect(&self) -> std::sync::MutexGuard<'_, Option<Bisect>> {
        self.inner.bisect.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Starts a search for the cause of run `run_id` of the active profile (D68 step 2). It
    /// replaces any earlier search. Needs the simulation when the crash names a file.
    pub fn bisect_start(&self, run_id: &str) -> QmResult<BisectView> {
        self.ensure_not_running()?;
        let profile = self.active_profile()?;
        if profile.id == VANILLA_ID {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "The Vanilla profile has no mods to test.",
            ));
        }
        let paths = self.paths();
        let runs = paths.list_runs(&profile.id);
        let at = runs
            .iter()
            .position(|r| r.run_id == run_id)
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("Run '{run_id}' of this profile was not found."),
                )
            })?;
        let run = runs.get(at).cloned().unwrap_or_default();
        if !crashed(run.outcome) {
            return Err(QmError::new(ErrorKind::InvalidInput, MSG_NOT_CRASHED));
        }
        let last_clean = runs
            .get(..at)
            .unwrap_or_default()
            .iter()
            .rev()
            .find(|r| r.outcome == RunOutcome::Clean && r.trial.is_none());
        // What a crashed trial since left out is not the cause.
        let mut cleared = TrialChanges::default();
        for t in runs
            .get(at..)
            .unwrap_or_default()
            .iter()
            .filter(|r| problem_came_back(run.outcome, r) == Some(true))
            .filter_map(|r| r.trial.as_ref())
        {
            push_new(&mut cleared.pause_fixes, &t.pause_fixes);
            push_new(&mut cleared.disable_patches, &t.disable_patches);
            push_new(&mut cleared.disable_mods, &t.disable_mods);
        }
        let all_crashes = self.crashes_list();
        let crashes: Vec<&CrashReport> = run
            .crashes
            .iter()
            .filter_map(|id| all_crashes.iter().find(|c| &c.id == id))
            .collect();
        let suspect = match run.crashes.first() {
            Some(id) => match self.crash_suspect(id) {
                Ok(s) => s,
                Err(e) if e.kind == ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            },
            None => None,
        };
        let file_patches = self.crash_file_patches(&profile, &crashes)?;
        let list = candidates(CandidateInput {
            suspect,
            fixes: fixes::fixes(&profile).collect(),
            last_clean,
            file_patches,
            cleared: &cleared,
        });
        if list.is_empty() {
            return Err(QmError::new(ErrorKind::NotFound, MSG_NOTHING));
        }
        let id: String = qm_manager::runs::new_run_id().chars().take(8).collect();
        let mut b = Bisect::new(id, profile.id.clone(), run.run_id.clone(), list);
        b.origin = run.outcome;
        let view = b.view();
        let old = self.lock_bisect().replace(b);
        // A search this one replaces has ended.
        if let Some(old) = old {
            self.note_search_end(&old, true);
        }
        Ok(view)
    }

    /// The enabled patches of the files the crashes name: the ones the crash snippet
    /// attributes first, then the rest in table order. Adjusted copies are left to their fix.
    fn crash_file_patches(
        &self,
        profile: &qm_manager::ModProfile,
        crashes: &[&CrashReport],
    ) -> QmResult<Vec<TrialSuspect>> {
        let files: Vec<&str> = crashes
            .iter()
            .filter_map(|c| c.crashing_file.as_deref())
            .map(file_key)
            .filter(|f| !f.is_empty())
            .collect();
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let table = self
            .inner
            .caches
            .patch_table
            .get()
            .ok_or_else(|| QmError::new(ErrorKind::Conflict, MSG_NOT_READY))?;
        let names: HashMap<String, String> = self
            .paths()
            .list_installed_mods()
            .into_iter()
            .map(|m| (m.manifest.id.to_ascii_lowercase(), m.manifest.name))
            .collect();
        let mod_name = |id: &str| {
            names
                .get(&id.to_ascii_lowercase())
                .filter(|n| !n.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| id.to_string())
        };
        let enabled = |m: &str| {
            profile
                .enabled_mods
                .iter()
                .any(|e| e.eq_ignore_ascii_case(m))
        };
        let mut eligible: Vec<(String, String, String)> = Vec::new();
        for (p, _, _) in &table.patches {
            let (Some(id), Some(m)) = (p.id(), p.mod_id()) else {
                continue;
            };
            if m == GLUE_MOD
                || !enabled(m)
                || profile.disabled_patches.iter().any(|d| d == id)
                || !files.iter().any(|f| p.can_apply_to_target(f))
            {
                continue;
            }
            let name = p.name().unwrap_or(id).to_string();
            eligible.push((id.to_string(), m.to_string(), name));
        }
        let snippet: Vec<&str> = crashes
            .iter()
            .flat_map(|c| c.code_snippet.iter())
            .filter_map(|l| l.attributed_patch_id.as_deref())
            .collect();
        eligible
            .sort_by_key(|(id, _, _)| snippet.iter().position(|s| s == id).unwrap_or(usize::MAX));
        Ok(eligible
            .into_iter()
            .map(|(id, m, name)| TrialSuspect {
                kind: SuspectKind::Patch,
                label: format!("the patch \"{name}\" of {}", mod_name(&m)),
                name,
                changes: TrialChanges {
                    disable_patches: vec![id.clone()],
                    ..TrialChanges::default()
                },
                id,
            })
            .collect())
    }

    /// The running search; after a launcher restart, a note that the last one ended.
    pub fn bisect_get(&self) -> BisectStatus {
        if let Some(b) = self.lock_bisect().as_ref() {
            return BisectStatus {
                bisect: Some(b.view()),
                note: None,
            };
        }
        let profile = self.paths().load_settings().active_profile_id;
        // Only for a search that was still going when the launcher closed.
        let restarted = self
            .paths()
            .last_run(&profile)
            .and_then(|r| r.trial)
            .is_some_and(|t| t.bisect_id.is_some() && !t.search_ended);
        BisectStatus {
            bisect: None,
            note: restarted.then(|| NOTE_RESTART.to_string()),
        }
    }

    /// Picks the next trial and previews it. A split that would stop a file from compiling
    /// is skipped and the next one checked; when none is left the search ends.
    pub fn bisect_preview(&self) -> QmResult<BisectPreview> {
        loop {
            let (rev, split, changes) = {
                let guard = self.lock_bisect();
                let b = guard
                    .as_ref()
                    .ok_or_else(|| QmError::new(ErrorKind::NotFound, MSG_NO_BISECT))?;
                let split = match (b.is_open(), b.running.as_ref().or(b.ask.as_ref())) {
                    (false, _) => None,
                    (true, Some(p)) => Some(p.left_out.clone()),
                    (true, None) => b.next_split(),
                };
                let Some(split) = split else {
                    return Ok(BisectPreview {
                        bisect: b.view(),
                        preview: None,
                    });
                };
                (b.rev, split.clone(), b.changes_for(&split))
            };
            // Without the lock: the simulation takes a while.
            let check = self.trial_check(&changes)?;
            let mut guard = self.lock_bisect();
            let b = guard
                .as_mut()
                .ok_or_else(|| QmError::new(ErrorKind::NotFound, MSG_NO_BISECT))?;
            if b.rev != rev {
                continue;
            }
            if check.impact.breaks_file && b.idle() {
                b.skip(split);
                self.note_search_end(b, false);
                continue;
            }
            return Ok(BisectPreview {
                bisect: b.view(),
                preview: Some(check),
            });
        }
    }

    /// Launches the next trial of the search on the active profile.
    pub fn bisect_launch(&self, method: Option<LaunchMethod>) -> QmResult<GameStatus> {
        self.ensure_not_running()?;
        let active = self.paths().load_settings().active_profile_id;
        let (id, step, split, req) = {
            let mut guard = self.lock_bisect();
            let b = guard
                .as_mut()
                .ok_or_else(|| QmError::new(ErrorKind::NotFound, MSG_NO_BISECT))?;
            if !b.is_open() {
                return Err(QmError::new(ErrorKind::InvalidInput, MSG_ENDED));
            }
            if !b.idle() {
                return Err(QmError::new(ErrorKind::Conflict, MSG_BUSY));
            }
            if b.profile_id != active {
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    "The search is for another profile. Switch back to it to go on.",
                ));
            }
            let split = b
                .next_split()
                .ok_or_else(|| QmError::new(ErrorKind::InvalidInput, MSG_ENDED))?;
            let step = b.step();
            let req = TrialRequest {
                changes: b.changes_for(&split),
                label: b.label_for(&split),
                method,
                bisect_id: Some(b.id.clone()),
                step: Some(step),
            };
            // Set before the launch: a run that ends at once still finds it.
            b.running = Some(Pending {
                step,
                left_out: split.clone(),
                run_id: None,
            });
            b.rev += 1;
            (b.id.clone(), step, split, req)
        };
        let result = self.game_launch_trial(req);
        if let Err(e) = &result {
            let mut guard = self.lock_bisect();
            if let Some(b) = guard.as_mut().filter(|b| b.id == id) {
                if b.running.as_ref().is_some_and(|p| p.step == step) {
                    b.running = None;
                    b.rev += 1;
                    if e.kind == ErrorKind::Conflict && e.message.ends_with(MSG_TRIAL_BREAKS_TAIL) {
                        b.skip(split);
                        self.note_search_end(b, false);
                    }
                }
            }
        }
        result
    }

    /// The user's answer for a trial the launcher could not read.
    pub fn bisect_answer(&self, result: TrialResult) -> QmResult<BisectView> {
        let mut guard = self.lock_bisect();
        let b = guard
            .as_mut()
            .ok_or_else(|| QmError::new(ErrorKind::NotFound, MSG_NO_BISECT))?;
        let p = b
            .ask
            .take()
            .ok_or_else(|| QmError::new(ErrorKind::InvalidInput, MSG_NO_ANSWER))?;
        b.record(p.step, p.left_out, p.run_id, result);
        self.note_search_end(b, false);
        Ok(b.view())
    }

    /// Ends the search; nothing in the profile changes.
    pub fn bisect_stop(&self) -> QmResult<BisectView> {
        let mut guard = self.lock_bisect();
        let b = guard
            .as_mut()
            .ok_or_else(|| QmError::new(ErrorKind::NotFound, MSG_NO_BISECT))?;
        if b.is_open() {
            b.finish(BisectEnd::Stopped, None);
        }
        self.note_search_end(b, false);
        Ok(b.view())
    }

    /// Reads a finished trial run of the search: a crash or the clean-exit mark is read at
    /// once, a launch that failed counts as unknown, anything else asks the user.
    pub(crate) fn bisect_after_run(&self, run: &RunEntry) {
        let Some(t) = &run.trial else {
            return;
        };
        let mut guard = self.lock_bisect();
        let Some(b) = guard
            .as_mut()
            .filter(|b| b.is_open() && t.bisect_id.as_deref() == Some(b.id.as_str()))
        else {
            return;
        };
        let Some(mut p) = b.running.take_if(|p| p.step == t.step) else {
            return;
        };
        if b.backup.is_none() {
            b.backup = t.save_slot_id.clone().zip(t.save_backup_id.clone());
        }
        p.run_id = Some(run.run_id.clone());
        b.rev += 1;
        let result = match problem_came_back(b.origin, run) {
            Some(true) => Some(TrialResult::Crashed),
            Some(false) => Some(TrialResult::Fine),
            None if run.outcome == RunOutcome::Unknown && run.message.is_some() => {
                Some(TrialResult::Unknown)
            }
            None => None,
        };
        match result {
            Some(r) => b.record(p.step, p.left_out, p.run_id, r),
            None => b.ask = Some(p),
        }
        self.note_search_end(b, false);
    }

    /// Records in the run log that search `b` ended (or, with `replaced`, that a new search
    /// replaced it), so a restart does not say it was cut short.
    fn note_search_end(&self, b: &Bisect, replaced: bool) {
        if (replaced || !b.is_open()) && !b.trials.is_empty() {
            self.paths().mark_search_ended(&b.profile_id, &b.id);
        }
    }
}

#[cfg(test)]
#[path = "bisect_tests.rs"]
mod tests;
