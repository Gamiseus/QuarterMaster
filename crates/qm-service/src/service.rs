//! The core `Service`: shared state, caches and the invalidation table.
use crate::cache::Caches;
use crate::error::{ErrorKind, QmError};
use crate::events::{EventSink, QmEvent, Scope};
use crate::game::GameStatus;
use crate::jobs::{JobId, JobKind, JobRunner};
use crate::net::NetPolicy;
use crate::process::{ProcessProbe, WatchTimings};
use crate::refresh::Refresher;
use qm_manager::{ModProfile, QuarterMasterPaths};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};

/// Refusal while an install job changes the library.
pub(crate) const MSG_WAIT_INSTALL: &str = "Wait for the install to finish.";

pub(crate) struct Inner {
    pub paths: QuarterMasterPaths,
    pub sink: Arc<dyn EventSink>,
    pub probe: Arc<dyn ProcessProbe>,
    pub jobs: JobRunner,
    pub caches: Caches,
    /// Serializes every game-folder transaction and recovery.
    pub game_folder: Mutex<()>,
    /// Held for the library-changing part of install jobs (never across a download).
    pub install: Mutex<()>,
    /// Current launch state; never held while emitting events.
    pub game: Mutex<GameStatus>,
    /// Cache generations, bumped on every clear; fills store only if unchanged.
    pub gens: Mutex<Generations>,
    pub refresher: Refresher,
    /// Background health refresh state (not a job; see `ops::diagnostics`).
    pub health_refresh: Mutex<BackgroundSlot>,
    /// Set by an explicit update check; the next `UpdatesRefresh` pass asks the network.
    pub updates_network: AtomicBool,
    /// Game sprites as data URLs, by payload path.
    pub sprite_cache: Mutex<HashMap<String, String>>,
    /// Ribbon names from the game's localisation file.
    pub ribbon_cache: Mutex<Option<Vec<crate::ops::sprites::RibbonInfo>>>,
    /// Held across every profile.json write in the service, and across the background
    /// stamp's read, change and save; counts the writes, so a pass can tell that the profile
    /// changed since it started.
    pub profile_write: Mutex<u64>,
    /// One simulation preview at a time; a newer request supersedes a waiting one.
    pub preview: Mutex<()>,
    pub preview_seq: std::sync::atomic::AtomicU64,
    /// The profile a run without mods staged empty; staged again when that run ends.
    pub restage_after_run: Mutex<Option<String>>,
    /// The open run of the launch in progress: profile id and run id (D67).
    pub current_run: Mutex<Option<(String, String)>>,
    /// The search for a crash's cause (D68 step 2); not kept across restarts.
    pub bisect: Mutex<Option<crate::bisect::Bisect>>,
    /// Refresh passes held by a test (`testing::hold_pass`); empty otherwise.
    pub pass_gates: PassGates,
    /// Test seam: the next suggestions pass fails (`testing::fail_next_suggestions`).
    pub fail_next_suggestions: AtomicBool,
    /// Test seam: runs once inside the next suggestions pass, after it read the generation
    /// and the profile (`testing::on_next_suggestions_pass`).
    #[allow(clippy::type_complexity)]
    pub on_suggestions_pass: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Test seam: runs once inside the next simulation pass, after it read the generation
    /// and the profile write count (`testing::on_next_simulation_pass`).
    #[allow(clippy::type_complexity)]
    pub on_simulation_pass: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

/// Refresh job kinds whose passes wait; only tests hold one (`testing::hold_pass`).
#[derive(Default)]
pub(crate) struct PassGates {
    held: Mutex<HashSet<JobKind>>,
    cv: Condvar,
}

impl PassGates {
    pub(crate) fn hold(&self, kind: JobKind) {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(kind);
    }

    pub(crate) fn release(&self, kind: JobKind) {
        self.held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&kind);
        self.cv.notify_all();
    }

    /// Returns once `kind` is not held.
    fn wait(&self, kind: JobKind) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        while held.contains(&kind) {
            held = self.cv.wait(held).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// Coalescing flags for a non-job background refresh.
#[derive(Debug, Default)]
pub(crate) struct BackgroundSlot {
    pub dirty: bool,
    pub running: bool,
}

/// Generation per refillable cache. Clearing a cache and bumping its counter happen under the
/// same lock, so a job that read older inputs cannot store over a newer invalidation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Generations {
    pub mods: u32,
    pub vanilla: u32,
    pub sim: u32,
    pub saves: u32,
    pub health: u32,
    pub updates: u32,
    pub catalog: u32,
}

#[derive(Clone)]
pub struct Service {
    pub(crate) inner: Arc<Inner>,
    pub(crate) timings: WatchTimings,
    /// GitHub API base and per-hop URL checks for every request the service makes.
    pub(crate) net: NetPolicy,
}

impl Service {
    pub fn new(
        paths: QuarterMasterPaths,
        sink: Arc<dyn EventSink>,
        probe: Arc<dyn ProcessProbe>,
    ) -> Self {
        let jobs = JobRunner::new(sink.clone());
        let refresher = Refresher::new(jobs.clone());
        Self {
            inner: Arc::new(Inner {
                paths,
                sink,
                probe,
                jobs,
                caches: Caches::default(),
                game_folder: Mutex::new(()),
                install: Mutex::new(()),
                game: Mutex::new(GameStatus::idle()),
                gens: Mutex::new(Generations::default()),
                refresher,
                health_refresh: Mutex::new(BackgroundSlot::default()),
                updates_network: AtomicBool::new(false),
                sprite_cache: Mutex::new(HashMap::new()),
                ribbon_cache: Mutex::new(None),
                profile_write: Mutex::new(0),
                preview: Mutex::new(()),
                preview_seq: std::sync::atomic::AtomicU64::new(0),
                restage_after_run: Mutex::new(None),
                current_run: Mutex::new(None),
                bisect: Mutex::new(None),
                pass_gates: PassGates::default(),
                fail_next_suggestions: AtomicBool::new(false),
                on_suggestions_pass: Mutex::new(None),
                on_simulation_pass: Mutex::new(None),
            }),
            timings: WatchTimings::default(),
            net: NetPolicy::production(),
        }
    }

    pub fn with_timings(mut self, timings: WatchTimings) -> Self {
        self.timings = timings;
        self
    }

    /// Replace the network policy (tests point it at a local server or deny everything).
    pub fn with_net(mut self, net: NetPolicy) -> Self {
        self.net = net;
        self
    }

    pub(crate) fn net(&self) -> &NetPolicy {
        &self.net
    }

    pub fn paths(&self) -> &QuarterMasterPaths {
        &self.inner.paths
    }

    pub fn jobs(&self) -> &JobRunner {
        &self.inner.jobs
    }

    /// True while a `ModImport`, `CatalogInstall` or `ModUpdateInstall` job is listed.
    pub(crate) fn install_running(&self) -> bool {
        self.jobs().list().iter().any(|j| j.kind.is_install())
    }

    /// Block until the install lock is free; held while an install job changes the library.
    pub(crate) fn lock_install(&self) -> MutexGuard<'_, ()> {
        self.inner.install.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Conflict` while an install job is listed or holds the install lock.
    pub(crate) fn ensure_no_install(&self) -> crate::QmResult<()> {
        let held = matches!(self.inner.install.try_lock(), Err(TryLockError::WouldBlock));
        if held || self.install_running() {
            Err(QmError::new(ErrorKind::Conflict, MSG_WAIT_INSTALL))
        } else {
            Ok(())
        }
    }

    /// Clear the caches affected by `scope`, emit its events (after all locks are released),
    /// then request the jobs that refill what was cleared.
    pub fn invalidate(&self, scope: Scope) {
        let c = &self.inner.caches;
        let (emits, refill): (&[Scope], Option<JobKind>) = match scope {
            Scope::Mods => {
                self.with_gens(|g| {
                    c.mods.clear();
                    lock_clear(&c.mod_detail);
                    g.mods = g.mods.wrapping_add(1);
                });
                (&[Scope::Mods, Scope::LoadOrder], Some(JobKind::WarmMods))
            }
            Scope::Profiles => (&[Scope::Profiles, Scope::LoadOrder], None),
            Scope::LoadOrder => (&[Scope::LoadOrder], None),
            Scope::Saves => {
                self.with_gens(|g| {
                    lock_clear(&c.save_detail);
                    lock_clear(&c.save_stamped);
                    g.saves = g.saves.wrapping_add(1);
                });
                (&[Scope::Saves], None)
            }
            Scope::Settings => {
                self.with_gens(|g| {
                    c.vanilla.clear();
                    g.vanilla = g.vanilla.wrapping_add(1);
                    c.health.clear();
                    g.health = g.health.wrapping_add(1);
                });
                (
                    &[Scope::Settings, Scope::LoadOrder, Scope::Health],
                    Some(JobKind::WarmVanilla),
                )
            }
            Scope::Health => {
                self.with_gens(|g| {
                    c.health.clear();
                    g.health = g.health.wrapping_add(1);
                });
                (&[Scope::Health], None)
            }
            Scope::Crashes => (&[Scope::Crashes], None),
            Scope::Fixes => {
                c.suggestions.clear();
                c.plan_view.clear();
                (&[Scope::Fixes], None)
            }
            Scope::Updates => {
                self.with_gens(|g| {
                    c.updates.clear();
                    g.updates = g.updates.wrapping_add(1);
                });
                (&[Scope::Updates], None)
            }
            Scope::Catalog => {
                self.with_gens(|g| {
                    c.catalog.clear();
                    g.catalog = g.catalog.wrapping_add(1);
                });
                (&[Scope::Catalog], None)
            }
        };
        // Every scope that emits LoadOrder makes the simulation stale.
        let resimulate = emits.contains(&Scope::LoadOrder);
        if resimulate {
            self.clear_simulation();
        }
        for s in emits {
            self.notify(*s);
        }
        // Settings refills vanilla first; WarmVanilla then requests the simulation itself.
        if resimulate && scope != Scope::Settings && c.vanilla.get().is_some() {
            self.schedule_simulation();
        }
        if let Some(kind) = refill {
            self.refresh(kind);
        }
    }

    /// Request one more pass of a cache-filling job (`WarmMods`, `WarmVanilla`, `Simulation`,
    /// `UpdatesRefresh`, `CatalogRefresh`).
    pub(crate) fn refresh(&self, kind: JobKind) -> JobId {
        let svc = self.clone();
        self.inner
            .refresher
            .request(kind, Arc::new(move |ctx| svc.refresh_pass(kind, ctx)))
    }

    /// Fill a cache on a miss: starts a refresh unless one is already running.
    pub(crate) fn refresh_if_idle(&self, kind: JobKind) -> JobId {
        let svc = self.clone();
        self.inner
            .refresher
            .request_if_idle(kind, Arc::new(move |ctx| svc.refresh_pass(kind, ctx)))
    }

    fn refresh_pass(
        &self,
        kind: JobKind,
        ctx: &crate::jobs::JobCtx,
    ) -> crate::QmResult<Option<String>> {
        self.inner.pass_gates.wait(kind);
        match kind {
            JobKind::WarmMods => self.warm_mods_pass(ctx),
            JobKind::WarmVanilla => self.warm_vanilla_pass(ctx),
            JobKind::Simulation => self.simulation_pass(ctx),
            JobKind::UpdatesRefresh => self.updates_pass(ctx),
            JobKind::CatalogRefresh => self.catalog_pass(ctx),
            JobKind::FixSuggestions => self.suggestions_pass(ctx),
            other => Err(crate::QmError::new(
                crate::ErrorKind::Internal,
                format!("{other:?} is not a refresh job"),
            )),
        }
    }

    pub(crate) fn generations(&self) -> Generations {
        *self.inner.gens.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` while holding the generations lock.
    pub(crate) fn with_gens<R>(&self, f: impl FnOnce(&mut Generations) -> R) -> R {
        let mut g = self.inner.gens.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g)
    }

    /// Saves `p` under the profile-write lock (see `Inner::profile_write`).
    pub(crate) fn save_profile(&self, p: &ModProfile) -> Result<(), String> {
        self.with_profile_write(|| self.paths().save_profile(p))
    }

    /// Runs the profile.json write `f` under the profile-write lock and counts it.
    pub(crate) fn with_profile_write<R>(&self, f: impl FnOnce() -> R) -> R {
        let mut n = self
            .inner
            .profile_write
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *n = n.wrapping_add(1);
        f()
    }

    /// The number of profile writes so far (see `Inner::profile_write`).
    pub(crate) fn profile_writes(&self) -> u64 {
        *self
            .inner
            .profile_write
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Clear the simulation and bump its generation under one lock.
    pub(crate) fn clear_simulation(&self) {
        self.with_gens(|g| {
            self.inner.caches.simulation.clear();
            self.inner.caches.patch_table.clear();
            self.inner.caches.raw_load.clear();
            self.inner.caches.suggestions.clear();
            self.inner.caches.plan_view.clear();
            g.sim = g.sim.wrapping_add(1);
        });
    }

    /// Emit `Invalidated { scope }` without clearing anything.
    pub(crate) fn notify(&self, scope: Scope) {
        self.inner.sink.emit(QmEvent::Invalidated { scope });
    }
}

fn lock_clear<K, V>(m: &Mutex<HashMap<K, V>>) {
    m.lock().unwrap_or_else(|e| e.into_inner()).clear();
}
