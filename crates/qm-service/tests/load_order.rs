#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_manager::ConflictResolutionSuggestion;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{
    ErrorKind, LoadOrderUpdate, Loadable, QmEvent, Scope, Service, SettingsPatch,
    SimulationOverview,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONSOLE_PATCH: &str = r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.pattern]
target = "conf.lua"
pattern = "t.console = false"
position = "at"
payload = "t.console = true"
"#;

const PLAIN_TOML: &str = "[manifest]\nversion = \"1.0.0\"\n";

struct Ctx {
    fx: support::Fixture,
    sink: Arc<RecordingSink>,
    svc: Service,
}

fn ctx() -> Ctx {
    let fx = support::Fixture::new();
    let sink = Arc::new(RecordingSink::new());
    let svc = Service::new(fx.paths.clone(), sink.clone(), Arc::new(FakeProbe::new()));
    Ctx { fx, sink, svc }
}

fn idle(svc: &Service) {
    assert!(svc.jobs().wait_idle(Duration::from_secs(30)));
}

/// Install the console patch mod, stage the default profile and run every warm-up job.
fn warmed_with_patch() -> Ctx {
    let c = ctx();
    c.fx.add_mod("probe", "1.0.0", CONSOLE_PATCH);
    let mut def = c.fx.paths.get_profile("default").unwrap();
    def.enabled_mods = vec!["probe".into()];
    c.fx.paths.save_profile(&def).unwrap();
    c.svc.profile_activate("default").unwrap();
    c.svc.warm_mods();
    c.svc.warm_vanilla();
    idle(&c.svc);
    c
}

fn overview(svc: &Service) -> SimulationOverview {
    match svc.simulation_get() {
        Loadable::Ready(o) => o,
        Loadable::Loading => panic!("simulation still loading"),
    }
}

#[test]
fn simulation_ready_after_warm_reports_applied_patch() {
    let c = ctx();
    assert!(matches!(c.svc.simulation_get(), Loadable::Loading));
    drop(c);

    let c = warmed_with_patch();
    let o = overview(&c.svc);
    assert_eq!(o.total_patches, 1);
    assert_eq!(o.applied_patches, 1);
    assert_eq!(o.broken_anchors, 0);
    assert_eq!(o.files, vec!["conf.lua".to_string()]);
    assert!(c.sink.events().iter().any(|e| matches!(
        e,
        QmEvent::Invalidated {
            scope: Scope::LoadOrder
        }
    )));
    match c.svc.app_info().game {
        Loadable::Ready(Some(g)) => assert_eq!(g.version, "0.5.3b"),
        other => panic!("unexpected game info {other:?}"),
    }
}

#[test]
fn file_diff_uses_cached_simulation() {
    let c = ctx();
    assert!(matches!(
        c.svc.file_diff_get("conf.lua").unwrap(),
        Loadable::Loading
    ));
    drop(c);

    let c = warmed_with_patch();
    // Remove the staged mod: a fresh simulation would no longer touch conf.lua.
    let staged = c.fx.paths.profiles_dir.join("default").join("Mods");
    qm_manager::remove_junction_or_dir(&staged.join("probe")).unwrap();

    let diff = match c.svc.file_diff_get("conf.lua").unwrap() {
        Loadable::Ready(d) => d,
        Loadable::Loading => panic!("diff still loading"),
    };
    assert_eq!(diff.result.target_file, "conf.lua");
    assert_eq!(diff.original_lines, vec!["t.console = false".to_string()]);
    assert!(diff
        .result
        .attributed_lines
        .iter()
        .any(|l| l.text.contains("t.console = true")));
    assert_eq!(
        c.svc.file_diff_get("nope.lua").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn load_order_set_persists_and_returns_resolution() {
    let c = ctx();
    c.fx.add_mod("probe", "1.0.0", PLAIN_TOML);
    c.fx.add_mod("other", "1.0.0", PLAIN_TOML);
    let mut def = c.fx.paths.get_profile("default").unwrap();
    def.enabled_mods = vec!["probe".into(), "other".into()];
    c.fx.paths.save_profile(&def).unwrap();
    assert!(matches!(c.svc.load_order_get(), Loadable::Loading));
    c.svc.warm_mods();
    idle(&c.svc);

    let mut prio = HashMap::new();
    prio.insert("probe".to_string(), 90);
    prio.insert("other".to_string(), 10);
    let mut after = HashMap::new();
    after.insert("probe".to_string(), vec!["other".to_string()]);
    let res = c
        .svc
        .load_order_set(LoadOrderUpdate {
            mod_priorities: prio,
            user_load_after: after.clone(),
            locked_mods: vec!["probe".into()],
            separators: vec![],
        })
        .unwrap();
    let order: Vec<&str> = res.ordered_mods.iter().map(|m| m.mod_id.as_str()).collect();
    assert_eq!(order, vec!["other", "probe"]);

    let saved = c.svc.profile_get("default").unwrap();
    assert_eq!(saved.mod_priorities.get("probe"), Some(&90));
    assert_eq!(saved.user_load_after, after);
    assert_eq!(saved.locked_mods, vec!["probe".to_string()]);
    match c.svc.load_order_get() {
        Loadable::Ready(r) => assert_eq!(r.ordered_mods.len(), 2),
        Loadable::Loading => panic!("load order still loading"),
    }
    idle(&c.svc);
}

#[test]
fn select_winner_without_index_is_invalid_input() {
    let c = ctx();
    let sug = ConflictResolutionSuggestion::SelectWinner {
        patch_a_id: "a".into(),
        mod_a_id: "ma".into(),
        patch_b_id: "b".into(),
        mod_b_id: "mb".into(),
        explanation: String::new(),
    };
    let e = c.svc.conflict_resolve(sug.clone(), None).unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidInput);

    c.svc.conflict_resolve(sug, Some(0)).unwrap();
    let p = c.svc.profile_get("default").unwrap();
    assert_eq!(p.disabled_patches, vec!["b".to_string()]);
    idle(&c.svc);
}

#[test]
fn settings_update_rejects_non_mnb_exe() {
    let c = ctx();
    let other = c.fx.game_dir.join("game.exe");
    std::fs::write(&other, b"x").unwrap();
    let bad = SettingsPatch {
        game_exe_path: Some(other),
        ..Default::default()
    };
    assert_eq!(
        c.svc.settings_update(bad).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let missing = SettingsPatch {
        game_exe_path: Some(c.fx.game_dir.join("sub").join("MNB.EXE")),
        ..Default::default()
    };
    assert_eq!(
        c.svc.settings_update(missing).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let backups = SettingsPatch {
        max_save_backups: Some(0),
        ..Default::default()
    };
    assert_eq!(
        c.svc.settings_update(backups).unwrap_err().kind,
        ErrorKind::InvalidInput
    );

    let ok = SettingsPatch {
        game_exe_path: Some(c.fx.game_exe.clone()),
        max_save_backups: Some(5),
        github_token: Some("tok".into()),
        ..Default::default()
    };
    let view = c.svc.settings_update(ok).unwrap();
    assert_eq!(view.max_save_backups, 5);
    assert!(view.github_token_set);
    let cleared = c
        .svc
        .settings_update(SettingsPatch {
            github_token: Some(String::new()),
            ..Default::default()
        })
        .unwrap();
    assert!(!cleared.github_token_set);
    assert!(c.sink.events().iter().any(|e| matches!(
        e,
        QmEvent::Invalidated {
            scope: Scope::Settings
        }
    )));
    idle(&c.svc);
}

#[test]
fn active_profile_edits_refused_while_running() {
    let probe = Arc::new(FakeProbe::new());
    let fx = support::Fixture::new();
    let svc = Service::new(
        fx.paths.clone(),
        Arc::new(RecordingSink::new()),
        probe.clone(),
    );
    svc.profile_get("default").unwrap();
    let game2 = fx.tmp.path().join("game2");
    std::fs::create_dir_all(&game2).unwrap();
    std::fs::copy(&fx.game_exe, game2.join("mnb.exe")).unwrap();
    probe.set_running(Some(4242));

    let update = LoadOrderUpdate {
        mod_priorities: HashMap::new(),
        user_load_after: HashMap::new(),
        locked_mods: vec![],
        separators: vec![],
    };
    assert_eq!(
        svc.load_order_set(update).unwrap_err().kind,
        ErrorKind::GameRunning
    );
    assert_eq!(
        svc.patch_set_enabled("p", false).unwrap_err().kind,
        ErrorKind::GameRunning
    );
    let sug = ConflictResolutionSuggestion::SelectWinner {
        patch_a_id: "a".into(),
        mod_a_id: "ma".into(),
        patch_b_id: "b".into(),
        mod_b_id: "mb".into(),
        explanation: String::new(),
    };
    assert_eq!(
        svc.conflict_resolve(sug, Some(1)).unwrap_err().kind,
        ErrorKind::GameRunning
    );
    let move_exe = SettingsPatch {
        game_exe_path: Some(game2.join("mnb.exe")),
        ..Default::default()
    };
    assert_eq!(
        svc.settings_update(move_exe).unwrap_err().kind,
        ErrorKind::GameRunning
    );
    // Re-sending the current path is not a change.
    let same = SettingsPatch {
        game_exe_path: Some(fx.game_exe.clone()),
        ..Default::default()
    };
    svc.settings_update(same).unwrap();
    assert!(svc
        .profile_get("default")
        .unwrap()
        .disabled_patches
        .is_empty());

    probe.exit(4242);
    let deadline = Instant::now() + Duration::from_secs(10);
    while svc.game_status().is_busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    idle(&svc);
}

#[test]
fn vanilla_rebuilds_after_game_exe_change() {
    let c = ctx();
    c.svc.warm_vanilla();
    idle(&c.svc);
    match c.svc.app_info().game {
        Loadable::Ready(Some(g)) => assert_eq!(g.version, "0.5.3b"),
        other => panic!("unexpected game info {other:?}"),
    }

    // A second install reporting another version.
    let game2 = c.fx.tmp.path().join("game2");
    std::fs::create_dir_all(&game2).unwrap();
    let exe2 = game2.join("mnb.exe");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&exe2).unwrap());
    zip.start_file("main.lua", zip::write::SimpleFileOptions::default())
        .unwrap();
    std::io::Write::write_all(
        &mut zip,
        b"version = \"0.6.0\"
",
    )
    .unwrap();
    zip.finish().unwrap();

    c.svc
        .settings_update(SettingsPatch {
            game_exe_path: Some(exe2),
            ..Default::default()
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Loadable::Ready(Some(g)) = c.svc.app_info().game {
            if g.version == "0.6.0" {
                break;
            }
        }
        assert!(Instant::now() < deadline, "vanilla was not rebuilt");
        std::thread::sleep(Duration::from_millis(10));
    }
    idle(&c.svc);
}
