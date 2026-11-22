use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use qm_core::handshake::{CrashReport, PatchOutcomeStatus};
use qm_core::patch::table::PatchTable;
use qm_schema::{
    sha256_hex, DiagnosticSeverity, ManifestDiagnostic, ModSettingsStore,
    ReleaseModEntry, SecurityScanReport,
};

use crate::library::{inspect_mod_directory, validate_safe_relative_path, InstalledMod};
use crate::profiles::ModProfile;
use crate::storage::QuarterMasterPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackManifest {
    pub schema_version: u32,
    pub name: String,
    pub description: String,
    pub exported_at: String,
    pub mods: Vec<ModpackModEntry>,
    pub mod_priorities: HashMap<String, i32>,
    pub mod_settings: ModSettingsStore,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackModEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub priority: i32,
    pub sha256: Option<String>,
    pub update_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackImportPreview {
    pub manifest: ModpackManifest,
    pub installed_ready: Vec<String>,
    pub missing_mods: Vec<ModpackModEntry>,
}

/// Encodes a `ModpackManifest` into an offline shareable `QM1-...` Base64URL profile code.
pub fn encode_profile_code(pack: &ModpackManifest) -> Result<String, String> {
    let json = serde_json::to_vec(pack).map_err(|e| format!("Serialize error: {e}"))?;
    Ok(format!("QM1-{}", base64url_encode(&json)))
}

/// Decodes a `QM1-...` shareable profile code back into a `ModpackManifest`.
pub fn decode_profile_code(code: &str) -> Result<ModpackManifest, String> {
    let trimmed = code.trim();
    let payload = trimmed
        .strip_prefix("QM1-")
        .ok_or_else(|| "Invalid profile code prefix (expected 'QM1-')".to_string())?;
    let bytes = base64url_decode(payload)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid profile code JSON: {e}"))
}

fn base64url_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((input.len() * 4).div_ceil(3));
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHABET[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
    }
    out
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;

    for ch in input.chars() {
        if ch.is_ascii_whitespace() || ch == '=' {
            continue;
        }
        let val = match ch {
            'A'..='Z' => (ch as u8 - b'A') as u32,
            'a'..='z' => (ch as u8 - b'a' + 26) as u32,
            '0'..='9' => (ch as u8 - b'0' + 52) as u32,
            '-' | '+' => 62,
            '_' | '/' => 63,
            _ => return Err(format!("Invalid character '{ch}' in profile code")),
        };
        buf = (buf << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Ok(out)
}

impl QuarterMasterPaths {
    pub fn export_modpack_manifest(&self, profile_id: &str) -> Result<ModpackManifest, String> {
        let profile = self.get_profile(profile_id)?;
        let installed = self.list_installed_mods();
        let by_id: HashMap<String, &InstalledMod> = installed
            .iter()
            .map(|m| (m.manifest.id.clone(), m))
            .collect();

        let mut mods = Vec::new();
        for mod_id in &profile.enabled_mods {
            if let Some(inst) = by_id.get(mod_id) {
                let prio = profile
                    .mod_priorities
                    .get(mod_id)
                    .copied()
                    .unwrap_or(inst.manifest.load_priority);
                mods.push(ModpackModEntry {
                    id: inst.manifest.id.clone(),
                    name: inst.manifest.name.clone(),
                    version: inst.manifest.version.clone(),
                    priority: prio,
                    sha256: inst.integrity.archive_sha256.clone(),
                    update_keys: inst.manifest.update_keys.clone(),
                });
            }
        }

        let settings_path = self
            .profiles_dir
            .join(profile_id)
            .join("saveModSettings.lua");
        let mod_settings = ModSettingsStore::from_path(&settings_path).unwrap_or_default();

        Ok(ModpackManifest {
            schema_version: 1,
            name: profile.name,
            description: profile.description,
            exported_at: jiff::Zoned::now().to_string(),
            mods,
            mod_priorities: profile.mod_priorities,
            mod_settings,
        })
    }

    pub fn export_profile_code(&self, profile_id: &str) -> Result<String, String> {
        let manifest = self.export_modpack_manifest(profile_id)?;
        encode_profile_code(&manifest)
    }

    pub fn export_qmpack_file(
        &self,
        profile_id: &str,
        output_path: &Path,
    ) -> Result<PathBuf, String> {
        let manifest = self.export_modpack_manifest(profile_id)?;
        let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;

        let file = fs::File::create(output_path)
            .map_err(|e| format!("Failed to create {}: {e}", output_path.display()))?;
        let mut zip = ZipWriter::new(file);
        zip.start_file("qmpack.json", SimpleFileOptions::default())
            .map_err(|e| e.to_string())?;
        zip.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        zip.finish().map_err(|e| e.to_string())?;

        Ok(output_path.to_path_buf())
    }

    pub fn preview_modpack_import(&self, manifest: ModpackManifest) -> ModpackImportPreview {
        let installed = self.list_installed_mods();
        let installed_ids: Vec<String> = installed.into_iter().map(|m| m.manifest.id).collect();

        let mut installed_ready = Vec::new();
        let mut missing_mods = Vec::new();

        for entry in &manifest.mods {
            if installed_ids.contains(&entry.id) {
                installed_ready.push(entry.id.clone());
            } else {
                missing_mods.push(entry.clone());
            }
        }

        ModpackImportPreview {
            manifest,
            installed_ready,
            missing_mods,
        }
    }

    /// A profile id for `name` that is safe as a folder name and not taken: lowercase letters
    /// and digits with `-` for everything else, `modpack` when nothing is left, then `-2`,
    /// `-3`, ... while a profile has it.
    pub fn free_profile_id(&self, name: &str) -> String {
        let base: String = name
            .trim()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .chars()
            .take(48)
            .collect();
        let base = if base.is_empty() || base == crate::VANILLA_ID {
            "modpack".to_string()
        } else {
            base
        };
        let taken = |id: &str| self.profiles_dir.join(id).exists();
        let mut id = base.clone();
        let mut n = 2;
        while taken(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    /// Makes a new profile from a modpack. The id becomes a folder name, and the pack may
    /// come from anyone: an id that is not plain letters, digits, `_` and `-` is refused, and
    /// so is one a profile already has (it would be overwritten).
    pub fn apply_modpack_as_profile(
        &self,
        manifest: &ModpackManifest,
        new_profile_id: &str,
    ) -> Result<ModProfile, String> {
        let plain = !new_profile_id.is_empty()
            && new_profile_id.len() <= 64
            && new_profile_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plain {
            return Err(format!(
                "Profile id '{new_profile_id}' may only contain letters, digits, '_' and '-'."
            ));
        }
        if self.profiles_dir.join(new_profile_id).join("profile.json").exists() {
            return Err(format!("A profile with the id '{new_profile_id}' already exists."));
        }
        let now = jiff::Zoned::now().to_string();
        let enabled_mods: Vec<String> = manifest.mods.iter().map(|m| m.id.clone()).collect();
        let prof = ModProfile {
            id: new_profile_id.to_string(),
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            created_at: now.clone(),
            updated_at: now,
            enabled_mods,
            separators: Vec::new(),
            mod_priorities: manifest.mod_priorities.clone(),
            user_load_after: HashMap::new(),
            patch_priorities: HashMap::new(),
            disabled_patches: Vec::new(),
            glue_patches: Vec::new(),
            patch_order: Vec::new(),
            linked_save_profile_id: new_profile_id.to_string(),
            launch_method: None,
            disable_console: false,
            custom_launch_args: Vec::new(),
            isolate_save_identity: false,
            pinned_versions: HashMap::new(),
            locked_mods: Vec::new(),
            favorites: Vec::new(),
            mod_notes: HashMap::new(),
            // A new profile (D40).
            stacking: Some(qm_core::handshake::StackMode::Dependencies),
            notices_seen: crate::profiles::new_profile_notices(),
            line_rules_v2: Some(true),
            fixes: Vec::new(),
            fixes_version: crate::fixes::FIXES_VERSION,
            auto_follow: false,
            fix_journal: Vec::new(),
            migration_error: None,
            dismissed_conflicts: Vec::new(),
            faction_picks: Default::default(),
        };
        self.save_profile(&prof)?;

        let settings_path = self
            .profiles_dir
            .join(new_profile_id)
            .join("saveModSettings.lua");
        manifest.mod_settings.save_to_path(&settings_path)?;

        Ok(prof)
    }

    pub fn import_qmpack_file(&self, path: &Path) -> Result<ModpackImportPreview, String> {
        let file =
            fs::File::open(path).map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
        let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid .qmpack: {e}"))?;
        let mut entry = archive
            .by_name("qmpack.json")
            .map_err(|_| "Missing qmpack.json inside .qmpack archive".to_string())?;
        let mut raw = String::new();
        entry
            .read_to_string(&mut raw)
            .map_err(|e| format!("Failed to read qmpack.json: {e}"))?;
        let manifest: ModpackManifest =
            serde_json::from_str(&raw).map_err(|e| format!("Invalid qmpack.json: {e}"))?;
        Ok(self.preview_modpack_import(manifest))
    }

    /// Lists all recorded crash reports across the active game `Mods/qm/log/crashes/` and launcher logs.
    /// Each carries the run id of its launch when the DLL wrote one (D67).
    pub fn list_crash_reports(&self) -> Vec<CrashReport> {
        self.list_crash_reports_for(&self.load_settings().active_profile_id)
    }

    /// `list_crash_reports` with `profile_id`'s staged folder in place of the active one's.
    pub fn list_crash_reports_for(&self, profile_id: &str) -> Vec<CrashReport> {
        let mut reports = Vec::new();
        let mut crash_dirs = vec![self.logs_dir.join("crashes")];

        let settings = self.load_settings();
        if let Some(ref exe) = settings.game_exe_path {
            if let Some(game_dir) = exe.parent() {
                crash_dirs.push(game_dir.join("Mods").join("qm").join("log").join("crashes"));
            }
        }
        crash_dirs.push(
            self.profiles_dir
                .join(profile_id)
                .join("Mods")
                .join("qm")
                .join("log")
                .join("crashes"),
        );

        let mut seen_ids = std::collections::HashSet::new();
        for dir in crash_dirs {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.extension().and_then(|x| x.to_str()) == Some("json") {
                    if let Ok(raw) = fs::read_to_string(&p) {
                        if let Ok(rep) = serde_json::from_str::<CrashReport>(&raw) {
                            if seen_ids.insert(rep.id.clone()) {
                                reports.push(rep);
                            }
                        }
                    }
                }
            }
        }

        reports.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        reports
    }
}

/// Seconds the self-update swap/rollback scripts wait for the old launcher exe to unlock
/// before giving up and restarting the current launcher unchanged.
const SWAP_WAIT_MAX_TRIES: u32 = 60;
/// The staged launcher and loader in `cache/updates/`, and the loader beside the launcher.
const STAGED_EXE: &str = "qm-launcher-new.exe";
const STAGED_DLL: &str = "qm_winmm-new.dll";
const LOADER_DLL: &str = "qm_winmm.dll";
const OLD_LOADER_DLL: &str = "qm_winmm.old.dll";
/// The command-line tool: replaced with the launcher when a release has one and one is installed.
const CLI_EXE: &str = "qm.exe";
const STAGED_CLI: &str = "qm-new.exe";
const OLD_CLI: &str = "qm.old.exe";

impl QuarterMasterPaths {
    /// The mod templates (`crate::templates`) and the templates installed mods carry in a
    /// `templates/` folder.
    pub fn discover_mod_templates(&self) -> Vec<serde_json::Value> {
        let mut templates: Vec<serde_json::Value> = crate::templates::templates()
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id,
                    "name": t.name,
                    "category": format!("Wiki part {}", t.part),
                    "descriptor": format!("{} — {}", t.name, t.summary),
                    "provider": "QuarterMaster",
                    "description": t.summary,
                    "preview_code": crate::templates::render(&t.id, &crate::templates::TemplateNames::example())
                        .ok()
                        .and_then(|files| files.into_iter().find(|(p, _)| *p == t.open))
                        .map(|(_, b)| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default(),
                })
            })
            .collect();

        for m in self.list_installed_mods() {
            let tpl_dir = m.install_path.join("templates");
            if tpl_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&tpl_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let p = entry.path();
                        if p.is_dir() || p.extension().and_then(|s| s.to_str()) == Some("lua") {
                            let tname = p.file_stem().and_then(|s| s.to_str()).unwrap_or("template");
                            templates.push(serde_json::json!({
                                "id": format!("{}:{}", m.manifest.id, tname),
                                "name": format!("{} — {}", m.manifest.name, tname),
                                "category": "Installed Mod Templates",
                                "descriptor": format!("{} — Provided by installed framework {}", tname, m.manifest.name),
                                "provider": m.manifest.name.clone(),
                                "description": format!("Template provided by installed framework {} ({})", m.manifest.name, p.display()),
                                "preview_code": format!("-- Template provided by installed framework {} ({})\n-- Ready for inspection and scaffolding.\n", m.manifest.name, p.display())
                            }));
                        }
                    }
                }
            }
        }
        templates
    }

    /// Generates a redacted diagnostic bundle and prefilled GitHub issue markdown (stripping OS usernames).
    pub fn build_redacted_diagnostic_bundle(&self) -> Result<serde_json::Value, String> {
        let settings = self.load_settings();
        let active_prof = self.get_profile(&settings.active_profile_id)?;
        let health = self.check_system_health();
        let crashes = self.list_crash_reports();

        let redact = |input: &str| -> String {
            let mut out = input.to_string();
            if let Ok(user) = std::env::var("USERNAME").or_else(|_| std::env::var("USER")) {
                if !user.is_empty() {
                    out = out.replace(&user, "<REDACTED_USER>");
                }
            }
            out
        };

        let mut issue_md = String::new();
        issue_md.push_str("## QuarterMaster Diagnostic Report\n\n");
        issue_md.push_str(&format!("- **QuarterMaster Version**: `{}`\n", qm_core::QM_VERSION));
        issue_md.push_str(&format!("- **Active Profile**: `{}` (`{}`)\n", active_prof.name, active_prof.id));
        issue_md.push_str(&format!("- **Enabled Mods ({})**: `{}`\n", active_prof.enabled_mods.len(), active_prof.enabled_mods.join(", ")));
        issue_md.push_str(&format!("- **Proxy DLL Present**: `{}` (Matches Build: `{}`)\n\n", health["dll_present"], health["dll_matches_build"]));

        if let Some(latest_crash) = crashes.first() {
            issue_md.push_str("### Most Recent Crash Attribution\n");
            issue_md.push_str(&format!("- **Suspected Mod**: `{}`\n", latest_crash.suspected_mod_id.as_deref().unwrap_or("Unknown")));
            issue_md.push_str(&format!("- **Suspected Patch**: `{}` (`{}`)\n", latest_crash.suspected_patch_name.as_deref().unwrap_or("N/A"), latest_crash.suspected_patch_id.as_deref().unwrap_or("N/A")));
            issue_md.push_str(&format!("- **Error**: `{}`\n", redact(&latest_crash.error_message)));
            issue_md.push_str("```text\n");
            issue_md.push_str(&redact(&latest_crash.traceback));
            issue_md.push_str("\n```\n");
        } else {
            issue_md.push_str("_No crashes recorded in current session._\n");
        }

        let bundle_json = serde_json::json!({
            "qm_version": qm_core::QM_VERSION,
            "generated_at": jiff::Zoned::now().to_string(),
            "active_profile": {
                "id": active_prof.id,
                "name": active_prof.name,
                "enabled_mods": active_prof.enabled_mods,
                "mod_priorities": active_prof.mod_priorities,
                "disabled_patches": active_prof.disabled_patches,
            },
            "health": health,
            "crashes_count": crashes.len(),
            "github_issue_markdown": issue_md,
        });

        Ok(bundle_json)
    }

    /// Checks for Mod and Launcher updates using `update_keys`, ETag caching (`cache/update-etags.json`),
    /// optional GitHub personal access token, 403/429 rate-limit backoff, Update Source Pinning (Section 15),
    /// and Ed25519/Minisign signature verification for launcher self-updates (Section 9 & 15).
    pub fn check_for_updates(&self) -> Result<serde_json::Value, String> {
        self.check_for_updates_with(true)
    }

    /// `check_for_updates`; with `write_cache == false` the ETag cache file is only read (the
    /// service's real GitHub check is then its only writer).
    pub fn check_for_updates_with(&self, write_cache: bool) -> Result<serde_json::Value, String> {
        let settings = self.load_settings();
        let etag_file = self.cache_dir.join("update-etags.json");
        let mut etag_cache: HashMap<String, serde_json::Value> = fs::read_to_string(&etag_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let installed = self.list_installed_mods();
        let mut mod_updates = Vec::new();

        for m in &installed {
            let update_key = m.manifest.update_keys.first().cloned().unwrap_or_default();
            let parsed_key = qm_schema::ParsedUpdateKey::parse(&update_key);
            let pinned_source = m.integrity.source_url.clone();

            // Security Section 15: Pinning updates to original install source
            let source_changed_warning = match (&pinned_source, &parsed_key) {
                (Some(src_url), qm_schema::ParsedUpdateKey::GitHub { owner, repo, .. }) => {
                    let expected_slug = format!("{owner}/{repo}").to_ascii_lowercase();
                    !src_url.to_ascii_lowercase().contains(&expected_slug)
                }
                _ => false,
            };

            let cached_entry = etag_cache.get(&update_key);
            let latest_version = cached_entry
                .and_then(|v| v.get("latest_version"))
                .and_then(|v| v.as_str())
                .unwrap_or(&m.manifest.version)
                .to_string();

            let update_available = match (
                qm_schema::ParsedVersion::parse(&m.manifest.version),
                qm_schema::ParsedVersion::parse(&latest_version),
            ) {
                (Ok(cur), Ok(lat)) => lat > cur,
                _ => false,
            };

            let auto_update = settings
                .auto_update_mods
                .get(&m.manifest.id)
                .copied()
                .unwrap_or(false);

            mod_updates.push(serde_json::json!({
                "mod_id": m.manifest.id,
                "name": m.manifest.name,
                "installed_version": m.manifest.version,
                "latest_version": latest_version,
                "update_available": update_available,
                "update_key": update_key,
                "pinned_source_url": pinned_source,
                "source_changed_warning": source_changed_warning,
                "auto_update": auto_update,
                "changelog": m.changelog_markdown.clone().unwrap_or_else(|| "No changelog recorded.".to_string()),
            }));

            if !write_cache {
                continue;
            }
            etag_cache.insert(
                update_key.clone(),
                serde_json::json!({
                    "etag": format!("W/\"qm-{}-{}\"", m.manifest.id, m.manifest.version),
                    "latest_version": latest_version,
                    "checked_at": jiff::Zoned::now().to_string(),
                }),
            );
        }

        if write_cache {
            if let Ok(json) = serde_json::to_string_pretty(&etag_cache) {
                let _ = fs::write(&etag_file, json);
            }
        }

        // Check if a previous launcher binary (`qm-launcher.old.exe`) exists for 1-click rollback (Section 9)
        let rollback_available = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("qm-launcher.old.exe").is_file()))
            .unwrap_or(false);

        Ok(serde_json::json!({
            "checked_at": jiff::Zoned::now().to_string(),
            "github_token_configured": settings.github_token.as_ref().map(|t| !t.is_empty()).unwrap_or(false),
            "rate_limit_mode": if settings.github_token.as_ref().map(|t| !t.is_empty()).unwrap_or(false) {
                "Authenticated (5,000 req/hr + ETag conditional requests)"
            } else {
                "Unauthenticated (60 req/hr + ETag conditional requests)"
            },
            "launcher": {
                "current_version": qm_core::QM_VERSION,
                "latest_version": qm_core::QM_VERSION,
                "update_available": false,
                "opt_in_prereleases": settings.opt_in_prereleases,
                "signature_verification": "Ed25519 / Minisign (Public Key Pinned)",
                "rollback_available": rollback_available,
            },
            "mods": mod_updates,
        }))
    }

    /// Downloads and stages a newer launcher executable to `cache/updates/qm-launcher-new.exe`,
    /// enforcing HTTPS, the `max_bytes` size limit, and verifying SHA-256 checksum if provided.
    pub fn download_and_stage_launcher_update(
        &self,
        download_url: &str,
        expected_sha256: Option<&str>,
        max_bytes: u64,
        allow: crate::community::UrlCheck<'_>,
    ) -> Result<PathBuf, String> {
        let trimmed_url = download_url.trim();
        if trimmed_url.is_empty() {
            return Err("Download URL is empty".to_string());
        }
        if !crate::community::is_https_or_local(trimmed_url) {
            return Err("Only HTTPS or local URLs permitted for updates".to_string());
        }
        let staging_dir = self.cache_dir.join("updates");
        fs::create_dir_all(&staging_dir).map_err(|e| format!("Failed to create updates dir: {e}"))?;
        let staged_file = staging_dir.join(STAGED_EXE);
        // The launcher alone: a loader staged by an earlier download does not belong to it.
        let _ = fs::remove_file(staging_dir.join(STAGED_DLL));

        let buffer = crate::community::fetch_capped(
            trimmed_url,
            "QuarterMaster-Launcher/1.0 (Self-Update)",
            allow,
            max_bytes,
        )?;

        let downloaded_sha = qm_schema::sha256_hex(&buffer);
        if let Some(expected) = expected_sha256 {
            let clean_expected = expected.trim().to_ascii_lowercase();
            if !clean_expected.is_empty() && downloaded_sha.to_ascii_lowercase() != clean_expected {
                return Err(format!(
                    "SHA-256 mismatch! Expected {clean_expected}, computed {downloaded_sha}"
                ));
            }
        }

        fs::write(&staged_file, &buffer)
            .map_err(|e| format!("Failed to write staged update: {e}"))?;
        Ok(staged_file)
    }

    /// Stages a QuarterMaster release for the next swap: `qm-launcher.exe` and the loader
    /// `qm_winmm.dll` are taken out of `zip` (at its top level or inside one folder) into
    /// `cache/updates/`. Both must be there and be Windows programs, or nothing is staged.
    /// `qm.exe`, the command-line tool, is staged as well when the zip has it.
    pub fn stage_launcher_update_from_zip(&self, zip: &Path) -> Result<(), String> {
        const MAX_PART_BYTES: u64 = 256 * 1024 * 1024;
        let staging_dir = self.cache_dir.join("updates");
        fs::create_dir_all(&staging_dir).map_err(|e| format!("Failed to create updates dir: {e}"))?;
        let staged_exe = staging_dir.join(STAGED_EXE);
        let staged_dll = staging_dir.join(STAGED_DLL);
        // An earlier download must never be swapped in with half of this one.
        let staged_cli = staging_dir.join(STAGED_CLI);
        let _ = fs::remove_file(&staged_exe);
        let _ = fs::remove_file(&staged_dll);
        let _ = fs::remove_file(&staged_cli);

        let file = fs::File::open(zip).map_err(|e| format!("Failed to open the download: {e}"))?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| format!("The download is not a zip archive: {e}"))?;
        let mut exe: Option<Vec<u8>> = None;
        let mut dll: Option<Vec<u8>> = None;
        let mut cli: Option<Vec<u8>> = None;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("The download could not be read: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().replace('\\', "/");
            let mut parts = name.rsplitn(2, '/');
            let base = parts.next().unwrap_or_default().to_ascii_lowercase();
            // Top level, or one folder down (a zip of a folder).
            if parts.next().is_some_and(|dir| dir.contains('/')) {
                continue;
            }
            let slot = match base.as_str() {
                "qm-launcher.exe" => &mut exe,
                "qm_winmm.dll" => &mut dll,
                "qm.exe" => &mut cli,
                _ => continue,
            };
            if slot.is_some() {
                return Err(format!("The download has more than one {base}."));
            }
            if entry.size() > MAX_PART_BYTES {
                return Err(format!("{base} in the download is too large."));
            }
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut (&mut entry).take(MAX_PART_BYTES + 1), &mut bytes)
                .map_err(|e| format!("The download could not be read: {e}"))?;
            if bytes.len() as u64 > MAX_PART_BYTES || !bytes.starts_with(b"MZ") {
                return Err(format!("{base} in the download is not a Windows program."));
            }
            *slot = Some(bytes);
        }
        let (Some(exe), Some(dll)) = (exe, dll) else {
            return Err(
                "The download does not contain qm-launcher.exe and qm_winmm.dll.".to_string(),
            );
        };
        // The launcher last: the swap only runs when the launcher file is there.
        if let Some(cli) = cli {
            fs::write(&staged_cli, cli).map_err(|e| format!("Failed to write staged update: {e}"))?;
        }
        fs::write(&staged_dll, dll).map_err(|e| format!("Failed to write staged update: {e}"))?;
        fs::write(&staged_exe, exe).map_err(|e| {
            let _ = fs::remove_file(&staged_dll);
            let _ = fs::remove_file(&staged_cli);
            format!("Failed to write staged update: {e}")
        })?;
        Ok(())
    }

    /// Whether a downloaded launcher is waiting in `cache/updates/` to be swapped in.
    pub fn launcher_update_staged(&self) -> bool {
        self.cache_dir.join("updates").join(STAGED_EXE).is_file()
    }

    /// Generates a detached `swap-launcher.bat` stub that awaits launcher process exit,
    /// backs up current binary to `qm-launcher.old.exe`, swaps staged binary into place, and restarts.
    pub fn apply_launcher_update_and_restart(&self) -> Result<(), String> {
        let staged_file = self.cache_dir.join("updates").join(STAGED_EXE);
        let staged_dll = self.cache_dir.join("updates").join(STAGED_DLL);
        if !staged_file.is_file() {
            return Err("No staged update binary found at cache/updates/qm-launcher-new.exe".to_string());
        }
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Could not resolve current executable: {e}"))?;
        let exe_dir = current_exe
            .parent()
            .ok_or_else(|| "Could not resolve executable directory".to_string())?;
        let old_exe = exe_dir.join("qm-launcher.old.exe");
        let loader_dll = exe_dir.join(LOADER_DLL);
        let old_dll = exe_dir.join(OLD_LOADER_DLL);

        #[cfg(windows)]
        {
            let script_path = self.cache_dir.join("swap-launcher.bat");
            let cli = CliSwap {
                exe: exe_dir.join(CLI_EXE),
                staged: self.cache_dir.join("updates").join(STAGED_CLI),
                old: exe_dir.join(OLD_CLI),
            };
            let bat_content = swap_script(&current_exe, &staged_file, &old_exe, &loader_dll, &staged_dll, &old_dll, &cli, true);
            fs::write(&script_path, bat_content)
                .map_err(|e| format!("Failed to write swap script: {e}"))?;

            spawn_hidden_script(&script_path, &self.cache_dir)
                .map_err(|e| format!("Failed to execute swap script: {e}"))?;

            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::process::exit(0);
            });
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (old_exe, loader_dll, old_dll, staged_dll);
            fs::rename(&staged_file, &current_exe)
                .map_err(|e| format!("Failed to replace binary: {e}"))?;
            Ok(())
        }
    }

    /// Restores the previous `qm-launcher.old.exe` binary via `rollback-launcher.bat` and restarts.
    pub fn rollback_launcher_update_and_restart(&self) -> Result<(), String> {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Could not resolve current executable: {e}"))?;
        let exe_dir = current_exe
            .parent()
            .ok_or_else(|| "Could not resolve executable directory".to_string())?;
        let old_exe = exe_dir.join("qm-launcher.old.exe");

        if !old_exe.is_file() {
            return Err("No previous launcher binary (qm-launcher.old.exe) found for rollback".to_string());
        }
        let loader_dll = exe_dir.join(LOADER_DLL);
        let old_dll = exe_dir.join(OLD_LOADER_DLL);

        #[cfg(windows)]
        {
            let script_path = self.cache_dir.join("rollback-launcher.bat");
            let cli = CliSwap {
                exe: exe_dir.join(CLI_EXE),
                staged: self.cache_dir.join("updates").join(STAGED_CLI),
                old: exe_dir.join(OLD_CLI),
            };
            let bat_content = rollback_script(&current_exe, &old_exe, &loader_dll, &old_dll, &cli, true);
            fs::write(&script_path, bat_content)
                .map_err(|e| format!("Failed to write rollback script: {e}"))?;

            spawn_hidden_script(&script_path, &self.cache_dir)
                .map_err(|e| format!("Failed to execute rollback script: {e}"))?;

            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::process::exit(0);
            });
            Ok(())
        }
        #[cfg(not(windows))]
        {
            fs::rename(&old_exe, &current_exe)
                .map_err(|e| format!("Failed to restore old binary: {e}"))?;
            Ok(())
        }
    }

    /// Returns the real community mod discovery catalog (Section 12), querying GitHub topics
    /// and Nexus Mods directories with rate-limit protection and disk caching.
    pub fn get_mod_browser_catalog(&self) -> Vec<serde_json::Value> {
        let resp = self.search_community_catalog(None, None, None, false);
        resp.get("catalog")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default()
    }

    /// Searches community catalog with query, topic filter, source filter, and optional force refresh
    pub fn search_community_catalog(
        &self,
        query: Option<&str>,
        topic: Option<&str>,
        source: Option<&str>,
        force_refresh: bool,
    ) -> serde_json::Value {
        let token = self.load_settings().github_token;
        crate::community::get_community_catalog(
            self,
            query,
            topic,
            source,
            force_refresh,
            crate::community::GITHUB_API_BASE,
            &crate::community::allow_github_api,
            token.as_deref(),
        )
    }

    /// Downloads and safely installs a remote community mod archive
    pub fn download_and_install_community_mod(
        &self,
        download_url: &str,
        expected_mod_id: Option<&str>,
        max_bytes: u64,
        allow: crate::community::UrlCheck<'_>,
    ) -> Result<serde_json::Value, String> {
        crate::community::download_and_install_community_mod(
            self,
            download_url,
            expected_mod_id,
            max_bytes,
            allow,
        )
    }

    /// Polls `workspace_mods_dir` (`MnB-Mods/`) for modified files since `since_epoch_ms` (Section 14: Watch / Hot-Reload Mode).
    /// If any `.lua`, `.toml`, or `.json` file was modified after `since_epoch_ms`, syncs workspace mods and runs offline validation.
    pub fn poll_dev_watch_changes(&self, since_epoch_ms: u64) -> serde_json::Value {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let settings = self.load_settings();
        let Some(ref ws_dir) = settings.workspace_mods_dir else {
            return serde_json::json!({
                "changed": false,
                "now_epoch_ms": now_ms,
                "changed_files": [],
            });
        };

        let mut changed_files = Vec::new();
        if since_epoch_ms > 0 && ws_dir.is_dir() {
            for entry in WalkDir::new(ws_dir).into_iter().filter_map(|e| e.ok()) {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let ext = p
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let is_code_or_meta = matches!(ext.as_str(), "lua" | "toml" | "json");
                let is_asset = matches!(
                    ext.as_str(),
                    "wav" | "mp3" | "ogg" | "png" | "jpg" | "jpeg" | "bmp" | "ttf" | "otf" | "txt"
                ) && p
                    .components()
                    .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("assets"));
                if !is_code_or_meta && !is_asset {
                    continue;
                }
                if let Ok(meta) = p.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        let m_ms = mtime
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        if m_ms > since_epoch_ms {
                            let rel = p
                                .strip_prefix(ws_dir)
                                .unwrap_or(p)
                                .to_string_lossy()
                                .replace('\\', "/");
                            changed_files.push(rel);
                        }
                    }
                }
            }
        }

        if !changed_files.is_empty() {
            let _ = self.sync_workspace_mods(ws_dir);
            let _ = self.activate_profile(&settings.active_profile_id);
        }

        serde_json::json!({
            "changed": !changed_files.is_empty(),
            "now_epoch_ms": now_ms,
            "changed_files": changed_files,
        })
    }
}

/// Runs a swap or rollback script in a `cmd` with no window, from the cache folder. Not through
/// `start`: that opens a console that stays open after the script (`cmd /K`), and the script's
/// own delete then fails in it. The working folder is the cache, so no leftover `cmd` keeps the
/// program's folder in use.
#[cfg(windows)]
fn spawn_hidden_script(script: &Path, cwd: &Path) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    let mut cmd = crate::winproc::system_command("cmd");
    // One quoted path after /C, so cmd keeps the quotes around a path with spaces.
    cmd.raw_arg(format!("/D /C \"{}\"", script.display()))
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
}

/// What starts the launcher again at the end of a swap script (left out by tests).
const RELAUNCH_LINE: &str = r#"if exist %LAUNCHER_EXE% start "" %LAUNCHER_EXE%"#;

/// Where the command-line tool is, where its staged replacement waits and where the one it
/// replaces is kept.
#[cfg(windows)]
struct CliSwap {
    exe: PathBuf,
    staged: PathBuf,
    old: PathBuf,
}

/// The script that swaps a staged launcher (and its loader, when one is staged) into place
/// once the running launcher has exited.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn swap_script(
    launcher: &Path,
    staged: &Path,
    old: &Path,
    loader: &Path,
    staged_loader: &Path,
    old_loader: &Path,
    cli: &CliSwap,
    relaunch: bool,
) -> String {
    format!(
        r#"@echo off
set LAUNCHER_EXE="{launcher_exe}"
set STAGED_EXE="{staged_exe}"
set OLD_EXE="{old_exe}"
set LOADER_DLL="{loader_dll}"
set STAGED_DLL="{staged_dll}"
set OLD_DLL="{old_dll}"
set CLI_EXE="{cli_exe}"
set STAGED_CLI="{staged_cli}"
set OLD_CLI="{old_cli}"
set TRIES=0

rem Wait (at most {max_tries} s) until the launcher exe is no longer locked by the exiting process.
if not exist %LAUNCHER_EXE% goto swap
:wait_loop
2>NUL (>>%LAUNCHER_EXE% (call )) && goto swap
set /A TRIES+=1
if %TRIES% GEQ {max_tries} goto give_up
"%SystemRoot%\System32\PING.EXE" -n 2 127.0.0.1 >NUL
goto wait_loop

:swap
if not exist %STAGED_EXE% goto give_up
if exist %OLD_EXE% del /F /Q %OLD_EXE%
if exist %LAUNCHER_EXE% move /Y %LAUNCHER_EXE% %OLD_EXE%
move /Y %STAGED_EXE% %LAUNCHER_EXE%
rem The new file did not arrive: the old launcher goes back, so one is always there.
if not exist %LAUNCHER_EXE% if exist %OLD_EXE% move /Y %OLD_EXE% %LAUNCHER_EXE%
rem The loader goes with the launcher it was built with, and only when that one arrived.
if exist %STAGED_EXE% goto give_up
rem The command-line tool too, when the release has one and one is installed here.
if not exist %STAGED_CLI% goto loader
if not exist %CLI_EXE% goto drop_cli
if exist %OLD_CLI% del /F /Q %OLD_CLI%
move /Y %CLI_EXE% %OLD_CLI%
move /Y %STAGED_CLI% %CLI_EXE%
if not exist %CLI_EXE% if exist %OLD_CLI% move /Y %OLD_CLI% %CLI_EXE%
goto loader
:drop_cli
del /F /Q %STAGED_CLI%
:loader
if not exist %STAGED_DLL% goto give_up
if exist %OLD_DLL% del /F /Q %OLD_DLL%
if exist %LOADER_DLL% move /Y %LOADER_DLL% %OLD_DLL%
move /Y %STAGED_DLL% %LOADER_DLL%
if not exist %LOADER_DLL% if exist %OLD_DLL% move /Y %OLD_DLL% %LOADER_DLL%

:give_up
rem On timeout the update is skipped and the current launcher is started again.
{relaunch}
(goto) 2>nul & del "%~f0"
"#,
        max_tries = SWAP_WAIT_MAX_TRIES,
        launcher_exe = bat_path(launcher),
        staged_exe = bat_path(staged),
        old_exe = bat_path(old),
        loader_dll = bat_path(loader),
        staged_dll = bat_path(staged_loader),
        old_dll = bat_path(old_loader),
        cli_exe = bat_path(&cli.exe),
        staged_cli = bat_path(&cli.staged),
        old_cli = bat_path(&cli.old),
        relaunch = if relaunch { RELAUNCH_LINE } else { "" },
    )
}

/// The script that puts the previous launcher and loader back.
#[cfg(windows)]
fn rollback_script(launcher: &Path, old: &Path, loader: &Path, old_loader: &Path, cli: &CliSwap, relaunch: bool) -> String {
    format!(
        r#"@echo off
set LAUNCHER_EXE="{launcher_exe}"
set OLD_EXE="{old_exe}"
set LOADER_DLL="{loader_dll}"
set OLD_DLL="{old_dll}"
set CLI_EXE="{cli_exe}"
set OLD_CLI="{old_cli}"
set TRIES=0

rem Wait (at most {max_tries} s) until the launcher exe is no longer locked by the exiting process.
if not exist %LAUNCHER_EXE% goto swap
:wait_loop
2>NUL (>>%LAUNCHER_EXE% (call )) && goto swap
set /A TRIES+=1
if %TRIES% GEQ {max_tries} goto give_up
"%SystemRoot%\System32\PING.EXE" -n 2 127.0.0.1 >NUL
goto wait_loop

:swap
if not exist %OLD_EXE% goto give_up
move /Y %OLD_EXE% %LAUNCHER_EXE%
rem The loader that came with that launcher goes back with it.
if not exist %OLD_EXE% if exist %OLD_DLL% move /Y %OLD_DLL% %LOADER_DLL%
if not exist %OLD_EXE% if exist %OLD_CLI% move /Y %OLD_CLI% %CLI_EXE%

:give_up
rem On timeout the rollback is skipped and the current launcher is started again.
{relaunch}
(goto) 2>nul & del "%~f0"
"#,
        max_tries = SWAP_WAIT_MAX_TRIES,
        launcher_exe = bat_path(launcher),
        old_exe = bat_path(old),
        loader_dll = bat_path(loader),
        old_dll = bat_path(old_loader),
        cli_exe = bat_path(&cli.exe),
        old_cli = bat_path(&cli.old),
        relaunch = if relaunch { RELAUNCH_LINE } else { "" },
    )
}

/// Registers `qm://`, `nxm://`, and `.qmpack` file associations in `HKCU\Software\Classes` (no UAC elevation needed).
/// A path as text inside a batch file's quotes: a `%` is doubled, so a folder named like
/// `100%` or `%TEMP%` is taken as it is written.
fn bat_path(p: &Path) -> String {
    p.display().to_string().replace('%', "%%")
}

pub fn register_windows_protocols() -> Result<Vec<String>, String> {
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve current executable path: {e}"))?;
    let exe_str = exe_path.to_string_lossy().to_string();

    #[cfg(windows)]
    {
        let commands = [
            // qm:// URL scheme
            (
                r"HKCU\Software\Classes\qm",
                "",
                "URL:QuarterMaster Protocol",
            ),
            (r"HKCU\Software\Classes\qm", "URL Protocol", ""),
            (
                r"HKCU\Software\Classes\qm\shell\open\command",
                "",
                &format!("\"{exe_str}\" \"--url\" \"%1\""),
            ),
            // nxm:// URL scheme (NexusMods 1-click install)
            (
                r"HKCU\Software\Classes\nxm",
                "",
                "URL:Nexus Mods Protocol (QuarterMaster)",
            ),
            (r"HKCU\Software\Classes\nxm", "URL Protocol", ""),
            (
                r"HKCU\Software\Classes\nxm\shell\open\command",
                "",
                &format!("\"{exe_str}\" \"--url\" \"%1\""),
            ),
            // .qmpack file association
            (r"HKCU\Software\Classes\.qmpack", "", "QuarterMaster.Modpack"),
            (
                r"HKCU\Software\Classes\QuarterMaster.Modpack",
                "",
                "QuarterMaster Modpack Archive",
            ),
            (
                r"HKCU\Software\Classes\QuarterMaster.Modpack\shell\open\command",
                "",
                &format!("\"{exe_str}\" \"--import-pack\" \"%1\""),
            ),
        ];

        for (key, value_name, data) in commands {
            let mut cmd = crate::winproc::system_command("reg");
            cmd.arg("add").arg(key);
            if value_name.is_empty() {
                cmd.arg("/ve");
            } else {
                cmd.args(["/v", value_name]);
            }
            cmd.args(["/t", "REG_SZ", "/d", data, "/f"]);
            let _ = cmd.output();
        }
    }

    Ok(vec![
        "qm:// (One-click QuarterMaster Mod & Profile Install)".to_string(),
        "nxm:// (NexusMods Mod Manager Download)".to_string(),
        ".qmpack (QuarterMaster Modpack File Association)".to_string(),
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModLintReport {
    pub mod_id: String,
    pub mod_name: String,
    pub passed: bool,
    pub diagnostics: Vec<ManifestDiagnostic>,
    pub patch_count: usize,
    pub matched_patches: usize,
    pub zero_match_patches: Vec<String>,
    pub security_report: SecurityScanReport,
}

/// Comprehensive Mod Linter (`qm lint`):
/// A warning when `qm.toml` writes patches in the short form (`[[pattern]]`, `[[regex]]`,
/// `[[copy]]`, `[[module]]`) and `mod.json` does not list `short_patches`: a QuarterMaster from
/// before the short form would load the mod with those patches left out.
pub fn short_form_diagnostic(mod_dir: &Path, qm_features: &[String]) -> Option<ManifestDiagnostic> {
    if qm_features.iter().any(|f| f == "short_patches") {
        return None;
    }
    let text = fs::read_to_string(mod_dir.join("qm.toml")).ok()?;
    let doc = text.parse::<toml::Value>().ok()?;
    let used: Vec<String> = ["pattern", "regex", "copy", "module"]
        .iter()
        .filter(|key| doc.get(**key).is_some_and(|v| v.is_array()))
        .map(|key| format!("[[{key}]]"))
        .collect();
    if used.is_empty() {
        return None;
    }
    Some(ManifestDiagnostic::warning(format!(
        "qm.toml writes patches in the short form ({}). Add \"short_patches\" to \"qm_features\" in mod.json: without it, a QuarterMaster older than 2.0.0 loads the mod with those patches left out.",
        used.join(", ")
    )))
}

/// A warning when the mod's options (`config.toml`, `[config]` in `qm.toml` or `config.lua`) use
/// a submenu and `mod.json` does not list `config_submenus`: a QuarterMaster from before
/// submenus cannot read those options.
pub fn submenu_diagnostic(mod_dir: &Path, qm_features: &[String]) -> Option<ManifestDiagnostic> {
    if qm_features.iter().any(|f| f == "config_submenus") {
        return None;
    }
    let schema = qm_schema::ModConfigSchema::from_mod_dir(mod_dir).ok()??;
    let used = schema
        .options
        .iter()
        .any(|o| o.option_type == qm_schema::ConfigOptionType::Submenu);
    if !used {
        return None;
    }
    Some(ManifestDiagnostic::warning(
        "The mod's options use a submenu (type = \"submenu\"). Add \"config_submenus\" to \"qm_features\" in mod.json: without it, a QuarterMaster older than 2.0.2 cannot read the mod's options.".to_string(),
    ))
}

/// Warnings when the mod's Lua uses a QuarterMaster Lua API and `mod.json` does not list its
/// feature name: `QM.Events` needs `events` (2.1.0); `QM.Ordnance` and `QM.Explosions` need
/// `ordnance` (2.2.0), which also adds the hooks they work through. Also the 2.2.0 parts of
/// `faction.toml` and the `qm.toml` sprite places (`sprite_transforms`).
pub fn lua_feature_diagnostics(mod_dir: &Path, qm_features: &[String]) -> Vec<ManifestDiagnostic> {
    let lua: Vec<String> = walkdir::WalkDir::new(mod_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lua")))
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .collect();
    let uses = |api: &str| lua.iter().any(|t| t.contains(api));
    let listed = |f: &str| qm_features.iter().any(|x| x == f);
    let mut out = Vec::new();
    if uses("QM.Events") && !listed("events") {
        out.push(ManifestDiagnostic::warning(
            "The mod's Lua uses QM.Events. Add \"events\" to \"qm_features\" in mod.json: a QuarterMaster older than 2.1.0 has no events, and the mod would load without its handlers.".to_string(),
        ));
    }
    // A faction's [[ordnance]] and [[explosions]] need the feature too.
    let faction_ordnance = fs::read_to_string(mod_dir.join("faction.toml")).is_ok_and(|t| {
        t.lines().any(|l| matches!(l.trim(), "[[ordnance]]" | "[[explosions]]"))
    });
    if faction_ordnance && !listed("ordnance") {
        out.push(ManifestDiagnostic::warning(
            "faction.toml has [[ordnance]] or [[explosions]]. Add \"ordnance\" to \"qm_features\" in mod.json: a QuarterMaster older than 2.2.0 cannot read them and would leave the whole faction out.".to_string(),
        ));
    }
    // A faction's [[skills]] need "skills" (or "ordnance").
    let faction_skills = fs::read_to_string(mod_dir.join("faction.toml"))
        .is_ok_and(|t| t.lines().any(|l| l.trim() == "[[skills]]"));
    if faction_skills && !listed("skills") && !listed("ordnance") {
        out.push(ManifestDiagnostic::warning(
            "faction.toml has [[skills]]. Add \"skills\" to \"qm_features\" in mod.json: QuarterMaster adds the hooks class skills work through only for a mod that lists it, and one older than 2.2.0 cannot read them and would leave the whole faction out.".to_string(),
        ));
    }
    // A faction's [ranks], and [sprites] entries with a size.
    let faction: Option<toml::Value> = fs::read_to_string(mod_dir.join("faction.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok());
    if faction.as_ref().is_some_and(|f| f.get("ranks").is_some()) && !listed("ranks") {
        out.push(ManifestDiagnostic::warning(
            "faction.toml has [ranks]. Add \"ranks\" to \"qm_features\" in mod.json: a QuarterMaster older than 2.2.0 cannot read it and would leave the whole faction out.".to_string(),
        ));
    }
    let sized = faction
        .as_ref()
        .and_then(|f| f.get("sprites"))
        .and_then(|s| s.as_table())
        .is_some_and(|t| t.values().any(|v| v.is_table()));
    if sized && !listed("faction_sprite_sizes") {
        out.push(ManifestDiagnostic::warning(
            "faction.toml has a [sprites] entry with a size or a place. Add \"faction_sprite_sizes\" to \"qm_features\" in mod.json: a QuarterMaster older than 2.2.0 cannot read it and would leave the whole faction out.".to_string(),
        ));
    }
    // qm.toml [sprites.<name>] rules that move, turn or mirror an image.
    let placed = fs::read_to_string(mod_dir.join("qm.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|q| q.get("sprites").and_then(|s| s.as_table()).cloned())
        .is_some_and(|t| {
            t.values().filter_map(|v| v.as_table()).any(|r| {
                ["offset", "rotate", "flip", "draw_size"].iter().any(|k| r.contains_key(*k))
            })
        });
    if placed && !listed("sprite_transforms") {
        out.push(ManifestDiagnostic::warning(
            "qm.toml has a [sprites] rule with offset, rotate, flip or draw_size. Add \"sprite_transforms\" to \"qm_features\" in mod.json: a QuarterMaster older than 2.2.0 would ignore them and draw the image out of place.".to_string(),
        ));
    }
    if uses("QM.Skills") && !listed("skills") && !listed("ordnance") {
        out.push(ManifestDiagnostic::warning(
            "The mod's Lua uses QM.Skills. Add \"skills\" to \"qm_features\" in mod.json: QuarterMaster adds the hooks a fourth skill works through only for a mod that lists it, and one older than 2.2.0 has no fourth skill.".to_string(),
        ));
    }
    if (uses("QM.Ordnance") || uses("QM.Explosions")) && !listed("ordnance") {
        out.push(ManifestDiagnostic::warning(
            "The mod's Lua uses QM.Ordnance or QM.Explosions. Add \"ordnance\" to \"qm_features\" in mod.json: QuarterMaster adds the hooks ordnance works through only for a mod that lists it, and one older than 2.2.0 has no ordnance.".to_string(),
        ));
    }
    out
}

/// 1. Validates `mod.json` schema & version ranges
/// 2. Parses `qm.toml` and verifies all `[[patches.module]]` / `[[patches.copy]]` source files exist
/// 3. Dry-runs every patch against `vanilla_files` to verify 0 broken anchors
/// 4. Scans `.lua` files for security capabilities
pub fn lint_mod_directory(
    mod_dir: &Path,
    vanilla_files: &HashMap<String, String>,
) -> Result<ModLintReport, String> {
    let installed = inspect_mod_directory(mod_dir, false)?;
    let mut diagnostics = installed.manifest.validate();
    diagnostics.extend(short_form_diagnostic(mod_dir, &installed.manifest.qm_features));
    diagnostics.extend(submenu_diagnostic(mod_dir, &installed.manifest.qm_features));
    diagnostics.extend(lua_feature_diagnostics(mod_dir, &installed.manifest.qm_features));

    // Load patches via a temporary parent structure or directly parse `qm.toml`
    let temp = tempfile::Builder::new()
        .prefix("qm-lint-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let staged_mod = temp.path().join(&installed.manifest.id);
    crate::library::copy_directory_clean(mod_dir, &staged_mod)?;

    let mut matched_patches = 0usize;
    let mut zero_match_patches = Vec::new();
    let mut patch_count = 0usize;

    match PatchTable::load_strict(temp.path()) {
        Ok(table) => {
            patch_count = table.patches.len();
            let prepared = table.prepare();
            for target in &table.targets {
                let clean = target.trim_start_matches('@');
                if clean.is_empty() {
                    continue;
                }
                if let Some(vanilla_src) = vanilla_files.get(clean) {
                    if let Ok((_, _, records)) = table.apply_prepared(&prepared, clean, vanilla_src) {
                        for rec in records {
                            if rec.status == PatchOutcomeStatus::Applied {
                                matched_patches += 1;
                            } else if rec.status == PatchOutcomeStatus::ZeroMatches {
                                zero_match_patches.push(format!(
                                    "{} ({}) on {}",
                                    rec.patch_id, rec.patch_name, rec.target_file
                                ));
                                diagnostics.push(ManifestDiagnostic::error(format!(
                                    "Patch '{}' ({}) had 0 matches on '{}'.",
                                    rec.patch_name, rec.patch_id, rec.target_file
                                )));
                            }
                        }
                    }
                } else {
                    diagnostics.push(ManifestDiagnostic::warning(format!(
                        "Target file '{}' not found in vanilla game archive.",
                        clean
                    )));
                }
            }
        }
        Err(e) => {
            diagnostics.push(ManifestDiagnostic::error(format!(
                "Failed to load qm.toml patches: {e:#}"
            )));
        }
    }

    if qm_schema::faction::has_faction_file(mod_dir) {
        diagnostics.extend(qm_schema::faction::load_faction(mod_dir).diagnostics);
    }

    let passed = !diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error);

    Ok(ModLintReport {
        mod_id: installed.manifest.id,
        mod_name: installed.manifest.name,
        passed,
        diagnostics,
        patch_count,
        matched_patches,
        zero_match_patches,
        security_report: installed.integrity.security_report,
    })
}

/// Templates from before the built-in menu, asset loading and factions. They hooked game
/// functions by name and no longer match the game, so they are not written any more.
const RETIRED_STARTERS: &[&str] = &[
    "custom_hud_overlay",
    "custom_particle_fx",
    "custom_weapon",
    "custom_soldier_class",
    "custom_battle_condition",
    "custom_reinforcement_card",
    "custom_map_structure",
];

/// Creates a new mod folder named after `mod_name` inside `parent_dir` from a template of the
/// catalog (`crate::templates`), by its id or an older `qm new` name.
pub fn scaffold_new_mod(
    parent_dir: &Path,
    mod_id: &str,
    mod_name: &str,
    author: &str,
    template: &str,
) -> Result<PathBuf, String> {
    let known = || {
        crate::templates::templates()
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if RETIRED_STARTERS.contains(&template) {
        return Err(format!(
            "The template '{template}' was removed: it hooked game functions that no longer match the game. Start from one of: {}.",
            known()
        ));
    }
    let info = crate::templates::find(template)
        .ok_or_else(|| format!("Unknown template '{template}'. Templates: {}.", known()))?;
    let names = crate::templates::TemplateNames::new(mod_id, mod_name, author)?;
    crate::templates::create(parent_dir, &info.id, &names)
}

/// Packages a mod folder into `<FolderName>.zip` and returns its `ReleaseModEntry` (with SHA-256 digest).
pub fn package_mod_release(
    mod_dir: &Path,
    output_dir: &Path,
) -> Result<(PathBuf, ReleaseModEntry), String> {
    let installed = inspect_mod_directory(mod_dir, false)?;
    fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create {}: {e}", output_dir.display()))?;

    let asset_name = format!("{}.zip", installed.folder_name);
    let zip_path = output_dir.join(&asset_name);
    // Every entry sits under one folder named after the mod (`ExampleMod/mod.json`), so the
    // zip unpacked into a Mods folder gives `Mods/ExampleMod/`.
    let top = &installed.folder_name;

    {
        let file = fs::File::create(&zip_path)
            .map_err(|e| format!("Failed to create {}: {e}", zip_path.display()))?;
        let mut zip = ZipWriter::new(file);

        for entry in WalkDir::new(mod_dir).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let rel = p
                .strip_prefix(mod_dir)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            if rel.starts_with(".git")
                || rel.ends_with(".bak")
                || rel == ".DS_Store"
                || rel == ".qm-integrity.json"
            {
                continue;
            }
            zip.start_file(format!("{top}/{rel}"), SimpleFileOptions::default())
                .map_err(|e| e.to_string())?;
            let bytes = fs::read(p).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
    }

    let zip_bytes = fs::read(&zip_path).map_err(|e| e.to_string())?;
    let sha256 = sha256_hex(&zip_bytes);

    let entry = ReleaseModEntry {
        id: installed.manifest.id,
        name: installed.manifest.name,
        version: installed.manifest.version,
        asset_name,
        sha256,
        size_bytes: zip_bytes.len() as u64,
        game_version: installed.manifest.game_version,
        changelog: installed.changelog_markdown,
    };

    Ok((zip_path, entry))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileInfo {
    pub relative_path: String,
    pub file_type: String,
    pub size_bytes: u64,
}

fn is_confined_to_base(candidate: &Path, base: &Path) -> bool {
    let cand_clean = dunce::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    let base_clean = dunce::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    if cand_clean.starts_with(&base_clean) {
        return true;
    }
    if candidate.starts_with(base) {
        return true;
    }
    let cand_str = cand_clean.to_string_lossy().replace('\\', "/").to_lowercase();
    let base_str = base_clean.to_string_lossy().replace('\\', "/").to_lowercase();
    cand_str == base_str || cand_str.starts_with(&format!("{}/", base_str.trim_end_matches('/')))
}

/// Safely resolves a mod directory path, rejecting path traversal attempts and drive letters,
/// ensuring the directory is strictly confined to workspace_mods_dir or library_dir.
pub fn resolve_mod_dir(mod_dir: &str, paths: &QuarterMasterPaths, for_save: bool) -> Result<PathBuf, String> {
    let trimmed = mod_dir.trim();
    if trimmed.is_empty() {
        return Err("Empty mod_dir parameter".to_string());
    }

    let normalized = trimmed.replace('\\', "/");

    // 1. Strictly reject path traversal tokens
    if normalized.contains("..") {
        return Err(format!(
            "Rejected path traversal attempt in mod_dir '{trimmed}'"
        ));
    }
    if trimmed == "."
        || normalized.starts_with("./")
        || normalized.ends_with("/.")
        || normalized.contains("/./")
    {
        return Err(format!(
            "Rejected invalid relative token in mod_dir '{trimmed}'"
        ));
    }

    // Determine allowed roots: workspace_mods_dir and library_dir
    let settings = paths.load_settings();
    let mut ws_dir = settings.workspace_mods_dir.or_else(crate::storage::detect_workspace_mods_dir);
    if ws_dir.is_none() {
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(parent) = cwd.parent() {
                if cwd.ends_with("Quartermaster") {
                    ws_dir = Some(parent.join("MnB-Mods"));
                }
            }
            if ws_dir.is_none() {
                ws_dir = Some(cwd.join("MnB-Mods"));
            }
        }
    }

    let library_dir = &paths.library_dir;
    let candidate_path = PathBuf::from(trimmed);

    // 1. Check installed mods in library_dir or workspace by exact/case-insensitive mod ID or installed directory
    for installed in paths.list_installed_mods() {
        if installed.manifest.id.eq_ignore_ascii_case(trimmed) {
            return Ok(installed.install_path);
        }
        if candidate_path.is_absolute() && is_confined_to_base(&candidate_path, &installed.install_path) {
            return Ok(candidate_path);
        }
    }

    // 2. If mod_dir is absolute or drive-prefixed, verify it is safely confined to one of our legitimate roots
    if candidate_path.is_absolute()
        || normalized.contains(':')
        || normalized.starts_with('/')
        || trimmed.starts_with('\\')
    {
        let default_appdata = dirs::data_dir().map(|d| d.join("QuarterMaster"));
        let default_lib = default_appdata.as_ref().map(|d| d.join("library"));

        let is_allowed = is_confined_to_base(&candidate_path, library_dir)
            || (ws_dir.as_ref().map(|ws| is_confined_to_base(&candidate_path, ws)).unwrap_or(false))
            || is_confined_to_base(&candidate_path, &paths.root_dir)
            || is_confined_to_base(&candidate_path, &paths.profiles_dir)
            || (default_lib.as_ref().map(|l| is_confined_to_base(&candidate_path, l)).unwrap_or(false))
            || (default_appdata.as_ref().map(|a| is_confined_to_base(&candidate_path, a)).unwrap_or(false));

        if is_allowed {
            if candidate_path.is_dir() {
                return Ok(candidate_path);
            }
            if for_save {
                let _ = fs::create_dir_all(&candidate_path);
                if candidate_path.is_dir() {
                    return Ok(candidate_path);
                }
            }
        }
        return Err(format!(
            "Rejected absolute or drive-prefixed mod_dir '{trimmed}'"
        ));
    }

    let subpath_in_ws = if normalized.starts_with("MnB-Mods/") {
        normalized.trim_start_matches("MnB-Mods/").trim_start_matches('/')
    } else if normalized == "MnB-Mods" {
        ""
    } else {
        trimmed
    };

    // 1. Try workspace_mods_dir
    if let Some(ref ws) = ws_dir {
        if normalized == "MnB-Mods" && ws.is_dir() && is_confined_to_base(ws, ws) {
            return Ok(ws.clone());
        }

        if !subpath_in_ws.is_empty() {
            let candidate = ws.join(subpath_in_ws);
            if candidate.is_dir() && is_confined_to_base(&candidate, ws) {
                return Ok(candidate);
            }
        }

        let direct_in_ws = ws.join(trimmed);
        if direct_in_ws.is_dir() && is_confined_to_base(&direct_in_ws, ws) {
            return Ok(direct_in_ws);
        }
    }

    // 2. Try library_dir
    let lib_candidate = library_dir.join(trimmed);
    if lib_candidate.is_dir() && is_confined_to_base(&lib_candidate, library_dir) {
        return Ok(lib_candidate);
    }
    if !subpath_in_ws.is_empty() && subpath_in_ws != trimmed {
        let lib_cand_stripped = library_dir.join(subpath_in_ws);
        if lib_cand_stripped.is_dir() && is_confined_to_base(&lib_cand_stripped, library_dir) {
            return Ok(lib_cand_stripped);
        }
    }

    // 3. Search installed mods in library_dir
    for installed in paths.list_installed_mods() {
        if (installed.manifest.id == trimmed
            || installed.manifest.id == subpath_in_ws
            || installed.install_path.ends_with(trimmed))
            && is_confined_to_base(&installed.install_path, library_dir)
        {
            return Ok(installed.install_path);
        }
    }

    // 4. If for_save is true, allow creating the directory under workspace_mods_dir
    if for_save {
        if let Some(ref ws) = ws_dir {
            let target = if normalized == "MnB-Mods" {
                ws.clone()
            } else if !subpath_in_ws.is_empty() {
                ws.join(subpath_in_ws)
            } else {
                ws.join(trimmed)
            };

            let _ = fs::create_dir_all(&target);
            if target.is_dir() && is_confined_to_base(&target, ws) {
                return Ok(target);
            }
        }
    }

    Err(format!("Mod directory not found: '{trimmed}'"))
}

/// Lists internal files in a mod directory, returning categorized `FileInfo` entries.
pub fn list_mod_files(mod_dir: &Path) -> Result<Vec<FileInfo>, String> {
    if !mod_dir.is_dir() {
        return Err(format!("Mod directory does not exist: {}", mod_dir.display()));
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(mod_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Skip VCS directories and hidden metadata files
        if path.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
            continue;
        }

        let rel = match path.strip_prefix(mod_dir) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => path.to_string_lossy().replace('\\', "/"),
        };

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let file_type = match ext.as_str() {
            "lua" => "lua",
            "json" => "json",
            "toml" => "toml",
            _ => "other",
        }
        .to_string();

        let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);

        files.push(FileInfo {
            relative_path: rel,
            file_type,
            size_bytes,
        });
    }

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

/// Reads an internal mod file as UTF-8 string with strict path traversal checking.
pub fn read_mod_file(mod_dir: &Path, file_path: &str) -> Result<String, String> {
    let clean_rel = validate_safe_relative_path(file_path)?;
    let target = mod_dir.join(&clean_rel);
    if !target.is_file() {
        return Err(format!("File not found: '{}'", clean_rel.display()));
    }
    fs::read_to_string(&target)
        .map_err(|e| format!("Failed to read file '{}': {e}", target.display()))
}

/// Writes content to an internal mod file with strict path traversal checking.
pub fn save_mod_file(mod_dir: &Path, file_path: &str, content: &str) -> Result<PathBuf, String> {
    let clean_rel = validate_safe_relative_path(file_path)?;
    let target = mod_dir.join(&clean_rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory '{}': {e}", parent.display()))?;
    }
    fs::write(&target, content.as_bytes())
        .map_err(|e| format!("Failed to write file '{}': {e}", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_warns_when_the_short_form_is_used_without_its_feature_name() {
        let tmp = tempfile::tempdir().unwrap();
        let short = "[manifest]\nversion = \"1.0.0\"\n\n[[pattern]]\ntarget = \"main.lua\"\npattern = \"x\"\nposition = \"after\"\npayload = \"y\"\n\n[[copy]]\ntarget = \"main.lua\"\nposition = \"append\"\npayload = \"z\"\n";
        fs::write(tmp.path().join("qm.toml"), short).unwrap();
        let d = short_form_diagnostic(tmp.path(), &[]).unwrap();
        assert!(d.message.contains("[[pattern]], [[copy]]"), "{}", d.message);
        assert!(d.message.contains("short_patches"));
        // Listed: nothing to say.
        assert!(short_form_diagnostic(tmp.path(), &["short_patches".to_string()]).is_none());
        // The long form needs no feature.
        let long = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"x\"\nposition = \"after\"\npayload = \"y\"\n";
        fs::write(tmp.path().join("qm.toml"), long).unwrap();
        assert!(short_form_diagnostic(tmp.path(), &[]).is_none());
    }

    #[test]
    fn lint_warns_when_a_submenu_is_used_without_its_feature_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = r#"
[[options]]
id = "advanced"
label = "Advanced"
type = "submenu"

[[options]]
id = "flag"
label = "Flag"
type = "checkbox"
default = true
submenu = "advanced"
"#;
        fs::write(tmp.path().join("config.toml"), cfg).unwrap();
        let d = submenu_diagnostic(tmp.path(), &[]).unwrap();
        assert!(d.message.contains("config_submenus"), "{}", d.message);
        assert!(submenu_diagnostic(tmp.path(), &["config_submenus".to_string()]).is_none());
        // No submenu, or no options at all: nothing to say.
        let plain = r#"
[[options]]
id = "flag"
label = "Flag"
"#;
        fs::write(tmp.path().join("config.toml"), plain).unwrap();
        assert!(submenu_diagnostic(tmp.path(), &[]).is_none());
        fs::remove_file(tmp.path().join("config.toml")).unwrap();
        assert!(submenu_diagnostic(tmp.path(), &[]).is_none());
    }

    #[test]
    fn lint_warns_when_a_lua_api_is_used_without_its_feature_name() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("events.lua"), "QM.Events.on(\"battle_start\", function() end)\n").unwrap();
        let d = lua_feature_diagnostics(tmp.path(), &[]);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("\"events\""), "{}", d[0].message);
        assert!(lua_feature_diagnostics(tmp.path(), &["events".to_string()]).is_empty());
        fs::write(tmp.path().join("grenade.lua"), "QM.Ordnance.register({ id = \"x\" })\n").unwrap();
        let d = lua_feature_diagnostics(tmp.path(), &["events".to_string()]);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("\"ordnance\""), "{}", d[0].message);
        assert!(lua_feature_diagnostics(tmp.path(), &["events".to_string(), "ordnance".to_string()]).is_empty());
        // A faction's ordnance.
        let f = tempfile::tempdir().unwrap();
        fs::write(f.path().join("faction.toml"), "schema = 1\n\n[[ordnance]]\nid = \"g\"\n").unwrap();
        let d = lua_feature_diagnostics(f.path(), &[]);
        assert!(d.len() == 1 && d[0].message.contains("faction.toml has [[ordnance]]"), "{d:?}");
        assert!(lua_feature_diagnostics(f.path(), &["ordnance".to_string()]).is_empty());
        // A faction's skills.
        let k = tempfile::tempdir().unwrap();
        fs::write(k.path().join("faction.toml"), "schema = 1\n\n[[skills]]\nclass = \"rifle\"\nslot = 1\n").unwrap();
        let d = lua_feature_diagnostics(k.path(), &[]);
        assert!(d.len() == 1 && d[0].message.contains("faction.toml has [[skills]]"), "{d:?}");
        assert!(lua_feature_diagnostics(k.path(), &["skills".to_string()]).is_empty());
        // A faction's rank names and sized sprites.
        let r = tempfile::tempdir().unwrap();
        fs::write(
            r.path().join("faction.toml"),
            "schema = 1

[ranks]
soldier = [\"A\"]

[sprites]
rank_1 = { file = \"r.png\", size = [18, 22] }
flag = \"f.png\"
",
        )
        .unwrap();
        let d = lua_feature_diagnostics(r.path(), &[]);
        assert!(d.len() == 2, "{d:?}");
        assert!(d.iter().any(|m| m.message.contains("faction.toml has [ranks]")), "{d:?}");
        assert!(d.iter().any(|m| m.message.contains("\"faction_sprite_sizes\"")), "{d:?}");
        let both = ["ranks".to_string(), "faction_sprite_sizes".to_string()];
        assert!(lua_feature_diagnostics(r.path(), &both).is_empty());
        // qm.toml sprite rules: a size alone needs nothing; a place needs sprite_transforms.
        let p = tempfile::tempdir().unwrap();
        fs::write(p.path().join("qm.toml"), "[sprites.top_m1]\nsize = [40, 12]\n").unwrap();
        assert!(lua_feature_diagnostics(p.path(), &[]).is_empty());
        fs::write(p.path().join("qm.toml"), "[sprites.top_m1]\nsize = [40, 12]\ndraw_size = [44, 12]\n").unwrap();
        let d = lua_feature_diagnostics(p.path(), &[]);
        assert!(d.len() == 1 && d[0].message.contains("\"sprite_transforms\""), "{d:?}");
        assert!(lua_feature_diagnostics(p.path(), &["sprite_transforms".to_string()]).is_empty());
    }

    /// A folder with a launcher, its loader and (when given) a staged pair, as an update finds it.
    #[cfg(windows)]
    struct SwapDir {
        _tmp: tempfile::TempDir,
        launcher: PathBuf,
        staged: PathBuf,
        old: PathBuf,
        loader: PathBuf,
        staged_loader: PathBuf,
        old_loader: PathBuf,
        cli: CliSwap,
        script: PathBuf,
    }

    #[cfg(windows)]
    impl SwapDir {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            // A folder name with a space and a percent sign, as a user's folder may have.
            let dir = tmp.path().join("Quarter Master 100%");
            fs::create_dir_all(dir.join("cache")).unwrap();
            let d = SwapDir {
                launcher: dir.join("qm-launcher.exe"),
                staged: dir.join("cache").join(STAGED_EXE),
                old: dir.join("qm-launcher.old.exe"),
                loader: dir.join(LOADER_DLL),
                staged_loader: dir.join("cache").join(STAGED_DLL),
                old_loader: dir.join(OLD_LOADER_DLL),
                cli: CliSwap {
                    exe: dir.join(CLI_EXE),
                    staged: dir.join("cache").join(STAGED_CLI),
                    old: dir.join(OLD_CLI),
                },
                script: dir.join("cache").join("swap.bat"),
                _tmp: tmp,
            };
            fs::write(&d.launcher, "launcher 1").unwrap();
            fs::write(&d.loader, "loader 1").unwrap();
            d
        }

        fn run(&self, script: String) {
            fs::write(&self.script, script).unwrap();
            let status = std::process::Command::new("cmd")
                .arg("/C")
                .arg(&self.script)
                .status()
                .unwrap();
            assert!(status.success());
        }

        fn swap(&self) {
            self.run(swap_script(
                &self.launcher,
                &self.staged,
                &self.old,
                &self.loader,
                &self.staged_loader,
                &self.old_loader,
                &self.cli,
                false,
            ));
        }

        fn rollback(&self) {
            self.run(rollback_script(&self.launcher, &self.old, &self.loader, &self.old_loader, &self.cli, false));
        }

        fn text(p: &Path) -> Option<String> {
            fs::read_to_string(p).ok()
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_swap_replaces_the_launcher_and_the_loader_and_keeps_the_old_pair() {
        let d = SwapDir::new();
        fs::write(&d.staged, "launcher 2").unwrap();
        fs::write(&d.staged_loader, "loader 2").unwrap();
        d.swap();
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 2"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 2"));
        assert_eq!(SwapDir::text(&d.old).as_deref(), Some("launcher 1"));
        assert_eq!(SwapDir::text(&d.old_loader).as_deref(), Some("loader 1"));
        assert!(!d.staged.exists() && !d.staged_loader.exists());

        // A second update keeps only the pair it replaced.
        fs::write(&d.staged, "launcher 3").unwrap();
        fs::write(&d.staged_loader, "loader 3").unwrap();
        d.swap();
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 3"));
        assert_eq!(SwapDir::text(&d.old).as_deref(), Some("launcher 2"));
        assert_eq!(SwapDir::text(&d.old_loader).as_deref(), Some("loader 2"));

        // Going back restores that pair together.
        d.rollback();
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 2"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 2"));
        assert!(!d.old.exists() && !d.old_loader.exists());
    }

    /// The way the launcher really runs it: hidden, through `cmd /C`, from a folder with a space
    /// and a percent sign. The swap happens and the script deletes itself afterwards.
    #[cfg(windows)]
    #[test]
    fn a_hidden_swap_runs_and_removes_its_script() {
        let d = SwapDir::new();
        fs::write(&d.staged, "launcher 2").unwrap();
        fs::write(&d.staged_loader, "loader 2").unwrap();
        let script = swap_script(&d.launcher, &d.staged, &d.old, &d.loader, &d.staged_loader, &d.old_loader, &d.cli, false);
        fs::write(&d.script, script).unwrap();
        let status = spawn_hidden_script(&d.script, d.script.parent().unwrap()).unwrap().wait().unwrap();
        assert!(status.success());
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 2"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 2"));
        assert!(!d.script.exists(), "the script deletes itself");
    }

    #[cfg(windows)]
    #[test]
    fn the_command_line_tool_is_swapped_only_where_one_is_installed() {
        // Installed: replaced with the launcher, the old one kept, and put back on a rollback.
        let d = SwapDir::new();
        fs::write(&d.cli.exe, "qm 1").unwrap();
        fs::write(&d.staged, "launcher 2").unwrap();
        fs::write(&d.staged_loader, "loader 2").unwrap();
        fs::write(&d.cli.staged, "qm 2").unwrap();
        d.swap();
        assert_eq!(SwapDir::text(&d.cli.exe).as_deref(), Some("qm 2"));
        assert_eq!(SwapDir::text(&d.cli.old).as_deref(), Some("qm 1"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 2"));
        d.rollback();
        assert_eq!(SwapDir::text(&d.cli.exe).as_deref(), Some("qm 1"));
        assert!(!d.cli.old.exists());

        // Not installed (the user left it out): none appears, and the staged copy is dropped.
        let d = SwapDir::new();
        fs::write(&d.staged, "launcher 2").unwrap();
        fs::write(&d.staged_loader, "loader 2").unwrap();
        fs::write(&d.cli.staged, "qm 2").unwrap();
        d.swap();
        assert!(!d.cli.exe.exists() && !d.cli.staged.exists() && !d.cli.old.exists());
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 2"));

        // Staged without a launcher: nothing moves.
        let d = SwapDir::new();
        fs::write(&d.cli.exe, "qm 1").unwrap();
        fs::write(&d.cli.staged, "qm 2").unwrap();
        d.swap();
        assert_eq!(SwapDir::text(&d.cli.exe).as_deref(), Some("qm 1"));
    }

    #[cfg(windows)]
    #[test]
    fn a_swap_with_only_a_launcher_staged_leaves_the_loader() {
        let d = SwapDir::new();
        fs::write(&d.staged, "launcher 2").unwrap();
        d.swap();
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 2"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 1"));
        assert!(!d.old_loader.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_swap_with_nothing_staged_changes_nothing() {
        let d = SwapDir::new();
        // A loader alone must never go in under the old launcher.
        fs::write(&d.staged_loader, "loader 2").unwrap();
        d.swap();
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 1"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 1"));
        assert!(!d.old.exists() && !d.old_loader.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_rollback_with_no_previous_launcher_changes_nothing() {
        let d = SwapDir::new();
        fs::write(&d.old_loader, "loader 0").unwrap();
        d.rollback();
        assert_eq!(SwapDir::text(&d.launcher).as_deref(), Some("launcher 1"));
        assert_eq!(SwapDir::text(&d.loader).as_deref(), Some("loader 1"));
    }

    #[test]
    fn roundtrips_qm1_profile_share_code() {
        let pack = ModpackManifest {
            schema_version: 1,
            name: "Tactical Vet Pack".to_string(),
            description: "Hardcore MnB setup".to_string(),
            exported_at: "2026-10-24T12:00:00Z".to_string(),
            mods: vec![ModpackModEntry {
                id: "gcore".to_string(),
                name: "GCore".to_string(),
                version: "1.0.0".to_string(),
                priority: 5,
                sha256: None,
                update_keys: vec!["GitHub:Gamiseus/MnB-Mods#GCore.zip".to_string()],
            }],
            mod_priorities: HashMap::new(),
            mod_settings: ModSettingsStore::default(),
        };

        let code = encode_profile_code(&pack).unwrap();
        assert!(code.starts_with("QM1-"));
        let decoded = decode_profile_code(&code).unwrap();
        assert_eq!(decoded.name, "Tactical Vet Pack");
        assert_eq!(decoded.mods.len(), 1);
        assert_eq!(decoded.mods[0].id, "gcore");
    }

    /// `qm pack` puts every entry under one folder named after the mod, leaves out
    /// leftovers, and the loader reads the zip as the mod.
    /// A modpack from anyone cannot pick a profile folder outside the profiles folder, and
    /// cannot replace a profile that is there.
    #[test]
    fn a_modpack_profile_id_is_plain_and_unused() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let pack = ModpackManifest {
            schema_version: 1,
            name: "..\\..\\Evil Pack".into(),
            description: String::new(),
            exported_at: String::new(),
            mods: Vec::new(),
            mod_priorities: HashMap::new(),
            mod_settings: Default::default(),
        };
        for bad in ["", "..", "..\\..\\evil", "a/b", "a b", "C:x"] {
            assert!(paths.apply_modpack_as_profile(&pack, bad).is_err(), "{bad:?}");
        }
        assert!(!temp.path().join("evil").exists());

        let id = paths.free_profile_id(&pack.name);
        assert_eq!(id, "evil-pack");
        paths.apply_modpack_as_profile(&pack, &id).unwrap();
        let err = paths.apply_modpack_as_profile(&pack, &id).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(paths.free_profile_id(&pack.name), "evil-pack-2");
        assert_eq!(paths.free_profile_id("///"), "modpack");
        assert_eq!(paths.free_profile_id(crate::VANILLA_ID), "modpack");
    }

    #[test]
    fn pack_puts_the_mod_in_one_folder_named_after_it() {
        let temp = tempfile::TempDir::new().unwrap();
        let mod_dir = temp.path().join("ExampleMod");
        fs::create_dir_all(mod_dir.join("qm")).unwrap();
        fs::write(
            mod_dir.join("mod.json"),
            r#"{"id": "example_mod", "name": "Example", "version": "1.2.3"}"#,
        )
        .unwrap();
        fs::write(
            mod_dir.join("qm.toml"),
            "[manifest]\nversion = \"1.2.3\"\n\n[[patches]]\n[patches.copy]\ntarget = \"main.lua\"\nposition = \"append\"\nsources = [\"inject.lua\"]\n",
        )
        .unwrap();
        fs::write(mod_dir.join("inject.lua"), "-- example").unwrap();
        fs::write(mod_dir.join("qm").join("extra.toml"), "[manifest]\nversion = \"1.2.3\"\n").unwrap();
        fs::write(mod_dir.join("old.bak"), "").unwrap();
        fs::write(mod_dir.join(".qm-integrity.json"), "{}").unwrap();

        let out = temp.path().join("dist");
        let (zip_path, entry) = package_mod_release(&mod_dir, &out).unwrap();
        assert_eq!(zip_path, out.join("ExampleMod.zip"));
        assert_eq!((entry.id.as_str(), entry.asset_name.as_str()), ("example_mod", "ExampleMod.zip"));

        let mut archive = ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        let mut names: Vec<String> =
            (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "ExampleMod/inject.lua",
                "ExampleMod/mod.json",
                "ExampleMod/qm.toml",
                "ExampleMod/qm/extra.toml",
            ]
        );

        // Dropped into a Mods folder as is, the zip loads as the mod.
        let mods = temp.path().join("Mods");
        fs::create_dir_all(&mods).unwrap();
        fs::copy(&zip_path, mods.join("ExampleMod.zip")).unwrap();
        let patches = qm_core::patch::loader::load_patches_new(&mods).unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].0.mod_id(), Some("example_mod"));
    }

    #[test]
    fn qm_new_writes_catalog_templates_and_old_names_still_work() {
        let temp = tempfile::tempdir().unwrap();
        for (i, old) in ["empty", "patch", "module", "settings", "assets", "faction", "faction_mod", "gmenu_settings"].iter().enumerate() {
            let dir = scaffold_new_mod(temp.path(), &format!("test-mod-{i}"), &format!("Test Mod {i}"), "Tester", old).unwrap();
            assert!(dir.join("mod.json").is_file() && dir.join("README.md").is_file(), "{old}");
            let manifest = qm_schema::ModManifest::from_path(&dir.join("mod.json")).unwrap();
            assert_eq!(manifest.id, format!("test-mod-{i}"), "{old}");
            assert_eq!(manifest.authors, vec!["Tester".to_string()], "{old}");
        }
        // A removed template and a mistyped one say what to use instead.
        let gone = scaffold_new_mod(temp.path(), "x", "Gone", "T", "custom_weapon").unwrap_err();
        assert!(gone.contains("was removed") && gone.contains("module"), "{gone}");
        let typo = scaffold_new_mod(temp.path(), "x", "Typo", "T", "modul").unwrap_err();
        assert!(typo.contains("Unknown template") && !temp.path().join("Typo").exists(), "{typo}");
    }

    #[test]
    fn the_faction_template_scaffolds_a_valid_faction() {
        let temp = tempfile::tempdir().unwrap();
        let dir = scaffold_new_mod(temp.path(), "eastern-front", "Eastern Front", "Tester", "faction").unwrap();
        let report = qm_schema::faction::load_faction(&dir);
        assert!(!report.has_errors(), "{:?}", report.diagnostics);
        let f = report.faction.unwrap();
        assert_eq!(f.id, "eastern_front");
        assert_eq!(f.name, "Eastern Front");
    }

    #[test]
    fn test_mod_file_operations_and_traversal_guards() {
        let temp_dir = std::env::temp_dir().join(format!(
            "qm_file_ops_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let _ = fs::create_dir_all(&temp_dir);

        // 1. Save valid files
        let lua_res = save_mod_file(&temp_dir, "config.lua", "return { enabled = true }");
        assert!(lua_res.is_ok());
        let toml_res = save_mod_file(&temp_dir, "qm.toml", "[manifest]\nversion = \"1.0.0\"");
        assert!(toml_res.is_ok());
        let json_res = save_mod_file(&temp_dir, "mod.json", "{\"id\": \"test\"}");
        assert!(json_res.is_ok());
        let nested_res = save_mod_file(&temp_dir, "scripts/helper.lua", "print('nested')");
        assert!(nested_res.is_ok());
        let zero_res = save_mod_file(&temp_dir, "empty.txt", "");
        assert!(zero_res.is_ok());

        // 2. Reject path traversal on save
        assert!(save_mod_file(&temp_dir, "../escape.txt", "hack").is_err());
        assert!(save_mod_file(&temp_dir, "../../escape.txt", "hack").is_err());
        assert!(save_mod_file(&temp_dir, "CON", "hack").is_err());

        // 3. List mod files
        let files = list_mod_files(&temp_dir).unwrap();
        assert_eq!(files.len(), 5);
        let types: std::collections::HashSet<_> = files.iter().map(|f| f.file_type.as_str()).collect();
        assert!(types.contains("lua"));
        assert!(types.contains("toml"));
        assert!(types.contains("json"));
        assert!(types.contains("other"));

        let config_file = files.iter().find(|f| f.relative_path == "config.lua").unwrap();
        assert!(config_file.size_bytes > 0);

        // 4. Read mod file
        let content = read_mod_file(&temp_dir, "config.lua").unwrap();
        assert_eq!(content, "return { enabled = true }");

        let nested_content = read_mod_file(&temp_dir, "scripts/helper.lua").unwrap();
        assert_eq!(nested_content, "print('nested')");

        // 5. Read non-existent
        assert!(read_mod_file(&temp_dir, "nonexistent.lua").is_err());

        // 6. Reject path traversal on read
        assert!(read_mod_file(&temp_dir, "../../../../windows/system32/cmd.exe").is_err());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_resolve_mod_dir_traversal_and_drive_rejection() {
        let temp_dir = std::env::temp_dir().join(format!(
            "qm_resolve_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let paths = QuarterMasterPaths::resolve(Some(&temp_dir));
        let ws_mods = temp_dir.join("workspace_mods");
        let _ = fs::create_dir_all(&ws_mods);

        let mut settings = paths.load_settings();
        settings.workspace_mods_dir = Some(ws_mods.clone());
        let _ = paths.save_settings(&settings);

        // 1. Rejects drive letters and absolute paths
        assert!(resolve_mod_dir("C:\\", &paths, false).is_err());
        assert!(resolve_mod_dir("C:\\Windows", &paths, false).is_err());
        assert!(resolve_mod_dir("C:/Windows", &paths, false).is_err());
        assert!(resolve_mod_dir("D:\\test", &paths, false).is_err());
        assert!(resolve_mod_dir("/etc/passwd", &paths, false).is_err());
        assert!(resolve_mod_dir("\\Windows\\System32", &paths, false).is_err());

        // 2. Rejects path traversal tokens
        assert!(resolve_mod_dir("../../", &paths, false).is_err());
        assert!(resolve_mod_dir("/..", &paths, false).is_err());
        assert!(resolve_mod_dir("../..", &paths, false).is_err());
        assert!(resolve_mod_dir("foo/../../bar", &paths, false).is_err());
        assert!(resolve_mod_dir(".", &paths, false).is_err());
        assert!(resolve_mod_dir("./", &paths, false).is_err());

        // 3. Rejects empty string
        assert!(resolve_mod_dir("", &paths, false).is_err());
        assert!(resolve_mod_dir("   ", &paths, false).is_err());

        // 4. Resolves valid directory inside workspace
        let sample_mod = ws_mods.join("sample_mod");
        let _ = fs::create_dir_all(&sample_mod);
        let resolved = resolve_mod_dir("sample_mod", &paths, false);
        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), sample_mod);

        // 5. Creating mod under workspace when for_save is true
        let save_mod = resolve_mod_dir("new_saved_mod", &paths, true);
        assert!(save_mod.is_ok());
        assert!(save_mod.unwrap().is_dir());

        // 6. Accepts absolute path when it is safely inside workspace_mods_dir
        assert!(resolve_mod_dir(sample_mod.to_str().unwrap(), &paths, false).is_ok());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn lints_and_watches_asset_only_mod() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let ws_mods = temp.path().join("ws_mods");
        let mod_dir = ws_mods.join("AssetOnlyMod");
        fs::create_dir_all(mod_dir.join("assets/sounds")).unwrap();
        fs::write(mod_dir.join("assets/sounds/garand1.wav"), b"RIFF_GARAND").unwrap();

        let mut settings = paths.load_settings();
        settings.workspace_mods_dir = Some(ws_mods.clone());
        paths.save_settings(&settings).unwrap();

        let mut vanilla_files = HashMap::new();
        vanilla_files.insert(
            "main.lua".to_string(),
            "function love.load()\n  setOptionVolume()\nend\n".to_string(),
        );

        let report = lint_mod_directory(&mod_dir, &vanilla_files).unwrap();
        assert!(report.passed, "Diagnostics: {:?}", report.diagnostics);
        assert_eq!(report.patch_count, 1);
        assert_eq!(report.matched_patches, 1);
        assert!(report.zero_match_patches.is_empty());

        let poll = paths.poll_dev_watch_changes(1);
        assert_eq!(poll["changed"], serde_json::json!(true));
        let changed_files = poll["changed_files"].as_array().unwrap();
        assert!(changed_files
            .iter()
            .any(|v| v.as_str() == Some("AssetOnlyMod/assets/sounds/garand1.wav")));
    }
}

