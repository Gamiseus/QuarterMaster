#![allow(clippy::unwrap_used)]
mod support;
use qm_service::testing::{classify, FakeProbe, RecordingSink};
use qm_service::{Classified, JobKind, LogSource, QmEvent, Scope, Service};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_secs(1);

fn svc() -> (support::Fixture, Arc<RecordingSink>, Service) {
    let fx = support::Fixture::new();
    let sink = Arc::new(RecordingSink::new());
    let s = Service::new(fx.paths.clone(), sink.clone(), Arc::new(FakeProbe::new()));
    (fx, sink, s)
}

fn log_dir(fx: &support::Fixture) -> PathBuf {
    let d = fx
        .paths
        .profiles_dir
        .join("default")
        .join("Mods")
        .join("qm")
        .join("log");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wait_until(sink: &RecordingSink, pred: impl Fn(&[QmEvent]) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if pred(&sink.events()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    pred(&sink.events())
}

fn log_lines(events: &[QmEvent], want: LogSource) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            QmEvent::LogLine { source, line } if *source == want => Some(line.clone()),
            _ => None,
        })
        .collect()
}

fn invalidated(events: &[QmEvent], want: Scope) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, QmEvent::Invalidated { scope } if *scope == want))
        .count()
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn classify_rules_table() {
    let (fx, _sink, s) = svc();
    let p = &fx.paths;
    let log = p
        .profiles_dir
        .join("default")
        .join("Mods")
        .join("qm")
        .join("log");
    // 1. Runtime logs and crash dumps under the profile's qm/log.
    assert_eq!(
        classify(&s, &log.join("latest.log")),
        Classified::Log(LogSource::Runtime)
    );
    assert_eq!(
        classify(&s, &log.join("err.log")),
        Classified::Log(LogSource::RuntimeErrors)
    );
    assert_eq!(
        classify(&s, &log.join("crashes").join("c1.txt")),
        Classified::CrashDir
    );
    // 2. Launcher log and other data logs.
    assert_eq!(
        classify(&s, &p.logs_dir.join("launcher.log")),
        Classified::Log(LogSource::Launcher)
    );
    assert_eq!(
        classify(&s, &p.logs_dir.join("crash-1.json")),
        Classified::CrashDir
    );
    // 3. Deployed Mods/ content is rebuilt by the launcher.
    let mods = p.profiles_dir.join("default").join("Mods");
    assert_eq!(
        classify(&s, &mods.join("some_mod").join("main.lua")),
        Classified::Ignore
    );
    // Widened rule 3: qm/ handshake files and other qm/log files are written by the game.
    assert_eq!(
        classify(&s, &mods.join("qm").join("launch-config.json")),
        Classified::Ignore
    );
    assert_eq!(classify(&s, &log.join("prev.log")), Classified::Ignore);
    if cfg!(windows) {
        let shouty = PathBuf::from(log.join("Latest.LOG").to_string_lossy().to_uppercase());
        assert_eq!(classify(&s, &shouty), Classified::Log(LogSource::Runtime));
    }
    // 4. Profile files.
    assert_eq!(
        classify(&s, &p.profiles_dir.join("default").join("profile.json")),
        Classified::Invalidate(Scope::Profiles)
    );
    // 5. Library.
    assert_eq!(
        classify(&s, &p.library_dir.join("m").join("1.0.0").join("mod.json")),
        Classified::Invalidate(Scope::Mods)
    );
    // 6. QM saves and the game's save folder.
    assert_eq!(
        classify(&s, &p.saves_dir.join("slot.sav")),
        Classified::Invalidate(Scope::Saves)
    );
    assert_eq!(
        classify(&s, &fx.save_dir.join("save1.dat")),
        Classified::Invalidate(Scope::Saves)
    );
    // 7. Everything else.
    assert_eq!(classify(&s, &p.cache_dir.join("x.bin")), Classified::Ignore);
    assert_eq!(
        classify(&s, &fx.game_dir.join("mnb.exe")),
        Classified::Ignore
    );
}

#[test]
fn game_log_writes_do_not_invalidate_profiles() {
    let (fx, sink, s) = svc();
    let latest = log_dir(&fx).join("latest.log");
    std::fs::write(&latest, "old line\n").unwrap();
    let _w = s.start_watching().unwrap();
    for i in 0..5 {
        append(&latest, &format!("line {i}\n"));
    }
    assert!(
        wait_until(&sink, |e| log_lines(e, LogSource::Runtime).len() >= 5),
        "events: {:?}",
        sink.events()
    );
    std::thread::sleep(SETTLE);
    let ev = sink.events();
    let lines = log_lines(&ev, LogSource::Runtime);
    assert_eq!(
        lines,
        (0..5).map(|i| format!("line {i}")).collect::<Vec<_>>()
    );
    assert_eq!(invalidated(&ev, Scope::Profiles), 0, "{ev:?}");
    assert_eq!(invalidated(&ev, Scope::LoadOrder), 0, "{ev:?}");
}

#[test]
fn library_change_emits_mods_once_per_batch() {
    let (fx, sink, s) = svc();
    // Occupy WarmMods so the refill (which also announces Mods) cannot run while we count.
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    s.jobs().spawn(JobKind::WarmMods, move |_| {
        let _ = rx.recv();
        Ok(None)
    });
    // Build the mod outside the library, then move it in with one rename so the change is a
    // single filesystem event regardless of disk speed.
    let staging = fx.tmp.path().join("staging").join("watched_mod");
    let version = staging.join("1.0.0");
    std::fs::create_dir_all(&version).unwrap();
    for i in 0..10 {
        std::fs::write(version.join(format!("f{i}.lua")), format!("-- {i}\n")).unwrap();
    }
    std::fs::create_dir_all(&fx.paths.library_dir).unwrap();
    let _w = s.start_watching().unwrap();
    std::fs::rename(&staging, fx.paths.library_dir.join("watched_mod")).unwrap();
    assert!(
        wait_until(&sink, |e| invalidated(e, Scope::Mods) >= 1),
        "events: {:?}",
        sink.events()
    );
    std::thread::sleep(SETTLE);
    let ev = sink.events();
    assert_eq!(invalidated(&ev, Scope::Mods), 1, "{ev:?}");
    drop(tx);
    assert!(s.jobs().wait_idle(Duration::from_secs(20)));
}

#[test]
fn log_truncation_resets_offset() {
    let (fx, sink, s) = svc();
    let err = log_dir(&fx).join("err.log");
    std::fs::write(&err, "before watching 1\nbefore watching 2\n").unwrap();
    let _w = s.start_watching().unwrap();
    append(&err, "after start\n");
    assert!(
        wait_until(&sink, |e| !log_lines(e, LogSource::RuntimeErrors)
            .is_empty()),
        "events: {:?}",
        sink.events()
    );
    // Shorter than the current offset: the tail restarts from the beginning.
    std::fs::write(&err, "fresh\n").unwrap();
    assert!(
        wait_until(&sink, |e| log_lines(e, LogSource::RuntimeErrors).len() >= 2),
        "events: {:?}",
        sink.events()
    );
    std::thread::sleep(SETTLE);
    let lines = log_lines(&sink.events(), LogSource::RuntimeErrors);
    assert_eq!(lines, vec!["after start".to_string(), "fresh".to_string()]);
}

#[test]
fn log_rotation_to_longer_file_restarts_tail() {
    let (fx, sink, s) = svc();
    let dir = log_dir(&fx);
    let latest = dir.join("latest.log");
    std::fs::write(&latest, "old 1\n").unwrap();
    let _w = s.start_watching().unwrap();
    // qm-core rotates latest.log -> prev.log; the new file is longer than the old offset.
    let staged = fx.tmp.path().join("new-latest.log");
    std::fs::write(&staged, "new line 1\nnew line 2\n").unwrap();
    std::fs::rename(&latest, dir.join("prev.log")).unwrap();
    std::fs::rename(&staged, &latest).unwrap();
    assert!(
        wait_until(&sink, |e| log_lines(e, LogSource::Runtime).len() >= 2),
        "events: {:?}",
        sink.events()
    );
    std::thread::sleep(SETTLE);
    assert_eq!(
        log_lines(&sink.events(), LogSource::Runtime),
        vec!["new line 1".to_string(), "new line 2".to_string()]
    );
}

#[test]
fn dropping_handle_stops_events() {
    let (fx, sink, s) = svc();
    let latest = log_dir(&fx).join("latest.log");
    std::fs::write(&latest, "").unwrap();
    let w = s.start_watching().unwrap();
    append(&latest, "seen\n");
    assert!(
        wait_until(&sink, |e| !log_lines(e, LogSource::Runtime).is_empty()),
        "events: {:?}",
        sink.events()
    );
    drop(w);
    std::thread::sleep(Duration::from_millis(100));
    let before = sink.events().len();
    append(&latest, "unseen\n");
    std::fs::write(fx.paths.library_dir.join("after-drop.txt"), "x").unwrap();
    std::thread::sleep(SETTLE);
    let ev = sink.events();
    assert_eq!(ev.len(), before, "{ev:?}");
}
