//! The core `Service`: shared state, caches and the invalidation table.
use crate::cache::Caches;
use crate::events::{EventSink, QmEvent, Scope};
use crate::game::GameStatus;
use crate::jobs::{JobId, JobKind, JobRunner};
use crate::process::{ProcessProbe, WatchTimings};
use crate::refresh::Refresher;
use qm_manager::QuarterMasterPaths;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub(crate) struct Inner {
    pub paths: QuarterMasterPaths,
    pub sink: Arc<dyn EventSink>,
    pub probe: Arc<dyn ProcessProbe>,
    pub jobs: JobRunner,
    pub caches: Caches,
    /// Serializes every game-folder transaction and recovery.
    pub game_folder: Mutex<()>,
    /// Current launch state; never held while emitting events.
    pub game: Mutex<GameStatus>,
    /// Cache generations, bumped on every clear; fills store only if unchanged.
    pub gens: Mutex<Generations>,
    pub refresher: Refresher,
    /// Background health refresh state (not a job; see `ops::diagnostics`).
    pub health_refresh: Mutex<BackgroundSlot>,
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
                game: Mutex::new(GameStatus::idle()),
                gens: Mutex::new(Generations::default()),
                refresher,
                health_refresh: Mutex::new(BackgroundSlot::default()),
            }),
            timings: WatchTimings::default(),
        }
    }

    pub fn with_timings(mut self, timings: WatchTimings) -> Self {
        self.timings = timings;
        self
    }

    pub fn paths(&self) -> &QuarterMasterPaths {
        &self.inner.paths
    }

    pub fn jobs(&self) -> &JobRunner {
        &self.inner.jobs
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
        match kind {
            JobKind::WarmMods => self.warm_mods_pass(ctx),
            JobKind::WarmVanilla => self.warm_vanilla_pass(ctx),
            JobKind::Simulation => self.simulation_pass(ctx),
            JobKind::UpdatesRefresh => self.updates_pass(ctx),
            JobKind::CatalogRefresh => self.catalog_pass(ctx),
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

    /// Clear the simulation and bump its generation under one lock.
    pub(crate) fn clear_simulation(&self) {
        self.with_gens(|g| {
            self.inner.caches.simulation.clear();
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
