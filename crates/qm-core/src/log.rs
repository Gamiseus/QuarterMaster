use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use jiff::Zoned;

// Exports for convenience
pub use log::{debug, error, info, trace, warn, LevelFilter};
use log::{Level, Log, Metadata, Record, SetLoggerError};

static LOGGER: OnceLock<QMLogger> = OnceLock::new();

struct QMLogger {
    use_console: bool,
    latest_file: RwLock<File>,
    err_file: RwLock<File>,
    log_dir: PathBuf,
    latest_path: String,
}

impl Log for QMLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        let args = format!("{}", record.args());
        let is_game = args.starts_with("[G]");
        let tag = if is_game { "" } else { " [QM]" };

        let now = Zoned::now();
        let time_str = now.strftime("%H:%M:%S");

        let line = format!("[{time_str}] {} -{tag} {args}\n", record.level());

        if self.enabled(record.metadata()) && self.use_console {
            print!("{line}");
        }

        // 1. Write to latest.log
        if let Ok(mut file) = self.latest_file.write() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }

        // 2. If Error level, also write to err.log
        if record.level() == Level::Error {
            if let Ok(mut err_file) = self.err_file.write() {
                let _ = err_file.write_all(line.as_bytes());
                let _ = err_file.flush();
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.latest_file.write() {
            let _ = file.flush();
        }
        if let Ok(mut err_file) = self.err_file.write() {
            let _ = err_file.flush();
        }
    }
}

pub fn init(log_dir: &Path) -> Result<(), SetLoggerError> {
    if !log_dir.is_dir() {
        let _ = fs::create_dir_all(log_dir);
    }

    let latest_path = log_dir.join("latest.log");
    let prev_path = log_dir.join("prev.log");
    let err_path = log_dir.join("err.log");
    let prev_err_path = log_dir.join("prev_err.log");

    // Rotate latest.log -> prev.log
    if latest_path.is_file() {
        let _ = fs::remove_file(&prev_path);
        let _ = fs::rename(&latest_path, &prev_path);
    }

    // Rotate err.log -> prev_err.log
    if err_path.is_file() {
        let _ = fs::remove_file(&prev_err_path);
        let _ = fs::rename(&err_path, &prev_err_path);
    }

    let latest_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&latest_path)
        .unwrap_or_else(|e| panic!("Failed to create latest.log at {latest_path:?}: {e}"));

    let err_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&err_path)
        .unwrap_or_else(|e| panic!("Failed to create err.log at {err_path:?}: {e}"));

    let logger = QMLogger {
        use_console: true,
        latest_file: RwLock::new(latest_file),
        err_file: RwLock::new(err_file),
        log_dir: log_dir.to_path_buf(),
        latest_path: String::from(dunce::canonicalize(&latest_path).unwrap_or(latest_path).to_str().unwrap()),
    };

    log::set_logger(LOGGER.get_or_init(|| logger)).map(|_| log::set_max_level(LevelFilter::Info))
}

pub fn get_log_path() -> Option<String> {
    LOGGER.get().map(|x| x.latest_path.clone())
}

/// Record a crash in the crashes/ subdirectory and prune oldest entries if > 10.
pub fn record_crash(panic_info: &str) {
    let Some(logger) = LOGGER.get() else { return };
    logger.flush();

    let crashes_dir = logger.log_dir.join("crashes");
    if !crashes_dir.is_dir() {
        let _ = fs::create_dir_all(&crashes_dir);
    }

    let now = Zoned::now();
    let timestamp = now.strftime("%Y.%m.%d-%H.%M.%S");
    let crash_file_path = crashes_dir.join(format!("crash-{timestamp}.log"));

    // Read whatever was logged in latest.log so far
    let latest_path = logger.log_dir.join("latest.log");
    let session_log = fs::read_to_string(&latest_path).unwrap_or_else(|_| "<could not read session log>".to_string());

    let report = format!(
        "================================================================================\n\
         QUARTERMASTER (QM) CRASH REPORT\n\
         Timestamp: {timestamp}\n\
         ================================================================================\n\
         CRASH / PANIC DETAILS:\n\
         {panic_info}\n\n\
         ================================================================================\n\
         SESSION LOG (latest.log):\n\
         ================================================================================\n\
         {session_log}\n"
    );

    let _ = fs::write(&crash_file_path, report);

    // Prune crashes folder to keep at most 10 logs
    prune_crashes(&crashes_dir, 10);
}

/// Remove oldest crash files until at most `max_count` remain.
fn prune_crashes(crashes_dir: &Path, max_count: usize) {
    let Ok(entries) = fs::read_dir(crashes_dir) else { return };

    let mut crash_files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |ext| ext == "log" || ext == "txt") {
            let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            crash_files.push((path, mtime));
        }
    }

    if crash_files.len() > max_count {
        // Sort oldest first
        crash_files.sort_by_key(|(_, time)| *time);
        let delete_count = crash_files.len() - max_count;
        for (path, _) in crash_files.into_iter().take(delete_count) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Record clean shutdown in latest.log
pub fn record_clean_shutdown() {
    info!("Clean shutdown complete.");
    if let Some(logger) = LOGGER.get() {
        logger.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_prune_crashes_caps_at_ten() {
        let dir = tempdir().unwrap();
        let crashes_dir = dir.path().join("crashes");
        fs::create_dir_all(&crashes_dir).unwrap();

        // Create 15 dummy crash files with spaced timestamps
        for i in 1..=15 {
            let file_path = crashes_dir.join(format!("crash-2026.10.21-00.00.{:02}.log", i));
            fs::write(&file_path, format!("Crash dump #{}", i)).unwrap();
        }

        let initial_count = fs::read_dir(&crashes_dir).unwrap().count();
        assert_eq!(initial_count, 15);

        // Prune to 10
        prune_crashes(&crashes_dir, 10);

        let final_count = fs::read_dir(&crashes_dir).unwrap().count();
        assert_eq!(final_count, 10, "Crashes directory should be capped at 10 files");
    }
}
