use qm_service::testing::RecordingSink;
use qm_service::{ErrorKind, JobKind, JobRunner, QmEvent};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

fn setup() -> (Arc<RecordingSink>, JobRunner) {
    let sink = Arc::new(RecordingSink::new());
    let runner = JobRunner::new(sink.clone());
    (sink, runner)
}

fn finished(sink: &RecordingSink) -> Vec<QmEvent> {
    sink.events()
        .into_iter()
        .filter(|e| matches!(e, QmEvent::JobFinished { .. }))
        .collect()
}

#[test]
fn job_emits_progress_and_finished_with_output() {
    let (sink, runner) = setup();
    runner.spawn(JobKind::WarmMods, |ctx| {
        ctx.progress(0.5, "half");
        Ok(Some("done".into()))
    });
    assert!(runner.wait_idle(Duration::from_secs(5)));
    let ev = sink.events();
    assert_eq!(ev.len(), 2);
    match &ev[0] {
        QmEvent::JobProgress {
            progress, label, ..
        } => {
            assert_eq!(*progress, 0.5);
            assert_eq!(label, "half");
        }
        o => panic!("unexpected {o:?}"),
    }
    match &ev[1] {
        QmEvent::JobFinished { error, output, .. } => {
            assert!(error.is_none());
            assert_eq!(output.as_deref(), Some("done"));
        }
        o => panic!("unexpected {o:?}"),
    }
}

#[test]
fn progress_is_clamped() {
    let (sink, runner) = setup();
    runner.spawn(JobKind::WarmMods, |ctx| {
        ctx.progress(7.0, "hi");
        ctx.progress(-1.0, "lo");
        Ok(None)
    });
    assert!(runner.wait_idle(Duration::from_secs(5)));
    let p: Vec<f32> = sink
        .events()
        .into_iter()
        .filter_map(|e| {
            if let QmEvent::JobProgress { progress, .. } = e {
                Some(progress)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(p, vec![1.0, 0.0]);
}

#[test]
fn singleton_jobs_are_deduplicated() {
    let (sink, runner) = setup();
    let (tx, rx) = mpsc::channel::<()>();
    let a = runner.spawn(JobKind::UpdatesRefresh, move |_| {
        rx.recv().ok();
        Ok(None)
    });
    let b = runner.spawn(JobKind::UpdatesRefresh, |_| Ok(None));
    assert_eq!(a, b);
    assert_eq!(runner.list().len(), 1);
    tx.send(()).unwrap();
    assert!(runner.wait_idle(Duration::from_secs(5)));
    assert_eq!(finished(&sink).len(), 1);
}

#[test]
fn cancel_sets_flag_and_reports_cancelled() {
    let (sink, runner) = setup();
    let (started_tx, started_rx) = mpsc::channel::<()>();
    let id = runner.spawn(JobKind::ModImport, move |ctx| {
        started_tx.send(()).ok();
        while !ctx.is_cancelled() {
            std::thread::yield_now();
        }
        Ok(Some("ignored".into()))
    });
    started_rx.recv().unwrap();
    runner.cancel(id).unwrap();
    assert!(runner.wait_idle(Duration::from_secs(5)));
    match &finished(&sink)[0] {
        QmEvent::JobFinished { error, .. } => {
            assert_eq!(error.as_ref().unwrap().kind, ErrorKind::Cancelled)
        }
        _ => unreachable!(),
    }
}

#[test]
fn cancel_rejects_non_cancellable_and_unknown() {
    let (_sink, runner) = setup();
    let (tx, rx) = mpsc::channel::<()>();
    let id = runner.spawn(JobKind::Simulation, move |_| {
        rx.recv().ok();
        Ok(None)
    });
    assert_eq!(runner.cancel(id).unwrap_err().kind, ErrorKind::InvalidInput);
    assert_eq!(
        runner.cancel(qm_service::JobId(9999)).unwrap_err().kind,
        ErrorKind::NotFound
    );
    tx.send(()).unwrap();
    assert!(runner.wait_idle(Duration::from_secs(5)));
}

#[test]
fn panicking_job_reports_internal() {
    let (sink, runner) = setup();
    runner.spawn(JobKind::DiagnosticBundle, |_| panic!("kaboom"));
    assert!(runner.wait_idle(Duration::from_secs(5)));
    match &finished(&sink)[0] {
        QmEvent::JobFinished { error, .. } => {
            let e = error.as_ref().unwrap();
            assert_eq!(e.kind, ErrorKind::Internal);
            assert!(e.message.contains("kaboom"));
        }
        _ => unreachable!(),
    }
    sink.clear();
    runner.spawn(JobKind::DiagnosticBundle, |_| Ok(Some("ok".into())));
    assert!(runner.wait_idle(Duration::from_secs(5)));
    assert_eq!(finished(&sink).len(), 1);
}

#[test]
fn singleton_respawn_from_finished_handler_runs() {
    use qm_service::{EventSink, JobId};
    use std::sync::{Mutex, OnceLock};

    struct Resp {
        runner: Arc<OnceLock<Arc<JobRunner>>>,
        finished: Mutex<Vec<JobId>>,
        spawned: Mutex<Option<JobId>>,
    }
    impl EventSink for Resp {
        fn emit(&self, event: QmEvent) {
            if let QmEvent::JobFinished {
                id,
                kind: JobKind::UpdatesRefresh,
                ..
            } = event
            {
                let first = self.finished.lock().unwrap().is_empty();
                self.finished.lock().unwrap().push(id);
                if first {
                    let r = self.runner.get().unwrap().clone();
                    let nid = r.spawn(JobKind::UpdatesRefresh, |_| Ok(None));
                    *self.spawned.lock().unwrap() = Some(nid);
                }
            }
        }
    }

    let cell = Arc::new(OnceLock::new());
    let sink = Arc::new(Resp {
        runner: cell.clone(),
        finished: Mutex::new(vec![]),
        spawned: Mutex::new(None),
    });
    let runner = Arc::new(JobRunner::new(sink.clone()));
    cell.set(runner.clone()).ok();
    let first = runner.spawn(JobKind::UpdatesRefresh, |_| Ok(None));
    assert!(runner.wait_idle(Duration::from_secs(5)));
    let second = sink.spawned.lock().unwrap().unwrap();
    assert_ne!(first, second);
    assert_eq!(sink.finished.lock().unwrap().len(), 2);
}

#[test]
fn panicking_sink_does_not_hang_wait_idle() {
    use qm_service::EventSink;
    struct Bad;
    impl EventSink for Bad {
        fn emit(&self, _e: QmEvent) {
            panic!("sink boom");
        }
    }
    let runner = JobRunner::new(Arc::new(Bad));
    runner.spawn(JobKind::DiagnosticBundle, |_| Ok(None));
    assert!(runner.wait_idle(Duration::from_secs(5)));
    assert!(runner.list().is_empty());
}

#[test]
fn event_serde_shape() {
    use qm_service::Scope;
    let v = serde_json::to_value(QmEvent::Invalidated {
        scope: Scope::LoadOrder,
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({"type":"invalidated","scope":"load_order"})
    );
}
