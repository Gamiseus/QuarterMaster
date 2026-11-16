//! Smaller launcher features: mod colours, the game window settings mod, the mods source,
//! copies of unlinked `Mods` folders, the full game dump, file previews, dismissed conflicts
//! and the version a profile uses.
use crate::dto::{ModSummary, SettingsView};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobId, JobKind};
use crate::ops::mods::is_plain_id;
use crate::service::Service;
use qm_manager::profiles::VANILLA_ID;
use qm_manager::storage::{ModsSource, WindowSettings};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// The mod the launcher writes the game window settings into.
pub const WINDOW_MOD_ID: &str = "qm_game_window";
const WINDOW_MOD_VERSION: &str = "1.0.0";
/// The folder of the full game dump inside the workspace mods folder (never read as a mod).
pub const GAME_DUMP_DIR: &str = "_game_dump";
const UNLINKED_PREFIX: &str = "unlinked-mods-";
/// Largest file a preview reads.
const MAX_VIEW_BYTES: u64 = 12 * 1024 * 1024;
const MOD_COLOURS: std::ops::RangeInclusive<u8> = 1..=8;
const WINDOW_SIZE: std::ops::RangeInclusive<u32> = 320..=16384;

/// A copy the launcher made of a real `Mods` folder it found in the game folder before
/// linking its own there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LeftoverBackup {
    /// The folder name inside the backups folder.
    pub name: String,
    pub size_bytes: u64,
    pub file_count: u32,
    /// The top-level folders in it (the mods), at most 20.
    pub contents: Vec<String>,
    /// Seconds since 1970, from the folder's modified time.
    pub modified: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FileViewKind {
    Text,
    Image,
    Audio,
    /// A kind the launcher cannot show; only the size is known.
    Other,
}

/// One file of the game or of a mod, for the viewer.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FileView {
    /// `vanilla`, or a mod id.
    pub source: String,
    pub path: String,
    pub kind: FileViewKind,
    pub size_bytes: u64,
    /// For `Text`.
    pub text: Option<String>,
    /// For `Image` and `Audio`: a `data:` URL.
    pub data_url: Option<String>,
}

/// One file of an installed mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ModFile {
    /// Relative to the mod's folder, with forward slashes.
    pub path: String,
    pub size_bytes: u64,
}

/// The files a mod comes with, in path order.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModFileList {
    /// At most [`MAX_LISTED_FILES`]; the totals count every file.
    pub files: Vec<ModFile>,
    pub total_files: u32,
    pub total_bytes: u64,
}

/// Most files a mod's file list carries.
pub const MAX_LISTED_FILES: usize = 2000;

impl Service {
    /// Every file in a mod's folder as the active profile uses it (the folder
    /// [`Service::file_view`] reads). Names that start with a dot, and links, are left out.
    pub fn mod_files(&self, id: &str) -> QmResult<ModFileList> {
        let root = self.mod_dir_of(id)?;
        let mut files = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.filter_map(Result::ok) {
                let path = e.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                // A dot name is a tool's own (`.git`, the launcher's integrity record).
                if e.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                if meta.is_file() {
                    if let Ok(rel) = path.strip_prefix(&root) {
                        files.push(ModFile {
                            path: rel.to_string_lossy().replace('\\', "/"),
                            size_bytes: meta.len(),
                        });
                    }
                } else if meta.is_dir() && !qm_manager::profiles::is_reparse_point(&path) {
                    stack.push(path);
                }
            }
        }
        files.sort_by_cached_key(|f| f.path.to_ascii_lowercase());
        let total_files = u32::try_from(files.len()).unwrap_or(u32::MAX);
        let total_bytes = files.iter().map(|f| f.size_bytes).sum();
        files.truncate(MAX_LISTED_FILES);
        Ok(ModFileList {
            files,
            total_files,
            total_bytes,
        })
    }

    /// `SettingsView` with what only the running launcher knows.
    pub(crate) fn settings_view(&self, s: &qm_manager::LauncherSettings) -> SettingsView {
        let mut v = SettingsView::from(s);
        v.workspace_mode = self.paths().workspace_mode;
        v
    }

    /// Sets (`Some(1..=8)`) or clears the colour a mod has in the patch views.
    pub fn mod_color_set(&self, mod_id: &str, color: Option<u8>) -> QmResult<SettingsView> {
        if !is_plain_id(mod_id) {
            return Err(invalid(format!("'{mod_id}' is not a mod id.")));
        }
        if color.is_some_and(|c| !MOD_COLOURS.contains(&c)) {
            return Err(invalid("A mod colour is a number from 1 to 8.".to_string()));
        }
        let paths = self.paths();
        let mut s = paths.load_settings();
        match color {
            Some(c) => s.mod_colors.insert(mod_id.to_string(), c),
            None => s.mod_colors.remove(mod_id),
        };
        paths.save_settings(&s).or_kind(ErrorKind::Io)?;
        self.notify(Scope::Settings);
        Ok(self.settings_view(&s))
    }

    /// Stores the game window settings and writes them into the launcher's own mod, which is
    /// turned on in the active profile. With every value left to the game the mod is removed.
    pub fn window_settings_set(&self, window: WindowSettings) -> QmResult<SettingsView> {
        validate_window(&window)?;
        self.ensure_not_running()?;
        self.ensure_no_install()?;
        let paths = self.paths();
        let mut s = paths.load_settings();
        s.window = window.clone();
        paths.save_settings(&s).or_kind(ErrorKind::Io)?;
        self.notify(Scope::Settings);

        let installed = paths.library_dir.join(WINDOW_MOD_ID).is_dir();
        if window.is_empty() {
            if installed {
                self.mod_uninstall(WINDOW_MOD_ID)?;
            }
            return Ok(self.settings_view(&s));
        }
        let stage = paths.cache_dir.join("window-mod").join(WINDOW_MOD_ID);
        // A leftover stage is only clutter in the cache folder.
        drop(std::fs::remove_dir_all(&stage));
        std::fs::create_dir_all(&stage).or_kind(ErrorKind::Io)?;
        let manifest = serde_json::json!({
            "schema_version": 1,
            "id": WINDOW_MOD_ID,
            "name": "Game window settings",
            "version": WINDOW_MOD_VERSION,
            "authors": ["QuarterMaster"],
            "description": "The window size, fullscreen, vsync and monitor picked in the launcher's Settings. Written by the launcher: change the values there.",
            "category": "Launcher",
            "load_priority": 900,
        });
        let toml = format!(
            "[manifest]\nversion = \"{WINDOW_MOD_VERSION}\"\npriority = 900\n\n[[patches]]\n[patches.copy]\nid = \"{WINDOW_MOD_ID}:conf\"\nname = \"Window values\"\ntarget = \"conf.lua\"\nposition = \"append\"\nsources = [\"window.lua\"]\n"
        );
        for (name, text) in [
            (
                "mod.json",
                serde_json::to_string_pretty(&manifest).unwrap_or_default(),
            ),
            ("qm.toml", toml),
            ("window.lua", window.conf_lua()),
        ] {
            std::fs::write(stage.join(name), text).or_kind(ErrorKind::Io)?;
        }
        {
            let _install = self.lock_install();
            paths
                .import_mod_from_path_with_options(
                    &stage,
                    None,
                    false,
                    None,
                    Some("replace".to_string()),
                )
                .or_kind(ErrorKind::Io)?;
        }
        // A leftover stage is only clutter in the cache folder.
        drop(std::fs::remove_dir_all(&stage));
        // First use: on in the active profile. Later the user's own choice stands.
        if !installed {
            if let Ok(mut p) = self.active_profile() {
                if p.id != VANILLA_ID && !p.enabled_mods.iter().any(|m| m == WINDOW_MOD_ID) {
                    p.enabled_mods.push(WINDOW_MOD_ID.to_string());
                    self.commit_active_profile(p)?;
                }
            }
        }
        self.after_mod_import(&[WINDOW_MOD_ID.to_string()])?;
        Ok(self.settings_view(&s))
    }

    /// Stores which set of mods and profiles to use. It takes effect when the launcher
    /// starts again (the caller restarts it).
    pub fn mods_source_set(&self, source: ModsSource) -> QmResult<()> {
        self.ensure_not_running()?;
        self.ensure_no_install()?;
        let paths = self.paths();
        let mut s = paths.load_settings();
        if source == ModsSource::Workspace && s.workspace_mods_dir.is_none() {
            return Err(QmError::new(
                ErrorKind::NotFound,
                "Set a workspace mods folder in Settings first.",
            ));
        }
        // The game folder goes back to clean; the next start stages the other set's profile.
        s.mods_source = source;
        paths.save_settings(&s).or_kind(ErrorKind::Io)
    }

    /// The copies of unlinked `Mods` folders in the backups folder, newest first.
    pub fn leftover_backups_list(&self) -> Vec<LeftoverBackup> {
        let Ok(rd) = std::fs::read_dir(&self.paths().backups_dir) else {
            return Vec::new();
        };
        let mut out: Vec<LeftoverBackup> = rd
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                if !name.starts_with(UNLINKED_PREFIX) {
                    return None;
                }
                let (size_bytes, file_count) = dir_size(&e.path());
                let mut contents: Vec<String> = std::fs::read_dir(e.path())
                    .map(|rd| {
                        rd.filter_map(Result::ok)
                            .filter_map(|c| c.file_name().to_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                contents.sort();
                contents.truncate(20);
                let modified = e
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                Some(LeftoverBackup {
                    name,
                    size_bytes,
                    file_count,
                    contents,
                    modified,
                })
            })
            .collect();
        out.sort_by(|a, b| b.name.cmp(&a.name));
        out
    }

    /// Deletes one copy for good. Only a folder the list shows can be named.
    pub fn leftover_backup_delete(&self, name: &str) -> QmResult<()> {
        if !is_plain_id(name) || !name.starts_with(UNLINKED_PREFIX) {
            return Err(invalid(format!("'{name}' is not a copy of a Mods folder.")));
        }
        let dir = self.paths().backups_dir.join(name);
        if !dir.is_dir() {
            return Err(QmError::new(
                ErrorKind::NotFound,
                format!("There is no copy called '{name}'."),
            ));
        }
        qm_manager::profiles::remove_junction_or_dir(&dir).or_kind(ErrorKind::Io)
    }

    /// Writes every file of the game (code, images, sounds, fonts) into `_game_dump` in the
    /// workspace mods folder, in a job. The job output is the number of files.
    pub fn game_dump_full(&self) -> QmResult<JobId> {
        let s = self.paths().load_settings();
        let ws = s.workspace_mods_dir.ok_or_else(|| {
            QmError::new(ErrorKind::NotFound, "The workspace mods folder is not set.")
        })?;
        let exe = s
            .game_exe_path
            .filter(|p| p.is_file())
            .ok_or_else(|| QmError::new(ErrorKind::NotFound, "The game executable is not set."))?;
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::GameDump, move |ctx| {
            ctx.progress(0.1, "Writing the game's files");
            let target = ws.join(GAME_DUMP_DIR);
            std::fs::create_dir_all(&target).or_kind(ErrorKind::Io)?;
            let n = svc
                .paths()
                .extract_all_vanilla_assets(&exe, &target)
                .or_kind(ErrorKind::Io)?;
            Ok(Some(n.to_string()))
        }))
    }

    /// One file of the game (`source` is `vanilla`) or of an installed mod (a mod id, or
    /// `faction:<id>`), for the viewer. `path` is relative: inside the game's files, or
    /// inside the mod's folder.
    pub fn file_view(&self, source: &str, path: &str) -> QmResult<FileView> {
        let rel = clean_relative(path)?;
        let rel_text = rel.to_string_lossy().replace('\\', "/");
        let bytes = if source == "vanilla" {
            let cached = self
                .inner
                .caches
                .vanilla
                .get()
                .and_then(|v| v.files.get(&rel_text).cloned());
            match cached {
                Some(text) => text.into_bytes(),
                None => self
                    .read_payload_file(&rel_text, MAX_VIEW_BYTES)?
                    .ok_or_else(|| not_found(format!("The game has no file {rel_text}.")))?,
            }
        } else {
            let file = self.mod_dir_of(source)?.join(&rel);
            let meta = std::fs::metadata(&file)
                .ok()
                .filter(std::fs::Metadata::is_file)
                .ok_or_else(|| not_found(format!("{source} has no file {rel_text}.")))?;
            if meta.len() > MAX_VIEW_BYTES {
                return Ok(FileView {
                    source: source.to_string(),
                    path: rel_text,
                    kind: FileViewKind::Other,
                    size_bytes: meta.len(),
                    text: None,
                    data_url: None,
                });
            }
            std::fs::read(&file).or_kind(ErrorKind::Io)?
        };
        let ext = rel
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let (kind, mime) = match ext.as_str() {
            "png" => (FileViewKind::Image, "image/png"),
            "jpg" | "jpeg" => (FileViewKind::Image, "image/jpeg"),
            "gif" => (FileViewKind::Image, "image/gif"),
            "bmp" => (FileViewKind::Image, "image/bmp"),
            "webp" => (FileViewKind::Image, "image/webp"),
            "ogg" => (FileViewKind::Audio, "audio/ogg"),
            "wav" => (FileViewKind::Audio, "audio/wav"),
            "mp3" => (FileViewKind::Audio, "audio/mpeg"),
            "flac" => (FileViewKind::Audio, "audio/flac"),
            "lua" | "txt" | "toml" | "json" | "md" | "glsl" | "frag" | "vert" | "fnt" | "csv"
            | "xml" | "ini" | "cfg" => (FileViewKind::Text, ""),
            _ => (FileViewKind::Other, ""),
        };
        let size_bytes = bytes.len() as u64;
        let (text, data_url) = match kind {
            FileViewKind::Text => (Some(String::from_utf8_lossy(&bytes).into_owned()), None),
            FileViewKind::Image | FileViewKind::Audio => (
                None,
                Some(format!(
                    "data:{mime};base64,{}",
                    crate::ops::sprites::base64(&bytes)
                )),
            ),
            FileViewKind::Other => (None, None),
        };
        Ok(FileView {
            source: source.to_string(),
            path: rel_text,
            kind,
            size_bytes,
            text,
            data_url,
        })
    }

    /// The folder of a mod as the active profile uses it: the staged copy, else the library's.
    fn mod_dir_of(&self, source: &str) -> QmResult<PathBuf> {
        let id = match source.strip_prefix("faction:") {
            Some(faction) => self
                .installed_mods()
                .iter()
                .find(|m| {
                    qm_schema::faction::read_faction_header(&m.install_path)
                        .is_some_and(|h| h.id == faction)
                })
                .map(|m| m.manifest.id.clone())
                .ok_or_else(|| {
                    not_found(format!("No installed mod has the faction '{faction}'."))
                })?,
            None => source.to_string(),
        };
        if !is_plain_id(&id) {
            return Err(invalid(format!("'{id}' is not a mod id.")));
        }
        let active = self.paths().load_settings().active_profile_id;
        let staged = self
            .paths()
            .profiles_dir
            .join(active)
            .join("Mods")
            .join(&id);
        if staged.is_dir() {
            return Ok(staged);
        }
        Ok(self.installed_mod(&id)?.install_path)
    }

    /// Dismisses a conflict in the active profile, or shows it again. A dismissed conflict
    /// stays listed, apart from the open ones.
    pub fn conflict_dismiss(&self, conflict_id: &str, dismissed: bool) -> QmResult<()> {
        if conflict_id.is_empty() || conflict_id.len() > 512 {
            return Err(invalid("That is not a conflict.".to_string()));
        }
        let mut p = self.active_profile()?;
        let had = p.dismissed_conflicts.iter().any(|c| c == conflict_id);
        if had == dismissed {
            return Ok(());
        }
        if dismissed {
            p.dismissed_conflicts.push(conflict_id.to_string());
        } else {
            p.dismissed_conflicts.retain(|c| c != conflict_id);
        }
        // Not a mod, order or patch edit: the staged files and the simulation stay as they are.
        self.ensure_not_running()?;
        p.updated_at = jiff::Zoned::now().to_string();
        self.with_profile_write(|| self.paths().save_profile(&p))
            .or_kind(ErrorKind::Io)?;
        self.notify(Scope::Profiles);
        self.notify(Scope::LoadOrder);
        Ok(())
    }

    /// Deletes one installed version of a mod; profiles pinned to it go back to the newest.
    /// The only installed version is not removed this way.
    pub fn mod_version_remove(&self, id: &str, version: &str) -> QmResult<()> {
        self.ensure_not_running()?;
        self.ensure_no_install()?;
        let m = self.installed_mod(id)?;
        if m.is_workspace_linked {
            return Err(QmError::new(
                ErrorKind::Conflict,
                "This mod is a folder in the workspace mods folder. Remove it there.",
            ));
        }
        if !is_plain_id(version) || !m.installed_versions.iter().any(|v| v == version) {
            return Err(not_found(format!(
                "Version {version} of {} is not installed.",
                m.manifest.name
            )));
        }
        if m.installed_versions.len() < 2 {
            return Err(QmError::new(
                ErrorKind::Conflict,
                format!(
                    "{version} is the only installed version of {}. Uninstall the mod instead.",
                    m.manifest.name
                ),
            ));
        }
        let was_enabled = self
            .active_profile()
            .is_ok_and(|p| p.enabled_mods.iter().any(|x| x == id));
        {
            let _install = self.lock_install();
            self.paths()
                .remove_mod_version(id, version)
                .or_kind(ErrorKind::Io)?;
        }
        self.invalidate(Scope::Mods);
        let restaged = if was_enabled {
            let active = self.paths().load_settings().active_profile_id;
            self.stage_profile(&active)
        } else {
            Ok(())
        };
        // A profile pinned to the version was rewritten.
        self.after_active_profile_change();
        restaged
    }

    /// Picks which installed version of a mod the active profile uses; `None` goes back to
    /// the newest.
    pub fn mod_version_set(&self, id: &str, version: Option<String>) -> QmResult<ModSummary> {
        self.ensure_not_running()?;
        let m = self.installed_mod(id)?;
        let mut p = self.active_profile()?;
        match &version {
            // The newest needs no pin, so a later update is picked up.
            Some(v) if *v != m.manifest.version => {
                if !m.installed_versions.iter().any(|x| x == v) {
                    return Err(not_found(format!(
                        "Version {v} of {} is not installed.",
                        m.manifest.name
                    )));
                }
                p.pinned_versions.insert(id.to_string(), v.clone());
            }
            _ => {
                p.pinned_versions.remove(id);
            }
        }
        let enabled = p.enabled_mods.iter().any(|x| x == id);
        let pin = p.pinned_versions.get(id).cloned();
        self.commit_active_profile(p)?;
        // The list shows the version in use; the index of installed mods is unchanged.
        self.notify(Scope::Mods);
        let mut summary = ModSummary::new(&m, enabled);
        if let Some(v) = pin {
            summary.version = v;
        }
        Ok(summary)
    }
}

fn validate_window(w: &WindowSettings) -> QmResult<()> {
    for (label, v) in [("width", w.width), ("height", w.height)] {
        if v.is_some_and(|v| !WINDOW_SIZE.contains(&v)) {
            return Err(invalid(format!(
                "The window {label} must be between 320 and 16384."
            )));
        }
    }
    if w.display.is_some_and(|d| !(1..=16).contains(&d)) {
        return Err(invalid("The monitor number must be 1 to 16.".to_string()));
    }
    if let Some(t) = &w.fullscreen_type {
        if t != "desktop" && t != "exclusive" {
            return Err(invalid(format!(
                "The fullscreen type must be desktop or exclusive (got {t:?})."
            )));
        }
    }
    Ok(())
}

/// `path` as a relative path with only plain names in it.
fn clean_relative(path: &str) -> QmResult<PathBuf> {
    let mut out = PathBuf::new();
    for c in Path::new(&path.replace('\\', "/")).components() {
        match c {
            Component::Normal(seg) => out.push(seg),
            Component::CurDir => {}
            _ => {
                return Err(invalid(format!(
                    "'{path}' is not a file inside the folder."
                )))
            }
        }
    }
    if out.as_os_str().is_empty() || path.contains(':') {
        return Err(invalid(format!(
            "'{path}' is not a file inside the folder."
        )));
    }
    Ok(out)
}

/// Total size and file count under `dir`; links are not followed.
fn dir_size(dir: &Path) -> (u64, u32) {
    let mut size = 0u64;
    let mut files = 0u32;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
                continue;
            };
            if meta.is_file() {
                size += meta.len();
                files = files.saturating_add(1);
            } else if meta.is_dir() && !qm_manager::profiles::is_reparse_point(&e.path()) {
                stack.push(e.path());
            }
        }
    }
    (size, files)
}

fn invalid(msg: String) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}

fn not_found(msg: String) -> QmError {
    QmError::new(ErrorKind::NotFound, msg)
}
