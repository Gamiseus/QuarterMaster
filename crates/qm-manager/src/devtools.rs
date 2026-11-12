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
    sha256_hex, DiagnosticSeverity, ManifestDiagnostic, ModManifest, ModSettingsStore,
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

    pub fn apply_modpack_as_profile(
        &self,
        manifest: &ModpackManifest,
        new_profile_id: &str,
    ) -> Result<ModProfile, String> {
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

/// Data model for QuarterMaster modding templates with inline documentation and interactive preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDefinition {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub descriptor: &'static str,
    pub provider: &'static str,
    pub description: &'static str,
    pub preview_code: &'static str,
}

/// Starter `faction.toml` for the `faction_mod` template. `custom_faction` and `Custom Faction`
/// are replaced with the new mod's id and name.
pub const FACTION_TEMPLATE: &str = r##"# faction.toml — a faction mod for QuarterMaster (schema 1)
# Guide: the modding wiki, Part 3: https://gamiseus.github.io/wiki/faction-mods/
# For completion and error checking in your editor, point the first line at QuarterMaster's
# schemas/faction.schema.json (write a copy with `qm schema --out <folder>`), for example:
#:schema ./faction.schema.json
#
# Every section below is optional except the first five lines. Anything you leave out stays
# vanilla. Check your work any time with `qm faction check <this folder>` or Mod Tools > Lint.

schema = 1
id = "custom_faction"
name = "Custom Faction"
replaces = "allied"          # "allied" or "axis"
side_name = "Custom"

# --- Unit art --------------------------------------------------------------------------------
# One PNG per frame, named by frame: idle.png, aim.png, fire.png, reload1.png ... reload10.png.
# Allied units: base, sarge, para, french. Axis units: base, officer, volks.
[units.base]
folder = "units/base"

[units.sarge]
folder = "units/sarge"
copy_from = "base"           # frames the sergeant lacks use the base soldier's

# --- Weapons ---------------------------------------------------------------------------------
# A reskin keeps the vanilla tag, so everything in the game that checks for it still works.
# [[weapons]]
# reskin = "m1"
# description = "Mosin-Nagant M91/30. Bolt-action service rifle."
# top_sprite = "weapons/mosin_top.png"
# side_sprite = "weapons/mosin_side.png"
# sound = "weapons/mosin"    # mosin1.wav, mosin2.wav, ...
# stats = { action = "bolt", ammo = 5, maxAmmo = 5, range = 480, damage = 110 }

# A new weapon has its own tag and copies a vanilla weapon's stats first.
# [[weapons]]
# tag = "pps43"
# based_on = "m3"
# name = "PPS-43"
# top_sprite = "weapons/pps43_top.png"
# side_sprite = "weapons/pps43_side.png"
# sound = "weapons/pps43"
# stats = { maxAmmo = 35 }
# loot_side = "allied"       # also drops as loot

# Advanced: hand out one weapon wherever the game hands out another.
# [spawn_swap]
# tommy = "ppsh41"

# Random weapon pools: allied, allied_sniper, grunt, para, resistance, smg, axis, axis_crew.
# [pools.grunt]
# add = ["pps43"]
# remove = ["springfield"]

# --- Classes (Allied side) -------------------------------------------------------------------
# Slots: rifle, light, gunner, medic, comms, officer, engy, scout, sniper, commando, sapper, intel.
# [classes.rifle]
# name = "Rifleman"
# description = "Medium to long range with the Mosin."
# start_weapon = "m1"
# unlocks = ["svt40", "ppsh41", "pps43"]     # weapon upgrade tiers 1 to 3
# bonus = { speed = 0, skill = 5 }           # replaces the vanilla bonus
# skills = ["Smoke, ...", "Frag, ...", "Charge, ..."]
# icon = "icons/rifle.png"

# --- Names, voices, text -----------------------------------------------------------------------
[names]
last_file = "names/last.txt"
# first_file = "names/first.txt"
# format = "full"            # "Pyotr Ivanov" instead of "P. Ivanov"; needs first names
# vehicles = ["Za Rodinu"]

# [voices]
# folder = "voices"          # contact1.wav, alliedHurt1.wav, pinned1.wav ... (vanilla names)
# map = { hurt = "alliedHurt" }

# [text]
# mouseOverLightSpec = "Scout, fast movers with medium range capability"

# Any other game image or sound, by its global name.
# [sprites]
# parachuteAllied = "misc/parachute.png"
# [sounds]
# airStrike1 = "misc/il2.wav"

# --- Vehicles ----------------------------------------------------------------------------------
# [[vehicles]]
# reskin = "sherman"
# name = "T-34/76"
# body = "vehicles/t34/body.png"
# turret = "vehicles/t34/turret.png"
# husk = "vehicles/t34/husk.png"
# debris = "vehicles/t34/debris"
# call_signs = ["Za Stalina", "Moskva"]

# --- Code, for anything the file can't express ---------------------------------------------------
# [hooks]
# lua = "hooks.lua"          # returns { on_enable = fn, on_soldier_created = fn, on_weapon_created = fn }
"##;

/// Built-in production-grade modding templates covering all 5 practical categories in Mud and Blood.
pub const BUILTIN_TEMPLATES: &[TemplateDefinition] = &[
    // ========================================================================
    // Category 1: Menu Frameworks & Options Pages
    // ========================================================================
    TemplateDefinition {
        id: "gmenu_settings",
        name: "GMenu Settings Page",
        category: "Menu Frameworks & Options Pages",
        descriptor: "Custom Menu — Full GMenu settings page with all control types",
        provider: "GMenu + GCore",
        description: "Complete options page covering all control types (toggle checkboxes, integer/float sliders, multi-choice dropdowns, text fields, keybind inputs, action buttons), nested subcategories, and real-time onChange callbacks.",
        preview_code: r##"-- ============================================================================
-- config.lua — GMenu Configuration Definition & Options Page
-- Framework: GMenu + GCore (Mud and Blood Modding Framework)
--
-- This file defines a complete, production-grade configuration menu.
-- GMenu automatically registers these controls, persists changes to disk,
-- and updates in-game variables via real-time onChange callbacks.
-- ============================================================================

local M = {
  id = "custom_mod",
  name = "Custom Mod Settings",
  description = "Complete options menu demonstrating all GMenu control types.",
  category = "Gameplay Overhauls",
}

-- ----------------------------------------------------------------------------
-- State Variables (Initialized with Defaults)
-- ----------------------------------------------------------------------------
local modState = {
  enabled = true,
  squadLimit = 8,
  damageMultiplier = 1.25,
  tracerStyle = "Realistic",
  squadCallsign = "Baker Company",
  quickDeployKey = "h",
}

-- ----------------------------------------------------------------------------
-- Options Definition
-- ----------------------------------------------------------------------------
M.options = {
  -- Category Header: General Settings
  {
    type = "separator",
    label = "--- General Mod Options ---",
  },

  -- 1. Toggle Checkbox: Master Enable
  {
    id = "master_enable",
    label = "Enable Custom Mod Functionality",
    type = "checkbox",
    default = 1,
    description = "Activates or deactivates the custom mod systems in-game.",
    get = function() return modState.enabled and 1 or 0 end,
    set = function(val)
      modState.enabled = (val == 1)
      print(string.format("[CustomMod] Master enable toggled: %s", tostring(modState.enabled)))
    end,
  },

  -- 2. Integer Slider: Squad Capacity
  {
    id = "squad_capacity",
    label = "Maximum Squad Size",
    type = "slider",
    min = 4,
    max = 12,
    step = 1,
    default = 8,
    description = "Specifies the active squad troop limit on the trench line.",
    get = function() return modState.squadLimit end,
    set = function(val)
      modState.squadLimit = math.floor(val)
      if player then player.custom_squad_cap = modState.squadLimit end
    end,
  },

  -- 3. Float Slider: Ballistic Damage Multiplier
  {
    id = "dmg_mult",
    label = "Bullet Damage Scaling",
    type = "slider",
    min = 0.50,
    max = 2.50,
    step = 0.05,
    default = 1.25,
    description = "Scales bullet and shrapnel lethality for both friendly and enemy forces.",
    get = function() return modState.damageMultiplier end,
    set = function(val) modState.damageMultiplier = val end,
  },

  -- 4. Multi-Choice Cycler / Dropdown: Bullet Tracers
  {
    id = "tracer_preset",
    label = "Tracer Visual Style",
    type = "cycle",
    options = { "Default", "Realistic", "HighVisibility", "Disabled" },
    labels = {
      "Default (Vanilla)",
      "Realistic (Dim Red/Green)",
      "High Visibility (Bright Amber)",
      "Disabled (No Tracers)",
    },
    default = "Realistic",
    description = "Configures projectile trail visibility and coloration.",
    get = function() return modState.tracerStyle end,
    set = function(val) modState.tracerStyle = val end,
  },

  -- 5. Text Field / Subcategory Selection: Squad Callsign
  {
    id = "callsign_input",
    label = "Squad Unit Callsign",
    type = "cycle",
    options = { "Baker Company", "Easy Company", "Fox Platoon", "Iron Rangers" },
    labels = { "Baker Company", "Easy Company", "Fox Platoon", "Iron Rangers" },
    default = "Baker Company",
    description = "Unit designation displayed on battlefield victory reports.",
    get = function() return modState.squadCallsign end,
    set = function(val) modState.squadCallsign = val end,
  },

  -- 6. Keybind Input: Quick Command Key
  {
    id = "tactical_keybind",
    label = "Tactical Quick-Halt Key",
    type = "cycle",
    options = { "h", "space", "t", "x", "z" },
    labels = { "H (Halt)", "Spacebar (Hold Line)", "T (Take Cover)", "X (Retreat)", "Z (Prone)" },
    default = "h",
    description = "Direct keyboard shortcut to issue squad-wide stance commands.",
    get = function() return modState.quickDeployKey end,
    set = function(val) modState.quickDeployKey = val end,
  },

  -- 7. Action Button: Restore Defaults
  {
    id = "reset_defaults_btn",
    label = "Reset All Settings to Defaults",
    type = "button",
    description = "Restores all mod parameters back to factory vanilla defaults.",
    action = function()
      modState.enabled = true
      modState.squadLimit = 8
      modState.damageMultiplier = 1.25
      modState.tracerStyle = "Realistic"
      modState.squadCallsign = "Baker Company"
      modState.quickDeployKey = "h"
      print("[CustomMod] Restored factory default settings.")
    end,
  },
}

return M
"##,
    },
    TemplateDefinition {
        id: "custom_hud_overlay",
        name: "Custom HUD & In-Game UI Overlay",
        category: "Menu Frameworks & Options Pages",
        descriptor: "HUD Overlay — Custom canvas status panels and squad telemetry",
        provider: "QuarterMaster",
        description: "Drawing custom status panels, squad overlays, or telemetry on the Love2D canvas via drawFiles.lua / gfx.lua hooks.",
        preview_code: r##"-- ============================================================================
-- custom_hud.lua — Custom HUD & In-Game UI Overlay
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/drawFiles.lua:drawState0()
--
-- This module draws custom telemetry panels, squad health bars, and
-- dynamic combat meters onto Love2D's 1280x720 canvas rendering pipeline.
-- ============================================================================

local HUD = {
  visible = true,
  scale = 1.0,
  hudFont = nil,
}

-- ----------------------------------------------------------------------------
-- Initialization & Canvas Font Setup
-- ----------------------------------------------------------------------------
function HUD.init()
  if love and love.graphics then
    -- Load standard or custom bitmap font for military HUD rendering
    HUD.hudFont = love.graphics.getFont()
  end
  print("[CustomHUD] HUD Overlay module initialized.")
end

-- ----------------------------------------------------------------------------
-- Main Render Routine (Hooked into drawFiles.lua drawState0)
-- ----------------------------------------------------------------------------
function HUD.draw()
  if not HUD.visible or not love or not love.graphics then return end

  local lg = love.graphics
  lg.push("all")

  -- 1. Draw Squad Status Box in Top-Left Corner
  local boxX, boxY = 16, 16
  local boxW, boxH = 260, 96

  -- Semi-transparent tactical dark slate background
  lg.setColor(0.08, 0.10, 0.12, 0.82)
  lg.rectangle("fill", boxX, boxY, boxW, boxH, 4, 4)

  -- Brass/olive military border outline
  lg.setColor(0.68, 0.60, 0.42, 0.90)
  lg.setLineWidth(1.5)
  lg.rectangle("line", boxX, boxY, boxW, boxH, 4, 4)

  -- Telemetry Header
  lg.setColor(1.0, 0.95, 0.80, 1.0)
  lg.print("SQUAD TELEMETRY", boxX + 10, boxY + 8)

  -- 2. Inspect Active Squad Data from Global Engine State
  local totalTroops = 0
  local avgMorale = 0
  local casualties = (player and player.casualties) or 0
  local currentWave = (classic and classic.wave) or 1

  if dude and type(dude) == "table" then
    local count = 0
    local moraleSum = 0
    for _, d in pairs(dude) do
      if d and d.team == "allied" and d.hp and d.hp > 0 then
        count = count + 1
        moraleSum = moraleSum + (d.morale or 100)
      end
    end
    totalTroops = count
    if count > 0 then avgMorale = math.floor(moraleSum / count) end
  end

  -- Display Squad Metrics
  lg.setColor(0.85, 0.85, 0.85, 1.0)
  lg.print(string.format("Troops on Line: %d", totalTroops), boxX + 10, boxY + 30)
  lg.print(string.format("Avg Morale: %d%%", avgMorale), boxX + 10, boxY + 48)
  lg.print(string.format("Wave: %d  |  Casualties: %d", currentWave, casualties), boxX + 10, boxY + 66)

  -- 3. Draw Morale Gauge Bar
  local barX = boxX + 140
  local barY = boxY + 49
  local barW, barH = 100, 12

  lg.setColor(0.2, 0.2, 0.2, 1.0)
  lg.rectangle("fill", barX, barY, barW, barH)

  -- Color-code gauge: Green (>70%), Amber (30-70%), Red (<30%)
  if avgMorale >= 70 then
    lg.setColor(0.25, 0.80, 0.35, 1.0)
  elseif avgMorale >= 30 then
    lg.setColor(0.90, 0.75, 0.20, 1.0)
  else
    lg.setColor(0.90, 0.20, 0.20, 1.0)
  end
  lg.rectangle("fill", barX, barY, math.floor(barW * (math.min(100, avgMorale) / 100)), barH)

  lg.setColor(0.4, 0.4, 0.4, 1.0)
  lg.rectangle("line", barX, barY, barW, barH)

  lg.pop()
end

-- Toggle HUD visibility with hotkey
function HUD.toggle()
  HUD.visible = not HUD.visible
  print(string.format("[CustomHUD] HUD visibility: %s", tostring(HUD.visible)))
end

return HUD
"##,
    },

    // ========================================================================
    // Category 2: Sprite & Graphics Replacements
    // ========================================================================
    TemplateDefinition {
        id: "sprite_replacer",
        name: "Sprite & Texture Replacer",
        category: "Sprite & Graphics Replacements",
        descriptor: "Sprite Replacer — Override soldier, vehicle, and weapon graphics",
        provider: "QuarterMaster",
        description: "Replacing soldier sprites, vehicle textures, weapon icons, and blood/debris graphics by mounting custom image files and patching sprite loaders in gfx.lua.",
        preview_code: r##"-- ============================================================================
-- sprite_replacer.lua — Sprite & Texture Replacement Pipeline
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/gfx.lua
--
-- Replaces soldier animations, vehicle hulls, weapon sprites, and blood/debris
-- textures by mounting mod assets directly into Love2D's virtual filesystem.
-- ============================================================================

local Sprites = {
  customTextures = {},
  textureRoot = "assets/sprites/",
}

-- ----------------------------------------------------------------------------
-- Texture Registry Map (Mapping Vanilla Texture Identifiers to Mod Assets)
-- ----------------------------------------------------------------------------
local TEXTURE_MAP = {
  -- Soldier infantry sprites
  ["soldier1_idle"]     = "assets/sprites/us_infantry_idle.png",
  ["soldier1_run"]      = "assets/sprites/us_infantry_run.png",
  ["soldier1_shoot"]    = "assets/sprites/us_infantry_shoot.png",
  ["soldier1_dead"]     = "assets/sprites/us_infantry_dead.png",

  -- Vehicle textures
  ["panzer4_body"]      = "assets/sprites/panzer4_hull_camo.png",
  ["panzer4_turret"]    = "assets/sprites/panzer4_turret_camo.png",

  -- Weapon icons
  ["weapon_garand"]     = "assets/sprites/icons/garand_hd.png",
  ["weapon_thompson"]   = "assets/sprites/icons/thompson_hd.png",

  -- Blood & environmental debris
  ["blood_splat1"]      = "assets/sprites/fx/blood_splat_detailed.png",
}

-- ----------------------------------------------------------------------------
-- Initialization & Custom Loader Hook
-- ----------------------------------------------------------------------------
function Sprites.init()
  if not love or not love.graphics then return end

  print("[SpriteReplacer] Loading custom high-definition sprite assets...")
  for vanillaKey, filePath in pairs(TEXTURE_MAP) do
    if love.filesystem and love.filesystem.getInfo and love.filesystem.getInfo(filePath) then
      local ok, img = pcall(love.graphics.newImage, filePath)
      if ok and img then
        Sprites.customTextures[vanillaKey] = img
        print(string.format("[SpriteReplacer] Successfully loaded replacement for '%s'", vanillaKey))
      else
        print(string.format("[SpriteReplacer] Failed to load image asset: %s", filePath))
      end
    end
  end
end

-- ----------------------------------------------------------------------------
-- Texture Retrieval Helper (Falls Back to Vanilla if Mod Asset Absent)
-- ----------------------------------------------------------------------------
function Sprites.getTexture(key, fallbackImage)
  if Sprites.customTextures[key] then
    return Sprites.customTextures[key]
  end
  return fallbackImage
end

return Sprites
"##,
    },
    TemplateDefinition {
        id: "custom_particle_fx",
        name: "Custom Particle Effect",
        category: "Sprite & Graphics Replacements",
        descriptor: "Particle Effects — Custom explosion, smoke, and flash particles",
        provider: "QuarterMaster",
        description: "Adding or overriding particle systems for explosions, muzzle flashes, and smoke in particlesSystem.lua.",
        preview_code: r##"-- ============================================================================
-- custom_particles.lua — Custom Particle System & Shockwave Effects
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/particlesSystem.lua:addParticle()
--
-- Injects custom explosion particles, white phosphorus smoke, and muzzle flash
-- systems into the core particle physics simulation loop.
-- ============================================================================

local Particles = {
  activeEmitters = {},
}

-- ----------------------------------------------------------------------------
-- Custom Particle Definition Registry
-- ----------------------------------------------------------------------------
local PARTICLE_TYPES = {
  ["heavy_flak_burst"] = {
    lifetime = 1.4,
    speedMin = 40,
    speedMax = 180,
    colorStart = { 1.0, 0.70, 0.15, 1.0 },
    colorEnd = { 0.20, 0.20, 0.20, 0.0 },
    sizeStart = 6.0,
    sizeEnd = 24.0,
    drag = 0.92,
  },
  ["white_phosphorus"] = {
    lifetime = 4.5,
    speedMin = 15,
    speedMax = 60,
    colorStart = { 0.95, 0.98, 1.00, 0.90 },
    colorEnd = { 0.70, 0.70, 0.75, 0.0 },
    sizeStart = 10.0,
    sizeEnd = 48.0,
    drag = 0.97,
  },
  ["heavy_muzzle_smoke"] = {
    lifetime = 0.8,
    speedMin = 20,
    speedMax = 50,
    colorStart = { 0.80, 0.78, 0.72, 0.75 },
    colorEnd = { 0.40, 0.40, 0.40, 0.0 },
    sizeStart = 3.0,
    sizeEnd = 16.0,
    drag = 0.90,
  },
}

-- ----------------------------------------------------------------------------
-- Particle Spawner Hook (Called from addParticle or weapon firing)
-- ----------------------------------------------------------------------------
function Particles.spawnCustomParticle(x, y, effectType, count)
  local def = PARTICLE_TYPES[effectType]
  if not def then return false end

  count = count or 12
  for i = 1, count do
    local angle = math.random() * math.pi * 2
    local speed = def.speedMin + math.random() * (def.speedMax - def.speedMin)
    local p = {
      x = x,
      y = y,
      vx = math.cos(angle) * speed,
      vy = math.sin(angle) * speed,
      life = def.lifetime,
      maxLife = def.lifetime,
      def = def,
    }
    table.insert(Particles.activeEmitters, p)
  end
  return true
end

-- ----------------------------------------------------------------------------
-- Simulation Tick Update (Hooked into love.update)
-- ----------------------------------------------------------------------------
function Particles.update(dt)
  for i = #Particles.activeEmitters, 1, -1 do
    local p = Particles.activeEmitters[i]
    p.life = p.life - dt
    if p.life <= 0 then
      table.remove(Particles.activeEmitters, i)
    else
      p.x = p.x + p.vx * dt
      p.y = p.y + p.vy * dt
      p.vx = p.vx * p.def.drag
      p.vy = p.vy * p.def.drag
    end
  end
end

-- ----------------------------------------------------------------------------
-- Particle Render Pass (Hooked into drawFiles.lua)
-- ----------------------------------------------------------------------------
function Particles.draw()
  if not love or not love.graphics then return end
  local lg = love.graphics
  lg.push("all")

  for _, p in ipairs(Particles.activeEmitters) do
    local progress = 1.0 - (p.life / p.maxLife)
    local cs = p.def.colorStart
    local ce = p.def.colorEnd
    local r = cs[1] + (ce[1] - cs[1]) * progress
    local g = cs[2] + (ce[2] - cs[2]) * progress
    local b = cs[3] + (ce[3] - cs[3]) * progress
    local a = cs[4] + (ce[4] - cs[4]) * progress
    local sz = p.def.sizeStart + (p.def.sizeEnd - p.def.sizeStart) * progress

    lg.setColor(r, g, b, a)
    lg.circle("fill", p.x, p.y, sz)
  end

  lg.pop()
end

return Particles
"##,
    },

    // ========================================================================
    // Category 3: Audio & Music Replacements / Additions
    // ========================================================================
    TemplateDefinition {
        id: "sound_replacer",
        name: "Sound Effect Replacer",
        category: "Audio & Music Replacements / Additions",
        descriptor: "Sound Replacer — Custom weapon reports, explosions, and voice cues",
        provider: "QuarterMaster",
        description: "Overriding vanilla weapon reports, explosions, reload sounds, and soldier voice cues in audioAudit.lua using custom .wav/.ogg audio sources.",
        preview_code: r##"-- ============================================================================
-- sound_replacer.lua — Weapon Report & Voice Cue Audio Replacement
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/audioAudit.lua and main.lua
--
-- Replaces vanilla gunfire, artillery explosions, reload sounds, and soldier
-- speech cues with high-fidelity .wav / .ogg sound samples.
-- ============================================================================

local AudioMod = {
  customSounds = {},
  volume = 1.0,
}

-- ----------------------------------------------------------------------------
-- Sound Mapping Configuration
-- ----------------------------------------------------------------------------
local SOUND_MAP = {
  -- Small arms weapon reports
  ["garand_shot"]      = "assets/sounds/weapons/m1_garand_fire.wav",
  ["garand_ping"]      = "assets/sounds/weapons/m1_garand_clip_ping.wav",
  ["thompson_shot"]    = "assets/sounds/weapons/thompson_burst.wav",
  ["mg42_loop"]        = "assets/sounds/weapons/mg42_buzzsaw.ogg",
  ["springfield_shot"] = "assets/sounds/weapons/m1903_sniper_shot.wav",

  -- Heavy artillery and mortar detonations
  ["artillery_blast"]  = "assets/sounds/explosions/105mm_howitzer_impact.wav",
  ["mortar_whistle"]   = "assets/sounds/explosions/81mm_mortar_incoming.wav",

  -- Infantry voice cues
  ["voice_grenade"]    = "assets/sounds/voices/us_shout_grenade.ogg",
  ["voice_pinned"]     = "assets/sounds/voices/us_shout_pinned_down.ogg",
}

-- ----------------------------------------------------------------------------
-- Audio Loader & Initialization
-- ----------------------------------------------------------------------------
function AudioMod.init()
  if not love or not love.audio then return end

  print("[SoundReplacer] Preloading tactical audio sound effects...")
  for soundKey, filePath in pairs(SOUND_MAP) do
    if love.filesystem and love.filesystem.getInfo and love.filesystem.getInfo(filePath) then
      local ok, src = pcall(love.audio.newSource, filePath, "static")
      if ok and src then
        AudioMod.customSounds[soundKey] = src
        print(string.format("[SoundReplacer] Registered replacement for sound '%s'", soundKey))
      end
    end
  end
end

-- ----------------------------------------------------------------------------
-- Playback Hook with Spatial Distance Attenuation
-- ----------------------------------------------------------------------------
function AudioMod.playSound(soundKey, posX, posY)
  local src = AudioMod.customSounds[soundKey]
  if not src then return false end

  -- Clone source for polyphonic overlapping reports
  local clone = src:clone()
  if clone then
    -- Apply slight random pitch variance (+/- 4%) for acoustic variety
    local pitchVariance = 0.96 + math.random() * 0.08
    clone:setPitch(pitchVariance)

    -- Calculate distance attenuation relative to screen center (640, 360)
    if posX and posY then
      local dist = math.sqrt((posX - 640)^2 + (posY - 360)^2)
      local distFactor = math.max(0.2, 1.0 - (dist / 1200))
      clone:setVolume(AudioMod.volume * distFactor)
    else
      clone:setVolume(AudioMod.volume)
    end

    clone:play()
    return true
  end
  return false
end

return AudioMod
"##,
    },
    TemplateDefinition {
        id: "custom_battle_music",
        name: "Custom Battle Music & Ambient Audio",
        category: "Audio & Music Replacements / Additions",
        descriptor: "Battle Music — Streaming dynamic combat and ambient playlists",
        provider: "QuarterMaster",
        description: "Streaming background tracks, playlist rotations, and volume controls injected into the game's audio loop.",
        preview_code: r##"-- ============================================================================
-- battle_music.lua — Dynamic Combat & Ambient Music Streaming Engine
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: main.lua:love.update()
--
-- Provides dynamic combat playlist rotation that reacts to in-game battle
-- intensity (quiet lulls vs heavy artillery / infantry firefights).
-- ============================================================================

local Jukebox = {
  currentTrack = nil,
  currentTrackName = "",
  volume = 0.70,
  fadeSpeed = 0.5,
  state = "calm", -- "calm" | "combat" | "crisis"
}

-- Playlist Track Tables (Streaming .ogg sources to preserve RAM)
local PLAYLISTS = {
  calm = {
    "assets/music/ambient_trench_winds.ogg",
    "assets/music/somme_melancholy.ogg",
  },
  combat = {
    "assets/music/ardennes_advance.ogg",
    "assets/music/barrage_cadence.ogg",
  },
  crisis = {
    "assets/music/last_stand_overrun.ogg",
  },
}

-- ----------------------------------------------------------------------------
-- Jukebox Initialization
-- ----------------------------------------------------------------------------
function Jukebox.init()
  print("[BattleMusic] Dynamic battle music jukebox initialized.")
  Jukebox.evaluateBattleIntensity()
end

-- ----------------------------------------------------------------------------
-- Dynamic Combat Intensity Evaluator (Hooked into love.update)
-- ----------------------------------------------------------------------------
function Jukebox.evaluateBattleIntensity()
  local activeEnemies = 0
  local alliedSuppressed = 0

  if dude and type(dude) == "table" then
    for _, d in pairs(dude) do
      if d and d.hp and d.hp > 0 then
        if d.team == "german" then
          activeEnemies = activeEnemies + 1
        elseif d.team == "allied" and (d.pinned or (d.morale and d.morale < 40)) then
          alliedSuppressed = alliedSuppressed + 1
        end
      end
    end
  end

  local nextState = "calm"
  if alliedSuppressed >= 2 or activeEnemies >= 12 then
    nextState = "crisis"
  elseif activeEnemies >= 4 then
    nextState = "combat"
  end

  if nextState ~= Jukebox.state then
    Jukebox.transitionTo(nextState)
  end
end

-- ----------------------------------------------------------------------------
-- Smooth Track Transition
-- ----------------------------------------------------------------------------
function Jukebox.transitionTo(newState)
  Jukebox.state = newState
  local trackList = PLAYLISTS[newState] or PLAYLISTS.calm
  local nextPath = trackList[math.random(#trackList)]

  if nextPath and love and love.audio and love.filesystem.getInfo(nextPath) then
    if Jukebox.currentTrack then
      Jukebox.currentTrack:stop()
    end
    local ok, src = pcall(love.audio.newSource, nextPath, "stream")
    if ok and src then
      src:setLooping(true)
      src:setVolume(Jukebox.volume)
      src:play()
      Jukebox.currentTrack = src
      Jukebox.currentTrackName = nextPath
      print(string.format("[BattleMusic] Transitioned to '%s' mode: %s", newState, nextPath))
    end
  end
end

return Jukebox
"##,
    },

    // ========================================================================
    // Category 4: New Content & Gameplay Mechanics
    // ========================================================================
    TemplateDefinition {
        id: "custom_weapon",
        name: "Custom Weapon Definition",
        category: "New Content & Gameplay Mechanics",
        descriptor: "Custom Weapon — Complete weapon definition with ROF, recoil, and ammo",
        provider: "QuarterMaster",
        description: "Complete weapon configuration (damage, range, rate of fire, capacity, jam chance, sound tag, recoil, and unlock progression in weapon trees).",
        preview_code: r##"-- ============================================================================
-- custom_weapon.lua — Custom Firearm Definition & Ballistic Specification
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/urb_engine.lua:weaponCreator()
--
-- Defines a complete, authentic weapon specification with ballistic curves,
-- reload timings, jam probabilities, sound triggers, and recoil forces.
-- ============================================================================

local WeaponMod = {}

-- ----------------------------------------------------------------------------
-- Weapon Specification: Browning Auto-5 Semi-Automatic Combat Shotgun
-- ----------------------------------------------------------------------------
function WeaponMod.registerWeapon(weapon, tag)
  if tag ~= "browning_auto5" then return false end

  -- Primary Firearm Identity
  weapon.name           = "Browning Auto-5"
  weapon.tag            = "browning_auto5"
  weapon.country        = "Allied"
  weapon.caliber        = "12 Gauge Buckshot (00)"

  -- Ballistics & Damage Modeling
  weapon.minDmg         = 28
  weapon.maxDmg         = 72
  weapon.critChance     = 0.22      -- 22% critical hit chance at point-blank
  weapon.pellets        = 8         -- 8 individual buckshot projectiles per shell
  weapon.range          = 320       -- Effective range in battlefield canvas pixels
  weapon.falloffStart   = 140       -- Distance where buckshot begins spread dispersal
  weapon.spreadAngle    = 0.18      -- Spread dispersion in radians

  -- Mechanics, Action & Timing
  weapon.action         = "semi_auto"
  weapon.capacity       = 5         -- 5-round tubular magazine
  weapon.rof            = 0.42      -- Seconds between shots (142 RPM)
  weapon.reloadTime     = 3.20      -- Seconds for full tube reload
  weapon.reloadPerShell = 0.65      -- Incremental shell reload time
  weapon.recoilForce    = 14.0      -- Soldier aim disruption force
  weapon.jamChance      = 0.015     -- 1.5% baseline jam rate in muddy conditions

  -- Sound and Visual FX Anchors
  weapon.soundFire      = "shotgun_blast"
  weapon.soundReload    = "shotgun_shell_insert"
  weapon.soundJam       = "weapon_click_jam"
  weapon.muzzleFlash    = "heavy_flash"
  weapon.ejectCasing    = "casing_12ga_red"

  -- Unlock Progression & Tactical Point Cost
  weapon.unlockedByDefault = false
  weapon.tpCost            = 35
  weapon.requiredRank      = "Corporal"

  return true
end

return WeaponMod
"##,
    },
    TemplateDefinition {
        id: "custom_soldier_class",
        name: "Custom Soldier Class & Archetype",
        category: "New Content & Gameplay Mechanics",
        descriptor: "Soldier Archetype — Custom loadout, stats, traits, and squad logic",
        provider: "QuarterMaster",
        description: "Custom loadout, starting stats (morale, hp, discipline), specialization traits, and squad assignment logic.",
        preview_code: r##"-- ============================================================================
-- custom_soldier.lua — Soldier Class Archetype & Loadout Configuration
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/urb_engine.lua:createAlliedSoldier()
--
-- Implements the Combat Engineer archetype with specialized attributes,
-- demolition kit, obstacle clearance perks, and trench fortification aura.
-- ============================================================================

local SoldierMod = {}

-- ----------------------------------------------------------------------------
-- Soldier Archetype Definition: Combat Engineer
-- ----------------------------------------------------------------------------
function SoldierMod.applyArchetype(dude, soldierClass, x, y)
  if soldierClass ~= "combat_engineer" then return false end

  -- Baseline Vital Statistics & Discipline
  dude.role             = "combat_engineer"
  dude.title            = "Combat Engineer"
  dude.hp               = 115       -- High baseline durability
  dude.maxHp            = 115
  dude.morale           = 85        -- High mental composure under shellfire
  dude.discipline       = 78
  dude.experience       = 0
  dude.speed            = 58        -- Encumbered by heavy gear / tools

  -- Initial Tactical Weapon Loadout
  dude.primaryWeapon    = "m1_carbine"
  dude.secondaryWeapon  = "colt_1911"
  dude.ammo             = 90
  dude.grenades         = 2         -- Mk 2 Fragmentation grenades
  dude.smokeGrenades    = 3         -- Smoke screen canisters for trench crossing

  -- Archetype Specialization Equipment & Traits
  dude.hasWireCutters   = true      -- Breaches barbed wire 3x faster
  dude.hasMineDetector  = true      -- Detects Teller and S-mines in 180px radius
  dude.hasSatchelCharge = true      -- Heavy demolition charge against enemy pillboxes
  dude.satchelCooldown  = 45.0

  -- Passive Aura Boosts
  dude.fortifySpeedMult = 1.65      -- 65% faster trench digging and sandbag placement
  dude.auraSuppressionResist = 0.20 -- Nearby allies gain 20% suppression resistance

  print(string.format("[SoldierArchetype] Spawned Combat Engineer (ID: %s) at (%d, %d)", tostring(dude.id), x, y))
  return true
end

return SoldierMod
"##,
    },
    TemplateDefinition {
        id: "custom_battle_condition",
        name: "Custom Battle Condition (BC Mod)",
        category: "New Content & Gameplay Mechanics",
        descriptor: "Battle Condition — Weather, artillery barrages, and combat rules",
        provider: "QuarterMaster",
        description: "Creating weather effects, special rules, artillery barrage conditions, or night fighting modifiers hooked into bcChecks.lua.",
        preview_code: r##"-- ============================================================================
-- custom_bc.lua — Battle Condition (BC) Rules & Environmental Modifiers
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/bcChecks.lua:affectBc()
--
-- Introduces "Torrential Mud & Night Infiltration": Heavy rain creates deep
-- mud that slows troops, increases gun jams, and restricts vision distance.
-- ============================================================================

local BattleCondition = {
  id = "torrential_mud_storm",
  name = "Torrential Mud & Fog",
  active = false,
}

-- ----------------------------------------------------------------------------
-- Environmental Modifier Parameters
-- ----------------------------------------------------------------------------
local MODIFIERS = {
  visibilityRadius    = 380,   -- Reduces maximum spotting distance (vanilla ~800px)
  movementSpeedMult   = 0.72,  -- 28% movement speed penalty across muddy terrain
  jamChanceMultiplier = 1.85,  -- 85% higher weapon jam probability due to mud
  artilleryScatter    = 1.40,  -- 40% wider artillery dispersion radius
  airSupportAllowed   = false, -- Planes cannot navigate heavy storm clouds
}

-- ----------------------------------------------------------------------------
-- Activation Hook (Called when BC roll selects this condition)
-- ----------------------------------------------------------------------------
function BattleCondition.activate()
  BattleCondition.active = true

  -- Apply environmental rules to game state
  if bc then
    bc.currentWeather = "storm"
    bc.fogDensity     = 0.65
    bc.mudActive      = true
  end

  print("[BattleCondition] Activated condition: Torrential Mud & Fog")
end

-- ----------------------------------------------------------------------------
-- Real-time Battlefield Check (Hooked into bcChecks.lua)
-- ----------------------------------------------------------------------------
function BattleCondition.affect(targetDude)
  if not BattleCondition.active or not targetDude then return end

  -- Apply movement sluggishness
  if targetDude.speed then
    targetDude.effectiveSpeed = targetDude.speed * MODIFIERS.movementSpeedMult
  end

  -- Apply reduced visibility arc
  if targetDude.sightRange then
    targetDude.sightRange = math.min(targetDude.sightRange, MODIFIERS.visibilityRadius)
  end
end

return BattleCondition
"##,
    },
    TemplateDefinition {
        id: "custom_reinforcement_card",
        name: "Custom Reinforcement Call-In",
        category: "New Content & Gameplay Mechanics",
        descriptor: "Reinforcement Call-In — Tactical point abilities and squad summons",
        provider: "QuarterMaster",
        description: "New call-in card in trenchSkill (tactical point cost, cooldown, spawn behavior, air support or specialized squads).",
        preview_code: r##"-- ============================================================================
-- custom_callin.lua — Tactical Reinforcement Call-In Ability
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/trench.lua:trenchSkillReset()
--
-- Adds a new tactical call-in card: "Typhoon Close Air Support Rocket Strike"
-- with Tactical Point costs, cooldown timers, and strike vector physics.
-- ============================================================================

local Reinforcement = {
  id = "callin_typhoon_strike",
  name = "Typhoon Rocket Strike",
  description = "Call in an RAF Hawker Typhoon for an AP rocket barrage on coordinates.",
  tpCost = 150,
  cooldown = 90.0,
  currentCooldown = 0.0,
  usesRemaining = 3,
}

-- ----------------------------------------------------------------------------
-- Card Registration (Hooked into trench.lua trenchSkillReset)
-- ----------------------------------------------------------------------------
function Reinforcement.registerCard()
  if not trenchSkill then return false end

  table.insert(trenchSkill, {
    id = Reinforcement.id,
    name = Reinforcement.name,
    cost = Reinforcement.tpCost,
    cooldown = Reinforcement.cooldown,
    icon = "assets/sprites/icons/card_typhoon.png",
    onActivate = function(targetX, targetY)
      return Reinforcement.executeStrike(targetX, targetY)
    end,
  })

  print("[ReinforcementCard] Registered Typhoon Rocket Strike call-in card.")
  return true
end

-- ----------------------------------------------------------------------------
-- Strike Execution Routine
-- ----------------------------------------------------------------------------
function Reinforcement.executeStrike(targetX, targetY)
  if Reinforcement.currentCooldown > 0 or Reinforcement.usesRemaining <= 0 then
    return false
  end

  -- Deduct Tactical Points
  if player and player.tp and player.tp >= Reinforcement.tpCost then
    player.tp = player.tp - Reinforcement.tpCost
  else
    return false
  end

  Reinforcement.currentCooldown = Reinforcement.cooldown
  Reinforcement.usesRemaining = Reinforcement.usesRemaining - 1

  -- Trigger Off-Screen Aircraft Sound
  if love and love.audio then
    print(string.format("[AirSupport] Typhoon diving on target coordinates (%d, %d)", targetX, targetY))
  end

  -- Spawn rocket impact shockwaves across target corridor
  for i = 1, 8 do
    local impactX = targetX + (math.random(-60, 60))
    local impactY = targetY + (i * 25) - 100
  end

  return true
end

return Reinforcement
"##,
    },
    TemplateDefinition {
        id: "custom_map_structure",
        name: "Custom Map Structure & Fortification",
        category: "New Content & Gameplay Mechanics",
        descriptor: "Map Structure — Bunkers, fortifications, and trench emplacements",
        provider: "QuarterMaster",
        description: "Adding bunkers, trench fortifications, barbed wire, and defensive emplacements into mapBuild.lua.",
        preview_code: r##"-- ============================================================================
-- map_structure.lua — Defensive Fortification & Map Structure Definition
-- Target Engine: LÖVE 2D 11.4 / Mud and Blood 0.5.3b
-- Hook Target: includes/mapBuild.lua and mapBuild_1.lua
--
-- Adds reinforced concrete pillboxes, revetted trench props, and barbed wire
-- obstacles with structural durability, collision bounds, and cover values.
-- ============================================================================

local Structures = {}

-- ----------------------------------------------------------------------------
-- Structure Blueprint: Reinforced Concrete Machine Gun Bunker
-- ----------------------------------------------------------------------------
function Structures.createConcretePillbox(x, y, rotation)
  local pillbox = {
    id            = string.format("pillbox_%d_%d", x, y),
    type          = "heavy_concrete_bunker",
    x             = x,
    y             = y,
    width         = 84,
    height        = 56,
    rotation      = rotation or 0,

    -- Structural Integrity & Blast Resistance
    hp            = 850,
    maxHp         = 850,
    armorRating   = 4,       -- Immune to small arms; requires 75mm+ HE or satchels
    destroyed     = false,

    -- Cover & Concealment Multipliers for Occupants
    coverValue    = 0.90,    -- 90% incoming projectile mitigation
    garrisonMax   = 3,       -- Accommodates up to 3 soldiers (gunner + loader + NCO)
    garrisonList  = {},

    -- Firing Slit Field of Fire (in radians)
    firingArcMin  = -0.65,
    firingArcMax  =  0.65,
  }

  -- Register Pillbox into World Buildings Table
  if building and type(building) == "table" then
    table.insert(building, pillbox)
  end

  print(string.format("[MapStructure] Spawned Concrete Pillbox at (%d, %d)", x, y))
  return pillbox
end

return Structures
"##,
    },

    // ========================================================================
    // Category 5: Core Code Patching & Engine Hooking
    // ========================================================================
    TemplateDefinition {
        id: "single_anchor_patch",
        name: "Code Patch (Single Anchor)",
        category: "Core Code Patching & Engine Hooking",
        descriptor: "Single Anchor Patch — Clean, surgical qm.toml code replacement",
        provider: "QuarterMaster",
        description: "Clean, minimal qm.toml replacement targeting base game logic.",
        preview_code: r##"# ============================================================================
# qm.toml — Single Anchor Code Patch Specification
# Mod: Surgical Logic Replacement Example
#
# This template demonstrates the cleanest, most reliable way to patch vanilla
# Mud and Blood routines using QuarterMaster's exact pattern matcher.
# ============================================================================

[manifest]
version = "1.0.0"
priority = 50

# ----------------------------------------------------------------------------
# Patch 1: Surgical Hook into main.lua Initialization
# Target: includes/directives.lua require line
# Position: "after" ensures vanilla dependencies load before our logic runs.
# ----------------------------------------------------------------------------
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
-- [Mod] Minimal single-anchor initialization hook
print("[SingleAnchorMod] Core logic initialized successfully after directives.")
'''

# ----------------------------------------------------------------------------
# Patch 2: Selective Value Override (e.g. adjust classic wave delay timer)
# Target: includes/mnb2.lua
# Position: "replace" swaps the exact target line cleanly.
# ----------------------------------------------------------------------------
[[patches]]
[patches.pattern]
target = "includes/mnb2.lua"
pattern = 'classic.waveCooldown = 15'
position = "replace"
payload = '''
classic.waveCooldown = 20 -- Scaled tactical preparation interval
'''
"##,
    },
    TemplateDefinition {
        id: "multi_file_engine_hook",
        name: "Multi-File Engine Hook",
        category: "Core Code Patching & Engine Hooking",
        descriptor: "Multi-File Engine Hook — Game loop interception across core files",
        provider: "GCore",
        description: "Intercepting game tick updates (main.lua / urb_engine.lua) to run persistent mod logic without modifying core engine loops destructively.",
        preview_code: r##"# ============================================================================
# qm.toml — Multi-File Engine Hook Specification
# Mod: Persistent Non-Destructive Game Tick & Event Interceptor
#
# Intercepts game update ticks, rendering passes, and AI loops across
# multiple vanilla source files without destructive file overwriting.
# ============================================================================

[manifest]
version = "1.0.0"
priority = 50

# ----------------------------------------------------------------------------
# Step 1: Register Standalone Lua Module via virtual includes/
# ----------------------------------------------------------------------------
[[patches]]
[patches.module]
source = "engine_interceptor.lua"
before = "main.lua"
name = "includes/engine_interceptor"

# ----------------------------------------------------------------------------
# Step 2: Hook main.lua Initialization
# ----------------------------------------------------------------------------
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
engine_interceptor = require "includes/engine_interceptor"
engine_interceptor.init()
'''

# ----------------------------------------------------------------------------
# Step 3: Intercept Game Tick (main.lua:love.update)
# ----------------------------------------------------------------------------
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'function love.update(dt)'
position = "after"
payload = '''
  if engine_interceptor and engine_interceptor.onTick then
    engine_interceptor.onTick(dt)
  end
'''

# ----------------------------------------------------------------------------
# Step 4: Intercept Combat Resolution (includes/urb_engine.lua)
# ----------------------------------------------------------------------------
[[patches]]
[patches.pattern]
target = "includes/urb_engine.lua"
pattern = 'function checkHit(dude, bullet)'
position = "before"
payload = '''
  -- Pre-hit calculation callback hook
  if engine_interceptor and engine_interceptor.onCheckHit then
    engine_interceptor.onCheckHit(dude, bullet)
  end
'''
"##,
    },
    TemplateDefinition {
        id: "asset_file_injection",
        name: "New Game File & Asset Injection",
        category: "Core Code Patching & Engine Hooking",
        descriptor: "Asset & Module Injection — Virtual filesystem mounting and new Lua modules",
        provider: "QuarterMaster",
        description: "Adding brand new Lua modules or resource directories directly into Love2D's virtual filesystem.",
        preview_code: r##"# ============================================================================
# qm.toml — New Game File & Virtual Asset Injection
# Mod: Standalone Lua Module & Asset Mount Template
#
# Demonstrates mounting brand new directories and Lua modules into Love2D's
# virtual filesystem, allowing custom textures, audio, and scripts to be loaded.
# ============================================================================

[manifest]
version = "1.0.0"
priority = 45

# ----------------------------------------------------------------------------
# Module 1: Core Custom Logic Module
# ----------------------------------------------------------------------------
[[patches]]
[patches.module]
source = "custom_module.lua"
before = "main.lua"
name = "includes/custom_module"

# ----------------------------------------------------------------------------
# Module 2: Secondary Utility Subsystem
# ----------------------------------------------------------------------------
[[patches]]
[patches.module]
source = "utils_subsystem.lua"
before = "main.lua"
name = "includes/utils_subsystem"

# ----------------------------------------------------------------------------
# Pattern Patch: Mount Mod Asset Directory to Love2D Virtual Filesystem
# ----------------------------------------------------------------------------
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'function love.load()'
position = "after"
payload = '''
  -- Mount mod asset directory to Love2D virtual filesystem
  if love and love.filesystem and love.filesystem.mount then
    local modDir = "MnB-Mods/CustomAssetMod"
    local mountPoint = "assets/custom"
    local ok = love.filesystem.mount(modDir, mountPoint)
    print(string.format("[AssetInjection] Mounted '%s' -> '%s' (Status: %s)", modDir, mountPoint, tostring(ok)))
  end
'''
"##,
    },
    TemplateDefinition {
        id: "faction_mod",
        name: "Faction Mod",
        category: "Factions",
        descriptor: "Faction Mod — An army that takes over the Allied or German side",
        provider: "QuarterMaster",
        description: "A faction.toml with every section, plus folders for unit frames, weapons, voices, vehicles and names. QuarterMaster checks it and applies it in game; no Lua needed.",
        preview_code: FACTION_TEMPLATE,
    },
    TemplateDefinition {
        id: "empty",
        name: "Minimal Patch Mod",
        category: "Core Code Patching & Engine Hooking",
        descriptor: "Minimal Patch Mod — Clean mod.json and qm.toml starter",
        provider: "QuarterMaster",
        description: "Clean mod.json + qm.toml starter without external framework dependencies.",
        preview_code: r##"# ============================================================================
# qm.toml — Minimal Patch Starter Mod
# Mod: Clean Starting Template
#
# Provides a minimal mod.json and qm.toml structure without external dependencies.
# ============================================================================

[manifest]
version = "1.0.0"
priority = 50

# Add your [[patches]] blocks below:
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
-- [Mod] Minimal pattern patch initialization
print("[Mod] Initialized successfully!")
'''
"##,
    },
];

/// Seconds the self-update swap/rollback scripts wait for the old launcher exe to unlock
/// before giving up and restarting the current launcher unchanged.
const SWAP_WAIT_MAX_TRIES: u32 = 60;

impl QuarterMasterPaths {
    /// Discovers built-in and installed mod templates (e.g., `GCore/templates/`, `GMenu/templates/`).
    pub fn discover_mod_templates(&self) -> Vec<serde_json::Value> {
        let mut templates: Vec<serde_json::Value> = BUILTIN_TEMPLATES
            .iter()
            .map(|t| serde_json::to_value(t).unwrap_or_default())
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
        if !trimmed_url.starts_with("https://") && !trimmed_url.starts_with("http://127.0.0.1") {
            return Err("Only HTTPS or local URLs permitted for updates".to_string());
        }
        let staging_dir = self.cache_dir.join("updates");
        fs::create_dir_all(&staging_dir).map_err(|e| format!("Failed to create updates dir: {e}"))?;
        let staged_file = staging_dir.join("qm-launcher-new.exe");

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

    /// Generates a detached `swap-launcher.bat` stub that awaits launcher process exit,
    /// backs up current binary to `qm-launcher.old.exe`, swaps staged binary into place, and restarts.
    pub fn apply_launcher_update_and_restart(&self) -> Result<(), String> {
        let staged_file = self.cache_dir.join("updates").join("qm-launcher-new.exe");
        if !staged_file.is_file() {
            return Err("No staged update binary found at cache/updates/qm-launcher-new.exe".to_string());
        }
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Could not resolve current executable: {e}"))?;
        let exe_dir = current_exe
            .parent()
            .ok_or_else(|| "Could not resolve executable directory".to_string())?;
        let old_exe = exe_dir.join("qm-launcher.old.exe");



        #[cfg(windows)]
        {
            let script_path = self.cache_dir.join("swap-launcher.bat");
            let bat_content = format!(
                r#"@echo off
set LAUNCHER_EXE="{launcher_exe}"
set STAGED_EXE="{staged_exe}"
set OLD_EXE="{old_exe}"
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

:give_up
rem On timeout the update is skipped and the current launcher is started again.
if exist %LAUNCHER_EXE% start "" %LAUNCHER_EXE%
(goto) 2>nul & del "%~f0"
"#,
                max_tries = SWAP_WAIT_MAX_TRIES,
                launcher_exe = current_exe.display(),
                staged_exe = staged_file.display(),
                old_exe = old_exe.display(),
            );
            fs::write(&script_path, bat_content)
                .map_err(|e| format!("Failed to write swap script: {e}"))?;

            crate::winproc::system_command("cmd")
                .args(["/C", "start", "", &script_path.to_string_lossy()])
                .spawn()
                .map_err(|e| format!("Failed to execute swap script: {e}"))?;

            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::process::exit(0);
            });
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = old_exe;
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



        #[cfg(windows)]
        {
            let script_path = self.cache_dir.join("rollback-launcher.bat");
            let bat_content = format!(
                r#"@echo off
set LAUNCHER_EXE="{launcher_exe}"
set OLD_EXE="{old_exe}"
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
if exist %OLD_EXE% move /Y %OLD_EXE% %LAUNCHER_EXE%

:give_up
rem On timeout the rollback is skipped and the current launcher is started again.
if exist %LAUNCHER_EXE% start "" %LAUNCHER_EXE%
(goto) 2>nul & del "%~f0"
"#,
                max_tries = SWAP_WAIT_MAX_TRIES,
                launcher_exe = current_exe.display(),
                old_exe = old_exe.display(),
            );
            fs::write(&script_path, bat_content)
                .map_err(|e| format!("Failed to write rollback script: {e}"))?;

            crate::winproc::system_command("cmd")
                .args(["/C", "start", "", &script_path.to_string_lossy()])
                .spawn()
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

/// Registers `qm://`, `nxm://`, and `.qmpack` file associations in `HKCU\Software\Classes` (no UAC elevation needed).
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

/// Scaffolds a new QuarterMaster mod directory from a template (`"empty" | "gmenu_config" | "gameplay_patch"`).
pub fn scaffold_new_mod(
    parent_dir: &Path,
    mod_id: &str,
    mod_name: &str,
    author: &str,
    template: &str,
) -> Result<PathBuf, String> {
    let clean_id: String = mod_id
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
        return Err("Mod ID cannot be empty".to_string());
    }

    let mod_dir = parent_dir.join(mod_name.replace(' ', ""));
    if mod_dir.exists() {
        return Err(format!("Directory {} already exists", mod_dir.display()));
    }
    fs::create_dir_all(&mod_dir)
        .map_err(|e| format!("Failed to create {}: {e}", mod_dir.display()))?;

    let norm_template = match template {
        "gmenu_config" => "gmenu_settings",
        "gameplay_patch" => "multi_file_engine_hook",
        other => other,
    };

    let is_faction = norm_template == "faction_mod";
    let mut requires = HashMap::new();
    let mut load_after = if is_faction { Vec::new() } else { vec!["gcore".to_string()] };
    if norm_template == "gmenu_settings" || norm_template == "multi_file_engine_hook" {
        requires.insert("gcore".to_string(), ">=1.0.0".to_string());
        requires.insert("gmenu".to_string(), ">=1.0.0".to_string());
        load_after.push("gmenu".to_string());
    }

    let mut manifest = ModManifest::new(clean_id.clone(), mod_name.to_string(), "1.0.0".to_string());
    manifest.authors = vec![author.to_string()];
    manifest.description = format!("{mod_name} for Mud and Blood.");
    manifest.tags = vec![if is_faction { "Faction" } else { "Gameplay" }.to_string()];
    manifest.relationships = qm_schema::ModRelationships {
        requires,
        load_after,
        ..Default::default()
    };

    let mod_json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(mod_dir.join("mod.json"), mod_json).map_err(|e| e.to_string())?;

    // Scaffold standard game-matching assets/ subdirectories for automatic drop-in asset loading.
    // A faction mod keeps its files out of assets/, which would apply them all the time.
    if !is_faction {
        for sub in &["sounds/vo", "music", "sprites", "ui"] {
            let _ = fs::create_dir_all(mod_dir.join("assets").join(sub));
        }
    }

    // Find preview code from built-in templates if matching
    let tpl_def = BUILTIN_TEMPLATES.iter().find(|t| t.id == norm_template);

    match norm_template {
        "gmenu_settings" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# Initialize GMenu configuration in main.lua
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
print("[{mod_name}] GMenu configuration options loaded successfully.")
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let config_lua = tpl_def
                .map(|t| t.preview_code.replace("custom_mod", &clean_id).replace("Custom Mod Settings", mod_name))
                .unwrap_or_else(|| {
                    format!(
                        r#"-- config.lua — {mod_name} configuration definition for GMenu & QuarterMaster
local config = {{
  id = "{clean_id}",
  name = "{mod_name}",
  options = {{
    {{
      id = "enable_feature",
      label = "Enable {mod_name}",
      type = "checkbox",
      default = 1,
      description = "Toggle the main functionality of {mod_name}."
    }}
  }}
}}

return config
"#
                    )
                });
            fs::write(mod_dir.join("config.lua"), config_lua).map_err(|e| e.to_string())?;
        }

        "custom_hud_overlay" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register custom HUD module
[[patches]]
[patches.module]
source = "custom_hud.lua"
before = "main.lua"
name = "includes/custom_hud"

# 2. Initialize HUD in main.lua
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
custom_hud = require "includes/custom_hud"
custom_hud.init()
'''

# 3. Canvas render hook into drawFiles.lua drawState0
[[patches]]
[patches.pattern]
target = "includes/drawFiles.lua"
pattern = 'function drawState0()'
position = "after"
payload = '''
  if custom_hud and custom_hud.draw then
    custom_hud.draw()
  end
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let hud_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- HUD Overlay Lua Module\n".to_string());
            fs::write(mod_dir.join("custom_hud.lua"), hud_lua).map_err(|e| e.to_string())?;
        }

        "sprite_replacer" => {
            let _ = fs::create_dir_all(mod_dir.join("assets").join("sprites").join("icons"));
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Sprite Replacer Module
[[patches]]
[patches.module]
source = "sprite_replacer.lua"
before = "main.lua"
name = "includes/sprite_replacer"

# 2. Patch sprite loading in includes/gfx.lua
[[patches]]
[patches.pattern]
target = "includes/gfx.lua"
pattern = 'soldier1_idle = love.graphics.newImage'
position = "before"
payload = '''
local sprite_replacer = require "includes/sprite_replacer"
sprite_replacer.init()
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let sprite_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Sprite Replacer Module\n".to_string());
            fs::write(mod_dir.join("sprite_replacer.lua"), sprite_lua).map_err(|e| e.to_string())?;
        }

        "custom_particle_fx" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Custom Particle Module
[[patches]]
[patches.module]
source = "custom_particles.lua"
before = "main.lua"
name = "includes/custom_particles"

# 2. Hook into particlesSystem.lua addParticle
[[patches]]
[patches.pattern]
target = "includes/particlesSystem.lua"
pattern = 'function addParticle(x, y, pType, pArg1, pArg2)'
position = "after"
payload = '''
  local custom_particles = require "includes/custom_particles"
  if custom_particles.spawnCustomParticle(x, y, pType, pArg1) then
    return
  end
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let part_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Custom Particles Module\n".to_string());
            fs::write(mod_dir.join("custom_particles.lua"), part_lua).map_err(|e| e.to_string())?;
        }

        "sound_replacer" => {
            let _ = fs::create_dir_all(mod_dir.join("assets").join("sounds").join("weapons"));
            let _ = fs::create_dir_all(mod_dir.join("assets").join("sounds").join("explosions"));
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Sound Replacer Module
[[patches]]
[patches.module]
source = "sound_replacer.lua"
before = "main.lua"
name = "includes/sound_replacer"

# 2. Initialize in audioAudit.lua
[[patches]]
[patches.pattern]
target = "includes/audioAudit.lua"
pattern = 'function audioAuditCheck()'
position = "after"
payload = '''
  local sound_replacer = require "includes/sound_replacer"
  sound_replacer.init()
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let sound_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Sound Replacer Module\n".to_string());
            fs::write(mod_dir.join("sound_replacer.lua"), sound_lua).map_err(|e| e.to_string())?;
        }

        "custom_battle_music" => {
            let _ = fs::create_dir_all(mod_dir.join("assets").join("music"));
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Battle Music Jukebox Module
[[patches]]
[patches.module]
source = "battle_music.lua"
before = "main.lua"
name = "includes/battle_music"

# 2. Intercept Game Tick in main.lua:love.update
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'function love.update(dt)'
position = "after"
payload = '''
  local battle_music = require "includes/battle_music"
  battle_music.evaluateBattleIntensity()
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let music_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Battle Music Module\n".to_string());
            fs::write(mod_dir.join("battle_music.lua"), music_lua).map_err(|e| e.to_string())?;
        }

        "custom_weapon" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Custom Weapon Module
[[patches]]
[patches.module]
source = "custom_weapon.lua"
before = "main.lua"
name = "includes/custom_weapon"

# 2. Hook into urb_engine.lua weaponCreator
[[patches]]
[patches.pattern]
target = "includes/urb_engine.lua"
pattern = 'function weaponCreator(weapon, tag)'
position = "after"
payload = '''
  local custom_weapon = require "includes/custom_weapon"
  if custom_weapon.registerWeapon(weapon, tag) then
    return
  end
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let wep_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Custom Weapon Module\n".to_string());
            fs::write(mod_dir.join("custom_weapon.lua"), wep_lua).map_err(|e| e.to_string())?;
        }

        "custom_soldier_class" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Soldier Archetype Module
[[patches]]
[patches.module]
source = "custom_soldier.lua"
before = "main.lua"
name = "includes/custom_soldier"

# 2. Hook into urb_engine.lua createAlliedSoldier
[[patches]]
[patches.pattern]
target = "includes/urb_engine.lua"
pattern = 'function createAlliedSoldier(dude, soldierClass, x, y)'
position = "after"
payload = '''
  local custom_soldier = require "includes/custom_soldier"
  if custom_soldier.applyArchetype(dude, soldierClass, x, y) then
    return
  end
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let sol_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Custom Soldier Archetype Module\n".to_string());
            fs::write(mod_dir.join("custom_soldier.lua"), sol_lua).map_err(|e| e.to_string())?;
        }

        "custom_battle_condition" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Custom Battle Condition Module
[[patches]]
[patches.module]
source = "custom_bc.lua"
before = "main.lua"
name = "includes/custom_bc"

# 2. Hook into bcChecks.lua affectBc
[[patches]]
[patches.pattern]
target = "includes/bcChecks.lua"
pattern = 'function affectBc(bc)'
position = "after"
payload = '''
  local custom_bc = require "includes/custom_bc"
  custom_bc.affect(bc)
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let bc_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Custom BC Module\n".to_string());
            fs::write(mod_dir.join("custom_bc.lua"), bc_lua).map_err(|e| e.to_string())?;
        }

        "custom_reinforcement_card" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Custom Call-in Card Module
[[patches]]
[patches.module]
source = "custom_callin.lua"
before = "main.lua"
name = "includes/custom_callin"

# 2. Hook into trench.lua trenchSkillReset
[[patches]]
[patches.pattern]
target = "includes/trench.lua"
pattern = 'function trenchSkillReset()'
position = "after"
payload = '''
  local custom_callin = require "includes/custom_callin"
  custom_callin.registerCard()
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let callin_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Custom Call-in Card Module\n".to_string());
            fs::write(mod_dir.join("custom_callin.lua"), callin_lua).map_err(|e| e.to_string())?;
        }

        "custom_map_structure" => {
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register Custom Map Structure Module
[[patches]]
[patches.module]
source = "map_structure.lua"
before = "main.lua"
name = "includes/map_structure"

# 2. Hook into mapBuild.lua
[[patches]]
[patches.pattern]
target = "includes/mapBuild.lua"
pattern = 'function createTrenchLine(map)'
position = "after"
payload = '''
  local map_structure = require "includes/map_structure"
  map_structure.createConcretePillbox(200, 360, 0)
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let struct_lua = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| "-- Custom Map Structure Module\n".to_string());
            fs::write(mod_dir.join("map_structure.lua"), struct_lua).map_err(|e| e.to_string())?;
        }

        "single_anchor_patch" => {
            let qm_toml = tpl_def
                .map(|t| t.preview_code.to_string())
                .unwrap_or_else(|| {
                    format!(
                        r#"[manifest]
version = "1.0.0"
priority = 50

[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
print("[{mod_name}] Initialized cleanly.")
'''
"#
                    )
                });
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;
        }

        "multi_file_engine_hook" => {
            let core_file = format!("{clean_id}_interceptor.lua");
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# 1. Register standalone interceptor module
[[patches]]
[patches.module]
source = "{core_file}"
before = "main.lua"
name = "includes/{clean_id}_interceptor"

# 2. Initialize in main.lua
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
{clean_id}_interceptor = require "includes/{clean_id}_interceptor"
{clean_id}_interceptor.init()
'''

# 3. Intercept Game Tick
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'function love.update(dt)'
position = "after"
payload = '''
  if {clean_id}_interceptor and {clean_id}_interceptor.onTick then
    {clean_id}_interceptor.onTick(dt)
  end
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let interceptor_lua = format!(
                r#"-- {core_file} — Multi-File Engine Interceptor for {mod_name}
local M = {{}}

function M.init()
  print("[{mod_name}] Engine interceptor initialized.")
end

function M.onTick(dt)
  -- Persistent per-tick mod logic without destructive loop override
end

return M
"#
            );
            fs::write(mod_dir.join(&core_file), interceptor_lua).map_err(|e| e.to_string())?;
        }

        "asset_file_injection" => {
            let _ = fs::create_dir_all(mod_dir.join("assets").join("custom"));
            let mod_file = format!("{clean_id}_module.lua");
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 45

# 1. Register new Lua module into Love2D includes/
[[patches]]
[patches.module]
source = "{mod_file}"
before = "main.lua"
name = "includes/{clean_id}_module"

# 2. Mount mod asset directory to Love2D virtual filesystem
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'function love.load()'
position = "after"
payload = '''
  if love and love.filesystem and love.filesystem.mount then
    love.filesystem.mount("MnB-Mods/{clean_id}", "assets/{clean_id}")
  end
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;

            let module_lua = format!(
                r#"-- {mod_file} — Injected Custom Module for {mod_name}
local M = {{}}

function M.hello()
  return "Injected module '{mod_name}' loaded from Love2D virtual filesystem."
end

return M
"#
            );
            fs::write(mod_dir.join(&mod_file), module_lua).map_err(|e| e.to_string())?;
        }

        "faction_mod" => {
            let faction_toml = FACTION_TEMPLATE
                .replace("custom_faction", &clean_id.replace('-', "_"))
                .replace("Custom Faction", mod_name);
            fs::write(mod_dir.join("faction.toml"), faction_toml).map_err(|e| e.to_string())?;
            fs::write(
                mod_dir.join("faction.schema.json"),
                qm_schema::faction::FACTION_JSON_SCHEMA,
            )
            .map_err(|e| e.to_string())?;
            for sub in ["units/base", "units/sarge", "weapons", "voices", "vehicles", "names"] {
                let _ = fs::create_dir_all(mod_dir.join(sub));
            }
            fs::write(
                mod_dir.join("names").join("last.txt"),
                "# One last name per line. Lines starting with # are ignored.
Ivanov
Petrov
Smirnov
",
            )
            .map_err(|e| e.to_string())?;
        }

        _ => {
            // Minimal patch mod fallback ("empty" or unknown template)
            let qm_toml = format!(
                r#"[manifest]
version = "1.0.0"
priority = 50

# Add your [[patches]] blocks below:
[[patches]]
[patches.pattern]
target = "main.lua"
pattern = 'require "includes/directives"'
position = "after"
payload = '''
print("[{mod_name}] Initialized successfully!")
'''
"#
            );
            fs::write(mod_dir.join("qm.toml"), qm_toml).map_err(|e| e.to_string())?;
        }
    }

    let readme = format!(
        "# {mod_name}\n\nCreated with QuarterMaster Mod Scaffolder (Template: {norm_template}).\n\n## Features\n- Compatible with Mud and Blood `>=0.5.3`\n"
    );
    fs::write(mod_dir.join("README.md"), readme).map_err(|e| e.to_string())?;

    Ok(mod_dir)
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
    fn test_all_14_mod_templates_defined_and_discoverable() {
        assert!(BUILTIN_TEMPLATES.len() >= 14);
        let categories: std::collections::HashSet<&str> =
            BUILTIN_TEMPLATES.iter().map(|t| t.category).collect();
        assert!(categories.contains("Menu Frameworks & Options Pages"));
        assert!(categories.contains("Sprite & Graphics Replacements"));
        assert!(categories.contains("Audio & Music Replacements / Additions"));
        assert!(categories.contains("New Content & Gameplay Mechanics"));
        assert!(categories.contains("Core Code Patching & Engine Hooking"));

        for t in BUILTIN_TEMPLATES {
            assert!(!t.id.is_empty());
            assert!(!t.name.is_empty());
            assert!(t.descriptor.contains(" — "));
            assert!(t.preview_code.contains("-- ") || t.preview_code.contains("# "));
            assert!(!t.preview_code.contains(&format!("{}: {}", "TODO", "write mod")));
        }
    }

    #[test]
    fn test_scaffold_new_mod_all_templates() {
        let temp_dir = std::env::temp_dir().join(format!(
            "qm_scaffold_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let _ = fs::create_dir_all(&temp_dir);

        for t in BUILTIN_TEMPLATES {
            let mod_id = format!("test_{}", t.id);
            let mod_name = format!("Test {}", t.name);
            let res = scaffold_new_mod(&temp_dir, &mod_id, &mod_name, "UnitTester", t.id);
            assert!(
                res.is_ok(),
                "Failed to scaffold template '{}': {:?}",
                t.id,
                res.err()
            );
            let mod_dir = res.unwrap();
            assert!(mod_dir.join("mod.json").is_file());
            if t.id == "faction_mod" {
                // A faction mod is data only: faction.toml instead of qm.toml.
                assert!(mod_dir.join("faction.toml").is_file());
                assert!(!mod_dir.join("assets").exists());
            } else {
                assert!(mod_dir.join("qm.toml").is_file());
            }
            assert!(mod_dir.join("README.md").is_file());
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn faction_template_scaffolds_a_valid_faction() {
        let temp = tempfile::tempdir().unwrap();
        let dir = scaffold_new_mod(temp.path(), "eastern-front", "Eastern Front", "Tester", "faction_mod").unwrap();
        let report = qm_schema::faction::load_faction(&dir);
        assert!(!report.has_errors(), "{:?}", report.diagnostics);
        let f = report.faction.unwrap();
        assert_eq!(f.id, "eastern_front");
        assert_eq!(f.name, "Eastern Front");
        assert_eq!(f.names.unwrap().soldiers.unwrap().last, vec!["Ivanov", "Petrov", "Smirnov"]);
        // The commented-out examples are valid too once uncommented; only their files are missing.
        let key = regex_lite::Regex::new(r"^# (\[|[A-Za-z_]+ = )").unwrap();
        let uncommented: String = FACTION_TEMPLATE
            .lines()
            .map(|l| if key.is_match(l) { &l[2..] } else { l })
            .collect::<Vec<_>>()
            .join("
");
        let report = qm_schema::faction::check_faction(&uncommented, &dir);
        let unexpected: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == qm_schema::DiagnosticSeverity::Error)
            .filter(|d| !d.message.contains("not found in the mod folder") && !d.message.contains("no wav/ogg/mp3 files") && !d.message.contains("no png/jpg/jpeg/bmp files"))
            .collect();
        assert!(unexpected.is_empty(), "{unexpected:?}");
        let lint = lint_mod_directory(&dir, &HashMap::new()).unwrap();
        assert!(lint.diagnostics.iter().any(|d| d.message.starts_with("faction.toml:")));
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

