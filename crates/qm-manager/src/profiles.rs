use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use qm_core::handshake::{GluePatchEntry, LaunchConfig, LoadOrderPlan, StackMode};
use qm_core::lenient::Lenient;
use qm_core::patch::Priority;
use qm_schema::{ModManifest, ModSettingsStore, SaveSummary};

use crate::library::{copy_directory_clean, copy_directory_skipping, InstalledMod};
use crate::solver::{resolve_load_order, LoadOrderResolution};
use crate::storage::{LaunchMethod, QuarterMasterPaths, StateJournal};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSeparator {
    pub id: String,
    pub title: String,
    pub color: String,
    pub insert_before_mod_id: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
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
    /// Profile patch order rules, written to `load-order.json` for the engine.
    #[serde(default)]
    pub patch_order: Vec<qm_core::patch::order::OrderRule>,
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
    /// How code from several mods at one line is ordered. `None`: a profile from before
    /// stacking that has not seen the stacking notice; it keeps the older order (D3).
    #[serde(default)]
    pub stacking: Option<StackMode>,
    /// One-time notices the user has seen in this profile (`stacking_v1`, `stacking_v2`,
    /// `fixes_import_v1`).
    #[serde(default)]
    pub notices_seen: Vec<String>,
    /// The Phase D line rules (D50, D51). `None`: a profile from before them, which keeps the
    /// older rules until the user answers the `stacking_v2` notice. New profiles: `Some(true)`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub line_rules_v2: Option<bool>,
    /// Side (`allied`, `axis`) -> the faction picked for it (D82). Set only when two enabled
    /// factions replace one side; without it the mod priority decides, as before.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub faction_picks: BTreeMap<String, String>,
    /// The fix ledger (§3.11). Entries of a kind this version does not know round-trip as
    /// they are.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub fixes: Vec<Lenient<crate::fixes::ProfileFix>>,
    /// 0: the legacy fields are not migrated to the ledger yet (D38).
    #[serde(default)]
    pub fixes_version: u32,
    /// D11; used only if safe follows are ever applied automatically.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub auto_follow: bool,
    /// Batch operations that can be undone as one step, newest last (at most 50). Entries
    /// this version cannot read (a fix kind it does not know) round-trip as they are.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub fix_journal: Vec<Lenient<crate::fixes::JournalEntry>>,
    /// Why the last migration to the ledger stopped (D38).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub migration_error: Option<String>,
}

/// The stacking notice (D3).
pub const NOTICE_STACKING: &str = "stacking_v1";

/// The line rules notice (D51): shown only when the newer rules change the profile's output.
pub const NOTICE_LINE_RULES: &str = "stacking_v2";

/// The notices `notice_dismiss` accepts.
pub const KNOWN_NOTICES: [&str; 3] = [
    NOTICE_STACKING,
    NOTICE_LINE_RULES,
    crate::fixes::NOTICE_FIXES_IMPORT,
];

/// The notices a profile made after Phase D has already seen (D40, D51).
pub fn new_profile_notices() -> Vec<String> {
    vec![NOTICE_STACKING.to_string(), NOTICE_LINE_RULES.to_string()]
}

impl ModProfile {
    /// The stacking mode the plan gets: the older order until the user picks one (D3).
    pub fn stack_mode(&self) -> StackMode {
        self.stacking.unwrap_or(StackMode::Legacy)
    }

    /// Whether the plan gets the Phase D line rules: only when the profile says so (D51).
    pub fn line_rules(&self) -> bool {
        self.line_rules_v2 == Some(true)
    }

    pub fn stacking_acknowledged(&self) -> bool {
        self.notice_seen(NOTICE_STACKING)
    }

    pub fn notice_seen(&self, key: &str) -> bool {
        self.notices_seen.iter().any(|k| k == key)
    }

    /// Marks `key` seen; false when it already was.
    pub fn mark_notice_seen(&mut self, key: &str) -> bool {
        if self.notice_seen(key) {
            return false;
        }
        self.notices_seen.push(key.to_string());
        true
    }
}

fn default_save_id() -> String {
    "default".to_string()
}

/// Id of the built-in Vanilla profile and of its save slot (they always go together).
pub const VANILLA_ID: &str = "vanilla";

/// The built-in Vanilla profile: no mods, ever, always on the Vanilla save slot. Its definition
/// comes from here, never from disk; only the timestamps of an existing file are kept.
pub fn builtin_vanilla_profile(existing: Option<&ModProfile>) -> ModProfile {
    let now = jiff::Zoned::now().to_string();
    ModProfile {
        id: VANILLA_ID.to_string(),
        name: "Vanilla".to_string(),
        description: "The game as released, with no mods. Built in.".to_string(),
        created_at: existing.map_or_else(|| now.clone(), |p| p.created_at.clone()),
        updated_at: existing.map_or(now, |p| p.updated_at.clone()),
        enabled_mods: Vec::new(),
        separators: Vec::new(),
        mod_priorities: HashMap::new(),
        user_load_after: HashMap::new(),
        patch_priorities: HashMap::new(),
        disabled_patches: Vec::new(),
        glue_patches: Vec::new(),
        patch_order: Vec::new(),
        linked_save_profile_id: VANILLA_ID.to_string(),
        launch_method: None,
        disable_console: false,
        custom_launch_args: Vec::new(),
        isolate_save_identity: false,
        pinned_versions: HashMap::new(),
        locked_mods: Vec::new(),
        favorites: Vec::new(),
        mod_notes: HashMap::new(),
        // No mods, so no stacking notice to show.
        stacking: Some(StackMode::Dependencies),
        notices_seen: new_profile_notices(),
        line_rules_v2: Some(true),
        fixes: Vec::new(),
        fixes_version: crate::fixes::FIXES_VERSION,
        auto_follow: false,
        fix_journal: Vec::new(),
        migration_error: None,
        faction_picks: BTreeMap::new(),
    }
}

/// The built-in Vanilla save slot: always marked vanilla-clean. Timestamps and backups of an
/// existing slot are kept; everything else is canonical.
pub fn builtin_vanilla_slot(existing: Option<&SaveProfile>) -> SaveProfile {
    SaveProfile {
        id: VANILLA_ID.to_string(),
        name: "Vanilla".to_string(),
        is_vanilla_clean: true,
        tags: SaveTags::default(),
        locked_vanilla: true,
        builtin: true,
        created_at: existing.map_or_else(|| jiff::Zoned::now().to_string(), |s| s.created_at.clone()),
        last_played_at: existing.and_then(|s| s.last_played_at.clone()),
        active_mods_fingerprint: HashMap::new(),
        backups: existing.map(|s| s.backups.clone()).unwrap_or_default(),
    }
}

/// Sticky facts about a save slot (docs/specs/save-tags.md): once set, nothing in the launcher
/// clears them, so removing mods cannot launder a modded save.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveTags {
    /// Launched with mods at least once, or the save carries a `_qm_meta` stamp.
    #[serde(default)]
    pub modded: bool,
    /// Changed by the save editor (reserved; nothing sets it yet).
    #[serde(default)]
    pub edited: bool,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveProfile {
    pub id: String,
    pub name: String,
    /// Legacy name of `locked_vanilla`; kept in step for older readers.
    #[serde(default)]
    pub is_vanilla_clean: bool,
    #[serde(default)]
    pub tags: SaveTags,
    /// Locked as vanilla: no launch with mods may use it.
    #[serde(default)]
    pub locked_vanilla: bool,
    /// The built-in Vanilla slot.
    #[serde(default)]
    pub builtin: bool,
    pub created_at: String,
    pub last_played_at: Option<String>,
    #[serde(default)]
    pub active_mods_fingerprint: HashMap<String, String>,
    #[serde(default)]
    pub backups: Vec<SaveBackupEntry>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveBackupEntry {
    pub id: String,
    pub timestamp: String,
    pub label: String,
    pub path: PathBuf,
    pub size_bytes: u64,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDiff {
    pub profile_a_id: String,
    pub profile_b_id: String,
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
    pub priority_diffs: Vec<(String, i32, i32)>,
    pub setting_diffs: Vec<(String, String, String, String)>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveCompatibilityReport {
    pub is_safe: bool,
    pub missing_mods_from_save: Vec<(String, String)>,
    pub is_tainting_vanilla_save: bool,
    pub warning_message: Option<String>,
}

/// What a successful activation returns: the plan it wrote and notices for the user.
#[derive(Debug, Clone)]
pub struct Activation {
    pub plan: LoadOrderPlan,
    /// For example "overwrite/qm/load-order.json is ignored".
    pub warnings: Vec<String>,
}

/// Why an activation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivateError {
    /// The staged load-order.json did not read back cleanly; the game would load no mods.
    InvalidPlan(String),
    Failed(String),
}

impl std::fmt::Display for ActivateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivateError::InvalidPlan(m) | ActivateError::Failed(m) => f.write_str(m),
        }
    }
}

impl From<String> for ActivateError {
    fn from(m: String) -> Self {
        ActivateError::Failed(m)
    }
}

/// Files in the staged Mods folder that the launcher writes; the overwrite bucket never
/// replaces them.
const LAUNCHER_OWNED: [&str; 2] = ["qm/load-order.json", "qm/launch-config.json"];

fn is_launcher_owned(rel: &Path) -> bool {
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect::<Vec<_>>()
        .join("/");
    LAUNCHER_OWNED.contains(&rel.as_str())
}

#[cfg(any(test, feature = "test-seams"))]
thread_local! {
    /// Test seam: break the staged plan after the overlay, as a hand edit or a bad disk would.
    static CORRUPT_PLAN_AFTER_OVERLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test seam for other crates' tests: break the staged plan after the overlay on this thread.
#[cfg(any(test, feature = "test-seams"))]
#[doc(hidden)]
pub fn set_corrupt_plan_after_overlay(on: bool) {
    CORRUPT_PLAN_AFTER_OVERLAY.with(|c| c.set(on));
}

#[cfg(any(test, feature = "test-seams"))]
thread_local! {
    /// Test seam: break only the next staged plan, so a later restage works.
    static CORRUPT_NEXT_PLAN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test seam for other crates' tests: break only the next staged plan on this thread.
#[cfg(any(test, feature = "test-seams"))]
#[doc(hidden)]
pub fn corrupt_next_staged_plan() {
    CORRUPT_NEXT_PLAN.with(|c| c.set(true));
}

/// Reads the staged plan and launch config back: any error or skipped entry is an error. A
/// launch config the game cannot read would start it with no mods (D65).
fn check_staged_plan(mods_dir: &Path) -> Result<(), String> {
    let read = LoadOrderPlan::read(mods_dir);
    if let Some(e) = read.error {
        return Err(e.to_string());
    }
    if read.plan.is_none() {
        return Err("load-order.json is missing after it was written".to_string());
    }
    if !read.warnings.is_empty() {
        return Err(read.warnings.join("; "));
    }
    match LaunchConfig::load_from_mod_dir(mods_dir)? {
        Some(_) => Ok(()),
        None => Err("launch-config.json is missing after it was written".to_string()),
    }
}

/// The JSON of `Mods/qm/patch-report.json` in `game_dir`: Null when it is missing, and why
/// when it cannot be read or is not JSON.
fn read_patch_report_json(game_dir: &Path) -> (serde_json::Value, Option<String>) {
    let path = game_dir.join("Mods").join("qm").join("patch-report.json");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (serde_json::Value::Null, None),
        Err(e) => return (serde_json::Value::Null, Some(format!("patch-report.json could not be read: {e}"))),
    };
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => (v, None),
        Err(e) => (serde_json::Value::Null, Some(format!("patch-report.json is not valid: {e}"))),
    }
}

/// `profile` with no mods and none of the plan entries that act on mods.
fn no_mods(profile: &ModProfile) -> ModProfile {
    ModProfile {
        enabled_mods: Vec::new(),
        disabled_patches: Vec::new(),
        glue_patches: Vec::new(),
        patch_order: Vec::new(),
        patch_priorities: HashMap::new(),
        ..profile.clone()
    }
}

/// The plan the launcher writes for `profile`, with the resolved mod order and priorities
/// (§7.3): `fixes::compile_plan`. Pure apart from the timestamp.
pub fn build_plan(
    profile: &ModProfile,
    enabled_mods: Vec<String>,
    mod_priorities: HashMap<String, Priority>,
) -> LoadOrderPlan {
    crate::fixes::compile_plan(profile, enabled_mods, mod_priorities)
}

/// The solver run activation does for `profile`: its enabled mods' manifests, found in
/// `manifests` by id (a later one with the same id wins, as in the installed list), with the
/// profile's priorities and user load-after rules.
pub fn resolve_profile_order(
    profile: &ModProfile,
    manifests: &[ModManifest],
) -> LoadOrderResolution {
    let by_id: HashMap<&str, &ModManifest> =
        manifests.iter().map(|m| (m.id.as_str(), m)).collect();
    let enabled: Vec<ModManifest> = profile
        .enabled_mods
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).map(|m| (*m).clone()))
        .collect();
    resolve_load_order(
        &enabled,
        &profile.mod_priorities,
        &profile.user_load_after,
        Some("0.5.3b"),
    )
}

/// `build_plan` with the mod order and priorities the solver gives for `manifests` (§8.2):
/// the plan activation would write, without staging anything.
pub fn build_plan_for(profile: &ModProfile, manifests: &[ModManifest]) -> LoadOrderPlan {
    let resolution = resolve_profile_order(profile, manifests);
    let enabled = resolution.ordered_mods.iter().map(|e| e.mod_id.clone()).collect();
    let priorities = resolution
        .ordered_mods
        .iter()
        .map(|e| (e.mod_id.clone(), e.effective_priority))
        .collect();
    build_plan(profile, enabled, priorities)
}

/// What activation may use beyond the profile.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivateExtras<'a> {
    /// The cached vanilla Lua files, for the migration's text guard (§7.4).
    pub vanilla: Option<&'a HashMap<String, String>>,
    /// A trial run (D68): staged with these changes on a copy of the profile. The profile
    /// file is not written and not migrated.
    pub trial: Option<&'a crate::runs::TrialChanges>,
}

/// The folder name of a profile.json, as shown in notices.
fn profile_dir_name(p_file: &Path) -> String {
    p_file
        .parent()
        .and_then(|d| d.file_name())
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Renames `file` to `<name>.unreadable-<n>` (the first free n) and returns the new path.
fn set_aside(file: &Path) -> std::io::Result<PathBuf> {
    let name = file.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let mut n = 1;
    let backup = loop {
        let candidate = file.with_file_name(format!("{name}.unreadable-{n}"));
        if !candidate.exists() {
            break candidate;
        }
        n += 1;
    };
    fs::rename(file, &backup).map(|()| backup)
}

/// Reads a built-in Vanilla file: `Ok(None)` when it is missing, `Err` when it cannot be read
/// or parsed.
fn read_builtin<T: serde::de::DeserializeOwned>(file: &Path) -> Result<Option<T>, String> {
    match fs::read_to_string(file) {
        Ok(raw) => serde_json::from_str(&raw).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Sets an unreadable built-in Vanilla file aside before it is rewritten.
fn set_aside_builtin(file: &Path, why: &str) -> Result<(), String> {
    match set_aside(file) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!(
            "{} could not be read ({why}) and could not be set aside: {e}",
            file.display()
        )),
    }
}

/// Copies `src` to `dst` and checks that every byte arrived.
fn copy_whole(src: &Path, dst: &Path) -> std::io::Result<()> {
    let copied = fs::copy(src, dst)?;
    let len = fs::metadata(src)?.len();
    if copied != len {
        return Err(std::io::Error::other(format!(
            "copied {copied} of {len} bytes"
        )));
    }
    Ok(())
}

/// Writes `path` through a temp file of its own (`<name>.<pid>-<n>.tmp`): write, flush to
/// disk, then rename over the old file. Two writers at once never share a temp file, so
/// neither can fail on the other's rename or leave an interleaved file.
fn write_unique_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".{}-{seq}.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let written = fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(data)?;
        f.flush()?;
        f.sync_all()
    });
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to write {}: {e}", tmp.display()));
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to replace {}: {e}", path.display())
    })
}

/// Renames an unreadable profile.json to `profile.json.unreadable-<n>` and says so.
fn back_up_unreadable(p_file: &Path, why: &str) -> String {
    let shown = profile_dir_name(p_file);
    match set_aside(p_file) {
        Ok(backup) => format!(
            "The profile \"{shown}\" could not be read ({why}). It was kept as {} and is not listed.",
            backup.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
        ),
        Err(e) => format!(
            "The profile \"{shown}\" could not be read ({why}) and could not be set aside ({e}). It is not listed."
        ),
    }
}

const SAVE_FILES: &[&str] = &[
    "saveMNB3.lua",
    "saveMNB2.lua",
    "saveMNBclassic.lua",
    "backUp.lua",
    // The copy QuarterMaster keeps of each save before the game overwrites it (save_guard.lua).
    "saveMNB3.lua.prev",
    "saveMNB2.lua.prev",
    "saveMNBclassic.lua.prev",
    "mod_settings.lua",
    "saveModSettings.lua",
    "save_meta.json",
];

impl QuarterMasterPaths {
    /// The profiles on disk. Notices about unreadable files wait for `take_profile_notices`.
    pub fn list_profiles(&self) -> Vec<ModProfile> {
        self.scan_profiles()
    }

    /// `list_profiles` plus the notices about profile.json files that could not be read,
    /// including ones found by earlier reads. A file that does not parse is renamed to
    /// `profile.json.unreadable-<n>` and never overwritten; one that cannot be read (locked,
    /// no access) is skipped and left alone. The default profile is created only when no
    /// profile.json exists at all.
    pub fn list_profiles_reporting(&self) -> (Vec<ModProfile>, Vec<String>) {
        let profiles = self.scan_profiles();
        (profiles, self.take_profile_notices())
    }

    /// The pending notices about unreadable profile files, oldest first.
    pub fn take_profile_notices(&self) -> Vec<String> {
        let mut q = self.profile_notices.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut q.pending)
    }

    fn queue_profile_notice(&self, notice: String) {
        let mut q = self.profile_notices.lock().unwrap_or_else(|e| e.into_inner());
        if q.seen.insert(notice.clone()) {
            q.pending.push(notice);
        }
    }

    fn scan_profiles(&self) -> Vec<ModProfile> {
        let mut profiles = Vec::new();
        let mut found_any = false;
        if let Ok(entries) = fs::read_dir(&self.profiles_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p_file = entry.path().join("profile.json");
                if !p_file.is_file() {
                    continue;
                }
                found_any = true;
                let raw = match fs::read_to_string(&p_file) {
                    Ok(raw) => raw,
                    Err(e) => {
                        // Maybe only locked for now: skip it, keep the file.
                        self.queue_profile_notice(format!(
                            "The profile \"{}\" could not be read ({e}). It is not listed for now.",
                            profile_dir_name(&p_file)
                        ));
                        continue;
                    }
                };
                match serde_json::from_str::<ModProfile>(&raw) {
                    Ok(prof) => profiles.push(prof),
                    Err(e) => self.queue_profile_notice(back_up_unreadable(&p_file, &e.to_string())),
                }
            }
        }
        if !found_any {
            let installed = self.list_installed_mods();
            let enabled_mods: Vec<String> = installed.iter().map(|m| m.manifest.id.clone()).collect();
            // With mods already installed, the user sees the stacking notice before the first
            // launch (D3); with none, it is a new profile (D40).
            let (stacking, notices_seen, line_rules_v2) = if enabled_mods.is_empty() {
                (Some(StackMode::Dependencies), new_profile_notices(), Some(true))
            } else {
                (None, Vec::new(), None)
            };
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
                patch_order: Vec::new(),
                linked_save_profile_id: "default".to_string(),
                launch_method: None,
                disable_console: false,
                custom_launch_args: Vec::new(),
                isolate_save_identity: false,
                pinned_versions: HashMap::new(),
                locked_mods: Vec::new(),
                favorites: Vec::new(),
                mod_notes: HashMap::new(),
                stacking,
                notices_seen,
                line_rules_v2,
                fixes: Vec::new(),
                fixes_version: crate::fixes::FIXES_VERSION,
                auto_follow: false,
                fix_journal: Vec::new(),
                migration_error: None,
                faction_picks: BTreeMap::new(),
            };
            let _ = self.save_profile(&default_prof);
            profiles.push(default_prof);

            let vanilla_prof = builtin_vanilla_profile(None);
            let _ = self.save_profile(&vanilla_prof);
            profiles.push(vanilla_prof);
        }
        // Whatever is on disk, the Vanilla profile is the built-in one (a hand-edited file that
        // adds mods has no effect).
        let existing = profiles.iter().position(|p| p.id == VANILLA_ID);
        let canonical = builtin_vanilla_profile(existing.map(|i| &profiles[i]));
        match existing {
            Some(i) => profiles[i] = canonical,
            None => {
                let _ = self.save_profile(&canonical);
                profiles.push(canonical);
            }
        }
        profiles.sort_by(|a, b| a.id.cmp(&b.id));
        profiles
    }

    fn vanilla_profile_file(&self) -> PathBuf {
        self.profiles_dir.join(VANILLA_ID).join("profile.json")
    }

    fn vanilla_slot_file(&self) -> PathBuf {
        self.saves_dir.join(VANILLA_ID).join("save_profile.json")
    }

    /// The Vanilla profile file as it is on disk, for the launch check: `Ok(None)` when
    /// missing, `Err` when it cannot be read or parsed (`repair_vanilla_builtins` sets it aside).
    pub fn vanilla_profile_on_disk(&self) -> Result<Option<ModProfile>, String> {
        read_builtin(&self.vanilla_profile_file())
    }

    /// The Vanilla save slot sidecar as it is on disk, as `vanilla_profile_on_disk`.
    pub fn vanilla_slot_on_disk(&self) -> Result<Option<SaveProfile>, String> {
        read_builtin(&self.vanilla_slot_file())
    }

    /// Rewrites both built-in files in their canonical form. A file that cannot be read is
    /// first kept as `<name>.unreadable-<n>`.
    pub fn repair_vanilla_builtins(&self) -> Result<(), String> {
        let profile = self.vanilla_profile_on_disk().or_else(|why| {
            set_aside_builtin(&self.vanilla_profile_file(), &why).map(|()| None)
        })?;
        self.save_profile(&builtin_vanilla_profile(profile.as_ref()))?;
        let slot = self
            .vanilla_slot_on_disk()
            .or_else(|why| set_aside_builtin(&self.vanilla_slot_file(), &why).map(|()| None))?;
        self.save_save_profile(&builtin_vanilla_slot(slot.as_ref()))
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
        write_unique_atomic(&dir.join("profile.json"), json.as_bytes())
    }

    /// `save_profile`, after the profile's stored references to a zip's stem id are renamed
    /// to the id in its mod.json (D62), for the zips `raw` loaded. The zips whose references
    /// changed.
    pub fn save_profile_renaming(
        &self,
        profile: &mut ModProfile,
        raw: Option<&qm_core::patch::plan::RawLoad>,
    ) -> Result<Vec<crate::fixes::ZipRename>, String> {
        let renamed = raw.map(|raw| crate::fixes::rename_zip_stems(profile, raw)).unwrap_or_default();
        self.save_profile(profile)?;
        Ok(renamed)
    }

    pub fn clone_profile(&self, source_id: &str, new_id: &str, new_name: &str) -> Result<ModProfile, String> {
        let mut src = self.get_profile(source_id)?;
        let now = jiff::Zoned::now().to_string();
        src.id = new_id.to_string();
        src.name = new_name.to_string();
        src.created_at = now.clone();
        src.updated_at = now;
        crate::fixes::renew_for_clone(&mut src);
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
            // The ledger comes along with its own ids; stacking and notices as they are (D40).
            crate::fixes::renew_for_clone(&mut cloned);
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
                patch_order: Vec::new(),
                linked_save_profile_id: save_id.to_string(),
                launch_method: None,
                disable_console: false,
                custom_launch_args: Vec::new(),
                isolate_save_identity: false,
                pinned_versions: HashMap::new(),
                locked_mods: Vec::new(),
                favorites: Vec::new(),
                mod_notes: HashMap::new(),
                // A new profile: dependency order and the line rules, notices seen (D3, D40, D51).
                stacking: Some(StackMode::Dependencies),
                notices_seen: new_profile_notices(),
                line_rules_v2: Some(true),
                fixes: Vec::new(),
                fixes_version: crate::fixes::FIXES_VERSION,
                auto_follow: false,
                fix_journal: Vec::new(),
                migration_error: None,
                faction_picks: BTreeMap::new(),
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
                        // Older sidecars only had `is_vanilla_clean`.
                        sp.locked_vanilla |= sp.is_vanilla_clean;
                        sp.builtin = sp.id == VANILLA_ID;
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
                tags: SaveTags::default(),
                locked_vanilla: false,
                builtin: false,
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

            let vanilla_save = builtin_vanilla_slot(None);
            let _ = self.save_save_profile(&vanilla_save);
            profiles.push(vanilla_save);
        }
        // The Vanilla slot is always the built-in one, locked as vanilla.
        let existing = profiles.iter().position(|s| s.id == VANILLA_ID);
        let canonical = builtin_vanilla_slot(existing.map(|i| &profiles[i]));
        match existing {
            Some(i) => profiles[i] = canonical,
            None => {
                // A sidecar that is there but did not read is kept, not overwritten.
                let file = self.vanilla_slot_file();
                if !file.is_file() || set_aside(&file).is_ok() {
                    let _ = self.save_save_profile(&canonical);
                }
                profiles.push(canonical);
            }
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

    /// The campaign save file the slot holds now: the live one for the active slot, else its copy.
    pub fn slot_save_file(&self, save_profile_id: &str) -> Option<PathBuf> {
        let settings = self.load_settings();
        if settings.active_save_profile_id == save_profile_id {
            if let Some(f) = settings
                .save_dir_path
                .as_ref()
                .map(|d| d.join("saveMNB3.lua"))
                .filter(|f| f.is_file())
            {
                return Some(f);
            }
        }
        let f = self
            .saves_dir
            .join(save_profile_id)
            .join("active")
            .join("saveMNB3.lua");
        f.is_file().then_some(f)
    }

    /// Whether the slot's save carries a QuarterMaster `_qm_meta` stamp (a text search; no parse).
    pub fn slot_has_qm_stamp(&self, save_profile_id: &str) -> bool {
        self.slot_save_file(save_profile_id)
            .and_then(|f| fs::read(f).ok())
            .is_some_and(|b| b.windows(8).any(|w| w == b"_qm_meta"))
    }

    fn find_save_profile(&self, id: &str) -> Result<SaveProfile, String> {
        self.list_save_profiles()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("Save slot '{id}' not found"))
    }

    /// Sets the sticky `modded` tag (it is never cleared).
    pub fn mark_slot_modded(&self, save_profile_id: &str) -> Result<(), String> {
        let mut sp = self.find_save_profile(save_profile_id)?;
        if sp.builtin || sp.tags.modded {
            return Ok(());
        }
        sp.tags.modded = true;
        self.save_save_profile(&sp)
    }

    /// Renames a slot; names are unique, ignoring case. The Vanilla slot keeps its name.
    pub fn rename_save_profile(&self, id: &str, name: &str) -> Result<SaveProfile, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A save slot needs a name.".to_string());
        }
        let mut sp = self.find_save_profile(id)?;
        if sp.builtin {
            return Err("The Vanilla save slot cannot be renamed.".to_string());
        }
        if self
            .list_save_profiles()
            .iter()
            .any(|s| s.id != id && s.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("A save slot named '{name}' already exists."));
        }
        sp.name = name.to_string();
        self.save_save_profile(&sp)?;
        Ok(sp)
    }

    /// An empty slot: a fresh game start, untagged and unlocked. Its id comes from the name.
    pub fn create_save_profile(&self, name: &str) -> Result<SaveProfile, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A save slot needs a name.".to_string());
        }
        let existing = self.list_save_profiles();
        if existing.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
            return Err(format!("A save slot named '{name}' already exists."));
        }
        let base: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        let base = if base.is_empty() || base == VANILLA_ID { "slot".to_string() } else { base };
        let mut id = base.clone();
        let mut n = 2;
        while existing.iter().any(|s| s.id == id) || self.saves_dir.join(&id).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let sp = SaveProfile {
            id,
            name: name.to_string(),
            is_vanilla_clean: false,
            tags: SaveTags::default(),
            locked_vanilla: false,
            builtin: false,
            created_at: jiff::Zoned::now().to_string(),
            last_played_at: None,
            active_mods_fingerprint: HashMap::new(),
            backups: Vec::new(),
        };
        self.save_save_profile(&sp)?;
        Ok(sp)
    }

    /// Moves the whole slot folder (saves and backups) to `saves/.deleted/<id>-<time>/`, where it
    /// is kept for 30 days. The caller refuses the active and the built-in slot.
    pub fn delete_save_profile(&self, id: &str) -> Result<PathBuf, String> {
        let sp = self.find_save_profile(id)?;
        if sp.builtin {
            return Err("The Vanilla save slot cannot be deleted.".to_string());
        }
        let trash = self.saves_dir.join(".deleted");
        fs::create_dir_all(&trash).map_err(|e| format!("Failed to create {}: {e}", trash.display()))?;
        let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
        let mut dest = trash.join(format!("{id}-{stamp}"));
        let mut n = 2;
        while dest.exists() {
            dest = trash.join(format!("{id}-{stamp}-{n}"));
            n += 1;
        }
        fs::rename(self.saves_dir.join(id), &dest)
            .map_err(|e| format!("Failed to move the slot to {}: {e}", dest.display()))?;
        self.prune_deleted_slots(std::time::Duration::from_secs(30 * 24 * 60 * 60));
        Ok(dest)
    }

    /// Removes deleted slots older than `max_age` from `saves/.deleted/`.
    pub fn prune_deleted_slots(&self, max_age: std::time::Duration) {
        let Ok(entries) = fs::read_dir(self.saves_dir.join(".deleted")) else {
            return;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > max_age);
            if old {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Backs the slot up, then clears it to a fresh new game (its tags and lock stay). For the
    /// active slot the live save files are cleared too. The caller refuses this while the game runs.
    pub fn reset_save_profile(&self, id: &str) -> Result<SaveBackupEntry, String> {
        let sp = self.find_save_profile(id)?;
        let backup = self.create_save_backup(&sp.id, "Before reset")?;
        let active_dir = self.saves_dir.join(&sp.id).join("active");
        for fname in SAVE_FILES {
            let f = active_dir.join(fname);
            if f.is_file() {
                fs::remove_file(&f).map_err(|e| format!("Failed to remove {}: {e}", f.display()))?;
            }
        }
        let settings = self.load_settings();
        if settings.active_save_profile_id == sp.id {
            if let Some(live) = settings.save_dir_path.as_ref() {
                for fname in SAVE_FILES {
                    let f = live.join(fname);
                    if f.is_file() {
                        fs::remove_file(&f).map_err(|e| format!("Failed to remove {}: {e}", f.display()))?;
                    }
                }
            }
        }
        Ok(backup)
    }

    /// Locks or unlocks a slot as vanilla. The Vanilla slot is always locked.
    pub fn set_save_lock(&self, id: &str, locked: bool) -> Result<SaveProfile, String> {
        let mut sp = self.find_save_profile(id)?;
        if sp.builtin && !locked {
            return Err("The Vanilla save slot is always locked.".to_string());
        }
        sp.locked_vanilla = locked;
        sp.is_vanilla_clean = locked;
        self.save_save_profile(&sp)?;
        Ok(sp)
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

        // The active slot's `active/` copy is synced only on switch, backup or clone,
        // so the live save is the current state when it exists.
        let live_file = if settings.active_save_profile_id == save_profile_id {
            settings
                .save_dir_path
                .as_ref()
                .map(|d| d.join("saveMNB3.lua"))
                .filter(|f| f.is_file())
        } else {
            None
        };
        let target_file = live_file.or_else(|| slot_file.is_file().then_some(slot_file));

        if let Some(file_path) = target_file {
            if let Ok(bytes) = fs::read(&file_path) {
                let sha = qm_schema::sha256_hex(&bytes);
                // v2: the campaign and classic rank fields were swapped before plan 2c Task 7.
                let cache_dir = self.cache_dir.join("save-summaries-v2");
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

    /// Creates a timestamped backup snapshot of `save_profile_id`, then prunes the oldest automatic
    /// backups (label starting with `Auto-backup`) beyond `max_save_backups`. Manual backups are kept.
    pub fn create_save_backup(
        &self,
        save_profile_id: &str,
        label: &str,
    ) -> Result<SaveBackupEntry, String> {
        self.save_backup(save_profile_id, label, false)
    }

    /// `create_save_backup` that fails when any save file present fails to copy, from the
    /// live save folder into the slot or from the slot into the snapshot (D68: a trial runs
    /// only on a complete backup). A failed snapshot is removed.
    pub fn create_save_backup_strict(
        &self,
        save_profile_id: &str,
        label: &str,
    ) -> Result<SaveBackupEntry, String> {
        self.save_backup(save_profile_id, label, true)
    }

    fn save_backup(
        &self,
        save_profile_id: &str,
        label: &str,
        strict: bool,
    ) -> Result<SaveBackupEntry, String> {
        let settings = self.load_settings();
        let slot_dir = self.saves_dir.join(save_profile_id);
        let active_dir = slot_dir.join("active");
        let backups_dir = slot_dir.join("backups");
        fs::create_dir_all(&backups_dir)
            .map_err(|e| format!("Failed to create backups dir: {e}"))?;

        // Sync from live `%APPDATA%\mnb\` first if this is the currently active save profile
        let mut failed: Vec<String> = Vec::new();
        if settings.active_save_profile_id == save_profile_id {
            if let Some(ref live_dir) = settings.save_dir_path {
                if let Err(e) = fs::create_dir_all(&active_dir) {
                    failed.push(format!("{}: {e}", active_dir.display()));
                }
                for fname in SAVE_FILES {
                    let src = live_dir.join(fname);
                    if src.is_file() {
                        if let Err(e) = copy_whole(&src, &active_dir.join(fname)) {
                            failed.push(format!("{}: {e}", src.display()));
                        }
                    }
                }
            }
        }
        if strict && !failed.is_empty() {
            return Err(format!(
                "Could not copy the live save files: {}",
                failed.join("; ")
            ));
        }

        let now = jiff::Zoned::now();
        let ts_slug = now.strftime("%Y-%m-%d_%H-%M-%S").to_string();
        // Two backups in the same second must not share (and overwrite) one snapshot dir;
        // `create_dir` claims the name atomically.
        let mut backup_id = format!("bak-{ts_slug}");
        let mut n = 1;
        let snap_dir = loop {
            let dir = backups_dir.join(&backup_id);
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => {
                    n += 1;
                    backup_id = format!("bak-{ts_slug}-{n}");
                }
                Err(e) => return Err(format!("Failed to create snapshot dir: {e}")),
            }
        };

        let mut total_size = 0u64;
        for fname in SAVE_FILES {
            let src = active_dir.join(fname);
            if src.is_file() {
                if let Ok(meta) = src.metadata() {
                    total_size += meta.len();
                }
                if let Err(e) = copy_whole(&src, &snap_dir.join(fname)) {
                    failed.push(format!("{}: {e}", src.display()));
                }
            }
        }
        if strict && !failed.is_empty() {
            let _ = fs::remove_dir_all(&snap_dir);
            return Err(format!(
                "Could not copy the save files into the backup: {}",
                failed.join("; ")
            ));
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

        // Newest first, so everything after the first `max_save_backups` automatic ones goes.
        let autos = list_backups_in_dir(&backups_dir)
            .into_iter()
            .filter(|b| b.label.starts_with(AUTO_BACKUP_PREFIX));
        // At least one: a 0 from a hand-edited or legacy settings.json must not delete every auto backup.
        for old in autos.skip(settings.max_save_backups.max(1)) {
            let _ = fs::remove_dir_all(backups_dir.join(&old.id));
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
        // A copy carries the source's history (its tags), but never the built-in flag or lock.
        let new_sp = SaveProfile {
            id: clean_id.clone(),
            name: new_name.to_string(),
            is_vanilla_clean: false,
            tags: src_sp.tags.clone(),
            locked_vanilla: false,
            builtin: false,
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

    /// Directly switches the active save slot and links the active mod profile to it,
    /// backing up the outgoing slot when `auto_backup_saves` is on.
    pub fn activate_save_profile(&self, target_save_id: &str) -> Result<(), String> {
        let backup_current = self.load_settings().auto_backup_saves;
        self.activate_save_profile_with(target_save_id, backup_current)
    }

    /// Like `activate_save_profile`; `backup_current` says whether the outgoing slot is backed up
    /// first (callers that already made a backup pass `false`).
    pub fn activate_save_profile_with(
        &self,
        target_save_id: &str,
        backup_current: bool,
    ) -> Result<(), String> {
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
            backup_current,
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

        let is_tainting_vanilla_save =
            (sp.locked_vanilla || sp.is_vanilla_clean) && !profile.enabled_mods.is_empty();
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
    /// A profile with no slot ("New slot on first launch") gets an empty slot named after it.
    /// Returns the new slot's name when one was created.
    pub fn ensure_linked_slot(&self, profile_id: &str) -> Result<Option<String>, String> {
        let mut profile = self.get_profile(profile_id)?;
        if !profile.linked_save_profile_id.trim().is_empty() {
            return Ok(None);
        }
        let taken: Vec<String> = self.list_save_profiles().into_iter().map(|s| s.name.to_lowercase()).collect();
        let mut name = profile.name.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            name = format!("{} {n}", profile.name);
            n += 1;
        }
        let slot = self.create_save_profile(&name)?;
        profile.linked_save_profile_id = slot.id;
        self.save_profile(&profile)?;
        Ok(Some(slot.name))
    }

    pub fn activate_profile(&self, profile_id: &str) -> Result<LoadOrderPlan, String> {
        self.activate_profile_reporting(profile_id)
            .map(|a| a.plan)
            .map_err(|e| e.to_string())
    }

    /// Stages the profile and makes it active; the result carries notices for the user. The
    /// plan is read back after the overwrite overlay, and any error or skipped entry fails
    /// activation with `ActivateError::InvalidPlan` (§7.3).
    pub fn activate_profile_reporting(&self, profile_id: &str) -> Result<Activation, ActivateError> {
        self.stage_and_activate(profile_id, false, ActivateExtras::default())
    }

    /// `activate_profile_reporting` with the cached vanilla files, so a first migration to the
    /// ledger also compares the patched text (§7.4).
    pub fn activate_profile_with(
        &self,
        profile_id: &str,
        extras: ActivateExtras<'_>,
    ) -> Result<Activation, ActivateError> {
        self.stage_and_activate(profile_id, false, extras)
    }

    /// `profile.json.pre-fixes-v1.bak`, the copy kept before the first migration.
    pub fn fixes_backup_file(&self, profile_id: &str) -> PathBuf {
        self.profiles_dir
            .join(profile_id)
            .join("profile.json.pre-fixes-v1.bak")
    }

    /// Copies profile.json to `profile.json.pre-fixes-v1.bak`, once.
    pub fn back_up_before_fixes(&self, profile_id: &str) -> Result<(), String> {
        let bak = self.fixes_backup_file(profile_id);
        if bak.exists() {
            return Ok(());
        }
        let file = self.profiles_dir.join(profile_id).join("profile.json");
        fs::copy(&file, &bak)
            .map(|_| ())
            .map_err(|e| format!("Failed to back up {}: {e}", file.display()))
    }

    /// The first migration of `profile` to the ledger, on the staged Mods folder (§7.4). On
    /// failure the profile keeps its legacy fields and records why; activation goes on. A
    /// warning is returned when the reason is new.
    fn migrate_staged(
        &self,
        profile: ModProfile,
        mods_dir: &Path,
        enabled: &[String],
        mod_priorities: &HashMap<String, Priority>,
        vanilla: Option<&HashMap<String, String>>,
    ) -> (ModProfile, Option<String>) {
        let needs_mods = crate::fixes::has_legacy_entries(&profile);
        let migrated = if needs_mods {
            let legacy = build_plan(&profile, enabled.to_vec(), mod_priorities.clone());
            let dir = dunce::canonicalize(mods_dir).unwrap_or_else(|_| mods_dir.to_path_buf());
            match qm_core::patch::plan::load_raw(&dir, Some(&legacy)) {
                Ok(raw) => crate::fixes::migrate(&profile, &raw, vanilla),
                Err(e) => Err(format!("the mods could not be read: {e:#}")),
            }
        } else {
            crate::fixes::migrate(&profile, &qm_core::patch::plan::RawLoad::default(), vanilla)
        };
        match migrated {
            Ok(m) => {
                if needs_mods {
                    if let Err(e) = self.back_up_before_fixes(&profile.id) {
                        return (profile, Some(e));
                    }
                }
                match self.save_profile(&m) {
                    Ok(()) => (m, None),
                    Err(e) => (profile, Some(e)),
                }
            }
            Err(why) => {
                let new = profile.migration_error.as_deref() != Some(why.as_str());
                let mut p = profile;
                p.migration_error = Some(why.clone());
                let _ = self.save_profile(&p);
                let warning = new.then(|| format!("Your earlier fixes could not be imported yet: {why}"));
                (p, warning)
            }
        }
    }

    /// Like `activate_profile_reporting`, but stages no mods and a plan with no mods, for a
    /// "Launch without mods" run (D16). The save slot, save identity and console setting are
    /// the profile's own, and the overwrite bucket is not overlaid.
    pub fn activate_profile_without_mods(&self, profile_id: &str) -> Result<Activation, ActivateError> {
        self.stage_and_activate(profile_id, true, ActivateExtras::default())
    }

    fn stage_and_activate(
        &self,
        profile_id: &str,
        without_mods: bool,
        extras: ActivateExtras<'_>,
    ) -> Result<Activation, ActivateError> {
        // Never swap saves into an empty slot id: create the profile's slot first.
        self.ensure_linked_slot(profile_id)?;
        if let Some(pid) = is_game_running() {
            return Err(format!(
                "Cannot switch profiles while Mud and Blood (mnb.exe, PID {pid}) is running. Please close the game first."
            )
            .into());
        }
        let mut settings = self.load_settings();
        let mut profile = self.get_profile(profile_id)?;
        let trial = extras.trial.filter(|_| !without_mods);
        if let Some(t) = trial {
            t.apply(&mut profile);
        }
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
        let resolution = resolve_profile_order(&profile, &enabled_manifests);

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

        let staged: &[_] = if without_mods { &[] } else { &resolution.ordered_mods };
        for entry in staged {
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

        // Once per profile: the legacy fields become ledger fixes, checked to give the same
        // game code (§7.4). Before the plan write, so the plan carries the refs.
        let mut warnings = Vec::new();
        if !without_mods && trial.is_none() && profile.fixes_version < crate::fixes::FIXES_VERSION {
            let (p, warning) = self.migrate_staged(
                profile,
                &profile_mods_dir,
                &ordered_mod_ids,
                &mod_priorities,
                extras.vanilla,
            );
            profile = p;
            warnings.extend(warning);
        }

        // 3. Write `Mods/qm/load-order.json` and `Mods/qm/launch-config.json`
        let load_order_plan = if without_mods {
            build_plan(&no_mods(&profile), Vec::new(), HashMap::new())
        } else {
            build_plan(&profile, ordered_mod_ids, mod_priorities)
        };
        load_order_plan.save_to_mod_dir_atomic(&profile_mods_dir)?;

        let is_vanilla = profile.enabled_mods.is_empty();
        let launch_cfg = LaunchConfig {
            profile_id: Some(profile.id.clone()),
            profile_name: Some(profile.name.clone()),
            vanilla_mode: is_vanilla,
            // The console is one global setting; the old per-profile flag is ignored.
            disable_console: settings.disable_console_on_launch,
            dump_all: false,
            debug: settings.debug_mode,
            dev_commands: settings.dev_commands,
            save_identity_override: if profile.isolate_save_identity {
                Some(format!("mnb_{}", profile.linked_save_profile_id))
            } else {
                None
            },
            active_mods: active_mods_map.clone(),
            dev_hot_reload: false,
            run_id: None,
            // Of every installed mod, enabled or not: the game needs it for the mods that
            // are off (Phase E D87). Not for a test launch without mods, which must leave
            // the save as it is.
            save_content: installed
                .iter()
                .filter(|_| !without_mods)
                .filter_map(|m| {
                    let weapons = &m.manifest.save_content.as_ref()?.weapons;
                    (!weapons.is_empty()).then(|| {
                        let content = qm_core::handshake::SaveContent {
                            weapons: weapons.clone(),
                        };
                        (m.manifest.id.clone(), content.into())
                    })
                })
                .collect(),
            ..LaunchConfig::default()
        };
        launch_cfg.save_to_mod_dir(&profile_mods_dir)?;

        // Overlay any files in `profiles/<profile_id>/overwrite/` into `profiles/<profile_id>/Mods/`,
        // except the two files the launcher owns.
        if overwrite_dir.is_dir() && !without_mods {
            for owned in LAUNCHER_OWNED {
                if overwrite_dir.join(owned).is_file() {
                    warnings.push(format!(
                        "overwrite/{owned} is ignored: QuarterMaster writes this file itself"
                    ));
                }
            }
            let _ = copy_directory_skipping(&overwrite_dir, &profile_mods_dir, &is_launcher_owned);
        }

        #[cfg(any(test, feature = "test-seams"))]
        if !without_mods
            && (CORRUPT_PLAN_AFTER_OVERLAY.with(std::cell::Cell::get)
                || CORRUPT_NEXT_PLAN.with(|c| c.replace(false)))
        {
            let _ = fs::write(profile_mods_dir.join("qm").join("load-order.json"), "{");
        }

        // The game reads exactly this file: it must read back with no error and no skipped entry.
        if let Err(why) = check_staged_plan(&profile_mods_dir) {
            let _ = journal.abort_transaction(&self.journal_file);
            return Err(ActivateError::InvalidPlan(why));
        }

        // Profile staging is isolated to `profiles/<profile_id>/Mods/`.
        // Linking into the live game directory (<game_dir>/Mods and winmm.dll) is performed
        // strictly by the qm-service game-folder transactions (`deploy_profile_to_game`),
        // keeping the Steam game install completely vanilla during regular profile management.

        // 4. Swap save profile if needed and sync per-profile `saveModSettings.lua`
        self.switch_save_profile_internal(
            &settings.active_save_profile_id,
            &profile.linked_save_profile_id,
            &active_mods_map,
            settings.save_dir_path.as_deref(),
            settings.auto_backup_saves,
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
        // The profile's own plan is staged again, so no test plan is left (D68).
        if !without_mods && trial.is_none() {
            if let Err(e) = self.clear_test_plan_staged(&profile.id) {
                warnings.push(e);
            }
        }

        Ok(Activation {
            plan: load_order_plan,
            warnings,
        })
    }

    fn switch_save_profile_internal(
        &self,
        from_save_id: &str,
        to_save_id: &str,
        active_mods: &HashMap<String, String>,
        live_save_dir: Option<&Path>,
        backup_current: bool,
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

        if backup_current && from_save_id != to_save_id {
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
    /// and copies `dll_source` to `<game_dir>/winmm.dll`. The caller owns the DLL ownership check.
    pub fn deploy_profile_to_game(&self, profile_id: &str, dll_source: &Path) -> Result<(), String> {
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
        if fs::symlink_metadata(&game_mods_dir).is_ok() {
            if !is_reparse_point(&game_mods_dir) {
                // A real directory: never delete it without a successful backup.
                let bak_name = format!(
                    "pre-qm-mods-{}",
                    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S")
                );
                let bak_dest = self.backups_dir.join(bak_name);
                copy_directory_clean(&game_mods_dir, &bak_dest)?;
            }
            remove_junction_or_dir(&game_mods_dir)?;
        }

        create_dir_junction_or_copy(&profile_mods_dir, &game_mods_dir)?;
        install_winmm_dll(dll_source, game_dir)?;
        Ok(())
    }

    /// Restores the game directory to a clean, unmodded vanilla state.
    /// Removes the `<game_dir>/Mods` junction, then removes `winmm.dll` only when its SHA-256
    /// is in `known_dll_hashes`. An unknown DLL is left in place and reported as an error
    /// (after the junction step has run).
    pub fn restore_clean_game_dir(&self, known_dll_hashes: &HashSet<String>) -> Result<(), String> {
        let settings = self.load_settings();
        let Some(ref game_exe) = settings.game_exe_path else {
            return Ok(());
        };
        let Some(game_dir) = game_exe.parent() else {
            return Ok(());
        };

        let game_mods_dir = game_dir.join("Mods");
        if fs::symlink_metadata(&game_mods_dir).is_ok() {
            if is_reparse_point(&game_mods_dir) {
                remove_junction_or_dir(&game_mods_dir)?;
            } else {
                // A real directory: never delete it without a successful backup.
                let bak_name = format!(
                    "unlinked-mods-{}",
                    jiff::Zoned::now().strftime("%Y%m%d-%H%M%S")
                );
                let bak_dest = self.backups_dir.join(bak_name);
                copy_directory_clean(&game_mods_dir, &bak_dest)?;
                remove_junction_or_dir(&game_mods_dir)?;
            }
        }

        // Leftover from an interrupted atomic DLL copy; always ours.
        let tmp = game_dir.join(WINMM_TMP_NAME);
        if tmp.is_file() {
            fs::remove_file(&tmp)
                .map_err(|e| format!("Failed to remove {}: {e}", tmp.display()))?;
        }

        let winmm = game_dir.join("winmm.dll");
        if winmm.is_file() {
            let bytes = fs::read(&winmm)
                .map_err(|e| format!("Failed to read {}: {e}", winmm.display()))?;
            let hash = qm_schema::sha256_hex(&bytes);
            if !known_dll_hashes.contains(&hash) {
                return Err(format!(
                    "{UNKNOWN_DLL_ERROR_PREFIX} in {} (sha256 {hash}); left in place",
                    game_dir.display()
                ));
            }
            fs::remove_file(&winmm)
                .map_err(|e| format!("Failed to remove {}: {e}", winmm.display()))?;
        }

        Ok(())
    }

    /// Spawns Mud and Blood for `profile` with `method`. Does no activation, deploy or backup;
    /// the caller prepares the game folder first.
    pub fn spawn_game_process(&self, profile: &ModProfile, method: LaunchMethod) -> Result<u32, String> {
        let settings = self.load_settings();
        let game_exe = settings
            .game_exe_path
            .clone()
            .ok_or_else(|| "Mud and Blood executable (mnb.exe) path is not configured.".to_string())?;
        let game_dir = game_exe
            .parent()
            .ok_or_else(|| "Invalid game directory".to_string())?;

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
                    crate::winproc::system_command("cmd")
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
                    crate::winproc::system_command("cmd")
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
                if settings.disable_console_on_launch {
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
        let status = crate::winproc::system_command("cmd")
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

/// Prefix of the error `restore_clean_game_dir` returns for a winmm.dll it does not recognise.
pub const UNKNOWN_DLL_ERROR_PREFIX: &str = "unknown winmm.dll";

/// Temp name used while copying `winmm.dll` into the game folder.
pub const WINMM_TMP_NAME: &str = "winmm.dll.qm-tmp";

/// Copies `src` to `<game_dir>/winmm.dll` via a temp file and rename, so a crash never
/// leaves a half-written DLL. Returns the SHA-256 of the installed file.
fn install_winmm_dll(src: &Path, game_dir: &Path) -> Result<String, String> {
    let dest = game_dir.join("winmm.dll");
    let tmp = game_dir.join(WINMM_TMP_NAME);
    if let Err(e) = fs::copy(src, &tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to copy {} to {}: {e}", src.display(), tmp.display()));
    }
    if let Err(e) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to install {}: {e}", dest.display()));
    }
    let bytes = fs::read(&dest).map_err(|e| format!("Failed to read {}: {e}", dest.display()))?;
    Ok(qm_schema::sha256_hex(&bytes))
}

/// `qm_winmm.dll` (the shipped proxy) next to the running executable (never the working directory).
fn exe_dir_winmm_dll() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.join("qm_winmm.dll");
    p.is_file().then_some(p)
}

impl QuarterMasterPaths {
    /// Checks `winmm.dll` installation & SHA-256 integrity, whether `mnb.exe` is running,
    /// and reads `Mods/qm/patch-report.json` + recent runtime logs, and the active profile's
    /// last run from its run log.
    pub fn check_system_health(&self) -> serde_json::Value {
        let settings = self.load_settings();
        let mut dll_present = false;
        let mut dll_sha256 = None;
        let mut dll_matches_build = true;
        let mut patch_report_val = serde_json::Value::Null;
        let mut patch_report_error = None;
        let mut recent_logs: Vec<serde_json::Value> = Vec::new();

        let source_dll_sha = exe_dir_winmm_dll()
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

                (patch_report_val, patch_report_error) = read_patch_report_json(game_dir);

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
        // The game folder's report goes with the junction; the run log keeps it (D67, D72).
        let last_run = self.last_run(&settings.active_profile_id);
        // A test run's plan still staged in place of the profile's own (D68).
        let test_plan = self.staged_test_plan(&settings.active_profile_id);

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
            "patch_report_error": patch_report_error,
            "recent_logs": recent_logs,
            "last_run": last_run,
            "test_plan": test_plan,
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

    /// Re-installs `winmm.dll` in the game directory from `dll_source` and returns its SHA-256.
    /// The caller owns the DLL ownership check.
    pub fn repair_winmm_dll(&self, dll_source: &Path) -> Result<String, String> {
        let settings = self.load_settings();
        let exe = settings
            .game_exe_path
            .ok_or_else(|| "Game executable path is not configured".to_string())?;
        let game_dir = exe.parent().ok_or_else(|| "Invalid game dir".to_string())?;
        install_winmm_dll(dll_source, game_dir)
    }
}

/// Label prefix of backups QuarterMaster makes on its own; only these are pruned.
pub const AUTO_BACKUP_PREFIX: &str = "Auto-backup";

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
    crate::winproc::find_process("mnb.exe")
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

    /// A temp root with the default profile (no mods) on disk.
    fn paths_with_default() -> (tempfile::TempDir, QuarterMasterPaths) {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        assert!(paths.list_profiles().iter().any(|p| p.id == "default"));
        (temp, paths)
    }

    fn staged_plan(paths: &QuarterMasterPaths, id: &str) -> LoadOrderPlan {
        let read = LoadOrderPlan::read(&paths.profiles_dir.join(id).join("Mods"));
        assert!(read.error.is_none(), "{:?}", read.error);
        read.plan.expect("staged plan")
    }

    #[test]
    fn a_faction_pick_goes_into_the_plan_as_schema_four() {
        let mut prof = builtin_vanilla_profile(None);
        prof.id = "p".into();
        let plan = build_plan(&prof, Vec::new(), HashMap::new());
        assert_eq!(plan.schema_version, 3, "no Phase E field: the plan is as before");
        assert!(plan.factions.is_empty());
        assert!(!serde_json::to_string(&prof).unwrap().contains("faction_picks"));

        prof.faction_picks.insert("allied".into(), "eastern_front".into());
        let plan = build_plan(&prof, Vec::new(), HashMap::new());
        assert_eq!(plan.schema_version, 4);
        assert_eq!(plan.faction_pick("allied"), Some("eastern_front"));
        // The pick round-trips through profile.json.
        let back: ModProfile =
            serde_json::from_str(&serde_json::to_string(&prof).unwrap()).unwrap();
        assert_eq!(back.faction_picks, prof.faction_picks);
    }

    #[test]
    fn build_plan_writes_v2_and_the_legacy_fields() {
        let mut prof = builtin_vanilla_profile(None);
        prof.id = "p".into();
        // A profile from before stacking.
        prof.stacking = None;
        prof.notices_seen.clear();
        prof.disabled_patches = vec!["gcore:main_lua:16".into()];
        prof.patch_priorities.insert("x".into(), 7);
        prof.patch_order = vec![qm_core::patch::order::OrderRule {
            patch: "a".into(),
            before: vec!["b".into()],
            after: Vec::new(),
        }];
        let plan = build_plan(&prof, vec!["gcore".into()], HashMap::from([("gcore".into(), 10)]));
        assert_eq!(plan.schema_version, 3);
        assert!(plan.generator.as_deref().unwrap_or("").starts_with("quartermaster "));
        assert_eq!(plan.stack_mode(), StackMode::Legacy);
        assert!(plan.stacking.is_some());
        let v = serde_json::to_value(&plan).unwrap();
        assert_eq!(v["disabled_patches"], serde_json::json!(["gcore:main_lua:16"]));
        assert_eq!(v["patch_priorities"], serde_json::json!({"x": 7}));
        assert_eq!(v["mod_priorities"], serde_json::json!({"gcore": 10}));
        assert_eq!(
            v["patch_order"],
            serde_json::json!([{"patch": "a", "before": ["b"], "after": []}])
        );
        assert_eq!(v["glue_patches"], serde_json::json!([]));
        assert!(plan.warnings().is_empty());
    }

    #[test]
    fn build_plan_for_runs_the_solver() {
        let manifest = |json: &str| ModManifest::from_json_str(json).unwrap();
        let manifests = vec![
            manifest(r#"{"id": "base", "name": "Base", "version": "1.0.0", "load_priority": 80}"#),
            // Lower priority, but it loads after base.
            manifest(r#"{"id": "compat_base", "name": "C", "version": "1.0.0", "load_priority": 5,
                "relationships": {"requires": {"base": ">=1.0.0"}}}"#),
            manifest(r#"{"id": "other", "name": "O", "version": "1.0.0", "load_priority": 50}"#),
            manifest(r#"{"id": "off", "name": "Off", "version": "1.0.0"}"#),
        ];
        let mut prof = builtin_vanilla_profile(None);
        prof.enabled_mods = vec!["compat_base".into(), "base".into(), "other".into()];
        prof.mod_priorities.insert("other".into(), 90);
        let plan = build_plan_for(&prof, &manifests);
        assert_eq!(plan.enabled_mods, ["base", "compat_base", "other"]);
        let prio = |id: &str| plan.mod_priority(&[id]);
        assert_eq!((prio("base"), prio("compat_base"), prio("other")), (Some(80), Some(5), Some(90)));
        assert_eq!(prio("off"), None);
    }

    /// A profile.json from before stacking: no `stacking`, no `notices_seen`, no
    /// `line_rules_v2`.
    fn make_existing(paths: &QuarterMasterPaths, id: &str) {
        let file = paths.profiles_dir.join(id).join("profile.json");
        let mut v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("stacking");
        obj.remove("notices_seen");
        obj.remove("line_rules_v2");
        fs::write(&file, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }

    #[test]
    fn a_new_profile_writes_dependencies() {
        let (_t, paths) = paths_with_default();
        let fresh = paths.get_profile("default").unwrap();
        assert_eq!(fresh.stacking, Some(StackMode::Dependencies));
        assert!(fresh.stacking_acknowledged());
        let p = paths.create_profile("Second", None, None, None).unwrap();
        assert_eq!(p.stacking, Some(StackMode::Dependencies));
        assert_eq!(p.notices_seen, [NOTICE_STACKING, NOTICE_LINE_RULES]);
        paths.activate_profile(&p.id).unwrap();
        assert_eq!(staged_plan(&paths, &p.id).stack_mode(), StackMode::Dependencies);
        let v: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(paths.profiles_dir.join(&p.id).join("Mods/qm/load-order.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(v["stacking"], "dependencies");
    }

    #[test]
    fn a_first_run_default_with_installed_mods_shows_the_notice() {
        let temp = tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let def = paths.get_profile("default").unwrap();
        assert!(def.enabled_mods.is_empty());
        assert!(def.stacking_acknowledged(), "no mods: a new profile");
        // A library with a mod but no profile yet.
        fs::remove_dir_all(&paths.profiles_dir).unwrap();
        let dir = paths.library_dir.join("gcore").join("1.0.0");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("mod.json"),
            r#"{"id":"gcore","name":"GCore","version":"1.0.0","author":"a","description":"d"}"#,
        )
        .unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let def = paths.get_profile("default").unwrap();
        assert_eq!(def.enabled_mods, ["gcore"]);
        assert_eq!(def.stacking, None);
        assert!(!def.stacking_acknowledged());
        assert_eq!(def.line_rules_v2, None, "the older rules until the notice is answered");
    }

    #[test]
    fn the_line_rules_reach_the_plan_only_when_the_profile_says_so() {
        let (_t, paths) = paths_with_default();
        // A new profile (no mods at first run, or created now): the newer rules (D51).
        let fresh = paths.get_profile("default").unwrap();
        assert_eq!(fresh.line_rules_v2, Some(true));
        assert!(fresh.notice_seen(NOTICE_LINE_RULES));
        let p = paths.create_profile("Second", None, None, None).unwrap();
        assert_eq!(p.line_rules_v2, Some(true));
        paths.activate_profile(&p.id).unwrap();
        assert!(staged_plan(&paths, &p.id).line_rules_v2());
        assert!(builtin_vanilla_profile(None).line_rules());

        // A profile.json from before Phase D: the field is absent, the plan says nothing.
        make_existing(&paths, "default");
        let mut old = paths.get_profile("default").unwrap();
        assert_eq!(old.line_rules_v2, None);
        assert!(!old.line_rules());
        paths.activate_profile("default").unwrap();
        let plan = staged_plan(&paths, "default");
        assert!(!plan.line_rules_v2());
        assert!(plan.line_rules_v2.is_none(), "an old profile's plan is unchanged");
        let raw = fs::read_to_string(paths.profiles_dir.join("default/profile.json")).unwrap();
        assert!(!raw.contains("line_rules_v2"));
        // "Keep the older order" and "Use the new order".
        for (answer, on) in [(Some(false), false), (Some(true), true)] {
            old.line_rules_v2 = answer;
            paths.save_profile(&old).unwrap();
            paths.activate_profile("default").unwrap();
            assert_eq!(staged_plan(&paths, "default").line_rules_v2(), on, "{answer:?}");
        }
        // A clone keeps the source's value.
        old.line_rules_v2 = None;
        paths.save_profile(&old).unwrap();
        assert_eq!(paths.clone_profile("default", "c2", "C2").unwrap().line_rules_v2, None);
    }

    #[test]
    fn an_existing_profile_writes_legacy_until_it_picks_a_mode() {
        let (_t, paths) = paths_with_default();
        make_existing(&paths, "default");
        let mut p = paths.get_profile("default").unwrap();
        assert_eq!(p.stacking, None);
        assert!(!p.stacking_acknowledged());
        assert_eq!(p.stack_mode(), StackMode::Legacy);
        paths.activate_profile("default").unwrap();
        assert_eq!(staged_plan(&paths, "default").stack_mode(), StackMode::Legacy);
        let plan = staged_plan(&paths, "default");
        assert!(plan.stacking.is_some(), "written explicitly");
        // Seeing the notice alone keeps the older order.
        assert!(p.mark_notice_seen(NOTICE_STACKING));
        assert!(!p.mark_notice_seen(NOTICE_STACKING));
        paths.save_profile(&p).unwrap();
        paths.activate_profile("default").unwrap();
        assert_eq!(staged_plan(&paths, "default").stack_mode(), StackMode::Legacy);
        for mode in [StackMode::Dependencies, StackMode::LoadOrder, StackMode::Legacy] {
            p.stacking = Some(mode);
            paths.save_profile(&p).unwrap();
            paths.activate_profile("default").unwrap();
            assert_eq!(staged_plan(&paths, "default").stack_mode(), mode);
        }
    }

    #[test]
    fn clones_copy_stacking_and_imports_get_dependencies() {
        let (_t, paths) = paths_with_default();
        make_existing(&paths, "default");
        let a = paths.clone_profile("default", "a", "A").unwrap();
        assert_eq!((a.stacking, a.notices_seen.len()), (None, 0));
        let b = paths
            .create_profile_with_id(Some("b"), "B", Some("default"), None, None)
            .unwrap();
        assert_eq!((b.stacking, b.notices_seen.len()), (None, 0));
        let mut src = paths.get_profile("default").unwrap();
        src.stacking = Some(StackMode::LoadOrder);
        src.mark_notice_seen(NOTICE_STACKING);
        paths.save_profile(&src).unwrap();
        let c = paths.clone_profile("default", "c", "C").unwrap();
        assert_eq!(c.stacking, Some(StackMode::LoadOrder));
        assert!(c.stacking_acknowledged());

        let pack = crate::devtools::ModpackManifest {
            schema_version: 1,
            name: "Pack".into(),
            description: String::new(),
            exported_at: String::new(),
            mods: Vec::new(),
            mod_priorities: HashMap::new(),
            mod_settings: Default::default(),
        };
        let d = paths.apply_modpack_as_profile(&pack, "d").unwrap();
        assert_eq!(d.stacking, Some(StackMode::Dependencies));
        assert!(d.stacking_acknowledged());
    }

    #[test]
    fn without_mods_stages_an_empty_plan_and_no_overlay() {
        let (_t, paths) = paths_with_default();
        let mut p = paths.get_profile("default").unwrap();
        p.enabled_mods = vec!["gone".into()];
        p.disabled_patches = vec!["gone:main_lua:1".into()];
        paths.save_profile(&p).unwrap();
        let overwrite = paths.profiles_dir.join("default").join("overwrite").join("dropped");
        fs::create_dir_all(&overwrite).unwrap();
        fs::write(overwrite.join("qm.toml"), "[mod]").unwrap();
        paths.activate_profile_without_mods("default").unwrap();
        let plan = staged_plan(&paths, "default");
        assert!(plan.enabled_mods.is_empty());
        assert!(plan.disabled_patches.is_empty());
        let staged = paths.profiles_dir.join("default").join("Mods");
        assert!(!staged.join("dropped").exists());
        let cfg = LaunchConfig::load_from_mod_dir(&staged).unwrap().unwrap();
        assert!(!cfg.vanilla_mode);
        assert_eq!(paths.load_settings().active_profile_id, "default");
    }

    #[test]
    fn overwrite_bucket_cannot_replace_plan() {
        let (_t, paths) = paths_with_default();
        let overwrite = paths.profiles_dir.join("default").join("overwrite").join("qm");
        fs::create_dir_all(&overwrite).unwrap();
        fs::write(
            overwrite.join("load-order.json"),
            r#"{"profile_id": "evil", "generated_at": "x", "enabled_mods": ["all"]}"#,
        )
        .unwrap();
        fs::write(overwrite.join("Launch-Config.json"), "{}").unwrap();
        fs::write(overwrite.join("notes.txt"), "kept").unwrap();
        let a = paths.activate_profile_reporting("default").unwrap();
        assert!(
            a.warnings.iter().any(|w| w.contains("overwrite/qm/load-order.json")),
            "{:?}",
            a.warnings
        );
        assert!(
            a.warnings.iter().any(|w| w.contains("overwrite/qm/launch-config.json")),
            "{:?}",
            a.warnings
        );
        let plan = staged_plan(&paths, "default");
        assert_eq!(plan.profile_id, "default");
        assert_eq!(plan.schema_version, 3);
        let staged = paths.profiles_dir.join("default").join("Mods");
        let cfg = LaunchConfig::load_from_mod_dir(&staged).unwrap().unwrap();
        assert_eq!(cfg.profile_id.as_deref(), Some("default"));
        // Other overwrite files still overlay as before.
        assert_eq!(fs::read_to_string(staged.join("qm").join("notes.txt")).unwrap(), "kept");
    }

    #[test]
    fn plan_invalid_blocks_activation() {
        let (_t, paths) = paths_with_default();
        CORRUPT_PLAN_AFTER_OVERLAY.with(|c| c.set(true));
        let res = paths.activate_profile_reporting("default");
        let legacy = paths.activate_profile("default");
        CORRUPT_PLAN_AFTER_OVERLAY.with(|c| c.set(false));
        match res {
            Err(ActivateError::InvalidPlan(m)) => assert!(m.contains("load-order.json"), "{m}"),
            other => panic!("expected InvalidPlan, got {other:?}"),
        }
        assert!(legacy.unwrap_err().contains("load-order.json"));
        // The journal holds no half-done swap.
        assert!(StateJournal::load(&paths.journal_file).pending_transaction.is_none());
        // The next activation writes a clean plan again.
        paths.activate_profile("default").unwrap();
        staged_plan(&paths, "default");
    }

    #[test]
    fn a_staged_launch_config_must_read_back() {
        let (_t, paths) = paths_with_default();
        paths.activate_profile("default").unwrap();
        let staged = paths.profiles_dir.join("default").join("Mods");
        check_staged_plan(&staged).unwrap();
        let file = staged.join("qm").join("launch-config.json");
        fs::write(&file, "{\"profile_id\": ").unwrap();
        let err = check_staged_plan(&staged).unwrap_err();
        assert!(err.starts_with("launch-config.json is not valid: "), "{err}");
        fs::remove_file(&file).unwrap();
        let err = check_staged_plan(&staged).unwrap_err();
        assert!(err.contains("launch-config.json is missing"), "{err}");
    }

    #[test]
    fn a_broken_patch_report_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let (v, e) = read_patch_report_json(dir.path());
        assert!(v.is_null() && e.is_none(), "a missing report is no error");
        let qm = dir.path().join("Mods").join("qm");
        fs::create_dir_all(&qm).unwrap();
        fs::write(qm.join("patch-report.json"), "{\"qm_version\": ").unwrap();
        let (v, e) = read_patch_report_json(dir.path());
        assert!(v.is_null());
        let e = e.unwrap();
        assert!(e.starts_with("patch-report.json is not valid: "), "{e}");
        fs::write(qm.join("patch-report.json"), "{\"qm_version\": \"1\"}").unwrap();
        let (v, e) = read_patch_report_json(dir.path());
        assert_eq!(v["qm_version"], "1");
        assert!(e.is_none());
        // Not a file: it cannot be read.
        fs::remove_file(qm.join("patch-report.json")).unwrap();
        fs::create_dir_all(qm.join("patch-report.json")).unwrap();
        let (_, e) = read_patch_report_json(dir.path());
        assert!(e.unwrap().starts_with("patch-report.json could not be read: "));
    }

    #[test]
    fn corrupt_next_plan_breaks_one_activation() {
        let (_t, paths) = paths_with_default();
        corrupt_next_staged_plan();
        let res = paths.activate_profile_reporting("default");
        assert!(matches!(res, Err(ActivateError::InvalidPlan(_))), "{res:?}");
        paths.activate_profile("default").unwrap();
        staged_plan(&paths, "default");
    }

    #[test]
    fn broken_profile_json_is_backed_up_not_overwritten() {
        let (_t, paths) = paths_with_default();
        let file = paths.profiles_dir.join("default").join("profile.json");
        let broken = "{\"id\": \"default\", \"name\": ";
        fs::write(&file, broken).unwrap();
        let (profiles, notices) = paths.list_profiles_reporting();
        assert!(!profiles.iter().any(|p| p.id == "default"));
        assert!(profiles.iter().any(|p| p.id == VANILLA_ID));
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("profile.json.unreadable-1"), "{}", notices[0]);
        let backup = file.with_file_name("profile.json.unreadable-1");
        assert_eq!(fs::read_to_string(&backup).unwrap(), broken);
        // No default was synthesized over it.
        assert!(!file.exists());
        // A second broken file gets the next number.
        fs::write(&file, "nope").unwrap();
        let (_, notices) = paths.list_profiles_reporting();
        assert!(notices[0].contains("profile.json.unreadable-2"), "{}", notices[0]);
        assert_eq!(fs::read_to_string(&backup).unwrap(), broken);
    }

    #[test]
    fn unreadable_vanilla_files_are_kept_before_the_rewrite() {
        let (_t, paths) = paths_with_default();
        let slot = paths.vanilla_slot_file();
        let profile = paths.vanilla_profile_file();
        paths.list_save_profiles();
        assert!(matches!(paths.vanilla_slot_on_disk(), Ok(Some(_))));
        fs::write(&slot, "{broken").unwrap();
        fs::write(&profile, "{broken").unwrap();
        assert!(paths.vanilla_slot_on_disk().is_err());
        assert!(paths.vanilla_profile_on_disk().is_err());
        paths.repair_vanilla_builtins().unwrap();
        for f in [&slot, &profile] {
            let kept = f.with_file_name(format!(
                "{}.unreadable-1",
                f.file_name().unwrap().to_string_lossy()
            ));
            assert_eq!(fs::read_to_string(kept).unwrap(), "{broken");
        }
        assert!(matches!(paths.vanilla_slot_on_disk(), Ok(Some(s)) if s.locked_vanilla));
        assert!(matches!(paths.vanilla_profile_on_disk(), Ok(Some(_))));

        // Listing the slots keeps a broken sidecar too.
        fs::write(&slot, "{again").unwrap();
        assert!(paths.list_save_profiles().iter().any(|s| s.id == VANILLA_ID));
        let kept = slot.with_file_name("save_profile.json.unreadable-2");
        assert_eq!(fs::read_to_string(kept).unwrap(), "{again");
        assert!(matches!(paths.vanilla_slot_on_disk(), Ok(Some(_))));
    }

    #[test]
    fn notice_from_a_plain_read_waits_for_the_service() {
        let (_t, paths) = paths_with_default();
        let file = paths.profiles_dir.join("default").join("profile.json");
        fs::write(&file, "nope").unwrap();
        assert!(paths.get_profile("default").is_err());
        let notices = paths.take_profile_notices();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("profile.json.unreadable-1"), "{}", notices[0]);
        assert!(paths.take_profile_notices().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_strict_save_backup_fails_when_a_live_save_cannot_be_copied() {
        use std::os::windows::fs::OpenOptionsExt;
        let (t, paths) = paths_with_default();
        let live = t.path().join("live");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join("saveMNB3.lua"), "new").unwrap();
        let mut s = paths.load_settings();
        s.save_dir_path = Some(live.clone());
        let slot = s.active_save_profile_id.clone();
        paths.save_settings(&s).unwrap();
        // The slot still holds an older copy from the last switch.
        let active = paths.saves_dir.join(&slot).join("active");
        fs::create_dir_all(&active).unwrap();
        fs::write(active.join("saveMNB3.lua"), "old").unwrap();
        let backups = paths.saves_dir.join(&slot).join("backups");
        let snapshots = || {
            fs::read_dir(&backups)
                .map(|rd| rd.filter_map(Result::ok).count())
                .unwrap_or(0)
        };

        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(live.join("saveMNB3.lua"))
            .unwrap();
        let err = paths.create_save_backup_strict(&slot, "trial 1").unwrap_err();
        assert!(err.contains("saveMNB3.lua"), "{err}");
        assert_eq!(snapshots(), 0, "no snapshot of the older save is kept");
        // The legacy backup still counts copy errors as made.
        assert!(paths.create_save_backup(&slot, "manual").is_ok());
        assert_eq!(snapshots(), 1);
        drop(lock);

        let b = paths.create_save_backup_strict(&slot, "trial 1").unwrap();
        assert_eq!(fs::read_to_string(b.path.join("saveMNB3.lua")).unwrap(), "new");
    }

    #[cfg(windows)]
    #[test]
    fn locked_profile_json_is_skipped_not_renamed() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_t, paths) = paths_with_default();
        let file = paths.profiles_dir.join("default").join("profile.json");
        let lock = fs::OpenOptions::new().read(true).share_mode(0).open(&file).unwrap();
        let (profiles, notices) = paths.list_profiles_reporting();
        assert!(!profiles.iter().any(|p| p.id == "default"));
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(notices[0].contains("not listed for now"), "{}", notices[0]);
        // Reported once, not on every read.
        let (_, again) = paths.list_profiles_reporting();
        assert!(again.is_empty(), "{again:?}");
        drop(lock);
        assert!(file.is_file());
        assert!(!file.with_file_name("profile.json.unreadable-1").exists());
        assert!(paths.get_profile("default").is_ok());
    }

    #[test]
    fn save_profile_is_atomic() {
        let (_t, paths) = paths_with_default();
        let mut p = paths.get_profile("default").unwrap();
        p.name = "Renamed".into();
        paths.save_profile(&p).unwrap();
        let dir = paths.profiles_dir.join("default");
        assert!(!dir.join("profile.json.tmp").exists());
        assert_eq!(paths.get_profile("default").unwrap().name, "Renamed");
    }

    #[test]
    fn concurrent_profile_saves_never_share_a_temp_file() {
        let (_t, paths) = paths_with_default();
        let paths = std::sync::Arc::new(paths);
        let base = paths.get_profile("default").unwrap();
        let workers: Vec<_> = (0..4)
            .map(|w| {
                let paths = paths.clone();
                let mut p = base.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        p.name = format!("w{w}-{i}");
                        paths.save_profile(&p).unwrap();
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }
        let name = paths.get_profile("default").unwrap().name;
        assert!(name.ends_with("-49"), "{name}");
        let dir = paths.profiles_dir.join("default");
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "profile.json")
            .collect();
        assert!(left.iter().all(|n| !n.ends_with(".tmp") && !n.contains("unreadable")), "{left:?}");
    }
}
