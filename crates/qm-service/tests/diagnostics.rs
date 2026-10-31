#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use qm_service::testing::{FakeProbe, RecordingSink};
use qm_service::{ErrorKind, JobId, LogSource, QmEvent, Service};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct Ctx {
    fx: support::Fixture,
    sink: Arc<RecordingSink>,
    svc: Service,
}

fn ctx() -> Ctx {
    let fx = support::Fixture::new();
    let sink = Arc::new(RecordingSink::new());
    let svc = Service::new(fx.paths.clone(), sink.clone(), Arc::new(FakeProbe::new()));
    Ctx { fx, sink, svc }
}

fn write_lines(path: &std::path::Path, n: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text: String = (0..n).map(|i| format!("line {i}\n")).collect();
    std::fs::write(path, text).unwrap();
}

fn launcher_log(c: &Ctx) -> PathBuf {
    c.fx.paths.logs_dir.join("launcher.log")
}

fn finished_output(c: &Ctx, id: JobId) -> Option<String> {
    assert!(c.svc.jobs().wait_idle(Duration::from_secs(60)));
    c.sink.events().into_iter().find_map(|e| match e {
        QmEvent::JobFinished {
            id: fid,
            error,
            output,
            ..
        } if fid == id => {
            assert!(error.is_none(), "job failed: {error:?}");
            output
        }
        _ => None,
    })
}

#[test]
fn logs_tail_returns_last_n_lines() {
    let c = ctx();
    write_lines(&launcher_log(&c), 10);
    assert_eq!(
        c.svc.logs_tail(LogSource::Launcher, 3).unwrap(),
        vec!["line 7", "line 8", "line 9"]
    );

    let runtime =
        c.fx.paths
            .profiles_dir
            .join("default")
            .join("Mods")
            .join("qm")
            .join("log");
    write_lines(&runtime.join("latest.log"), 5);
    std::fs::write(runtime.join("err.log"), "boom\r\n").unwrap();
    assert_eq!(
        c.svc.logs_tail(LogSource::Runtime, 2).unwrap(),
        vec!["line 3", "line 4"]
    );
    assert_eq!(
        c.svc.logs_tail(LogSource::RuntimeErrors, 10).unwrap(),
        vec!["boom"]
    );
}

#[test]
fn logs_tail_caps_at_1000() {
    let c = ctx();
    write_lines(&launcher_log(&c), 1500);
    let lines = c.svc.logs_tail(LogSource::Launcher, 5000).unwrap();
    assert_eq!(lines.len(), 1000);
    assert_eq!(lines.first().unwrap(), "line 500");
    assert_eq!(lines.last().unwrap(), "line 1499");
}

#[test]
fn logs_tail_reads_across_chunks_of_large_log() {
    let c = ctx();
    write_lines(&launcher_log(&c), 30_000);
    let lines = c.svc.logs_tail(LogSource::Launcher, 1000).unwrap();
    assert_eq!(lines.len(), 1000);
    assert_eq!(lines.first().unwrap(), "line 29000");
    assert_eq!(lines.last().unwrap(), "line 29999");
    assert!(c.svc.logs_tail(LogSource::Launcher, 0).unwrap().is_empty());
}

#[test]
fn bundle_job_outputs_written_path() {
    let c = ctx();
    let id = c.svc.diagnostic_bundle_build().unwrap();
    let out = PathBuf::from(finished_output(&c, id).expect("bundle path"));
    assert!(out.is_file());
    assert_eq!(
        out.parent().unwrap(),
        c.fx.paths.cache_dir.join("diagnostics")
    );
    let name = out.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("bundle-") && name.ends_with(".json"),
        "{name}"
    );
    assert_eq!(name.len(), "bundle-yyyymmdd-hhmmss.json".len());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert!(v.get("qm_version").is_some());
    assert!(v["launcher_log"].is_array());
}

#[test]
fn bundle_redacts_user_paths_in_launcher_log() {
    let c = ctx();
    std::fs::create_dir_all(&c.fx.paths.logs_dir).unwrap();
    std::fs::write(
        launcher_log(&c),
        "opened C:\\Users\\alice\\AppData\\Roaming\\mnb\\saveMNB3.lua\nplain line\n",
    )
    .unwrap();
    let id = c.svc.diagnostic_bundle_build().unwrap();
    let out = finished_output(&c, id).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    let lines: Vec<String> = v["launcher_log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        lines,
        vec![
            "opened C:\\Users\\[REDACTED]\\AppData\\Roaming\\mnb\\saveMNB3.lua",
            "plain line"
        ]
    );
}

#[test]
fn crash_disable_suspect_missing_crash_is_not_found() {
    let c = ctx();
    let err = c.svc.crash_disable_suspect("no-such-crash").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn bundles_never_overwrite_each_other() {
    let c = ctx();
    let a = c.svc.diagnostic_bundle_build().unwrap();
    let pa = finished_output(&c, a).unwrap();
    let b = c.svc.diagnostic_bundle_build().unwrap();
    let pb = finished_output(&c, b).unwrap();
    assert_ne!(pa, pb);
    assert!(PathBuf::from(&pa).is_file() && PathBuf::from(&pb).is_file());
}
