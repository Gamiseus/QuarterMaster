//! Background job runner: one std::thread per job, singleton dedupe, cooperative cancel.
use crate::error::{ErrorKind, QmError, QmResult};
use crate::events::{EventSink, QmEvent};
use serde::{Deserialize, Serialize};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub struct JobId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    WarmMods,
    WarmVanilla,
    Simulation,
    UpdatesRefresh,
    CatalogRefresh,
    CatalogInstall,
    ModImport,
    LauncherUpdateDownload,
    DiagnosticBundle,
}

impl JobKind {
    pub fn is_singleton(&self) -> bool {
        matches!(
            self,
            JobKind::WarmMods
                | JobKind::WarmVanilla
                | JobKind::Simulation
                | JobKind::UpdatesRefresh
                | JobKind::CatalogRefresh
        )
    }

    pub fn is_cancellable(&self) -> bool {
        matches!(
            self,
            JobKind::CatalogInstall | JobKind::ModImport | JobKind::LauncherUpdateDownload
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct JobInfo {
    pub id: JobId,
    pub kind: JobKind,
    pub progress: f32,
    pub label: String,
    pub cancelling: bool,
}

struct Entry {
    info: JobInfo,
    cancel: Arc<AtomicBool>,
    finished: bool,
}

#[derive(Default)]
struct State {
    next_id: u32,
    jobs: Vec<Entry>,
}

struct Inner {
    sink: Arc<dyn EventSink>,
    state: Mutex<State>,
    idle: Condvar,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct JobCtx {
    id: JobId,
    kind: JobKind,
    cancel: Arc<AtomicBool>,
    inner: Arc<Inner>,
}

impl JobCtx {
    pub fn progress(&self, fraction: f32, label: &str) {
        let fraction = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        {
            let mut st = lock(&self.inner.state);
            if let Some(e) = st.jobs.iter_mut().find(|e| e.info.id == self.id) {
                e.info.progress = fraction;
                e.info.label = label.to_string();
            }
        }
        self.inner.sink.emit(QmEvent::JobProgress {
            id: self.id,
            kind: self.kind,
            progress: fraction,
            label: label.to_string(),
        });
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Cheap to clone: clones share one job table.
#[derive(Clone)]
pub struct JobRunner {
    inner: Arc<Inner>,
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "job panicked".to_string()
    }
}

impl JobRunner {
    pub fn new(sink: Arc<dyn EventSink>) -> Self {
        Self {
            inner: Arc::new(Inner {
                sink,
                state: Mutex::new(State::default()),
                idle: Condvar::new(),
            }),
        }
    }

    pub fn spawn<F>(&self, kind: JobKind, f: F) -> JobId
    where
        F: FnOnce(&JobCtx) -> QmResult<Option<String>> + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let id = {
            let mut st = lock(&self.inner.state);
            if kind.is_singleton() {
                if let Some(e) = st.jobs.iter().find(|e| !e.finished && e.info.kind == kind) {
                    return e.info.id;
                }
            }
            st.next_id += 1;
            let id = JobId(st.next_id);
            st.jobs.push(Entry {
                info: JobInfo {
                    id,
                    kind,
                    progress: 0.0,
                    label: String::new(),
                    cancelling: false,
                },
                cancel: cancel.clone(),
                finished: false,
            });
            id
        };

        let ctx = JobCtx {
            id,
            kind,
            cancel,
            inner: self.inner.clone(),
        };
        let inner = self.inner.clone();
        let started = std::thread::Builder::new()
            .name(format!("qm-job-{}", id.0))
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| f(&ctx)));
                let cancelled = Self::mark_done(&inner, id);
                let (error, output) = match result {
                    Ok(Ok(out)) => {
                        if cancelled {
                            (
                                Some(QmError::new(ErrorKind::Cancelled, "Job cancelled")),
                                None,
                            )
                        } else {
                            (None, out)
                        }
                    }
                    Ok(Err(e)) => (Some(e), None),
                    Err(p) => (
                        Some(QmError::new(ErrorKind::Internal, panic_message(p.as_ref()))),
                        None,
                    ),
                };
                Self::finish(&inner, id, kind, error, output);
            });
        if let Err(e) = started {
            let err = QmError::new(ErrorKind::Internal, "Failed to start job thread")
                .with_details(e.to_string());
            Self::mark_done(&self.inner, id);
            Self::finish(&self.inner, id, kind, Some(err), None);
        }
        id
    }

    /// Marks the job finished under the lock and returns the cancel flag as of that instant.
    fn mark_done(inner: &Arc<Inner>, id: JobId) -> bool {
        let mut st = lock(&inner.state);
        match st.jobs.iter_mut().find(|e| e.info.id == id) {
            Some(e) => {
                e.finished = true;
                e.cancel.load(Ordering::SeqCst)
            }
            None => false,
        }
    }

    // Entry is already marked finished; emit, then always remove + notify so wait_idle
    // returning implies the event was delivered (and never hangs on a panicking sink).
    fn finish(
        inner: &Arc<Inner>,
        id: JobId,
        kind: JobKind,
        error: Option<QmError>,
        output: Option<String>,
    ) {
        let emitted = catch_unwind(AssertUnwindSafe(|| {
            inner.sink.emit(QmEvent::JobFinished {
                id,
                kind,
                error,
                output,
            });
        }));
        if emitted.is_err() {
            tracing::error!("event sink panicked while emitting JobFinished");
        }
        let mut st = lock(&inner.state);
        st.jobs.retain(|e| e.info.id != id);
        inner.idle.notify_all();
    }

    pub fn list(&self) -> Vec<JobInfo> {
        lock(&self.inner.state)
            .jobs
            .iter()
            .filter(|e| !e.finished)
            .map(|e| e.info.clone())
            .collect()
    }

    pub fn cancel(&self, id: JobId) -> QmResult<()> {
        let mut st = lock(&self.inner.state);
        let entry = st
            .jobs
            .iter_mut()
            .find(|e| e.info.id == id && !e.finished)
            .ok_or_else(|| QmError::new(ErrorKind::NotFound, format!("No running job {}", id.0)))?;
        if !entry.info.kind.is_cancellable() {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "This job cannot be cancelled",
            ));
        }
        entry.info.cancelling = true;
        entry.cancel.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Returns true once job `id` is no longer in the table, waiting up to `timeout`.
    pub fn wait_for(&self, id: JobId, timeout: Duration) -> bool {
        let st = lock(&self.inner.state);
        match self
            .inner
            .idle
            .wait_timeout_while(st, timeout, |s| s.jobs.iter().any(|e| e.info.id == id))
        {
            Ok((_, res)) => !res.timed_out(),
            Err(e) => !e.into_inner().1.timed_out(),
        }
    }

    /// Returns true if no jobs are running, waiting up to `timeout`.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let st = lock(&self.inner.state);
        match self
            .inner
            .idle
            .wait_timeout_while(st, timeout, |s| !s.jobs.is_empty())
        {
            Ok((_, res)) => !res.timed_out(),
            Err(e) => !e.into_inner().1.timed_out(),
        }
    }
}
