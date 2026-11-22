//! The Patch Studio (Mod Engine, Stage B): write a patch against the game's own code with the
//! real patch engine's answer in view, and save it into the mod's `qm.toml` with the file's
//! comments and layout kept.
//!
//! Previews never touch the mod: it is staged in a temp folder with the draft written into its
//! `qm.toml`, loaded with `PatchTable::load_strict` and applied with the engine, as `qm lint`
//! does. "With other mods" starts from the file as the active profile's mods leave it (the
//! simulation's trace) and applies the mod's patches over it, as if it loaded last.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::ops::engine::{decode_text, encode_text, write_atomic};
use crate::service::Service;
use qm_core::handshake::PatchOutcomeStatus;
use qm_core::patch::origin::FileTrace;
use qm_core::patch::table::PatchTable;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use toml_edit::{Array, ArrayOfTables, Document as DocumentMut, Item, Table, Value};

/// How many match places `studio_matches` lists.
const MAX_LISTED_MATCHES: usize = 20;

/// A game Lua file, for the file picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioGameFile {
    pub path: String,
    pub lines: u32,
    /// How many of the active profile's patches change it.
    pub changed_by: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioGameFiles {
    /// False until the game's files have been read (no game folder, or still starting).
    pub game_ready: bool,
    pub files: Vec<StudioGameFile>,
}

/// A run of lines (1-based, inclusive) one patch wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioLineRun {
    pub start: u32,
    pub end: u32,
    pub mod_id: String,
    pub patch_id: String,
    pub patch_name: String,
}

/// A game file as the studio shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioFileText {
    pub path: String,
    pub text: String,
    /// The file as the active profile's mods leave it; false for the game's own, also when
    /// "with mods" was asked and no mod of the profile changes the file.
    pub with_mods: bool,
    /// With mods: the lines each patch wrote.
    pub runs: Vec<StudioLineRun>,
}

/// Where chosen lines match as a pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioMatches {
    pub count: u32,
    /// The first line of the first few places (1-based).
    pub lines: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum StudioPatchKind {
    Pattern,
    Regex,
    Module,
    Copy,
}

impl StudioPatchKind {
    fn key(self) -> &'static str {
        match self {
            Self::Pattern => "pattern",
            Self::Regex => "regex",
            Self::Module => "module",
            Self::Copy => "copy",
        }
    }

    const ALL: [StudioPatchKind; 4] = [Self::Pattern, Self::Regex, Self::Module, Self::Copy];
}

/// How a patch is written in its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum StudioForm {
    /// `[[pattern]]`, `[[regex]]`, `[[copy]]`, `[[module]]`.
    Short,
    /// `[[patches]]` with `[patches.<kind>]`.
    Long,
}

/// A patch in the form. Fields a kind does not use stay empty; on save, only the keys the
/// form owns are written or removed, so any other key of an existing patch is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PatchDraft {
    pub kind: Option<StudioPatchKind>,
    pub id: String,
    pub name: String,
    pub group: String,
    /// Game files the patch changes (pattern, regex, copy). Usually one.
    pub target: Vec<String>,
    /// The lines to find (pattern) or the expression (regex).
    pub pattern: String,
    /// `at`, `before` or `after`; for copy, `prepend` or `append`.
    pub position: String,
    pub payload: String,
    pub match_indent: Option<bool>,
    pub times: Option<u32>,
    pub requires: Vec<String>,
    /// Order hints: patches this one runs before or after in the same file.
    pub before: Vec<String>,
    pub after: Vec<String>,
    /// `warn`, `skip_patch`, `skip_mod` or `abort`; empty for the default.
    pub on_failure: String,
    /// Module: the mod's Lua file; copy: the files copied.
    pub source: Vec<String>,
    /// Module: the game file it loads before.
    pub load_before: String,
}

/// Where a patch is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioPatchRef {
    /// `qm.toml`, or `qm/<name>.toml`.
    pub file: String,
    pub form: StudioForm,
    pub kind: StudioPatchKind,
    /// Its place in `[[patches]]` (long) or in `[[<kind>]]` (short).
    pub index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioPatch {
    pub at: StudioPatchRef,
    pub draft: PatchDraft,
    /// The studio can write it back (any patch file of the mod).
    pub editable: bool,
    /// The hash of its file as read, for saving.
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioPatches {
    pub patches: Vec<StudioPatch>,
    /// The hash of `qm.toml` as read (empty when there is none), for saving.
    pub hash: String,
    /// The id new patch ids start with.
    pub mod_id: String,
    /// `qm.toml` uses the long form, so new patches do too.
    pub long_form: bool,
}

/// A patch id to pick for `requires`, `before` and `after`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioPatchId {
    pub id: String,
    pub name: String,
    pub mod_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum StudioOutcomeKind {
    Applied,
    NoMatch,
    WrongCount,
    Skipped,
    Refused,
    /// The draft could not be read (a missing id, a TOML or loader error).
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioOutcome {
    pub kind: StudioOutcomeKind,
    pub matched: u32,
    pub expected: Option<u32>,
    /// What the engine said, in its words.
    pub messages: Vec<String>,
}

/// Another patch of the mod on the same file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioOtherOutcome {
    pub patch_id: String,
    pub name: String,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioPreview {
    pub target: String,
    /// The file as the game would load it with the draft.
    pub text: String,
    /// Lines the mod's patches wrote; `mine` marks the draft's.
    pub changed: Vec<StudioChangedRun>,
    pub outcome: StudioOutcome,
    pub others: Vec<StudioOtherOutcome>,
    /// The result's Lua compile error, when the game's Lua is there and finds one.
    pub compile_error: Option<String>,
    pub compile_checked: bool,
    /// The base was the active profile's result and it already holds this mod's saved patches.
    pub mod_in_base: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioChangedRun {
    pub start: u32,
    pub end: u32,
    pub mine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StudioSaved {
    pub at: StudioPatchRef,
    pub hash: String,
    /// `short_patches` was added to `mod.json`'s `qm_features`.
    pub feature_added: bool,
}

impl Service {
    /// The game's Lua files, with how many of the active profile's patches change each.
    pub fn studio_game_files(&self) -> QmResult<StudioGameFiles> {
        let Some(vanilla) = self.vanilla_files() else {
            return Ok(StudioGameFiles {
                game_ready: false,
                files: Vec::new(),
            });
        };
        let report = self.inner.caches.simulation.get();
        let mut files: Vec<StudioGameFile> = vanilla
            .files
            .iter()
            .filter(|(p, _)| p.to_ascii_lowercase().ends_with(".lua"))
            .map(|(p, text)| StudioGameFile {
                path: p.clone(),
                lines: to_u32(text.lines().count()),
                changed_by: report
                    .as_ref()
                    .and_then(|r| r.traces.get(p))
                    .map(|t| {
                        to_u32(
                            t.edits
                                .iter()
                                .map(|e| e.patch_id.as_str())
                                .collect::<BTreeSet<_>>()
                                .len(),
                        )
                    })
                    .unwrap_or(0),
            })
            .collect();
        files.sort_by_key(|a| a.path.to_lowercase());
        Ok(StudioGameFiles {
            game_ready: true,
            files,
        })
    }

    /// A game file: the game's own text, or as the active profile's mods leave it, with the
    /// lines each patch wrote.
    pub fn studio_game_file(&self, path: &str, with_mods: bool) -> QmResult<StudioFileText> {
        let vanilla = self.vanilla_text(path)?;
        if with_mods {
            if let Some(trace) = self.profile_trace(path)? {
                return Ok(StudioFileText {
                    path: path.to_string(),
                    text: trace.text.clone(),
                    with_mods: true,
                    runs: patch_runs(&trace)
                        .into_iter()
                        .map(|(start, end, e)| StudioLineRun {
                            start,
                            end,
                            mod_id: e.mod_id.clone(),
                            patch_id: e.patch_id.clone(),
                            patch_name: e.patch_name.clone(),
                        })
                        .collect(),
                });
            }
        }
        Ok(StudioFileText {
            path: path.to_string(),
            text: vanilla,
            with_mods: false,
            runs: Vec::new(),
        })
    }

    /// How many places in the file `lines` match as a pattern, by the engine's own matching.
    pub fn studio_matches(
        &self,
        path: &str,
        lines: &str,
        with_mods: bool,
    ) -> QmResult<StudioMatches> {
        let base = self.base_text(path, with_mods)?.0;
        let anchor = lines.trim_matches('\n');
        if anchor.trim().is_empty() {
            return Ok(StudioMatches {
                count: 0,
                lines: Vec::new(),
            });
        }
        let (count, starts) = crate::ops::patch_tools::match_places(path, &base, anchor);
        Ok(StudioMatches {
            count: to_u32(count),
            lines: starts
                .into_iter()
                .take(MAX_LISTED_MATCHES)
                .map(to_u32)
                .collect(),
        })
    }

    /// The mod's patches as drafts, from `qm.toml` and `qm/*.toml`.
    pub fn studio_patches(&self, folder: &str) -> QmResult<StudioPatches> {
        let dir = self.engine_project_dir(folder)?;
        let (root_text, hash) = read_qm_toml(&dir)?;
        let mut patches = Vec::new();
        let mut long_form = false;
        if let Some(text) = &root_text {
            let doc = parse_doc("qm.toml", text)?;
            long_form = doc.get("patches").is_some();
            patches.extend(drafts_of(&doc, "qm.toml", true, &hash));
        }
        for file in qm_core::patch::loader::dir_patch_files(&dir) {
            let Ok(rel) = file.strip_prefix(&dir) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.eq_ignore_ascii_case("qm.toml") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            let Ok(f) = decode_text(&rel, &bytes) else {
                continue;
            };
            if let Ok(doc) = f.text.parse::<DocumentMut>() {
                patches.extend(drafts_of(&doc, &rel, true, &f.hash));
            }
        }
        Ok(StudioPatches {
            patches,
            hash,
            mod_id: mod_id_of(&dir, folder),
            long_form,
        })
    }

    /// Patch ids to pick from: the mod's own, then the active profile's.
    pub fn studio_patch_ids(&self, folder: &str) -> QmResult<Vec<StudioPatchId>> {
        let own = self.studio_patches(folder)?;
        let mut out: Vec<StudioPatchId> = own
            .patches
            .iter()
            .filter(|p| !p.draft.id.is_empty())
            .map(|p| StudioPatchId {
                id: p.draft.id.clone(),
                name: p.draft.name.clone(),
                mod_id: own.mod_id.clone(),
            })
            .collect();
        if let Some(report) = self.inner.caches.simulation.get() {
            let mut seen: BTreeSet<String> = out.iter().map(|p| p.id.clone()).collect();
            for f in &report.files_touched {
                for r in &f.records {
                    if r.mod_id != own.mod_id && seen.insert(r.patch_id.clone()) {
                        out.push(StudioPatchId {
                            id: r.patch_id.clone(),
                            name: r.patch_name.clone(),
                            mod_id: r.mod_id.clone(),
                        });
                    }
                }
            }
        }
        Ok(out)
    }

    /// The draft's target as the game would load it, worked out by the engine. Nothing is
    /// written to the mod.
    pub fn studio_preview(
        &self,
        folder: &str,
        draft: &PatchDraft,
        editing: Option<&StudioPatchRef>,
        with_mods: bool,
    ) -> QmResult<StudioPreview> {
        let dir = self.engine_project_dir(folder)?;
        let kind = draft
            .kind
            .ok_or_else(|| invalid("Choose what kind of patch this is."))?;
        let target = draft
            .target
            .first()
            .map(|t| t.trim().trim_start_matches('@').to_string())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| invalid("Choose the game file the patch changes."))?;
        let (base, mod_in_base) = self.base_text_for(&target, with_mods, &dir, folder)?;
        let invalid_outcome = |messages: Vec<String>| StudioPreview {
            target: target.clone(),
            text: base.clone(),
            changed: Vec::new(),
            outcome: StudioOutcome {
                kind: StudioOutcomeKind::Invalid,
                matched: 0,
                expected: None,
                messages,
            },
            others: Vec::new(),
            compile_error: None,
            compile_checked: false,
            mod_in_base,
        };
        if draft.id.trim().is_empty() {
            return Ok(invalid_outcome(vec!["Give the patch an id.".to_string()]));
        }
        if kind == StudioPatchKind::Module {
            return Ok(invalid_outcome(vec![
                "A module patch adds a file of its own; save it and launch to see it load."
                    .to_string(),
            ]));
        }
        // The patch's file (qm.toml for a new one) with the draft in it; with the mod's saved
        // patches already in the base, only the draft.
        let (file_path, file_name) =
            patch_file(&dir, editing.map_or("qm.toml", |a| a.file.as_str()))?;
        let root_text = std::fs::read(&file_path)
            .ok()
            .and_then(|b| decode_text(&file_name, &b).ok())
            .map(|f| f.text);
        let staged_toml = if mod_in_base {
            let mut doc = parse_doc("qm.toml", &with_manifest(&dir, ""))?;
            insert_draft(&mut doc, draft, kind, false, None, "qm.toml")?;
            doc.to_string()
        } else {
            let mut doc = parse_doc(
                &file_name,
                &with_manifest(&dir, root_text.as_deref().unwrap_or("")),
            )?;
            let long = doc.get("patches").is_some();
            insert_draft(&mut doc, draft, kind, long, editing, &file_name)?;
            doc.to_string()
        };
        let temp = tempfile::Builder::new()
            .prefix("qm-studio-")
            .tempdir()
            .or_kind(ErrorKind::Io)?;
        let mod_id = mod_id_of(&dir, folder);
        let staged = temp.path().join(&mod_id);
        if mod_in_base {
            std::fs::create_dir_all(&staged).or_kind(ErrorKind::Io)?;
            if dir.join("mod.json").is_file() {
                std::fs::copy(dir.join("mod.json"), staged.join("mod.json"))
                    .or_kind(ErrorKind::Io)?;
            }
        } else {
            qm_manager::library::copy_directory_clean(&dir, &staged).map_err(|e| {
                QmError::new(ErrorKind::Io, "The mod could not be staged.").with_details(e)
            })?;
            // The other patch files stay; the root file is the one with the draft.
        }
        let staged_file = if mod_in_base {
            staged.join("qm.toml")
        } else {
            staged.join(&file_name)
        };
        if let Some(parent) = staged_file.parent() {
            std::fs::create_dir_all(parent).or_kind(ErrorKind::Io)?;
        }
        std::fs::write(staged_file, staged_toml).or_kind(ErrorKind::Io)?;
        let table = match PatchTable::load_strict(temp.path()) {
            Ok(t) => t,
            Err(e) => return Ok(invalid_outcome(vec![format!("{e:#}")])),
        };
        let prep = table.prepare();
        let traced = match table.apply_prepared_traced(&prep, &target, &base) {
            Ok(t) => t,
            Err(e) => return Ok(invalid_outcome(vec![e])),
        };
        let draft_id = draft.id.trim();
        let is_draft = |id: &str| id == draft_id || id.ends_with(&format!(":{draft_id}"));
        let changed = changed_runs(&traced.trace, &is_draft);
        let mut outcome = StudioOutcome {
            kind: StudioOutcomeKind::NoMatch,
            matched: 0,
            expected: None,
            messages: Vec::new(),
        };
        let mut others = Vec::new();
        for rec in &traced.records {
            if is_draft(&rec.patch_id) {
                outcome = StudioOutcome {
                    kind: match rec.status {
                        PatchOutcomeStatus::Applied => StudioOutcomeKind::Applied,
                        PatchOutcomeStatus::ZeroMatches => StudioOutcomeKind::NoMatch,
                        PatchOutcomeStatus::CountMismatch => StudioOutcomeKind::WrongCount,
                        PatchOutcomeStatus::SkippedByPolicy
                        | PatchOutcomeStatus::SkippedDisabled => StudioOutcomeKind::Skipped,
                        PatchOutcomeStatus::Aborted => StudioOutcomeKind::Refused,
                    },
                    matched: to_u32(rec.matched_count),
                    expected: rec.expected_count.map(to_u32),
                    messages: rec.warnings.clone(),
                };
            } else if rec.mod_id == mod_id || !mod_in_base {
                others.push(StudioOtherOutcome {
                    patch_id: rec.patch_id.clone(),
                    name: rec.patch_name.clone(),
                    applied: rec.status == PatchOutcomeStatus::Applied,
                });
            }
        }
        let (compile_error, compile_checked) = match self.lua_compiler() {
            Ok(lua) => (
                lua.compile(&target, &traced.text)
                    .err()
                    .map(|e| match e.line {
                        Some(l) => format!("line {l}: {}", e.message),
                        None => e.message,
                    }),
                true,
            ),
            Err(_) => (None, false),
        };
        Ok(StudioPreview {
            target,
            text: traced.text,
            changed,
            outcome,
            others,
            compile_error,
            compile_checked,
            mod_in_base,
        })
    }

    /// Writes the draft into `qm.toml`: in place of `editing`, or as a new patch. The file's
    /// comments and layout are kept; `Conflict` when it changed since `expected_hash`.
    pub fn studio_save(
        &self,
        folder: &str,
        draft: &PatchDraft,
        editing: Option<&StudioPatchRef>,
        expected_hash: Option<&str>,
    ) -> QmResult<StudioSaved> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let kind = draft
            .kind
            .ok_or_else(|| invalid("Choose what kind of patch this is."))?;
        if draft.id.trim().is_empty() {
            return Err(invalid("Give the patch an id."));
        }
        // An edited patch goes back into its own file; a new one into qm.toml.
        let (path, shown) = patch_file(&dir, editing.map_or("qm.toml", |a| a.file.as_str()))?;
        let (text, crlf, bom) = match std::fs::read(&path) {
            Ok(bytes) => {
                check_hash(&bytes, expected_hash, &shown)?;
                let f = decode_text(&shown, &bytes)?;
                (f.text, f.crlf, f.bom)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if expected_hash.is_some_and(|h| !h.is_empty()) {
                    return Err(QmError::new(
                        ErrorKind::Conflict,
                        format!("{shown} was deleted or moved since it was read."),
                    ));
                }
                (String::new(), false, false)
            }
            Err(e) => {
                return Err(QmError::new(
                    ErrorKind::Io,
                    format!("{shown} could not be read: {e}"),
                ))
            }
        };
        let mut doc = parse_doc(&shown, &with_manifest(&dir, &text))?;
        if let Some(at) = editing {
            if draft_at(&doc, at).is_none() {
                return Err(QmError::new(
                    ErrorKind::Conflict,
                    format!("That patch is no longer where it was in {shown}. Open it again."),
                ));
            }
        }
        // A new patch: the short form, unless the file uses the long form or there is no
        // mod.json to list the feature in.
        let has_manifest = dir.join("mod.json").is_file();
        let long = match editing {
            Some(at) => at.form == StudioForm::Long,
            None => doc.get("patches").is_some() || !has_manifest,
        };
        let at = insert_draft(&mut doc, draft, kind, long, editing, &shown)?;
        let bytes = encode_text(&doc.to_string(), crlf, bom);
        // The result must still read as a patch file.
        if let Err(e) = toml::from_str::<toml::Value>(&String::from_utf8_lossy(&bytes)) {
            return Err(
                QmError::new(ErrorKind::Internal, "The patch could not be written.")
                    .with_details(e.to_string()),
            );
        }
        write_atomic(&path, &bytes)?;
        let feature_added = if long {
            false
        } else {
            add_short_patches_feature(&dir)?
        };
        Ok(StudioSaved {
            at,
            hash: qm_schema::sha256_hex(&bytes),
            feature_added,
        })
    }

    /// Removes a patch from `qm.toml`, keeping the rest of the file as it is.
    pub fn studio_remove(
        &self,
        folder: &str,
        at: &StudioPatchRef,
        expected_hash: Option<&str>,
    ) -> QmResult<String> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let (path, shown) = patch_file(&dir, &at.file)?;
        let bytes = std::fs::read(&path).or_kind(ErrorKind::NotFound)?;
        check_hash(&bytes, expected_hash, &shown)?;
        let f = decode_text(&shown, &bytes)?;
        let mut doc = parse_doc(&shown, &f.text)?;
        let key = match at.form {
            StudioForm::Long => "patches",
            StudioForm::Short => at.kind.key(),
        };
        let removed = doc
            .get_mut(key)
            .and_then(Item::as_array_of_tables_mut)
            .filter(|a| (at.index as usize) < a.len())
            .map(|a| a.remove(at.index as usize));
        if removed.is_none() {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("That patch is no longer where it was in {shown}. Open it again."),
            ));
        }
        if doc
            .get(key)
            .and_then(Item::as_array_of_tables)
            .is_some_and(ArrayOfTables::is_empty)
        {
            doc.remove(key);
        }
        let out = encode_text(&doc.to_string(), f.crlf, f.bom);
        write_atomic(&path, &out)?;
        Ok(qm_schema::sha256_hex(&out))
    }

    fn vanilla_text(&self, path: &str) -> QmResult<String> {
        let vanilla = self.vanilla_files().ok_or_else(|| {
            QmError::new(
                ErrorKind::NotFound,
                "The game's files are not read yet. Set the game folder in Settings.",
            )
        })?;
        vanilla
            .files
            .get(path)
            .cloned()
            .ok_or_else(|| not_found(format!("The game has no file {path}.")))
    }

    /// The active profile's trace of `path`: None when no patch of it changes the file, or
    /// while the simulation is still running (the game's text is shown then).
    fn profile_trace(&self, path: &str) -> QmResult<Option<std::sync::Arc<FileTrace>>> {
        Ok(self
            .inner
            .caches
            .simulation
            .get()
            .and_then(|r| r.traces.get(path).cloned()))
    }

    /// The text patterns match on: the game's, or the active profile's result (before
    /// `[vars]` values are filled in). True when that is the profile's result.
    fn base_text(&self, path: &str, with_mods: bool) -> QmResult<(String, bool)> {
        let vanilla = self.vanilla_text(path)?;
        if with_mods {
            if let Some(trace) = self.profile_trace(path)? {
                return Ok((trace.raw_lines().join("\n"), true));
            }
        }
        Ok((vanilla, false))
    }

    /// `base_text`, and whether the base already holds the mod's own saved patches.
    fn base_text_for(
        &self,
        path: &str,
        with_mods: bool,
        dir: &Path,
        folder: &str,
    ) -> QmResult<(String, bool)> {
        let (text, from_profile) = self.base_text(path, with_mods)?;
        if !from_profile {
            return Ok((text, false));
        }
        let mod_id = mod_id_of(dir, folder);
        let in_base = self
            .profile_trace(path)?
            .is_some_and(|t| t.edits.iter().any(|e| e.mod_id == mod_id));
        Ok((text, in_base))
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn invalid(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}

fn not_found(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::NotFound, msg)
}

fn check_hash(bytes: &[u8], expected: Option<&str>, file: &str) -> QmResult<()> {
    match expected {
        Some(h) if !qm_schema::sha256_hex(bytes).eq_ignore_ascii_case(h) => Err(QmError::new(
            ErrorKind::Conflict,
            format!("{file} changed on disk since it was read."),
        )),
        _ => Ok(()),
    }
}

/// A patch file of the mod: `qm.toml`, or an existing `qm/<name>.toml`. Its path and the
/// name shown for it.
fn patch_file(dir: &Path, file: &str) -> QmResult<(std::path::PathBuf, String)> {
    let shown = file.replace('\\', "/");
    if shown.eq_ignore_ascii_case("qm.toml") {
        return Ok((dir.join("qm.toml"), "qm.toml".to_string()));
    }
    let lower = shown.to_ascii_lowercase();
    let name = lower
        .strip_prefix("qm/")
        .filter(|n| n.ends_with(".toml") && !n.contains('/'))
        .ok_or_else(|| invalid(format!("'{file}' is not a patch file of the mod.")))?;
    qm_manager::library::validate_safe_relative_path(&shown).map_err(invalid)?;
    let path = dir.join("qm").join(&shown[3..]);
    let known = qm_core::patch::loader::dir_patch_files(dir)
        .iter()
        .any(|p| {
            p.file_name()
                .is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
        });
    if !known || !path.is_file() {
        return Err(not_found(format!("The mod has no patch file {shown}.")));
    }
    Ok((path, shown))
}

/// `qm.toml`'s text (None when there is none) and the hash of its bytes ("" when none).
fn read_qm_toml(dir: &Path) -> QmResult<(Option<String>, String)> {
    match std::fs::read(dir.join("qm.toml")) {
        Ok(bytes) => {
            let f = decode_text("qm.toml", &bytes)?;
            Ok((Some(f.text), f.hash))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((None, String::new())),
        Err(e) => Err(QmError::new(
            ErrorKind::Io,
            format!("qm.toml could not be read: {e}"),
        )),
    }
}

fn parse_doc(name: &str, text: &str) -> QmResult<DocumentMut> {
    text.parse::<DocumentMut>().map_err(|e| {
        invalid(format!(
            "{name} has a mistake, so the studio cannot change it: {}",
            e.message()
        ))
    })
}

/// `text` with a `[manifest]` table at the top when it has none: a patch file needs one,
/// with the version `mod.json` gives (else 1.0.0).
pub(crate) fn with_manifest(dir: &Path, text: &str) -> String {
    let has = text
        .parse::<DocumentMut>()
        .ok()
        .is_some_and(|d| d.get("manifest").is_some());
    if has {
        return text.to_string();
    }
    let version = std::fs::read_to_string(dir.join("mod.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("version")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        })
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "1.0.0".to_string());
    let head = format!("[manifest]\nversion = {}\n", Value::from(version.as_str()));
    if text.trim().is_empty() {
        head
    } else {
        format!("{head}\n{text}")
    }
}

/// The mod's id: `mod.json`'s, else `[manifest] id` in `qm.toml`, else the folder name.
pub(crate) fn mod_id_of(dir: &Path, folder: &str) -> String {
    let from_json = std::fs::read_to_string(dir.join("mod.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(str::to_string));
    let from_toml = || {
        std::fs::read_to_string(dir.join("qm.toml"))
            .ok()
            .and_then(|t| t.parse::<DocumentMut>().ok())
            .and_then(|d| {
                d.get("manifest")
                    .and_then(|m| m.get("id"))
                    .and_then(Item::as_str)
                    .map(str::to_string)
            })
    };
    from_json
        .or_else(from_toml)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| folder.to_string())
}

/// 1-based runs of lines a patch wrote, with the edit that wrote them.
fn patch_runs(trace: &FileTrace) -> Vec<(u32, u32, &qm_core::patch::origin::JournalEdit)> {
    let mut out: Vec<(u32, u32, &qm_core::patch::origin::JournalEdit)> = Vec::new();
    for (i, o) in trace.lines.iter().enumerate() {
        if !o.is_patch_output() {
            continue;
        }
        let Some(edit) = o.edit().and_then(|e| trace.edits.get(e)) else {
            continue;
        };
        let n = to_u32(i + 1);
        match out.last_mut() {
            Some(last) if last.1 + 1 == n && last.2.patch_id == edit.patch_id => last.1 = n,
            _ => out.push((n, n, edit)),
        }
    }
    out
}

fn changed_runs(trace: &FileTrace, is_draft: &dyn Fn(&str) -> bool) -> Vec<StudioChangedRun> {
    patch_runs(trace)
        .into_iter()
        .map(|(start, end, e)| StudioChangedRun {
            start,
            end,
            mine: is_draft(&e.patch_id),
        })
        .collect()
}

// ---- Reading patches ----

fn str_of(t: &Table, key: &str) -> String {
    t.get(key)
        .and_then(Item::as_str)
        .map(str::to_string)
        .unwrap_or_default()
}

fn list_of(t: &Table, key: &str) -> Vec<String> {
    match t.get(key) {
        Some(item) => match item.as_value() {
            Some(Value::String(s)) => vec![s.value().clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        },
        None => Vec::new(),
    }
}

fn or_entry(own: Vec<String>, entry: Option<&Table>, key: &str) -> Vec<String> {
    if own.is_empty() {
        entry.map(|e| list_of(e, key)).unwrap_or_default()
    } else {
        own
    }
}

fn draft_from(kind: StudioPatchKind, t: &Table, entry: Option<&Table>) -> PatchDraft {
    let from_entry = |k: &str| entry.map(|e| str_of(e, k)).unwrap_or_default();
    let pick = |k: &str| {
        let v = str_of(t, k);
        if v.is_empty() {
            from_entry(k)
        } else {
            v
        }
    };
    let (source, load_before) = match kind {
        StudioPatchKind::Module => (list_of(t, "source"), str_of(t, "before")),
        StudioPatchKind::Copy => (list_of(t, "sources"), String::new()),
        _ => (Vec::new(), String::new()),
    };
    PatchDraft {
        kind: Some(kind),
        id: pick("id"),
        name: pick("name"),
        group: str_of(t, "group"),
        target: list_of(t, "target"),
        pattern: str_of(t, "pattern"),
        position: str_of(t, "position"),
        payload: str_of(t, "payload"),
        match_indent: t.get("match_indent").and_then(Item::as_bool),
        times: t
            .get("times")
            .and_then(Item::as_integer)
            .and_then(|n| u32::try_from(n).ok()),
        requires: list_of(t, "requires"),
        // In the patch's table; an entry-level list (as the schema allows) is read too.
        before: if kind == StudioPatchKind::Module {
            Vec::new()
        } else {
            or_entry(list_of(t, "before"), entry, "before")
        },
        after: or_entry(list_of(t, "after"), entry, "after"),
        on_failure: {
            let v = str_of(t, "on_failure");
            if v.is_empty() {
                from_entry("on_failure")
            } else {
                v
            }
        },
        source,
        load_before,
    }
}

fn drafts_of(doc: &DocumentMut, file: &str, editable: bool, hash: &str) -> Vec<StudioPatch> {
    let mut out = Vec::new();
    if let Some(entries) = doc.get("patches").and_then(Item::as_array_of_tables) {
        for (i, entry) in entries.iter().enumerate() {
            let Some((kind, t)) = StudioPatchKind::ALL
                .iter()
                .find_map(|k| entry.get(k.key()).and_then(Item::as_table).map(|t| (*k, t)))
            else {
                continue;
            };
            out.push(StudioPatch {
                at: StudioPatchRef {
                    file: file.to_string(),
                    form: StudioForm::Long,
                    kind,
                    index: to_u32(i),
                },
                draft: draft_from(kind, t, Some(entry)),
                editable,
                hash: hash.to_string(),
            });
        }
    }
    for kind in StudioPatchKind::ALL {
        if let Some(list) = doc.get(kind.key()).and_then(Item::as_array_of_tables) {
            for (i, t) in list.iter().enumerate() {
                out.push(StudioPatch {
                    at: StudioPatchRef {
                        file: file.to_string(),
                        form: StudioForm::Short,
                        kind,
                        index: to_u32(i),
                    },
                    draft: draft_from(kind, t, None),
                    editable,
                    hash: hash.to_string(),
                });
            }
        }
    }
    out
}

/// The patch table at `at` (the kind's table; for the long form, inside its entry).
fn draft_at<'a>(doc: &'a DocumentMut, at: &StudioPatchRef) -> Option<&'a Table> {
    match at.form {
        StudioForm::Long => doc
            .get("patches")?
            .as_array_of_tables()?
            .get(at.index as usize)?
            .get(at.kind.key())?
            .as_table(),
        StudioForm::Short => doc
            .get(at.kind.key())?
            .as_array_of_tables()?
            .get(at.index as usize),
    }
}

// ---- Writing patches ----

/// A TOML string value: one line as a basic string, several lines as a literal `'''` block
/// (no escapes, as patch payloads are usually written), or a basic block when the text
/// holds `'''`.
fn string_value(s: &str) -> Value {
    if !s.contains('\n') {
        return Value::from(s);
    }
    let body = if s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    };
    let literal = format!("'''\n{body}'''");
    if !body.contains("'''") {
        if let Ok(v) = literal.parse::<Value>() {
            return v;
        }
    }
    Value::from(s)
}

fn list_value(items: &[String]) -> Value {
    let mut a = Array::new();
    for i in items {
        a.push(i.as_str());
    }
    Value::Array(a)
}

/// Sets `key` to `value`, or removes it when the value is empty, keeping the key's place and
/// comments when it is already there.
fn set_key(t: &mut Table, key: &str, value: Option<Value>) {
    match value {
        Some(v) => match t.get_mut(key).and_then(Item::as_value_mut) {
            Some(old) => {
                let decor = old.decor().clone();
                *old = v;
                *old.decor_mut() = decor;
            }
            None => {
                t.insert(key, Item::Value(v));
            }
        },
        None => {
            t.remove(key);
        }
    }
}

fn opt_str(s: &str) -> Option<Value> {
    let s = s.trim_end_matches(['\r']);
    (!s.trim().is_empty()).then(|| string_value(s))
}

fn opt_list(v: &[String]) -> Option<Value> {
    let clean: Vec<String> = v
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    (!clean.is_empty()).then(|| list_value(&clean))
}

/// Writes the keys the form owns into the patch's own table.
fn fill_patch_table(t: &mut Table, d: &PatchDraft, kind: StudioPatchKind) {
    set_key(t, "id", opt_str(d.id.trim()));
    set_key(t, "name", opt_str(d.name.trim()));
    set_key(t, "group", opt_str(d.group.trim()));
    match kind {
        StudioPatchKind::Pattern | StudioPatchKind::Regex => {
            let targets: Vec<String> = d
                .target
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .collect();
            set_key(
                t,
                "target",
                match targets.len() {
                    0 => None,
                    1 => Some(Value::from(targets[0].trim())),
                    _ => Some(list_value(&targets)),
                },
            );
            set_key(t, "pattern", opt_str(&d.pattern));
            set_key(t, "position", opt_str(d.position.trim()));
            set_key(t, "times", d.times.map(|n| Value::from(i64::from(n))));
            if kind == StudioPatchKind::Pattern {
                set_key(t, "match_indent", d.match_indent.map(Value::from));
            }
            set_key(t, "requires", opt_list(&d.requires));
            set_key(t, "before", opt_list(&d.before));
            set_key(t, "after", opt_list(&d.after));
            set_key(t, "on_failure", opt_str(d.on_failure.trim()));
            set_key(t, "payload", Some(string_value(&d.payload)));
        }
        StudioPatchKind::Copy => {
            let targets: Vec<String> = d
                .target
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .collect();
            set_key(
                t,
                "target",
                match targets.len() {
                    0 => None,
                    1 => Some(Value::from(targets[0].trim())),
                    _ => Some(list_value(&targets)),
                },
            );
            set_key(t, "position", opt_str(d.position.trim()));
            set_key(t, "sources", opt_list(&d.source));
            set_key(t, "before", opt_list(&d.before));
            set_key(t, "after", opt_list(&d.after));
            set_key(t, "on_failure", opt_str(d.on_failure.trim()));
            set_key(t, "payload", opt_str(&d.payload));
        }
        StudioPatchKind::Module => {
            set_key(
                t,
                "source",
                d.source.first().and_then(|s| opt_str(s.trim())),
            );
            set_key(t, "before", opt_str(d.load_before.trim()));
        }
    }
}

/// Puts the draft into `doc` (in place of `editing`, else at the end) and returns where it is.
fn insert_draft(
    doc: &mut DocumentMut,
    d: &PatchDraft,
    kind: StudioPatchKind,
    long: bool,
    editing: Option<&StudioPatchRef>,
    file: &str,
) -> QmResult<StudioPatchRef> {
    let stale = || {
        QmError::new(
            ErrorKind::Conflict,
            format!("That patch is no longer where it was in {file}. Open it again."),
        )
    };
    if let Some(at) = editing {
        match at.form {
            StudioForm::Long => {
                let entry = doc
                    .get_mut("patches")
                    .and_then(Item::as_array_of_tables_mut)
                    .and_then(|a| a.get_mut(at.index as usize))
                    .ok_or_else(stale)?;
                // A changed kind replaces the type table.
                if at.kind != kind {
                    entry.remove(at.kind.key());
                }
                if !entry.contains_key(kind.key()) {
                    let mut sub = Table::new();
                    sub.set_implicit(false);
                    entry.insert(kind.key(), Item::Table(sub));
                }
                // Entry-level hints, when the file has them, follow the form's lists.
                for key in ["before", "after"] {
                    if entry.contains_key(key) {
                        let v = if key == "before" { &d.before } else { &d.after };
                        set_key(entry, key, opt_list(v));
                    }
                }
                let sub = entry
                    .get_mut(kind.key())
                    .and_then(Item::as_table_mut)
                    .ok_or_else(stale)?;
                fill_patch_table(sub, d, kind);
                // id and name live in the type table; an entry-level copy would disagree.
                if entry.contains_key("id") {
                    set_key(entry, "id", opt_str(d.id.trim()));
                }
                if entry.contains_key("name") {
                    set_key(entry, "name", opt_str(d.name.trim()));
                }
                return Ok(StudioPatchRef { kind, ..at.clone() });
            }
            StudioForm::Short if at.kind == kind => {
                let t = doc
                    .get_mut(kind.key())
                    .and_then(Item::as_array_of_tables_mut)
                    .and_then(|a| a.get_mut(at.index as usize))
                    .ok_or_else(stale)?;
                fill_patch_table(t, d, kind);
                return Ok(at.clone());
            }
            StudioForm::Short => {
                // Another kind in the short form: remove it and add the new one below.
                let list = doc
                    .get_mut(at.kind.key())
                    .and_then(Item::as_array_of_tables_mut)
                    .filter(|a| (at.index as usize) < a.len())
                    .ok_or_else(stale)?;
                list.remove(at.index as usize);
                if list.is_empty() {
                    doc.remove(at.kind.key());
                }
            }
        }
    }
    let mut t = Table::new();
    fill_patch_table(&mut t, d, kind);
    if long {
        let mut entry = Table::new();
        t.set_implicit(false);
        entry.insert(kind.key(), Item::Table(t));
        entry.decor_mut().set_prefix("\n");
        let list = array_of_tables(doc, "patches")?;
        list.push(entry);
        Ok(StudioPatchRef {
            file: file.to_string(),
            form: StudioForm::Long,
            kind,
            index: to_u32(list.len() - 1),
        })
    } else {
        t.decor_mut().set_prefix("\n");
        let list = array_of_tables(doc, kind.key())?;
        list.push(t);
        Ok(StudioPatchRef {
            file: file.to_string(),
            form: StudioForm::Short,
            kind,
            index: to_u32(list.len() - 1),
        })
    }
}

fn array_of_tables<'a>(doc: &'a mut DocumentMut, key: &str) -> QmResult<&'a mut ArrayOfTables> {
    if !doc.contains_key(key) {
        doc.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    doc.get_mut(key)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| {
            invalid(format!(
                "qm.toml has `{key}` as something other than a list of tables, so a patch cannot be added to it."
            ))
        })
}

/// Lists `short_patches` in `mod.json`'s `qm_features` (see [`add_feature`]).
fn add_short_patches_feature(dir: &Path) -> QmResult<bool> {
    add_feature(dir, "short_patches")
}

/// Lists `feature` in `mod.json`'s `qm_features`, editing the text in place so the file's key
/// order and layout stay. True when it was added; false when it was there, or when there is
/// no `mod.json` (or it does not read).
pub(crate) fn add_feature(dir: &Path, feature: &str) -> QmResult<bool> {
    let path = dir.join("mod.json");
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(false);
    };
    let f = decode_text("mod.json", &bytes)?;
    let Some(edited) = with_feature(&f.text, feature) else {
        return Ok(false);
    };
    write_atomic(&path, &encode_text(&edited, f.crlf, f.bom))?;
    Ok(true)
}

/// `mod.json`'s text with `feature` in `qm_features`, or None when it is there already or the
/// text does not read (only a result that reads, with the feature listed, is given).
pub(crate) fn with_feature(text: &str, feature: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let listed = |v: &serde_json::Value| {
        v.get("qm_features")
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(feature)))
    };
    if listed(&value) {
        return None;
    }
    let edited = insert_feature_named(text, value.get("qm_features").is_some(), feature)?;
    let check = serde_json::from_str::<serde_json::Value>(&edited).ok()?;
    listed(&check).then_some(edited)
}

#[cfg(test)]
fn insert_feature(text: &str, has_key: bool) -> Option<String> {
    insert_feature_named(text, has_key, "short_patches")
}

/// `text` with `feature` added to its `qm_features` list, or the key added after the opening
/// brace.
fn insert_feature_named(text: &str, has_key: bool, feature: &str) -> Option<String> {
    let quoted = serde_json::to_string(feature).ok()?;
    if has_key {
        let key = text.find("\"qm_features\"")?;
        let open = key + text[key..].find('[')?;
        let close = open + text[open..].find(']')?;
        let inside = &text[open + 1..close];
        if inside.trim().is_empty() {
            return Some(format!("{}{quoted}{}", &text[..=open], &text[close..]));
        }
        let trimmed = inside.trim_end();
        let tail = &inside[trimmed.len()..];
        return Some(format!(
            "{}{trimmed}, {quoted}{tail}{}",
            &text[..=open],
            &text[close..]
        ));
    }
    let brace = text.find('{')?;
    let after = &text[brace + 1..];
    let rest = after.trim_start();
    if rest.starts_with('}') {
        return Some(format!(
            "{}\"qm_features\": [{quoted}]{}",
            &text[..=brace],
            after
        ));
    }
    // Indent like the next key.
    let indent: String = after
        .trim_start_matches(['\r', '\n'])
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    Some(format!(
        "{}\n{indent}\"qm_features\": [{quoted}],{}",
        &text[..=brace],
        after
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn draft(kind: StudioPatchKind) -> PatchDraft {
        PatchDraft {
            kind: Some(kind),
            id: "example_mod:hello".to_string(),
            name: "Say hello".to_string(),
            target: vec!["main.lua".to_string()],
            pattern: "function love.load()".to_string(),
            position: "after".to_string(),
            payload: "    print(\"hello\")\n".to_string(),
            ..PatchDraft::default()
        }
    }

    #[test]
    fn a_new_patch_goes_at_the_end_in_the_short_form_with_a_literal_payload() {
        let mut doc: DocumentMut = "# My mod\n[manifest]\nversion = \"1.0.0\"\n"
            .parse()
            .unwrap();
        let at = insert_draft(
            &mut doc,
            &draft(StudioPatchKind::Pattern),
            StudioPatchKind::Pattern,
            false,
            None,
            "qm.toml",
        )
        .unwrap();
        assert_eq!(at.form, StudioForm::Short);
        assert_eq!(at.index, 0);
        let text = doc.to_string();
        assert!(
            text.starts_with("# My mod\n[manifest]\nversion = \"1.0.0\"\n"),
            "{text}"
        );
        assert!(
            text.contains("\n[[pattern]]\nid = \"example_mod:hello\"\n"),
            "{text}"
        );
        assert!(
            text.contains("payload = '''\n    print(\"hello\")\n'''"),
            "{text}"
        );
        // It reads back as the same draft.
        let back = drafts_of(&text.parse().unwrap(), "qm.toml", true, "");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].draft, draft(StudioPatchKind::Pattern));
    }

    #[test]
    fn the_long_form_keeps_order_hints_on_the_entry_and_edits_in_place() {
        let src = "[[patches]]\n# keep me\n[patches.pattern]\nid = \"a\"\ntarget = \"main.lua\"\npattern = \"x\" # the anchor\nposition = \"at\"\npayload = \"y\"\nstack = \"own\"\n";
        let mut doc: DocumentMut = src.parse().unwrap();
        let found = drafts_of(&doc, "qm.toml", true, "");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].at.form, StudioForm::Long);
        let mut d = found[0].draft.clone();
        d.payload = "z".to_string();
        d.before = vec!["other:b".to_string()];
        insert_draft(
            &mut doc,
            &d,
            StudioPatchKind::Pattern,
            true,
            Some(&found[0].at),
            "qm.toml",
        )
        .unwrap();
        let text = doc.to_string();
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("pattern = \"x\" # the anchor"), "{text}");
        assert!(
            text.contains("stack = \"own\""),
            "unknown keys stay: {text}"
        );
        assert!(text.contains("payload = \"z\""), "{text}");
        // Order hints go in the patch's own table, where the loader reads them.
        let table = text.split("[patches.pattern]").nth(1).unwrap();
        assert!(table.contains("before = [\"other:b\"]"), "{text}");
        assert!(
            !text
                .split("[patches.pattern]")
                .next()
                .unwrap()
                .contains("before"),
            "{text}"
        );
        let back = drafts_of(&text.parse().unwrap(), "qm.toml", true, "");
        assert_eq!(back[0].draft.before, vec!["other:b".to_string()]);
    }

    #[test]
    fn module_and_copy_patches_round_trip() {
        let src = "[[module]]\nid = \"m:main\"\nsource = \"main.lua\"\nname = \"includes/m\"\nbefore = \"main.lua\"\n\n[[copy]]\nid = \"m:copy\"\ntarget = \"conf.lua\"\nposition = \"append\"\nsources = [\"extra.lua\"]\n";
        let doc: DocumentMut = src.parse().unwrap();
        let found = drafts_of(&doc, "qm.toml", true, "");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].draft.kind, Some(StudioPatchKind::Module));
        assert_eq!(found[0].draft.source, vec!["main.lua".to_string()]);
        assert_eq!(found[0].draft.load_before, "main.lua");
        assert_eq!(found[1].draft.source, vec!["extra.lua".to_string()]);
        let mut doc2 = doc.clone();
        insert_draft(
            &mut doc2,
            &found[0].draft,
            StudioPatchKind::Module,
            false,
            Some(&found[0].at),
            "qm.toml",
        )
        .unwrap();
        insert_draft(
            &mut doc2,
            &found[1].draft,
            StudioPatchKind::Copy,
            false,
            Some(&found[1].at),
            "qm.toml",
        )
        .unwrap();
        assert_eq!(doc2.to_string(), src);
    }

    #[test]
    fn short_patches_is_added_to_mod_json_in_place() {
        let pretty = "{\n  \"id\": \"m\",\n  \"name\": \"M\"\n}\n";
        assert_eq!(
            insert_feature(pretty, false).unwrap(),
            "{\n  \"qm_features\": [\"short_patches\"],\n  \"id\": \"m\",\n  \"name\": \"M\"\n}\n"
        );
        let listed = "{ \"id\": \"m\", \"qm_features\": [\"config_submenus\"] }";
        assert_eq!(
            insert_feature(listed, true).unwrap(),
            "{ \"id\": \"m\", \"qm_features\": [\"config_submenus\", \"short_patches\"] }"
        );
        assert_eq!(
            insert_feature("{\"qm_features\": []}", true).unwrap(),
            "{\"qm_features\": [\"short_patches\"]}"
        );
        assert_eq!(
            insert_feature("{}", false).unwrap(),
            "{\"qm_features\": [\"short_patches\"]}"
        );
    }

    #[test]
    fn a_payload_with_triple_quotes_stays_a_basic_string() {
        let v = string_value("a '''\nb");
        assert!(v.as_str().is_some_and(|s| s == "a '''\nb"));
        // Not a literal block, which would end at the text's own quotes.
        assert!(!v.to_string().trim_start().starts_with("'''"));
    }
}
