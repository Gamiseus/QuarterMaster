use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use qm_core::handshake::{GluePatchEntry, LaunchConfig, LoadOrderPlan};
use qm_schema::{ModSettingsStore, SaveSummary};

use crate::library::{copy_directory_clean, InstalledMod};
use crate::solver::resolve_load_order;
use crate::storage::{LaunchMethod, QuarterMasterPaths, StateJournal};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSeparator {
    pub id: String,
    pub title: String,
    pub color: String,
    pub insert_before_mod_id: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub enabled_mods: Vec<String>,
    #[serde(default)]
    pub separators: Vec<ProfileSeparator>,
    #[serde(default)]
    pub mod_priorities: HashMap<String, i32>,
    #[serde(default)]
    pub user_load_after: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub patch_priorities: HashMap<String, i32>,
    #[serde(default)]
    pub disabled_patches: Vec<String>,
    #[serde(default)]
    pub glue_patches: Vec<GluePatchEntry>,
    #[serde(default = "default_save_id")]
    pub linked_save_profile_id: String,
    #[serde(default)]
    pub launch_method: Option<LaunchMethod>,
    #[serde(default)]
    pub disable_console: bool,
    #[serde(default)]
    pub custom_launch_args: Vec<String>,
    /// Optional LÖVE `t.identity` override if the user wants complete Steam Cloud isolation.
    #[serde(default)]
    pub isolate_save_identity: bool,
    /// Per-mod pinned version overrides (`mod_id -> version`).
    #[serde(default)]
    pub pinned_versions: HashMap<String, String>,
    /// Mods whose priority is locked against automatic re-sorting.
    #[serde(default)]
    pub locked_mods: Vec<String>,
    /// Starred / favorite mods in this profile.
    #[serde(default)]
    pub favorites: Vec<String>,
    /// User notes attached to individual mods (`mod_id -> note`).
    #[serde(default)]
    pub mod_notes: HashMap<String, String>,
}

fn default_save_id() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub is_vanilla_clean: bool,
    pub created_at: String,
    pub last_played_at: Option<String>,
    #[serde(default)]
    pub active_mods_fingerprint: HashMap<String, String>,
    #[serde(default)]
    pub backups: Vec<SaveBackupEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveBackupEntry {
    pub id: String,
    pub timestamp: String,
    pub label: String,
    pub path: PathBuf,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDiff {
    pub profile_a_id: String,
    pub profile_b_id: String,
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
    pub priority_diffs: Vec<(String, i32, i32)>,
    pub setting_diffs: Vec<(String, String, String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveCompatibilityReport {
    pub is_safe: bool,
    pub missing_mods_from_save: Vec<(String, String)>,
    pub is_tainting_vanilla_save: bool,
    pub warning_message: Option<String>,
}

const SAVE_FILES: &[&str] = &[
    "saveMNB3.lua",
    "saveMNB2.lua",
    "saveMNBclassic.lua",
    "backUp.lua",
    "mod_settings.lua",
    "saveModSettings.lua",
    "save_meta.json",
];

impl QuarterMasterPaths {
    pub fn list_profiles(&self) -> Vec<ModProfile> {
        let mut profiles = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.profiles_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p_file = entry.path().join("profile.json");
                if let Ok(raw) = fs::read_to_string(&p_file) {
                    if let Ok(prof) = serde_json::from_str::<ModProfile>(&raw) {
                        profiles.push(prof);
                    }
                }
            }
        }
        if profiles.is_empty() {
            let installed = self.list_installed_mods();
            let enabled_mods: Vec<String> = installed.iter().map(|m| m.manifest.id.clone()).collect();
            let now = jiff::Zoned::now().to_string();
            let default_prof = ModProfile {
                id: "default".to_string(),
                name: "Default Tactical Loadout".to_string(),
                description: "Standard modded profile with all core frameworks and gameplay mods."
                    .to_string(),
                created_at: now.clone(),
                updated_at: now.clone(),
                enabled_mods,
                separators: vec![
                    ProfileSeparator {
                        id: "sep-core".to_string(),
                        title: "CORE FRAMEWORKS".to_string(),
                        color: "#d97706".to_string(),
                        insert_before_mod_id: Some("gcore".to_string()),
                        collapsed: false,
                    },
                    ProfileSeparator {
                        id: "sep-gameplay".to_string(),
                        title: "GAMEPLAY & EVENTS".to_string(),
                        color: "#16a34a".to_string(),
                        insert_before_mod_id: Some("g_special_events".to_string()),
                        collapsed: false,
                    },
                    ProfileSeparator {
                        id: "sep-tools".to_string(),
                        title: "DEVELOPER & DIAGNOSTICS".to_string(),
                        color: "#3b82f6".to_string(),
                        insert_before_mod_id: Some("gconsole".to_string()),
                        collapsed: false,
                    },
                ],
                mod_priorities: HashMap::new(),
                user_load_after: HashMap::new(),
                patch_priorities: HashMap::new(),
                disabled_patches: Vec::new(),
                glue_patches: Vec::new(),
                linked_save_profile_id: "default".to_string(),
                launch_method: None,
                disable_console: false,
                custom_launch_args: Vec::new(),
                isolate_save_identity: false,
                pinned_versions: HashMap::new(),
                locked_mods: Vec::new(),
                favorites: Vec::new(),
                mod_notes: HashMap::new(),
            };
            let _ = self.save_profile(&default_prof);
            profiles.push(default_prof);

            let vanilla_prof = ModProfile {
                id: "vanilla".to_string(),
                name: "Pure Vanilla (Unmodded)".to_string(),
                description: "Clean vanilla Mud and Blood with zero mods active.".to_string(),
                created_at: now.clone(),
                updated_at: now,
                enabled_mods: Vec::new(),
                separators: Vec::new(),
                mod_priorities: HashMap::new(),
                user_load_after: HashMap::new(),
                patch_priorities: HashMap::new(),
                disabled_patches: Vec::new(),
                glue_patches: Vec::new(),
                linked_save_profile_id: "vanilla".to_string(),
                launch_method: Some(LaunchMethod::Vanilla),
                disable_console: true,
                custom_launch_args: Vec::new(),
                isolate_save_identity: false,
                pinned_versions: HashMap::new(),
                locked_mods: Vec::new(),
                favorites: Vec::new(),
                mod_notes: HashMap::new(),
            };
            let _ = self.save_profile(&vanilla_prof);
            profiles.push(vanilla_prof);
        }
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        profiles
    }

    pub fn get_profile(&self, profile_id: &str) -> Result<ModProfile, String> {
        self.list_profiles()
            .into_iter()
            .find(|p| p.id == profile_id)
            .ok_or_else(|| format!("Profile '{}' not found", profile_id))
    }

    pub fn save_profile(&self, profile: &ModProfile) -> Result<(), String> {
        let dir = self.profiles_dir.join(&profile.id);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create profile dir {}: {e}", dir.display()))?;
        let json = serde_json::to_string_pretty(profile)
            .map_err(|e| format!("Failed to serialize profile: {e}"))?;
        fs::write(dir.join("profile.json"), json)
            .map_err(|e| format!("Failed to write profile.json: {e}"))
    }

    pub fn clone_profile(&self, source_id: &str, new_id: &str, new_name: &str) -> Result<ModProfile, String> {
        let mut src = self.get_profile(source_id)?;
        let now = jiff::Zoned::now().to_string();
        src.id = new_id.to_string();
        src.name = new_name.to_string();
        src.created_at = now.clone();
        src.updated_at = now;
        self.save_profile(&src)?;

        // Also copy per-profile saveModSettings.lua if present
        let src_settings = self.profiles_dir.join(source_id).join("saveModSettings.lua");
        let dst_settings = self.profiles_dir.join(new_id).join("saveModSettings.lua");
        if src_settings.is_file() {
            let _ = fs::copy(&src_settings, &dst_settings);
        }
        Ok(src)
    }

    pub fn create_profile(
        &self,
        name: &str,
        clone_from_id: Option<&str>,
        linked_save_id: Option<&str>,
        description: Option<&str>,
    ) -> Result<ModProfile, String> {
        self.create_profile_with_id(None, name, clone_from_id, linked_save_id, description)
    }

    pub fn create_profile_with_id(
        &self,
        custom_id: Option<&str>,
        name: &str,
        clone_from_id: Option<&str>,
        linked_save_id: Option<&str>,
        description: Option<&str>,
    ) -> Result<ModProfile, String> {
        let trimmed_name = name.trim();
        if trimmed_name.is_empty() {
            return Err("Profile name cannot be empty or whitespace".to_string());
        }

        let prof_id = if let Some(cid) = custom_id.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let sanitized: String = cid
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
                .collect();
            let cleaned = sanitized.trim_matches('_');
            if cleaned.is_empty() {
                return Err("Profile ID cannot be empty or invalid".to_string());
            }
            if self.get_profile(cleaned).is_ok() {
                return Err(format!("Profile with ID '{}' already exists", cleaned));
            }
            cleaned.to_string()
        } else {
            let sanitized: String = trimmed_name
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c.to_ascii_lowercase() } else { '_' })
                .collect();
            let base = sanitized.trim_matches('_');
            let base = if base.is_empty() { "profile" } else { base };
            let mut candidate = base.to_string();
            let mut counter = 2;
            while self.get_profile(&candidate).is_ok() {
                candidate = format!("{}_{}", base, counter);
                counter += 1;
            }
            candidate
        };

        let now = jiff::Zoned::now().to_string();
        let save_id = linked_save_id
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("default");

        let prof = if let Some(clone_id) = clone_from_id.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let src = self.get_profile(clone_id)?;
            let mut cloned = src;
            cloned.id = prof_id.clone();
            cloned.name = trimmed_name.to_string();
            if let Some(desc) = description {
                cloned.description = desc.trim().to_string();
            }
            cloned.linked_save_profile_id = save_id.to_string();
            cloned.created_at = now.clone();
            cloned.updated_at = now;
            cloned
        } else {
            ModProfile {
                id: prof_id.clone(),
                name: trimmed_name.to_string(),
                description: description.map(|s| s.trim().to_string()).unwrap_or_default(),
                created_at: now.clone(),
                updated_at: now,
                enabled_mods: Vec::new(),
                separators: Vec::new(),
                mod_priorities: HashMap::new(),
                user_load_after: HashMap::new(),
                patch_priorities: HashMap::new(),
                disabled_patches: Vec::new(),
                glue_patches: Vec::new(),
                linked_save_profile_id: save_id.to_string(),
                launch_method: None,
                disable_console: false,
                custom_launch_args: Vec::new(),
                isolate_save_identity: false,
                pinned_versions: HashMap::new(),
                locked_mods: Vec::new(),
                favorites: Vec::new(),
                mod_notes: HashMap::new(),
            }
        };

        self.save_profile(&prof)?;

        if let Some(clone_id) = clone_from_id.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let src_settings = self.profiles_dir.join(clone_id).join("saveModSettings.lua");
            let dst_settings = self.profiles_dir.join(&prof_id).join("saveModSettings.lua");
            if src_settings.is_file() {
                let _ = fs::copy(&src_settings, &dst_settings);
            }
        }

        Ok(prof)
    }

    /// Validates `profile_id` against directory traversal characters, path separators,
    /// and whitespace, confirms strict containment within `self.profiles_dir`,
    /// and verifies that `profile.json` exists as a regular file.
    pub fn sanitize_and_resolve_profile_dir(&self, profile_id: &str) -> Result<PathBuf, String> {
        let trimmed_id = profile_id.trim();
        if trimmed_id.is_empty() {
            return Err("Profile ID cannot be empty or whitespace".to_string());
        }
        if profile_id.chars().any(|c| c.is_whitespace()) {
            return Err(format!("Profile ID '{profile_id}' cannot contain whitespace"));
        }
        if profile_id == "."
            || profile_id.contains("..")
            || profile_id.contains('/')
            || profile_id.contains('\\')
            || profile_id.contains(':')
        {
            return Err(format!(
                "Profile ID '{profile_id}' contains invalid characters or path traversal sequences"
            ));
        }

        let dir = self.profiles_dir.join(trimmed_id);

        // Verify that the resolved directory is strictly confined to self.profiles_dir
        if dir.parent() != Some(&self.profiles_dir) {
            return Err(format!(
                "Profile path escapes profiles directory: {}",
                dir.display()
            ));
        }

        // Canonicalization containment check if the directory exists on disk
        if dir.exists() {
            if let (Ok(canon_dir), Ok(canon_profiles)) =
                (dir.canonicalize(), self.profiles_dir.canonicalize())
            {
                if !canon_dir.starts_with(&canon_profiles) || canon_dir == canon_profiles {
                    return Err(format!(
                        "Profile path escapes profiles directory: {}",
                        dir.display()
                    ));
                }
            }
        }

        // Verify that self.profiles_dir.join(trimmed_id).join("profile.json").is_file()
        if !dir.join("profile.json").is_file() {
            return Err(format!("Profile '{trimmed_id}' not found"));
        }

        Ok(dir)
    }

    pub fn rename_profile(&self, profile_id: &str, new_name: &str) -> Result<ModProfile, String> {
        let trimmed_id = profile_id.trim();
        if trimmed_id == "vanilla" {
            return Err("Cannot rename protected 'vanilla' profile".to_string());
        }
        let trimmed_name = new_name.trim();
        if trimmed_name.is_empty() {
            return Err("Profile name cannot be empty or whitespace".to_string());
        }

        let _dir = self.sanitize_and_resolve_profile_dir(profile_id)?;

        let mut prof = self.get_profile(trimmed_id)?;
        prof.name = trimmed_name.to_string();
        prof.updated_at = jiff::Zoned::now().to_string();
        self.save_profile(&prof)?;
        Ok(prof)
    }

    pub fn delete_profile(&self, profile_id: &str, active_profile_id: &str) -> Result<String, String> {
        let trimmed_id = profile_id.trim();
        if trimmed_id == "vanilla" {
            return Err("Cannot delete protected 'vanilla' profile".to_string());
        }
        if trimmed_id == active_profile_id.trim() {
            return Err(format!("Cannot delete currently active profile '{}'", trimmed_id));
        }

        let dir = self.sanitize_and_resolve_profile_dir(profile_id)?;

        fs::remove_dir_all(&dir)
            .map_err(|e| format!("Failed to delete profile directory {}: {e}", dir.display()))?;
        Ok(trimmed_id.to_string())
    }

    pub fn diff_profiles(&self, id_a: &str, id_b: &str) -> Result<ProfileDiff, String> {
        let pa = self.get_profile(id_a)?;
        let pb = self.get_profile(id_b)?;

        let only_in_a: Vec<String> = pa
            .enabled_mods
            .iter()
            .filter(|m| !pb.enabled_mods.contains(m))
            .cloned()
            .collect();
        let only_in_b: Vec<String> = pb
            .enabled_mods
            .iter()
            .filter(|m| !pa.enabled_mods.contains(m))
            .cloned()
            .collect();

        let mut priority_diffs = Vec::new();
        for m in &pa.enabled_mods {
            if pb.enabled_mods.contains(m) {
                let prio_a = pa.mod_priorities.get(m).copied().unwrap_or(50);
                let prio_b = pb.mod_priorities.get(m).copied().unwrap_or(50);
                if prio_a != prio_b {
                    priority_diffs.push((m.clone(), prio_a, prio_b));
                }
            }
        }

        let store_a = ModSettingsStore::from_path(
            &self.profiles_dir.join(id_a).join("saveModSettings.lua"),
        )
        .unwrap_or_default();
        let store_b = ModSettingsStore::from_path(
            &self.profiles_dir.join(id_b).join("saveModSettings.lua"),
        )
        .unwrap_or_default();

        let mut setting_diffs = Vec::new();
        for (mod_id, opts_a) in &store_a.mods {
            if let Some(opts_b) = store_b.mods.get(mod_id) {
                for (opt_id, val_a) in opts_a {
                    if let Some(val_b) = opts_b.get(opt_id) {
                        if val_a != val_b {
                            setting_diffs.push((
                                mod_id.clone(),
                                opt_id.clone(),
                                val_a.as_display_string(),
                                val_b.as_display_string(),
                            ));
                        }
                    }
                }
            }
        }

        Ok(ProfileDiff {
            profile_a_id: id_a.to_string(),
            profile_b_id: id_b.to_string(),
            only_in_a,
            only_in_b,
            priority_diffs,
            setting_diffs,
        })
    }

    pub fn list_save_profiles(&self) -> Vec<SaveProfile> {
        let mut profiles = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.saves_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let s_file = entry.path().join("save_profile.json");
                if let Ok(raw) = fs::read_to_string(&s_file) {
                    if let Ok(mut sp) = serde_json::from_str::<SaveProfile>(&raw) {
                        sp.backups = list_backups_in_dir(&entry.path().join("backups"));
                        profiles.push(sp);
                    }
                }
            }
        }

        if profiles.is_empty() {
            let now = jiff::Zoned::now().to_string();
            let default_save = SaveProfile {
                id: "default".to_string(),
                name: "Primary Campaign & Classic Save".to_string(),
                is_vanilla_clean: false,
                created_at: now.clone(),
                last_played_at: None,
                active_mods_fingerprint: HashMap::new(),
                backups: Vec::new(),
            };
            let _ = self.save_save_profile(&default_save);

            // Seed `saves/default/active/` with the user's current `%APPDATA%\mnb\` saves if present
            let settings = self.load_settings();
            if let Some(ref live_save_dir) = settings.save_dir_path {
                let slot_active = self.saves_dir.join("default").join("active");
                let _ = fs::create_dir_all(&slot_active);
                for fname in SAVE_FILES {
                    let src = live_save_dir.join(fname);
                    if src.is_file() {
                        let _ = fs::copy(&src, slot_active.join(fname));
                    }
                }
            }
            profiles.push(default_save);

            let vanilla_save = SaveProfile {
                id: "vanilla".to_string(),
                name: "Clean Vanilla Save Slot".to_string(),
                is_vanilla_clean: true,
                created_at: now,
                last_played_at: None,
                active_mods_fingerprint: HashMap::new(),
                backups: Vec::new(),
            };
            let _ = self.save_save_profile(&vanilla_save);
            profiles.push(vanilla_save);
        }

        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        profiles
    }

    pub fn save_save_profile(&self, sp: &SaveProfile) -> Result<(), String> {
        let dir = self.saves_dir.join(&sp.id);
        fs::create_dir_all(dir.join("active"))
            .map_err(|e| format!("Failed to create save slot dir: {e}"))?;
        fs::create_dir_all(dir.join("backups"))
            .map_err(|e| format!("Failed to create save backups dir: {e}"))?;
        let json = serde_json::to_string_pretty(sp)
            .map_err(|e| format!("Failed to serialize save_profile.json: {e}"))?;
        fs::write(dir.join("save_profile.json"), json)
            .map_err(|e| format!("Failed to write save_profile.json: {e}"))
    }

    /// Inspects a save profile's `saveMNB3.lua` (or the live `%APPDATA%\mnb\saveMNB3.lua`)
    /// using the defensive zero-execution `SaveSummary` parser, caching parsed results by SHA-256 hash (Section 11).
    pub fn inspect_save_profile(&self, save_profile_id: &str) -> Result<SaveSummary, String> {
        let settings = self.load_settings();
        let slot_file = self
            .saves_dir
            .join(save_profile_id)
            .join("active")
            .join("saveMNB3.lua");

        let target_file = if slot_file.is_file() {
            Some(slot_file)
        } else if settings.active_save_profile_id == save_profile_id {
            settings
                .save_dir_path
                .as_ref()
                .map(|d| d.join("saveMNB3.lua"))
                .filter(|f| f.is_file())
        } else {
            None
        };

        if let Some(file_path) = target_file {
            if let Ok(bytes) = fs::read(&file_path) {
                let sha = qm_schema::sha256_hex(&bytes);
                let cache_dir = self.cache_dir.join("save-summaries");
                let cache_file = cache_dir.join(format!("{sha}.json"));
                if let Ok(cached_raw) = fs::read_to_string(&cache_file) {
                    if let Ok(summary) = serde_json::from_str::<SaveSummary>(&cached_raw) {
                        return Ok(summary);
                    }
                }
                let summary = SaveSummary::from_path(&file_path)?;
                let _ = fs::create_dir_all(&cache_dir);
                if let Ok(json) = serde_json::to_string(&summary) {
                    let _ = fs::write(&cache_file, json);
                }
                return Ok(summary);
            }
            return SaveSummary::from_path(&file_path);
        }

        Ok(SaveSummary {
            file_name: "saveMNB3.lua (New / Empty Slot)".to_string(),
            area_of_operations: "No Campaign Started".to_string(),
            ..Default::default()
        })
    }

    /// Creates a timestamped backup snapshot of `save_profile_id` and prunes old backups beyond `max_save_backups`.
    pub fn create_save_backup(
        &self,
        save_profile_id: &str,
        label: &str,
    ) -> Result<SaveBackupEntry, String> {
        let settings = self.load_settings();
        let slot_dir = self.saves_dir.join(save_profile_id);
        let active_dir = slot_dir.join("active");
        let backups_dir = slot_dir.join("backups");
        fs::create_dir_all(&backups_dir)
            .map_err(|e| format!("Failed to create backups dir: {e}"))?;

        // Sync from live `%APPDATA%\mnb\` first if this is the currently active save profile
        if settings.active_save_profile_id == save_profile_id {
            if let Some(ref live_dir) = settings.save_dir_path {
                let _ = fs::create_dir_all(&active_dir);
                for fname in SAVE_FILES {
                    let src = live_dir.join(fname);
                    if src.is_file() {
                        let _ = fs::copy(&src, active_dir.join(fname));
                    }
                }
            }
        }

        let now = jiff::Zoned::now();
        let ts_slug = now.strftime("%Y-%m-%d_%H-%M-%S").to_string();
        let backup_id = format!("bak-{ts_slug}");
        let snap_dir = backups_dir.join(&backup_id);
        fs::create_dir_all(&snap_dir)
            .map_err(|e| format!("Failed to create snapshot dir: {e}"))?;

        let mut total_size = 0u64;
        for fname in SAVE_FILES {
            let src = active_dir.join(fname);
            if src.is_file() {
                if let Ok(meta) = src.metadata() {
                    total_size += meta.len();
                }
                let _ = fs::copy(&src, snap_dir.join(fname));
            }
        }

        let entry = SaveBackupEntry {
            id: backup_id,
            timestamp: now.to_string(),
            label: label.to_string(),
            path: snap_dir.clone(),
            size_bytes: total_size,
        };
        if let Ok(meta_json) = serde_json::to_string_pretty(&entry) {
            let _ = fs::write(snap_dir.join("backup_meta.json"), meta_json);
        }

        // Prune oldest backups beyond max_save_backups
        let mut all_backups = list_backups_in_dir(&backups_dir);
        let max_keep = settings.max_save_backups.max(3);
        while all_backups.len() > max_keep {
            if let Some(oldest) = all_backups.pop() {
                let _ = fs::remove_dir_all(&oldest.path);
            }
        }

        Ok(entry)
    }

    /// Restores a backup snapshot into the save profile (and live `%APPDATA%\mnb\` if active).
    pub fn restore_save_backup(
        &self,
        save_profile_id: &str,
        backup_id: &str,
    ) -> Result<(), String> {
        let settings = self.load_settings();
        let slot_dir = self.saves_dir.join(save_profile_id);
        let snap_dir = slot_dir.join("backups").join(backup_id);
        if !snap_dir.is_dir() {
            return Err(format!("Backup '{}' not found", backup_id));
        }

        let active_dir = slot_dir.join("active");
        fs::create_dir_all(&active_dir).map_err(|e| e.to_string())?;
        for fname in SAVE_FILES {
            let src = snap_dir.join(fname);
            if src.is_file() {
                let _ = fs::copy(&src, active_dir.join(fname));
            }
        }

        if settings.active_save_profile_id == save_profile_id {
            if let Some(ref live_dir) = settings.save_dir_path {
                for fname in SAVE_FILES {
                    let src = active_dir.join(fname);
                    if src.is_file() {
                        let _ = fs::copy(&src, live_dir.join(fname));
                    }
                }
            }
        }
        Ok(())
    }

    /// Deletes a backup snapshot from `saves/<save_profile_id>/backups/<backup_id>`.
    pub fn delete_save_backup(
        &self,
        save_profile_id: &str,
        backup_id: &str,
    ) -> Result<(), String> {
        let snap_dir = self
            .saves_dir
            .join(save_profile_id)
            .join("backups")
            .join(backup_id);
        if !snap_dir.is_dir() {
            return Err(format!("Backup '{}' not found", backup_id));
        }
        fs::remove_dir_all(&snap_dir)
            .map_err(|e| format!("Failed to delete backup '{}': {e}", backup_id))
    }

    /// Clones a save profile (`active/` save files and metadata) into a new save slot.
    pub fn clone_save_profile(
        &self,
        src_save_id: &str,
        new_save_id: &str,
        new_name: &str,
    ) -> Result<SaveProfile, String> {
        let clean_id: String = new_save_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        if clean_id.is_empty() {
            return Err("Save profile ID cannot be empty".to_string());
        }

        let existing = self.list_save_profiles();
        if existing.iter().any(|s| s.id == clean_id) {
            return Err(format!("Save slot '{}' already exists", clean_id));
        }

        let src_sp = existing
            .into_iter()
            .find(|s| s.id == src_save_id)
            .ok_or_else(|| format!("Source save slot '{}' not found", src_save_id))?;

        let now = jiff::Zoned::now().to_string();
        let new_sp = SaveProfile {
            id: clean_id.clone(),
            name: new_name.to_string(),
            is_vanilla_clean: src_sp.is_vanilla_clean,
            created_at: now,
            last_played_at: None,
            active_mods_fingerprint: src_sp.active_mods_fingerprint.clone(),
            backups: Vec::new(),
        };
        self.save_save_profile(&new_sp)?;

        let settings = self.load_settings();
        let src_active = self.saves_dir.join(src_save_id).join("active");
        let dst_active = self.saves_dir.join(&clean_id).join("active");
        let _ = fs::create_dir_all(&dst_active);

        for fname in SAVE_FILES {
            let candidate = src_active.join(fname);
            if candidate.is_file() {
                let _ = fs::copy(&candidate, dst_active.join(fname));
            } else if settings.active_save_profile_id == src_save_id {
                if let Some(ref live_dir) = settings.save_dir_path {
                    let live_file = live_dir.join(fname);
                    if live_file.is_file() {
                        let _ = fs::copy(&live_file, dst_active.join(fname));
                    }
                }
            }
        }

        Ok(new_sp)
    }

    /// Directly switches the active save slot and links the active mod profile to it.
    pub fn activate_save_profile(&self, target_save_id: &str) -> Result<(), String> {
        if let Some(pid) = is_game_running() {
            return Err(format!(
                "Cannot switch save slots while Mud and Blood (mnb.exe, PID {pid}) is running. Please close the game first."
            ));
        }
        let mut settings = self.load_settings();
        let mut active_prof = self.get_profile(&settings.active_profile_id)?;
        let installed = self.list_installed_mods();
        let mut active_mods_map = HashMap::new();
        for id in &active_prof.enabled_mods {
            if let Some(m) = installed.iter().find(|x| &x.manifest.id == id) {
                active_mods_map.insert(id.clone(), m.manifest.version.clone());
            }
        }

        self.switch_save_profile_internal(
            &settings.active_save_profile_id,
            target_save_id,
            &active_mods_map,
            settings.save_dir_path.as_deref(),
        )?;

        active_prof.linked_save_profile_id = target_save_id.to_string();
        self.save_profile(&active_prof)?;
        settings.active_save_profile_id = target_save_id.to_string();
        self.save_settings(&settings)?;
        Ok(())
    }

    /// Checks whether activating `profile` is compatible with its linked `SaveProfile`.
    pub fn check_save_compatibility(&self, profile: &ModProfile) -> SaveCompatibilityReport {
        let save_profiles = self.list_save_profiles();
        let Some(sp) = save_profiles
            .into_iter()
            .find(|s| s.id == profile.linked_save_profile_id)
        else {
            return SaveCompatibilityReport {
                is_safe: true,
                missing_mods_from_save: Vec::new(),
                is_tainting_vanilla_save: false,
                warning_message: None,
            };
        };

        let is_tainting_vanilla_save = sp.is_vanilla_clean && !profile.enabled_mods.is_empty();
        let mut missing_mods = Vec::new();
        for (mod_id, ver) in &sp.active_mods_fingerprint {
            if !profile.enabled_mods.contains(mod_id) {
                missing_mods.push((mod_id.clone(), ver.clone()));
            }
        }

        let is_safe = !is_tainting_vanilla_save && missing_mods.is_empty();
        let warning_message = if is_tainting_vanilla_save {
            Some(format!(
                "Save slot '{}' is marked as Clean Vanilla, but profile '{}' has {} enabled mods.",
                sp.name,
                profile.name,
                profile.enabled_mods.len()
            ))
        } else if !missing_mods.is_empty() {
            let names: Vec<String> = missing_mods
                .iter()
                .map(|(m, v)| format!("{m} ({v})"))
                .collect();
            Some(format!(
                "Save slot '{}' was last played with mods that are currently disabled in '{}': {}",
                sp.name,
                profile.name,
                names.join(", ")
            ))
        } else {
            None
        };

        SaveCompatibilityReport {
            is_safe,
            missing_mods_from_save: missing_mods,
            is_tainting_vanilla_save,
            warning_message,
        }
    }

    /// Atomically activates `profile_id`:
    /// 1. Logs transaction to `state-journal.json`
    /// 2. Materializes `profiles/<profile_id>/Mods/` with directory junctions to `library/<mod_id>/<version>/`
    /// 3. Writes `Mods/qm/load-order.json` and `Mods/qm/launch-config.json`
    /// 4. Retargets `<game_dir>/Mods` junction -> `profiles/<profile_id>/Mods/`
    /// 5. Swaps `%APPDATA%\mnb\` save files and `saveModSettings.lua`
    /// 6. Commits `state-journal.json`
    pub fn activate_profile(&self, profile_id: &str) -> Result<LoadOrderPlan, String> {
        if let Some(pid) = is_game_running() {
            return Err(format!(
                "Cannot switch profiles while Mud and Blood (mnb.exe, PID {pid}) is running. Please close the game first."
            ));
        }
        let mut settings = self.load_settings();
        let profile = self.get_profile(profile_id)?;
        let installed = self.list_installed_mods();
        let by_id: HashMap<String, &InstalledMod> = installed
            .iter()
            .map(|m| (m.manifest.id.clone(), m))
            .collect();

        // Capture any untracked runtime-generated files from the outgoing profile's `Mods/` into its MO2 `overwrite/` bucket
        let prev_profile_dir = self.profiles_dir.join(&settings.active_profile_id);
        let prev_mods_dir = prev_profile_dir.join("Mods");
        let prev_overwrite_dir = prev_profile_dir.join("overwrite");
        let _ = fs::create_dir_all(&prev_overwrite_dir);
        capture_untracked_into_overwrite(&prev_mods_dir, &prev_overwrite_dir);

        let mut journal = StateJournal::load(&self.journal_file);
        journal.begin_transaction(
            &self.journal_file,
            "profile_swap",
            Some(settings.active_profile_id.clone()),
            profile_id,
            None,
        )?;

        // 1. Resolve topological load order for enabled mods
        let enabled_manifests: Vec<_> = profile
            .enabled_mods
            .iter()
            .filter_map(|id| by_id.get(id).map(|m| m.manifest.clone()))
            .collect();
        let resolution = resolve_load_order(
            &enabled_manifests,
            &profile.mod_priorities,
            &profile.user_load_after,
            Some("0.5.3b"),
        );

        // 2. Materialize `profiles/<profile_id>/Mods/` and ensure `overwrite/` bucket exists
        let profile_dir = self.profiles_dir.join(&profile.id);
        let overwrite_dir = profile_dir.join("overwrite");
        let _ = fs::create_dir_all(&overwrite_dir);
        let profile_mods_dir = profile_dir.join("Mods");
        if profile_mods_dir.exists() {
            capture_untracked_into_overwrite(&profile_mods_dir, &overwrite_dir);
            remove_junction_or_dir(&profile_mods_dir)?;
        }
        fs::create_dir_all(&profile_mods_dir)
            .map_err(|e| format!("Failed to create {}: {e}", profile_mods_dir.display()))?;

        let mut mod_priorities = HashMap::new();
        let mut active_mods_map = HashMap::new();
        let mut ordered_mod_ids = Vec::new();

        for entry in &resolution.ordered_mods {
            let pinned_ver = profile.pinned_versions.get(&entry.mod_id).map(|s| s.as_str());
            let inst_opt = if pinned_ver.is_some() {
                self.get_mod_version(&entry.mod_id, pinned_ver)
            } else {
                by_id.get(&entry.mod_id).map(|m| (*m).clone())
            };
            if let Some(inst) = inst_opt {
                let link_path = profile_mods_dir.join(&entry.mod_id);
                create_dir_junction_or_copy(&inst.install_path, &link_path)?;
                mod_priorities.insert(entry.mod_id.clone(), entry.effective_priority);
                active_mods_map.insert(entry.mod_id.clone(), inst.manifest.version.clone());
                ordered_mod_ids.push(entry.mod_id.clone());
            }
        }

        // 3. Write `Mods/qm/load-order.json` and `Mods/qm/launch-config.json`
        let load_order_plan = LoadOrderPlan {
            profile_id: profile.id.clone(),
            generated_at: jiff::Zoned::now().to_string(),
            enabled_mods: ordered_mod_ids,
            mod_priorities,
            patch_priorities: profile.patch_priorities.clone(),
            disabled_patches: profile.disabled_patches.clone(),
            glue_patches: profile.glue_patches.clone(),
        };
        load_order_plan.save_to_mod_dir(&profile_mods_dir)?;

        let is_vanilla = profile.launch_method == Some(LaunchMethod::Vanilla)
            || profile.enabled_mods.is_empty();
        let launch_cfg = LaunchConfig {
            profile_id: Some(profile.id.clone()),
            profile_name: Some(profile.name.clone()),
            vanilla_mode: is_vanilla,
            disable_console: profile.disable_console || settings.disable_console_on_launch,
            dump_all: false,
            save_identity_override: if profile.isolate_save_identity {
                Some(format!("mnb_{}", profile.linked_save_profile_id))
            } else {
                None
            },
            active_mods: active_mods_map.clone(),
            dev_hot_reload: false,
        };
        launch_cfg.save_to_mod_dir(&profile_mods_dir)?;

        // Overlay any files in `profiles/<profile_id>/overwrite/` into `profiles/<profile_id>/Mods/`
        if overwrite_dir.is_dir() {
            let _ = copy_directory_clean(&overwrite_dir, &profile_mods_dir);
        }

        // Profile staging is isolated to `profiles/<profile_id>/Mods/`.
        // Linking into the live game directory (<game_dir>/Mods and winmm.dll) is performed
        // strictly when launching the game via `launch_game` / `deploy_profile_to_game`,
        // keeping the Steam game install completely vanilla during regular profile management.

        // 4. Swap save profile if needed and sync per-profile `saveModSettings.lua`
        self.switch_save_profile_internal(
            &settings.active_save_profile_id,
            &profile.linked_save_profile_id,
            &active_mods_map,
            settings.save_dir_path.as_deref(),
        )?;

        if let Some(ref live_save_dir) = settings.save_dir_path {
            let prof_mod_settings = profile_dir.join("saveModSettings.lua");
            let live_mod_settings = live_save_dir.join("saveModSettings.lua");
            if prof_mod_settings.is_file() {
                let _ = fs::copy(&prof_mod_settings, &live_mod_settings);
            } else if live_mod_settings.is_file() {
                let _ = fs::copy(&live_mod_settings, &prof_mod_settings);
            }
        }

        settings.active_profile_id = profile.id.clone();
        settings.active_save_profile_id = profile.linked_save_profile_id.clone();
        self.save_settings(&settings)?;
        journal.commit_transaction(&self.journal_file)?;

        Ok(load_order_plan)
    }

    fn switch_save_profile_internal(
        &self,
        from_save_id: &str,
        to_save_id: &str,
        active_mods: &HashMap<String, String>,
        live_save_dir: Option<&Path>,
    ) -> Result<(), String> {
        let Some(live_dir) = live_save_dir else {
            return Ok(());
        };
        let _ = fs::create_dir_all(live_dir);

        // Save outgoing files back to `saves/<from_save_id>/active/`
        let from_active_dir = self.saves_dir.join(from_save_id).join("active");
        let _ = fs::create_dir_all(&from_active_dir);
        for fname in SAVE_FILES {
            let live_file = live_dir.join(fname);
            if live_file.is_file() {
                let _ = fs::copy(&live_file, from_active_dir.join(fname));
            }
        }

        if from_save_id != to_save_id {
            let _ = self.create_save_backup(from_save_id, "Auto-backup before profile switch");
        }

        // Load incoming files from `saves/<to_save_id>/active/` into `%APPDATA%\mnb\`
        let to_active_dir = self.saves_dir.join(to_save_id).join("active");
        let _ = fs::create_dir_all(&to_active_dir);
        for fname in SAVE_FILES {
            let incoming = to_active_dir.join(fname);
            let target_live = live_dir.join(fname);
            if incoming.is_file() {
                let _ = fs::copy(&incoming, &target_live);
            } else if from_save_id != to_save_id && target_live.is_file() {
                let _ = fs::remove_file(&target_live);
            }
        }

        // Update save_profile.json fingerprint
        let mut save_profiles = self.list_save_profiles();
        if let Some(sp) = save_profiles.iter_mut().find(|s| s.id == to_save_id) {
            sp.last_played_at = Some(jiff::Zoned::now().to_string());
            if !active_mods.is_empty() {
                sp.active_mods_fingerprint = active_mods.clone();
            }
            let _ = self.save_save_profile(sp);
        }

        Ok(())
    }

    /// Checks whether `<game_dir>/Mods` or `<game_dir>/winmm.dll` exists in the game directory.
    pub fn is_game_dir_modded(&self) -> bool {
        let settings = self.load_settings();
        let Some(ref game_exe) = settings.game_exe_path else {
            return false;
        };
        let Some(game_dir) = game_exe.parent() else {
            return false;
        };
        game_dir.join("Mods").exists() || game_dir.join("winmm.dll").is_file()
    }

    /// Deploys the active profile to the physical game directory.
    /// Creates the `<game_dir>/Mods` junction pointing to `profiles/<profile_id>/Mods/`
    /// and ensures `winmm.dll` is present in the game directory.
    pub fn deploy_profile_to_game(&self, profile_id: &str) -> Result<(), String> {
        let settings = self.load_settings();
        let game_exe = settings
            .game_exe_path
            .as_ref()
            .ok_or_else(|| "Mud and Blood executable (mnb.exe) path is not configured.".to_string())?;
        let game_dir = game_exe
            .parent()
            .ok_or_else(|| "Invalid game directory".to_string())?;

        let profile_dir = self.profiles_dir.join(profile_id);
        let profile_mods_dir = profile_dir.join("Mods");
        if !profile_mods_dir.is_dir() {
            let _ = self.activate_profile(profile_id)?;
        }

        let game_mods_dir = game_dir.join("Mods");
        if game_mods_dir.exists() {
            if !is_reparse_point(&game_mods_dir) {
                let bak_name = format!(
                    "pre-qm-mods-{}",
                    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S")
                );
                let bak_dest = self.backups_dir.join(bak_name);
                let _ = copy_directory_clean(&game_mods_dir, &bak_dest);
            }
            remove_junction_or_dir(&game_mods_dir)?;
        }

        create_dir_junction_or_copy(&profile_mods_dir, &game_mods_dir)?;
        ensure_winmm_dll_installed(game_dir);
        Ok(())
    }

    /// Restores the game directory to a completely clean, unmodded vanilla state.
    /// Removes the `<game_dir>/Mods` junction and `winmm.dll`.
    pub fn restore_clean_game_dir(&self) -> Result<(), String> {
        let settings = self.load_settings();
        let Some(ref game_exe) = settings.game_exe_path else {
            return Ok(());
        };
        let Some(game_dir) = game_exe.parent() else {
            return Ok(());
        };

        let game_mods_dir = game_dir.join("Mods");
        if game_mods_dir.exists() {
            if is_reparse_point(&game_mods_dir) {
                let _ = remove_junction_or_dir(&game_mods_dir);
            } else {
                let bak_name = format!(
                    "unlinked-mods-{}",
                    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S")
                );
                let bak_dest = self.backups_dir.join(bak_name);
                let _ = copy_directory_clean(&game_mods_dir, &bak_dest);
                let _ = fs::remove_dir_all(&game_mods_dir);
            }
        }

        let winmm = game_dir.join("winmm.dll");
        if winmm.is_file() {
            let _ = fs::remove_file(&winmm);
        }

        Ok(())
    }

    /// Activates `profile_id` and launches Mud and Blood using the specified or profile-default `LaunchMethod`.
    pub fn launch_game(
        &self,
        profile_id: &str,
        method_override: Option<LaunchMethod>,
    ) -> Result<u32, String> {
        let _plan = self.activate_profile(profile_id)?;
        let settings = self.load_settings();
        let profile = self.get_profile(profile_id)?;

        let method = method_override
            .or(profile.launch_method)
            .unwrap_or(settings.default_launch_method);

        let game_exe = settings
            .game_exe_path
            .ok_or_else(|| "Mud and Blood executable (mnb.exe) path is not configured.".to_string())?;
        let game_dir = game_exe
            .parent()
            .ok_or_else(|| "Invalid game directory".to_string())?;

        let is_vanilla = method == LaunchMethod::Vanilla
            || profile.launch_method == Some(LaunchMethod::Vanilla)
            || profile.enabled_mods.is_empty();

        if is_vanilla {
            self.restore_clean_game_dir()?;
        } else {
            self.deploy_profile_to_game(profile_id)?;
        }

        // Auto-backup saves before launch when enabled (Section 5)
        if settings.auto_backup_saves {
            let _ = self.create_save_backup(
                &profile.linked_save_profile_id,
                "Auto-backup before game launch",
            );
        }

        // Ensure `steam_appid.txt` (`1391530`) exists when launching `mnb.exe` directly so Steam API initializes cleanly
        if method != LaunchMethod::SteamUrl {
            let appid_file = game_dir.join("steam_appid.txt");
            let needs_write = match fs::read_to_string(&appid_file) {
                Ok(content) => !content.contains("1391530"),
                Err(_) => true,
            };
            if needs_write {
                let _ = fs::write(&appid_file, "1391530\n");
            }
        }

        match method {
            LaunchMethod::SteamUrl => {
                if crate::storage::is_steam_running() {
                    Command::new("cmd")
                        .args(["/C", "start", "", "steam://rungameid/1391530"])
                        .spawn()
                        .map(|child| child.id())
                        .map_err(|e| format!("Failed to launch via Steam URL: {e}"))
                } else if let Some(steam_exe) = crate::storage::detect_steam_exe() {
                    Command::new(&steam_exe)
                        .arg("steam://rungameid/1391530")
                        .spawn()
                        .map(|child| child.id())
                        .map_err(|e| format!("Failed to launch Steam at {}: {e}", steam_exe.display()))
                } else {
                    Command::new("cmd")
                        .args(["/C", "start", "", "steam://rungameid/1391530"])
                        .spawn()
                        .map(|child| child.id())
                        .map_err(|e| format!("Failed to launch via Steam URL: {e}"))
                }
            }
            LaunchMethod::DirectExe | LaunchMethod::Vanilla => {
                let mut cmd = Command::new(&game_exe);
                cmd.current_dir(game_dir);
                if method == LaunchMethod::Vanilla {
                    cmd.arg("--vanilla");
                }
                if profile.disable_console || settings.disable_console_on_launch {
                    cmd.arg("--disable-console");
                }
                for arg in &profile.custom_launch_args {
                    cmd.arg(arg);
                }
                cmd.spawn()
                    .map(|child| child.id())
                    .map_err(|e| format!("Failed to launch {}: {e}", game_exe.display()))
            }
        }
    }
}

/// Creates an unprivileged Windows Directory Junction (`mklink /J "<link>" "<target>"`),
/// falling back to a recursive directory copy if junctions are unsupported on the filesystem.
pub fn create_dir_junction_or_copy(target: &Path, link: &Path) -> Result<(), String> {
    let abs_target = dunce::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    if let Some(parent) = link.parent() {
        let _ = fs::create_dir_all(parent);
    }

    #[cfg(windows)]
    {
        let status = Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.to_string_lossy(),
                &abs_target.to_string_lossy(),
            ])
            .output();
        if let Ok(out) = status {
            if out.status.success() {
                return Ok(());
            }
        }
    }

    copy_directory_clean(&abs_target, link)
}

/// Safely removes either a Windows Directory Junction (without deleting target contents!)
/// or a regular directory (first unlinking any child reparse points inside it).
pub fn remove_junction_or_dir(path: &Path) -> Result<(), String> {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    if is_reparse_point(path) {
        // On Windows, `fs::remove_dir` removes a directory junction without touching the target!
        fs::remove_dir(path)
            .or_else(|_| fs::remove_dir_all(path))
            .map_err(|e| format!("Failed to remove junction {}: {e}", path.display()))
    } else {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let child = entry.path();
                if is_reparse_point(&child) {
                    let _ = fs::remove_dir(&child);
                }
            }
        }
        fs::remove_dir_all(path)
            .map_err(|e| format!("Failed to remove dir {}: {e}", path.display()))
    }
}

pub fn is_reparse_point(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(meta) = fs::symlink_metadata(path) {
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
            return (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
        }
    }
    false
}

fn find_source_winmm_dll() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("Quartermaster/target/release/winmm.dll"));
        candidates.push(cwd.join("target/release/winmm.dll"));
        candidates.push(cwd.join("Quartermaster/target/release/qm_win.dll"));
        candidates.push(cwd.join("target/release/qm_win.dll"));
        candidates.push(cwd.join("Quartermaster/winmm.dll"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent() {
            candidates.push(p.join("winmm.dll"));
            candidates.push(p.join("qm_win.dll"));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn ensure_winmm_dll_installed(game_dir: &Path) {
    let dest = game_dir.join("winmm.dll");
    if let Some(src) = find_source_winmm_dll() {
        let _ = fs::copy(src, dest);
    }
}

impl QuarterMasterPaths {
    /// Checks `winmm.dll` installation & SHA-256 integrity, whether `mnb.exe` is running,
    /// and reads `Mods/qm/patch-report.json` + recent runtime logs.
    pub fn check_system_health(&self) -> serde_json::Value {
        let settings = self.load_settings();
        let mut dll_present = false;
        let mut dll_sha256 = None;
        let mut dll_matches_build = true;
        let mut patch_report_val = serde_json::Value::Null;
        let mut recent_logs: Vec<serde_json::Value> = Vec::new();

        let source_dll_sha = find_source_winmm_dll()
            .and_then(|p| fs::read(p).ok())
            .map(|b| qm_schema::sha256_hex(&b));

        if let Some(ref exe) = settings.game_exe_path {
            if let Some(game_dir) = exe.parent() {
                let game_dll = game_dir.join("winmm.dll");
                if let Ok(bytes) = fs::read(&game_dll) {
                    dll_present = true;
                    let sha = qm_schema::sha256_hex(&bytes);
                    if let Some(ref src_sha) = source_dll_sha {
                        dll_matches_build = &sha == src_sha;
                    }
                    dll_sha256 = Some(sha);
                }

                let pr_path = game_dir.join("Mods").join("qm").join("patch-report.json");
                if let Ok(raw) = fs::read_to_string(&pr_path) {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                        patch_report_val = v;
                    }
                }

                let log_dir = game_dir.join("Mods").join("qm").join("log");
                if let Ok(entries) = fs::read_dir(&log_dir) {
                    let mut log_files: Vec<PathBuf> = entries
                        .filter_map(|e| e.ok().map(|x| x.path()))
                        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("log"))
                        .collect();
                    log_files.sort();
                    log_files.reverse();
                    if let Some(latest_log) = log_files.first() {
                        if let Ok(raw_log) = fs::read_to_string(latest_log) {
                            for line in raw_log.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev() {
                                let level = if line.contains("ERROR") || line.contains("[ERR]") {
                                    "ERROR"
                                } else if line.contains("WARN") {
                                    "WARN"
                                } else {
                                    "INFO"
                                };
                                recent_logs.push(serde_json::json!({
                                    "level": level,
                                    "line": line,
                                    "file": latest_log.file_name().and_then(|n| n.to_str()).unwrap_or("qm.log"),
                                }));
                            }
                        }
                    }
                }
            }
        }

        let game_running_pid = is_game_running();
        let game_update_detected = self.check_game_update_occurred();
        let overwrite_files = self.list_overwrite_files(&settings.active_profile_id);
        let steam_cloud_warning = detect_steam_cloud_warning(&settings);

        let is_modded = self.is_game_dir_modded();
        let source_ready = source_dll_sha.is_some();

        serde_json::json!({
            "dll_present": dll_present,
            "is_game_dir_modded": is_modded,
            "source_dll_ready": source_ready,
            "dll_sha256": dll_sha256,
            "source_dll_sha256": source_dll_sha,
            "dll_matches_build": dll_matches_build,
            "game_running_pid": game_running_pid,
            "game_update_detected": game_update_detected.map(|(old_sha, new_sha)| serde_json::json!({
                "previous_sha256": old_sha,
                "current_sha256": new_sha,
            })),
            "overwrite_files": overwrite_files,
            "steam_cloud_warning": steam_cloud_warning,
            "patch_report": patch_report_val,
            "recent_logs": recent_logs,
        })
    }

    /// Lists relative file paths inside the MO2-style `profiles/<profile_id>/overwrite/` bucket (Section 1).
    pub fn list_overwrite_files(&self, profile_id: &str) -> Vec<String> {
        let overwrite_dir = self.profiles_dir.join(profile_id).join("overwrite");
        let mut files = Vec::new();
        if !overwrite_dir.is_dir() {
            return files;
        }
        for entry in walkdir::WalkDir::new(&overwrite_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().is_file() {
                if let Ok(rel) = entry.path().strip_prefix(&overwrite_dir) {
                    files.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        files.sort();
        files
    }

    /// Clears all captured files in `profiles/<profile_id>/overwrite/`.
    pub fn clear_overwrite_files(&self, profile_id: &str) -> Result<usize, String> {
        let files = self.list_overwrite_files(profile_id);
        let count = files.len();
        let overwrite_dir = self.profiles_dir.join(profile_id).join("overwrite");
        if overwrite_dir.is_dir() {
            let _ = fs::remove_dir_all(&overwrite_dir);
            let _ = fs::create_dir_all(&overwrite_dir);
        }
        Ok(count)
    }

    /// Re-installs / repairs `winmm.dll` in the game directory if missing or quarantined.
    pub fn repair_winmm_dll(&self) -> Result<String, String> {
        let settings = self.load_settings();
        let exe = settings
            .game_exe_path
            .ok_or_else(|| "Game executable path is not configured".to_string())?;
        let game_dir = exe.parent().ok_or_else(|| "Invalid game dir".to_string())?;
        let src = find_source_winmm_dll()
            .ok_or_else(|| "Could not locate source winmm.dll build artifact".to_string())?;
        let dest = game_dir.join("winmm.dll");
        fs::copy(&src, &dest)
            .map_err(|e| format!("Failed to copy winmm.dll to {}: {e}", dest.display()))?;
        let bytes = fs::read(&dest).map_err(|e| e.to_string())?;
        Ok(qm_schema::sha256_hex(&bytes))
    }
}

fn list_backups_in_dir(backups_dir: &Path) -> Vec<SaveBackupEntry> {
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(backups_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let meta_path = entry.path().join("backup_meta.json");
            if let Ok(raw) = fs::read_to_string(&meta_path) {
                if let Ok(b) = serde_json::from_str::<SaveBackupEntry>(&raw) {
                    list.push(b);
                }
            }
        }
    }
    list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    list
}

/// Checks whether `mnb.exe` is currently running on Windows and returns its PID if active (Section 5 & 6).
pub fn is_game_running() -> Option<u32> {
    #[cfg(windows)]
    {
        if let Ok(out) = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq mnb.exe", "/FO", "CSV", "/NH"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.to_ascii_lowercase().contains("\"mnb.exe\"") {
                    let parts: Vec<&str> = line.split(',').collect();
                    if parts.len() >= 2 {
                        let pid_str = parts[1].trim().trim_matches('"');
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            return Some(pid);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Captures any untracked runtime-generated files inside `profile_mods_dir` (outside mod junctions and `qm/`)
/// into the profile's MO2 `overwrite/` bucket before `Mods/` is rebuilt.
fn capture_untracked_into_overwrite(profile_mods_dir: &Path, overwrite_dir: &Path) {
    let Ok(entries) = fs::read_dir(profile_mods_dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let p = entry.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "qm" || is_reparse_point(&p) {
            continue;
        }
        let dest = overwrite_dir.join(name);
        if p.is_file() {
            let _ = fs::create_dir_all(overwrite_dir);
            let _ = fs::copy(&p, &dest);
        } else if p.is_dir() {
            let _ = copy_directory_clean(&p, &dest);
        }
    }
}

/// Checks whether Steam Cloud `remotecache.vdf` for AppID `1391530` exists and may overwrite swapped saves (Section 5).
fn detect_steam_cloud_warning(settings: &crate::storage::LauncherSettings) -> Option<String> {
    let steam_userdata = PathBuf::from(r"C:\Program Files (x86)\Steam\userdata");
    if steam_userdata.is_dir() {
        if let Ok(users) = fs::read_dir(&steam_userdata) {
            for user in users.filter_map(|e| e.ok()) {
                let app_cloud = user.path().join("1391530").join("remotecache.vdf");
                if app_cloud.is_file() {
                    return Some(format!(
                        "Steam Cloud sync is active for Mud and Blood ({}). QuarterMaster automatically backs up saves before swaps, or you can enable 'Isolate Save Identity' per profile.",
                        app_cloud.display()
                    ));
                }
            }
        }
    }
    let _ = settings;
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_clone_profile() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));

        // Create base profile
        let prof = paths
            .create_profile("Tactical Realism", None, Some("default"), Some("Test description"))
            .expect("create_profile should succeed");
        assert_eq!(prof.id, "tactical_realism");
        assert_eq!(prof.name, "Tactical Realism");
        assert_eq!(prof.linked_save_profile_id, "default");
        assert_eq!(prof.description, "Test description");

        // Clone profile
        let cloned = paths
            .create_profile("Tactical Hardcore", Some("tactical_realism"), Some("slot_2"), None)
            .expect("cloning profile should succeed");
        assert_eq!(cloned.id, "tactical_hardcore");
        assert_eq!(cloned.linked_save_profile_id, "slot_2");
        assert_eq!(cloned.name, "Tactical Hardcore");
    }

    #[test]
    fn test_create_profile_guards() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));

        // Whitespace name guard
        let err = paths.create_profile("   ", None, None, None);
        assert!(err.is_err());

        // Explicit duplicate ID guard
        let p1 = paths.create_profile_with_id(Some("unique_id"), "Profile 1", None, None, None);
        assert!(p1.is_ok());
        let p2 = paths.create_profile_with_id(Some("unique_id"), "Profile 2", None, None, None);
        assert!(p2.is_err());
    }

    #[test]
    fn test_rename_profile_and_vanilla_guard() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));

        let prof = paths
            .create_profile("Initial Name", None, None, None)
            .unwrap();

        // Rename success
        let renamed = paths
            .rename_profile(&prof.id, "Updated Name")
            .expect("rename should succeed");
        assert_eq!(renamed.name, "Updated Name");

        // Reload from disk to verify persistence
        let loaded = paths.get_profile(&prof.id).unwrap();
        assert_eq!(loaded.name, "Updated Name");

        // Vanilla rename guard
        let vanilla_err = paths.rename_profile("vanilla", "Hacked Vanilla");
        assert!(vanilla_err.is_err());

        // Empty name guard
        let empty_err = paths.rename_profile(&prof.id, "   ");
        assert!(empty_err.is_err());
    }

    #[test]
    fn test_delete_profile_and_guards() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));

        let prof = paths
            .create_profile("To Delete", None, None, None)
            .unwrap();
        assert!(paths.get_profile(&prof.id).is_ok());

        // Active profile guard
        let active_err = paths.delete_profile(&prof.id, &prof.id);
        assert!(active_err.is_err());

        // Vanilla delete guard
        let vanilla_err = paths.delete_profile("vanilla", "default");
        assert!(vanilla_err.is_err());

        // Nonexistent guard
        let nonexist_err = paths.delete_profile("does_not_exist_xyz", "default");
        assert!(nonexist_err.is_err());

        // Successful delete
        let deleted = paths
            .delete_profile(&prof.id, "other_active")
            .expect("delete should succeed");
        assert_eq!(deleted, prof.id);
        assert!(paths.get_profile(&prof.id).is_err());
        assert!(!paths.profiles_dir.join(&prof.id).exists());
    }

    #[test]
    fn test_delete_profile_traversal_rejection() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));

        // Create an external directory outside profiles_dir that must never be deleted
        let canary_dir = temp.path().join("canary_dir");
        fs::create_dir_all(&canary_dir).unwrap();
        assert!(canary_dir.is_dir());

        let traversal_payloads = [
            "../canary_dir",
            "..",
            "../..",
            "..\\canary_dir",
            "C:\\Windows",
            "foo/bar",
            "foo\\bar",
            "colon:test",
            "has space",
            " leading",
            "trailing ",
            "   ",
            "",
            ".",
        ];

        for payload in traversal_payloads {
            let res = paths.delete_profile(payload, "default");
            assert!(
                res.is_err(),
                "delete_profile must reject payload '{}', got {:?}",
                payload,
                res
            );
        }

        // Verify canary_dir was NOT deleted
        assert!(
            canary_dir.is_dir(),
            "Canary directory outside profiles_dir must not be deleted"
        );

        // Verify that a directory inside profiles_dir lacking profile.json is not deleted
        let rogue_dir = paths.profiles_dir.join("rogue_dir");
        fs::create_dir_all(&rogue_dir).unwrap();
        let rogue_res = paths.delete_profile("rogue_dir", "default");
        assert!(
            rogue_res.is_err(),
            "delete_profile must reject directory without profile.json"
        );
        assert!(rogue_res.unwrap_err().contains("not found"));
        assert!(
            rogue_dir.is_dir(),
            "Rogue directory without profile.json must remain untouched"
        );
    }

    #[test]
    fn test_rename_profile_traversal_rejection() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));

        let traversal_payloads = [
            "../canary_dir",
            "..",
            "../..",
            "..\\canary_dir",
            "C:\\Windows",
            "foo/bar",
            "foo\\bar",
            "colon:test",
            "has space",
            " leading",
            "trailing ",
            "   ",
            "",
            ".",
        ];

        for payload in traversal_payloads {
            let res = paths.rename_profile(payload, "New Name");
            assert!(
                res.is_err(),
                "rename_profile must reject payload '{}', got {:?}",
                payload,
                res
            );
        }

        // Verify directory inside profiles_dir lacking profile.json returns not found
        let rogue_dir = paths.profiles_dir.join("rogue_dir_rename");
        fs::create_dir_all(&rogue_dir).unwrap();
        let rogue_res = paths.rename_profile("rogue_dir_rename", "New Name");
        assert!(
            rogue_res.is_err(),
            "rename_profile must reject directory without profile.json"
        );
        assert!(rogue_res.unwrap_err().contains("not found"));
    }
}

