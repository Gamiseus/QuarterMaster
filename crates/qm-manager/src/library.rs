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
                let stem = seg.split('.').next().unwrap_or(seg).to_ascii_uppercase();
                let reserved = [
                    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6",
                    "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
                    "LPT8", "LPT9",
                ];
                if reserved.contains(&stem.as_str()) {
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
                if !ver_path.is_dir() {
                    continue;
                }
                if let Ok(installed) = inspect_mod_directory(&ver_path, false) {
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
        if let Some(ver) = version {
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
            if path.join("qm.toml").is_file() || path.join("mod.json").is_file() {
                if let Ok(summary) = self.import_mod_from_path(&path, None, false) {
                    synced.extend(summary.imported_mods);
                }
            }
        }
        Ok(synced)
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
                let mut mod_roots = Vec::new();
                for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
                    let p = entry.path();
                    if p.is_file() {
                        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if fname == "qm.toml" || fname == "quartermaster.toml" {
                            if let Some(parent) = p.parent() {
                                if !mod_roots.contains(&parent.to_path_buf()) {
                                    mod_roots.push(parent.to_path_buf());
                                }
                            }
                        }
                    }
                }
                if mod_roots.is_empty() {
                    if !allow_unmanaged {
                        return Err(format!("No `qm.toml` found in '{fallback_name}'."));
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
        // Search downward recursively for all folders containing `qm.toml` or `quartermaster.toml`
        let mut mod_roots = Vec::new();
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_file() {
                let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if fname == "qm.toml" || fname == "quartermaster.toml" {
                    if let Some(parent) = p.parent() {
                        if !mod_roots.contains(&parent.to_path_buf()) {
                            mod_roots.push(parent.to_path_buf());
                        }
                    }
                }
            }
        }

        if mod_roots.is_empty() {
            if !allow_unmanaged {
                return Err(format!(
                    "No `qm.toml` found in '{}'. Pass `allow_unmanaged = true` to import as an unmanaged mod.",
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

        for mod_root in mod_roots {
            let default_folder = mod_root
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.starts_with("import-staging-"))
                .unwrap_or(fallback_name);

            let manifest = load_or_synthesize_manifest(&mod_root, default_folder)?;
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
                if policy == "replace" {
                    // Remove older version directories when "replace" is chosen
                    let mod_parent = self.library_dir.join(&manifest.id);
                    if mod_parent.is_dir() {
                        let _ = fs::remove_dir_all(&mod_parent);
                    }
                }
            }

            let target_dir = self.library_dir.join(&manifest.id).join(&manifest.version);
            if target_dir.exists() {
                let _ = fs::remove_dir_all(&target_dir);
            }
            copy_directory_clean(&mod_root, &target_dir)?;

            // Ensure normalized mod.json is written inside the library copy
            let mod_json_path = target_dir.join("mod.json");
            if let Ok(json_str) = serde_json::to_string_pretty(&manifest) {
                let _ = fs::write(&mod_json_path, json_str);
            }

            // Compute and write .qm-integrity.json
            let now = jiff::Zoned::now().to_string();
            let integrity = IntegrityManifest::compute_for_directory(
                &target_dir,
                &manifest.id,
                &manifest.version,
                archive_sha256.clone(),
                source_url.clone(),
                now,
            )?;
            if let Ok(int_json) = serde_json::to_string_pretty(&integrity) {
                let _ = fs::write(target_dir.join(".qm-integrity.json"), int_json);
            }

            // Content-Addressed Storage (.cas): store/deduplicate every file by SHA-256 hash
            for (rel_path, sha) in &integrity.files {
                if sha.len() >= 4 {
                    let shard_dir = cas_root.join(&sha[..2]);
                    let cas_blob = shard_dir.join(sha);
                    let file_in_mod = target_dir.join(rel_path);
                    if cas_blob.is_file() {
                        cas_deduplicated_files += 1;
                    } else if file_in_mod.is_file() {
                        let _ = fs::create_dir_all(&shard_dir);
                        let _ = fs::hard_link(&file_in_mod, &cas_blob)
                            .or_else(|_| fs::copy(&file_in_mod, &cas_blob).map(|_| ()));
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

    /// Re-verifies every installed mod in `library/` against its `.qm-integrity.json` SHA-256 digests,
    /// automatically repairing any modified or missing files from `library/.cas/<sha256>` when available.
    pub fn verify_library(&self) -> Vec<(String, String, IntegrityVerifyResult)> {
        let mut results = Vec::new();
        let cas_root = self.library_dir.join(".cas");
        for m in self.list_installed_mods() {
            let mut verify = m.integrity.verify_directory(&m.install_path);
            if !verify.is_pristine && cas_root.is_dir() {
                for rel_file in verify.modified.iter().chain(verify.missing.iter()) {
                    if let Some(sha) = m.integrity.files.get(rel_file) {
                        if sha.len() >= 4 {
                            let cas_blob = cas_root.join(&sha[..2]).join(sha);
                            if cas_blob.is_file() {
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
        if buf.len() as u64 > MAX_ARCHIVE_TOTAL_BYTES {
            return Err("Archive entry exceeded size limit during stream read".to_string());
        }
        fs::write(&out_path, &buf)
            .map_err(|e| format!("Failed to write '{}': {e}", out_path.display()))?;
    }

    Ok(())
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

    let config_path = mod_dir.join("config.lua");
    let config_schema = if config_path.is_file() {
        ModConfigSchema::from_path(&config_path).ok()
    } else {
        None
    };

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
}
