//! File logging and the panic hook.
use std::backtrace::Backtrace;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, InitError, RollingFileAppender, Rotation};

const LOG_FILE: &str = "launcher.log";
/// Size at which the log is moved to `launcher.log.1` at startup.
pub const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

/// The fixed launcher log path that qm-service reads and watches.
pub fn log_file_path(logs_dir: &Path) -> PathBuf {
    logs_dir.join(LOG_FILE)
}

/// Startup housekeeping: moves an oversized `launcher.log` to `launcher.log.1` (replacing the
/// older one) and deletes daily `launcher.log.20*` files left by the old naming.
pub fn rotate(logs_dir: &Path) {
    let current = log_file_path(logs_dir);
    let oversized = std::fs::metadata(&current).is_ok_and(|m| m.len() > ROTATE_BYTES);
    if oversized {
        let previous = logs_dir.join(format!("{LOG_FILE}.1"));
        if let Err(e) = std::fs::rename(&current, &previous) {
            eprintln!("could not rotate {}: {e}", current.display());
        }
    }
    let Ok(entries) = std::fs::read_dir(logs_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("launcher.log.20") {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                eprintln!("could not delete {}: {e}", entry.path().display());
            }
        }
    }
}

/// Opens the non-rotating appender that writes exactly `log_file_path(logs_dir)`.
pub fn open_appender(logs_dir: &Path) -> Result<RollingFileAppender, InitError> {
    Builder::new()
        .rotation(Rotation::NEVER)
        .filename_prefix(LOG_FILE)
        .build(logs_dir)
}

/// Appends to the fixed `<logs_dir>/launcher.log`, rotated once at startup. Keep the guard alive.
pub fn init(logs_dir: &Path) -> WorkerGuard {
    if let Err(e) = std::fs::create_dir_all(logs_dir) {
        eprintln!("could not create {}: {e}", logs_dir.display());
    }
    rotate(logs_dir);
    let (writer, guard) = match open_appender(logs_dir) {
        Ok(a) => tracing_appender::non_blocking(a),
        Err(e) => {
            eprintln!("launcher log unavailable: {e}");
            tracing_appender::non_blocking(std::io::sink())
        }
    };
    let installed = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .try_init();
    if let Err(e) = installed {
        eprintln!("tracing subscriber not installed: {e}");
    }
    guard
}

/// Writes `launcher-crash-<yyyymmdd-hhmmss>.log` with the panic message and backtrace, then
/// runs the previous hook.
pub fn install_panic_hook(logs_dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
        let backtrace = Backtrace::force_capture();
        let report = format!("{info}\n\nBacktrace:\n{backtrace}\n");
        tracing::error!("panic: {info}");
        let path = logs_dir.join(format!("launcher-crash-{stamp}.log"));
        let written = std::fs::create_dir_all(&logs_dir)
            .and_then(|()| std::fs::File::create(&path))
            .and_then(|mut f| f.write_all(report.as_bytes()));
        if let Err(e) = written {
            eprintln!("could not write {}: {e}", path.display());
        }
        previous(info);
    }));
}
