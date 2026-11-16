//! Debounced file watching: maps filesystem changes to cache invalidations and tails log files.
use crate::error::{ErrorKind, QmError, QmResult};
use crate::events::{LogSource, QmEvent, Scope};
use crate::service::Service;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer_opt, Config, DebounceEventResult, Debouncer};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(300);
const MAX_LINES_PER_BATCH: usize = 200;
/// Larger jumps than this only stream their most recent part.
const MAX_TAIL_BYTES: u64 = 256 * 1024;

/// What a changed path means to the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classified {
    Invalidate(Scope),
    Log(LogSource),
    CrashDir,
    Ignore,
}

/// Stops the watcher when dropped.
pub struct WatchHandle {
    _debouncer: Debouncer<RecommendedWatcher>,
}

/// Snapshot of the roots that classification compares against.
struct Rules {
    profiles: PathBuf,
    library: PathBuf,
    saves: PathBuf,
    logs: PathBuf,
    game_saves: Option<PathBuf>,
}

impl Rules {
    fn of(svc: &Service) -> Self {
        let p = svc.paths();
        Self {
            profiles: p.profiles_dir.clone(),
            library: p.library_dir.clone(),
            saves: p.saves_dir.clone(),
            logs: p.logs_dir.clone(),
            game_saves: p.load_settings().save_dir_path,
        }
    }

    fn classify(&self, path: &Path) -> Classified {
        if let Some(rel) = under(path, &self.profiles) {
            // 1. <profile>/Mods/qm/log/...
            if rel.len() >= 5 && eq(&rel[1], "Mods") && eq(&rel[2], "qm") && eq(&rel[3], "log") {
                let rest = &rel[4..];
                if rest.len() == 1 && eq(&rest[0], "latest.log") {
                    return Classified::Log(LogSource::Runtime);
                }
                if rest.len() == 1 && eq(&rest[0], "err.log") {
                    return Classified::Log(LogSource::RuntimeErrors);
                }
                if eq(&rest[0], "crashes") {
                    return Classified::CrashDir;
                }
            }
            // 3. Deployed Mods/ (junctions and qm/ handshake files) is rebuilt by the launcher
            //    and written by the game while it runs; never a profile change.
            if rel.len() >= 2 && eq(&rel[1], "Mods") {
                return Classified::Ignore;
            }
            // The run log and the test plan mark (and their temp files) feed Health, not the
            // profile.
            if rel.len() == 2 {
                let name = rel[1].to_ascii_lowercase();
                if name.starts_with("runs.json") || name.starts_with("test-plan.json") {
                    return Classified::Invalidate(Scope::Health);
                }
            }
        }
        // 2. Data logs.
        if let Some(rel) = under(path, &self.logs) {
            if rel.len() == 1 && eq(&rel[0], "launcher.log") {
                return Classified::Log(LogSource::Launcher);
            }
            return Classified::CrashDir;
        }
        // 4.
        if under(path, &self.profiles).is_some() {
            return Classified::Invalidate(Scope::Profiles);
        }
        // 5.
        if under(path, &self.library).is_some() {
            return Classified::Invalidate(Scope::Mods);
        }
        // 6.
        if under(path, &self.saves).is_some()
            || self
                .game_saves
                .as_deref()
                .is_some_and(|g| under(path, g).is_some())
        {
            return Classified::Invalidate(Scope::Saves);
        }
        Classified::Ignore
    }
}

/// Components of `path` below `root` (non-empty), or None. Case-insensitive on Windows.
fn under(path: &Path, root: &Path) -> Option<Vec<String>> {
    let mut p = path
        .components()
        .filter(|c| !matches!(c, Component::CurDir));
    for r in root
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
    {
        let c = p.next()?;
        if !eq(
            &c.as_os_str().to_string_lossy(),
            &r.as_os_str().to_string_lossy(),
        ) {
            return None;
        }
    }
    let rest: Vec<String> = p
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!rest.is_empty()).then_some(rest)
}

fn eq(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Opaque file identity; a change means the path now names a different file (e.g. rotation).
type FileId = (u64, u64);

#[cfg(windows)]
fn file_id(file: &std::fs::File) -> Option<FileId> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is owned by `file` and valid for this call; `info` is a valid out pointer.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle() as isize), &mut info) }.ok()?;
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    Some((u64::from(info.dwVolumeSerialNumber), index))
}

#[cfg(unix)]
fn file_id(file: &std::fs::File) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    file.metadata().ok().map(|m| (m.dev(), m.ino()))
}

#[cfg(not(any(windows, unix)))]
fn file_id(_file: &std::fs::File) -> Option<FileId> {
    None
}

/// Offsets are keyed case-insensitively on Windows so path casing never replays a log.
fn tail_key(path: &Path) -> String {
    let s = path.to_string_lossy();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.into_owned()
    }
}

/// Only a missing file forgets its offset; any other open error is transient (e.g. locked).
fn forget_on_open_error(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

/// Where to resume reading: restart at 0 when the file shrank or is a different file.
fn resume_offset(prev: Option<&TailPos>, len: u64, id: Option<FileId>) -> u64 {
    match prev {
        None => 0,
        Some(p) if p.id.is_some() && id.is_some() && p.id != id => 0,
        Some(p) if len < p.offset => 0,
        Some(p) => p.offset,
    }
}

#[derive(Debug, Clone, Copy)]
struct TailPos {
    offset: u64,
    id: Option<FileId>,
}

/// Per-file read positions for log tailing.
#[derive(Default)]
struct Tail {
    pos: HashMap<String, TailPos>,
}

impl Tail {
    fn start_at_end(&mut self, path: PathBuf) {
        if let Ok(file) = std::fs::File::open(&path) {
            let offset = file.metadata().map(|m| m.len()).unwrap_or(0);
            let id = file_id(&file);
            self.pos.insert(tail_key(&path), TailPos { offset, id });
        }
    }

    /// New complete lines appended to `path` since the last read.
    fn read_new(&mut self, path: &Path) -> Vec<String> {
        let key = tail_key(path);
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                if forget_on_open_error(&e) {
                    // Deleted: a recreated file is read from its start.
                    self.pos.remove(&key);
                } else {
                    tracing::warn!("cannot open log {}: {e}", path.display());
                }
                return Vec::new();
            }
        };
        let len = match file.metadata() {
            Ok(m) => m.len(),
            Err(e) => {
                tracing::warn!("cannot stat log {}: {e}", path.display());
                return Vec::new();
            }
        };
        let id = file_id(&file);
        let mut offset = resume_offset(self.pos.get(&key), len, id);
        let mut skip_partial = false;
        if len - offset > MAX_TAIL_BYTES {
            offset = len - MAX_TAIL_BYTES;
            skip_partial = true;
        }
        let mut buf = Vec::new();
        if let Err(e) = file.seek(SeekFrom::Start(offset)) {
            tracing::warn!("cannot seek log {}: {e}", path.display());
            return Vec::new();
        }
        if let Err(e) = file.take(len - offset).read_to_end(&mut buf) {
            tracing::warn!("cannot read log {}: {e}", path.display());
            return Vec::new();
        }
        // Only complete lines; a trailing partial line waits for the next batch.
        let Some(end) = buf.iter().rposition(|b| *b == b'\n') else {
            self.pos.insert(key, TailPos { offset, id });
            return Vec::new();
        };
        let next = offset + end as u64 + 1;
        self.pos.insert(key, TailPos { offset: next, id });
        let text = String::from_utf8_lossy(&buf[..end]);
        let mut lines = text.split('\n');
        if skip_partial {
            lines.next();
        }
        lines
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect()
    }
}

fn source_index(src: LogSource) -> usize {
    match src {
        LogSource::Runtime => 0,
        LogSource::RuntimeErrors => 1,
        LogSource::Launcher => 2,
    }
}

impl Service {
    /// Classify a changed path (settings are read on every call; the watcher snapshots them).
    pub(crate) fn classify(&self, path: &Path) -> Classified {
        Rules::of(self).classify(path)
    }

    /// Watch the library, profiles, QM saves, data logs and the game save folder.
    pub fn start_watching(&self) -> QmResult<WatchHandle> {
        let rules = Rules::of(self);
        let mut tail = Tail::default();
        tail.start_at_end(rules.logs.join("launcher.log"));
        if let Ok(rd) = std::fs::read_dir(&rules.profiles) {
            for entry in rd.filter_map(Result::ok) {
                let log = entry.path().join("Mods").join("qm").join("log");
                tail.start_at_end(log.join("latest.log"));
                tail.start_at_end(log.join("err.log"));
            }
        }

        let mut roots = vec![
            rules.profiles.clone(),
            rules.library.clone(),
            rules.saves.clone(),
            rules.logs.clone(),
        ];
        for r in &roots {
            std::fs::create_dir_all(r).map_err(|e| {
                QmError::new(ErrorKind::Io, format!("Cannot create {}", r.display()))
                    .with_details(e.to_string())
            })?;
        }
        if let Some(g) = rules.game_saves.as_ref() {
            if !g.is_dir() {
                tracing::warn!(
                    "save folder {} does not exist; not watching it",
                    g.display()
                );
            } else if !roots.iter().any(|r| under(g, r).is_some() || g == r) {
                roots.push(g.clone());
            }
        }

        let svc = self.clone();
        let handler = move |res: DebounceEventResult| match res {
            Ok(events) => svc.handle_batch(&rules, &mut tail, events.iter().map(|e| &e.path)),
            Err(e) => tracing::warn!("file watcher error: {e}"),
        };
        let mut debouncer = new_debouncer_opt::<_, RecommendedWatcher>(
            Config::default()
                .with_timeout(DEBOUNCE)
                .with_batch_mode(true),
            handler,
        )
        .map_err(watch_err)?;
        for r in &roots {
            debouncer
                .watcher()
                .watch(r, RecursiveMode::Recursive)
                .map_err(watch_err)?;
        }
        Ok(WatchHandle {
            _debouncer: debouncer,
        })
    }

    /// Runs on the debouncer thread with no service locks held.
    fn handle_batch<'a>(
        &self,
        rules: &Rules,
        tail: &mut Tail,
        paths: impl Iterator<Item = &'a PathBuf>,
    ) {
        let mut scopes: Vec<Scope> = Vec::new();
        let mut logs: Vec<(PathBuf, LogSource)> = Vec::new();
        for path in paths {
            let scope = match rules.classify(path) {
                Classified::Invalidate(s) => s,
                Classified::CrashDir => Scope::Crashes,
                Classified::Log(src) => {
                    if !logs.iter().any(|(p, _)| p == path) {
                        logs.push((path.clone(), src));
                    }
                    continue;
                }
                Classified::Ignore => continue,
            };
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        let mut lines: Vec<(LogSource, String)> = Vec::new();
        for (path, src) in &logs {
            lines.extend(tail.read_new(path).into_iter().map(|l| (*src, l)));
        }
        // Per source, keep the most recent lines when a batch overflows.
        let mut total = [0usize; 3];
        for (src, _) in &lines {
            total[source_index(*src)] += 1;
        }
        let mut to_skip = total.map(|n| n.saturating_sub(MAX_LINES_PER_BATCH));
        for (i, dropped) in to_skip.iter().enumerate() {
            if *dropped > 0 {
                tracing::debug!("log tail dropped {dropped} lines for source {i} this batch");
            }
        }
        for (source, line) in lines {
            let skip = &mut to_skip[source_index(source)];
            if *skip > 0 {
                *skip -= 1;
                continue;
            }
            self.inner.sink.emit(QmEvent::LogLine { source, line });
        }
        for scope in scopes {
            self.invalidate(scope);
        }
    }
}

fn watch_err(e: notify::Error) -> QmError {
    QmError::new(ErrorKind::Io, "Cannot start the file watcher").with_details(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_not_found_forgets_the_offset() {
        let nf = std::io::Error::from(std::io::ErrorKind::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let other = std::io::Error::other("sharing violation");
        assert!(forget_on_open_error(&nf));
        assert!(!forget_on_open_error(&denied));
        assert!(!forget_on_open_error(&other));
    }

    #[test]
    fn resume_offset_resets_on_shrink_or_new_identity() {
        let p = TailPos {
            offset: 10,
            id: Some((1, 1)),
        };
        assert_eq!(resume_offset(None, 50, Some((1, 1))), 0);
        assert_eq!(resume_offset(Some(&p), 50, Some((1, 1))), 10);
        assert_eq!(resume_offset(Some(&p), 5, Some((1, 1))), 0);
        assert_eq!(resume_offset(Some(&p), 50, Some((1, 2))), 0);
        assert_eq!(resume_offset(Some(&p), 50, None), 10);
    }

    #[test]
    fn tail_keys_ignore_case_on_windows() {
        let a = tail_key(Path::new("C:/Data/Latest.LOG"));
        let b = tail_key(Path::new("c:/data/latest.log"));
        assert_eq!(a == b, cfg!(windows));
    }
}
