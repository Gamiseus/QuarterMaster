#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_manager::{QuarterMasterPaths, StateJournal};
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, QmEvent, Service};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

// QM_WINMM_DLL is process-global; every test in this binary holds this lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const QM_DLL: &[u8] = b"quartermaster-test-winmm-build";

struct Ctx {
    _guard: MutexGuard<'static, ()>,
    fx: support::Fixture,
    sink: Arc<RecordingSink>,
    svc: Service,
}

impl Ctx {
    fn new() -> Ctx {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fx = support::Fixture::new();
        let src = fx.tmp.path().join("dll-src").join("winmm.dll");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, QM_DLL).unwrap();
        std::env::set_var("QM_WINMM_DLL", &src);
        let sink = Arc::new(RecordingSink::new());
        let svc = Service::new(fx.paths.clone(), sink.clone(), Arc::new(FakeProbe::new()));
        Ctx {
            _guard: guard,
            fx,
            sink,
            svc,
        }
    }

    fn mods(&self) -> PathBuf {
        self.fx.game_dir.join("Mods")
    }

    fn dll(&self) -> PathBuf {
        self.fx.game_dir.join("winmm.dll")
    }

    fn mods_present(&self) -> bool {
        std::fs::symlink_metadata(self.mods()).is_ok()
    }

    fn pending(&self) -> Option<String> {
        StateJournal::load(&self.fx.paths.journal_file)
            .pending_transaction
            .map(|t| t.kind)
    }

    fn write_pending(&self, kind: &str, from: Option<&str>, to: &str) {
        let mut j = StateJournal::load(&self.fx.paths.journal_file);
        j.begin_transaction(
            &self.fx.paths.journal_file,
            kind,
            from.map(String::from),
            to,
            None,
        )
        .unwrap();
    }

    fn recovered(&self) -> Vec<String> {
        self.sink
            .events()
            .into_iter()
            .filter_map(|e| match e {
                QmEvent::Recovered { summary } => Some(summary),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn deploy_creates_junction_and_dll_and_records_hash() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();

    assert!(qm_manager::profiles::is_reparse_point(&c.mods()));
    assert_eq!(std::fs::read(c.dll()).unwrap(), QM_DLL);
    let raw = std::fs::read_to_string(c.fx.paths.cache_dir.join("qm-dll-hashes.json")).unwrap();
    let hashes: Vec<String> = serde_json::from_str(&raw).unwrap();
    assert!(hashes.contains(&qm_schema::sha256_hex(QM_DLL)));
    assert_eq!(c.pending(), None);
}

#[test]
fn restore_removes_junction_and_known_dll() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.svc.tx_restore_clean().unwrap();

    assert!(!c.mods_present());
    assert!(!c.dll().exists());
    // The junction target (profile staging) must survive.
    assert!(c
        .fx
        .paths
        .profiles_dir
        .join("default")
        .join("Mods")
        .is_dir());
    assert_eq!(c.pending(), None);
}

#[test]
fn restore_leaves_unknown_dll() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.fx.write_dll(b"foreign");

    let err = c.svc.tx_restore_clean().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Integrity);
    assert_eq!(
        err.message,
        "The winmm.dll in the game folder is not a QuarterMaster build. It was left in place."
    );
    assert_eq!(std::fs::read(c.dll()).unwrap(), b"foreign");
    assert!(!c.mods_present());
}

#[test]
fn deploy_refuses_unknown_dll() {
    let c = Ctx::new();
    c.fx.write_dll(b"foreign");

    let err = c.svc.tx_deploy("default").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Integrity);
    assert!(!c.mods_present());
    assert_eq!(std::fs::read(c.dll()).unwrap(), b"foreign");
    assert_eq!(c.pending(), None);
}

#[test]
fn recover_rolls_back_interrupted_deploy() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.write_pending("deploy", None, "default");

    let summary = c.svc.recover().unwrap();
    assert_eq!(
        summary.as_deref(),
        Some("Recovered from an interrupted deploy (target 'default').")
    );
    assert!(!c.mods_present());
    assert!(!c.dll().exists());
    assert_eq!(c.pending(), None);
    assert_eq!(c.recovered().len(), 1);
}

#[test]
fn recover_completes_interrupted_restore() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.write_pending("restore_clean", None, "vanilla");

    let summary = c.svc.recover().unwrap();
    assert!(summary.is_some());
    assert!(!c.mods_present());
    assert!(!c.dll().exists());
    assert_eq!(c.pending(), None);
    assert_eq!(c.recovered().len(), 1);
}

#[test]
fn recover_with_empty_journal_returns_none() {
    let c = Ctx::new();
    assert_eq!(c.svc.recover().unwrap(), None);
    assert!(c.recovered().is_empty());
}

#[test]
fn resolve_does_not_auto_recover() {
    let c = Ctx::new();
    c.write_pending("profile_swap", Some("default"), "vanilla");
    let _paths = QuarterMasterPaths::resolve(Some(&c.fx.paths.root_dir));
    assert_eq!(c.pending().as_deref(), Some("profile_swap"));
}

#[test]
fn repair_refuses_unknown_dll() {
    let c = Ctx::new();
    c.fx.write_dll(b"foreign");
    let err = c.svc.tx_repair_dll().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Integrity);
    assert_eq!(std::fs::read(c.dll()).unwrap(), b"foreign");
    assert_eq!(c.pending(), None);
}

#[test]
fn repair_installs_dll_and_returns_hash() {
    let c = Ctx::new();
    let hash = c.svc.tx_repair_dll().unwrap();
    assert_eq!(hash, qm_schema::sha256_hex(QM_DLL));
    assert_eq!(std::fs::read(c.dll()).unwrap(), QM_DLL);
    assert_eq!(c.pending(), None);
}

#[test]
fn known_hashes_include_recorded_builds() {
    let c = Ctx::new();
    let old = qm_schema::sha256_hex(b"older-qm-build");
    std::fs::write(
        c.fx.paths.cache_dir.join("qm-dll-hashes.json"),
        serde_json::to_string(&vec![old.clone()]).unwrap(),
    )
    .unwrap();
    let known = c.svc.known_dll_hashes();
    assert!(known.contains(&old));
    assert!(known.contains(&qm_schema::sha256_hex(QM_DLL)));

    // A DLL from an older recorded build is replaced, not refused.
    c.fx.write_dll(b"older-qm-build");
    c.svc.tx_repair_dll().unwrap();
    assert_eq!(std::fs::read(c.dll()).unwrap(), QM_DLL);
}

#[test]
fn source_dll_missing_is_not_found() {
    let c = Ctx::new();
    std::env::set_var("QM_WINMM_DLL", c.fx.tmp.path().join("nope.dll"));
    let err = c.svc.source_dll().unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(
        err.message,
        "winmm.dll was not found next to qm-launcher.exe."
    );
}

#[test]
fn switch_save_journals_and_commits() {
    let c = Ctx::new();
    c.svc.tx_switch_save("slot2").unwrap();
    assert_eq!(c.fx.paths.load_settings().active_save_profile_id, "slot2");
    let j = StateJournal::load(&c.fx.paths.journal_file);
    assert!(j.pending_transaction.is_none());
    assert_eq!(j.active_save_profile_id.as_deref(), Some("slot2"));
}

#[test]
fn recover_rolls_back_interrupted_save_swap() {
    let c = Ctx::new();
    let before = c.fx.paths.load_settings().active_save_profile_id;
    c.svc.tx_switch_save("slot2").unwrap();
    c.write_pending("save_swap", Some(&before), "slot2");

    c.svc.recover().unwrap();
    assert_eq!(c.fx.paths.load_settings().active_save_profile_id, before);
    assert_eq!(c.pending(), None);
}

#[test]
fn recover_deploy_with_unknown_dll_reports_it() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.fx.write_dll(b"foreign");
    c.write_pending("deploy", None, "default");

    let summary = c.svc.recover().unwrap().unwrap();
    assert!(summary.contains("An unknown winmm.dll was left in place."));
    assert_eq!(std::fs::read(c.dll()).unwrap(), b"foreign");
    assert!(!c.mods_present());
    assert_eq!(c.pending(), None);
    assert_eq!(c.recovered(), vec![summary]);
}

#[test]
fn recover_completes_interrupted_dll_repair() {
    let c = Ctx::new();
    c.write_pending("dll_repair", None, "winmm.dll");

    let summary = c.svc.recover().unwrap();
    assert_eq!(
        summary.as_deref(),
        Some("Recovered from an interrupted dll_repair (target 'winmm.dll').")
    );
    assert_eq!(std::fs::read(c.dll()).unwrap(), QM_DLL);
    assert_eq!(c.pending(), None);
    assert_eq!(c.recovered().len(), 1);
}

#[test]
fn deploy_failure_rolls_back_and_clears_journal() {
    let c = Ctx::new();
    // A directory named winmm.dll passes the ownership check (not a file) but makes the
    // DLL install fail after the deploy entry is journaled and the junction is created.
    std::fs::create_dir_all(c.dll()).unwrap();

    let err = c.svc.tx_deploy("default").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Io);
    assert!(!c.mods_present());
    assert!(c.dll().is_dir());
    assert!(!c.fx.game_dir.join("winmm.dll.qm-tmp").exists());
    assert_eq!(c.pending(), None);
}
