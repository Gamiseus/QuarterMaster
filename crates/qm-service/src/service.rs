//! The core `Service`: shared state, caches and the invalidation table.
use crate::cache::Caches;
use crate::events::{EventSink, QmEvent, Scope};
use crate::game::GameStatus;
use crate::jobs::JobRunner;
use crate::process::{ProcessProbe, WatchTimings};
use qm_manager::QuarterMasterPaths;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
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
    /// Bumped whenever the simulation cache is cleared; a run only stores if it is unchanged.
    pub sim_gen: AtomicU32,
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
        Self {
            inner: Arc::new(Inner {
                paths,
                sink,
                probe,
                jobs,
                caches: Caches::default(),
                game_folder: Mutex::new(()),
                game: Mutex::new(GameStatus::idle()),
                sim_gen: AtomicU32::new(0),
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

    /// Clear the caches affected by `scope`, then emit its events (after all locks are released).
    pub fn invalidate(&self, scope: Scope) {
        let pending = self.clear_scope(scope);
        self.finish_invalidate(pending);
    }

    /// First half of `invalidate`: clears caches only. Callers may refill a cache before
    /// `finish_invalidate` emits, so listeners refetch data that is already there.
    pub(crate) fn clear_scope(&self, scope: Scope) -> PendingInvalidation {
        let c = &self.inner.caches;
        let mut resimulate = false;
        let emits: &'static [Scope] = match scope {
            Scope::Mods => {
                c.mods.clear();
                lock_clear(&c.mod_detail);
                c.simulation.clear();
                resimulate = true;
                &[Scope::Mods, Scope::LoadOrder]
            }
            Scope::Profiles => {
                c.simulation.clear();
                resimulate = true;
                &[Scope::Profiles, Scope::LoadOrder]
            }
            Scope::LoadOrder => {
                c.simulation.clear();
                resimulate = true;
                &[Scope::LoadOrder]
            }
            Scope::Saves => {
                lock_clear(&c.save_detail);
                &[Scope::Saves]
            }
            Scope::Settings => {
                c.vanilla.clear();
                c.simulation.clear();
                c.health.clear();
                &[Scope::Settings, Scope::LoadOrder, Scope::Health]
            }
            Scope::Health => {
                c.health.clear();
                &[Scope::Health]
            }
            Scope::Crashes => &[Scope::Crashes],
            Scope::Updates => {
                c.updates.clear();
                &[Scope::Updates]
            }
            Scope::Catalog => {
                c.catalog.clear();
                &[Scope::Catalog]
            }
        };
        if emits.contains(&Scope::LoadOrder) {
            self.inner.sim_gen.fetch_add(1, Ordering::SeqCst);
        }
        PendingInvalidation { emits, resimulate }
    }

    /// Second half of `invalidate`: emits the events and reschedules the simulation.
    pub(crate) fn finish_invalidate(&self, pending: PendingInvalidation) {
        for s in pending.emits {
            self.notify(*s);
        }
        if pending.resimulate && self.inner.caches.vanilla.get().is_some() {
            self.schedule_simulation();
        }
    }

    /// Emit `Invalidated { scope }` without clearing anything.
    pub(crate) fn notify(&self, scope: Scope) {
        self.inner.sink.emit(QmEvent::Invalidated { scope });
    }
}

pub(crate) struct PendingInvalidation {
    emits: &'static [Scope],
    resimulate: bool,
}

fn lock_clear<K, V>(m: &Mutex<HashMap<K, V>>) {
    m.lock().unwrap_or_else(|e| e.into_inner()).clear();
}
