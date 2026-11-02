use anyhow::{bail, Context, Result};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::handshake::LoadOrderPlan;
use crate::patch::{Patch, PatchFile, Priority};
use itertools::Itertools;
use log::*;
use qm_schema::ModManifest;
use walkdir::WalkDir;
use zip::ZipArchive;

/// Patch file with preloaded TOML content and referenced sources
#[derive(Debug)]
struct IntermediatePatch {
    pub path: PathBuf,
    pub content: String,
    /// Preloaded source files referenced by module/copy patches (relative path -> content)
    pub sources: HashMap<PathBuf, String>,
}

pub fn patch_buffer(target: &str, rope: &mut crop::Rope) {
    // 1. Diagnostics: ALWAYS log every single buffer passing through
    info!("[DEBUG] patch_buffer called for target: '{}'", target);

    // 2. Prepend safety lockdown to the very start of main.lua (offset 0)
    if target.ends_with("main.lua") {
        info!("[DEBUG] Prepending Steam lockdown directly into byte 0 of {}", target);
        rope.insert(0, STEAM_SAFETY_PAYLOAD);
    }

    // 3. Directly defuse uploadScoresHelper in external.lua
    if target.ends_with("external.lua") {
        info!("[DEBUG] Defusing uploadScoresHelper in {}", target);
        let content = rope.to_string();
        if content.contains("function uploadScoresHelper") {
            let patched = content.replace(
                "function uploadScoresHelper(handle, scorez)",
                "function uploadScoresHelper(handle, scorez) return false end function _disabled_uploadScoresHelper(handle, scorez)",
            );

            let mut builder = crop::RopeBuilder::new();
            builder.append(&patched);
            *rope = builder.build();
            info!("[DEBUG] Successfully defused uploadScoresHelper in {}", target);
        }
    }
}

pub const STEAM_SAFETY_PAYLOAD: &str = r#"
-- Quartermaster: Steam Safety Lockdown & Auto-Asset Runtime
print("[QM] *** STEAM SAFETY HOOK INITIALIZED ***")

local function qm_neuter_steam()
  if Steam and Steam.userStats then
    Steam.userStats.uploadLeaderboardScore = function(h, m, s, d, cb)
      print("[QM] BLOCKED: Steam.userStats.uploadLeaderboardScore")
      if cb then cb({ success = false, globalRankNew = 0 }, "Quartermaster: Disabled") end
      return false
    end
    Steam.userStats.findLeaderboard = function(n, cb)
      print("[QM] BLOCKED: Steam.userStats.findLeaderboard (" .. tostring(n) .. ")")
      if cb then cb({ leaderboardFound = false, steamLeaderboard = 0 }, "Quartermaster: Disabled") end
      return false
    end
    Steam.userStats.setAchievement = function() return true end
    Steam.userStats.clearAchievement = function() return true end
    Steam.userStats.getAchievement = function() return false, false end
    Steam.userStats.setStatInt = function() return true end
    Steam.userStats.setStatFloat = function() return true end
    Steam.userStats.storeStats = function() return true end
    Steam.userStats.requestCurrentStats = function() return false end
    Steam.userStats.getStatInt = function() return false, 0 end
    Steam.userStats.getStatFloat = function() return false, 0.0 end
  end
  if Steam and Steam.apps then
    Steam.apps.isDlcInstalled = function() return false end
  end
end

qm_neuter_steam()

local blocked_globals = {
  uploadScoresHelper = true,
  steam_campaignTime = true,
  steam_classicWaves = true,
  steam_branderburging = true,
  steam_increaseClassicWins = true,
  steam_increaseCampaignWins = true,
  steam_campaignGG = true,
  steam_keener = true,
  steam_counterSnipe = true,
  steam_sink = true,
  steam_marquis = true,
  steam_mudBleed = true,
  steam_chunk = true,
  steam_comChallenge = true,
  steam_officerClub = true,
  steam_tigerHunter = true,
  steam_increaseBranderburg = true,
  steam_victoryDay = true,
}

for name, _ in pairs(blocked_globals) do
  _G[name] = function(...)
    print("[QM] BLOCKED CALL: " .. tostring(name))
    return false
  end
end

-- QuarterMaster Auto-Asset Runtime State
local __qm_obj_info = {}
local __qm_path_overrides = {}
local __qm_by_cat_stem = {}
local __qm_by_stem = {}
local __qm_var_lower = {}
local __qm_vanilla_keys = {}
local __qm_indexed_vanilla = false
local __qm_touched_audio_prefixes = {}
local __qm_touched_weapon_tags = {}
local __qm_pool_key_map = {}
local __qm_pools_by_prefix = {}

local orig_newSource = love and love.audio and love.audio.newSource or nil
local orig_newImage = love and love.graphics and love.graphics.newImage or nil

if orig_newSource then
  love.audio.newSource = function(a, b, ...)
    if type(a) == "string" then
      local norm = a:gsub("\\", "/"):gsub("^/+", "")
      local cat_stem = (norm:match("^(.+)%.[^%.]+$") or norm):lower()
      local stem = (norm:match("([^/]+)%.[^%.]+$") or norm):lower()
      local ov = __qm_path_overrides[norm:lower()] or __qm_path_overrides[cat_stem]
      if ov and ov.fd then
        local ok, s = pcall(orig_newSource, ov.fd, b or ov.mode or "static", ...)
        if ok and s then return s end
      end
      local src = orig_newSource(a, b, ...)
      if src then
        __qm_obj_info[src] = {
          kind = "audio",
          path = norm:lower(),
          cat_stem = cat_stem,
          stem = stem,
          mode = b or "static",
        }
      end
      return src
    end
    return orig_newSource(a, b, ...)
  end
end

if orig_newImage then
  love.graphics.newImage = function(a, ...)
    if type(a) == "string" then
      local norm = a:gsub("\\", "/"):gsub("^/+", "")
      local cat_stem = (norm:match("^(.+)%.[^%.]+$") or norm):lower()
      local stem = (norm:match("([^/]+)%.[^%.]+$") or norm):lower()
      local ov = __qm_path_overrides[norm:lower()] or __qm_path_overrides[cat_stem]
      if ov and ov.fd then
        local ok, img = pcall(orig_newImage, ov.fd, ...)
        if ok and img then return img end
      end
      local img = orig_newImage(a, ...)
      if img then
        __qm_obj_info[img] = {
          kind = "image",
          path = norm:lower(),
          cat_stem = cat_stem,
          stem = stem,
        }
      end
      return img
    end
    return orig_newImage(a, ...)
  end
end

local function __qm_build_vanilla_index()
  if __qm_indexed_vanilla then return end
  __qm_indexed_vanilla = true
  for k, v in pairs(_G) do
    if type(k) == "string" then
      __qm_var_lower[k:lower()] = k
      __qm_vanilla_keys[k] = true
      local info = __qm_obj_info[v]
      if info then
        local cs = info.cat_stem
        if cs then
          __qm_by_cat_stem[cs] = __qm_by_cat_stem[cs] or {}
          table.insert(__qm_by_cat_stem[cs], k)
        end
        local st = info.stem
        if st then
          __qm_by_stem[st] = __qm_by_stem[st] or {}
          table.insert(__qm_by_stem[st], k)
        end
      end
    end
  end
end

_G.__qm_auto_asset = function(mod_dir, rel_path, category, stem, ext, default_mode)
  __qm_build_vanilla_index()
  local full_path = tostring(mod_dir) .. "/" .. tostring(rel_path)
  local f = io and io.open and io.open(full_path, "rb") or nil
  if not f then
    print("[QM] WARN: Could not open auto-asset file: " .. full_path)
    return false
  end
  local bytes = f:read("*all")
  f:close()
  if not bytes then return false end

  local ext_l = tostring(ext or ""):lower()
  local cat_l = tostring(category or ""):lower()
  local stem_l = tostring(stem or ""):lower()

  if ext_l == "txt" then
    local text = bytes:gsub("^\239\187\191", ""):gsub("\r\n", "\n"):gsub("\n+$", "")
    local target_var = __qm_var_lower[stem_l] or stem
    rawset(_G, target_var, text)
    __qm_var_lower[target_var:lower()] = target_var
    return true
  end

  if not (love and love.filesystem and love.filesystem.newFileData) then
    return false
  end
  local ok_fd, fd = pcall(love.filesystem.newFileData, bytes, tostring(stem) .. "." .. ext_l)
  if not ok_fd or not fd then return false end

  local targets = {}
  local seen = {}
  local function add_target(name)
    if name and not seen[name] then
      seen[name] = true
      table.insert(targets, name)
    end
  end

  if rawget(_G, stem) ~= nil or __qm_pool_key_map[stem] ~= nil then
    add_target(stem)
  elseif __qm_var_lower[stem_l] then
    add_target(__qm_var_lower[stem_l])
  end

  local by_cs = __qm_by_cat_stem[cat_l .. "/" .. stem_l]
  if by_cs then
    for _, vname in ipairs(by_cs) do add_target(vname) end
  end

  if #targets == 0 then
    local by_s = __qm_by_stem[stem_l]
    if by_s then
      for _, vname in ipairs(by_s) do add_target(vname) end
    end
  end

  if #targets == 0 then
    add_target(stem)
  end

  __qm_path_overrides[cat_l .. "/" .. stem_l .. "." .. ext_l] = { fd = fd, mode = default_mode }
  __qm_path_overrides[cat_l .. "/" .. stem_l] = { fd = fd, mode = default_mode }

  if ext_l == "wav" or ext_l == "mp3" or ext_l == "ogg" then
    if not orig_newSource then return false end
    for _, var_name in ipairs(targets) do
      local old = rawget(_G, var_name)
      local mode = (default_mode and default_mode ~= "") and default_mode or "static"
      local was_loop = false
      local old_vol = nil
      local old_info = old and __qm_obj_info[old] or nil
      if old ~= nil and type(old) == "userdata" then
        pcall(function() mode = old:getType() or mode end)
        pcall(function() was_loop = old:isLooping() end)
        pcall(function() old_vol = old:getVolume() end)
        pcall(function() old:stop() end)
      end
      local ok_src, src = pcall(orig_newSource, fd, mode)
      if ok_src and src then
        if was_loop then pcall(function() src:setLooping(true) end) end
        if old_vol then pcall(function() src:setVolume(old_vol) end) end
        __qm_obj_info[src] = old_info or {
          kind = "audio",
          path = cat_l .. "/" .. stem_l .. "." .. ext_l,
          cat_stem = cat_l .. "/" .. stem_l,
          stem = stem_l,
          mode = mode,
        }
        if old_info then
          __qm_path_overrides[old_info.path] = { fd = fd, mode = mode }
          __qm_path_overrides[old_info.cat_stem] = { fd = fd, mode = mode }
        end
        rawset(_G, var_name, src)
        __qm_var_lower[var_name:lower()] = var_name
        local pfx = var_name:match("^(.-[^%d])(%d+)$")
        if pfx then __qm_touched_audio_prefixes[pfx] = true end
      end
    end
    return true
  elseif ext_l == "png" or ext_l == "jpg" or ext_l == "jpeg" or ext_l == "bmp" then
    if not orig_newImage then return false end
    local ok_img, img = pcall(orig_newImage, fd)
    if not ok_img or not img then return false end
    for _, var_name in ipairs(targets) do
      local old = rawget(_G, var_name)
      local old_info = old and __qm_obj_info[old] or nil
      __qm_obj_info[img] = old_info or {
        kind = "image",
        path = cat_l .. "/" .. stem_l .. "." .. ext_l,
        cat_stem = cat_l .. "/" .. stem_l,
        stem = stem_l,
      }
      if old_info then
        __qm_path_overrides[old_info.path] = { fd = fd }
        __qm_path_overrides[old_info.cat_stem] = { fd = fd }
      end
      rawset(_G, var_name, img)
      __qm_var_lower[var_name:lower()] = var_name
      local wep_tag = var_name:match("^top_(.+)$") or var_name:match("^side_(.+)$")
      if wep_tag then __qm_touched_weapon_tags[wep_tag] = true end
    end
    return true
  elseif ext_l == "ttf" or ext_l == "otf" then
    if not (love and love.graphics and love.graphics.newFont) then return false end
    local ok_fnt, fnt = pcall(love.graphics.newFont, fd, 16)
    if not ok_fnt or not fnt then return false end
    for _, var_name in ipairs(targets) do
      rawset(_G, var_name, fnt)
      __qm_var_lower[var_name:lower()] = var_name
    end
    return true
  end
  return false
end

_G.__qm_finalize_auto_assets = function()
  for tag, _ in pairs(__qm_touched_weapon_tags) do
    local wt_key = "weaponText_" .. tag
    if rawget(_G, wt_key) == nil then
      rawset(_G, wt_key, tag:upper() .. "\nCustom weapon loaded via QuarterMaster.")
    end
  end

  for pfx, _ in pairs(__qm_touched_audio_prefixes) do
    -- Restore any previously pooled entries into _G before rescanning
    local prev_pool = __qm_pools_by_prefix[pfx]
    if prev_pool then
      for idx, s in ipairs(prev_pool) do
        local k = pfx .. tostring(idx)
        if rawget(_G, k) == nil then rawset(_G, k, s) end
        __qm_pool_key_map[k] = nil
      end
    end

    local pool = {}
    local vanilla_count = 0
    local i = 1
    while true do
      local k = pfx .. tostring(i)
      local v = rawget(_G, k)
      if v == nil then break end
      table.insert(pool, v)
      if __qm_vanilla_keys[k] then vanilla_count = i end
      i = i + 1
    end

    if #pool >= 2 then
      __qm_pools_by_prefix[pfx] = pool
      -- If a mod added extra numbered variations beyond vanilla's count (e.g. garand5..6 when vanilla had 1..4),
      -- route lookups through __qm_pool_key_map so _G[pfx .. randNbr(vanilla_count)] rolls across 1..#pool.
      if vanilla_count > 0 and #pool > vanilla_count then
        for idx = 1, #pool do
          local k = pfx .. tostring(idx)
          __qm_pool_key_map[k] = pool
          rawset(_G, k, nil)
        end
      end
      -- If vanilla had a single sound at _G[pfx] (e.g. mg42Sound) and a mod added mg42Sound1..N,
      -- route _G[pfx] through the pool as well.
      local base_Name = pfx:gsub("_$", "")
      if __qm_vanilla_keys[base_Name] and rawget(_G, base_Name) ~= nil then
        __qm_pool_key_map[base_Name] = pool
        rawset(_G, base_Name, nil)
      end
    end
  end
end

local prev_mt = getmetatable(_G) or {}
local prev_index = prev_mt.__index
local prev_newindex = prev_mt.__newindex

setmetatable(_G, {
  __index = function(t, k)
    local pool = __qm_pool_key_map[k]
    if pool and #pool > 0 then
      return pool[math.random(1, #pool)]
    end
    if type(k) == "string" then
      local direct_pool = __qm_pools_by_prefix[k] or __qm_pools_by_prefix[k .. "_"]
      if direct_pool and #direct_pool > 0 then
        return direct_pool[math.random(1, #direct_pool)]
      end
      local pfx = k:match("^(.-[^%d])(%d+)$")
      if pfx and __qm_pools_by_prefix[pfx] then
        local p = __qm_pools_by_prefix[pfx]
        return p[math.random(1, #p)]
      end
    end
    if type(prev_index) == "function" then
      return prev_index(t, k)
    elseif type(prev_index) == "table" then
      return prev_index[k]
    end
    return nil
  end,
  __newindex = function(t, k, v)
    if blocked_globals[k] then
      rawset(t, k, function(...)
        print("[QM] BLOCKED CALL: " .. tostring(k))
        return false
      end)
    elseif k == "Steam" then
      rawset(t, k, v)
      qm_neuter_steam()
    else
      if type(prev_newindex) == "function" then
        prev_newindex(t, k, v)
      else
        rawset(t, k, v)
      end
    end
  end
})
"#;

/// Compare two file paths by their lowercase filenames
fn filename_cmp(first: &Path, second: &Path) -> Ordering {
    let first = first.file_name().unwrap().to_string_lossy().to_lowercase();
    let second = second.file_name().unwrap().to_string_lossy().to_lowercase();
    first.cmp(&second)
}

/// Metadata for an asset file discovered under a mod's `assets/` folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredAsset {
    pub rel_path: String,
    pub category: String,
    pub stem: String,
    pub ext: String,
    pub default_mode: String,
}

/// Classifies a relative path from a mod root (e.g. `"assets/sounds/garand1.wav"`) into a `DiscoveredAsset`.
/// Returns `None` if the path is not inside `assets/`, is a hidden file, or has an unsupported extension.
pub fn classify_asset_relative_path(rel_from_mod_root: &str) -> Option<DiscoveredAsset> {
    let norm = rel_from_mod_root.replace('\\', "/");
    let norm = norm.trim_start_matches('/');
    let parts: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 || !parts[0].eq_ignore_ascii_case("assets") {
        return None;
    }
    if parts.iter().any(|seg| seg.starts_with('.')) {
        return None;
    }

    let file_name = *parts.last()?;
    let dot_idx = file_name.rfind('.')?;
    if dot_idx == 0 || dot_idx + 1 >= file_name.len() {
        return None;
    }
    let stem = file_name[..dot_idx].to_string();
    let ext = file_name[dot_idx + 1..].to_ascii_lowercase();

    let is_audio = matches!(ext.as_str(), "wav" | "mp3" | "ogg");
    let is_image = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp");
    let is_font = matches!(ext.as_str(), "ttf" | "otf");
    let is_text = ext == "txt";

    if !is_audio && !is_image && !is_font && !is_text {
        return None;
    }

    let subdirs: Vec<String> = parts[1..parts.len() - 1]
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let (category, default_mode) = match subdirs.first().map(|s| s.as_str()) {
        Some("sounds") | Some("sfx") | Some("audio") => {
            if subdirs.get(1).is_some_and(|s| s == "vo" || s == "voice" || s == "voices") {
                ("sounds/vo".to_string(), "stream".to_string())
            } else {
                ("sounds".to_string(), "static".to_string())
            }
        }
        Some("vo") | Some("voice") | Some("voices") => {
            ("sounds/vo".to_string(), "stream".to_string())
        }
        Some("music") | Some("bgm") => ("music".to_string(), "stream".to_string()),
        Some("ui") => ("ui".to_string(), String::new()),
        Some("cheevos") => ("cheevos".to_string(), String::new()),
        Some("splash") => ("splash".to_string(), String::new()),
        Some("sprites") | Some("gfx") | Some("images") => {
            ("sprites".to_string(), String::new())
        }
        Some("fonts") => ("fonts".to_string(), String::new()),
        Some("text") | Some("descriptions") => ("text".to_string(), String::new()),
        _ => {
            if is_audio {
                ("sounds".to_string(), "static".to_string())
            } else if is_image {
                ("sprites".to_string(), String::new())
            } else if is_font {
                ("fonts".to_string(), String::new())
            } else {
                ("text".to_string(), String::new())
            }
        }
    };

    let canonical_rel = format!("assets/{}", parts[1..].join("/"));
    Some(DiscoveredAsset {
        rel_path: canonical_rel,
        category,
        stem,
        ext,
        default_mode,
    })
}

/// Recursively scans `<mod_dir>/assets` and returns all supported assets in deterministic path order.
pub fn discover_dir_assets(mod_dir: &Path) -> Vec<DiscoveredAsset> {
    let assets_dir = mod_dir.join("assets");
    if !assets_dir.is_dir() {
        return Vec::new();
    }
    let mut assets: Vec<DiscoveredAsset> = WalkDir::new(&assets_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let rel = e.path().strip_prefix(mod_dir).ok()?;
            classify_asset_relative_path(&rel.to_string_lossy())
        })
        .collect();
    assets.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    assets
}

/// Synthesizes a `Patch::Pattern` that auto-loads all discovered assets in `<mod_id>` immediately before `setOptionVolume()`.
pub fn build_auto_asset_patch(
    mod_id: &str,
    asset_root_dir: &Path,
    assets: &[DiscoveredAsset],
    priority: Priority,
) -> Option<Patch> {
    if assets.is_empty() {
        return None;
    }
    let escaped_dir = asset_root_dir
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"");

    let mut payload = format!(
        "-- QuarterMaster Auto-Asset Patch ({mod_id}: {} files)\nif _G.__qm_auto_asset then\n  local _qm_mod_dir = \"{escaped_dir}\"\n",
        assets.len()
    );
    for a in assets {
        let rel_esc = a.rel_path.replace('\\', "/").replace('"', "\\\"");
        let cat_esc = a.category.replace('"', "\\\"");
        let stem_esc = a.stem.replace('"', "\\\"");
        let ext_esc = a.ext.replace('"', "\\\"");
        let mode_esc = a.default_mode.replace('"', "\\\"");
        payload.push_str(&format!(
            "  _G.__qm_auto_asset(_qm_mod_dir, \"{rel_esc}\", \"{cat_esc}\", \"{stem_esc}\", \"{ext_esc}\", \"{mode_esc}\")\n"
        ));
    }
    payload.push_str("  if _G.__qm_finalize_auto_assets then _G.__qm_finalize_auto_assets() end\nend");

    Some(Patch::Pattern(crate::patch::PatternPatch {
        pattern: "setOptionVolume()".to_string(),
        position: crate::patch::InsertPosition::Before,
        target: crate::patch::Target::Single("main.lua".to_string()),
        payload,
        match_indent: true,
        times: Some(1),
        overwrite: false,
        id: Some(format!("{mod_id}_auto_assets")),
        name: Some(format!("Auto-Load Assets ({} files)", assets.len())),
        description: Some(format!(
            "Automatically loads and replaces {} asset file(s) from assets/.",
            assets.len()
        )),
        requires: Vec::new(),
        priority: Some(priority),
        before: Vec::new(),
        after: Vec::new(),
        on_failure: crate::patch::OnFailurePolicy::Warn,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
    }))
}

/// Extracts and discovers `assets/` inside a `.zip` mod archive if present.
fn discover_zip_assets(zip_file: &Path) -> Result<(PathBuf, Vec<DiscoveredAsset>)> {
    let file = fs::File::open(zip_file)
        .with_context(|| format!("Failed to open zip file at {:?}", zip_file))?;
    let mut zip = ZipArchive::new(file)
        .with_context(|| format!("Failed to open zip archive for {:?}", zip_file))?;

    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    let mod_root = names
        .iter()
        .find_map(|name| {
            let parent = get_parent(name);
            let qm_toml = format!("{}qm.toml", parent);
            let qm_dir = format!("{}qm/", parent);
            if names.iter().any(|n| n == &qm_toml || n.starts_with(&qm_dir)) {
                Some(parent)
            } else {
                None
            }
        })
        .or_else(|| {
            names.iter().find_map(|name| {
                let norm = name.replace('\\', "/");
                if let Some(idx) = norm.find("assets/") {
                    if idx == 0 || norm.as_bytes().get(idx - 1) == Some(&b'/') {
                        let parent = &norm[..idx];
                        let rel = &norm[idx..];
                        if classify_asset_relative_path(rel).is_some() {
                            return Some(parent.to_string());
                        }
                    }
                }
                None
            })
        });

    let Some(mod_root) = mod_root else {
        return Ok((zip_file.to_path_buf(), Vec::new()));
    };

    let zip_stem = zip_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("zip_mod");
    let extract_root = zip_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("qm")
        .join("zip_assets")
        .join(zip_stem);

    let mut discovered = Vec::new();
    for name in &names {
        if name.ends_with('/') || name.ends_with('\\') || !name.starts_with(&mod_root) {
            continue;
        }
        let rel = &name[mod_root.len()..];
        if let Some(asset) = classify_asset_relative_path(rel) {
            if let Ok(mut entry) = zip.by_name(name) {
                let out_path = extract_root.join(&asset.rel_path);
                if let Some(p) = out_path.parent() {
                    let _ = fs::create_dir_all(p);
                }
                let mut buf = Vec::new();
                if entry.read_to_end(&mut buf).is_ok() {
                    let _ = fs::write(&out_path, &buf);
                    discovered.push(asset);
                }
            }
        }
    }

    discovered.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok((extract_root, discovered))
}

/// Load patch files from the specified mod directory.
fn get_dir_patches(mod_dir: &Path) -> Result<(PathBuf, Vec<IntermediatePatch>)> {
    let mut toml_files = Vec::new();

    // Check root manifest candidates in order of priority:
    // qm.toml, quartermaster.toml, lovely.toml
    for name in &["qm.toml", "quartermaster.toml"] {
        let p = mod_dir.join(name);
        if p.is_file() {
            toml_files.push(p);
            break;
        }
    }

    // Check patch subdirectories: qm/ or lovely/
    for dir_name in &["qm"] {
        let dir_path = mod_dir.join(dir_name);
        if dir_path.is_dir() {
            let mut subfiles = WalkDir::new(&dir_path)
                .into_iter()
                .filter_map(|x| x.ok())
                .map(|x| x.path().to_path_buf())
                .filter(|x| x.is_file())
                .filter(|x| x.extension().is_some_and(|x| x == "toml"))
                .sorted_by(|a, b| filename_cmp(a, b))
                .collect_vec();
            toml_files.append(&mut subfiles);
            break;
        }
    }

    let intermediate_patches = toml_files
        .into_iter()
        .map(|toml_path| {
            let content = fs::read_to_string(&toml_path)
                .with_context(|| format!("Failed to read patch file at {:?}", toml_path))?;

            // Parse TOML to find referenced sources and preload them
            let mut sources: HashMap<PathBuf, String> = HashMap::new();
            let file_identifier = format!("{:?}", toml_path);
            if let Ok(patch_file) = parse_patch_file(&content, &file_identifier, mod_dir) {
                for patch in &patch_file.patches {
                    match patch {
                        Patch::Module(x) => {
                            let full_path = mod_dir.join(&x.source);
                            if let Ok(source_content) = fs::read_to_string(&full_path) {
                                sources.insert(x.source.clone(), source_content);
                            }
                        }
                        Patch::Copy(x) => {
                            let Some(ref copy_sources) = x.sources else { continue };
                            for source in copy_sources {
                                let full_path = mod_dir.join(source);
                                if let Ok(source_content) = fs::read_to_string(&full_path) {
                                    sources.insert(source.clone(), source_content);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            Ok(IntermediatePatch {
                path: toml_path,
                content,
                sources,
            })
        })
        .collect::<Result<Vec<IntermediatePatch>>>()?;

    Ok((mod_dir.to_path_buf(), intermediate_patches))
}

/// Load patch files from the specified zip.
fn get_zip_patches(zip_file: &Path) -> Result<(PathBuf, Vec<IntermediatePatch>)> {
    let file = fs::File::open(zip_file)
        .with_context(|| format!("Failed to open zip file at {:?}", zip_file))?;
    let mut zip = ZipArchive::new(file)
        .with_context(|| format!("Failed to open zip archive for {:?}", zip_file))?;

    // Grab files names
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    // Find the mod root. This is the dir that contains lovely.toml or lovely/
    let mod_root = names
        .iter()
        .find_map(|name| {
            let parent = get_parent(name);
            let qm_toml = format!("{}qm.toml", parent);
            let qm_dir = format!("{}qm/", parent);

            if names
                .iter()
                .any(|n| n == &qm_toml || n.starts_with(&qm_dir))
            {
                Some(parent)
            } else {
                None
            }
        });
    let mod_root = match mod_root {
            Some(v) => v,
            None => {
                log::warn!("No mod root found in zip {:?}. This may happen if the mod does not contain any QuarterMaster (qm.toml) patches (uses another loader)", zip_file);
                return Ok((zip_file.to_path_buf(), Vec::new()));
            },
    };


    let qm_toml_path = format!("{}qm.toml", mod_root);
    let qm_dir_prefix = format!("{}qm/", mod_root);

    let mut toml_paths: Vec<String> = names
        .iter()
        .filter(|name| {
            !name.ends_with('/')
                && (name == &&qm_toml_path
                    || (name.starts_with(&qm_dir_prefix) && name.ends_with(".toml")))
        })
        .cloned()
        .collect();

    // Sort toml paths by filename
    toml_paths.sort_by(|a, b| {
        let a_path = Path::new(a);
        let b_path = Path::new(b);
        filename_cmp(a_path, b_path)
    });

    // First pass: read all TOML files and parse them to find referenced sources
    let mut toml_contents: Vec<(String, String)> = Vec::new();
    let mut source_paths: HashSet<String> = HashSet::new();

    for toml_path in &toml_paths {
        let mut file = zip
            .by_name(toml_path)
            .with_context(|| format!("Failed to read {} from zip {:?}", toml_path, zip_file))?;

        let mut content = String::new();
        file.read_to_string(&mut content).with_context(|| {
            format!("Failed to read contents of {} from zip {:?}", toml_path, zip_file)
        })?;

        // Parse TOML to find referenced module/copy sources
        let file_identifier = format!("{} from zip {:?}", toml_path, zip_file);
        if let Ok(patch_file) = parse_patch_file(&content, &file_identifier, zip_file) {
            for patch in &patch_file.patches {
                match patch {
                    Patch::Module(x) => {
                        let source_path = format!("{}{}", mod_root, x.source.to_string_lossy());
                        source_paths.insert(source_path);
                    }
                    Patch::Copy(x) => {
                        if let Some(ref sources) = x.sources {
                            for source in sources {
                                let source_path = format!("{}{}", mod_root, source.to_string_lossy());
                                source_paths.insert(source_path);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        toml_contents.push((toml_path.clone(), content));
    }

    // Second pass: read all referenced source files from zip
    let mut file_contents: HashMap<String, String> = HashMap::new();
    for source_path in &source_paths {
        if let Ok(mut file) = zip.by_name(source_path) {
            let mut content = String::new();
            file.read_to_string(&mut content).with_context(|| {
                format!("Failed to read source {} from zip {:?}", source_path, zip_file)
            })?;
            file_contents.insert(source_path.clone(), content);
        }
    }

    // Build IntermediatePatches with preloaded sources
    let intermediate_patches = toml_contents
        .into_iter()
        .map(|(toml_path, content)| {
            // Create intermediate path: zip_file/relative_path_from_mod_root
            let relative_path = &toml_path[mod_root.len()..];
            let intermediate_path = zip_file.join(relative_path);

            // Build sources map with paths relative to mod_root
            let mut sources: HashMap<PathBuf, String> = HashMap::new();
            for (full_path, content) in &file_contents {
                let relative = &full_path[mod_root.len()..];
                sources.insert(PathBuf::from(relative), content.clone());
            }

            IntermediatePatch {
                path: intermediate_path,
                content,
                sources,
            }
        })
        .collect();

    Ok((zip_file.to_path_buf(), intermediate_patches))
}

/// Load patches from the provided mod directory. This scans for lovely patch files
/// within each subdirectory that matches either:
/// - MOD_DIR/lovely.toml
/// - MOD_DIR/lovely/*.toml
/// 
/// Zip archives are supported and uniquely support directory nesting 
/// (i.e., mod.zip/dir/lovely.toml), but otherwise are treated the same as dir mods.
pub fn load_patches_new(mod_dir: &Path) -> Result<Vec<(Patch, Priority, PathBuf, HashMap<String, String>)>> {
    let blacklist_file = mod_dir.join("qm").join("blacklist.txt");

    let mut blacklist: HashSet<String> = HashSet::new();
    if fs::exists(&blacklist_file)? {
        let text = fs::read_to_string(blacklist_file).context("Could not read blacklist")?;

        blacklist.extend(
            text.lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| line.to_string()),
        );
    } else {
        info!("No blacklist.txt in Mods/qm.");
    }

    let mod_contents = fs::read_dir(mod_dir)
        .with_context(|| format!("Failed to read from mod directory within {mod_dir:?}"))?
        .filter_map(|x| x.ok())
        .map(|x| x.path())
        .filter(|x| {
            let cname = x.file_name();
            let name = cname.and_then(|x| x.to_str()).unwrap_or_default();
            let blacklisted = blacklist.contains(name);
            if blacklisted {
                info!("'{name}' was found in blacklist, skipping it.");
            }
            !blacklisted
        })
        .collect_vec();

    // Collect directory patches (read TOMLs into IntermediatePatch) and discovered assets
    let dir_results: Vec<(PathBuf, Vec<IntermediatePatch>, PathBuf, Vec<DiscoveredAsset>)> =
        mod_contents
            .iter()
            .filter(|x| x.is_dir())
            .filter(|x| {
                let ignore_file = x.join(".qmignore");
                let dirname = x
                    .file_name()
                    .unwrap_or_else(|| panic!("Failed to read directory name of {x:?}"))
                    .to_string_lossy();
                if dirname == "qm" || dirname.starts_with('.') {
                    return false;
                }
                if ignore_file.is_file() {
                    info!("Found .qmignore in '{dirname}', skipping it.");
                }
                !ignore_file.is_file()
            })
            .sorted_by(|a, b| filename_cmp(a, b))
            .map(|x| {
                let (bp, ips) = get_dir_patches(x)?;
                let assets = discover_dir_assets(x);
                Ok((bp, ips, x.to_path_buf(), assets))
            })
            .collect::<Result<Vec<_>>>()?;

    // Collect zip patches (read TOMLs into IntermediatePatch) and discovered assets
    let zip_results: Vec<(PathBuf, Vec<IntermediatePatch>, PathBuf, Vec<DiscoveredAsset>)> =
        mod_contents
            .iter()
            .filter(|x| x.is_file())
            .filter(|x| x.extension().is_some_and(|ext| ext == "zip"))
            .sorted_by(|a, b| filename_cmp(a, b))
            .map(|x| {
                let (bp, ips) = get_zip_patches(x)?;
                let (asset_root, assets) = discover_zip_assets(x)?;
                Ok((bp, ips, asset_root, assets))
            })
            .collect::<Result<Vec<_>>>()?;

    // Parse TOML contents into PatchFile structures
    let mut patches: Vec<(Patch, Priority, PathBuf, HashMap<String, String>)> = Vec::new();

    // Handle all patch files using preloaded sources
    let all_results = dir_results.into_iter().chain(zip_results.into_iter());
    let load_order_plan = LoadOrderPlan::load_from_mod_dir(mod_dir);
    let replaced_patch_ids: HashSet<String> = load_order_plan
        .as_ref()
        .map(|plan| {
            plan.glue_patches
                .iter()
                .filter_map(|g| g.replaces_patch_id.clone())
                .collect()
        })
        .unwrap_or_default();

    for (base_path, ips, asset_root, discovered_assets) in all_results {
        if ips.is_empty() && discovered_assets.is_empty() {
            continue;
        }

        let folder_name = if base_path.extension().is_some_and(|e| e == "zip") {
            base_path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            base_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        };

        // Read mod.json if present for authoritative mod_id and load_priority
        let manifest_meta = ModManifest::from_path(&base_path.join("mod.json")).ok();
        let mod_id = manifest_meta
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_else(|| folder_name.to_lowercase());

        if let Some(ref plan) = load_order_plan {
            if !plan.enabled_mods.iter().any(|m| {
                m.eq_ignore_ascii_case(&mod_id) || m.eq_ignore_ascii_case(&folder_name)
            }) {
                info!(
                    "Skipping mod '{}' ({}) as it is not in load-order.json enabled_mods.",
                    mod_id, folder_name
                );
                continue;
            }
        }

        // Determine mod-level base priority for synthesized auto-asset patches
        let first_toml_prio = ips.first().and_then(|ip| {
            let fid = format!("{:?}", ip.path);
            parse_patch_file(&ip.content, &fid, &base_path)
                .ok()
                .map(|pf| pf.manifest.priority)
        });
        let mut mod_base_priority = manifest_meta
            .as_ref()
            .map(|m| m.load_priority)
            .or(first_toml_prio)
            .unwrap_or(50);
        if let Some(ref plan) = load_order_plan {
            if let Some(&override_prio) = plan
                .mod_priorities
                .get(&mod_id)
                .or_else(|| plan.mod_priorities.get(&folder_name))
            {
                mod_base_priority = override_prio;
            }
        }

        if let Some(mut auto_patch) =
            build_auto_asset_patch(&mod_id, &asset_root, &discovered_assets, mod_base_priority)
        {
            let patch_id = auto_patch.id().unwrap_or_default().to_string();
            let named_id = format!("{}:{}", mod_id, auto_patch.name().unwrap_or(""));
            if !replaced_patch_ids.contains(&patch_id) && !replaced_patch_ids.contains(&named_id) {
                if let Some(ref plan) = load_order_plan {
                    if plan.disabled_patches.contains(&patch_id)
                        || plan.disabled_patches.contains(&named_id)
                    {
                        auto_patch.set_disabled(true);
                    }
                }
                let mut effective_prio = mod_base_priority;
                if let Some(ref plan) = load_order_plan {
                    if let Some(&p_override) = plan
                        .patch_priorities
                        .get(&patch_id)
                        .or_else(|| plan.patch_priorities.get(&named_id))
                    {
                        effective_prio = p_override;
                    }
                }
                let rel_assets_path = base_path
                    .strip_prefix(mod_dir)
                    .unwrap_or(&base_path)
                    .join("assets");
                patches.push((auto_patch, effective_prio, rel_assets_path, HashMap::new()));
            }
        }

        for ip in ips {
            let file_identifier = format!("{:?}", ip.path);
            let comment_names = extract_patch_comments(&ip.content);
            let mut patch_file: PatchFile =
                parse_patch_file(&ip.content, &file_identifier, &base_path)?;

            // Base priority: load-order.json override > mod.json load_priority > qm.toml [manifest].priority
            let mut base_priority = manifest_meta
                .as_ref()
                .map(|m| m.load_priority)
                .unwrap_or(patch_file.manifest.priority);
            if let Some(ref plan) = load_order_plan {
                if let Some(&override_prio) = plan
                    .mod_priorities
                    .get(&mod_id)
                    .or_else(|| plan.mod_priorities.get(&folder_name))
                {
                    base_priority = override_prio;
                }
            }

            // For module and copy patches, use preloaded sources
            for (patch_idx, patch) in patch_file.patches.iter_mut().enumerate() {
                let target_clean = patch
                    .primary_target()
                    .replace(['/', '\\', '.'], "_")
                    .to_lowercase();
                let fallback_id = format!("{}:{}:{}", mod_id, target_clean, patch_idx + 1);
                let comment_name = comment_names.get(patch_idx).cloned().flatten();
                patch.set_metadata(mod_id.clone(), fallback_id, comment_name);

                if let Patch::Module(ref mut x) = patch {
                    if x.load_now && x.before.is_none() {
                        bail!(
                            "Error at patch file {}:\nModule \"{}\" has \"load_now\" set to true, but does not have required parameter \"before\" set",
                            ip.path.display(),
                            x.name
                        );
                    }

                    x.display_source = x.source.to_string_lossy().to_string();
                    x.content = ip
                        .sources
                        .get(&x.source)
                        .with_context(|| {
                            format!(
                                "Module source {:?} not found in preloaded sources for patch from {}",
                                x.source,
                                ip.path.display()
                            )
                        })?
                        .clone();
                }

                let Patch::Copy(ref mut x) = patch else {
                    continue;
                };
                let Some(ref sources) = x.sources else {
                    continue;
                };

                for source in sources {
                    let source_content = ip.sources.get(source).with_context(|| {
                        format!(
                            "Copy source {:?} not found in preloaded sources for patch from {}",
                            source,
                            ip.path.display()
                        )
                    })?;
                    x.contents.push(source_content.clone());
                }
            }

            let vars = patch_file.vars;

            // mod_relative_path: path relative to top-level mod_dir
            let mod_relative_path = ip.path.strip_prefix(mod_dir).with_context(|| {
                format!(
                    "Base mod directory path {} expected to be a prefix of patch file path {}",
                    mod_dir.display(),
                    ip.path.display()
                )
            })?;

            // --- Log output immediately after each mod manifest loads ---
            info!(
                "Loaded manifest: '{}' (mod_id: '{}') | Priority: {} | Patches: {}",
                mod_relative_path.display(),
                mod_id,
                base_priority,
                patch_file.patches.len()
            );

            for mut patch in patch_file.patches {
                let patch_id = patch.id().unwrap_or_default().to_string();
                let named_id = format!("{}:{}", mod_id, patch.name().unwrap_or(""));
                if replaced_patch_ids.contains(&patch_id) || replaced_patch_ids.contains(&named_id) {
                    info!(
                        "Replacing patch '{}' with profile glue patch from load-order.json",
                        patch_id
                    );
                    continue;
                }
                if let Some(ref plan) = load_order_plan {
                    if plan.disabled_patches.contains(&patch_id)
                        || plan.disabled_patches.contains(&named_id)
                    {
                        info!("Marking disabled patch '{}' per load-order.json", patch_id);
                        patch.set_disabled(true);
                    }
                }
                let mut effective_prio = patch.priority_override().unwrap_or(base_priority);
                if let Some(ref plan) = load_order_plan {
                    if let Some(&p_override) = plan
                        .patch_priorities
                        .get(&patch_id)
                        .or_else(|| plan.patch_priorities.get(&named_id))
                    {
                        effective_prio = p_override;
                    }
                }
                patches.push((
                    patch,
                    effective_prio,
                    mod_relative_path.to_path_buf(),
                    vars.clone(),
                ));
            }
        }
    }

    // Append any profile-local glue patches from load-order.json
    if let Some(plan) = load_order_plan {
        for glue in plan.glue_patches {
            let mut p = glue.patch;
            p.set_metadata(
                "qm_glue".to_string(),
                glue.id.clone(),
                Some(glue.name.clone()),
            );
            patches.push((
                p,
                glue.priority,
                PathBuf::from("qm/load-order.json"),
                HashMap::new(),
            ));
        }
    }

    Ok(patches)
}

/// Process raw patches to extract targets and consolidate variables
pub fn process_patches(
    raw_patches: Vec<(Patch, Priority, PathBuf, HashMap<String, String>)>,
) -> (
    Vec<(Patch, Priority, PathBuf)>,
    HashSet<String>,
    HashMap<String, String>,
) {
    let mut targets: HashSet<String> = HashSet::new();
    let mut patches: Vec<(Patch, Priority, PathBuf)> = Vec::new();
    let mut var_table: HashMap<String, String> = HashMap::new();

    for (patch, priority, path, vars) in raw_patches {
        // Extract targets from patches
        match &patch {
            Patch::Copy(x) => {
                x.target.insert_into(&mut targets);
            }
            Patch::Module(x) => {
                targets.insert(x.before.clone().unwrap_or_default());
            }
            Patch::Pattern(x) => {
                x.target.insert_into(&mut targets);
            }
            Patch::Regex(x) => {
                x.target.insert_into(&mut targets);
            }
        }

        // Add to final patches
        patches.push((patch, priority, path));

        // Add variables (later ones override earlier ones)
        var_table.extend(vars);
    }

    (patches, targets, var_table)
}

/// Parse TOML content into a PatchFile
fn parse_patch_file(content: &str, file_identifier: &str, mod_dir: &Path) -> Result<PatchFile> {
    // HACK: Replace instances of {{qm_hack:patch_dir}} with mod directory.
    let clean_mod_dir = &mod_dir.to_string_lossy().replace("\\", "\\\\");
    let content = content.replace("{{qm_hack:patch_dir}}", clean_mod_dir);

    let ignored_key_callback = |key: serde_ignored::Path| {
        warn!("Unknown key `{key}` found in patch file {file_identifier}, ignoring it");
    };

    serde_ignored::deserialize(toml::Deserializer::new(&content), ignored_key_callback)
        .with_context(|| format!("Failed to parse patch file {file_identifier}"))
}



/// Extract preceding `# comment` headers above each `[[patches]]` block in a TOML string.
pub fn extract_patch_comments(content: &str) -> Vec<Option<String>> {
    let mut results = Vec::new();
    let mut pending_comments: Vec<String> = Vec::new();
    let mut in_multiline_str = false;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        let triple_count = raw_line.matches("'''").count() + raw_line.matches("\"\"\"").count();
        if triple_count % 2 == 1 {
            in_multiline_str = !in_multiline_str;
            continue;
        }
        if in_multiline_str {
            continue;
        }

        if let Some(comment) = trimmed.strip_prefix('#') {
            let clean = comment.trim().trim_matches('=').trim_matches('-').trim();
            if !clean.is_empty() {
                pending_comments.push(clean.to_string());
            }
        } else if trimmed == "[[patches]]" {
            let chosen = pending_comments.first().cloned();
            results.push(chosen);
            pending_comments.clear();
        } else if !trimmed.is_empty() {
            pending_comments.clear();
        }
    }

    results
}

/// Helper to extract parent directory path with trailing slash
fn get_parent(path: &str) -> String {
    path.rfind('/')
        .map(|i| &path[..=i])
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    const PATCH_TOML: &str = r#"
[manifest]
version = "1.0.0"

[[patches]]
[patches.copy]
target = "main.lua"
position = "append"
sources = ["inject.lua"]
"#;

    fn make_zip(temp: &TempDir, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let path = temp.path().join(name);
        let mut zip = ZipWriter::new(fs::File::create(&path).unwrap());
        for (name, content) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn dir_patches_preloads_sources() {
        let temp = TempDir::new().unwrap();
        let mod_dir = temp.path().join("mod");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("qm.toml"), PATCH_TOML).unwrap();
        fs::write(mod_dir.join("inject.lua"), "-- injected").unwrap();

        let (_, patches) = get_dir_patches(&mod_dir).unwrap();

        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].sources.get(Path::new("inject.lua")).unwrap(), "-- injected");
    }

    #[test]
    fn zip_patches_preloads_sources() {
        let temp = TempDir::new().unwrap();
        let zip = make_zip(&temp, "mod.zip", &[
            ("qm.toml", PATCH_TOML),
            ("inject.lua", "-- from zip"),
        ]);

        let (_, patches) = get_zip_patches(&zip).unwrap();

        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].sources.get(Path::new("inject.lua")).unwrap(), "-- from zip");
    }

    #[test]
    fn zip_nested_mod_root() {
        let temp = TempDir::new().unwrap();
        let zip = make_zip(&temp, "mod.zip", &[
            ("Nested/qm.toml", PATCH_TOML),
            ("Nested/inject.lua", "-- nested"),
        ]);

        let (_, patches) = get_zip_patches(&zip).unwrap();

        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].sources.get(Path::new("inject.lua")).unwrap(), "-- nested");
    }

    #[test]
    fn zip_no_mod_root_is_empty() {
        let temp = TempDir::new().unwrap();
        let zip = make_zip(&temp, "empty.zip", &[("random.txt", "")]);

        let (_, patches) = get_zip_patches(&zip).unwrap();
        assert!(patches.is_empty());
    }

    #[test]
    fn blacklist_excludes_dir_and_zip() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();
        fs::write(
            mods.join("qm/blacklist.txt"),
            "blocked_dir\nblocked.zip\n# comment\n\n",
        ).unwrap();

        // Allowed dir mod
        let allowed = mods.join("allowed");
        fs::create_dir_all(&allowed).unwrap();
        fs::write(allowed.join("qm.toml"), PATCH_TOML).unwrap();
        fs::write(allowed.join("inject.lua"), "").unwrap();

        // Blocked dir mod
        let blocked = mods.join("blocked_dir");
        fs::create_dir_all(&blocked).unwrap();
        fs::write(blocked.join("qm.toml"), PATCH_TOML).unwrap();
        fs::write(blocked.join("inject.lua"), "").unwrap();

        // Blocked zip mod
        make_zip(&temp, "blocked.zip", &[
            ("qm.toml", PATCH_TOML),
            ("inject.lua", ""),
        ]);
        fs::rename(temp.path().join("blocked.zip"), mods.join("blocked.zip")).unwrap();

        let patches = load_patches_new(mods).unwrap();

        assert_eq!(patches.len(), 1);
        assert!(patches[0].2.to_string_lossy().contains("allowed"));
    }

    #[test]
    fn qmignore_excludes_mod() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();

        let ignored = mods.join("ignored");
        fs::create_dir_all(&ignored).unwrap();
        fs::write(ignored.join("qm.toml"), PATCH_TOML).unwrap();
        fs::write(ignored.join("inject.lua"), "").unwrap();
        fs::write(ignored.join(".qmignore"), "").unwrap();

        let patches = load_patches_new(mods).unwrap();
        assert!(patches.is_empty());
    }

    #[test]
    fn missing_source_file_errors() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();

        let m = mods.join("mod");
        fs::create_dir_all(&m).unwrap();
        fs::write(m.join("qm.toml"), PATCH_TOML).unwrap();
        // inject.lua intentionally missing

        let result = load_patches_new(mods);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn qm_subdir_tomls_loaded() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();

        let m = mods.join("mod");
        fs::create_dir_all(m.join("qm")).unwrap();
        fs::write(m.join("qm/patch1.toml"), r#"
[manifest]
version = "1.0.0"

[[patches]]
[patches.copy]
target = "a.lua"
position = "append"
payload = "-- a"
"#).unwrap();
        fs::write(m.join("qm/patch2.toml"), r#"
[manifest]
version = "1.0.0"

[[patches]]
[patches.copy]
target = "b.lua"
position = "append"
payload = "-- b"
"#).unwrap();

        let patches = load_patches_new(mods).unwrap();
        assert_eq!(patches.len(), 2);
    }

    #[test]
    fn process_patches_extracts_targets_and_vars() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();

        let m = mods.join("mod");
        fs::create_dir_all(&m).unwrap();
        fs::write(m.join("qm.toml"), r#"
[manifest]
version = "1.0.0"

[vars]
FOO = "bar"

[[patches]]
[patches.copy]
target = "game.lua"
position = "append"
payload = "-- hi"
"#).unwrap();

        let raw = load_patches_new(mods).unwrap();
        let (patches, targets, vars) = process_patches(raw);

        assert_eq!(patches.len(), 1);
        assert!(targets.contains("game.lua"));
        assert_eq!(vars.get("FOO"), Some(&"bar".to_string()));
    }

    #[test]
    fn get_parent_extracts_dir() {
        assert_eq!(get_parent("a/b/c.txt"), "a/b/");
        assert_eq!(get_parent("file.txt"), "");
        assert_eq!(get_parent(""), "");
    }

    #[test]
    fn classify_asset_relative_path_maps_categories_and_modes() {
        let sfx = classify_asset_relative_path("assets/sounds/garand1.wav").unwrap();
        assert_eq!(sfx.category, "sounds");
        assert_eq!(sfx.stem, "garand1");
        assert_eq!(sfx.ext, "wav");
        assert_eq!(sfx.default_mode, "static");

        let vo = classify_asset_relative_path("assets/sounds/vo/allied/rog1.wav").unwrap();
        assert_eq!(vo.category, "sounds/vo");
        assert_eq!(vo.stem, "rog1");
        assert_eq!(vo.default_mode, "stream");

        let vo_alias = classify_asset_relative_path("assets/vo/axis/ger1.ogg").unwrap();
        assert_eq!(vo_alias.category, "sounds/vo");
        assert_eq!(vo_alias.stem, "ger1");
        assert_eq!(vo_alias.default_mode, "stream");

        let music = classify_asset_relative_path("assets/music/MudAndBloodFull.mp3").unwrap();
        assert_eq!(music.category, "music");
        assert_eq!(music.stem, "MudAndBloodFull");
        assert_eq!(music.default_mode, "stream");

        let sprite = classify_asset_relative_path("assets/sprites/weapons/top_carcano.png").unwrap();
        assert_eq!(sprite.category, "sprites");
        assert_eq!(sprite.stem, "top_carcano");
        assert_eq!(sprite.default_mode, "");

        let ui = classify_asset_relative_path("assets/ui/ambulance.png").unwrap();
        assert_eq!(ui.category, "ui");
        assert_eq!(ui.stem, "ambulance");

        let txt = classify_asset_relative_path("assets/text/weaponText_carcano.txt").unwrap();
        assert_eq!(txt.category, "text");
        assert_eq!(txt.stem, "weaponText_carcano");

        let root_wav = classify_asset_relative_path("assets/garand2.wav").unwrap();
        assert_eq!(root_wav.category, "sounds");
        assert_eq!(root_wav.stem, "garand2");
        assert_eq!(root_wav.default_mode, "static");

        assert!(classify_asset_relative_path("assets/custom/script.lua").is_none());
        assert!(classify_asset_relative_path("assets/.DS_Store").is_none());
        assert!(classify_asset_relative_path("other/garand1.wav").is_none());
    }

    #[test]
    fn asset_only_dir_mod_synthesizes_auto_asset_patch_without_qm_toml() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();

        let sound_mod = mods.join("MySoundSwap");
        fs::create_dir_all(sound_mod.join("assets/sounds")).unwrap();
        fs::create_dir_all(sound_mod.join("assets/music")).unwrap();
        fs::write(sound_mod.join("assets/sounds/garand1.wav"), b"RIFF_GARAND").unwrap();
        fs::write(sound_mod.join("assets/music/MudAndBloodFull.mp3"), b"ID3_MUSIC").unwrap();

        let patches = load_patches_new(mods).unwrap();
        assert_eq!(patches.len(), 1);
        let (patch, prio, _, _) = &patches[0];
        assert_eq!(patch.id(), Some("mysoundswap_auto_assets"));
        assert_eq!(patch.mod_id(), Some("mysoundswap"));
        assert_eq!(patch.primary_target(), "main.lua");
        assert_eq!(*prio, 50);

        if let Patch::Pattern(p) = patch {
            assert_eq!(p.pattern, "setOptionVolume()");
            assert_eq!(p.position, crate::patch::InsertPosition::Before);
            assert!(p.payload.contains("\"assets/music/MudAndBloodFull.mp3\""));
            assert!(p.payload.contains("\"assets/sounds/garand1.wav\""));
        } else {
            panic!("Expected Pattern patch");
        }
    }

    #[test]
    fn asset_only_zip_mod_extracts_and_synthesizes_auto_asset_patch() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();

        make_zip(
            &temp,
            "ZipSoundMod.zip",
            &[("NestedMod/assets/sounds/m60_1.wav", "RIFF_M60")],
        );

        let patches = load_patches_new(mods).unwrap();
        assert_eq!(patches.len(), 1);
        let (patch, _, _, _) = &patches[0];
        assert_eq!(patch.primary_target(), "main.lua");
        if let Patch::Pattern(p) = patch {
            assert!(p.payload.contains("\"assets/sounds/m60_1.wav\""));
        } else {
            panic!("Expected Pattern patch");
        }
        let extracted = mods
            .join("qm")
            .join("zip_assets")
            .join("ZipSoundMod")
            .join("assets/sounds/m60_1.wav");
        assert!(extracted.is_file());
    }
}
