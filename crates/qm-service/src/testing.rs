//! Test doubles shared across qm-service tests.
use crate::events::{EventSink, QmEvent};
use std::sync::Mutex;

#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<QmEvent>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<QmEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn clear(&self) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: QmEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(event);
    }
}

pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: QmEvent) {}
}

use crate::cache::{CatalogResult, HealthReport, UpdatesReport, VanillaFiles};
use crate::dto::{ModDetail, ModSummary};
use crate::error::{ErrorKind, QmError, QmResult};
use crate::process::ProcessProbe;
use crate::service::Service;
use qm_manager::{InstalledMod, SimulationReport};
use std::sync::Arc;
use std::sync::Condvar;
use std::time::{Duration, Instant};

/// Fake process probe: tests set the "running" pid and release waiters with `exit`.
#[derive(Default)]
pub struct FakeProbe {
    state: Mutex<FakeState>,
    cv: Condvar,
}

#[derive(Default)]
struct FakeState {
    running: Option<u32>,
    exited: Vec<u32>,
}

impl FakeProbe {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_running(&self, pid: Option<u32>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).running = pid;
    }

    pub fn exit(&self, pid: u32) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.exited.push(pid);
        if st.running == Some(pid) {
            st.running = None;
        }
        self.cv.notify_all();
    }
}

impl ProcessProbe for FakeProbe {
    fn find_game(&self) -> Option<u32> {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).running
    }

    fn wait_for_exit(&self, pid: u32) -> QmResult<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while !st.exited.contains(&pid) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(QmError::new(
                    ErrorKind::Internal,
                    "FakeProbe wait timed out",
                ));
            }
            st = self
                .cv
                .wait_timeout(st, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        Ok(())
    }

    fn spawn(&self, _spawn_fn: &dyn Fn() -> QmResult<u32>) -> QmResult<u32> {
        const FAKE_PID: u32 = 4242;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Only clears the exit flag; tests decide via `set_running` whether the game process exists.
        st.exited.retain(|p| *p != FAKE_PID);
        Ok(FAKE_PID)
    }
}

/// Which caches currently hold a value (test inspection only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheState {
    pub mods: bool,
    pub vanilla: bool,
    pub simulation: bool,
    pub mod_detail_empty: bool,
    pub save_detail_empty: bool,
    pub health: bool,
    pub updates: bool,
    pub catalog: bool,
}

pub fn cached(svc: &Service) -> CacheState {
    let c = &svc.inner.caches;
    let empty_mod = c
        .mod_detail
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty();
    let empty_save = c
        .save_detail
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty();
    CacheState {
        mods: c.mods.get().is_some(),
        vanilla: c.vanilla.get().is_some(),
        simulation: c.simulation.get().is_some(),
        mod_detail_empty: empty_mod,
        save_detail_empty: empty_save,
        health: c.health.get().is_some(),
        updates: c.updates.get().is_some(),
        catalog: c.catalog.get().is_some(),
    }
}

pub fn seed_mods(svc: &Service, mods: Vec<InstalledMod>) {
    svc.inner.caches.mods.set(Arc::new(mods));
}

pub fn seed_vanilla(svc: &Service, vanilla: VanillaFiles) {
    svc.inner.caches.vanilla.set(Arc::new(vanilla));
}

pub fn seed_simulation(svc: &Service, report: SimulationReport) {
    svc.inner.caches.simulation.set(Arc::new(report));
}

/// Fill every cache with a placeholder value (the simulation cache stays empty; use `seed_simulation`).
pub fn seed_all_but_simulation(svc: &Service) {
    let c = &svc.inner.caches;
    seed_mods(svc, vec![]);
    seed_vanilla(
        svc,
        VanillaFiles {
            game_version: "t".into(),
            info: None,
            files: Default::default(),
        },
    );
    c.mod_detail
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert("m".into(), placeholder_mod_detail());
    c.save_detail
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert("s".into(), Default::default());
    c.health.set(HealthReport);
    c.updates.set(UpdatesReport);
    c.catalog.set(CatalogResult);
}

pub fn empty_simulation_report() -> SimulationReport {
    SimulationReport {
        files_touched: vec![],
        conflicts: vec![],
        module_registrations: Default::default(),
        total_patches: 0,
        applied_patches: 0,
        broken_anchors: 0,
    }
}

fn placeholder_mod_detail() -> ModDetail {
    let manifest = qm_schema::ModManifest::new("m".into(), "m".into(), "1.0.0".into());
    ModDetail {
        summary: ModSummary {
            id: "m".into(),
            name: "m".into(),
            version: "1.0.0".into(),
            authors: vec![],
            category: None,
            enabled: false,
            patch_count: 0,
            installed_versions: vec![],
            is_workspace_linked: false,
            has_config: false,
        },
        manifest,
        readme_md: None,
        changelog_md: None,
        config_schema: None,
        saved_config: Default::default(),
        security_report: Default::default(),
        integrity: qm_schema::IntegrityVerifyResult {
            is_pristine: true,
            modified: vec![],
            missing: vec![],
            untracked: vec![],
        },
    }
}
