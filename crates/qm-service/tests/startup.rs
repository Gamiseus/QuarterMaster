#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_manager::StateJournal;
use qm_service::game::GameStage;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{JobKind, QmEvent, Service};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

// QM_WINMM_DLL is process-global; every test in this binary holds this lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const QM_DLL: &[u8] = b"quartermaster-test-winmm-build";

struct Ctx {
    _guard: MutexGuard<'static, ()>,
    fx: support::Fixture,
    sink: Arc<RecordingSink>,
    probe: Arc<FakeProbe>,
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
        let probe = Arc::new(FakeProbe::new());
        let svc = Service::new(fx.paths.clone(), sink.clone(), probe.clone());
        let c = Ctx {
            _guard: guard,
            fx,
            sink,
            probe,
            svc,
        };
        // Fresh caches keep start() off the network.
        c.touch_cache("update-etags.json");
        c.touch_cache("community-catalog-cache.json");
        c
    }

    fn touch_cache(&self, name: &str) {
        let dir = &self.fx.paths.cache_dir;
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), b"{}").unwrap();
    }

    fn mods_present(&self) -> bool {
        std::fs::symlink_metadata(self.fx.game_dir.join("Mods")).is_ok()
    }

    fn dll_present(&self) -> bool {
        self.fx.game_dir.join("winmm.dll").exists()
    }

    fn pending(&self) -> Option<String> {
        StateJournal::load(&self.fx.paths.journal_file)
            .pending_transaction
            .map(|t| t.kind)
    }

    fn write_pending(&self, kind: &str, to: &str) {
        let mut j = StateJournal::load(&self.fx.paths.journal_file);
        j.begin_transaction(&self.fx.paths.journal_file, kind, None, to, None)
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

    fn stages(&self) -> Vec<GameStage> {
        self.sink
            .events()
            .into_iter()
            .filter_map(|e| match e {
                QmEvent::GameStatusChanged { status } => Some(status.stage),
                _ => None,
            })
            .collect()
    }

    fn wait_for_idle(&self) -> Vec<GameStage> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let stages = self.stages();
            if stages.iter().any(|s| matches!(s, GameStage::Idle)) {
                return stages;
            }
            assert!(Instant::now() < deadline, "never went idle; got {stages:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn startup_leaves_modded_game_dir_alone_without_pending_tx() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();

    let report = c.svc.start();

    assert_eq!(report.recovered, None);
    assert_eq!(report.attached_pid, None);
    assert!(c.mods_present());
    assert!(c.dll_present());
    assert!(c.recovered().is_empty());
}

#[test]
fn startup_recovers_pending_tx() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.write_pending("deploy", "default");

    let report = c.svc.start();

    let summary = report.recovered.expect("recovered");
    assert!(summary.starts_with("Recovered from an interrupted deploy"));
    assert_eq!(c.recovered(), vec![summary]);
    assert_eq!(c.pending(), None);
    assert!(!c.mods_present());
    assert!(!c.dll_present());
}

#[test]
fn startup_attaches_and_defers_recovery() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.write_pending("deploy", "default");
    c.probe.set_running(Some(9));

    let report = c.svc.start();

    assert_eq!(report.attached_pid, Some(9));
    assert_eq!(report.recovered, None);
    assert_eq!(c.pending().as_deref(), Some("deploy"));
    assert!(c.mods_present());
    // A status query must not attach a second time.
    assert_eq!(c.svc.game_status().stage, GameStage::Running { pid: 9 });

    c.probe.exit(9);
    let stages = c.wait_for_idle();
    let tail: Vec<_> = stages
        .iter()
        .skip_while(|s| !matches!(s, GameStage::CleaningUp))
        .cloned()
        .collect();
    assert_eq!(tail, vec![GameStage::CleaningUp, GameStage::Idle]);
    assert_eq!(
        stages
            .iter()
            .filter(|s| matches!(s, GameStage::Running { .. }))
            .count(),
        1
    );
    assert_eq!(c.pending(), None);
    assert!(!c.mods_present());
    assert!(!c.dll_present());
}

#[test]
fn startup_skips_fresh_update_cache() {
    let c = Ctx::new();
    c.touch_cache("update-etags.json");

    let _report = c.svc.start();

    assert!(!c
        .svc
        .jobs()
        .list()
        .iter()
        .any(|j| j.kind == JobKind::UpdatesRefresh));
    assert!(!c.sink.events().iter().any(|e| matches!(
        e,
        QmEvent::JobProgress {
            kind: JobKind::UpdatesRefresh,
            ..
        } | QmEvent::JobFinished {
            kind: JobKind::UpdatesRefresh,
            ..
        }
    )));
}

#[test]
fn startup_starts_watching() {
    let c = Ctx::new();
    let report = c.svc.start();
    assert!(report.watch.is_some());
}

#[test]
fn startup_reports_recovery_failure() {
    let c = Ctx::new();
    c.write_pending("profile_swap", "no-such-profile");

    let report = c.svc.start();

    assert_eq!(report.recovered, None);
    let recovered = c.recovered();
    assert_eq!(recovered.len(), 1, "{recovered:?}");
    assert!(
        recovered[0].starts_with("Recovery failed: "),
        "{recovered:?}"
    );
    assert_eq!(c.pending().as_deref(), Some("profile_swap"));
}
