//! Modpacks (`.qmpack`): a profile's mods, order and settings in one file, with the mod files
//! bundled under `mods/<id>/` so a pack installs without downloads.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::events::Scope;
use crate::jobs::{JobId, JobKind};
use crate::service::Service;
use qm_manager::devtools::ModpackManifest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Largest `qmpack.json` read.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModpackPreviewMod {
    pub id: String,
    pub name: String,
    pub version: String,
    /// This exact version is already installed.
    pub installed: bool,
    /// The pack carries the mod's files.
    pub bundled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModpackPreview {
    pub name: String,
    pub description: String,
    pub mods: Vec<ModpackPreviewMod>,
}

impl Service {
    /// Write `profile_id` as a modpack to `dest` (a path from the native save dialog) in a job.
    pub fn modpack_export(&self, profile_id: &str, dest: PathBuf) -> QmResult<JobId> {
        self.profile_get(profile_id)?;
        let id = profile_id.to_string();
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::ModpackExport, move |ctx| {
            ctx.progress(0.1, "Collecting the profile's mods");
            let paths = svc.paths();
            let manifest = paths
                .export_modpack_manifest(&id)
                .or_kind(ErrorKind::NotFound)?;
            let installed = paths.list_installed_mods();
            let tmp = dest.with_extension("qmpack.part");
            let file = std::fs::File::create(&tmp).or_kind(ErrorKind::Io)?;
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            let json = serde_json::to_string_pretty(&manifest)
                .map_err(|e| QmError::new(ErrorKind::Internal, e.to_string()))?;
            zip.start_file("qmpack.json", opts).or_kind_zip()?;
            zip.write_all(json.as_bytes()).or_kind(ErrorKind::Io)?;
            let total = manifest.mods.len().max(1);
            for (i, entry) in manifest.mods.iter().enumerate() {
                ctx.progress(
                    0.1 + 0.85 * i as f32 / total as f32,
                    &format!("Packing {}", entry.name),
                );
                let Some(m) = installed
                    .iter()
                    .find(|m| m.manifest.id == entry.id && m.manifest.version == entry.version)
                    .or_else(|| installed.iter().find(|m| m.manifest.id == entry.id))
                else {
                    continue;
                };
                add_dir(
                    &mut zip,
                    &m.install_path,
                    &format!("mods/{}", entry.id),
                    opts,
                )?;
            }
            zip.finish().or_kind_zip()?;
            std::fs::rename(&tmp, &dest).or_kind(ErrorKind::Io)?;
            Ok(dest.file_name().map(|n| n.to_string_lossy().into_owned()))
        }))
    }

    /// What a modpack holds, before importing it.
    pub fn modpack_inspect(&self, path: &Path) -> QmResult<ModpackPreview> {
        let (manifest, bundled) = read_pack(path)?;
        let installed = self.paths().list_installed_mods();
        let mods = manifest
            .mods
            .iter()
            .map(|e| ModpackPreviewMod {
                id: e.id.clone(),
                name: e.name.clone(),
                version: e.version.clone(),
                installed: installed.iter().any(|m| {
                    m.manifest.id == e.id
                        && (m.manifest.version == e.version
                            || m.installed_versions.contains(&e.version))
                }),
                bundled: bundled.contains(&e.id),
            })
            .collect();
        Ok(ModpackPreview {
            name: manifest.name,
            description: manifest.description,
            mods,
        })
    }

    /// Install the pack's bundled mods that are not installed yet, then create a profile named
    /// `name` from it. `slot` is an existing save slot, or `None` for a new slot on first launch.
    /// One job; it holds the install lock and is refused while the game runs.
    pub fn modpack_import(
        &self,
        path: PathBuf,
        name: String,
        slot: Option<String>,
    ) -> QmResult<JobId> {
        if name.trim().is_empty() {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                "The profile needs a name.",
            ));
        }
        if slot.as_deref() == Some(qm_manager::VANILLA_ID) {
            return Err(QmError::new(
                ErrorKind::InvalidInput,
                crate::ops::profiles::MSG_VANILLA_SLOT,
            ));
        }
        if let Some(s) = slot.as_deref() {
            if !self.paths().list_save_profiles().iter().any(|p| p.id == s) {
                return Err(QmError::new(
                    ErrorKind::NotFound,
                    format!("Save slot '{s}' does not exist."),
                ));
            }
        }
        let preview = self.modpack_inspect(&path)?;
        self.ensure_not_running()?;
        let svc = self.clone();
        Ok(self.jobs().spawn(JobKind::ModpackImport, move |ctx| {
            let _install = svc.lock_install();
            svc.ensure_not_running()?;
            let paths = svc.paths();
            let to_install: Vec<String> = preview
                .mods
                .iter()
                .filter(|m| m.bundled && !m.installed)
                .map(|m| m.id.clone())
                .collect();
            if !to_install.is_empty() {
                ctx.progress(0.2, "Installing the pack's mods");
                // Alongside: other installed versions stay, so other profiles keep theirs.
                paths
                    .import_mod_from_path_with_options(
                        &path,
                        None,
                        false,
                        Some(to_install.clone()),
                        Some("alongside".into()),
                    )
                    .or_kind(ErrorKind::InvalidInput)?;
            }
            ctx.progress(0.8, "Creating the profile");
            let (manifest, _) = read_pack(&path)?;
            let id = unique_profile_id(&svc, &name);
            let mut profile = paths
                .apply_modpack_as_profile(&manifest, &id)
                .or_kind(ErrorKind::Io)?;
            profile.name = name.trim().to_string();
            profile.linked_save_profile_id = slot.clone().unwrap_or_default();
            // Each mod at the pack's version.
            for m in &manifest.mods {
                profile
                    .pinned_versions
                    .insert(m.id.clone(), m.version.clone());
            }
            svc.save_profile(&profile).or_kind(ErrorKind::Io)?;
            svc.after_mod_import(&to_install)?;
            svc.notify(Scope::Profiles);
            Ok(Some(profile.id))
        }))
    }
}

trait ZipKind<T> {
    fn or_kind_zip(self) -> QmResult<T>;
}

impl<T> ZipKind<T> for Result<T, zip::result::ZipError> {
    fn or_kind_zip(self) -> QmResult<T> {
        self.map_err(|e| {
            QmError::new(ErrorKind::Io, "The modpack could not be written.")
                .with_details(e.to_string())
        })
    }
}

/// Adds `dir` to the archive under `prefix`, skipping version-control folders.
fn add_dir(
    zip: &mut zip::ZipWriter<std::fs::File>,
    dir: &Path,
    prefix: &str,
    opts: zip::write::SimpleFileOptions,
) -> QmResult<()> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Ok(rel) = p.strip_prefix(dir) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            zip.start_file(format!("{prefix}/{rel}"), opts)
                .or_kind_zip()?;
            let mut f = std::fs::File::open(&p).or_kind(ErrorKind::Io)?;
            std::io::copy(&mut f, zip).or_kind(ErrorKind::Io)?;
        }
    }
    Ok(())
}

/// The manifest and the ids of the mods whose files the pack carries.
fn read_pack(path: &Path) -> QmResult<(ModpackManifest, HashSet<String>)> {
    let bad = |details: String| {
        QmError::new(
            ErrorKind::InvalidInput,
            "This is not a QuarterMaster modpack.",
        )
        .with_details(details)
    };
    let file = std::fs::File::open(path).map_err(|e| {
        QmError::new(
            ErrorKind::NotFound,
            format!("Path does not exist: {}", path.display()),
        )
        .with_details(e.to_string())
    })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| bad(e.to_string()))?;
    let manifest: ModpackManifest = {
        let entry = zip
            .by_name("qmpack.json")
            .map_err(|_| bad("qmpack.json is missing".into()))?;
        if entry.size() > MAX_MANIFEST_BYTES {
            return Err(bad("qmpack.json is too large".into()));
        }
        let mut raw = String::new();
        entry
            .take(MAX_MANIFEST_BYTES)
            .read_to_string(&mut raw)
            .map_err(|e| bad(e.to_string()))?;
        serde_json::from_str(&raw).map_err(|e| bad(e.to_string()))?
    };
    let bundled = (0..zip.len())
        .filter_map(|i| zip.name_for_index(i).map(str::to_string))
        .filter_map(|n| {
            n.strip_prefix("mods/")
                .and_then(|r| r.split('/').next())
                .map(str::to_string)
        })
        .filter(|id| !id.is_empty())
        .collect();
    Ok((manifest, bundled))
}

fn unique_profile_id(svc: &Service, name: &str) -> String {
    let base: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() || base == qm_manager::VANILLA_ID {
        "modpack".to_string()
    } else {
        base
    };
    let taken: HashSet<String> = svc
        .paths()
        .list_profiles()
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}
