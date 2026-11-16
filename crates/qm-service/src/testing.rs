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

use crate::cache::VanillaFiles;
use crate::dto::{CatalogResult, HealthReport, ModDetail, ModSummary, UpdatesReport};
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
    /// Exited pids and their exit codes.
    exited: Vec<(u32, Option<u32>)>,
    /// `(checks, pid)`: the game shows as `pid` once `checks` more probes have seen none.
    running_after: Option<(u32, u32)>,
}

impl FakeProbe {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_running(&self, pid: Option<u32>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).running = pid;
    }

    /// The next `checks` probes find no game; every probe after that finds `pid`.
    pub fn set_running_after(&self, checks: u32, pid: u32) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running_after = Some((checks, pid));
    }

    /// `pid` exits with code 0.
    pub fn exit(&self, pid: u32) {
        self.exit_with(pid, Some(0));
    }

    /// `pid` exits with `code` (None: the code could not be read).
    pub fn exit_with(&self, pid: u32, code: Option<u32>) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        st.exited.push((pid, code));
        if st.running == Some(pid) {
            st.running = None;
        }
        self.cv.notify_all();
    }
}

impl ProcessProbe for FakeProbe {
    fn find_game(&self) -> Option<u32> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match st.running_after {
            Some((0, pid)) => {
                st.running = Some(pid);
                st.running_after = None;
            }
            Some((n, pid)) => st.running_after = Some((n - 1, pid)),
            None => {}
        }
        st.running
    }

    fn wait_for_exit(&self, pid: u32) -> QmResult<Option<u32>> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some((_, code)) = st.exited.iter().find(|(p, _)| *p == pid) {
                return Ok(*code);
            }
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
    }

    fn spawn(&self, _spawn_fn: &dyn Fn() -> QmResult<u32>) -> QmResult<u32> {
        const FAKE_PID: u32 = 4242;
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Only clears the exit flag; tests decide via `set_running` whether the game process exists.
        st.exited.retain(|(p, _)| *p != FAKE_PID);
        Ok(FAKE_PID)
    }
}

/// Which caches currently hold a value (test inspection only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheState {
    pub mods: bool,
    pub vanilla: bool,
    pub simulation: bool,
    /// The `RawLoad` previews reuse.
    pub raw_load: bool,
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
        raw_load: c.raw_load.get().is_some(),
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

/// Seeds the `RawLoad` previews reuse.
pub fn seed_raw_load(svc: &Service, raw: qm_core::patch::plan::RawLoad) {
    svc.inner.caches.raw_load.set(Arc::new(raw));
}

/// Drops the cached `RawLoad` alone, as if the simulation had not stored one.
pub fn clear_raw_load(svc: &Service) {
    svc.inner.caches.raw_load.clear();
}

/// The cached `RawLoad`.
pub fn raw_load(svc: &Service) -> Option<Arc<qm_core::patch::plan::RawLoad>> {
    svc.inner.caches.raw_load.get()
}

/// The cached simulation report, in full.
pub fn simulation_report(svc: &Service) -> Option<Arc<SimulationReport>> {
    svc.inner.caches.simulation.get()
}

/// The export of `req` as `compat_export_preview` works it out: the preview, the plan
/// activation would write after the export, and the simulation with that plan.
pub fn compat_export_work(
    svc: &Service,
    req: &crate::dto::CompatExportRequest,
) -> crate::QmResult<(
    crate::dto::CompatExportPreview,
    Option<qm_core::handshake::LoadOrderPlan>,
    Option<SimulationReport>,
)> {
    let raw = svc
        .current_raw()
        .ok_or_else(|| crate::QmError::new(crate::ErrorKind::Conflict, "no mods"))?;
    let p = svc.active_profile()?;
    let w = svc.export_work(&p, &raw, req)?;
    Ok((w.preview, w.plan, w.report))
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
            assets: None,
            listing: None,
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
    c.health.set(HealthReport::default());
    c.updates.set(UpdatesReport::default());
    c.catalog.set(CatalogResult::default());
}

pub fn seed_health(svc: &Service, report: HealthReport) {
    svc.inner.caches.health.set(report);
}

pub fn seed_updates(svc: &Service, report: UpdatesReport) {
    svc.inner.caches.updates.set(report);
}

pub fn seed_catalog(svc: &Service, catalog: CatalogResult) {
    svc.inner.caches.catalog.set(catalog);
}

pub fn empty_simulation_report() -> SimulationReport {
    SimulationReport {
        files_touched: vec![],
        conflicts: vec![],
        module_registrations: Default::default(),
        total_patches: 0,
        applied_patches: 0,
        broken_anchors: 0,
        order_diagnostics: Vec::new(),
        traces: Default::default(),
        compile_check: Default::default(),
        plan_error: None,
        plan_warnings: Vec::new(),
        load_errors: Vec::new(),
        fix_bindings: Vec::new(),
        overridden_rules: Vec::new(),
        unknown_ids: Vec::new(),
        suspended_rungs: Vec::new(),
        invalid_rungs: Vec::new(),
        overrides: Vec::new(),
        losses: Vec::new(),
        kept_anchors: Vec::new(),
        runs_as: Vec::new(),
        failure_groups: Vec::new(),
        inferred_edges: Vec::new(),
        save_fields: Default::default(),
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
            load_priority: 50,
            kind: qm_manager::ModKind::Managed,
            faction: None,
        },
        manifest,
        readme_md: None,
        changelog_md: None,
        config_schema: None,
        saved_config: Default::default(),
        asset_files: vec![],
        security_report: Default::default(),
        integrity: qm_schema::IntegrityVerifyResult {
            is_pristine: true,
            modified: vec![],
            missing: vec![],
            untracked: vec![],
        },
    }
}

/// Hold the service install lock: install jobs block before touching the library until the
/// guard drops (a deterministic gate for tests; no release code path changes).
pub fn hold_install_lock(svc: &Service) -> std::sync::MutexGuard<'_, ()> {
    svc.lock_install()
}

/// Hold the preview lock: previews wait until the guard drops.
pub fn hold_preview_lock(svc: &Service) -> std::sync::MutexGuard<'_, ()> {
    svc.inner.preview.lock().unwrap_or_else(|e| e.into_inner())
}

/// Classify a changed path the way the file watcher does.
pub fn classify(svc: &Service, path: &std::path::Path) -> crate::watch::Classified {
    svc.classify(path)
}

/// Holds every refresh pass of `kind` (a job of that kind starts and waits) until dropped.
pub struct PassHold {
    svc: Service,
    kind: crate::jobs::JobKind,
}

impl Drop for PassHold {
    fn drop(&mut self) {
        self.svc.inner.pass_gates.release(self.kind);
    }
}

pub fn hold_pass(svc: &Service, kind: crate::jobs::JobKind) -> PassHold {
    svc.inner.pass_gates.hold(kind);
    PassHold {
        svc: svc.clone(),
        kind,
    }
}

/// How many line rules comparisons the Simulation job has run (D51).
pub fn line_rules_passes(svc: &Service) -> u32 {
    svc.inner
        .caches
        .line_rules_passes
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// How many plan views the Fixes view and its suggestions have built.
pub fn plan_views_built(svc: &Service) -> u32 {
    svc.inner
        .caches
        .plan_views_built
        .load(std::sync::atomic::Ordering::Relaxed)
}

/// Runs `f` once inside the next suggestions pass, after it read the simulation generation
/// and the active profile.
pub fn on_next_suggestions_pass(svc: &Service, f: impl FnOnce() + Send + 'static) {
    *svc.inner
        .on_suggestions_pass
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

/// Runs `f` once inside the next simulation pass, after it read the simulation generation
/// and the profile write count.
pub fn on_next_simulation_pass(svc: &Service, f: impl FnOnce() + Send + 'static) {
    *svc.inner
        .on_simulation_pass
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
}

/// Saves `p` the way the service's own commands do (under the profile-write lock).
pub fn save_profile(svc: &Service, p: &qm_manager::ModProfile) -> Result<(), String> {
    svc.save_profile(p)
}

/// The next suggestions pass fails as if it had panicked.
pub fn fail_next_suggestions(svc: &Service) {
    svc.inner
        .fail_next_suggestions
        .store(true, std::sync::atomic::Ordering::SeqCst);
}
