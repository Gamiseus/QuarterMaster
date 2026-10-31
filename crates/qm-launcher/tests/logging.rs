//! Launcher log naming and startup rotation.
use qm_launcher_lib::logging::{log_file_path, open_appender, rotate, ROTATE_BYTES};
use std::io::Write;

#[test]
fn log_path_is_fixed_launcher_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = log_file_path(dir.path());
    assert!(path.ends_with("launcher.log"), "{}", path.display());

    let mut appender = open_appender(dir.path()).expect("appender");
    appender.write_all(b"hello\n").expect("write");
    appender.flush().expect("flush");
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["launcher.log".to_string()]);
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "hello\n");
}

#[test]
fn rotate_moves_oversized_log_to_dot_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = log_file_path(dir.path());
    let previous = dir.path().join("launcher.log.1");
    std::fs::write(&previous, b"older").expect("seed .1");
    let f = std::fs::File::create(&path).expect("create");
    f.set_len(ROTATE_BYTES + 1).expect("set_len");
    drop(f);

    rotate(dir.path());

    assert!(!path.exists(), "launcher.log should have moved");
    let moved = std::fs::metadata(&previous).expect(".1 exists");
    assert_eq!(moved.len(), ROTATE_BYTES + 1, "older .1 is replaced");
}

#[test]
fn rotate_keeps_small_log_and_removes_daily_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = log_file_path(dir.path());
    std::fs::write(&path, b"small").expect("seed");
    let stale = dir.path().join("launcher.log.2026-10-29");
    std::fs::write(&stale, b"old daily").expect("seed daily");
    let crash = dir.path().join("launcher-crash-20261029-101010.log");
    std::fs::write(&crash, b"crash").expect("seed crash");

    rotate(dir.path());

    assert_eq!(std::fs::read(&path).expect("read"), b"small");
    assert!(!stale.exists(), "stale daily file removed");
    assert!(crash.exists(), "crash logs untouched");
    assert!(!dir.path().join("launcher.log.1").exists());
}
