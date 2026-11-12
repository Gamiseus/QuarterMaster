//! The text the game loads before any mod patch, and the DLL's text decision (plan 2e D1,
//! §4.1, §4.2).
//!
//! `prepare_buffer` holds QuarterMaster's own edits: the conf.lua save identity, the main.lua
//! Steam safety prefix and runtime footer, and the external.lua score upload defuse. The DLL
//! and the simulation both patch its output, so the simulation sees the text the game runs.
//! `dll_transform` is everything the DLL decides about a chunk's text, with no Lua state.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::ops::Range;

use log::info;
use regex_lite::Regex;

use crate::dump::{PatchDebug, PatchRegion};
use crate::handshake::{LaunchConfig, PatchExecutionRecord};
use crate::patch::loader::STEAM_SAFETY_PAYLOAD;
use crate::patch::origin::{FileTrace, LineOrigin, RemovedLine};
use crate::patch::table::{PatchTable, Prepared, TracedApply};
use crate::QM_RUNTIME_FOOTER_PAYLOAD;

/// Start of the record warning for a patch whose match touched QuarterMaster's own lines.
pub const MATCHED_QM_STARTUP_CODE: &str = "matched_qm_startup_code";

/// Start of the record warning for a match that was refused because it covers QuarterMaster's
/// own lines (D42).
pub const BUILTIN_MATCH_SKIPPED: &str = "matched_qm_startup_code: skipped";

/// Start of the warning for a main.lua append that ends in a top-level `return`, kept after
/// QuarterMaster's footer (D43).
pub const COPY_AFTER_FOOTER_RETURN: &str = "copy_after_footer:return";

/// The `BUILTIN_MATCH_SKIPPED` warning for a refused match whose first refused line is
/// 0-based `line` of the prepared text; `file_line` is its line in the file, `None` for a line
/// QuarterMaster added.
pub fn builtin_skip_warning(target: &str, line: usize, file_line: Option<usize>) -> String {
    match file_line {
        Some(n) => format!(
            "{BUILTIN_MATCH_SKIPPED}: a match at line {n} of '{target}' would remove or rewrite a line QuarterMaster changes at startup"
        ),
        None => format!(
            "{BUILTIN_MATCH_SKIPPED}: a match in '{target}' covers code QuarterMaster adds at startup (line {} of the loaded text)",
            line + 1
        ),
    }
}

/// A chunk with QuarterMaster's edits applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedBuffer {
    pub text: String,
    /// 0-based line ranges of `text` that QuarterMaster added.
    pub builtin: Vec<Range<usize>>,
    /// 0-based lines of `text` that QuarterMaster rewrote in place.
    pub builtin_edited: Vec<usize>,
    /// An edit rule ran on this chunk, even if it changed nothing. The DLL then loads the
    /// prepared text (and writes dumps) instead of passing the chunk through.
    pub touched: bool,
}

impl PreparedBuffer {
    /// Whether any line of `text` is QuarterMaster's.
    pub fn has_builtin(&self) -> bool {
        !self.builtin.is_empty() || !self.builtin_edited.is_empty()
    }

    /// Whether 0-based `line` was added by QuarterMaster.
    pub fn is_builtin(&self, line: usize) -> bool {
        self.builtin.iter().any(|r| r.contains(&line))
    }

    /// `file_line` for a 0-based line of `text`.
    pub fn file_line(&self, line: usize) -> Option<usize> {
        file_line(&self.builtin, line)
    }
}

/// The 1-based line, in the file's own numbering, of 0-based `line` of a prepared text whose
/// added lines are `builtin`. None for an added line.
pub fn file_line(builtin: &[Range<usize>], line: usize) -> Option<usize> {
    let added = |k: &usize| builtin.iter().any(|r| r.contains(k));
    if added(&line) {
        return None;
    }
    Some(line - (0..line).filter(added).count() + 1)
}

fn line_count(text: &str) -> usize {
    text.lines().count()
}

/// The game's root files QuarterMaster edits (D44).
pub const MAIN_LUA: &str = "main.lua";
pub const CONF_LUA: &str = "conf.lua";
pub const EXTERNAL_LUA: &str = "includes/external.lua";

/// What a chunk name names (D44).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkFile {
    /// `=[love "x"]`: LÖVE's embedded scripts, never a game file.
    Love(String),
    /// `=[qm <name> "<source>"]`: a mod module.
    Module { name: String, source: String },
    /// Not a file: a code string, `=stdin`, `=(load)`, `=[string ...]`.
    Other,
    /// A game file, by its canonical path (`canonical_path`).
    Game(String),
}

impl ChunkFile {
    /// The canonical path of a game file.
    pub fn game_path(&self) -> Option<&str> {
        match self {
            ChunkFile::Game(path) => Some(path),
            _ => None,
        }
    }

    /// Whether this is the game's root file `file` (`is_root_file`).
    pub fn is_root(&self, file: &str) -> bool {
        self.game_path().is_some_and(|p| is_root_file(p, file))
    }
}

/// What `chunk_name` names (D44). Only the exact love and module forms are not game files,
/// plus names with a newline or without a `.lua` end; every other name is a game file.
pub fn chunk_file(chunk_name: &str) -> ChunkFile {
    if let Some(x) = chunk_name
        .strip_prefix("=[love \"")
        .and_then(|s| s.strip_suffix("\"]"))
        .filter(|x| !x.contains('"'))
    {
        return ChunkFile::Love(x.to_string());
    }
    if let Some((name, source)) = chunk_name
        .strip_prefix("=[qm ")
        .and_then(|s| s.strip_suffix("\"]"))
        .and_then(|s| s.split_once(" \""))
        .filter(|(n, s)| !n.is_empty() && !n.contains(char::is_whitespace) && !s.contains('"'))
    {
        return ChunkFile::Module {
            name: name.to_string(),
            source: source.to_string(),
        };
    }
    if chunk_name.contains('\n') {
        return ChunkFile::Other;
    }
    let path = canonical_path(chunk_name);
    if path.ends_with(".lua") {
        ChunkFile::Game(path)
    } else {
        ChunkFile::Other
    }
}

/// A chunk name or patch target in one form: one leading `@` or `=` stripped, `\` turned
/// into `/`, and any leading `./` stripped (D44).
pub fn canonical_path(name: &str) -> String {
    let name = name
        .strip_prefix('@')
        .or_else(|| name.strip_prefix('='))
        .unwrap_or(name);
    let name = name.replace('\\', "/");
    let mut rest = name.as_str();
    while let Some(r) = rest.strip_prefix("./") {
        rest = r;
    }
    rest.to_string()
}

/// Whether canonical `path` is the game's root file `file`: equal to it, or an absolute path
/// (a drive letter or a leading `/`) ending in `/<file>` (D44).
pub fn is_root_file(path: &str, file: &str) -> bool {
    if path == file {
        return true;
    }
    let b = path.as_bytes();
    let absolute = path.starts_with('/')
        || (b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1) == Some(&b':'));
    absolute
        && path
            .strip_suffix(file)
            .is_some_and(|head| head.ends_with('/'))
}

/// QuarterMaster's edits to `vanilla`. `chunk_name` is a chunk name or a canonical path;
/// only the game's root files get edits (D44).
pub fn prepare_buffer(chunk_name: &str, vanilla: &str, launch: &LaunchConfig) -> PreparedBuffer {
    let file = chunk_file(chunk_name);
    let name = file.game_path().unwrap_or_default();
    let is_main = file.is_root(MAIN_LUA);
    let launch_config = launch;
    let mut current_buf = vanilla.to_string();
    let mut was_safety_patched = false;

    if file.is_root(CONF_LUA) {
        if let Some(ref custom_identity) = launch_config.save_identity_override {
            if !custom_identity.trim().is_empty() {
                info!(
                    "Overriding LÖVE save identity in conf.lua to '{}'",
                    custom_identity
                );
                current_buf = current_buf.replace(
                    "t.identity = nil",
                    &format!("t.identity = {:?}", custom_identity),
                );
                was_safety_patched = true;
            }
        }
    }

    let mut prefix_lines = 0;
    if is_main {
        info!("Injecting Steam safety hook into {}", name);
        current_buf.insert_str(0, STEAM_SAFETY_PAYLOAD);
        current_buf.push('\n');
        current_buf.push_str(QM_RUNTIME_FOOTER_PAYLOAD);
        was_safety_patched = true;
        prefix_lines = STEAM_SAFETY_PAYLOAD.matches('\n').count();
    }

    if file.is_root(EXTERNAL_LUA) {
        info!("Neutralizing uploadScoresHelper in {}", name);
        if current_buf.contains("function uploadScoresHelper") {
            current_buf = current_buf.replace(
                "function uploadScoresHelper(handle, scorez)",
                "function uploadScoresHelper(handle, scorez) return false end function _disabled_uploadScoresHelper(handle, scorez)",
            );
            was_safety_patched = true;
        }
    }

    // The prefix ends with a newline, and the in-place edits add none, so the vanilla part
    // keeps its line count: lines `prefix_lines..prefix_lines + vanilla_lines`.
    let vanilla_lines = line_count(vanilla);
    let total = line_count(&current_buf);
    let mut builtin = Vec::new();
    if prefix_lines > 0 {
        builtin.push(0..prefix_lines);
    }
    let vanilla_end = prefix_lines + vanilla_lines;
    if is_main && total > vanilla_end {
        builtin.push(vanilla_end..total);
    }
    let builtin_edited = vanilla
        .lines()
        .zip(current_buf.lines().skip(prefix_lines))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + prefix_lines)
        .collect();

    PreparedBuffer {
        text: current_buf,
        builtin,
        builtin_edited,
        touched: was_safety_patched,
    }
}

/// The file name of a chunk's dumps: `=[love "main.lua"]` becomes `love//main.lua`,
/// `@main.lua` becomes `main.lua`. Everything else uses `chunk_file`.
pub fn pretty_name(chunk_name: &str) -> String {
    let regex = Regex::new(r#"=\[(\w+)(?: (\S+))? "([^"]+)"\]"#).ok();
    if let Some(capture) = regex.as_ref().and_then(|r| r.captures(chunk_name)) {
        let f1 = capture.get(1).map_or("", |x| x.as_str());
        let f2 = capture.get(2).map_or("", |x| x.as_str()).replace('.', "/");
        let f3 = capture.get(3).map_or("", |x| x.as_str());
        format!("{f1}/{f2}/{f3}")
    } else {
        chunk_name.replace('@', "")
    }
}

/// What the DLL does with a chunk's text.
#[derive(Debug, Clone)]
pub enum DllOutcome {
    /// Load the chunk as it is.
    Passthrough,
    /// Load QuarterMaster's edits only (no mod patch targets this chunk, or mods are off).
    Prepared(String),
    Patched {
        text: String,
        debug: PatchDebug,
        records: Vec<PatchExecutionRecord>,
        /// Per line of `text`, the index in `records` of the record that wrote it, or
        /// `linemap::NO_RECORD` (crash attribution).
        line_records: Vec<u32>,
    },
    /// A patch with `on_failure = "abort"` failed: the chunk fails to load with this error.
    SyntaxError(String),
}

/// Everything the DLL decides about a chunk's text, with no Lua state (§4.1). `prep` is the
/// table's shared `Prepared`. Only game files are prepared and patched, by their canonical
/// path (D44); the love, module and other chunks load as they are.
pub fn dll_transform(
    table: &PatchTable,
    prep: &Prepared,
    launch: &LaunchConfig,
    chunk_name: &str,
    buffer: &str,
    dump_all: bool,
) -> DllOutcome {
    let ChunkFile::Game(path) = chunk_file(chunk_name) else {
        if dump_all {
            return DllOutcome::Prepared(buffer.to_string());
        }
        return DllOutcome::Passthrough;
    };
    let prepared = prepare_buffer(&path, buffer, launch);

    if !table.needs_patching(&path) {
        if !prepared.touched && !dump_all {
            return DllOutcome::Passthrough;
        }
        return DllOutcome::Prepared(prepared.text);
    }

    #[cfg(test)]
    crate::inject_test_panic(chunk_name);

    match table.apply_prepared_buffer(prep, &path, &prepared) {
        Ok(a) => DllOutcome::Patched {
            text: a.text,
            debug: a.debug,
            records: a.records,
            line_records: a.line_records,
        },
        Err(e) => DllOutcome::SyntaxError(e),
    }
}

/// The simulation's side of D1: `vanilla` prepared as the DLL prepares chunk `target`, then
/// patched with a trace over the prepared text. The text and records equal the DLL's. A
/// `target` that is not a game file (`chunk_file`) stays unpatched, as in the DLL.
pub fn simulate_buffer(
    table: &PatchTable,
    prep: &Prepared,
    launch: &LaunchConfig,
    target: &str,
    vanilla: &str,
) -> Result<(PreparedBuffer, TracedApply), String> {
    let ChunkFile::Game(path) = chunk_file(target) else {
        let raw = PreparedBuffer {
            text: vanilla.to_string(),
            ..Default::default()
        };
        let none = PatchTable::default();
        let traced = none.apply_prepared_buffer_traced(&none.prepare(), target, &raw)?;
        return Ok((raw, traced));
    };
    let prepared = prepare_buffer(&path, vanilla, launch);
    let traced = table.apply_prepared_buffer_traced(prep, &path, &prepared)?;
    Ok((prepared, traced))
}

/// Whether `rec` carries the `matched_qm_startup_code` warning.
pub fn is_builtin_match(rec: &PatchExecutionRecord) -> bool {
    rec.warnings
        .iter()
        .any(|w| w.starts_with(MATCHED_QM_STARTUP_CODE))
}

/// A trace over prepared text, as the file shows it (D1, §4.2): QuarterMaster's added lines
/// are hidden and vanilla lines keep the file's own numbering. Lines it rewrote in place stay,
/// with their file number. Edits keep the prepared text's byte offsets.
#[derive(Debug, Clone, Default)]
pub struct FileView {
    pub trace: FileTrace,
    /// Per 0-based line of the prepared trace, its 1-based line in `trace`, or `None` when
    /// hidden.
    pub file_line: Vec<Option<usize>>,
}

impl FileView {
    pub fn new(game: &FileTrace, buf: &PreparedBuffer) -> FileView {
        // Prepared vanilla line (1-based) -> file line (1-based), None for added lines.
        let prepared_lines = line_count(&buf.text);
        let mut vmap: Vec<Option<usize>> = Vec::with_capacity(prepared_lines + 1);
        vmap.push(None);
        let mut n = 0;
        for i in 0..prepared_lines {
            if buf.is_builtin(i) {
                vmap.push(None);
            } else {
                n += 1;
                vmap.push(Some(n));
            }
        }
        let map_v = |v: usize| vmap.get(v).copied().flatten();
        let hidden = |o: &LineOrigin| match *o {
            LineOrigin::Vanilla(v)
            | LineOrigin::Kept {
                vanilla: Some(v), ..
            } => map_v(v).is_none(),
            _ => false,
        };
        let remap = |o: &LineOrigin| match *o {
            LineOrigin::Vanilla(v) => LineOrigin::Vanilla(map_v(v).unwrap_or(0)),
            LineOrigin::Inserted { edit } => LineOrigin::Inserted { edit },
            LineOrigin::Kept { edit, vanilla } => LineOrigin::Kept {
                edit,
                vanilla: vanilla.and_then(map_v),
            },
            LineOrigin::Modified { edit, vanilla } => LineOrigin::Modified {
                edit,
                vanilla: vanilla.and_then(map_v),
            },
        };

        let mut text = String::with_capacity(game.text.len());
        let mut lines = Vec::with_capacity(game.lines.len());
        let mut file_line = Vec::with_capacity(game.lines.len());
        for (i, span) in game.text.split_inclusive('\n').enumerate() {
            let origin = game.lines.get(i).copied().unwrap_or(LineOrigin::Vanilla(0));
            if hidden(&origin) {
                file_line.push(None);
                continue;
            }
            text.push_str(span);
            lines.push(remap(&origin));
            file_line.push(Some(lines.len()));
        }
        // The pre-vars text, with the same lines hidden.
        let raw_text = game.raw_text.as_deref().map(|raw| {
            let mut out = String::with_capacity(raw.len());
            for (i, span) in raw.split_inclusive('\n').enumerate() {
                if file_line.get(i).is_some_and(Option::is_some) {
                    out.push_str(span);
                }
            }
            out
        });
        let removed = game
            .removed
            .iter()
            .map(|gone| {
                gone.iter()
                    .filter(|r| !hidden(&r.origin))
                    .map(|r| RemovedLine {
                        origin: remap(&r.origin),
                        kept: r.kept,
                    })
                    .collect()
            })
            .collect();
        FileView {
            trace: FileTrace {
                text,
                lines,
                edits: game.edits.clone(),
                removed,
                raw_text,
            },
            file_line,
        }
    }

    /// The file line (1-based) of prepared line `game_line` (1-based), when it is shown.
    pub fn line(&self, game_line: usize) -> Option<usize> {
        self.file_line
            .get(game_line.checked_sub(1)?)
            .copied()
            .flatten()
    }

    /// The first shown file line at or after `game_line` (1-based), clamped to the file.
    pub fn line_at_or_after(&self, game_line: usize) -> usize {
        let shown_before = self
            .file_line
            .iter()
            .take(game_line.saturating_sub(1))
            .filter(|l| l.is_some())
            .count();
        (shown_before + 1).min(self.trace.lines.len().max(1))
    }

    /// The last shown file line at or before `game_line` (1-based), at least 1.
    pub fn line_at_or_before(&self, game_line: usize) -> usize {
        self.file_line
            .iter()
            .take(game_line)
            .filter(|l| l.is_some())
            .count()
            .max(1)
    }

    /// `regions` (prepared numbering) in file numbering.
    pub fn regions(&self, regions: &[PatchRegion]) -> Vec<PatchRegion> {
        regions
            .iter()
            .map(|r| {
                let start_line = self.line_at_or_after(r.start_line);
                let end_line = self.line_at_or_before(r.end_line).max(start_line);
                PatchRegion {
                    start_line,
                    end_line,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
