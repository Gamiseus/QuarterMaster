#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, NewProfile, QmEvent, Scope, Service};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROBE_TOML: &str = "[manifest]\nversion = \"1.0.0\"\n";

struct Ctx {
    fx: support::Fixture,
    sink: Arc<RecordingSink>,
    probe: Arc<FakeProbe>,
    svc: Service,
}

fn ctx() -> Ctx {
    let fx = support::Fixture::new();
    let sink = Arc::new(RecordingSink::new());
    let probe = Arc::new(FakeProbe::new());
    let svc = Service::new(fx.paths.clone(), sink.clone(), probe.clone());
    Ctx {
        fx,
        sink,
        probe,
        svc,
    }
}

fn new_profile(name: &str) -> NewProfile {
    NewProfile {
        name: name.into(),
        id: None,
        clone_from: None,
        linked_save_profile_id: None,
        description: Some("desc".into()),
    }
}

fn ids(svc: &Service) -> Vec<String> {
    svc.profiles_list().into_iter().map(|p| p.id).collect()
}

/// Release a FakeProbe "running" game and wait for the watcher to finish cleanup.
fn stop_game(c: &Ctx) {
    c.probe.exit(4242);
    let deadline = Instant::now() + Duration::from_secs(10);
    while c.svc.game_status().is_busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn create_rename_clone_delete_roundtrip() {
    let c = ctx();
    let a = c.svc.profile_create(new_profile("Alpha")).unwrap();
    assert_eq!(a.id, "alpha");
    assert_eq!(a.description, "desc");

    let r = c.svc.profile_rename("alpha", "Alpha Two").unwrap();
    assert_eq!(r.name, "Alpha Two");
    assert_eq!(c.svc.profile_get("alpha").unwrap().name, "Alpha Two");

    let b = c.svc.profile_clone("alpha", "Beta").unwrap();
    assert_eq!(b.id, "beta");
    assert_eq!(b.description, "desc");
    assert_eq!(b.linked_save_profile_id, a.linked_save_profile_id);
    let list = ids(&c.svc);
    assert!(list.contains(&"alpha".to_string()) && list.contains(&"beta".to_string()));

    c.svc.profile_delete("beta").unwrap();
    assert!(!ids(&c.svc).contains(&"beta".to_string()));
    assert_eq!(
        c.svc.profile_get("beta").unwrap_err().kind,
        ErrorKind::NotFound
    );
    let dup = NewProfile {
        id: Some("alpha".into()),
        ..new_profile("Other")
    };
    assert_eq!(
        c.svc.profile_create(dup).unwrap_err().kind,
        ErrorKind::Conflict
    );
}

#[test]
fn delete_active_profile_is_conflict() {
    let c = ctx();
    let active = c.svc.settings_get().active_profile_id;
    c.svc.profile_get(&active).unwrap();
    let e = c.svc.profile_delete(&active).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert_eq!(
        e.message,
        "Switch to another profile before deleting this one."
    );
}

#[test]
fn activate_updates_settings_and_emits_profiles() {
    let c = ctx();
    c.svc.profile_create(new_profile("Alpha")).unwrap();
    c.sink.clear();
    let p = c.svc.profile_activate("alpha").unwrap();
    assert_eq!(p.id, "alpha");
    assert_eq!(c.svc.settings_get().active_profile_id, "alpha");
    assert!(c.sink.events().iter().any(|e| matches!(
        e,
        QmEvent::Invalidated {
            scope: Scope::Profiles
        }
    )));
    let row = c
        .svc
        .profiles_list()
        .into_iter()
        .find(|p| p.id == "alpha")
        .unwrap();
    assert!(row.is_active);
    assert!(c.fx.paths.profiles_dir.join("alpha").join("Mods").is_dir());
}

#[test]
fn inactive_profile_edits_allowed_while_running() {
    let c = ctx();
    let active = c.svc.settings_get().active_profile_id;
    c.svc.profile_get(&active).unwrap();
    c.probe.set_running(Some(4242));

    c.svc.profile_create(new_profile("Alpha")).unwrap();
    c.svc.profile_rename("alpha", "Alpha Two").unwrap();
    c.svc.profile_clone("alpha", "Beta").unwrap();
    c.svc.profile_delete("beta").unwrap();

    assert_eq!(
        c.svc.profile_activate("alpha").unwrap_err().kind,
        ErrorKind::GameRunning
    );
    assert_eq!(
        c.svc.profile_rename(&active, "Renamed").unwrap_err().kind,
        ErrorKind::GameRunning
    );
    stop_game(&c);
}

#[test]
fn export_then_import_code_roundtrip() {
    let c = ctx();
    c.fx.add_mod("probe", "1.0.0", PROBE_TOML);
    let mut def = c.fx.paths.get_profile("default").unwrap();
    def.enabled_mods = vec!["probe".into()];
    def.mod_priorities.insert("probe".into(), 70);
    c.fx.paths.save_profile(&def).unwrap();

    let code = c.svc.profile_export_code("default").unwrap();
    assert!(code.starts_with("QM1-"));
    let imported = c.svc.profile_import_code(&code, "imported").unwrap();
    assert_eq!(imported.id, "imported");
    assert_eq!(imported.enabled_mods, vec!["probe".to_string()]);
    assert_eq!(imported.mod_priorities.get("probe"), Some(&70));
    assert_eq!(c.svc.profile_get("imported").unwrap().name, def.name);

    assert_eq!(
        c.svc
            .profile_import_code(&code, "imported")
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        c.svc.profile_import_code("nope", "x").unwrap_err().kind,
        ErrorKind::InvalidInput
    );
}
