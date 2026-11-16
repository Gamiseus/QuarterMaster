//! A manual install found in the game folder, data left in `%APPDATA%`, and the issue report.
use crate::dto::{
    IssueArea, IssueReportRequest, IssueReportResult, LegacyData, LegacyMoveResult,
    ManualImportResult, ManualInstall, ManualMod, ManualSkipped,
};
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::ops::diagnostics::create_unique;
use crate::service::Service;
use qm_manager::library::{declares_version, scan_mod_roots};
use qm_manager::profiles::{is_reparse_point, remove_junction_or_dir, VANILLA_ID};
use qm_manager::relocate::{
    appdata_root, copy_data_root, remove_tree_keeping_link_targets, windows_version, MOVED_MARKER,
    RESTAGE_MARKER,
};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Every QuarterMaster loader build holds this text.
const LOADER_MARKER: &[u8] = b"QuarterMaster";
const ISSUES_URL: &str = "https://github.com/Gamiseus/QuarterMaster/issues/new";
const ISSUE_TEMPLATE: &str = "bug_report.yml";
const MAX_TITLE: usize = 150;
const MAX_DESCRIPTION: usize = 10_000;
/// The description is cut to this in the link; the report file holds all of it.
const MAX_URL_DESCRIPTION: usize = 3_000;

impl Service {
    /// What an install by hand left in the game folder, if anything. None when the game is not
    /// set or its folder holds only what this launcher put there.
    pub fn manual_install_check(&self) -> Option<ManualInstall> {
        let game_dir = self.game_dir().ok()?;
        self.manual_install_in(&game_dir)
    }

    fn manual_install_in(&self, game_dir: &Path) -> Option<ManualInstall> {
        let mods_dir = game_dir.join("Mods");
        // A launcher that cannot make links copies the profile here; that folder holds its
        // launch settings and is not a manual install.
        let has_mods_folder = mods_dir.is_dir()
            && !is_reparse_point(&mods_dir)
            && !mods_dir.join("qm").join("launch-config.json").is_file();
        // Beside a Mods folder made by hand, any winmm.dll is the loader that went with it, even
        // one of a build this launcher knows.
        let dll = game_dir.join("winmm.dll");
        let has_loader = dll.is_file() && (has_mods_folder || self.foreign_loader(game_dir));
        let mut mods = Vec::new();
        let mut unreadable = Vec::new();
        if has_mods_folder {
            let installed = self.paths().list_installed_mods();
            for (dir, manifest) in scan_mod_roots(&mods_dir) {
                let Ok(rel) = dir.strip_prefix(&mods_dir) else {
                    continue;
                };
                // The loader's own folder (logs, dumps) is not a mod.
                if rel.as_os_str().is_empty() || rel.starts_with("qm") {
                    continue;
                }
                let folder = rel.to_string_lossy().replace('\\', "/");
                match manifest {
                    Ok(m) => mods.push(ManualMod {
                        installed_version: installed
                            .iter()
                            .find(|i| i.manifest.id == m.id)
                            .map(|i| i.manifest.version.clone()),
                        version_missing: !declares_version(&dir),
                        id: m.id,
                        name: m.name,
                        version: m.version,
                        folder,
                    }),
                    Err(reason) => unreadable.push(ManualSkipped { folder, reason }),
                }
            }
        }
        if mods.is_empty() && unreadable.is_empty() && !has_loader {
            return None;
        }
        Some(ManualInstall {
            game_dir: game_dir.to_path_buf(),
            has_loader,
            has_mods_folder,
            mods,
            unreadable,
        })
    }

    /// `winmm.dll` in the game folder is a QuarterMaster loader this launcher never copied.
    fn foreign_loader(&self, game_dir: &Path) -> bool {
        let Ok(bytes) = std::fs::read(game_dir.join("winmm.dll")) else {
            return false;
        };
        let hash = qm_schema::sha256_hex(&bytes);
        !self.known_hashes().contains(&hash)
            && bytes
                .windows(LOADER_MARKER.len())
                .any(|w| w == LOADER_MARKER)
    }

    /// Bring a manual install into the launcher: its mods go into the library and are switched
    /// on in the active profile, the old `Mods` folder moves to the backups folder and the old
    /// `winmm.dll` is deleted. A mod already in the library is left as it is.
    pub fn manual_install_import(&self) -> QmResult<ManualImportResult> {
        self.ensure_not_running()?;
        self.ensure_no_install()?;
        let install = self.lock_install();
        let game_dir = self.game_dir()?;
        let found = self.manual_install_in(&game_dir).ok_or_else(|| {
            QmError::new(
                ErrorKind::NotFound,
                "No manual install was found in the game folder.",
            )
        })?;
        let mods_dir = game_dir.join("Mods");
        let mut ids = Vec::new();
        let mut imported = Vec::new();
        let mut unversioned = Vec::new();
        let mut skipped = found.unreadable.clone();
        for m in &found.mods {
            if let Some(v) = &m.installed_version {
                skipped.push(ManualSkipped {
                    folder: m.folder.clone(),
                    reason: format!("{} {v} is already in the library.", m.name),
                });
                continue;
            }
            let outcome = self.paths().import_mod_from_path_with_options(
                &mods_dir.join(&m.folder),
                None,
                false,
                Some(vec![m.id.clone()]),
                Some("upgrade".to_string()),
            );
            match outcome {
                Ok(_) => {
                    ids.push(m.id.clone());
                    imported.push(m.name.clone());
                    if m.version_missing {
                        unversioned.push(m.name.clone());
                    }
                }
                Err(reason) => skipped.push(ManualSkipped {
                    folder: m.folder.clone(),
                    reason,
                }),
            }
        }
        drop(install);

        let mut enabled = false;
        if !ids.is_empty() {
            self.invalidate(Scope::Mods);
            let mut profile = self.active_profile()?;
            if profile.id != VANILLA_ID {
                for id in &ids {
                    if !profile.enabled_mods.contains(id) {
                        profile.enabled_mods.push(id.clone());
                    }
                }
                self.commit_active_profile(profile)?;
                enabled = true;
            }
        }

        // The mods are safe in the library; now the game folder is cleared for the launcher.
        let _g = self.lock_game_folder();
        let mut backup_dir = None;
        if found.has_mods_folder {
            let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S");
            let dir = self
                .paths()
                .backups_dir
                .join(format!("manual-install-{stamp}"));
            std::fs::create_dir_all(&dir).or_kind(ErrorKind::Io)?;
            let dest = dir.join("Mods");
            // A rename when both are on one drive; otherwise a copy, then the delete.
            if std::fs::rename(&mods_dir, &dest).is_err() {
                qm_manager::copy_directory_clean(&mods_dir, &dest).or_kind(ErrorKind::Io)?;
                std::fs::remove_dir_all(&mods_dir).or_kind(ErrorKind::Io)?;
            }
            backup_dir = Some(dir);
        }
        let mut loader_removed = false;
        if found.has_loader {
            let dll = game_dir.join("winmm.dll");
            // A read-only file (some unzip tools mark them) is cleared first.
            if let Ok(meta) = std::fs::metadata(&dll) {
                let mut perms = meta.permissions();
                if perms.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    perms.set_readonly(false);
                    if let Err(e) = std::fs::set_permissions(&dll, perms) {
                        tracing::warn!("could not clear read-only on {}: {e}", dll.display());
                    }
                }
            }
            std::fs::remove_file(&dll).map_err(|e| {
                QmError::new(
                    ErrorKind::Io,
                    "Your mods were imported, but the old winmm.dll could not be deleted. Delete it from the game folder by hand.",
                )
                .with_details(format!("{}: {e}", dll.display()))
            })?;
            loader_removed = !dll.exists();
        }
        drop(_g);
        self.invalidate(Scope::Health);
        Ok(ManualImportResult {
            imported,
            unversioned,
            skipped,
            enabled,
            backup_dir,
            loader_removed,
        })
    }

    /// Data in `%APPDATA%\QuarterMaster`, offered to a data folder beside the program that has
    /// nothing in it yet.
    pub fn legacy_data_check(&self) -> Option<LegacyData> {
        let paths = self.paths();
        if !paths.is_portable {
            return None;
        }
        let old = appdata_root()?;
        if old == paths.root_dir
            || !old.join("settings.json").is_file()
            || old.join(MOVED_MARKER).exists()
        {
            return None;
        }
        if paths.load_settings().first_run_completed || !paths.list_installed_mods().is_empty() {
            return None;
        }
        Some(LegacyData {
            mods: count_dirs(&old.join("library")),
            profiles: count_dirs(&old.join("profiles")),
            path: old,
        })
    }

    /// Move the `%APPDATA%` data here. The launcher must restart afterwards: this service still
    /// holds what it read from the empty folder.
    pub fn legacy_data_move(&self) -> QmResult<LegacyMoveResult> {
        self.ensure_not_running()?;
        self.ensure_no_install()?;
        let _install = self.lock_install();
        let found = self.legacy_data_check().ok_or_else(|| {
            QmError::new(ErrorKind::NotFound, "There is no earlier data to move.")
        })?;
        let new_root = self.paths().root_dir.clone();
        let files = copy_data_root(&found.path, &new_root).or_kind(ErrorKind::Io)?;
        // The game folder's Mods link points into the old folder; the next launch makes it again.
        if let Ok(game_dir) = self.game_dir() {
            let _g = self.lock_game_folder();
            let link = game_dir.join("Mods");
            if is_reparse_point(&link) {
                if let Err(e) = remove_junction_or_dir(&link) {
                    tracing::warn!("could not remove the old Mods link: {e}");
                }
            }
        }
        let note = format!("QuarterMaster moved this data to {}.", new_root.display());
        if let Err(e) = std::fs::write(found.path.join(MOVED_MARKER), note) {
            tracing::warn!("could not mark the old data folder as moved: {e}");
        }
        let old_removed = match remove_tree_keeping_link_targets(&found.path) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("the old data folder was not deleted: {e}");
                false
            }
        };
        Ok(LegacyMoveResult {
            files: u32::try_from(files).unwrap_or(u32::MAX),
            old_removed,
        })
    }

    /// After a data move: links are not copied, so the active profile is staged again.
    pub(crate) fn restage_after_move(&self) {
        let marker = self.paths().cache_dir.join(RESTAGE_MARKER);
        if !marker.is_file() {
            return;
        }
        let active = self.paths().load_settings().active_profile_id;
        match self.stage_profile(&active) {
            Ok(()) => {
                if let Err(e) = std::fs::remove_file(&marker) {
                    tracing::warn!("could not remove {}: {e}", marker.display());
                }
            }
            Err(e) => tracing::error!("staging after the data move failed: {e}"),
        }
    }

    /// The link to a new issue with the text filled in and, when asked, a zip beside the bug
    /// reports with the description and the redacted diagnostics. Nothing is sent anywhere.
    pub fn issue_report_build(&self, req: IssueReportRequest) -> QmResult<IssueReportResult> {
        let title = req.title.trim();
        let description = req.description.trim();
        if title.is_empty() || description.is_empty() {
            return Err(invalid("Give the report a title and say what happened."));
        }
        if title.chars().count() > MAX_TITLE {
            return Err(invalid("Keep the title under 150 characters."));
        }
        if description.chars().count() > MAX_DESCRIPTION {
            return Err(invalid("Keep the description under 10,000 characters."));
        }
        let version = qm_core::QM_VERSION;
        let windows = windows_version().unwrap_or_else(|| "Windows".to_string());
        let game = self
            .paths()
            .auto_detect_all_paths()
            .game_version
            .unwrap_or_else(|| "not detected".to_string());

        let file = if req.attach_files {
            Some(self.write_issue_zip(req.area, title, description, version, &windows, &game)?)
        } else {
            None
        };

        let short: String = description.chars().take(MAX_URL_DESCRIPTION).collect();
        let mut what = short;
        if description.chars().count() > MAX_URL_DESCRIPTION {
            what.push_str("\n\n(The rest is in the attached report file.)");
        }
        let enc = crate::github::encode_segment;
        let url = format!(
            "{ISSUES_URL}?template={ISSUE_TEMPLATE}&title={}&area={}&version={}&game-version={}&windows={}&what-happened={}",
            enc(title),
            enc(area_label(req.area)),
            enc(version),
            enc(&game),
            enc(&windows),
            enc(&what),
        );
        Ok(IssueReportResult { url, file })
    }

    fn write_issue_zip(
        &self,
        area: IssueArea,
        title: &str,
        description: &str,
        version: &str,
        windows: &str,
        game: &str,
    ) -> QmResult<PathBuf> {
        let diagnostics = self.redacted_bundle_json()?;
        let dir = self.diagnostics_dir()?;
        let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
        let (path, file) = create_unique(&dir, &format!("QuarterMaster-report-{stamp}"), "zip")?;
        let report = format!(
            "# {title}\n\n- Area: {}\n- QuarterMaster: {version}\n- Game: {game}\n- System: {windows}\n\n## What happened\n\n{description}\n",
            area_label(area)
        );
        let io = |e: zip::result::ZipError| QmError::new(ErrorKind::Io, e.to_string());
        let options = zip::write::SimpleFileOptions::default();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("report.md", options).map_err(io)?;
        zip.write_all(report.as_bytes()).or_kind(ErrorKind::Io)?;
        zip.start_file("diagnostics.json", options).map_err(io)?;
        zip.write_all(diagnostics.as_bytes())
            .or_kind(ErrorKind::Io)?;
        zip.finish().map_err(io)?;
        Ok(path)
    }
}

/// The option text of the issue form's Area list.
fn area_label(area: IssueArea) -> &'static str {
    match area {
        IssueArea::Launcher => "The launcher",
        IssueArea::Loader => "The game with mods (mod loader)",
        IssueArea::Mod => "A mod",
        IssueArea::Other => "Something else",
    }
}

fn count_dirs(dir: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let n = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn invalid(msg: &str) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}
