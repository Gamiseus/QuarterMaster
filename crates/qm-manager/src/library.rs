use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;
use zip::ZipArchive;

use qm_core::patch::PatchFile;
use qm_schema::{
    sha256_hex, IntegrityManifest, IntegrityVerifyResult, ModConfigSchema, ModManifest,
    ParsedVersion, SecurityScanReport,
};

use crate::storage::QuarterMasterPaths;

pub const MAX_ARCHIVE_FILES: usize = 1000;
pub const MAX_ARCHIVE_TOTAL_BYTES: u64 = 1024 * 1024 * 512; // 512 MB
pub const MAX_COMPRESSION_RATIO: u64 = 250;

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledMod {
    pub manifest: ModManifest,
    pub folder_name: String,
    pub install_path: PathBuf,
    pub integrity: IntegrityManifest,
    pub config_schema: Option<ModConfigSchema>,
    pub readme_markdown: Option<String>,
    pub changelog_markdown: Option<String>,
    pub patch_count: usize,
    pub target_files: Vec<String>,
    pub is_workspace_linked: bool,
    #[serde(default)]
    pub installed_versions: Vec<String>,
    #[serde(default)]
    pub kind: ModKind,
}

/// How a library folder is loaded: through its patch files, as a faction (`faction.toml` and no
/// patch files), as an `assets/` folder only (the autoloader), or neither.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModKind {
    #[default]
    Managed,
    Unmanaged,
    Assets,
    Faction,
}

impl ModKind {
    /// Uses the loader's own rules for patch files and assets.
    pub fn of_dir(mod_dir: &Path) -> Self {
        if !qm_core::patch::loader::dir_patch_files(mod_dir).is_empty() {
            ModKind::Managed
        } else if qm_schema::faction::has_faction_file(mod_dir) {
            ModKind::Faction
        } else if has_supported_assets_dir(mod_dir) {
            ModKind::Assets
        } else {
            ModKind::Unmanaged
        }
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveModCandidate {
    pub manifest: ModManifest,
    pub relative_folder: String,
    pub already_installed_version: Option<String>,
    pub all_installed_versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub imported_mods: Vec<InstalledMod>,
    pub was_multi_mod_pack: bool,
    pub upgraded_from_versions: Vec<(String, String)>,
    #[serde(default)]
    pub cas_deduplicated_files: usize,
}

const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Library staging area for copy-then-swap installs (top-level dot folder, skipped by indexing).
const STAGING_DIR: &str = ".qm-staging";

fn is_reserved_device_stem(seg: &str) -> bool {
    let stem = seg.split('.').next().unwrap_or(seg).to_ascii_uppercase();
    RESERVED_DEVICE_NAMES.contains(&stem.as_str())
}

/// True when `s` can be used as one folder name under the library: non-empty, not `.`/`..`,
/// no `/ \ :`, not a Windows reserved device name, no trailing dot or space.
pub(crate) fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains(['/', '\\', ':'])
        && !s.ends_with('.')
        && !s.ends_with(' ')
        && !is_reserved_device_stem(s)
}

/// Validates a relative path from an archive or folder against zip-slip, absolute paths,
/// and Windows reserved device names (`CON`, `NUL`, trailing dots/spaces).
pub fn validate_safe_relative_path(raw_path: &str) -> Result<PathBuf, String> {
    let normalized = raw_path.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        return Err(format!(
            "Rejected absolute or drive-prefixed path '{raw_path}'"
        ));
    }

    let path = Path::new(&normalized);
    let mut clean = PathBuf::new();

    for comp in path.components() {
        match comp {
            Component::Normal(os_str) => {
                let seg = os_str
                    .to_str()
                    .ok_or_else(|| format!("Invalid UTF-8 path segment in '{raw_path}'"))?;
                if seg.is_empty() {
                    continue;
                }
                if seg.ends_with('.') || seg.ends_with(' ') {
                    return Err(format!(
                        "Rejected path segment '{seg}' with trailing dot or space"
                    ));
                }
                // Windows device stem guard: Only reject files whose exact base stem matches a reserved device name
                // (e.g. `con.lua`, `aux.json`). Files like `console.lua`, `con_utils.lua`, `aux_audio.lua` are fully permitted.
                if is_reserved_device_stem(seg) {
                    return Err(format!(
                        "Rejected Windows reserved filename '{seg}' in archive"
                    ));
                }
                clean.push(seg);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("Rejected path traversal attempt in '{raw_path}'"));
            }
        }
    }

    if clean.as_os_str().is_empty() {
        return Err("Empty relative path".to_string());
    }
    Ok(clean)
}

impl QuarterMasterPaths {
    /// Scans `library/<mod_id>/<version>/` and returns all installed mods (latest version per mod_id).
    pub fn list_installed_mods(&self) -> Vec<InstalledMod> {
        let mut by_id: std::collections::HashMap<String, Vec<InstalledMod>> =
            std::collections::HashMap::new();

        let Ok(mod_dirs) = fs::read_dir(&self.library_dir) else {
            return Vec::new();
        };

        for mod_entry in mod_dirs.filter_map(|e| e.ok()) {
            if !mod_entry.path().is_dir() {
                continue;
            }
            let dir_name = mod_entry.file_name().to_str().unwrap_or("").to_string();
            if dir_name.starts_with('.') {
                continue;
            }
            let Ok(ver_dirs) = fs::read_dir(mod_entry.path()) else {
                continue;
            };
            for ver_entry in ver_dirs.filter_map(|e| e.ok()) {
                let ver_path = ver_entry.path();
                if !ver_path.is_dir() || ver_entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                // A link is a workspace mod (workspace mode).
                let linked = crate::profiles::is_reparse_point(&ver_path);
                if let Ok(installed) = inspect_mod_directory(&ver_path, linked) {
                    by_id
                        .entry(installed.manifest.id.clone())
                        .or_default()
                        .push(installed);
                }
            }
        }

        let mut results: Vec<InstalledMod> = by_id
            .into_values()
            .filter_map(|mut versions| {
                versions.sort_by(|a, b| {
                    let va =
                        ParsedVersion::parse(&a.manifest.version).unwrap_or(ParsedVersion::ZERO);
                    let vb =
                        ParsedVersion::parse(&b.manifest.version).unwrap_or(ParsedVersion::ZERO);
                    vb.cmp(&va)
                });
                let all_vers: Vec<String> = versions
                    .iter()
                    .map(|v| v.manifest.version.clone())
                    .collect();
                versions.into_iter().next().map(|mut latest| {
                    latest.installed_versions = all_vers;
                    latest
                })
            })
            .collect();

        results.sort_by(|a, b| {
            a.manifest
                .load_priority
                .cmp(&b.manifest.load_priority)
                .then_with(|| a.manifest.name.cmp(&b.manifest.name))
        });
        results
    }

    /// Retrieves a specific version of a mod from `library/<mod_id>/<version>/`, or the latest version if `None`.
    pub fn get_mod_version(&self, mod_id: &str, version: Option<&str>) -> Option<InstalledMod> {
        if let Some(ver) = version.filter(|v| !v.starts_with('.')) {
            let ver_dir = self.library_dir.join(mod_id).join(ver);
            if ver_dir.is_dir() {
                if let Ok(mut inst) = inspect_mod_directory(&ver_dir, false) {
                    if let Some(latest) = self
                        .list_installed_mods()
                        .into_iter()
                        .find(|m| m.manifest.id == mod_id)
                    {
                        inst.installed_versions = latest.installed_versions;
                    }
                    return Some(inst);
                }
            }
        }
        self.list_installed_mods()
            .into_iter()
            .find(|m| m.manifest.id == mod_id)
    }

    /// Syncs all mods from `MnB-Mods/` (or any workspace mod collection folder) into `library/`
    /// so existing workspace mods are automatically available in the pristine library.
    pub fn sync_workspace_mods(
        &self,
        workspace_mods_dir: &Path,
    ) -> Result<Vec<InstalledMod>, String> {
        if !workspace_mods_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut synced = Vec::new();
        let entries = fs::read_dir(workspace_mods_dir)
            .map_err(|e| format!("Failed to read {}: {e}", workspace_mods_dir.display()))?;

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dir_name.starts_with('.') || dir_name == "qm" {
                continue;
            }
            if path.join("qm.toml").is_file()
                || path.join("mod.json").is_file()
                || qm_schema::faction::has_faction_file(&path)
                || has_supported_assets_dir(&path)
            {
                if let Ok(summary) = self.import_mod_from_path(&path, None, false) {
                    synced.extend(summary.imported_mods);
                }
            }
        }
        Ok(synced)
    }

    /// The `(mod id, version)` of every library entry that is a link to a workspace folder.
    pub fn linked_workspace_versions(&self) -> std::collections::BTreeSet<(String, String)> {
        let mut out = std::collections::BTreeSet::new();
        let Ok(mod_dirs) = fs::read_dir(&self.library_dir) else {
            return out;
        };
        for mod_dir in mod_dirs.filter_map(|e| e.ok()) {
            let Ok(versions) = fs::read_dir(mod_dir.path()) else {
                continue;
            };
            for ver in versions.filter_map(|e| e.ok()) {
                if crate::profiles::is_reparse_point(&ver.path()) {
                    out.insert((
                        mod_dir.file_name().to_string_lossy().to_string(),
                        ver.file_name().to_string_lossy().to_string(),
                    ));
                }
            }
        }
        out
    }

    /// Workspace mode: makes the library a set of links to the workspace's mod folders, so the
    /// game runs the files being edited. Old links are removed first; a mod installed into
    /// this library as a real copy is left alone and wins over a workspace folder of the
    /// same id and version. Nothing in the workspace folder is changed.
    pub fn link_workspace_mods(&self, workspace_mods_dir: &Path) -> Result<Vec<InstalledMod>, String> {
        let _ = fs::create_dir_all(&self.library_dir);
        let plain =
            |s: &str| !s.is_empty() && !s.starts_with('.') && !s.contains(['/', '\\', ':']);
        // What the folder holds now: (id, version) -> the mod's folder.
        let mut wanted: Vec<(String, String, PathBuf, InstalledMod)> = Vec::new();
        if let Ok(entries) = fs::read_dir(workspace_mods_dir) {
            for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !path.is_dir() || dir_name.starts_with(['.', '_']) || dir_name == "qm" {
                    continue;
                }
                let is_mod = path.join("qm.toml").is_file()
                    || path.join("mod.json").is_file()
                    || qm_schema::faction::has_faction_file(&path)
                    || has_supported_assets_dir(&path);
                if !is_mod {
                    continue;
                }
                let Ok(inst) = inspect_mod_directory(&path, true) else {
                    continue;
                };
                let id = inst.manifest.id.clone();
                if !plain(&id) || wanted.iter().any(|w| w.0 == id) {
                    continue;
                }
                let version = if plain(&inst.manifest.version) {
                    inst.manifest.version.clone()
                } else {
                    "0.0.0".to_string()
                };
                wanted.push((id, version, path, inst));
            }
        }
        let same_dir = |a: &Path, b: &Path| {
            dunce::canonicalize(a).is_ok_and(|a| dunce::canonicalize(b).is_ok_and(|b| a == b))
        };
        // Links that no longer match go; a link that is still right is left as it is, so
        // nothing that reads through it (the staged profile, a running check) loses it.
        if let Ok(mod_dirs) = fs::read_dir(&self.library_dir) {
            for mod_dir in mod_dirs.filter_map(|e| e.ok()).map(|e| e.path()) {
                let Ok(versions) = fs::read_dir(&mod_dir) else {
                    continue;
                };
                let id = mod_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
                for ver in versions.filter_map(|e| e.ok()).map(|e| e.path()) {
                    if !crate::profiles::is_reparse_point(&ver) {
                        continue;
                    }
                    let version = ver.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let keep = wanted
                        .iter()
                        .any(|w| w.0 == id && w.1 == version && same_dir(&ver, &w.2));
                    if !keep {
                        crate::profiles::remove_junction_or_dir(&ver)?;
                    }
                }
                // Only an emptied folder goes.
                let _ = fs::remove_dir(&mod_dir);
            }
        }
        let mut linked = Vec::new();
        for (id, version, path, mut inst) in wanted {
            let link = self.library_dir.join(&id).join(&version);
            if fs::symlink_metadata(&link).is_err() {
                crate::profiles::create_dir_junction_or_copy(&path, &link)?;
            } else if !crate::profiles::is_reparse_point(&link) {
                // A real copy installed into this library wins over the folder.
                continue;
            }
            inst.install_path = link;
            linked.push(inst);
        }
        Ok(linked)
    }

    /// Fills an empty library from the workspace mods folder, once per folder.
    ///
    /// The folder that last seeded the library is noted in `library/.workspace-seeded`. With
    /// that note in place an empty library stays empty: the user uninstalled everything, and
    /// the mods must not come back by themselves ("Sync now" still copies them). A library
    /// that already has mods is noted as seeded without a copy. Returns whether mods were
    /// copied.
    pub fn seed_library_from_workspace(&self, workspace_mods_dir: &Path) -> Result<bool, String> {
        let note = self.library_dir.join(WORKSPACE_SEEDED_NOTE);
        let folder = workspace_mods_dir.to_string_lossy().to_string();
        if fs::read_to_string(&note).is_ok_and(|seeded| seeded.trim() == folder) {
            return Ok(false);
        }
        let empty = fs::read_dir(&self.library_dir).map_or(true, |rd| {
            !rd.filter_map(Result::ok)
                .any(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'))
        });
        let copied = if empty {
            !self.sync_workspace_mods(workspace_mods_dir)?.is_empty()
        } else {
            false
        };
        let _ = fs::create_dir_all(&self.library_dir);
        let _ = fs::write(&note, &folder);
        Ok(copied)
    }

    /// Inspects a `.zip` archive or folder before importing to discover all contained mods
    /// and check if any are already installed (Section 4: multi-mod archive picker & Upgrade/Replace/Alongside prompt).
    pub fn inspect_archive_candidates(
        &self,
        source_path: &Path,
        allow_unmanaged: bool,
    ) -> Result<Vec<ArchiveModCandidate>, String> {
        if !source_path.exists() {
            return Err(format!("Path does not exist: {}", source_path.display()));
        }
        let existing_mods = self.list_installed_mods();

        let inspect_root =
            |root: &Path, fallback_name: &str| -> Result<Vec<ArchiveModCandidate>, String> {
                let mut mod_roots = find_mod_roots_in_tree(root);
                if mod_roots.is_empty() {
                    if !allow_unmanaged {
                        return Err(format!("No `qm.toml`, `faction.toml` or supported `assets/` directory found in '{fallback_name}'."));
                    }
                    mod_roots.push(root.to_path_buf());
                }

                let mut candidates = Vec::new();
                for mod_root in mod_roots {
                    let default_folder = mod_root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .filter(|s| !s.starts_with("import-inspect-"))
                        .unwrap_or(fallback_name);
                    let manifest = load_or_synthesize_manifest(&mod_root, default_folder)?;
                    check_manifest_segments(&manifest)?;
                    let existing = existing_mods.iter().find(|m| m.manifest.id == manifest.id);
                    candidates.push(ArchiveModCandidate {
                        already_installed_version: existing.map(|m| m.manifest.version.clone()),
                        all_installed_versions: existing
                            .map(|m| m.installed_versions.clone())
                            .unwrap_or_default(),
                        relative_folder: default_folder.to_string(),
                        manifest,
                    });
                }
                Ok(candidates)
            };

        if source_path.is_file() {
            let temp = tempfile::Builder::new()
                .prefix("import-inspect-")
                .tempdir()
                .map_err(|e| e.to_string())?;
            extract_zip_safely(source_path, temp.path())?;
            let fallback = source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("mod");
            inspect_root(temp.path(), fallback)
        } else {
            let fallback = source_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("mod");
            inspect_root(source_path, fallback)
        }
    }

    /// Imports a mod (or multi-mod pack) from a `.zip` archive or directory into `library/<mod_id>/<version>/`.
    pub fn import_mod_from_path(
        &self,
        source_path: &Path,
        source_url: Option<String>,
        allow_unmanaged: bool,
    ) -> Result<ImportSummary, String> {
        self.import_mod_from_path_with_options(source_path, source_url, allow_unmanaged, None, None)
    }

    /// Imports a mod or multi-mod archive with optional selective `selected_mod_ids` filter
    /// and conflict handling mode (`"upgrade"` | `"replace"` | `"alongside"`).
    pub fn import_mod_from_path_with_options(
        &self,
        source_path: &Path,
        source_url: Option<String>,
        allow_unmanaged: bool,
        selected_mod_ids: Option<Vec<String>>,
        conflict_policy: Option<String>,
    ) -> Result<ImportSummary, String> {
        if !source_path.exists() {
            return Err(format!(
                "Import path does not exist: {}",
                source_path.display()
            ));
        }

        let existing_mods = self.list_installed_mods();

        if source_path.is_file() {
            let archive_bytes = fs::read(source_path)
                .map_err(|e| format!("Failed to read archive {}: {e}", source_path.display()))?;
            let archive_sha256 = sha256_hex(&archive_bytes);

            let staging_dir = self
                .cache_dir
                .join(format!("import-staging-{}", &archive_sha256[..12]));
            if staging_dir.exists() {
                let _ = fs::remove_dir_all(&staging_dir);
            }
            fs::create_dir_all(&staging_dir)
                .map_err(|e| format!("Failed to create staging dir: {e}"))?;

            let extract_res = extract_zip_safely(source_path, &staging_dir);
            if let Err(e) = extract_res {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(e);
            }

            let res = self.import_extracted_root(
                &staging_dir,
                source_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("mod"),
                Some(archive_sha256),
                source_url,
                allow_unmanaged,
                &existing_mods,
                selected_mod_ids.as_deref(),
                conflict_policy.as_deref(),
            );
            let _ = fs::remove_dir_all(&staging_dir);
            res
        } else {
            let fallback_name = source_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("mod");
            self.import_extracted_root(
                source_path,
                fallback_name,
                None,
                source_url,
                allow_unmanaged,
                &existing_mods,
                selected_mod_ids.as_deref(),
                conflict_policy.as_deref(),
            )
        }
    }

    fn import_extracted_root(
        &self,
        root: &Path,
        fallback_name: &str,
        archive_sha256: Option<String>,
        source_url: Option<String>,
        allow_unmanaged: bool,
        existing_mods: &[InstalledMod],
        selected_mod_ids: Option<&[String]>,
        conflict_policy: Option<&str>,
    ) -> Result<ImportSummary, String> {
        let mut mod_roots = find_mod_roots_in_tree(root);

        if mod_roots.is_empty() {
            if !allow_unmanaged {
                return Err(format!(
                    "No `qm.toml`, `faction.toml` or supported `assets/` directory found in '{}'. Pass `allow_unmanaged = true` to import as an unmanaged mod.",
                    fallback_name
                ));
            }
            mod_roots.push(root.to_path_buf());
        }

        let was_multi_mod_pack = mod_roots.len() > 1;
        let mut imported_mods = Vec::new();
        let mut upgraded_from_versions = Vec::new();
        let mut cas_deduplicated_files = 0usize;
        let cas_root = self.library_dir.join(".cas");
        let _ = fs::create_dir_all(&cas_root);
        separate_cas_blobs(&cas_root);

        for mod_root in mod_roots {
            let default_folder = mod_root
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.starts_with("import-staging-"))
                .unwrap_or(fallback_name);

            let manifest = load_or_synthesize_manifest(&mod_root, default_folder)?;
            check_manifest_segments(&manifest)?;
            if let Some(filter_ids) = selected_mod_ids {
                if !filter_ids.is_empty() && !filter_ids.contains(&manifest.id) {
                    continue;
                }
            }

            let policy = conflict_policy.unwrap_or("upgrade");
            if let Some(prev) = existing_mods.iter().find(|m| m.manifest.id == manifest.id) {
                if prev.manifest.version != manifest.version {
                    upgraded_from_versions
                        .push((manifest.id.clone(), prev.manifest.version.clone()));
                }
            }

            // Copy-then-swap: build the new version in the staging folder (same volume), and only
            // then remove what it replaces and rename it into place. A failed copy leaves the
            // installed versions untouched.
            let staging = self
                .library_dir
                .join(STAGING_DIR)
                .join(format!("{}-{}", manifest.id, manifest.version));
            if staging.is_dir() {
                let _ = fs::remove_dir_all(&staging);
            }
            let target_dir = self.library_dir.join(&manifest.id).join(&manifest.version);
            let staged = self
                .stage_mod_copy(
                    &mod_root,
                    &staging,
                    &manifest,
                    archive_sha256.clone(),
                    source_url.clone(),
                )
                .and_then(|integrity| {
                    self.swap_into_place(&staging, &target_dir, &manifest.id, policy == "replace")?;
                    Ok(integrity)
                });
            let integrity = match staged {
                Ok(i) => i,
                Err(e) => {
                    remove_any(&staging);
                    return Err(e);
                }
            };

            // Content-Addressed Storage (.cas): store/deduplicate every file by SHA-256 hash
            for (rel_path, sha) in &integrity.files {
                if sha.len() >= 4 {
                    let shard_dir = cas_root.join(&sha[..2]);
                    let cas_blob = shard_dir.join(sha);
                    let file_in_mod = target_dir.join(rel_path);
                    if cas_blob.is_file() {
                        cas_deduplicated_files += 1;
                    } else if file_in_mod.is_file() {
                        // A copy of its own: a hard link would change with the installed
                        // file, and then there is nothing to repair it from.
                        let _ = fs::create_dir_all(&shard_dir);
                        let _ = fs::copy(&file_in_mod, &cas_blob);
                    }
                }
            }

            // When upgrading ("upgrade" or "replace"), unpin any old version in profiles so they seamlessly use the new version
            // while preserving enabled state, load-order position, and per-profile config.
            // When "alongside" is chosen and a previous version was installed, keep existing profiles pinned to their prior version.
            if let Some(prev) = existing_mods.iter().find(|m| m.manifest.id == manifest.id) {
                if prev.manifest.version != manifest.version {
                    for mut prof in self.list_profiles() {
                        let mut changed = false;
                        if policy == "alongside" {
                            if prof.enabled_mods.contains(&manifest.id)
                                && !prof.pinned_versions.contains_key(&manifest.id)
                            {
                                prof.pinned_versions
                                    .insert(manifest.id.clone(), prev.manifest.version.clone());
                                changed = true;
                            }
                        } else if prof.pinned_versions.get(&manifest.id)
                            == Some(&prev.manifest.version)
                        {
                            prof.pinned_versions.remove(&manifest.id);
                            changed = true;
                        }
                        if changed {
                            let _ = self.save_profile(&prof);
                        }
                    }
                }
            }

            let installed = inspect_mod_directory(&target_dir, false)?;
            imported_mods.push(installed);
        }

        Ok(ImportSummary {
            imported_mods,
            was_multi_mod_pack,
            upgraded_from_versions,
            cas_deduplicated_files,
        })
    }

    /// Copy `src` into `staging` and write the normalized `mod.json` and `.qm-integrity.json`.
    fn stage_mod_copy(
        &self,
        src: &Path,
        staging: &Path,
        manifest: &ModManifest,
        archive_sha256: Option<String>,
        source_url: Option<String>,
    ) -> Result<IntegrityManifest, String> {
        copy_directory_clean(src, staging)?;
        let json = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("Failed to serialize mod.json: {e}"))?;
        fs::write(staging.join("mod.json"), json)
            .map_err(|e| format!("Failed to write mod.json: {e}"))?;
        let integrity = IntegrityManifest::compute_for_directory(
            staging,
            &manifest.id,
            &manifest.version,
            archive_sha256,
            source_url,
            jiff::Zoned::now().to_string(),
        )?;
        if let Ok(int_json) = serde_json::to_string_pretty(&integrity) {
            fs::write(staging.join(".qm-integrity.json"), int_json)
                .map_err(|e| format!("Failed to write .qm-integrity.json: {e}"))?;
        }
        Ok(integrity)
    }

    /// Remove what the staged copy replaces (every version for `replace_all`, else only a same
    /// version folder), then rename `staging` to `target_dir`.
    fn swap_into_place(
        &self,
        staging: &Path,
        target_dir: &Path,
        mod_id: &str,
        replace_all: bool,
    ) -> Result<(), String> {
        let mod_parent = self.library_dir.join(mod_id);
        if replace_all && mod_parent.is_dir() {
            let entries = fs::read_dir(&mod_parent)
                .map_err(|e| format!("Failed to read {}: {e}", mod_parent.display()))?;
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.is_dir() {
                    fs::remove_dir_all(&p)
                        .map_err(|e| format!("Failed to delete {}: {e}", p.display()))?;
                }
            }
        } else if target_dir.exists() {
            fs::remove_dir_all(target_dir)
                .map_err(|e| format!("Failed to delete {}: {e}", target_dir.display()))?;
        }
        fs::create_dir_all(&mod_parent)
            .map_err(|e| format!("Failed to create {}: {e}", mod_parent.display()))?;
        // A rename can fail on a transient handle (antivirus); fall back to a copy so the old
        // versions are not lost with nothing in their place.
        match fs::rename(staging, target_dir) {
            Ok(()) => Ok(()),
            Err(e) => copy_directory_clean(staging, target_dir).map_err(|copy_err| {
                format!(
                    "Failed to move {} to {}: {e} ({copy_err})",
                    staging.display(),
                    target_dir.display()
                )
            }),
        }
    }

    /// Re-verifies every installed mod in `library/` against its `.qm-integrity.json` SHA-256 digests,
    /// automatically repairing any modified or missing files from `library/.cas/<sha256>` when available.
    pub fn verify_library(&self) -> Vec<(String, String, IntegrityVerifyResult)> {
        let mut results = Vec::new();
        let cas_root = self.library_dir.join(".cas");
        if cas_root.is_dir() {
            separate_cas_blobs(&cas_root);
        }
        for m in self.list_installed_mods() {
            let mut verify = m.integrity.verify_directory(&m.install_path);
            if !verify.is_pristine && cas_root.is_dir() {
                for rel_file in verify.modified.iter().chain(verify.missing.iter()) {
                    if let Some(sha) = m.integrity.files.get(rel_file) {
                        if sha.len() >= 4 {
                            let cas_blob = cas_root.join(&sha[..2]).join(sha);
                            // Only a stored copy that still has its content is used.
                            let intact = fs::read(&cas_blob)
                                .is_ok_and(|b| sha256_hex(&b).eq_ignore_ascii_case(sha));
                            if intact {
                                let dest = m.install_path.join(rel_file);
                                if let Some(p) = dest.parent() {
                                    let _ = fs::create_dir_all(p);
                                }
                                let _ = fs::copy(&cas_blob, &dest);
                            }
                        }
                    }
                }
                verify = m.integrity.verify_directory(&m.install_path);
            }
            results.push((m.manifest.id, m.manifest.version, verify));
        }
        results
    }

    /// Deletes `library/<mod_id>/` (every version) and removes the id from every profile's
    /// enabled list, priorities, locks, favorites, notes and pinned versions.
    pub fn uninstall_mod(&self, mod_id: &str) -> Result<(), String> {
        if mod_id.is_empty()
            || mod_id == "."
            || mod_id == ".."
            || mod_id.contains(['/', '\\', ':'])
        {
            return Err(format!("Invalid mod id '{mod_id}'"));
        }
        let dir = self.library_dir.join(mod_id);
        if !dir.is_dir() {
            return Err(format!("Mod '{mod_id}' is not installed"));
        }
        // A version that is a link (a workspace mod) is unlinked; its folder is not touched.
        crate::profiles::remove_junction_or_dir(&dir)?;

        for mut profile in self.list_profiles() {
            let before = serde_json::to_string(&profile).unwrap_or_default();
            profile.enabled_mods.retain(|m| m != mod_id);
            profile.mod_priorities.remove(mod_id);
            profile.locked_mods.retain(|m| m != mod_id);
            profile.favorites.retain(|m| m != mod_id);
            profile.mod_notes.remove(mod_id);
            profile.pinned_versions.remove(mod_id);
            if serde_json::to_string(&profile).unwrap_or_default() != before {
                self.save_profile(&profile)?;
            }
        }
        Ok(())
    }
}

/// In the library folder: the workspace mods folder that seeded it (see
/// `seed_library_from_workspace`).
const WORKSPACE_SEEDED_NOTE: &str = ".workspace-seeded";

/// Written into `.cas` once its files are copies of their own.
const CAS_COPIES_MARKER: &str = ".copies";

/// Earlier versions hard-linked each installed file into `.cas`, so both names were one file
/// on disk. Once per library, every stored file is written anew, which gives it its own
/// content. A stored file that no longer has the content its name says (it was edited
/// through the link) is removed. Best effort: a file that cannot be rewritten is left, and
/// the next call tries again.
fn separate_cas_blobs(cas_root: &Path) {
    let marker = cas_root.join(CAS_COPIES_MARKER);
    if marker.is_file() {
        return;
    }
    let mut all_done = true;
    for entry in WalkDir::new(cas_root)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
    {
        let blob = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".tmp") {
            let _ = fs::remove_file(blob);
            continue;
        }
        let Ok(bytes) = fs::read(blob) else {
            all_done = false;
            continue;
        };
        if !sha256_hex(&bytes).eq_ignore_ascii_case(&name) {
            let _ = fs::remove_file(blob);
            continue;
        }
        let tmp = blob.with_extension("tmp");
        let rewritten = fs::write(&tmp, &bytes).and_then(|()| fs::rename(&tmp, blob));
        if rewritten.is_err() {
            let _ = fs::remove_file(&tmp);
            all_done = false;
        }
    }
    if all_done {
        let _ = fs::write(&marker, b"");
    }
}

/// Refuse a manifest whose id or version cannot be used as a library folder name.
fn check_manifest_segments(manifest: &ModManifest) -> Result<(), String> {
    if !is_safe_segment(&manifest.id) {
        return Err(format!(
            "Mod id '{}' is not a valid folder name.",
            manifest.id
        ));
    }
    if !is_safe_segment(&manifest.version) {
        return Err(format!(
            "Mod version '{}' is not a valid folder name.",
            manifest.version
        ));
    }
    Ok(())
}

/// Best-effort removal of a file or directory tree.
fn remove_any(p: &Path) {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(p);
        }
        Ok(_) => {
            let _ = fs::remove_file(p);
        }
        Err(_) => {}
    }
}

/// Safely extracts a `.zip` archive with zip-slip, symlink, file-count, and zip-bomb protections.
pub fn extract_zip_safely(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path)
        .map_err(|e| format!("Failed to open zip {}: {e}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("Invalid zip archive {}: {e}", zip_path.display()))?;

    if archive.len() > MAX_ARCHIVE_FILES {
        return Err(format!(
            "Archive contains {} entries, exceeding safety limit of {}",
            archive.len(),
            MAX_ARCHIVE_FILES
        ));
    }

    let mut total_uncompressed: u64 = 0;
    // What was really written: an entry's header can understate its size.
    let mut total_written: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry #{i}: {e}"))?;

        if entry.is_symlink() {
            return Err(format!(
                "Rejected archive containing symlink entry '{}'",
                entry.name()
            ));
        }

        let raw_name = entry.name().to_string();
        if entry.is_dir() || raw_name.ends_with('/') || raw_name.ends_with('\\') {
            let safe_rel = validate_safe_relative_path(&raw_name)?;
            fs::create_dir_all(dest_dir.join(safe_rel))
                .map_err(|e| format!("Failed to create dir: {e}"))?;
            continue;
        }

        let safe_rel = validate_safe_relative_path(&raw_name)?;
        let uncompressed_size = entry.size();
        let compressed_size = entry.compressed_size().max(1);

        if uncompressed_size > 1024 * 1024
            && (uncompressed_size / compressed_size) > MAX_COMPRESSION_RATIO
        {
            return Err(format!(
                "Rejected potential zip bomb in entry '{}' (compression ratio {}:1)",
                raw_name,
                uncompressed_size / compressed_size
            ));
        }

        total_uncompressed = total_uncompressed.saturating_add(uncompressed_size);
        if total_uncompressed > MAX_ARCHIVE_TOTAL_BYTES {
            return Err(format!(
                "Archive exceeds maximum uncompressed size limit ({} MB)",
                MAX_ARCHIVE_TOTAL_BYTES / (1024 * 1024)
            ));
        }

        let out_path = dest_dir.join(safe_rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
        }

        let mut limited_reader = (&mut entry).take(MAX_ARCHIVE_TOTAL_BYTES + 1);
        let mut buf = Vec::new();
        limited_reader
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to extract '{}': {e}", raw_name))?;
        total_written = total_written.saturating_add(buf.len() as u64);
        if total_written > MAX_ARCHIVE_TOTAL_BYTES {
            return Err(format!(
                "Archive exceeds maximum uncompressed size limit ({} MB)",
                MAX_ARCHIVE_TOTAL_BYTES / (1024 * 1024)
            ));
        }
        fs::write(&out_path, &buf)
            .map_err(|e| format!("Failed to write '{}': {e}", out_path.display()))?;
    }

    Ok(())
}

/// Returns `true` if `mod_dir/assets/` contains at least one supported asset file.
pub fn has_supported_assets_dir(mod_dir: &Path) -> bool {
    !qm_core::patch::loader::discover_dir_assets(mod_dir).is_empty()
}

/// Discovers all mod root directories inside `root`: those with a `qm.toml` / `quartermaster.toml`,
/// a `faction.toml` (a faction mod has no patch file), or a supported `assets/` directory.
fn find_mod_roots_in_tree(root: &Path) -> Vec<PathBuf> {
    let mut mod_roots = Vec::new();
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_file() {
            let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if fname == "qm.toml"
                || fname == "quartermaster.toml"
                || fname == qm_schema::faction::FACTION_FILE
            {
                if let Some(parent) = p.parent() {
                    if !mod_roots.contains(&parent.to_path_buf()) {
                        mod_roots.push(parent.to_path_buf());
                    }
                }
            }
        } else if p.is_dir() {
            let dname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if dname.eq_ignore_ascii_case("assets") {
                if let Some(parent) = p.parent() {
                    if !mod_roots.contains(&parent.to_path_buf()) && has_supported_assets_dir(parent) {
                        mod_roots.push(parent.to_path_buf());
                    }
                }
            }
        }
    }
    mod_roots
}

fn load_or_synthesize_manifest(
    mod_root: &Path,
    fallback_name: &str,
) -> Result<ModManifest, String> {
    let mod_json_path = mod_root.join("mod.json");
    if mod_json_path.is_file() {
        return ModManifest::from_path(&mod_json_path);
    }

    // Fallback: synthesize from qm.toml [manifest] if mod.json is missing
    let qm_toml_path = mod_root.join("qm.toml");
    let (version, priority) = if qm_toml_path.is_file() {
        let raw = fs::read_to_string(&qm_toml_path)
            .map_err(|e| format!("Failed to read qm.toml: {e}"))?;
        if let Ok(pf) = toml::from_str::<PatchFile>(&raw) {
            (pf.manifest.version, pf.manifest.priority)
        } else {
            ("1.0.0".to_string(), 50)
        }
    } else {
        ("1.0.0".to_string(), 50)
    };

    let id = fallback_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();

    let mut m = ModManifest::new(id, fallback_name.to_string(), version);
    m.authors = vec!["Unknown Author".to_string()];
    m.category = Some("Other".to_string());
    m.load_priority = priority;
    Ok(m)
}

pub fn inspect_mod_directory(
    mod_dir: &Path,
    is_workspace_linked: bool,
) -> Result<InstalledMod, String> {
    let folder_name = mod_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("mod")
        .to_string();

    let manifest = load_or_synthesize_manifest(mod_dir, &folder_name)?;

    let integrity_path = mod_dir.join(".qm-integrity.json");
    let integrity = if let Ok(raw) = fs::read_to_string(&integrity_path) {
        serde_json::from_str::<IntegrityManifest>(&raw).unwrap_or_else(|_| {
            IntegrityManifest::compute_for_directory(
                mod_dir,
                &manifest.id,
                &manifest.version,
                None,
                None,
                jiff::Zoned::now().to_string(),
            )
            .unwrap_or_else(|_| empty_integrity(&manifest.id, &manifest.version, mod_dir))
        })
    } else {
        IntegrityManifest::compute_for_directory(
            mod_dir,
            &manifest.id,
            &manifest.version,
            None,
            None,
            jiff::Zoned::now().to_string(),
        )
        .unwrap_or_else(|_| empty_integrity(&manifest.id, &manifest.version, mod_dir))
    };

    let config_schema = ModConfigSchema::from_mod_dir(mod_dir).ok().flatten();

    let readme_markdown = ["README.md", "readme.md", "README.txt"]
        .iter()
        .find_map(|name| fs::read_to_string(mod_dir.join(name)).ok());
    let changelog_markdown = ["CHANGELOG.md", "changelog.md", "HISTORY.md"]
        .iter()
        .find_map(|name| fs::read_to_string(mod_dir.join(name)).ok());

    let mut patch_count = 0usize;
    let mut targets_set = BTreeSet::new();
    let qm_toml_path = mod_dir.join(&manifest.entry_patch);
    if let Ok(raw_toml) = fs::read_to_string(&qm_toml_path) {
        if let Ok(pf) = toml::from_str::<PatchFile>(&raw_toml) {
            patch_count = pf.patches.len();
            for p in &pf.patches {
                let t = p.primary_target().trim_start_matches('@');
                if !t.is_empty() {
                    targets_set.insert(t.to_string());
                }
            }
        }
    }
    if has_supported_assets_dir(mod_dir) {
        patch_count += 1;
        targets_set.insert("main.lua".to_string());
    }

    Ok(InstalledMod {
        manifest,
        folder_name,
        install_path: mod_dir.to_path_buf(),
        integrity,
        config_schema,
        readme_markdown,
        changelog_markdown,
        patch_count,
        target_files: targets_set.into_iter().collect(),
        is_workspace_linked,
        installed_versions: Vec::new(),
        kind: ModKind::of_dir(mod_dir),
    })
}

fn empty_integrity(mod_id: &str, version: &str, mod_dir: &Path) -> IntegrityManifest {
    IntegrityManifest {
        mod_id: mod_id.to_string(),
        version: version.to_string(),
        archive_sha256: None,
        source_url: None,
        imported_at: String::new(),
        files: Default::default(),
        security_report: SecurityScanReport::scan_directory(mod_dir),
    }
}

pub fn copy_directory_clean(src: &Path, dst: &Path) -> Result<(), String> {
    copy_directory_skipping(src, dst, &|_| false)
}

/// `copy_directory_clean` without the files for which `skip(relative path)` is true.
pub fn copy_directory_skipping(
    src: &Path,
    dst: &Path,
    skip: &dyn Fn(&Path) -> bool,
) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create directory {}: {e}", dst.display()))?;
    for entry in WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel = path
            .strip_prefix(src)
            .map_err(|e| format!("Prefix error: {e}"))?;
        if rel.as_os_str().is_empty() {
            continue;
        }
        if path.is_file() && skip(rel) {
            continue;
        }
        let target = dst.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| format!("Failed to create {}: {e}", target.display()))?;
        } else if path.is_file() {
            if let Some(p) = target.parent() {
                let _ = fs::create_dir_all(p);
            }
            fs::copy(path, &target).map_err(|e| {
                format!(
                    "Failed to copy {} to {}: {e}",
                    path.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zip_slip_and_reserved_windows_names() {
        assert!(validate_safe_relative_path("../../Windows/System32/cmd.exe").is_err());
        assert!(validate_safe_relative_path("/etc/passwd").is_err());
        assert!(validate_safe_relative_path("C:\\evil.lua").is_err());
        assert!(validate_safe_relative_path("mod/CON.lua").is_err());
        assert!(validate_safe_relative_path("mod/NUL").is_err());
        assert!(validate_safe_relative_path("mod/trailing.").is_err());
        assert!(validate_safe_relative_path("GCore/qm.toml").is_ok());
    }

    /// A source folder whose `mod.json` declares `id` and `version` verbatim.
    fn hostile_mod(root: &Path, name: &str, id: &str, version: &str) -> PathBuf {
        let dir = root.join("hostile").join(name);
        fs::create_dir_all(&dir).unwrap();
        let json = serde_json::json!({ "id": id, "name": "Hostile", "version": version });
        fs::write(dir.join("mod.json"), json.to_string()).unwrap();
        fs::write(dir.join("qm.toml"), "[manifest]\nversion = \"1.0.0\"\n").unwrap();
        dir
    }

    #[test]
    fn rejects_manifest_id_dotdot_and_separators() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        for (i, id) in [
            "..", ".", "a/b", "a\\b", "c:x", "CON", "nul.txt", "x.", "x ",
        ]
        .iter()
        .enumerate()
        {
            let src = hostile_mod(temp.path(), &format!("m{i}"), id, "1.0.0");
            let want = format!("Mod id '{id}' is not a valid folder name.");
            let err = paths.inspect_archive_candidates(&src, false).unwrap_err();
            assert_eq!(err, want);
            let err = paths.import_mod_from_path(&src, None, false).unwrap_err();
            assert_eq!(err, want);
        }
        // Nothing escaped into or next to the library.
        assert!(!temp.path().join("1.0.0").exists());
        assert!(paths.list_installed_mods().is_empty());
    }

    #[test]
    fn rejects_manifest_version_with_separator() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        for (i, v) in ["../../evil", "1.0/2", "1\\0", ".."].iter().enumerate() {
            let src = hostile_mod(temp.path(), &format!("v{i}"), "fine", v);
            let want = format!("Mod version '{v}' is not a valid folder name.");
            let err = paths.inspect_archive_candidates(&src, false).unwrap_err();
            assert_eq!(err, want);
            let err = paths.import_mod_from_path(&src, None, false).unwrap_err();
            assert_eq!(err, want);
        }
        assert!(!paths.library_dir.join("fine").exists());
    }

    /// The content store keeps copies of its own, so an installed file edited in place is
    /// repaired; a store made with hard links by an earlier version is separated first.
    #[test]
    fn verify_library_repairs_a_file_edited_in_place() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let src = temp.path().join("downloads/ExampleMod");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("mod.json"), r#"{"id": "example_mod", "name": "Example Mod", "version": "1.0.0"}"#).unwrap();
        fs::write(src.join("qm.toml"), "[manifest]\nversion = \"1.0.0\"\n").unwrap();
        fs::write(src.join("code.lua"), "return 1\n").unwrap();
        let imported = paths.import_mod_from_path(&src, None, false).unwrap();
        let installed = imported.imported_mods[0].install_path.join("code.lua");

        // Written through the same file, as an editor that saves in place does.
        let edit = |text: &str| {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().write(true).truncate(true).open(&installed).unwrap();
            f.write_all(text.as_bytes()).unwrap();
        };
        edit("return 2\n");
        let results = paths.verify_library();
        assert!(results.iter().all(|(_, _, r)| r.is_pristine), "{results:?}");
        assert_eq!(fs::read_to_string(&installed).unwrap(), "return 1\n");

        // A store from an earlier version: the stored file is a hard link to the installed one.
        let sha = sha256_hex(b"return 1\n");
        let cas = paths.library_dir.join(".cas");
        let blob = cas.join(&sha[..2]).join(&sha);
        fs::remove_file(&blob).unwrap();
        fs::hard_link(&installed, &blob).unwrap();
        fs::remove_file(cas.join(CAS_COPIES_MARKER)).unwrap();
        separate_cas_blobs(&cas);
        assert!(cas.join(CAS_COPIES_MARKER).is_file());
        edit("return 3\n");
        assert_eq!(fs::read_to_string(&blob).unwrap(), "return 1\n", "the stored copy is its own file");
        let results = paths.verify_library();
        assert!(results.iter().all(|(_, _, r)| r.is_pristine), "{results:?}");

        // A stored file that was already changed through its link is not used for repair.
        fs::write(&blob, "return 9\n").unwrap();
        edit("return 4\n");
        let results = paths.verify_library();
        assert!(results.iter().any(|(_, _, r)| !r.is_pristine));
        assert_eq!(fs::read_to_string(&installed).unwrap(), "return 4\n");
    }

    #[test]
    fn safe_segment_rules() {
        assert!(is_safe_segment("gcore"));
        assert!(is_safe_segment("1.2.3-beta"));
        assert!(is_safe_segment("console"));
        for bad in [
            "", ".", "..", "a/b", "a\\b", "a:b", "COM1", "lpt9.x", "x.", "x ",
        ] {
            assert!(!is_safe_segment(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_emptied_library_is_not_seeded_from_the_same_workspace_again() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let ws_mods = temp.path().join("ws_mods");
        fs::create_dir_all(ws_mods.join("SoundPack/assets/sounds")).unwrap();
        fs::write(ws_mods.join("SoundPack/assets/sounds/shot.wav"), b"RIFF").unwrap();

        assert!(paths.seed_library_from_workspace(&ws_mods).unwrap());
        assert_eq!(paths.list_installed_mods().len(), 1);

        // The user uninstalls everything: the next start must leave the library empty.
        paths.uninstall_mod("soundpack").unwrap();
        assert!(!paths.seed_library_from_workspace(&ws_mods).unwrap());
        assert!(paths.list_installed_mods().is_empty());

        // Another workspace folder seeds the empty library.
        let other = temp.path().join("other_ws");
        fs::create_dir_all(other.join("MusicPack/assets/music")).unwrap();
        fs::write(other.join("MusicPack/assets/music/theme.mp3"), b"ID3").unwrap();
        assert!(paths.seed_library_from_workspace(&other).unwrap());
        assert_eq!(paths.list_installed_mods().len(), 1);
    }

    #[test]
    fn imports_and_syncs_asset_only_mods_without_qm_toml() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        use zip::ZipWriter;

        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let ws_mods = temp.path().join("ws_mods");
        fs::create_dir_all(ws_mods.join("PureSoundPack/assets/sounds")).unwrap();
        fs::create_dir_all(ws_mods.join("PureSoundPack/assets/music")).unwrap();
        fs::write(
            ws_mods.join("PureSoundPack/assets/sounds/garand1.wav"),
            b"RIFF_GARAND",
        )
        .unwrap();
        fs::write(
            ws_mods.join("PureSoundPack/assets/music/MudAndBloodFull.mp3"),
            b"ID3_MUSIC",
        )
        .unwrap();

        let synced = paths.sync_workspace_mods(&ws_mods).unwrap();
        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].manifest.id, "puresoundpack");
        assert_eq!(synced[0].patch_count, 1);
        assert_eq!(synced[0].target_files, vec!["main.lua".to_string()]);

        let zip_path = temp.path().join("PureSpritePack.zip");
        {
            let mut zip = ZipWriter::new(fs::File::create(&zip_path).unwrap());
            zip.start_file(
                "PureSpritePack/assets/sprites/top_garand.png",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"PNG_GARAND").unwrap();
            zip.finish().unwrap();
        }

        let candidates = paths.inspect_archive_candidates(&zip_path, false).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].manifest.id, "purespritepack");

        let imported = paths.import_mod_from_path(&zip_path, None, false).unwrap();
        assert_eq!(imported.imported_mods.len(), 1);
        assert_eq!(imported.imported_mods[0].manifest.id, "purespritepack");
        assert_eq!(imported.imported_mods[0].patch_count, 1);
        assert_eq!(
            imported.imported_mods[0].target_files,
            vec!["main.lua".to_string()]
        );
    }

    #[test]
    fn imports_a_faction_only_mod_without_qm_toml() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let src = temp.path().join("downloads/ExampleFaction");
        fs::create_dir_all(src.join("frames")).unwrap();
        fs::write(
            src.join("mod.json"),
            r#"{"id": "example_faction", "name": "Example Faction", "version": "1.0.0"}"#,
        )
        .unwrap();
        fs::write(
            src.join("faction.toml"),
            "schema = 1\nid = \"example\"\nname = \"Example\"\nreplaces = \"allied\"\n",
        )
        .unwrap();

        let candidates = paths.inspect_archive_candidates(&src, false).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].manifest.id, "example_faction");

        let imported = paths.import_mod_from_path(&src, None, false).unwrap();
        assert_eq!(imported.imported_mods.len(), 1);
        assert_eq!(imported.imported_mods[0].manifest.id, "example_faction");
        assert_eq!(imported.imported_mods[0].kind, ModKind::Faction);
    }

    #[test]
    fn test_library_loads_config_schema_from_toml() {
        let temp = tempfile::TempDir::new().unwrap();
        let mod_dir = temp.path().join("test_toml_mod");
        fs::create_dir_all(&mod_dir).unwrap();

        let mod_json = r#"{
            "id": "test_toml_mod",
            "name": "Test TOML Mod",
            "version": "1.0.0",
            "author": "Tester",
            "description": "Mod with config.toml"
        }"#;
        fs::write(mod_dir.join("mod.json"), mod_json).unwrap();

        let config_toml = r#"
id = "test_toml_mod"
name = "Test TOML Mod"
[[options]]
id = "test_opt"
label = "Test Option"
type = "checkbox"
default = true
"#;
        fs::write(mod_dir.join("config.toml"), config_toml).unwrap();

        let installed = inspect_mod_directory(&mod_dir, false).unwrap();
        assert_eq!(installed.manifest.id, "test_toml_mod");
        let cfg = installed
            .config_schema
            .as_ref()
            .expect("config_schema must be loaded from config.toml");
        assert_eq!(cfg.name, "Test TOML Mod");
        assert_eq!(cfg.options.len(), 1);
        assert_eq!(cfg.options[0].id, "test_opt");
    }
}
