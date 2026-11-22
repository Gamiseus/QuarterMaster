//! The Mod Engine's backend (sub-project 3): the mods in the workspace mods folder, their
//! files, live checks with the loader's own rules, and copying a library mod into the
//! workspace to edit it.
//!
//! A workspace mod is named by its folder: a direct child of the workspace mods folder that
//! is not a link and does not start with `.` or `_`. A file is a relative path inside it. No
//! absolute path ever comes from the UI, and a path that leaves the mod through a link is
//! refused.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::service::Service;
use qm_manager::library::validate_safe_relative_path;
use qm_manager::profiles::is_reparse_point;
use qm_schema::{DiagnosticSeverity, ManifestDiagnostic, ModManifest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Largest file the editor opens.
pub const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;
/// Most entries a file tree carries.
pub const MAX_TREE_ENTRIES: usize = 5000;

/// What a workspace mod is, from the files it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EngineModKind {
    /// Has a `qm.toml` (or patch files under `qm/`), or only a `mod.json`.
    Patch,
    /// Has a `faction.toml`.
    Faction,
    /// Has only an `assets/` folder.
    Assets,
    /// Has none of the mod files yet.
    Unmanaged,
}

/// The counts of a project's last full check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineCheckCounts {
    pub errors: u32,
    pub warnings: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineProject {
    /// The folder's name in the workspace: how the Mod Engine names the mod.
    pub folder: String,
    /// From `mod.json`, else the folder name.
    pub name: String,
    pub id: Option<String>,
    pub version: Option<String>,
    pub kind: EngineModKind,
    /// A library mod has the same id.
    pub in_library: bool,
    /// `None` until the project was checked in this session.
    pub last_check: Option<EngineCheckCounts>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineProjects {
    /// False when no workspace mods folder is set (`projects` is then empty).
    pub workspace_set: bool,
    pub projects: Vec<EngineProject>,
}

/// An installed library mod, for the read-only list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineLibraryMod {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EngineFileKind {
    Lua,
    Toml,
    Json,
    Markdown,
    Text,
    Image,
    Audio,
    Other,
}

impl EngineFileKind {
    fn of(path: &str) -> Self {
        let ext = path
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "lua" => Self::Lua,
            "toml" => Self::Toml,
            "json" => Self::Json,
            "md" => Self::Markdown,
            "txt" | "cfg" | "ini" | "csv" | "glsl" => Self::Text,
            "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tga" => Self::Image,
            "ogg" | "wav" | "mp3" | "flac" => Self::Audio,
            _ => Self::Other,
        }
    }

    /// Opened in the editor as text.
    fn is_text(self) -> bool {
        matches!(
            self,
            Self::Lua | Self::Toml | Self::Json | Self::Markdown | Self::Text
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineEntry {
    /// Relative to the mod's folder, with `/`.
    pub path: String,
    pub folder: bool,
    pub size_bytes: u64,
    pub kind: EngineFileKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineTree {
    pub entries: Vec<EngineEntry>,
    /// More entries than `MAX_TREE_ENTRIES` were found and left out.
    pub truncated: bool,
}

/// A file opened for editing. `text` always has `\n` line breaks and no byte order mark;
/// `crlf` and `bom` say how it is written back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineFile {
    pub path: String,
    pub text: String,
    /// SHA-256 of the bytes on disk, for the save's change check.
    pub hash: String,
    pub crlf: bool,
    pub bom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineWriteRequest {
    pub path: String,
    pub text: String,
    /// The hash the file had when it was opened; `None` for a new file or an overwrite.
    pub expected_hash: Option<String>,
    pub crlf: bool,
    pub bom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EngineSeverity {
    Error,
    Warning,
    Info,
}

impl From<DiagnosticSeverity> for EngineSeverity {
    fn from(s: DiagnosticSeverity) -> Self {
        match s {
            DiagnosticSeverity::Error => Self::Error,
            DiagnosticSeverity::Warning => Self::Warning,
            DiagnosticSeverity::Info => Self::Info,
        }
    }
}

/// One problem. Lines and columns are 1-based; `None` when the check has no place for it
/// (the message is about the whole file or the whole mod).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineDiagnostic {
    /// The file it is in; `None` for the mod as a whole.
    pub path: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
    pub severity: EngineSeverity,
    pub message: String,
    /// What found it: `toml`, `json`, `lua`, `patches`, `manifest`, `config`, `faction`, `lint`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineCheck {
    pub diagnostics: Vec<EngineDiagnostic>,
    /// False for a Lua file when the game's `lua51.dll` was not found.
    pub lua_checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineLint {
    pub passed: bool,
    pub diagnostics: Vec<EngineDiagnostic>,
    pub patch_count: u32,
    pub matched_patches: u32,
    pub zero_match_patches: u32,
    /// The capabilities the security scan found, as the launcher names them.
    pub capabilities: Vec<String>,
    /// False when the game's files are not cached, so no pattern was tried.
    pub game_checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EngineSymbolKind {
    Function,
    Table,
}

/// A global the game's own code defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EngineSymbol {
    /// `name`, or `Table.name` for a function in a table.
    pub name: String,
    pub kind: EngineSymbolKind,
    /// `function(a, b)` for a function; empty for a table.
    pub signature: String,
    pub file: String,
    pub line: u32,
}

/// Session state of the Mod Engine, kept in the service.
#[derive(Default)]
pub(crate) struct EngineState {
    /// Folder -> counts of its last full check.
    pub checks: std::collections::HashMap<String, EngineCheckCounts>,
    /// The game's symbols, with the game hash they were read for.
    pub symbols: Option<(String, Arc<Vec<EngineSymbol>>)>,
    /// The game's images by global, for the sprite line-up, with the game hash.
    pub images: Option<(String, Arc<crate::ops::sprite_lineup::GameImages>)>,
}

impl Service {
    /// The workspace mods folder, or `NotFound`.
    pub(crate) fn engine_workspace(&self) -> QmResult<PathBuf> {
        let ws = self
            .paths()
            .load_settings()
            .workspace_mods_dir
            .ok_or_else(|| {
                not_found("No workspace mods folder is set. Pick one in Settings, Paths.")
            })?;
        if !ws.is_dir() {
            return Err(not_found(format!(
                "The workspace mods folder is missing: {}",
                ws.display()
            )));
        }
        Ok(ws)
    }

    /// [`Service::engine_project_dir`] for the shell, which opens the folder in Explorer.
    pub fn engine_project_dir_public(&self, folder: &str) -> QmResult<PathBuf> {
        self.engine_project_dir(folder)
    }

    /// The folder of workspace mod `folder`, checked as the module comment says.
    pub(crate) fn engine_project_dir(&self, folder: &str) -> QmResult<PathBuf> {
        let ws = self.engine_workspace()?;
        let clean = validate_safe_relative_path(folder).map_err(invalid)?;
        let mut parts = clean.components();
        let one = parts.next().is_some() && parts.next().is_none();
        if !one || folder.starts_with('.') || folder.starts_with('_') || folder == "qm" {
            return Err(invalid(format!(
                "'{folder}' is not a mod folder of the workspace."
            )));
        }
        let dir = ws.join(&clean);
        if is_reparse_point(&dir) {
            return Err(invalid(format!(
                "'{folder}' is a link; the Mod Engine edits only real folders in the workspace."
            )));
        }
        if !dir.is_dir() {
            return Err(not_found(format!(
                "The workspace has no mod folder '{folder}'."
            )));
        }
        Ok(dir)
    }

    /// `rel` inside the mod folder `dir`. No part of the way may be a link, so the path
    /// cannot leave the folder.
    pub(crate) fn engine_file_path(dir: &Path, rel: &str) -> QmResult<(PathBuf, String)> {
        let clean = validate_safe_relative_path(rel).map_err(invalid)?;
        let mut at = dir.to_path_buf();
        for part in clean.components() {
            at.push(part);
            if is_reparse_point(&at) {
                return Err(invalid(format!(
                    "'{rel}' goes through a link; the Mod Engine edits only files inside the mod's folder."
                )));
            }
        }
        let shown = clean.to_string_lossy().replace('\\', "/");
        Ok((at, shown))
    }

    /// The workspace's mods, sorted by name.
    pub fn engine_projects(&self) -> QmResult<EngineProjects> {
        let ws = match self.paths().load_settings().workspace_mods_dir {
            Some(ws) if ws.is_dir() => ws,
            Some(ws) => {
                return Err(not_found(format!(
                    "The workspace mods folder is missing: {}",
                    ws.display()
                )))
            }
            None => {
                return Ok(EngineProjects {
                    workspace_set: false,
                    projects: Vec::new(),
                })
            }
        };
        let library: BTreeSet<String> = self
            .paths()
            .list_installed_mods()
            .into_iter()
            .filter(|m| !m.is_workspace_linked)
            .map(|m| m.manifest.id)
            .collect();
        let checks = self.engine_state().checks.clone();
        let mut projects = Vec::new();
        for entry in std::fs::read_dir(&ws).or_kind(ErrorKind::Io)?.flatten() {
            let path = entry.path();
            let folder = entry.file_name().to_string_lossy().into_owned();
            if folder.starts_with('.') || folder.starts_with('_') || folder == "qm" {
                continue;
            }
            if !path.is_dir() || is_reparse_point(&path) {
                continue;
            }
            let manifest = ModManifest::from_path(&path.join("mod.json")).ok();
            let kind = project_kind(&path);
            let version = manifest.as_ref().map(|m| m.version.clone()).or_else(|| {
                std::fs::read_to_string(path.join("qm.toml"))
                    .ok()
                    .and_then(|t| toml::from_str::<qm_core::patch::PatchFile>(&t).ok())
                    .map(|p| p.manifest.version)
            });
            projects.push(EngineProject {
                name: manifest
                    .as_ref()
                    .map(|m| m.name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| folder.clone()),
                in_library: manifest.as_ref().is_some_and(|m| library.contains(&m.id)),
                id: manifest.map(|m| m.id),
                version,
                kind,
                last_check: checks.get(&folder).copied(),
                folder,
            });
        }
        projects.sort_by_cached_key(|p| (p.name.to_ascii_lowercase(), p.folder.clone()));
        Ok(EngineProjects {
            workspace_set: true,
            projects,
        })
    }

    /// The library's own mods (not the links to workspace folders), by name.
    pub fn engine_library_mods(&self) -> Vec<EngineLibraryMod> {
        let mut out: Vec<EngineLibraryMod> = self
            .paths()
            .list_installed_mods()
            .into_iter()
            .filter(|m| !m.is_workspace_linked)
            .map(|m| EngineLibraryMod {
                id: m.manifest.id,
                name: m.manifest.name,
                version: m.manifest.version,
            })
            .collect();
        out.sort_by_cached_key(|m| m.name.to_ascii_lowercase());
        out
    }

    /// The mod's files and folders. Dot names and links are left out.
    pub fn engine_tree(&self, folder: &str) -> QmResult<EngineTree> {
        let root = self.engine_project_dir(folder)?;
        let mut entries = Vec::new();
        let mut truncated = false;
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let path = e.path();
                if e.file_name().to_string_lossy().starts_with('.') || is_reparse_point(&path) {
                    continue;
                }
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                let Ok(rel) = path.strip_prefix(&root) else {
                    continue;
                };
                if entries.len() >= MAX_TREE_ENTRIES {
                    truncated = true;
                    break;
                }
                let rel = rel.to_string_lossy().replace('\\', "/");
                if meta.is_dir() {
                    stack.push(path);
                    entries.push(EngineEntry {
                        kind: EngineFileKind::Other,
                        path: rel,
                        folder: true,
                        size_bytes: 0,
                    });
                } else if meta.is_file() {
                    entries.push(EngineEntry {
                        kind: EngineFileKind::of(&rel),
                        path: rel,
                        folder: false,
                        size_bytes: meta.len(),
                    });
                }
            }
        }
        entries.sort_by_cached_key(|e| e.path.to_ascii_lowercase());
        Ok(EngineTree { entries, truncated })
    }

    /// A text file of the mod, for the editor.
    pub fn engine_file_read(&self, folder: &str, path: &str) -> QmResult<EngineFile> {
        let dir = self.engine_project_dir(folder)?;
        let (file, shown) = Self::engine_file_path(&dir, path)?;
        let meta = std::fs::metadata(&file)
            .map_err(|_| not_found(format!("No file '{shown}' in {folder}.")))?;
        if !meta.is_file() {
            return Err(invalid(format!("'{shown}' is a folder.")));
        }
        let kind = EngineFileKind::of(&shown);
        // An unknown extension is tried as text; images and sounds never are.
        if !kind.is_text() && kind != EngineFileKind::Other {
            return Err(invalid(format!("'{shown}' is not a text file.")));
        }
        if meta.len() > MAX_EDIT_BYTES {
            return Err(invalid(format!(
                "'{shown}' is over {} MB, too large to edit here. Open it in Explorer.",
                MAX_EDIT_BYTES / 1024 / 1024
            )));
        }
        let bytes = std::fs::read(&file).or_kind(ErrorKind::Io)?;
        decode_text(&shown, &bytes)
    }

    /// Writes a file of the mod through a temp file and a rename. A file that changed on
    /// disk since it was opened is not overwritten (`Conflict`). Returns the new state.
    pub fn engine_file_write(&self, folder: &str, req: EngineWriteRequest) -> QmResult<EngineFile> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let (file, shown) = Self::engine_file_path(&dir, &req.path)?;
        if file.is_dir() {
            return Err(invalid(format!("'{shown}' is a folder.")));
        }
        if let Some(expected) = &req.expected_hash {
            match std::fs::read(&file) {
                Ok(now) if !qm_schema::sha256_hex(&now).eq_ignore_ascii_case(expected) => {
                    return Err(QmError::new(
                        ErrorKind::Conflict,
                        format!("'{shown}' changed on disk since it was opened."),
                    ))
                }
                Err(_) => {
                    return Err(QmError::new(
                        ErrorKind::Conflict,
                        format!("'{shown}' was deleted or moved since it was opened."),
                    ))
                }
                Ok(_) => {}
            }
        }
        let bytes = encode_text(&req.text, req.crlf, req.bom);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).or_kind(ErrorKind::Io)?;
        }
        write_atomic(&file, &bytes)?;
        Ok(EngineFile {
            path: shown,
            text: req.text.replace("\r\n", "\n"),
            hash: qm_schema::sha256_hex(&bytes),
            crlf: req.crlf,
            bom: req.bom,
        })
    }

    /// A new empty file or folder in the mod; `Conflict` when the name is taken.
    pub fn engine_file_create(&self, folder: &str, path: &str, is_folder: bool) -> QmResult<()> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let (target, shown) = Self::engine_file_path(&dir, path)?;
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("'{shown}' already exists."),
            ));
        }
        if is_folder {
            std::fs::create_dir_all(&target).or_kind(ErrorKind::Io)
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).or_kind(ErrorKind::Io)?;
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map(drop)
                .or_kind(ErrorKind::Io)
        }
    }

    /// Renames or moves a file or folder within the mod.
    pub fn engine_file_rename(&self, folder: &str, from: &str, to: &str) -> QmResult<()> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let (src, src_shown) = Self::engine_file_path(&dir, from)?;
        let (dst, dst_shown) = Self::engine_file_path(&dir, to)?;
        if std::fs::symlink_metadata(&src).is_err() {
            return Err(not_found(format!("No file '{src_shown}' in {folder}.")));
        }
        if dst.starts_with(&src) && dst != src {
            return Err(invalid(format!("'{src_shown}' cannot move into itself.")));
        }
        // A rename that only changes the case of the name is the same file on Windows.
        let same_file = src_shown.eq_ignore_ascii_case(&dst_shown);
        if std::fs::symlink_metadata(&dst).is_ok() && !same_file {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("'{dst_shown}' already exists."),
            ));
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).or_kind(ErrorKind::Io)?;
        }
        std::fs::rename(&src, &dst).or_kind(ErrorKind::Io)
    }

    /// Moves a file or folder of the mod to the Recycle Bin.
    pub fn engine_file_delete(&self, folder: &str, path: &str) -> QmResult<()> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let (target, shown) = Self::engine_file_path(&dir, path)?;
        if std::fs::symlink_metadata(&target).is_err() {
            return Err(not_found(format!("No file '{shown}' in {folder}.")));
        }
        if self
            .inner
            .engine_delete_for_good
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return if target.is_dir() {
                std::fs::remove_dir_all(&target).or_kind(ErrorKind::Io)
            } else {
                std::fs::remove_file(&target).or_kind(ErrorKind::Io)
            };
        }
        crate::recycle::to_recycle_bin(&target).map_err(|e| {
            QmError::new(
                ErrorKind::Io,
                format!("'{shown}' could not be moved to the Recycle Bin."),
            )
            .with_details(e)
        })
    }

    /// The fast checks of a file's text as the editor holds it, with the loader's rules.
    pub fn engine_check(&self, folder: &str, path: &str, text: &str) -> QmResult<EngineCheck> {
        let dir = self.engine_project_dir(folder)?;
        let (_, shown) = Self::engine_file_path(&dir, path)?;
        let mut lua_checked = true;
        let lower = shown.to_ascii_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower).to_string();
        let diagnostics = match EngineFileKind::of(&shown) {
            EngineFileKind::Toml if is_patch_file(&lower) => patch_diagnostics(&shown, text),
            EngineFileKind::Toml if lower == "config.toml" => config_diagnostics(&shown, text),
            EngineFileKind::Toml if lower == qm_schema::faction::FACTION_FILE => {
                faction_diagnostics(&shown, text, &dir)
            }
            EngineFileKind::Toml => toml_syntax(&shown, text, "toml"),
            EngineFileKind::Json if name == "mod.json" && lower == "mod.json" => {
                manifest_diagnostics(&shown, text)
            }
            EngineFileKind::Json => json_syntax(&shown, text),
            EngineFileKind::Lua => match self.lua_compiler() {
                Ok(lua) => match lua.compile(&shown, text) {
                    Ok(()) => Vec::new(),
                    Err(e) => vec![lua_diagnostic(&shown, &e)],
                },
                Err(_) => {
                    lua_checked = false;
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        Ok(EngineCheck {
            diagnostics,
            lua_checked,
        })
    }

    /// The full check of a mod, the same as `qm lint`, with findings placed in their files
    /// where they name one. Stored as the project's last check.
    pub fn engine_lint(&self, folder: &str) -> QmResult<EngineLint> {
        let dir = self.engine_project_dir(folder)?;
        let vanilla = self.vanilla_files();
        let empty = std::collections::HashMap::new();
        let files = vanilla.as_ref().map(|v| &v.files).unwrap_or(&empty);
        let report = qm_manager::lint_mod_directory(&dir, files).map_err(|e| {
            QmError::new(ErrorKind::InvalidInput, "The mod could not be checked.").with_details(e)
        })?;
        let patch_text = std::fs::read_to_string(dir.join("qm.toml")).unwrap_or_default();
        let diagnostics: Vec<EngineDiagnostic> = report
            .diagnostics
            .iter()
            .map(|d| place_lint_finding(d, &patch_text))
            .collect();
        let counts = EngineCheckCounts {
            errors: count(&diagnostics, EngineSeverity::Error),
            warnings: count(&diagnostics, EngineSeverity::Warning),
        };
        self.engine_state_mut()
            .checks
            .insert(folder.to_string(), counts);
        Ok(EngineLint {
            passed: report.passed,
            patch_count: u32::try_from(report.patch_count).unwrap_or(u32::MAX),
            matched_patches: u32::try_from(report.matched_patches).unwrap_or(u32::MAX),
            zero_match_patches: u32::try_from(report.zero_match_patches.len()).unwrap_or(u32::MAX),
            capabilities: report
                .security_report
                .capabilities
                .iter()
                .map(|c| c.label().to_string())
                .collect(),
            game_checked: vanilla.is_some(),
            diagnostics,
        })
    }

    /// The game's global functions and tables, read from the cached game files once per game
    /// version. Empty while the game files are not cached.
    pub fn engine_symbols(&self) -> Arc<Vec<EngineSymbol>> {
        let Some(vanilla) = self.vanilla_files() else {
            return Arc::new(Vec::new());
        };
        let key = vanilla
            .info
            .as_ref()
            .map(|i| i.exe_sha256.clone())
            .unwrap_or_else(|| vanilla.game_version.clone());
        if let Some((k, s)) = &self.engine_state().symbols {
            if *k == key {
                return s.clone();
            }
        }
        let mut symbols = Vec::new();
        let mut names: Vec<&String> = vanilla.files.keys().collect();
        names.sort();
        for file in names {
            if let Some(text) = vanilla.files.get(file) {
                game_symbols(file, text, &mut symbols);
            }
        }
        symbols.sort_by(|a, b| a.name.cmp(&b.name).then(a.file.cmp(&b.file)));
        symbols.dedup_by(|a, b| a.name == b.name);
        let symbols = Arc::new(symbols);
        self.engine_state_mut().symbols = Some((key, symbols.clone()));
        symbols
    }

    /// Copies a library mod's version into a new workspace folder named after the mod, so it
    /// can be edited. The launcher's integrity record is left out. Returns the folder.
    pub fn engine_copy_to_workspace(&self, mod_id: &str, version: &str) -> QmResult<String> {
        self.ensure_no_install()?;
        let ws = self.engine_workspace()?;
        let installed = self
            .paths()
            .get_mod_version(mod_id, Some(version))
            .filter(|m| m.manifest.version == version)
            .ok_or_else(|| not_found(format!("Mod '{mod_id}' {version} is not installed.")))?;
        if installed.is_workspace_linked || is_reparse_point(&installed.install_path) {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!("{} is already a workspace folder.", installed.manifest.name),
            ));
        }
        let base = folder_name_for(&installed.manifest.name, mod_id);
        let mut folder = base.clone();
        let mut n = 2;
        while std::fs::symlink_metadata(ws.join(&folder)).is_ok() {
            folder = format!("{base}-{n}");
            n += 1;
        }
        let target = ws.join(&folder);
        qm_manager::library::copy_directory_skipping(&installed.install_path, &target, &|rel| {
            rel.components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        })
        .map_err(|e| {
            drop(std::fs::remove_dir_all(&target));
            QmError::new(ErrorKind::Io, "The mod could not be copied.").with_details(e)
        })?;
        if self.paths().workspace_mode {
            self.invalidate(Scope::Mods);
        }
        Ok(folder)
    }

    pub(crate) fn engine_state(&self) -> std::sync::MutexGuard<'_, EngineState> {
        self.inner.engine.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn engine_state_mut(&self) -> std::sync::MutexGuard<'_, EngineState> {
        self.engine_state()
    }
}

fn not_found(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::NotFound, msg)
}

fn invalid(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}

fn count(d: &[EngineDiagnostic], s: EngineSeverity) -> u32 {
    u32::try_from(d.iter().filter(|x| x.severity == s).count()).unwrap_or(u32::MAX)
}

fn project_kind(dir: &Path) -> EngineModKind {
    if qm_schema::faction::has_faction_file(dir) {
        EngineModKind::Faction
    } else if !qm_core::patch::loader::dir_patch_files(dir).is_empty()
        || dir.join("mod.json").is_file()
    {
        EngineModKind::Patch
    } else if qm_manager::library::has_supported_assets_dir(dir) {
        EngineModKind::Assets
    } else {
        EngineModKind::Unmanaged
    }
}

/// A folder name from a mod's name: letters, digits, `_` and `-`, else the id.
fn folder_name_for(name: &str, id: &str) -> String {
    let clean: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        .collect();
    if clean.is_empty() || clean.starts_with(['_', '.']) {
        id.to_string()
    } else {
        clean
    }
}

/// The files the loader reads as patch files: `qm.toml` or `quartermaster.toml` at the root,
/// and every `.toml` under `qm/`.
fn is_patch_file(lower: &str) -> bool {
    lower == "qm.toml"
        || lower == "quartermaster.toml"
        || (lower.starts_with("qm/") && lower.ends_with(".toml"))
}

pub(crate) fn decode_text(path: &str, bytes: &[u8]) -> QmResult<EngineFile> {
    let hash = qm_schema::sha256_hex(bytes);
    let (bom, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(rest) => (true, rest),
        None => (false, bytes),
    };
    if body.contains(&0) {
        return Err(invalid(format!("'{path}' is not a text file.")));
    }
    let text = std::str::from_utf8(body).map_err(|_| {
        invalid(format!(
            "'{path}' is not UTF-8 text, so it cannot be edited here."
        ))
    })?;
    Ok(EngineFile {
        path: path.to_string(),
        crlf: text.contains("\r\n"),
        text: text.replace("\r\n", "\n"),
        hash,
        bom,
    })
}

pub(crate) fn encode_text(text: &str, crlf: bool, bom: bool) -> Vec<u8> {
    let unix = text.replace("\r\n", "\n");
    let body = if crlf {
        unix.replace('\n', "\r\n")
    } else {
        unix
    };
    let mut out = Vec::with_capacity(body.len() + 3);
    if bom {
        out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    out.extend_from_slice(body.as_bytes());
    out
}

pub(crate) fn write_atomic(target: &Path, bytes: &[u8]) -> QmResult<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = target.with_file_name(format!(".{name}.{}-{seq}.qm-tmp", std::process::id()));
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, target));
    if let Err(e) = written {
        drop(std::fs::remove_file(&tmp));
        return Err(QmError::new(
            ErrorKind::Io,
            format!("The file could not be saved: {e}"),
        ));
    }
    Ok(())
}

/// The 1-based line and column of byte `offset` in `text` (columns count characters).
fn line_col(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    let col = before.get(start..).map_or(0, |s| s.chars().count()) + 1;
    (to_u32(line), to_u32(col))
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn at_span(
    path: &str,
    text: &str,
    span: Option<std::ops::Range<usize>>,
    severity: EngineSeverity,
    message: String,
    source: &str,
) -> EngineDiagnostic {
    let (line, column, end_line, end_column) = match span {
        Some(r) => {
            let (l, c) = line_col(text, r.start);
            let (el, ec) = line_col(text, r.end.max(r.start + 1));
            (Some(l), Some(c), Some(el), Some(ec))
        }
        None => (None, None, None, None),
    };
    EngineDiagnostic {
        path: Some(path.to_string()),
        line,
        column,
        end_line,
        end_column,
        severity,
        message,
        source: source.to_string(),
    }
}

fn whole_file(path: &str, d: &ManifestDiagnostic, source: &str) -> EngineDiagnostic {
    EngineDiagnostic {
        path: Some(path.to_string()),
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        severity: d.severity.clone().into(),
        message: d.message.clone(),
        source: source.to_string(),
    }
}

fn toml_syntax(path: &str, text: &str, source: &str) -> Vec<EngineDiagnostic> {
    match text.parse::<toml::Table>() {
        Ok(_) => Vec::new(),
        Err(e) => vec![at_span(
            path,
            text,
            e.span(),
            EngineSeverity::Error,
            e.message().trim_end().to_string(),
            source,
        )],
    }
}

fn patch_diagnostics(path: &str, text: &str) -> Vec<EngineDiagnostic> {
    qm_core::patch::loader::check_patch_text(text)
        .into_iter()
        .map(|p| {
            let severity = if p.is_error {
                EngineSeverity::Error
            } else {
                EngineSeverity::Warning
            };
            at_span(path, text, p.span, severity, p.message, "patches")
        })
        .collect()
}

fn config_diagnostics(path: &str, text: &str) -> Vec<EngineDiagnostic> {
    let syntax = toml_syntax(path, text, "toml");
    if !syntax.is_empty() {
        return syntax;
    }
    match qm_schema::ModConfigSchema::from_toml_str(text) {
        Ok(_) => Vec::new(),
        Err(e) => vec![at_span(
            path,
            text,
            None,
            EngineSeverity::Error,
            e,
            "config",
        )],
    }
}

fn faction_diagnostics(path: &str, text: &str, dir: &Path) -> Vec<EngineDiagnostic> {
    let syntax = toml_syntax(path, text, "toml");
    if !syntax.is_empty() {
        return syntax;
    }
    qm_schema::faction::check_faction(text, dir)
        .diagnostics
        .iter()
        .map(|d| {
            let mut out = whole_file(path, d, "faction");
            // "faction.toml: units.base: ..." names a table; point at it when it is there.
            let msg = d.message.trim_start_matches("faction.toml: ");
            if let Some((key, _)) = msg.split_once(": ") {
                if let Some(at) = find_table(text, key) {
                    let (l, c) = line_col(text, at);
                    out.line = Some(l);
                    out.column = Some(c);
                }
            }
            out.message = msg.to_string();
            out
        })
        .collect()
}

/// The byte offset of the `[key]` or `[[key]]` header in `text`.
fn find_table(text: &str, key: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t == format!("[{key}]") || t == format!("[[{key}]]") {
            return Some(offset + (line.len() - line.trim_start().len()));
        }
        offset += line.len();
    }
    None
}

fn json_syntax(path: &str, text: &str) -> Vec<EngineDiagnostic> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(_) => Vec::new(),
        Err(e) => vec![json_error(path, &e)],
    }
}

fn json_error(path: &str, e: &serde_json::Error) -> EngineDiagnostic {
    let msg = e.to_string();
    // serde_json appends " at line L column C"; the place is given separately.
    let message = msg
        .rsplit_once(" at line ")
        .map_or(msg.as_str(), |(m, _)| m)
        .to_string();
    EngineDiagnostic {
        path: Some(path.to_string()),
        line: Some(to_u32(e.line().max(1))),
        column: Some(to_u32(e.column().max(1))),
        end_line: None,
        end_column: None,
        severity: EngineSeverity::Error,
        message,
        source: "json".to_string(),
    }
}

fn manifest_diagnostics(path: &str, text: &str) -> Vec<EngineDiagnostic> {
    if let Err(e) = serde_json::from_str::<serde_json::Value>(text) {
        return vec![json_error(path, &e)];
    }
    match serde_json::from_str::<ModManifest>(text) {
        Err(e) => vec![json_error(path, &e)],
        Ok(mut m) => {
            m.normalize();
            m.validate()
                .iter()
                .map(|d| {
                    let mut out = whole_file(path, d, "manifest");
                    // A message naming a key in backticks points at that key.
                    if let Some(key) = d.message.split('`').nth(1) {
                        if let Some(at) = text.find(&format!("\"{key}\"")) {
                            let (l, c) = line_col(text, at);
                            out.line = Some(l);
                            out.column = Some(c);
                        }
                    }
                    out
                })
                .collect()
        }
    }
}

fn lua_diagnostic(path: &str, e: &qm_core::compile::CompileError) -> EngineDiagnostic {
    let mut message = e.message.clone();
    if let Some(open) = e.opened_at {
        message.push_str(&format!(" (opened at line {open})"));
    }
    EngineDiagnostic {
        path: Some(path.to_string()),
        line: e.line.map(to_u32),
        column: None,
        end_line: None,
        end_column: None,
        severity: EngineSeverity::Error,
        message,
        source: "lua".to_string(),
    }
}

/// A `qm lint` finding, placed in `qm.toml` when it names a patch found there.
fn place_lint_finding(d: &ManifestDiagnostic, patch_text: &str) -> EngineDiagnostic {
    let mut out = EngineDiagnostic {
        path: None,
        line: None,
        column: None,
        end_line: None,
        end_column: None,
        severity: d.severity.clone().into(),
        message: d.message.clone(),
        source: "lint".to_string(),
    };
    let lower = d.message.to_ascii_lowercase();
    if lower.starts_with("faction.toml:") {
        out.path = Some(qm_schema::faction::FACTION_FILE.to_string());
        return out;
    }
    if lower.contains("mod.json") || lower.starts_with("manifest") {
        out.path = Some("mod.json".to_string());
    }
    // "Patch 'name' (id) had 0 matches on 'file'.": find the patch's id or name in qm.toml.
    if let Some(rest) = d.message.strip_prefix("Patch '") {
        if let Some((name, after)) = rest.split_once("' (") {
            let id = after.split_once(')').map_or("", |(i, _)| i);
            let short = id.rsplit_once(':').map_or(id, |(_, s)| s);
            let found = [
                format!("\"{id}\""),
                format!("\"{short}\""),
                format!("\"{name}\""),
            ]
            .iter()
            .find_map(|needle| patch_text.find(needle.as_str()));
            out.path = Some("qm.toml".to_string());
            if let Some(at) = found {
                let (l, c) = line_col(patch_text, at);
                out.line = Some(l);
                out.column = Some(c);
            }
        }
    }
    if lower.contains("qm.toml") && out.path.is_none() {
        out.path = Some("qm.toml".to_string());
    }
    out
}

/// The global functions and tables defined at the start of a line in a game file:
/// `function name(...)`, `function T.name(...)`, `name = function(...)` and `name = {`.
fn game_symbols(file: &str, text: &str, out: &mut Vec<EngineSymbol>) {
    /// A dotted name at the start of `s` (`a`, `T.f`, `T:f` as `T.f`), and the rest.
    fn ident(s: &str) -> Option<(String, &str)> {
        let end = s
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':'))
            .unwrap_or(s.len());
        let name = s.get(..end)?;
        let ok = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        (ok && !name.ends_with(['.', ':']))
            .then(|| (name.replace(':', "."), s.get(end..).unwrap_or("")))
    }
    for (i, line) in text.lines().enumerate() {
        // Only top-level lines: no indentation, not local.
        if line.starts_with([' ', '\t']) || line.starts_with("local ") {
            continue;
        }
        let line_no = to_u32(i + 1);
        if let Some(rest) = line.strip_prefix("function ") {
            if let Some((name, after)) = ident(rest.trim_start()) {
                let args = after.trim_start();
                let sig = args
                    .strip_prefix('(')
                    .and_then(|a| a.split_once(')'))
                    .map(|(a, _)| format!("function({})", a.trim()))
                    .unwrap_or_else(|| "function()".to_string());
                out.push(EngineSymbol {
                    name,
                    kind: EngineSymbolKind::Function,
                    signature: sig,
                    file: file.to_string(),
                    line: line_no,
                });
            }
            continue;
        }
        if let Some((name, after)) = ident(line) {
            let Some(value) = after.trim_start().strip_prefix('=') else {
                continue;
            };
            if value.starts_with('=') {
                continue;
            }
            let value = value.trim_start();
            if let Some(args) = value.strip_prefix("function") {
                let sig = args
                    .trim_start()
                    .strip_prefix('(')
                    .and_then(|a| a.split_once(')'))
                    .map(|(a, _)| format!("function({})", a.trim()))
                    .unwrap_or_else(|| "function()".to_string());
                out.push(EngineSymbol {
                    name,
                    kind: EngineSymbolKind::Function,
                    signature: sig,
                    file: file.to_string(),
                    line: line_no,
                });
            } else if value.starts_with('{') && !name.contains('.') {
                out.push(EngineSymbol {
                    name,
                    kind: EngineSymbolKind::Table,
                    signature: String::new(),
                    file: file.to_string(),
                    line: line_no,
                });
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn text_keeps_its_line_endings_and_bom() {
        let crlf = b"\xEF\xBB\xBFa = 1\r\nb = 2\r\n";
        let f = decode_text("x.toml", crlf).unwrap();
        assert!(f.crlf && f.bom);
        assert_eq!(f.text, "a = 1\nb = 2\n");
        assert_eq!(encode_text(&f.text, f.crlf, f.bom), crlf.to_vec());
        let lf = decode_text("x.toml", b"a = 1\n").unwrap();
        assert!(!lf.crlf && !lf.bom);
        assert!(decode_text("x.bin", b"PNG\0\x01").is_err());
        assert!(decode_text("x.txt", &[0xFF, 0xFE, b'a']).is_err());
    }

    #[test]
    fn lines_and_columns_are_one_based_and_count_characters() {
        let text = "ab\nc\u{e9}d\n";
        assert_eq!(line_col(text, 0), (1, 1));
        assert_eq!(line_col(text, 3), (2, 1));
        // After the two-byte e acute: the third character of line 2.
        assert_eq!(line_col(text, 6), (2, 3));
    }

    #[test]
    fn game_symbols_are_the_top_level_globals() {
        let text = "function initGame(a, b)\nend\nlocal function hidden() end\nSoldier = {}\nfunction Soldier:move(x)\nend\nupdateAll = function(dt) end\n  indented = function() end\nx == 1\n";
        let mut out = Vec::new();
        game_symbols("main.lua", text, &mut out);
        let names: Vec<(&str, EngineSymbolKind, &str)> = out
            .iter()
            .map(|s| (s.name.as_str(), s.kind, s.signature.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("initGame", EngineSymbolKind::Function, "function(a, b)"),
                ("Soldier", EngineSymbolKind::Table, ""),
                ("Soldier.move", EngineSymbolKind::Function, "function(x)"),
                ("updateAll", EngineSymbolKind::Function, "function(dt)"),
            ]
        );
        assert_eq!(out[2].line, 5);
    }

    #[test]
    fn a_zero_match_finding_points_at_its_patch() {
        let toml =
            "[manifest]\nversion = \"1.0.0\"\n\n[[pattern]]\nid = \"m:hud\"\nname = \"HUD\"\n";
        let d = ManifestDiagnostic::error("Patch 'HUD' (m:hud) had 0 matches on 'main.lua'.");
        let placed = place_lint_finding(&d, toml);
        assert_eq!(placed.path.as_deref(), Some("qm.toml"));
        assert_eq!(placed.line, Some(5));
    }
}
