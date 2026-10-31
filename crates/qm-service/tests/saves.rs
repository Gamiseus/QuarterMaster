#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, SaveSlotSummary, Service};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAKE_PID: u32 = 4242;

struct Ctx {
    fx: support::Fixture,
    probe: Arc<FakeProbe>,
    svc: Service,
}

fn ctx(auto_backup: bool) -> Ctx {
    let fx = support::Fixture::new();
    let mut s = fx.paths.load_settings();
    s.auto_backup_saves = auto_backup;
    fx.paths.save_settings(&s).unwrap();
    let probe = Arc::new(FakeProbe::new());
    let svc = Service::new(
        fx.paths.clone(),
        Arc::new(RecordingSink::new()),
        probe.clone(),
    );
    Ctx { fx, probe, svc }
}

fn slot<'a>(list: &'a [SaveSlotSummary], id: &str) -> &'a SaveSlotSummary {
    list.iter().find(|s| s.profile.id == id).unwrap()
}

fn write_live(c: &Ctx, text: &str) {
    std::fs::write(c.fx.save_dir.join("saveMNB3.lua"), text).unwrap();
}

fn stop_game(c: &Ctx) {
    c.probe.exit(FAKE_PID);
    let deadline = Instant::now() + Duration::from_secs(10);
    while c.svc.game_status().is_busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn switch_save_backs_up_first() {
    let c = ctx(true);
    let before = c.svc.saves_list();
    assert!(slot(&before, "default").is_active);
    write_live(&c, "campaign = 1\n");

    c.svc.save_activate("vanilla").unwrap();

    let after = c.svc.saves_list();
    assert!(slot(&after, "vanilla").is_active);
    assert!(!slot(&after, "default").is_active);
    let labels: Vec<&str> = slot(&after, "default")
        .profile
        .backups
        .iter()
        .map(|b| b.label.as_str())
        .collect();
    assert!(
        labels.contains(&"Auto-backup before save switch"),
        "labels: {labels:?}"
    );
    assert_eq!(
        c.svc.save_activate("missing-slot").unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[test]
fn save_activate_refused_while_running() {
    let c = ctx(false);
    c.svc.saves_list();
    let backup = c.svc.save_backup("default", "manual").unwrap();
    c.probe.set_running(Some(FAKE_PID));
    assert_eq!(
        c.svc.save_activate("vanilla").unwrap_err().kind,
        ErrorKind::GameRunning
    );
    assert_eq!(
        c.svc.save_restore("default", &backup.id).unwrap_err().kind,
        ErrorKind::GameRunning
    );
    stop_game(&c);
    assert_eq!(c.fx.paths.load_settings().active_save_profile_id, "default");
}

#[test]
fn restore_creates_auto_backup() {
    let c = ctx(true);
    c.svc.saves_list();
    write_live(&c, "old = true\n");
    let manual = c.svc.save_backup("default", "manual").unwrap();
    write_live(&c, "new = true\n");

    c.svc.save_restore("default", &manual.id).unwrap();

    let live = std::fs::read_to_string(c.fx.save_dir.join("saveMNB3.lua")).unwrap();
    assert_eq!(live, "old = true\n");
    let list = c.svc.saves_list();
    let backups = &slot(&list, "default").profile.backups;
    let auto = backups
        .iter()
        .find(|b| b.label == "Auto-backup before restore")
        .expect("auto-backup entry");
    let saved = std::fs::read_to_string(auto.path.join("saveMNB3.lua")).unwrap();
    assert_eq!(saved, "new = true\n");
    assert!(backups.iter().any(|b| b.id == manual.id));
    assert_eq!(
        c.svc
            .save_restore("default", "bak-missing")
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

#[test]
fn saves_list_does_not_parse_saves() {
    let c = ctx(false);
    c.svc.saves_list();
    let corrupt = "this is {{ not lua ]] at all \u{0}\u{1}";
    let slot_file =
        c.fx.paths
            .saves_dir
            .join("default")
            .join("active")
            .join("saveMNB3.lua");
    std::fs::create_dir_all(slot_file.parent().unwrap()).unwrap();
    std::fs::write(&slot_file, corrupt).unwrap();
    write_live(&c, corrupt);

    let list = c.svc.saves_list();
    let ids: Vec<&str> = list.iter().map(|s| s.profile.id.as_str()).collect();
    assert_eq!(ids, vec!["default", "vanilla"]);
}
