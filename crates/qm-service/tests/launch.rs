#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_manager::LaunchMethod;
use qm_service::game::GameStage;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, LaunchRequest, QmEvent, Service, WatchTimings};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

// QM_WINMM_DLL is process-global; every test in this binary holds this lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const QM_DLL: &[u8] = b"quartermaster-test-winmm-build";
const FAKE_PID: u32 = 4242;
const UNKNOWN_DLL_MSG: &str =
    "The winmm.dll in the game folder is not a QuarterMaster build. It was left in place.";
const STEAM_TIMEOUT_MSG: &str = "Mud and Blood did not start within 60 seconds.";

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

        fx.add_mod(
            "probe",
            "1.0.0",
            "[mod]\nid = \"probe\"\nname = \"probe\"\nversion = \"1.0.0\"\n",
        );
        let mut prof = fx.paths.get_profile("default").unwrap();
        if !prof.enabled_mods.iter().any(|m| m == "probe") {
            prof.enabled_mods.push("probe".into());
        }
        fx.paths.save_profile(&prof).unwrap();

        let sink = Arc::new(RecordingSink::new());
        let probe = Arc::new(FakeProbe::new());
        let svc = Service::new(fx.paths.clone(), sink.clone(), probe.clone()).with_timings(
            WatchTimings {
                steam_poll: Duration::from_millis(10),
                steam_timeout: Duration::from_millis(200),
            },
        );
        Ctx {
            _guard: guard,
            fx,
            sink,
            probe,
            svc,
        }
    }

    fn mods(&self) -> PathBuf {
        self.fx.game_dir.join("Mods")
    }

    fn mods_present(&self) -> bool {
        std::fs::symlink_metadata(self.mods()).is_ok()
    }

    fn dll_present(&self) -> bool {
        self.fx.game_dir.join("winmm.dll").exists()
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

    /// Poll until a stage matching `pred` was emitted; returns all stages so far.
    fn wait_for(&self, pred: impl Fn(&GameStage) -> bool) -> Vec<GameStage> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let stages = self.stages();
            if stages.iter().any(&pred) {
                return stages;
            }
            assert!(
                Instant::now() < deadline,
                "stage never emitted; got {stages:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn launch(
        &self,
        method: Option<LaunchMethod>,
    ) -> qm_service::QmResult<qm_service::game::GameStatus> {
        self.svc.game_launch(LaunchRequest {
            profile_id: "default".into(),
            method,
        })
    }
}

fn is_idle(s: &GameStage) -> bool {
    matches!(s, GameStage::Idle)
}

#[test]
fn direct_launch_runs_then_cleans_up() {
    let c = Ctx::new();
    let st = c.launch(Some(LaunchMethod::DirectExe)).unwrap();
    c.probe.set_running(Some(FAKE_PID));
    assert_eq!(st.stage, GameStage::Running { pid: FAKE_PID });
    assert!(st.is_modded);
    assert!(c.mods_present());
    assert!(c.dll_present());

    c.probe.exit(FAKE_PID);
    let stages = c.wait_for(is_idle);
    assert_eq!(
        stages,
        vec![
            GameStage::Deploying,
            GameStage::Running { pid: FAKE_PID },
            GameStage::CleaningUp,
            GameStage::Idle,
        ]
    );
    assert!(!c.mods_present());
    assert!(!c.dll_present());
    assert_eq!(c.svc.game_status().stage, GameStage::Idle);
}

#[test]
fn steam_launch_waits_for_process() {
    let c = Ctx::new();
    let st = c.launch(Some(LaunchMethod::SteamUrl)).unwrap();
    assert_eq!(st.stage, GameStage::WaitingForSteam);
    let probe = c.probe.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        probe.set_running(Some(77));
    });

    let stages = c.wait_for(|s| *s == GameStage::Running { pid: 77 });
    t.join().unwrap();
    let w = stages
        .iter()
        .position(|s| *s == GameStage::WaitingForSteam)
        .unwrap();
    let r = stages
        .iter()
        .position(|s| *s == GameStage::Running { pid: 77 })
        .unwrap();
    assert!(w < r, "{stages:?}");

    c.probe.exit(77);
    c.wait_for(is_idle);
    assert!(!c.mods_present());
}

#[test]
fn steam_timeout_fails_and_cleans() {
    let c = Ctx::new();
    c.launch(Some(LaunchMethod::SteamUrl)).unwrap();

    let stages = c.wait_for(|s| matches!(s, GameStage::Failed { .. }));
    assert_eq!(
        stages.last(),
        Some(&GameStage::Failed {
            reason: STEAM_TIMEOUT_MSG.into()
        })
    );
    assert!(!c.mods_present());
    assert!(!c.dll_present());
    assert!(!c.svc.game_status().is_busy());

    let st = c.launch(Some(LaunchMethod::DirectExe)).unwrap();
    c.probe.set_running(Some(FAKE_PID));
    assert_eq!(st.stage, GameStage::Running { pid: FAKE_PID });
    c.probe.exit(FAKE_PID);
    c.wait_for(is_idle);
}

#[test]
fn launch_refused_while_running() {
    let c = Ctx::new();
    c.launch(Some(LaunchMethod::DirectExe)).unwrap();
    c.probe.set_running(Some(FAKE_PID));
    assert!(c.svc.game_status().is_busy());

    let err = c.launch(Some(LaunchMethod::DirectExe)).unwrap_err();
    assert_eq!(err.kind, ErrorKind::GameRunning);
    let err = c.svc.game_restore_clean().unwrap_err();
    assert_eq!(err.kind, ErrorKind::GameRunning);

    c.probe.exit(FAKE_PID);
    c.wait_for(is_idle);
}

#[test]
fn vanilla_launch_restores_instead_of_deploying() {
    let c = Ctx::new();
    // Leave a modded folder behind; a vanilla launch must clean it.
    c.svc.tx_deploy("default").unwrap();
    assert!(c.mods_present());

    let st = c.launch(Some(LaunchMethod::Vanilla)).unwrap();
    c.probe.set_running(Some(FAKE_PID));
    assert_eq!(st.stage, GameStage::Running { pid: FAKE_PID });
    assert!(!st.is_modded);
    assert!(!c.mods_present());
    assert!(!c.dll_present());

    c.probe.exit(FAKE_PID);
    c.wait_for(is_idle);
}

#[test]
fn late_game_after_steam_timeout_is_attached() {
    let c = Ctx::new();
    c.launch(Some(LaunchMethod::SteamUrl)).unwrap();
    c.wait_for(|s| matches!(s, GameStage::Failed { .. }));

    // The game shows up after the timeout; the next status read attaches to it.
    c.probe.set_running(Some(99));
    assert_eq!(c.svc.game_status().stage, GameStage::Running { pid: 99 });
    let err = c.launch(Some(LaunchMethod::DirectExe)).unwrap_err();
    assert_eq!(err.kind, ErrorKind::GameRunning);

    c.sink.clear();
    c.probe.exit(99);
    let stages = c.wait_for(is_idle);
    assert_eq!(stages, vec![GameStage::CleaningUp, GameStage::Idle]);
    assert!(!c.mods_present());
}

#[test]
fn deploy_error_fails_with_restore_and_reason() {
    let c = Ctx::new();
    c.fx.write_dll(b"foreign");

    let err = c.launch(Some(LaunchMethod::DirectExe)).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Integrity);
    assert_eq!(
        c.stages().last(),
        Some(&GameStage::Failed {
            reason: UNKNOWN_DLL_MSG.into()
        })
    );
    assert!(!c.mods_present());
    assert_eq!(
        std::fs::read(c.fx.game_dir.join("winmm.dll")).unwrap(),
        b"foreign"
    );
}

#[test]
fn restore_clean_claims_busy_stage() {
    let c = Ctx::new();
    c.svc.tx_deploy("default").unwrap();
    c.svc.game_restore_clean().unwrap();
    assert_eq!(c.stages(), vec![GameStage::CleaningUp, GameStage::Idle]);
    assert!(!c.mods_present());
}

#[cfg(windows)]
#[test]
fn win32_probe_waits_for_real_process() {
    use qm_service::process::Win32Probe;
    use qm_service::ProcessProbe;
    let child = std::process::Command::new("cmd")
        .args(["/C", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        tx.send(Win32Probe.wait_for_exit(pid)).unwrap();
    });
    let res = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(res.is_ok());
    drop(child);
}
