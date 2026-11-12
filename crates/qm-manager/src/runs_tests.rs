use super::*;
use std::collections::HashMap;

const START: &str = "2026-11-09T10:00:00Z";

fn at(s: &str) -> Timestamp {
    s.parse().unwrap()
}

fn entry(run_id: &str) -> RunEntry {
    RunEntry {
        run_id: run_id.into(),
        started_at: START.into(),
        ..RunEntry::default()
    }
}

fn report(run_id: Option<&str>, profile: Option<&str>, timestamp: &str) -> PatchReportSummary {
    PatchReportSummary {
        run_id: run_id.map(str::to_string),
        profile_id: profile.map(str::to_string),
        timestamp: timestamp.into(),
        ..PatchReportSummary::default()
    }
}

fn crash(id: &str, run_id: Option<&str>, profile: Option<&str>, timestamp: &str) -> CrashReport {
    CrashReport {
        id: id.into(),
        timestamp: timestamp.into(),
        crash_kind: "lua_runtime".into(),
        error_message: format!("{id} failed"),
        traceback: String::new(),
        crashing_file: None,
        crashing_line: None,
        suspected_mod_id: None,
        suspected_patch_id: None,
        suspected_patch_name: None,
        code_snippet: Vec::new(),
        active_profile_id: profile.map(str::to_string),
        active_mods: HashMap::new(),
        run_id: run_id.map(str::to_string),
        suspected_fix_ids: Vec::new(),
        suspected_adjusted_copy: false,
    }
}

fn finish(
    e: &mut RunEntry,
    reports: &[PatchReportSummary],
    crashes: &[CrashReport],
    exit: Option<u32>,
) {
    finish_entry(
        e,
        "p",
        reports,
        crashes,
        exit,
        at("2026-11-09T10:30:00Z"),
        None,
    );
}

#[test]
fn a_clean_exit_mark_gives_clean() {
    let mut e = entry("r1");
    let r = PatchReportSummary {
        clean_exit: true,
        ..report(Some("r1"), Some("p"), START)
    };
    finish(&mut e, &[r], &[], Some(0));
    assert_eq!(e.outcome, RunOutcome::Clean);
    assert_eq!(e.exit_code, Some(0));
    assert!(e.report.as_ref().unwrap().clean_exit);
    assert_eq!(e.ended_at.as_deref(), Some("2026-11-09T10:30:00Z"));
    assert_eq!(e.message, None);
}

#[test]
fn a_crash_report_with_the_run_id_gives_lua_error() {
    let mut e = entry("r1");
    // The time does not matter when the run id matches.
    let c = crash("c1", Some("r1"), Some("p"), "2026-11-09T12:00:00Z");
    let other = crash("c2", Some("r0"), Some("p"), "2026-11-09T10:10:00Z");
    finish(
        &mut e,
        &[report(Some("r1"), Some("p"), START)],
        &[c, other],
        Some(0),
    );
    assert_eq!(e.outcome, RunOutcome::LuaError);
    assert_eq!(e.crashes, vec!["c1".to_string()]);
    assert_eq!(e.message.as_deref(), Some("c1 failed"));
}

#[test]
fn a_non_zero_exit_without_a_report_is_a_native_crash() {
    let mut e = entry("r1");
    finish(&mut e, &[], &[], Some(0xC000_0005));
    assert_eq!(e.outcome, RunOutcome::NativeCrash);
    assert_eq!(e.report, None);
    // A report of another run is not this run's.
    let mut e = entry("r1");
    let old = PatchReportSummary {
        clean_exit: true,
        ..report(Some("r0"), Some("p"), START)
    };
    finish(&mut e, &[old], &[], Some(1));
    assert_eq!(e.outcome, RunOutcome::NativeCrash);
    assert_eq!(e.report, None);
}

#[test]
fn exit_zero_without_the_mark_is_closed_and_no_exit_code_is_unknown() {
    let mut e = entry("r1");
    finish(
        &mut e,
        &[report(Some("r1"), Some("p"), START)],
        &[],
        Some(0),
    );
    assert_eq!(e.outcome, RunOutcome::Closed);
    let mut e = entry("r1");
    finish(&mut e, &[], &[], Some(0));
    assert_eq!(e.outcome, RunOutcome::Closed);
    let mut e = entry("r1");
    finish_entry(&mut e, "p", &[], &[], None, at(START), Some("lost"));
    assert_eq!(e.outcome, RunOutcome::Unknown);
    assert_eq!(e.message.as_deref(), Some("lost"));
}

#[test]
fn load_problems_give_load_error() {
    let mut e = entry("r1");
    let r = PatchReportSummary {
        clean_exit: true,
        load_errors: vec![LoadError {
            mod_id: "gcore".into(),
            folder: "gcore".into(),
            message: "bad mod.json".into(),
            skipped_dependents: Vec::new(),
        }],
        ..report(Some("r1"), Some("p"), START)
    };
    finish(&mut e, &[r], &[], Some(0));
    assert_eq!(e.outcome, RunOutcome::LoadError);
    assert_eq!(e.message.as_deref(), Some("gcore: bad mod.json"));

    let mut e = entry("r1");
    let r = PatchReportSummary {
        plan_error: Some("load-order.json is not valid".into()),
        ..report(Some("r1"), Some("p"), START)
    };
    finish(&mut e, &[r], &[], Some(0));
    assert_eq!(e.outcome, RunOutcome::LoadError);
    assert_eq!(e.message.as_deref(), Some("load-order.json is not valid"));

    // A caught DLL panic alone is a load problem, not a Lua error.
    let mut e = entry("r1");
    let panic = CrashReport {
        crash_kind: "rust_panic".into(),
        ..crash("c1", Some("r1"), Some("p"), START)
    };
    finish(&mut e, &[], &[panic], Some(0));
    assert_eq!(e.outcome, RunOutcome::LoadError);
    assert_eq!(e.crashes, vec!["c1".to_string()]);
}

#[test]
fn an_old_dll_report_links_by_time_and_profile() {
    let in_run = "2026-11-09T10:00:02+00:00[UTC]";
    let mut e = entry("r1");
    let r = PatchReportSummary {
        clean_exit: true,
        ..report(None, Some("p"), in_run)
    };
    finish(&mut e, &[r], &[], Some(0));
    assert_eq!(e.outcome, RunOutcome::Clean, "same profile, within the run");

    for (profile, time) in [
        (Some("q"), in_run),
        (Some("p"), "2026-11-09T09:00:00+00:00[UTC]"),
        (Some("p"), "2026-11-09T11:00:00+00:00[UTC]"),
        (Some("p"), "not a time"),
    ] {
        let mut e = entry("r1");
        let r = PatchReportSummary {
            clean_exit: true,
            ..report(None, profile, time)
        };
        finish(&mut e, &[r], &[], Some(0));
        assert_eq!(e.report, None, "{profile:?} {time}");
        assert_eq!(e.outcome, RunOutcome::Closed);
    }
}

#[test]
fn an_old_dll_crash_links_by_time_and_profile() {
    let mut e = entry("r1");
    let crashes = [
        crash("in", None, Some("p"), "2026-11-09T10:05:00+00:00[UTC]"),
        crash(
            "other_profile",
            None,
            Some("q"),
            "2026-11-09T10:05:00+00:00[UTC]",
        ),
        crash("before", None, Some("p"), "2026-11-08T10:05:00+00:00[UTC]"),
        crash("after", None, Some("p"), "2026-11-09T10:45:00+00:00[UTC]"),
    ];
    finish(&mut e, &[], &crashes, Some(1));
    assert_eq!(e.crashes, vec!["in".to_string()]);
    assert_eq!(e.outcome, RunOutcome::LuaError);
}

#[test]
fn the_summary_reads_leniently() {
    let v = serde_json::json!({
        "qm_version": "1.0.0",
        "timestamp": START,
        "profile_id": "p",
        "run_id": "r1",
        "clean_exit": true,
        "total_patches_applied": 3,
        "total_failures": -1,
        "plan_warnings": ["a", 2, "b"],
        "load_errors": "not a list",
        "records": [{"anything": true}],
    });
    let s = PatchReportSummary::from_json(&v);
    assert_eq!(s.run_id.as_deref(), Some("r1"));
    assert!(s.clean_exit);
    assert_eq!(s.total_patches_applied, 3);
    assert_eq!(s.total_failures, 0);
    assert_eq!(s.plan_warnings, vec!["a".to_string(), "b".to_string()]);
    assert!(s.load_errors.is_empty());
}

#[test]
fn an_unknown_outcome_from_a_newer_launcher_reads_as_unknown() {
    let e: RunEntry =
        serde_json::from_str(r#"{"run_id": "r", "started_at": "x", "outcome": "frozen"}"#).unwrap();
    assert_eq!(e.outcome, RunOutcome::Unknown);
    let json = serde_json::to_string(&RunOutcome::NativeCrash).unwrap();
    assert_eq!(json, "\"native_crash\"");
}

fn staged(paths: &QuarterMasterPaths, profile: &str) -> PathBuf {
    let mods = paths.profiles_dir.join(profile).join("Mods");
    // A game folder inside the test's own folder: without one the settings find the real
    // install, and its crash reports would be listed with the test's.
    let mut settings = paths.load_settings();
    settings.game_exe_path = Some(paths.profiles_dir.with_file_name("game").join("mnb.exe"));
    paths.save_settings(&settings).unwrap();
    LaunchConfig {
        profile_id: Some(profile.into()),
        ..LaunchConfig::default()
    }
    .save_to_mod_dir(&mods)
    .unwrap();
    mods
}

#[test]
fn begin_writes_the_run_id_and_finish_reads_the_report() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    let mods = staged(&paths, "p");
    let run = paths.begin_run("p", false).unwrap();
    assert!(run.is_open());
    assert_eq!(run.outcome, RunOutcome::Unknown);
    let cfg = LaunchConfig::load_from_mod_dir(&mods).unwrap().unwrap();
    assert_eq!(cfg.run_id.as_deref(), Some(run.run_id.as_str()));
    assert_eq!(paths.open_run("p").unwrap().run_id, run.run_id);

    let report = serde_json::json!({"profile_id": "p", "run_id": run.run_id, "clean_exit": true});
    std::fs::write(
        mods.join("qm").join("patch-report.json"),
        report.to_string(),
    )
    .unwrap();
    let done = paths
        .finish_run("p", &run.run_id, Some(0), None)
        .unwrap()
        .unwrap();
    assert_eq!(done.outcome, RunOutcome::Clean);
    assert_eq!(paths.last_run("p").unwrap(), done);
    assert_eq!(paths.open_run("p"), None);
    assert_eq!(
        paths.finish_run("p", "missing", Some(0), None).unwrap(),
        None
    );
}

#[test]
fn the_log_keeps_the_newest_twenty_runs() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    staged(&paths, "p");
    let mut ids = Vec::new();
    for _ in 0..(MAX_RUNS + 3) {
        let run = paths.begin_run("p", false).unwrap();
        paths.finish_run("p", &run.run_id, Some(0), None).unwrap();
        ids.push(run.run_id);
    }
    let runs = paths.list_runs("p");
    assert_eq!(runs.len(), MAX_RUNS);
    let kept: Vec<String> = runs.into_iter().map(|r| r.run_id).collect();
    assert_eq!(
        kept,
        ids[3..].to_vec(),
        "oldest first, the oldest three dropped"
    );
}

#[test]
fn a_run_left_open_is_finished_by_the_next_launch() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    staged(&paths, "p");
    let first = paths.begin_run("p", false).unwrap();
    let second = paths.begin_run("p", false).unwrap();
    let runs = paths.list_runs("p");
    assert_eq!(runs.len(), 2);
    let old = runs.iter().find(|r| r.run_id == first.run_id).unwrap();
    assert!(!old.is_open());
    assert_eq!(old.outcome, RunOutcome::Unknown);
    assert_eq!(old.message.as_deref(), Some(NOTE_LEFT_OPEN));
    assert_eq!(paths.open_run("p").unwrap().run_id, second.run_id);
}

#[test]
fn a_broken_log_is_kept_aside_and_started_again() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    staged(&paths, "p");
    let file = paths.runs_file("p");
    std::fs::write(&file, "{ not json").unwrap();
    assert!(paths.list_runs("p").is_empty());
    paths.begin_run("p", false).unwrap();
    assert_eq!(paths.list_runs("p").len(), 1);
    assert_eq!(
        std::fs::read_to_string(file.with_extension("json.bad")).unwrap(),
        "{ not json"
    );
    // A second broken log does not overwrite the first copy.
    std::fs::write(&file, "[ also bad").unwrap();
    paths.begin_run("p", false).unwrap();
    assert_eq!(paths.list_runs("p").len(), 1);
    assert_eq!(
        std::fs::read_to_string(file.with_extension("json.bad")).unwrap(),
        "{ not json"
    );
    let kept: Vec<String> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("runs.json.") && n.ends_with(".bad") && n != "runs.json.bad")
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    let copy = file.parent().unwrap().join(&kept[0]);
    assert_eq!(std::fs::read_to_string(copy).unwrap(), "[ also bad");
}

#[test]
fn a_log_that_cannot_be_read_is_never_written_over() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    let mods = staged(&paths, "p");
    // A folder where the file should be: reading fails, but not with NotFound.
    let file = paths.runs_file("p");
    std::fs::create_dir_all(file.join("keep")).unwrap();
    let run = paths.begin_run("p", false).unwrap();
    // The run id still reaches the game; the log is left alone.
    let cfg = LaunchConfig::load_from_mod_dir(&mods).unwrap().unwrap();
    assert_eq!(cfg.run_id.as_deref(), Some(run.run_id.as_str()));
    assert!(file.join("keep").is_dir());
    assert!(paths.finish_run("p", &run.run_id, Some(0), None).is_err());
    paths.finish_open_runs("p");
    assert!(!paths.mark_search_ended("p", "b1"));
    assert!(file.join("keep").is_dir());
    assert!(!file.with_extension("json.bad").exists());
}

#[test]
fn an_open_run_is_finished_from_what_it_left_and_keeps_its_crashes() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    let mods = staged(&paths, "p");
    let run = paths.begin_run("p", false).unwrap();
    // The launcher closed while the game ran; the game left its report and a crash.
    let report = serde_json::json!({"profile_id": "p", "run_id": run.run_id, "clean_exit": false});
    std::fs::write(
        mods.join("qm").join("patch-report.json"),
        report.to_string(),
    )
    .unwrap();
    let crashes = mods.join("qm").join("log").join("crashes");
    std::fs::create_dir_all(&crashes).unwrap();
    let c = crash("crash:1", Some(&run.run_id), Some("p"), START);
    std::fs::write(
        crashes.join("crash-1.json"),
        serde_json::to_string(&c).unwrap(),
    )
    .unwrap();

    paths.finish_open_runs("p");
    let done = paths.last_run("p").unwrap();
    assert!(!done.is_open());
    assert_eq!(done.outcome, RunOutcome::LuaError);
    assert_eq!(done.crashes, vec!["crash:1".to_string()]);
    assert!(done.report.is_some());
    // Staging removes the staged crash log; the launcher's copy keeps the id resolving.
    std::fs::remove_dir_all(mods.join("qm")).unwrap();
    let kept = paths.list_crash_reports_for("p");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].id, "crash:1");
    assert_eq!(kept[0].run_id.as_deref(), Some(run.run_id.as_str()));
    assert!(paths
        .logs_dir
        .join("crashes")
        .join("crash_1.json")
        .is_file());
}

#[test]
fn a_test_is_read_against_the_crash_it_tests() {
    let load_error = LoadError {
        mod_id: "broken".into(),
        folder: "broken".into(),
        message: "patch file does not parse".into(),
        skipped_dependents: Vec::new(),
    };
    let ran = |outcome, report: Option<PatchReportSummary>, crashes: &[&str]| RunEntry {
        outcome,
        report,
        crashes: crashes.iter().map(|c| c.to_string()).collect(),
        ..entry("t")
    };
    let clean_with_load_error = PatchReportSummary {
        clean_exit: true,
        load_errors: vec![load_error],
        ..PatchReportSummary::default()
    };
    // A mod that never loads, and the game quit cleanly: the Lua crash is gone.
    let standing = ran(
        RunOutcome::LoadError,
        Some(clean_with_load_error.clone()),
        &[],
    );
    assert_eq!(
        problem_came_back(RunOutcome::LuaError, &standing),
        Some(false)
    );
    assert_eq!(
        problem_came_back(RunOutcome::NativeCrash, &standing),
        Some(false)
    );
    // For a load error, that is the problem coming back.
    assert_eq!(
        problem_came_back(RunOutcome::LoadError, &standing),
        Some(true)
    );
    // Closed from outside, a bad plan or a DLL panic cannot tell.
    let closed = PatchReportSummary {
        clean_exit: false,
        ..clean_with_load_error.clone()
    };
    let bad_plan = PatchReportSummary {
        plan_error: Some("bad".into()),
        ..clean_with_load_error.clone()
    };
    for r in [
        ran(RunOutcome::LoadError, Some(closed), &[]),
        ran(RunOutcome::LoadError, Some(bad_plan), &[]),
        ran(
            RunOutcome::LoadError,
            Some(clean_with_load_error),
            &["panic"],
        ),
        ran(RunOutcome::Closed, None, &[]),
        ran(RunOutcome::Unknown, None, &[]),
    ] {
        assert_eq!(problem_came_back(RunOutcome::LuaError, &r), None, "{r:?}");
    }
    for o in [RunOutcome::LuaError, RunOutcome::NativeCrash] {
        assert_eq!(
            problem_came_back(RunOutcome::LuaError, &ran(o, None, &[])),
            Some(true)
        );
    }
    assert_eq!(
        problem_came_back(RunOutcome::LuaError, &ran(RunOutcome::Clean, None, &[])),
        Some(false)
    );
    // A load error search: a run whose mods loaded did not bring it back.
    let loaded = PatchReportSummary::default();
    assert_eq!(
        problem_came_back(
            RunOutcome::LoadError,
            &ran(RunOutcome::LuaError, Some(loaded), &["c"])
        ),
        Some(false)
    );
    assert_eq!(
        problem_came_back(
            RunOutcome::LoadError,
            &ran(RunOutcome::NativeCrash, None, &[])
        ),
        None
    );
}

#[test]
fn the_end_of_a_search_is_kept_in_its_trial_runs() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    staged(&paths, "p");
    let trial = |b: &str| TrialInfo {
        bisect_id: Some(b.into()),
        step: 1,
        ..TrialInfo::default()
    };
    paths.begin_trial_run("p", trial("b1")).unwrap();
    paths.begin_trial_run("p", trial("b2")).unwrap();
    assert!(paths.mark_search_ended("p", "b1"));
    assert!(!paths.mark_search_ended("p", "b1"), "already marked");
    let runs = paths.list_runs("p");
    let ended: Vec<bool> = runs
        .iter()
        .map(|r| r.trial.as_ref().unwrap().search_ended)
        .collect();
    assert_eq!(ended, [true, false]);
    let json = serde_json::to_value(&runs[1]).unwrap();
    assert!(json["trial"].get("search_ended").is_none());
}

#[test]
fn begin_fails_without_a_staged_launch_config() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    assert!(paths.begin_run("p", false).is_err());
    assert!(paths.list_runs("p").is_empty());
}

#[test]
fn trial_changes_leave_mods_and_patches_out_of_a_copy() {
    let mut p = crate::builtin_vanilla_profile(None);
    p.enabled_mods = vec!["Core".into(), "addon".into()];
    p.disabled_patches = vec!["old".into()];
    let changes = TrialChanges {
        pause_fixes: vec!["no-such-fix".into()],
        disable_patches: vec!["old".into(), "new".into()],
        disable_mods: vec!["core".into()],
    };
    assert!(!changes.is_empty());
    assert!(TrialChanges::default().is_empty());
    changes.apply(&mut p);
    assert_eq!(p.enabled_mods, vec!["addon".to_string()]);
    assert_eq!(
        p.disabled_patches,
        vec!["old".to_string(), "new".to_string()]
    );
}

#[test]
fn a_paused_fix_takes_out_only_its_own_legacy_entries() {
    use crate::fixes::{find, op_disable, FixSource, Provenance, FIXES_VERSION};
    let mut p = crate::builtin_vanilla_profile(None);
    p.fixes_version = FIXES_VERSION;
    let ids = |id: &str| std::collections::HashSet::from([id.to_string()]);
    let a = op_disable(
        &mut p,
        None,
        "a:1",
        &ids("a:1"),
        Provenance::new(FixSource::Resolver),
    )
    .unwrap();
    op_disable(
        &mut p,
        None,
        "b:1",
        &ids("b:1"),
        Provenance::new(FixSource::Resolver),
    )
    .unwrap();
    // Hand edits the ledger does not know: one entry added, one fix entry taken out.
    p.disabled_patches.retain(|d| d != "b:1");
    p.disabled_patches.push("hand:1".into());
    p.patch_priorities.insert("hand:2".into(), 7);
    let changes = TrialChanges {
        pause_fixes: vec![a.clone()],
        ..TrialChanges::default()
    };
    let mut q = p.clone();
    changes.apply(&mut q);
    // Only the paused fix's entry is gone; the hand edits stay as a normal launch has them.
    assert_eq!(q.disabled_patches, vec!["hand:1".to_string()]);
    assert_eq!(q.patch_priorities.get("hand:2"), Some(&7));
    assert!(find(&q, &a).unwrap().paused);
}

#[test]
fn a_trial_run_records_the_trial() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    staged(&paths, "p");
    let trial = TrialInfo {
        step: 2,
        label: "the fix \"Run A first\"".into(),
        pause_fixes: vec!["f1".into()],
        ..TrialInfo::default()
    };
    let run = paths.begin_trial_run("p", trial.clone()).unwrap();
    assert_eq!(run.trial, Some(trial.clone()));
    assert_eq!(paths.open_run("p").unwrap().trial, Some(trial.clone()));
    assert_eq!(trial.changes().pause_fixes, vec!["f1".to_string()]);
}

/// Writes crash copy `id` into the launcher's crash log, as `keep_crashes` names it.
fn crash_copy(paths: &QuarterMasterPaths, id: &str) -> PathBuf {
    let dir = paths.logs_dir.join("crashes");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(crash_file_name(id));
    let c = crash(id, None, Some("p"), START);
    std::fs::write(&file, serde_json::to_string(&c).unwrap()).unwrap();
    file
}

fn linking(run_id: &str, crashes: &[&str]) -> RunEntry {
    RunEntry {
        ended_at: Some(START.into()),
        crashes: crashes.iter().map(|c| c.to_string()).collect(),
        ..entry(run_id)
    }
}

#[test]
fn pruning_keeps_linked_crash_copies_and_removes_unlinked_ones() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    let old = crash_copy(&paths, "crash:old");
    let new = crash_copy(&paths, "crash:new");
    let other = crash_copy(&paths, "crash:other");
    // Not crash reports: never touched.
    let dir = paths.logs_dir.join("crashes");
    std::fs::write(dir.join("notes.json"), "{}").unwrap();
    std::fs::write(dir.join("crash-1.log"), "panic").unwrap();
    // Another profile links its own crash.
    paths
        .save_runs("q", vec![linking("q1", &["crash:other"])])
        .unwrap();

    let mut runs = vec![linking("p0", &["crash:old"])];
    runs.extend((1..MAX_RUNS).map(|n| linking(&format!("p{n}"), &[])));
    runs.push(linking("p-last", &["crash:new"]));
    assert_eq!(runs.len(), MAX_RUNS + 1);
    paths.save_runs("p", runs).unwrap();

    assert_eq!(paths.list_runs("p").len(), MAX_RUNS);
    assert!(!old.exists(), "its run dropped out of the log");
    assert!(new.is_file());
    assert!(other.is_file());
    assert!(dir.join("notes.json").is_file());
    assert!(dir.join("crash-1.log").is_file());
    let listed: Vec<String> = paths
        .list_crash_reports_for("p")
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert!(listed.contains(&"crash:new".to_string()), "{listed:?}");
    assert!(!listed.contains(&"crash:old".to_string()), "{listed:?}");
    // Nothing more to prune.
    assert_eq!(paths.prune_crash_copies(), 0);
}

#[test]
fn pruning_waits_while_a_log_cannot_be_read() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    let lost = crash_copy(&paths, "crash:lost");
    // A log that is not a file cannot be read; its links are not known.
    std::fs::create_dir_all(paths.runs_file("q")).unwrap();
    assert_eq!(paths.prune_crash_copies(), 0);
    assert!(lost.is_file());
    std::fs::remove_dir(paths.runs_file("q")).unwrap();
    assert_eq!(paths.prune_crash_copies(), 1);
    assert!(!lost.exists());
}

#[test]
fn the_test_plan_mark_is_kept_until_cleared() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    assert_eq!(paths.staged_test_plan("p"), None);
    paths
        .mark_test_plan_staged("p", Some("the fix \"Copy\""))
        .unwrap();
    let mark = paths.staged_test_plan("p").unwrap();
    assert_eq!(mark.label.as_deref(), Some("the fix \"Copy\""));
    assert!(parse_time(&mark.staged_at).is_some());
    paths.clear_test_plan_staged("p").unwrap();
    assert_eq!(paths.staged_test_plan("p"), None);
    paths.clear_test_plan_staged("p").unwrap();
    // A mark that does not read still says a test plan is staged.
    std::fs::write(paths.test_plan_file("p"), "not json").unwrap();
    assert_eq!(paths.staged_test_plan("p"), Some(StagedTestPlan::default()));
}

#[test]
fn a_run_with_mods_gets_a_save_stamp_and_a_run_without_does_not() {
    let temp = tempfile::tempdir().unwrap();
    let paths = QuarterMasterPaths::resolve(Some(temp.path()));
    let mods = staged(&paths, "p");
    let fields: BTreeMap<String, Vec<String>> = [
        ("ModA".to_string(), vec!["option_a".to_string()]),
        ("not_enabled".to_string(), vec!["option_x".to_string()]),
    ]
    .into();

    // No plan, so no mods: nothing is stamped.
    paths.begin_run_stamped("p", false, None, &fields).unwrap();
    let cfg = LaunchConfig::load_from_mod_dir(&mods).unwrap().unwrap();
    assert!(cfg.save_stamp().is_none());

    LoadOrderPlan {
        enabled_mods: vec!["moda".into(), "modb".into()],
        ..LoadOrderPlan::default()
    }
    .save_to_mod_dir_atomic(&mods)
    .unwrap();
    LaunchConfig {
        profile_id: Some("p".into()),
        profile_name: Some("Profile P".into()),
        active_mods: HashMap::from([("moda".to_string(), "1.2.0".to_string())]),
        ..LaunchConfig::default()
    }
    .save_to_mod_dir(&mods)
    .unwrap();
    paths.begin_run_stamped("p", false, None, &fields).unwrap();
    let cfg = LaunchConfig::load_from_mod_dir(&mods).unwrap().unwrap();
    let stamp = cfg.save_stamp().unwrap();
    assert_eq!(stamp.profile_id, "p");
    assert_eq!(stamp.profile_name, "Profile P");
    assert_eq!(stamp.mods.len(), 2);
    assert_eq!(stamp.mods["moda"], "1.2.0");
    assert_eq!(stamp.fields.len(), 1, "only enabled mods' fields");
    assert_eq!(stamp.fields["ModA"], ["option_a"]);

    // A launch without mods takes the stamp out again.
    paths.begin_run_stamped("p", true, None, &fields).unwrap();
    let cfg = LaunchConfig::load_from_mod_dir(&mods).unwrap().unwrap();
    assert!(cfg.save_stamp().is_none());
}
