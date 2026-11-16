//! Coalescing refresh requests for the cache-filling singleton jobs (WarmMods, WarmVanilla,
//! Simulation). A request is never lost: it either marks a running job dirty (the job loops),
//! starts a job, or, if the runner joined a job that is already finishing, retries once that
//! job has left the table.
use crate::error::QmResult;
use crate::jobs::{JobCtx, JobId, JobKind, JobRunner};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// One compute pass; the refresher calls it again while requests keep arriving.
pub(crate) type RefreshBody = Arc<dyn Fn(&JobCtx) -> QmResult<Option<String>> + Send + Sync>;

const RETRY_WAIT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Slot {
    dirty: bool,
    running: bool,
    last_id: Option<JobId>,
}

struct Inner {
    jobs: JobRunner,
    slots: Mutex<HashMap<JobKind, Slot>>,
    #[cfg(test)]
    on_exit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[derive(Clone)]
pub(crate) struct Refresher {
    inner: Arc<Inner>,
}

impl Refresher {
    pub(crate) fn new(jobs: JobRunner) -> Self {
        Self {
            inner: Arc::new(Inner {
                jobs,
                slots: Mutex::new(HashMap::new()),
                #[cfg(test)]
                on_exit: Mutex::new(None),
            }),
        }
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<JobKind, Slot>> {
        self.inner.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask for one more pass of `body` for `kind`; returns the job that will do it.
    pub(crate) fn request(&self, kind: JobKind, body: RefreshBody) -> JobId {
        self.request_with(kind, body, false)
    }

    /// Like `request`, but joins a running pass without asking for another one. Cache misses
    /// use this so repeated reads while loading do not queue extra passes.
    pub(crate) fn request_if_idle(&self, kind: JobKind, body: RefreshBody) -> JobId {
        self.request_with(kind, body, true)
    }

    fn request_with(&self, kind: JobKind, body: RefreshBody, if_idle: bool) -> JobId {
        let mut slots = self.slots();
        let slot = slots.entry(kind).or_default();
        if if_idle && slot.running {
            if let Some(id) = slot.last_id {
                return id;
            }
        }
        slot.dirty = true;
        if slot.running {
            if let Some(id) = slot.last_id {
                return id;
            }
        }
        slot.running = true;
        // The closure owns `token`; if the runner joins an existing job it drops the closure
        // before `spawn` returns. The job body cannot finish meanwhile: it needs `slots`.
        let token = Arc::new(());
        let held = token.clone();
        let me = self.clone();
        let run_body = body.clone();
        let id = self.inner.jobs.spawn(kind, move |ctx| {
            let _held = held;
            me.run(kind, &run_body, ctx)
        });
        slot.last_id = Some(id);
        if Arc::strong_count(&token) == 1 {
            // Joined a job we do not drive (finishing, or started elsewhere): retry after it.
            slot.running = false;
            drop(slots);
            self.retry_after(id, kind, body);
        }
        id
    }

    fn run(&self, kind: JobKind, body: &RefreshBody, ctx: &JobCtx) -> QmResult<Option<String>> {
        let mut last = Ok(None);
        loop {
            {
                let mut slots = self.slots();
                let slot = slots.entry(kind).or_default();
                if !slot.dirty {
                    slot.running = false;
                    break;
                }
                slot.dirty = false;
            }
            let guard = ResetOnPanic { r: self, kind };
            last = body(ctx);
            std::mem::forget(guard);
        }
        #[cfg(test)]
        {
            let hook = self
                .inner
                .on_exit
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(h) = hook {
                h();
            }
        }
        last
    }

    fn mark_stopped(&self, kind: JobKind) {
        self.slots().entry(kind).or_default().running = false;
    }

    fn retry_after(&self, id: JobId, kind: JobKind, body: RefreshBody) {
        let me = self.clone();
        let started = std::thread::Builder::new()
            .name("qm-refresh-retry".into())
            .spawn(move || {
                if !me.inner.jobs.wait_for(id, RETRY_WAIT) {
                    tracing::warn!("job {} still running; retrying {kind:?} anyway", id.0);
                }
                me.request(kind, body);
            });
        if let Err(e) = started {
            tracing::error!("could not start refresh retry thread: {e}");
        }
    }

    /// Test hook run once, after the next job body exits its loop but before it returns.
    #[cfg(test)]
    fn set_on_exit(&self, f: impl FnOnce() + Send + 'static) {
        *self.inner.on_exit.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }
}

/// A panicking body must not leave the slot marked running, or later requests would be ignored.
struct ResetOnPanic<'a> {
    r: &'a Refresher,
    kind: JobKind,
}

impl Drop for ResetOnPanic<'_> {
    fn drop(&mut self) {
        self.r.mark_stopped(self.kind);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::testing::NullSink;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;
    use std::time::Instant;

    fn wait_until(f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    #[test]
    fn simulation_request_during_finish_is_not_lost() {
        let jobs = JobRunner::new(Arc::new(NullSink));
        let r = Refresher::new(jobs.clone());
        let runs = Arc::new(AtomicU32::new(0));
        let counter = runs.clone();
        let body: RefreshBody = Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        });

        // Park the first job after it decided to stop but before it leaves the job table.
        let (exiting_tx, exiting_rx) = mpsc::channel::<()>();
        let (proceed_tx, proceed_rx) = mpsc::channel::<()>();
        r.set_on_exit(move || {
            exiting_tx.send(()).unwrap();
            proceed_rx.recv().unwrap();
        });

        let first = r.request(JobKind::Simulation, body.clone());
        exiting_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1);

        // Lands in the window: the runner joins the finishing job.
        let joined = r.request(JobKind::Simulation, body.clone());
        assert_eq!(joined, first);
        proceed_tx.send(()).unwrap();

        assert!(wait_until(|| runs.load(Ordering::SeqCst) == 2));
        assert!(wait_until(|| jobs.list().is_empty()));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn request_if_idle_joins_a_running_pass_without_another() {
        let jobs = JobRunner::new(Arc::new(NullSink));
        let r = Refresher::new(jobs.clone());
        let runs = Arc::new(AtomicU32::new(0));
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let counter = runs.clone();
        let body: RefreshBody = Arc::new(move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
            }
            Ok(None)
        });

        let first = r.request_if_idle(JobKind::CatalogRefresh, body.clone());
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        for _ in 0..3 {
            assert_eq!(
                r.request_if_idle(JobKind::CatalogRefresh, body.clone()),
                first
            );
        }
        release_tx.send(()).unwrap();

        assert!(jobs.wait_idle(Duration::from_secs(10)));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn vanilla_request_after_settings_change_rebuilds() {
        let jobs = JobRunner::new(Arc::new(NullSink));
        let r = Refresher::new(jobs.clone());
        // `setting` models the game path; `stored` is what the cache ends up holding.
        let setting = Arc::new(Mutex::new("old.exe".to_string()));
        let stored = Arc::new(Mutex::new(String::new()));
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(Mutex::new(release_rx));
        let (s, out) = (setting.clone(), stored.clone());
        let first_pass = Arc::new(AtomicU32::new(0));
        let body: RefreshBody = Arc::new(move |_| {
            let read = s.lock().unwrap().clone();
            if first_pass.fetch_add(1, Ordering::SeqCst) == 0 {
                started_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
            }
            *out.lock().unwrap() = read;
            Ok(None)
        });

        r.request(JobKind::WarmVanilla, body.clone());
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        // Settings change while the job is computing from the old path.
        *setting.lock().unwrap() = "new.exe".to_string();
        r.request(JobKind::WarmVanilla, body.clone());
        release_tx.send(()).unwrap();

        assert!(jobs.wait_idle(Duration::from_secs(10)));
        assert_eq!(*stored.lock().unwrap(), "new.exe");
    }
}
