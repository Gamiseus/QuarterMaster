use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use qm_schema::sha256_hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMethod {
    SteamUrl,
    DirectExe,
    Vanilla,
}

impl Default for LaunchMethod {
    fn default() -> Self {
        Self::DirectExe
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherSettings {
    pub game_exe_path: Option<PathBuf>,
    pub save_dir_path: Option<PathBuf>,
    pub workspace_mods_dir: Option<PathBuf>,
    #[serde(default = "default_theme")]
    pub theme: String, // "field_command" | "modern_slate"
    #[serde(default)]
    pub default_launch_method: LaunchMethod,
    #[serde(default)]
    pub opt_in_prereleases: bool,
    #[serde(default = "default_true")]
    pub auto_backup_saves: bool,
    #[serde(default = "default_max_backups")]
    pub max_save_backups: usize,
    #[serde(default)]
    pub disable_console_on_launch: bool,
    #[serde(default)]
    pub first_run_completed: bool,
    #[serde(default = "default_profile_id")]
    pub active_profile_id: String,
    #[serde(default = "default_save_profile_id")]
    pub active_save_profile_id: String,
    #[serde(default)]
    pub portable_mode: bool,
    /// Optional personal GitHub token to raise API rate limits from 60/hr to 5,000/hr.
    #[serde(default)]
    pub github_token: Option<String>,
    /// Per-mod opt-in auto-update flags (`mod_id -> bool`).
    #[serde(default)]
    pub auto_update_mods: HashMap<String, bool>,
    /// Last known vanilla `mnb.exe` SHA-256 fingerprint to detect game updates across launches.
    #[serde(default)]
    pub last_known_exe_sha256: Option<String>,
}

fn default_theme() -> String {
    "field_command".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_backups() -> usize {
    15
}

fn default_profile_id() -> String {
    "default".to_string()
}

fn default_save_profile_id() -> String {
    "default".to_string()
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            game_exe_path: detect_game_installation(),
            save_dir_path: detect_save_directory(),
            workspace_mods_dir: detect_workspace_mods_dir(),
            theme: default_theme(),
            default_launch_method: LaunchMethod::DirectExe,
            opt_in_prereleases: false,
            auto_backup_saves: true,
            max_save_backups: 15,
            disable_console_on_launch: false,
            first_run_completed: false,
            active_profile_id: default_profile_id(),
            active_save_profile_id: default_save_profile_id(),
            portable_mode: false,
            github_token: None,
            auto_update_mods: HashMap::new(),
            last_known_exe_sha256: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarterMasterPaths {
    pub root_dir: PathBuf,
    pub library_dir: PathBuf,
    pub profiles_dir: PathBuf,
    pub saves_dir: PathBuf,
    pub vanilla_cache_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub settings_file: PathBuf,
    pub journal_file: PathBuf,
    pub is_portable: bool,
}

impl QuarterMasterPaths {
    pub fn resolve(custom_root: Option<&Path>) -> Self {
        let (root_dir, is_portable) = if let Some(r) = custom_root {
            (r.to_path_buf(), true)
        } else if let Ok(custom_env_dir) = std::env::var("QM_DATA_DIR").map(PathBuf::from) {
            (custom_env_dir, true)
        } else if std::env::var("QM_PORTABLE").map(|v| v == "1").unwrap_or(false) {
            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."));
            (exe_dir.join("qm-data"), true)
        } else if let Some(exe_dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            if exe_dir.join("portable.txt").is_file() || exe_dir.join(".portable").is_file() {
                (exe_dir.join("qm-data"), true)
            } else {
                let appdata = dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("QuarterMaster");
                (appdata, false)
            }
        } else {
            let appdata = dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("QuarterMaster");
            (appdata, false)
        };

        let paths = Self {
            library_dir: root_dir.join("library"),
            profiles_dir: root_dir.join("profiles"),
            saves_dir: root_dir.join("saves"),
            vanilla_cache_dir: root_dir.join("vanilla-cache"),
            cache_dir: root_dir.join("cache"),
            logs_dir: root_dir.join("logs"),
            backups_dir: root_dir.join("backups"),
            settings_file: root_dir.join("settings.json"),
            journal_file: root_dir.join("state-journal.json"),
            root_dir,
            is_portable,
        };

        paths.ensure_directories();
        let _ = paths.recover_if_needed();
        paths
    }

    pub fn ensure_directories(&self) {
        for dir in [
            &self.root_dir,
            &self.library_dir,
            &self.profiles_dir,
            &self.saves_dir,
            &self.vanilla_cache_dir,
            &self.cache_dir,
            &self.logs_dir,
            &self.backups_dir,
        ] {
            let _ = fs::create_dir_all(dir);
        }
    }

    pub fn load_settings(&self) -> LauncherSettings {
        if let Ok(raw) = fs::read_to_string(&self.settings_file) {
            if let Ok(mut s) = serde_json::from_str::<LauncherSettings>(&raw) {
                if s.game_exe_path.is_none() {
                    s.game_exe_path = detect_game_installation();
                }
                if s.save_dir_path.is_none() {
                    s.save_dir_path = detect_save_directory();
                }
                if s.workspace_mods_dir.is_none() {
                    s.workspace_mods_dir = detect_workspace_mods_dir();
                }
                return s;
            }
        }
        let s = LauncherSettings::default();
        let _ = self.save_settings(&s);
        s
    }

    pub fn save_settings(&self, settings: &LauncherSettings) -> Result<(), String> {
        let json = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Failed to serialize settings.json: {e}"))?;
        fs::write(&self.settings_file, json)
            .map_err(|e| format!("Failed to write {}: {e}", self.settings_file.display()))
    }

    /// Auto-detects all system paths (Steam game installation, save directory, mod library storage).
    pub fn auto_detect_all_paths(&self) -> DetectedPaths {
        let game_exe = detect_game_installation();
        let save_dir = detect_save_directory();
        let is_steam = game_exe
            .as_ref()
            .map(|p| p.to_string_lossy().to_lowercase().contains("steamapps"))
            .unwrap_or(false);
        let game_version = game_exe.as_ref().and_then(|_p| {
            let meta_file = self.vanilla_cache_dir.join(".vanilla-meta.json");
            if let Ok(raw) = fs::read_to_string(&meta_file) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(v) = val.get("info").and_then(|i| i.get("game_version")).and_then(|v| v.as_str()) {
                        return Some(v.to_string());
                    }
                }
            }
            Some("0.5.3b".to_string())
        });

        DetectedPaths {
            game_exe_path: game_exe,
            save_dir_path: save_dir,
            library_dir_path: self.library_dir.clone(),
            is_steam,
            game_version,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedPaths {
    pub game_exe_path: Option<PathBuf>,
    pub save_dir_path: Option<PathBuf>,
    pub library_dir_path: PathBuf,
    pub is_steam: bool,
    pub game_version: Option<String>,
}

/// Auto-detects the Steam client executable (`Steam.exe`).
pub fn detect_steam_exe() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for (hive, value_name) in [
            (r"HKCU\Software\Valve\Steam", "SteamExe"),
            (r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", "SteamExe"),
            (r"HKLM\SOFTWARE\Valve\Steam", "SteamExe"),
        ] {
            if let Ok(output) = std::process::Command::new("reg")
                .args(["query", hive, "/v", value_name])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    for line in text.lines() {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 3 && parts[1].starts_with("REG_") {
                            if let Some(idx) = line.find(parts[1]) {
                                let path_str = line[idx + parts[1].len()..].trim().replace('/', "\\");
                                let path = PathBuf::from(path_str);
                                if path.is_file() {
                                    return Some(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    for drive in ['C', 'D', 'E', 'F'] {
        for sub in [
            r"Program Files (x86)\Steam\steam.exe",
            r"Program Files\Steam\steam.exe",
            r"Steam\steam.exe",
        ] {
            let p = PathBuf::from(format!(r"{}:\{}", drive, sub));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Checks if Steam is currently running.
pub fn is_steam_running() -> bool {
    #[cfg(windows)]
    {
        if let Ok(output) = std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            return text.to_lowercase().contains("steam.exe");
        }
    }
    false
}

/// Auto-detects the Steam game installation of Mud and Blood (`mnb.exe`, Steam AppID 1391530).
pub fn detect_steam_game_installation() -> Option<PathBuf> {
    let mut steam_roots = Vec::new();

    // 1. Query Windows Registry for SteamPath (HKCU and HKLM)
    #[cfg(windows)]
    {
        for (hive, value_name) in [
            (r"HKCU\Software\Valve\Steam", "SteamPath"),
            (r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
            (r"HKLM\SOFTWARE\Valve\Steam", "InstallPath"),
        ] {
            if let Ok(output) = std::process::Command::new("reg")
                .args(["query", hive, "/v", value_name])
                .output()
            {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout);
                    for line in text.lines() {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 3 && parts[1].starts_with("REG_") {
                            if let Some(idx) = line.find(parts[1]) {
                                let path_str = line[idx + parts[1].len()..].trim();
                                let path = PathBuf::from(path_str);
                                if path.is_dir() && !steam_roots.contains(&path) {
                                    steam_roots.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Add common default Steam root folders across common drive letters
    for drive in ['C', 'D', 'E', 'F', 'G'] {
        for sub in [
            r"Program Files (x86)\Steam",
            r"Program Files\Steam",
            r"Steam",
            r"SteamLibrary",
            r"WindowsGames\SteamLibrary",
            r"Games\SteamLibrary",
            r"Games\Steam",
        ] {
            let p = PathBuf::from(format!(r"{}:\{}", drive, sub));
            if p.is_dir() && !steam_roots.contains(&p) {
                steam_roots.push(p);
            }
        }
    }

    // 3. For each steam root, parse steamapps/libraryfolders.vdf and gather all library paths
    let mut library_paths = Vec::new();
    for root in &steam_roots {
        if !library_paths.contains(root) {
            library_paths.push(root.clone());
        }
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(vdf_text) = fs::read_to_string(&vdf) {
            let path_re = regex_lite::Regex::new(r#""path"\s+"([^"]+)""#).unwrap();
            for cap in path_re.captures_iter(&vdf_text) {
                let lib_path = PathBuf::from(cap[1].replace("\\\\", "\\"));
                if lib_path.is_dir() && !library_paths.contains(&lib_path) {
                    library_paths.push(lib_path);
                }
            }
        }
    }

    // 4. In each library path, check for appmanifest_1391530.acf and common install names
    for lib in &library_paths {
        let manifest = lib.join("steamapps").join("appmanifest_1391530.acf");
        if let Ok(manifest_text) = fs::read_to_string(&manifest) {
            let dir_re = regex_lite::Regex::new(r#""installdir"\s+"([^"]+)""#).unwrap();
            if let Some(cap) = dir_re.captures(&manifest_text) {
                let exe = lib
                    .join("steamapps")
                    .join("common")
                    .join(&cap[1])
                    .join("mnb.exe");
                if exe.is_file() {
                    return Some(exe);
                }
            }
        }

        // Direct candidate names under steamapps/common/
        for folder_name in ["MudandBlood", "Mud and Blood", "mudandblood", "mud and blood"] {
            let exe = lib
                .join("steamapps")
                .join("common")
                .join(folder_name)
                .join("mnb.exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
    }

    None
}

/// Auto-detects `mnb.exe` prioritizing real Steam installation, standard game paths, and falling back to workspace.
pub fn detect_game_installation() -> Option<PathBuf> {
    // 1. Check Steam installation (primary source)
    if let Some(steam_exe) = detect_steam_game_installation() {
        return Some(steam_exe);
    }

    // 2. Check standard non-Steam installation directories
    for drive in ['C', 'D', 'E', 'F'] {
        for folder in [
            r"Games\MudandBlood",
            r"Games\Mud and Blood",
            r"GOG Games\Mud and Blood",
            r"MudandBlood",
            r"Mud and Blood",
        ] {
            let candidate = PathBuf::from(format!(r"{}:\{}\mnb.exe", drive, folder));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 3. Fallback: only if no installed game was found on system, check exe/cwd
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(5) {
            let p1 = ancestor.join("game").join("mnb.exe");
            if p1.is_file() {
                return Some(p1);
            }
            let p2 = ancestor.join("mnb.exe");
            if p2.is_file() {
                return Some(p2);
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let p1 = cwd.join("game").join("mnb.exe");
        if p1.is_file() {
            return Some(p1);
        }
        let p2 = cwd.join("mnb.exe");
        if p2.is_file() {
            return Some(p2);
        }
    }

    None
}

/// Auto-detects the LÖVE save directory for Mud and Blood (`%APPDATA%\mnb\`).
pub fn detect_save_directory() -> Option<PathBuf> {
    if let Some(appdata) = dirs::data_dir() {
        let mnb_save = appdata.join("mnb");
        if mnb_save.is_dir() {
            return Some(mnb_save);
        }
        let love_save = appdata.join("LOVE").join("mnb");
        if love_save.is_dir() {
            return Some(love_save);
        }
        return Some(mnb_save);
    }
    None
}

/// Auto-detects the `MnB-Mods` development repository if present in the workspace.
pub fn detect_workspace_mods_dir() -> Option<PathBuf> {
    if let Ok(env_ws) = std::env::var("QM_WORKSPACE_DIR") {
        let p = PathBuf::from(env_ws);
        if p.is_dir() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("MnB-Mods"));
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.join("MnB-Mods"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().take(5) {
            candidates.push(ancestor.join("MnB-Mods"));
        }
    }
    candidates.into_iter().find(|p| p.is_dir())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VanillaGameInfo {
    pub exe_path: PathBuf,
    pub exe_sha256: String,
    pub game_version: String,
    pub steam_app_id: Option<String>,
    pub cache_dir: PathBuf,
    pub lua_file_count: usize,
}

impl QuarterMasterPaths {
    /// Extracts and caches all vanilla `.lua` files from `mnb.exe` (or `decompile/` fallback)
    /// into `vanilla-cache/<version>-<hash8>/`, and detects the exact game version (e.g. `0.5.3b`).
    pub fn ensure_vanilla_cache(&self, game_exe: &Path) -> Result<VanillaGameInfo, String> {
        if !game_exe.is_file() {
            return Err(format!("Game executable not found at {}", game_exe.display()));
        }

        let meta = fs::metadata(game_exe)
            .map_err(|e| format!("Failed to stat {}: {e}", game_exe.display()))?;
        let exe_size = meta.len();
        let exe_mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let meta_file = self.vanilla_cache_dir.join(".vanilla-meta.json");
        if let Ok(raw) = fs::read_to_string(&meta_file) {
            #[derive(serde::Deserialize)]
            struct CachedMeta {
                exe_path: String,
                exe_size: u64,
                exe_mtime: u64,
                info: VanillaGameInfo,
            }
            if let Ok(cached) = serde_json::from_str::<CachedMeta>(&raw) {
                if cached.exe_path == game_exe.to_string_lossy()
                    && cached.exe_size == exe_size
                    && cached.exe_mtime == exe_mtime
                    && cached.info.cache_dir.join("main.lua").is_file()
                {
                    return Ok(cached.info);
                }
            }
        }

        let exe_bytes = fs::read(game_exe)
            .map_err(|e| format!("Failed to read {}: {e}", game_exe.display()))?;
        let exe_sha256 = sha256_hex(&exe_bytes);
        let hash8 = &exe_sha256[..8];

        let steam_app_id = game_exe
            .parent()
            .and_then(|d| fs::read_to_string(d.join("steam_appid.txt")).ok())
            .map(|s| s.trim().to_string());

        let cache_dir = self.vanilla_cache_dir.join(format!("mnb-{hash8}"));
        let main_lua_cached = cache_dir.join("main.lua");

        let mut lua_file_count = 0usize;
        if !main_lua_cached.is_file() {
            fs::create_dir_all(&cache_dir)
                .map_err(|e| format!("Failed to create vanilla cache dir: {e}"))?;

            let file = fs::File::open(game_exe)
                .map_err(|e| format!("Failed to open {}: {e}", game_exe.display()))?;
            if let Ok(mut archive) = ZipArchive::new(file) {
                for i in 0..archive.len() {
                    let Ok(mut entry) = archive.by_index(i) else {
                        continue;
                    };
                    if entry.is_dir() || !entry.name().ends_with(".lua") {
                        continue;
                    }
                    let Some(rel) = entry.enclosed_name() else {
                        continue;
                    };
                    let dest = cache_dir.join(rel);
                    if let Some(p) = dest.parent() {
                        let _ = fs::create_dir_all(p);
                    }
                    let mut buf = Vec::new();
                    if entry.read_to_end(&mut buf).is_ok() && fs::write(&dest, &buf).is_ok() {
                        lua_file_count += 1;
                    }
                }
            }
        } else {
            for entry in walkdir::WalkDir::new(&cache_dir)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.path().is_file()
                    && entry.path().extension().and_then(|x| x.to_str()) == Some("lua")
                {
                    lua_file_count += 1;
                }
            }
        }

        let game_version = fs::read_to_string(&main_lua_cached)
            .ok()
            .and_then(|s| extract_game_version_from_main_lua(&s))
            .unwrap_or_else(|| "0.5.3b".to_string());

        let info = VanillaGameInfo {
            exe_path: game_exe.to_path_buf(),
            exe_sha256,
            game_version,
            steam_app_id,
            cache_dir,
            lua_file_count,
        };

        let _ = fs::create_dir_all(&self.vanilla_cache_dir);
        let _ = fs::write(
            &meta_file,
            serde_json::to_string(&serde_json::json!({
                "exe_path": game_exe.to_string_lossy(),
                "exe_size": exe_size,
                "exe_mtime": exe_mtime,
                "info": info
            }))
            .unwrap_or_default(),
        );

        Ok(info)
    }

    /// Loads all vanilla `.lua` files into a map (`relative_path -> content`) for fast in-memory simulation.
    pub fn load_vanilla_lua_files(
        &self,
        game_exe: Option<&Path>,
    ) -> Result<(String, HashMap<String, String>), String> {
        if let Some(exe) = game_exe {
            if exe.is_file() {
                let info = self.ensure_vanilla_cache(exe)?;
                let files = read_lua_tree(&info.cache_dir);
                return Ok((info.game_version, files));
            }
        }

        // Fallback to workspace `decompile/` directory if present
        if let Ok(cwd) = std::env::current_dir() {
            for candidate in [cwd.join("decompile"), cwd.parent().map(|p| p.join("decompile")).unwrap_or_default()] {
                if candidate.join("main.lua").is_file() {
                    let files = read_lua_tree(&candidate);
                    let ver = files
                        .get("main.lua")
                        .and_then(|s| extract_game_version_from_main_lua(s))
                        .unwrap_or_else(|| "0.5.3b".to_string());
                    return Ok((ver, files));
                }
            }
        }

        Err("Could not locate vanilla game files (mnb.exe or decompile/)".to_string())
    }
}

fn read_lua_tree(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if p.is_file() && p.extension().and_then(|x| x.to_str()) == Some("lua") {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            if let Ok(content) = fs::read_to_string(p) {
                map.insert(rel, content);
            }
        }
    }
    map
}

pub fn extract_game_version_from_main_lua(main_lua: &str) -> Option<String> {
    let re = regex_lite::Regex::new(r#"\bversion\s*=\s*"([0-9]+\.[0-9]+(?:\.[0-9]+)?[a-zA-Z]?)""#)
        .ok()?;
    re.captures(main_lua).map(|c| c[1].to_string())
}

/// Crash-resilient transaction journal (`state-journal.json`) for atomic profile & save swaps.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateJournal {
    pub active_profile_id: Option<String>,
    pub active_save_profile_id: Option<String>,
    pub pending_transaction: Option<PendingTransaction>,
    pub last_completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTransaction {
    pub id: String,
    pub kind: String, // "profile_swap" | "save_swap"
    pub started_at: String,
    pub from_id: Option<String>,
    pub to_id: String,
    pub backup_path: Option<PathBuf>,
}

impl StateJournal {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize state-journal.json: {e}"))?;
        fs::write(path, json).map_err(|e| format!("Failed to write {}: {e}", path.display()))
    }

    pub fn begin_transaction(
        &mut self,
        journal_path: &Path,
        kind: &str,
        from_id: Option<String>,
        to_id: &str,
        backup_path: Option<PathBuf>,
    ) -> Result<(), String> {
        let now = jiff::Zoned::now().to_string();
        self.pending_transaction = Some(PendingTransaction {
            id: format!("tx-{}-{}", kind, jiff::Timestamp::now().as_millisecond()),
            kind: kind.to_string(),
            started_at: now,
            from_id,
            to_id: to_id.to_string(),
            backup_path,
        });
        self.save(journal_path)
    }

    pub fn commit_transaction(&mut self, journal_path: &Path) -> Result<(), String> {
        if let Some(tx) = self.pending_transaction.take() {
            if tx.kind == "profile_swap" {
                self.active_profile_id = Some(tx.to_id);
            } else if tx.kind == "save_swap" {
                self.active_save_profile_id = Some(tx.to_id);
            }
        }
        self.last_completed_at = Some(jiff::Zoned::now().to_string());
        self.save(journal_path)
    }
}

impl QuarterMasterPaths {
    /// Checks `state-journal.json` on startup for an interrupted profile or save swap transaction
    /// and rolls back or completes it so the game directory and saves are never left in a half-swapped state.
    pub fn recover_if_needed(&self) -> Option<String> {
        let mut journal = StateJournal::load(&self.journal_file);
        let tx = journal.pending_transaction.clone()?;
        let msg = format!(
            "Recovered from interrupted {} transaction '{}' (target '{}', started at {})",
            tx.kind, tx.id, tx.to_id, tx.started_at
        );
        // Clear pending transaction first to avoid recursion, then re-apply the target profile/save cleanly
        journal.pending_transaction = None;
        let _ = journal.save(&self.journal_file);

        if tx.kind == "profile_swap" {
            let fallback_id = tx.from_id.as_deref().unwrap_or(&tx.to_id);
            let _ = self.activate_profile(fallback_id);
        } else if tx.kind == "save_swap" {
            let fallback_id = tx.from_id.as_deref().unwrap_or(&tx.to_id);
            let _ = self.activate_save_profile(fallback_id);
        }
        Some(msg)
    }

    /// Detects whether `mnb.exe` has changed since the last known SHA-256 fingerprint (Section 16 & 17).
    /// Returns `Some((previous_sha256, current_sha256))` if a game update occurred.
    pub fn check_game_update_occurred(&self) -> Option<(String, String)> {
        let mut settings = self.load_settings();
        let exe = settings.game_exe_path.as_ref()?;
        let info = self.ensure_vanilla_cache(exe).ok()?;
        let current_sha = info.exe_sha256;

        match settings.last_known_exe_sha256.clone() {
            None => {
                settings.last_known_exe_sha256 = Some(current_sha);
                let _ = self.save_settings(&settings);
                None
            }
            Some(prev_sha) if prev_sha != current_sha => Some((prev_sha, current_sha)),
            _ => None,
        }
    }

    /// Acknowledges a detected game update and updates `last_known_exe_sha256` after re-validating patches.
    pub fn acknowledge_game_update(&self) -> Result<String, String> {
        let mut settings = self.load_settings();
        let exe = settings
            .game_exe_path
            .clone()
            .ok_or_else(|| "Game executable path not configured".to_string())?;
        let info = self.ensure_vanilla_cache(&exe)?;
        settings.last_known_exe_sha256 = Some(info.exe_sha256.clone());
        self.save_settings(&settings)?;
        Ok(info.exe_sha256)
    }

    /// Decompiles and extracts all assets (Lua, sprites, sounds, fonts, shaders) from the game executable into the target directory.
    pub fn extract_all_vanilla_assets(&self, game_exe: &Path, target_dir: &Path) -> Result<usize, String> {
        let file = fs::File::open(game_exe)
            .map_err(|e| format!("Failed to open {}: {e}", game_exe.display()))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|e| format!("Failed to read zip archive from {}: {e}", game_exe.display()))?;
        let _ = fs::create_dir_all(target_dir);

        let mut count = 0usize;
        for i in 0..archive.len() {
            let Ok(mut entry) = archive.by_index(i) else { continue };
            if entry.is_dir() { continue };
            let Some(rel) = entry.enclosed_name() else { continue };
            let dest = target_dir.join(rel);
            if let Some(p) = dest.parent() {
                let _ = fs::create_dir_all(p);
            }
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_ok() && fs::write(&dest, &buf).is_ok() {
                count += 1;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_steam_game_detection_prioritizes_real_steam_copy() {
        let detected = detect_steam_game_installation();
        println!("Detected Steam Game: {:?}", detected);
        assert!(detected.is_some(), "Steam Mud and Blood installation should be detected on this machine");
        if let Some(ref path) = detected {
            assert!(path.is_file(), "Detected game exe must exist");
            let path_str = path.to_string_lossy().to_lowercase();
            assert!(
                path_str.contains("mudandblood") || path_str.contains("mud and blood"),
                "Path should be for Mud and Blood"
            );
            assert!(path_str.ends_with("mnb.exe"), "Path must end with mnb.exe");
        }
    }

    #[test]
    fn test_detect_game_installation_resolves_valid_exe() {
        let detected = detect_game_installation();
        if let Some(ref path) = detected {
            assert!(path.is_file());
            assert!(path.to_string_lossy().ends_with("mnb.exe"));
        }
    }

    #[test]
    fn test_auto_detect_all_paths_returns_consistent_structure() {
        let temp = tempfile::tempdir().unwrap();
        let paths = QuarterMasterPaths::resolve(Some(temp.path()));
        let detected = paths.auto_detect_all_paths();
        assert_eq!(detected.library_dir_path, paths.library_dir);
    }
}
