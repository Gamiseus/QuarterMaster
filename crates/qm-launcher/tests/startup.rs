//! A panicking service start still produces a finished startup report.
use qm_launcher_lib::commands::{run_startup, STARTUP_FAILED};
use qm_service::StartupReport;

#[test]
fn panicking_start_reports_failure() {
    let report = run_startup(|| panic!("boom"));
    assert_eq!(report.recovered.as_deref(), Some(STARTUP_FAILED));
    assert!(report.attached_pid.is_none());
    assert!(report.watch.is_none());
}

#[test]
fn normal_start_passes_through() {
    let report = run_startup(|| StartupReport {
        recovered: Some("ok".into()),
        attached_pid: Some(7),
        watch: None,
    });
    assert_eq!(report.recovered.as_deref(), Some("ok"));
    assert_eq!(report.attached_pid, Some(7));
}
