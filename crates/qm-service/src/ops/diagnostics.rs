//! Health, crashes, logs, the diagnostic bundle, and the DLL repair action.
use crate::cache::Loadable;
use crate::dto::HealthReport;
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::{LogSource, Scope};
use crate::jobs::{JobId, JobKind};
use crate::service::Service;
use qm_core::handshake::CrashReport;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const MAX_LOG_LINES: u32 = 1000;
const BUNDLE_LOG_LINES: u32 = 500;
const REDACTED_USER: &str = "[REDACTED]";

impl Service {
    /// Cached health; a miss starts one background refresh and returns `Loading`.
    pub fn health_get(&self) -> Loadable<HealthReport> {
        match self.inner.caches.health.get() {
            Some(h) => Loadable::Ready(h),
            None => {
                self.schedule_health_refresh();
                Loadable::Loading
            }
        }
    }

    pub fn crashes_list(&self) -> Vec<CrashReport> {
        self.paths().list_crash_reports()
    }

    /// The last `lines` lines (at most 1000) of a log; a missing log is empty.
    pub fn logs_tail(&self, source: LogSource, lines: u32) -> QmResult<Vec<String>> {
        let path = self.log_path(source);
        match tail_lines(&path, lines.min(MAX_LOG_LINES)) {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(
                QmError::new(ErrorKind::Io, "The log file could not be read.")
                    .with_details(format!("{}: {e}", path.display())),
            ),
        }
    }

    /// Write `<data>/cache/diagnostics/bundle-<yyyymmdd-hhmmss>.json`; the job outputs its path.
    pub fn diagnostic_bundle_build(&self) -> QmResult<JobId> {
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::DiagnosticBundle, move |ctx| {
            ctx.progress(0.1, "Collecting diagnostics");
            let mut bundle = svc
                .paths()
                .build_redacted_diagnostic_bundle()
                .or_kind(ErrorKind::Io)?;
            ctx.progress(0.7, "Adding launcher log");
            let log = svc.logs_tail(LogSource::Launcher, BUNDLE_LOG_LINES)?;
            match bundle.as_object_mut() {
                Some(obj) => {
                    obj.insert("launcher_log".into(), serde_json::json!(log));
                }
                None => {
                    return Err(QmError::new(
                        ErrorKind::Internal,
                        "Diagnostic bundle is not a JSON object.",
                    ))
                }
            }
            let dir = svc.paths().cache_dir.join("diagnostics");
            std::fs::create_dir_all(&dir).or_kind(ErrorKind::Io)?;
            let json = serde_json::to_string_pretty(&bundle)
                .map_err(|e| QmError::new(ErrorKind::Internal, e.to_string()))?;
            let profile_leaf = std::env::var_os("USERPROFILE").and_then(|p| {
                Path::new(&p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            });
            let username = std::env::var("USERNAME").ok();
            let json = redact_bundle_text(&json, profile_leaf.as_deref(), username.as_deref());
            let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
            let (path, mut file) = create_unique(&dir, &format!("bundle-{stamp}"), "json")?;
            file.write_all(json.as_bytes()).or_kind(ErrorKind::Io)?;
            Ok(Some(path.display().to_string()))
        }))
    }

    /// Disable the mod a crash report blames, in the active profile.
    pub fn crash_disable_suspect(&self, crash_id: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        let crash = self
            .crashes_list()
            .into_iter()
            .find(|c| c.id == crash_id)
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("Crash report '{crash_id}' was not found."),
                )
            })?;
        let mod_id = crash
            .suspected_mod_id
            .filter(|m| !m.is_empty())
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("Crash report '{crash_id}' does not name a suspected mod."),
                )
            })?;
        self.mod_set_enabled(&mod_id, false)?;
        Ok(())
    }

    /// Reinstall the QuarterMaster winmm.dll; returns the installed hash.
    pub fn dll_repair(&self) -> QmResult<String> {
        self.ensure_not_running()?;
        let result = self.tx_repair_dll();
        self.invalidate(Scope::Health);
        result
    }

    fn log_path(&self, source: LogSource) -> PathBuf {
        let p = self.paths();
        let runtime = || {
            let active = p.load_settings().active_profile_id;
            p.profiles_dir
                .join(active)
                .join("Mods")
                .join("qm")
                .join("log")
        };
        match source {
            LogSource::Runtime => runtime().join("latest.log"),
            LogSource::RuntimeErrors => runtime().join("err.log"),
            LogSource::Launcher => p.logs_dir.join("launcher.log"),
        }
    }

    /// Start a health refresh thread unless one is running; a running one does one more pass.
    fn schedule_health_refresh(&self) {
        {
            let mut slot = self
                .inner
                .health_refresh
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            slot.dirty = true;
            if slot.running {
                return;
            }
            slot.running = true;
        }
        let svc = self.clone();
        let started = std::thread::Builder::new()
            .name("qm-health".into())
            .spawn(move || svc.health_refresh_loop());
        if let Err(e) = started {
            tracing::error!("could not start health refresh thread: {e}");
            self.inner
                .health_refresh
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .running = false;
        }
    }

    fn health_refresh_loop(&self) {
        let _reset = HealthRunningGuard(self);
        loop {
            {
                let mut slot = self
                    .inner
                    .health_refresh
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if !slot.dirty {
                    slot.running = false;
                    return;
                }
                slot.dirty = false;
            }
            let gen = self.generations().health;
            let raw = self.paths().check_system_health();
            let report = match serde_json::from_value::<HealthReport>(raw) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("health report did not parse: {e}");
                    continue;
                }
            };
            let stored = self.with_gens(|g| {
                if g.health != gen {
                    return false;
                }
                self.inner.caches.health.set(report);
                true
            });
            if stored {
                self.notify(Scope::Health);
            }
        }
    }
}

/// Clears the health slot's `running` flag if the loop panics, so later misses still refresh.
struct HealthRunningGuard<'a>(&'a Service);

impl Drop for HealthRunningGuard<'_> {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            return;
        }
        self.0
            .inner
            .health_refresh
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .running = false;
    }
}

/// Redact a serialized diagnostic bundle: the `%USERPROFILE%` leaf folder name and the
/// `%USERNAME%` wherever they follow a `Users` path segment, any other `X:\Users\<name>`, and
/// Steam ids in `userdata\<digits>`. Matching is ASCII case-insensitive and accepts `\`, `/`
/// and JSON-escaped `\\` separators, mixed freely.
pub(crate) fn redact_bundle_text(
    text: &str,
    profile_leaf: Option<&str>,
    username: Option<&str>,
) -> String {
    let mut out = text.to_string();
    for name in [profile_leaf, username].into_iter().flatten() {
        let name = name.trim();
        if !name.is_empty() {
            out = redact_named_user(&out, name);
        }
    }
    redact_steam_ids(&redact_user_paths(&out))
}

/// Byte length of the path separator at `i`: `\\` (JSON-escaped), `\`, or `/`.
fn sep_len(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    match b.get(i) {
        Some(b'\\') if b.get(i + 1) == Some(&b'\\') => Some(2),
        Some(b'\\') | Some(b'/') => Some(1),
        _ => None,
    }
}

/// True when a path segment ends at `i` (end of text, separator, quote, or whitespace).
fn segment_ends(s: &str, i: usize) -> bool {
    s[i..]
        .chars()
        .next()
        .is_none_or(|c| matches!(c, '\\' | '/' | '"' | '\'') || c.is_whitespace())
}

/// Byte offset just past `<word><sep>` when `word` (lowercase) starts at `at` on a word boundary.
fn after_word_and_sep(s: &str, lower: &str, at: usize, word: &str) -> Option<usize> {
    let boundary = at == 0 || !s.as_bytes()[at - 1].is_ascii_alphanumeric();
    if !boundary || !lower[at..].starts_with(word) {
        return None;
    }
    let sep_at = at + word.len();
    sep_len(s, sep_at).map(|n| sep_at + n)
}

/// Replace `name` (which may contain spaces) when it is the whole segment after `Users<sep>`.
fn redact_named_user(text: &str, name: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(off) = lower[i..].find("users") {
        let at = i + off;
        let hit = after_word_and_sep(text, &lower, at, "users").filter(|&start| {
            lower[start..].starts_with(&name_lower) && segment_ends(text, start + name.len())
        });
        match hit {
            Some(start) => {
                out.push_str(&text[i..start]);
                out.push_str(REDACTED_USER);
                i = start + name.len();
            }
            None => {
                out.push_str(&text[i..at + 1]);
                i = at + 1;
            }
        }
    }
    out.push_str(&text[i..]);
    out
}

/// Replace the user name in `X:<sep>Users<sep><name>` with `[REDACTED]` (case-insensitive,
/// separators may be mixed). The name ends at the first separator, quote, whitespace, or the end.
pub(crate) fn redact_user_paths(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while let Some(off) = line[i..].find(':') {
        let colon = i + off;
        let preceded_by_drive = colon > 0 && line.as_bytes()[colon - 1].is_ascii_alphabetic();
        let name_start = sep_len(line, colon + 1)
            .filter(|_| preceded_by_drive)
            .and_then(|n| after_word_and_sep(line, &lower, colon + 1 + n, "users"));
        let Some(name_start) = name_start else {
            out.push_str(&line[i..colon + 1]);
            i = colon + 1;
            continue;
        };
        let name_end = line[name_start..]
            .find(|c: char| matches!(c, '\\' | '/' | '"' | '\'') || c.is_whitespace())
            .map_or(line.len(), |n| name_start + n);
        out.push_str(&line[i..name_start]);
        if name_end > name_start {
            out.push_str(REDACTED_USER);
        }
        i = name_end;
    }
    out.push_str(&line[i..]);
    out
}

/// `userdata<sep><digits>` (a Steam account id) becomes `userdata<sep>[REDACTED]`.
fn redact_steam_ids(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(off) = lower[i..].find("userdata") {
        let at = i + off;
        let Some(start) = after_word_and_sep(text, &lower, at, "userdata") else {
            out.push_str(&text[i..at + 1]);
            i = at + 1;
            continue;
        };
        let digits = text[start..].bytes().take_while(u8::is_ascii_digit).count();
        out.push_str(&text[i..start]);
        if digits > 0 {
            out.push_str(REDACTED_USER);
        }
        i = start + digits;
    }
    out.push_str(&text[i..]);
    out
}

/// Create `<stem>.<ext>` in `dir`, or `<stem>-2.<ext>`, `-3`... if taken (atomic `create_new`).
fn create_unique(dir: &Path, stem: &str, ext: &str) -> QmResult<(PathBuf, std::fs::File)> {
    for n in 1..1000 {
        let name = if n == 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}-{n}.{ext}")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(QmError::new(ErrorKind::Io, e.to_string())
                    .with_details(path.display().to_string()))
            }
        }
    }
    Err(QmError::new(
        ErrorKind::Io,
        format!("Too many files named {stem} in {}", dir.display()),
    ))
}

/// Last `n` lines of a file, read backwards in chunks so large logs stay cheap.
fn tail_lines(path: &Path, n: u32) -> std::io::Result<Vec<String>> {
    const CHUNK: u64 = 64 * 1024;
    let mut file = std::fs::File::open(path)?;
    if n == 0 {
        return Ok(Vec::new());
    }
    let len = file.metadata()?.len();
    let mut pos = len;
    let mut buf: Vec<u8> = Vec::new();
    // One extra newline lets the first kept line be complete.
    let need = n as usize + 1;
    while pos > 0 && bytecount(&buf, b'\n') <= need {
        let step = CHUNK.min(pos);
        pos -= step;
        file.seek(SeekFrom::Start(pos))?;
        let mut chunk = vec![0u8; step as usize];
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&buf);
        buf = chunk;
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if pos > 0 && !lines.is_empty() {
        // The first line may be cut by the chunk boundary.
        lines.remove(0);
    }
    let skip = lines.len().saturating_sub(n as usize);
    Ok(lines[skip..].iter().map(|l| l.to_string()).collect())
}

fn bytecount(buf: &[u8], b: u8) -> usize {
    buf.iter().filter(|x| **x == b).count()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{redact_bundle_text, redact_user_paths};

    #[test]
    fn redacts_any_case_and_keeps_other_text() {
        assert_eq!(
            redact_user_paths(r"a c:\USERS\Bob\x and D:\Users\eve\y"),
            r"a c:\USERS\[REDACTED]\x and D:\Users\[REDACTED]\y"
        );
        assert_eq!(redact_user_paths(r"C:\Users\"), r"C:\Users\");
        assert_eq!(redact_user_paths(r"no paths here"), "no paths here");
        assert_eq!(redact_user_paths(r"\Users\bob\x"), r"\Users\bob\x");
    }

    #[test]
    fn redacts_forward_slash_and_doubled_backslash_forms() {
        assert_eq!(
            redact_user_paths("C:/Users/bob/AppData"),
            "C:/Users/[REDACTED]/AppData"
        );
        assert_eq!(
            redact_user_paths(r#"{"p":"C:\\Users\\bob\\AppData"}"#),
            r#"{"p":"C:\\Users\\[REDACTED]\\AppData"}"#
        );
    }

    #[test]
    fn name_ends_at_quote_whitespace_or_end_of_line() {
        assert_eq!(
            redact_user_paths(r"at C:\Users\bob"),
            r"at C:\Users\[REDACTED]"
        );
        assert_eq!(
            redact_user_paths(r#"path "C:\Users\bob" then a/b\c"#),
            r#"path "C:\Users\[REDACTED]" then a/b\c"#
        );
        assert_eq!(
            redact_user_paths(r"C:\Users\bob was here\x"),
            r"C:\Users\[REDACTED] was here\x"
        );
        assert_eq!(
            redact_user_paths("'C:/Users/bob' ok"),
            "'C:/Users/[REDACTED]' ok"
        );
    }

    fn bundle(text: &str) -> String {
        redact_bundle_text(text, Some("John Smith"), Some("jsmith"))
    }

    #[test]
    fn profile_leaf_with_spaces_is_redacted() {
        assert_eq!(
            bundle(r"C:\Users\John Smith\Documents"),
            r"C:\Users\[REDACTED]\Documents"
        );
        assert_eq!(
            bundle(r"c:\users\JOHN SMITH\Documents"),
            r"c:\users\[REDACTED]\Documents"
        );
    }

    #[test]
    fn json_escaped_forward_and_mixed_forms() {
        assert_eq!(
            bundle(r#"{"p": "C:\\Users\\John Smith\\x"}"#),
            r#"{"p": "C:\\Users\\[REDACTED]\\x"}"#
        );
        assert_eq!(
            bundle("C:/Users/John Smith/AppData"),
            "C:/Users/[REDACTED]/AppData"
        );
        assert_eq!(
            bundle(r"C:\Users/John Smith\AppData and C:/Users\\bob/x"),
            r"C:\Users/[REDACTED]\AppData and C:/Users\\[REDACTED]/x"
        );
    }

    #[test]
    fn username_differing_from_profile_leaf_is_redacted() {
        assert_eq!(
            bundle(r"\\?\UNC\server\Users\JSmith\share and D:\Users\jsmith"),
            r"\\?\UNC\server\Users\[REDACTED]\share and D:\Users\[REDACTED]"
        );
        // Only whole segments are matched by name; drive paths still fall back to the generic rule.
        assert_eq!(
            bundle(r"\\srv\Users\jsmithers\x"),
            r"\\srv\Users\jsmithers\x"
        );
        assert_eq!(bundle(r"C:\Users\jsmithers\x"), r"C:\Users\[REDACTED]\x");
        assert_eq!(bundle(r"notes about jsmith"), "notes about jsmith");
    }

    #[test]
    fn steam_userdata_ids_are_redacted() {
        assert_eq!(
            bundle(r"C:\Program Files (x86)\Steam\userdata\123456789\1391530\remote"),
            r"C:\Program Files (x86)\Steam\userdata\[REDACTED]\1391530\remote"
        );
        assert_eq!(
            bundle(r#""C:\\Steam\\UserData\\42\\x" and D:/Steam/userdata/77"#),
            r#""C:\\Steam\\UserData\\[REDACTED]\\x" and D:/Steam/userdata/[REDACTED]"#
        );
        assert_eq!(bundle(r"userdata\config"), r"userdata\config");
    }

    #[test]
    fn nested_bundle_values_are_redacted() {
        let raw = serde_json::json!({
            "health": {
                "recent_logs": [
                    r"loaded C:\Users\John Smith\AppData\Roaming\mnb\saveMNB3.lua",
                    r"steam at D:\Steam\userdata\987654\remote"
                ]
            },
            "launcher_log": [r"opened C:\Users\jsmith\x"]
        });
        let text = serde_json::to_string_pretty(&raw).expect("serialize");
        let out = bundle(&text);
        assert!(!out.contains("John Smith"), "{out}");
        assert!(!out.contains("jsmith"), "{out}");
        assert!(!out.contains("987654"), "{out}");
        let v: serde_json::Value = serde_json::from_str(&out).expect("still valid JSON");
        assert_eq!(
            v["health"]["recent_logs"][0],
            r"loaded C:\Users\[REDACTED]\AppData\Roaming\mnb\saveMNB3.lua"
        );
        assert_eq!(
            v["health"]["recent_logs"][1],
            r"steam at D:\Steam\userdata\[REDACTED]\remote"
        );
        assert_eq!(v["launcher_log"][0], r"opened C:\Users\[REDACTED]\x");
    }
}
