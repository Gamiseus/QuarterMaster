#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! Patch fingerprints, references and snapshots (Phase C §3.7). A fingerprint hashes what a
//! patch matches (`anchor`) and what it adds (`payload`), so a fix can tell a moved patch from
//! a changed one. Ids, names, priorities and `before`/`after` are not hashed: renames and
//! reorders are not drift.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::patch::{InsertPosition, Patch, Target};

fn one() -> u8 {
    1
}

/// Content hashes of a patch, plus where it sits in its mod.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct PatchFingerprint {
    #[serde(default = "one")]
    pub v: u8,
    pub anchor: String,
    pub payload: String,
    /// Mod-relative toml path, '/' and lowercase.
    pub source: String,
    /// 1-based among the mod's patches with this primary target.
    pub ordinal: u32,
    #[serde(default)]
    pub mod_version: Option<String>,
}

impl PatchFingerprint {
    /// Same anchor and payload hashes (where the patch sits is not compared).
    pub fn same_content(&self, other: &PatchFingerprint) -> bool {
        self.anchor == other.anchor && self.payload == other.payload
    }

    /// The content hashes and place, as a compat override or a hunk key stores them.
    pub fn lite(&self) -> PatchFingerprintLite {
        PatchFingerprintLite {
            anchor: self.anchor.clone(),
            payload: self.payload.clone(),
            source: Some(self.source.clone()),
            ordinal: Some(self.ordinal),
        }
    }
}

/// A patch named by id, with its fingerprint when known.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PatchRef {
    pub id: String,
    pub mod_id: String,
    #[serde(default)]
    pub fp: Option<PatchFingerprint>,
}

/// A compat override's fingerprint (§3.10): the content hashes are required; `source` and
/// `ordinal` only narrow re-binding.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PatchFingerprintLite {
    pub anchor: String,
    pub payload: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub ordinal: Option<u32>,
}

/// The text of a patch for drift diffs (D41). Kept in profile.json only, never in
/// load-order.json.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PatchSnapshot {
    /// "pattern" | "regex" | "copy" | "module".
    pub kind: String,
    /// Normalized, before `[vars]`.
    pub pattern: String,
    /// Normalized, with the mod folder put back as `{{qm_hack:patch_dir}}`.
    pub payload: String,
    pub position: Option<InsertPosition>,
    pub times: Option<u32>,
    /// A field was over 64 KiB and was cut.
    #[serde(default)]
    pub truncated: bool,
    pub mod_version: Option<String>,
    pub taken_at: String,
}

/// The placeholder the loader replaces with the mod folder.
pub const PATCH_DIR_HACK: &str = "{{qm_hack:patch_dir}}";

/// The most a snapshot keeps of one field.
pub const SNAPSHOT_CAP: usize = 64 * 1024;

const TAG: &str = "qm-fp1";

/// The first 32 hex chars of the SHA-256 of `x`.
pub fn hash32(x: &str) -> String {
    hex_prefix(x, 32)
}

/// The first 16 hex chars of the SHA-256 of the trimmed line (edge rung checks).
pub fn short_hash(line: &str) -> String {
    hex_prefix(line.trim(), 16)
}

fn hex_prefix(x: &str, n: usize) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(x.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out.truncate(n);
    out
}

/// `s` with the mod folder (as the loader wrote it, with single or doubled backslashes) put
/// back as `{{qm_hack:patch_dir}}`.
pub fn unhack(s: &str, mod_dir: &Path) -> String {
    let single = mod_dir.to_string_lossy().to_string();
    if single.is_empty() {
        return s.to_string();
    }
    let doubled = single.replace('\\', "\\\\");
    let mut out = s.to_string();
    if doubled != single {
        out = out.replace(&doubled, PATCH_DIR_HACK);
    }
    out.replace(&single, PATCH_DIR_HACK)
}

/// `unhack`, then LF line ends, each line trimmed at the end and trailing empty lines removed.
pub fn norm(s: &str, mod_dir: &Path) -> String {
    let s = unhack(s, mod_dir).replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = s.split('\n').map(str::trim_end).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// The context hash of a vanilla site: lines `[u-2, u+k+2)` of the prepared vanilla text,
/// trimmed, with QuarterMaster's builtin lines skipped and the range clamped to the file.
pub fn site_context(
    lines: &[&str],
    is_builtin: impl Fn(usize) -> bool,
    u: usize,
    k: usize,
) -> String {
    let start = u.saturating_sub(2);
    let end = u.saturating_add(k).saturating_add(2).min(lines.len());
    let mut text = format!("{TAG}\0ctx\0");
    let picked: Vec<&str> = (start..end)
        .filter(|&i| !is_builtin(i))
        .filter_map(|i| lines.get(i).map(|l| l.trim()))
        .collect();
    text.push_str(&picked.join("\n"));
    hash32(&text)
}

fn targets(t: &Target) -> String {
    let mut v: Vec<String> = match t {
        Target::Single(s) => vec![s.clone()],
        Target::Multi(v) => v.clone(),
    };
    for s in &mut v {
        *s = s.to_lowercase().replace('\\', "/");
    }
    v.sort();
    v.join("\n")
}

fn position_name(p: InsertPosition) -> &'static str {
    match p {
        InsertPosition::At => "at",
        InsertPosition::Before => "before",
        InsertPosition::After => "after",
    }
}

fn times_text(t: Option<usize>) -> String {
    t.map_or_else(|| "-".to_string(), |n| n.to_string())
}

fn joined(parts: &[&str]) -> String {
    parts.join("\0")
}

/// Whether this is a mod's generated asset patch (its payload holds absolute paths).
fn is_auto_assets(source: &str) -> bool {
    source == "assets"
}

/// The anchor and payload hashes of `patch` (§3.7).
pub(crate) fn content_hashes(patch: &Patch, mod_dir: &Path, source: &str) -> (String, String) {
    if is_auto_assets(source) {
        let m = patch.mod_id().unwrap_or_default();
        return (hash32(&joined(&[TAG, "auto_assets", m])), String::new());
    }
    let anchor = match patch {
        Patch::Pattern(p) => {
            let pattern: Vec<String> = unhack(&p.pattern, mod_dir)
                .lines()
                .map(|l| l.trim().to_string())
                .collect();
            joined(&[
                TAG,
                "anchor",
                "pattern",
                &targets(&p.target),
                position_name(p.position),
                &times_text(p.times),
                if p.match_indent { "true" } else { "false" },
                &pattern.join("\n"),
            ])
        }
        Patch::Regex(r) => {
            let mut parts = vec![
                TAG.to_string(),
                "anchor".to_string(),
                "regex".to_string(),
                targets(&r.target),
                position_name(r.position).to_string(),
                times_text(r.times),
                if r.verbose { "true" } else { "false" }.to_string(),
                r.pattern.clone(),
                r.root_capture.clone().unwrap_or_default(),
                norm(&r.line_prepend, mod_dir),
            ];
            // Author rungs decide where a regex goes (D52); a regex without them keeps the
            // hash it always had.
            if !r.anchors.is_empty() || !r.sites.is_empty() {
                parts.push("rungs".to_string());
                parts.push(
                    serde_json::to_string(&(&r.anchors, &r.sites))
                        .unwrap_or_else(|e| e.to_string()),
                );
            }
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            joined(&refs)
        }
        Patch::Copy(c) => {
            let pos = match c.position {
                crate::patch::copy::CopyPosition::Prepend => "prepend",
                crate::patch::copy::CopyPosition::Append => "append",
            };
            joined(&[TAG, "anchor", "copy", pos, &targets(&c.target)])
        }
        Patch::Module(m) => joined(&[
            TAG,
            "anchor",
            "module",
            &m.name,
            m.before.as_deref().unwrap_or(""),
            if m.load_now { "true" } else { "false" },
        ]),
    };
    let payload = match patch {
        Patch::Pattern(p) => norm(&p.payload, mod_dir),
        Patch::Regex(r) => norm(&r.payload, mod_dir),
        Patch::Copy(c) => {
            let parts: Vec<String> = c
                .contents
                .iter()
                .map(String::as_str)
                .chain(c.payload.as_deref())
                .map(|s| norm(s, mod_dir))
                .collect();
            parts.join("\0")
        }
        Patch::Module(m) => norm(&m.content, mod_dir),
    };
    (
        hash32(&anchor),
        hash32(&joined(&[TAG, "payload", &payload])),
    )
}

#[cfg(test)]
thread_local! {
    /// How many times `Patch::fingerprint` ran on this thread (the DLL laziness tests).
    pub(crate) static FINGERPRINTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) fn fingerprint_of(
    patch: &Patch,
    mod_dir: &Path,
    source: &str,
    ordinal: u32,
) -> PatchFingerprint {
    #[cfg(test)]
    FINGERPRINTS.with(|c| c.set(c.get() + 1));
    let (anchor, payload) = content_hashes(patch, mod_dir, source);
    PatchFingerprint {
        v: 1,
        anchor,
        payload,
        source: source.to_string(),
        ordinal,
        mod_version: None,
    }
}

/// Cuts `s` to at most `SNAPSHOT_CAP` bytes on a char boundary; true when it cut.
fn cap(s: &mut String) -> bool {
    if s.len() <= SNAPSHOT_CAP {
        return false;
    }
    let mut end = SNAPSHOT_CAP;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    true
}

pub(crate) fn snapshot_of(patch: &Patch, mod_dir: &Path) -> PatchSnapshot {
    let times = |t: Option<usize>| t.and_then(|n| u32::try_from(n).ok());
    let (kind, pattern, payload, position, t) = match patch {
        Patch::Pattern(p) => (
            "pattern",
            norm(&p.pattern, mod_dir),
            norm(&p.payload, mod_dir),
            Some(p.position),
            times(p.times),
        ),
        Patch::Regex(r) => (
            "regex",
            norm(&r.pattern, mod_dir),
            norm(&r.payload, mod_dir),
            Some(r.position),
            times(r.times),
        ),
        Patch::Copy(c) => {
            let parts: Vec<String> = c
                .contents
                .iter()
                .map(String::as_str)
                .chain(c.payload.as_deref())
                .map(|s| norm(s, mod_dir))
                .collect();
            ("copy", String::new(), parts.join("\n"), None, Some(1))
        }
        Patch::Module(m) => (
            "module",
            m.name.clone(),
            norm(&m.content, mod_dir),
            None,
            Some(1),
        ),
    };
    let (mut pattern, mut payload) = (pattern, payload);
    let truncated = cap(&mut pattern) | cap(&mut payload);
    PatchSnapshot {
        kind: kind.to_string(),
        pattern,
        payload,
        position,
        times: t,
        truncated,
        mod_version: None,
        taken_at: jiff::Timestamp::now().to_string(),
    }
}

#[cfg(test)]
#[path = "fingerprint/tests.rs"]
mod tests;
