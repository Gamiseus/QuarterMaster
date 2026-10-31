#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_schema::{ConfigValue, ModSettingsStore};
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, Loadable, ModSummary, Scope, Service};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROBE_TOML: &str = "[manifest]\nversion = \"1.0.0\"\n";

struct Ctx {
    fx: support::Fixture,
    probe: Arc<FakeProbe>,
    svc: Service,
}

fn ctx() -> Ctx {
    let fx = support::Fixture::new();
    fx.add_mod("probe", "1.0.0", PROBE_TOML);
    let probe = Arc::new(FakeProbe::new());
    let svc = Service::new(
        fx.paths.clone(),
        Arc::new(RecordingSink::new()),
        probe.clone(),
    );
    Ctx { fx, probe, svc }
}

fn warm(svc: &Service) {
    svc.warm_mods();
    assert!(svc.jobs().wait_idle(Duration::from_secs(20)));
}

fn ready_mods(svc: &Service) -> Vec<ModSummary> {
    match svc.mods_list() {
        Loadable::Ready(v) => v,
        Loadable::Loading => panic!("mods still loading"),
    }
}

fn stop_game(c: &Ctx) {
    c.probe.exit(4242);
    let deadline = Instant::now() + Duration::from_secs(10);
    while c.svc.game_status().is_busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn mods_list_loading_then_ready_after_warm() {
    let c = ctx();
    assert!(matches!(c.svc.mods_list(), Loadable::Loading));
    warm(&c.svc);
    let mods = ready_mods(&c.svc);
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].id, "probe");
    assert_eq!(mods[0].version, "1.0.0");
}

#[test]
fn set_enabled_updates_active_profile() {
    let c = ctx();
    warm(&c.svc);
    let off = c.svc.mod_set_enabled("probe", false).unwrap();
    assert!(!off.enabled);
    assert!(!c
        .svc
        .profile_get("default")
        .unwrap()
        .enabled_mods
        .contains(&"probe".into()));

    let on = c.svc.mod_set_enabled("probe", true).unwrap();
    assert!(on.enabled);
    let prof = c.svc.profile_get("default").unwrap();
    assert_eq!(
        prof.enabled_mods.iter().filter(|m| *m == "probe").count(),
        1
    );
    assert!(ready_mods(&c.svc)[0].enabled);
    // Activation staged the mod into the profile's Mods folder.
    assert!(c
        .fx
        .paths
        .profiles_dir
        .join("default")
        .join("Mods")
        .join("probe")
        .exists());
    assert_eq!(
        c.svc.mod_set_enabled("missing", true).unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn mod_set_enabled_refused_while_running() {
    let c = ctx();
    warm(&c.svc);
    c.probe.set_running(Some(4242));
    assert_eq!(
        c.svc.mod_set_enabled("probe", true).unwrap_err().kind,
        ErrorKind::GameRunning
    );
    assert_eq!(
        c.svc.mod_uninstall("probe").unwrap_err().kind,
        ErrorKind::GameRunning
    );
    stop_game(&c);
}

#[test]
fn config_set_writes_profile_settings_file() {
    let c = ctx();
    warm(&c.svc);
    let mut vals = HashMap::new();
    vals.insert("speed".to_string(), ConfigValue::Number(2.0));
    vals.insert("loud".to_string(), ConfigValue::Bool(true));
    let out = c.svc.mod_config_set("probe", vals).unwrap();
    assert_eq!(out.get("speed"), Some(&ConfigValue::Number(2.0)));

    let prof_file =
        c.fx.paths
            .profiles_dir
            .join("default")
            .join("saveModSettings.lua");
    let store = ModSettingsStore::from_path(&prof_file).unwrap();
    assert_eq!(
        store.mods["probe"].get("loud"),
        Some(&ConfigValue::Bool(true))
    );
    let mirror = ModSettingsStore::from_path(&c.fx.save_dir.join("saveModSettings.lua")).unwrap();
    assert_eq!(mirror, store);

    // Merges with existing values.
    let mut more = HashMap::new();
    more.insert("speed".to_string(), ConfigValue::Number(3.0));
    let out = c.svc.mod_config_set("probe", more).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(
        c.svc.mod_detail("probe").unwrap().saved_config.get("speed"),
        Some(&ConfigValue::Number(3.0))
    );

    let reset = c
        .svc
        .mod_config_reset("probe", qm_service::ConfigResetMode::Defaults)
        .unwrap();
    assert!(reset.is_empty());
}

#[test]
fn uninstall_removes_library_and_profile_refs() {
    let c = ctx();
    warm(&c.svc);
    c.svc.mod_set_enabled("probe", true).unwrap();
    let mut other = c.fx.paths.get_profile("vanilla").unwrap();
    other.enabled_mods.push("probe".into());
    other.mod_priorities.insert("probe".into(), 10);
    other.locked_mods.push("probe".into());
    other.favorites.push("probe".into());
    other.mod_notes.insert("probe".into(), "note".into());
    other.pinned_versions.insert("probe".into(), "1.0.0".into());
    c.fx.paths.save_profile(&other).unwrap();

    c.svc.mod_uninstall("probe").unwrap();
    assert!(c.svc.jobs().wait_idle(Duration::from_secs(20)));

    assert!(!c.fx.paths.library_dir.join("probe").exists());
    for p in c.fx.paths.list_profiles() {
        assert!(!p.enabled_mods.contains(&"probe".into()), "{}", p.id);
        assert!(!p.mod_priorities.contains_key("probe"));
        assert!(!p.locked_mods.contains(&"probe".into()));
        assert!(!p.favorites.contains(&"probe".into()));
        assert!(!p.mod_notes.contains_key("probe"));
        assert!(!p.pinned_versions.contains_key("probe"));
    }
    assert!(ready_mods(&c.svc).is_empty());
    assert!(!c
        .fx
        .paths
        .profiles_dir
        .join("default")
        .join("Mods")
        .join("probe")
        .exists());
    assert_eq!(
        c.svc.mod_uninstall("probe").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn mod_detail_is_cached_until_mods_invalidated() {
    let c = ctx();
    let readme =
        c.fx.paths
            .library_dir
            .join("probe")
            .join("1.0.0")
            .join("README.md");
    std::fs::write(&readme, "# Probe").unwrap();
    warm(&c.svc);

    let first = c.svc.mod_detail("probe").unwrap();
    assert_eq!(first.readme_md.as_deref(), Some("# Probe"));
    assert!(first.integrity.untracked.contains(&"README.md".to_string()));

    std::fs::remove_file(&readme).unwrap();
    let second = c.svc.mod_detail("probe").unwrap();
    assert_eq!(second.readme_md.as_deref(), Some("# Probe"));
    // The integrity check was not re-run.
    assert_eq!(second.integrity, first.integrity);

    c.svc.invalidate(Scope::Mods);
    warm(&c.svc);
    let third = c.svc.mod_detail("probe").unwrap();
    assert_eq!(third.readme_md, None);
    assert!(!third.integrity.untracked.contains(&"README.md".to_string()));
}

#[test]
fn invalidate_mods_refills_mods_list() {
    let c = ctx();
    warm(&c.svc);
    c.fx.add_mod("second", "1.0.0", PROBE_TOML);
    c.svc.invalidate(Scope::Mods);
    let deadline = Instant::now() + Duration::from_secs(20);
    let ids = loop {
        if let Loadable::Ready(v) = c.svc.mods_list() {
            break v.into_iter().map(|m| m.id).collect::<Vec<_>>();
        }
        assert!(Instant::now() < deadline, "mods_list stayed Loading");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(ids.contains(&"second".to_string()), "{ids:?}");
}

#[test]
fn corrupt_settings_file_is_not_overwritten() {
    let c = ctx();
    warm(&c.svc);
    let file =
        c.fx.paths
            .profiles_dir
            .join("default")
            .join("saveModSettings.lua");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "return {{{ not lua").unwrap();
    let mut vals = HashMap::new();
    vals.insert("speed".to_string(), ConfigValue::Number(2.0));
    let e = c.svc.mod_config_set("probe", vals).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io);
    assert_eq!(
        e.message,
        "Mod settings file could not be read; it was not changed."
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "return {{{ not lua"
    );
}
