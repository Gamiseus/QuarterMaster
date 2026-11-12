#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use anyhow::{anyhow, bail, Context, Result};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::handshake::{LoadError, LoadOrderPlan};
use crate::lenient::Lenient;
use crate::patch::compat::Override;
use crate::patch::plan::{self, ModMeta, RawLoad};
use crate::patch::{Patch, PatchFile, Priority};
use itertools::Itertools;
use log::*;
use qm_schema::{ModManifest, ParsedVersion};
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

pub const STEAM_SAFETY_PAYLOAD: &str = concat!(
    r#"
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

-- Without Steam the game's first Steam call crashes the process with no message. The game
-- is held on a message box instead, and closes when it is dismissed.
do
  local ok, steam = pcall(require, "luasteam")
  if ok and type(steam) == "table" and type(steam.init) == "function" then
    local init = steam.init
    steam.init = function(...)
      local started = init(...)
      if not started then
        print("[QM] Steam did not start: it is not running, or this account does not own the game. Closing.")
        if love and love.window and love.window.showMessageBox then
          pcall(love.window.showMessageBox, "Mud and Blood cannot start",
            "Steam is not running, or this Steam account does not own the game.\n\n"
            .. "Start Steam, then launch the game again.", "error", true)
        end
        os.exit(1)
      end
      return started
    end
  end
end

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
-- Slot (lowercase "folder/stem") -> the mod or faction whose file the game ends up using.
local __qm_asset_winners = {}

-- The winners as "slot<TAB>source" lines, sorted; "" when no source loaded an asset.
_G.__qm_asset_winner_text = function()
  local lines = {}
  for slot, source in pairs(__qm_asset_winners) do
    lines[#lines + 1] = tostring(slot) .. "\t" .. tostring(source)
  end
  table.sort(lines)
  return table.concat(lines, "\n")
end

local orig_newSource = love and love.audio and love.audio.newSource or nil
local orig_newImage = love and love.graphics and love.graphics.newImage or nil

-- Sprite sizes. The game draws an image at its pixel size. An image listed here is drawn at
-- another size: `w`, `h` is the size it is shown at, `kx`, `ky` that size over its pixels.
-- getWidth/getHeight then answer with the shown size, and love.graphics.draw scales the
-- image to it, so code written for the old pixel size keeps working with a file of any
-- resolution.
local __qm_sprite = setmetatable({}, { __mode = "k" })
local __qm_sprite_rule_set = {}
local __qm_sprite_rule_order = {}
local __qm_img_w, __qm_img_h, __qm_img_dims = nil, nil, nil

local function __qm_is_image(v)
  return type(v) == "userdata" and type(v.typeOf) == "function" and v:typeOf("Image") == true
end

-- Makes Image:getWidth, getHeight and getDimensions answer with the shown size. Done once,
-- with the first image that gets one.
local function __qm_hook_image_type(img)
  if __qm_img_w then return true end
  local mt = getmetatable(img)
  local methods = type(mt) == "table" and (type(mt.__index) == "table" and mt.__index or mt) or nil
  if not methods or type(methods.getWidth) ~= "function" or type(methods.getHeight) ~= "function" then
    return false
  end
  local raw_w, raw_h, raw_dims = methods.getWidth, methods.getHeight, methods.getDimensions
  __qm_img_w, __qm_img_h, __qm_img_dims = raw_w, raw_h, raw_dims
  methods.getWidth = function(self)
    local s = __qm_sprite[self]
    if s then return s.w end
    return raw_w(self)
  end
  methods.getHeight = function(self)
    local s = __qm_sprite[self]
    if s then return s.h end
    return raw_h(self)
  end
  methods.getDimensions = function(self)
    local s = __qm_sprite[self]
    if s then return s.w, s.h end
    return raw_w(self), raw_h(self)
  end
  return true
end

local function __qm_raw_size(img)
  if __qm_img_w then return __qm_img_w(img), __qm_img_h(img) end
  return img:getWidth(), img:getHeight()
end

-- Shows `img` at w x h. Its own pixel size clears the entry.
local function __qm_sprite_set(img, w, h)
  if not __qm_is_image(img) or type(w) ~= "number" or type(h) ~= "number" or w <= 0 or h <= 0 then
    return false
  end
  local rw, rh = __qm_raw_size(img)
  if rw <= 0 or rh <= 0 then return false end
  if w == rw and h == rh then
    __qm_sprite[img] = nil
  else
    if not __qm_hook_image_type(img) then return false end
    __qm_sprite[img] = { w = w, h = h, kx = w / rw, ky = h / rh }
  end
  -- When the game loads the file again, the new image is shown at the same size.
  local info = __qm_obj_info[img]
  if info then
    local shown = __qm_sprite[img]
    for _, key in ipairs({ info.path or false, info.cat_stem or false }) do
      local ov = key and __qm_path_overrides[key]
      if ov then
        ov.shown_w, ov.shown_h = shown and shown.w or nil, shown and shown.h or nil
      end
    end
  end
  return true
end

-- A replacement takes the shown size of the image it replaces, whatever its resolution.
local function __qm_sprite_inherit(old, new, name)
  if not __qm_is_image(old) or not __qm_is_image(new) or old == new then return end
  local ow, oh = old:getWidth(), old:getHeight()
  local nw, nh = __qm_raw_size(new)
  if ow == nw and oh == nh then return end
  if __qm_sprite_set(new, ow, oh) then
    local stretch = (nw / nh) / (ow / oh)
    if stretch > 1.02 or stretch < 0.98 then
      print(string.format("[QM] [WARN] '%s': the replacement is %dx%d and the original %dx%d, another shape. It is stretched to the original's size; set a size for it under [sprites] in qm.toml to choose.", tostring(name), nw, nh, ow, oh))
    end
  end
end
_G.__qm_sprite_inherit = __qm_sprite_inherit

local __qm_raw_draw = love and love.graphics and love.graphics.draw or nil
if __qm_raw_draw then
  local function draw_sized(s, img, x, y, r, sx, sy, ox, oy, kx, ky)
    sx = sx or 1
    sy = sy or sx
    -- The origin is given in shown pixels; LÖVE wants it in the image's own.
    return __qm_raw_draw(img, x or 0, y or 0, r or 0, sx * s.kx, sy * s.ky, (ox or 0) / s.kx, (oy or 0) / s.ky, kx, ky)
  end
  love.graphics.draw = function(a, x, ...)
    local s = __qm_sprite[a]
    -- Only the plain form. A draw with a Quad or a Transform is left as it is.
    if s and type(x) ~= "userdata" then
      return draw_sized(s, a, x, ...)
    end
    return __qm_raw_draw(a, x, ...)
  end
end

_G.QM = _G.QM or {}
_G.QM.Sprites = {
  -- Show an image at an exact size, in the game's pixels.
  setSize = function(img, w, h) return __qm_sprite_set(img, w, h) end,
  -- Show an image at a fraction of its own pixel size: 0.25 shows a 400x400 file at 100x100.
  setScale = function(img, k)
    if not __qm_is_image(img) or type(k) ~= "number" then return false end
    local rw, rh = __qm_raw_size(img)
    return __qm_sprite_set(img, rw * k, rh * k)
  end,
  -- Show an image at the size another image is shown at.
  matchSize = function(img, other)
    if not __qm_is_image(other) then return false end
    return __qm_sprite_set(img, other:getWidth(), other:getHeight())
  end,
  -- Back to its pixel size.
  reset = function(img) __qm_sprite[img] = nil end,
  -- The image's real pixel size.
  rawSize = function(img) return __qm_raw_size(img) end,
  -- Shown size over pixel size, per axis. 1, 1 for an image shown at its pixel size.
  scaleOf = function(img)
    local s = __qm_sprite[img]
    if s then return s.kx, s.ky end
    return 1, 1
  end,
}

-- [sprites] rules from a mod's qm.toml: image global -> { size = {w, h} } | { scale = n }
-- | { like = "otherGlobal" }. A later mod's rule for the same image wins.
_G.__qm_sprite_rules = function(mod_id, rules)
  if type(rules) ~= "table" then return end
  for name, rule in pairs(rules) do
    if type(rule) == "table" then
      rule.mod = mod_id
      if not __qm_sprite_rule_set[name] then __qm_sprite_rule_order[#__qm_sprite_rule_order + 1] = name end
      __qm_sprite_rule_set[name] = rule
    end
  end
end

local function __qm_apply_sprite_rule(name, rule)
  local img = rawget(_G, name)
  if not __qm_is_image(img) then
    print(string.format("[QM] [WARN] [sprites] rule of '%s' for '%s': the game has no image of that name.", tostring(rule.mod), tostring(name)))
    return
  end
  local ok = false
  if type(rule.size) == "table" then
    ok = __qm_sprite_set(img, tonumber(rule.size[1]), tonumber(rule.size[2]))
  elseif rule.scale then
    local rw, rh = __qm_raw_size(img)
    local k = tonumber(rule.scale)
    ok = k ~= nil and __qm_sprite_set(img, rw * k, rh * k)
  elseif rule.like then
    local other = rawget(_G, rule.like)
    ok = __qm_is_image(other) and __qm_sprite_set(img, other:getWidth(), other:getHeight())
  end
  if not ok then
    print(string.format("[QM] [WARN] [sprites] rule of '%s' for '%s' could not be applied.", tostring(rule.mod), tostring(name)))
  end
end

-- Applies every rule. Runs after love.load, when every image is loaded and replaced.
-- Exact sizes and scales first, so a `like` rule sees the final size of the image it copies.
_G.__qm_apply_sprite_rules = function()
  for pass = 1, 2 do
    for _, name in ipairs(__qm_sprite_rule_order) do
      local rule = __qm_sprite_rule_set[name]
      if (rule.like ~= nil) == (pass == 2) then
        __qm_apply_sprite_rule(name, rule)
      end
    end
  end
end

local function __qm_load_ov_fd(ov)
  if not ov then return nil end
  if ov.fd then return ov.fd end
  if ov.full_path and io and io.open and love and love.filesystem and love.filesystem.newFileData then
    local f = io.open(ov.full_path, "rb")
    if not f then return nil end
    local bytes = f:read("*all")
    f:close()
    if not bytes then return nil end
    local ok, fd = pcall(love.filesystem.newFileData, bytes, ov.fname or "asset")
    if ok and fd then return fd end
  end
  return nil
end

if orig_newSource then
  love.audio.newSource = function(a, b, ...)
    if type(a) == "string" then
      local norm = a:gsub("\\", "/"):gsub("^/+", "")
      local cat_stem = (norm:match("^(.+)%.[^%.]+$") or norm):lower()
      local stem = (norm:match("([^/]+)%.[^%.]+$") or norm):lower()
      local ov = __qm_path_overrides[norm:lower()] or __qm_path_overrides[cat_stem]
      local ov_fd = __qm_load_ov_fd(ov)
      if ov and ov_fd then
        local ok, s = pcall(orig_newSource, ov_fd, b or ov.mode or "static", ...)
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
      local ov_fd = __qm_load_ov_fd(ov)
      if ov and ov_fd then
        local ok, img = pcall(orig_newImage, ov_fd, ...)
        if ok and img then
          -- The game loaded this path again: the replacement keeps the size it was shown at.
          if ov.shown_w then __qm_sprite_set(img, ov.shown_w, ov.shown_h) end
          return img
        end
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

_G.__qm_auto_asset = function(mod_dir, rel_path, category, stem, ext, default_mode, source)
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
  local fname = tostring(stem) .. "." .. ext_l
  local ok_fd, fd = pcall(love.filesystem.newFileData, bytes, fname)
  if not ok_fd or not fd then return false end

  local is_audio = (ext_l == "wav" or ext_l == "mp3" or ext_l == "ogg")
  local is_image = (ext_l == "png" or ext_l == "jpg" or ext_l == "jpeg" or ext_l == "bmp")
  local is_font = (ext_l == "ttf" or ext_l == "otf")

  local function is_compat(val)
    if val == nil then return true end
    if is_audio then
      local info = __qm_obj_info[val]
      if info and info.kind == "audio" then return true end
      if type(val) == "userdata" then
        local ok_w, has_w = pcall(function() return val.getWidth ~= nil end)
        if ok_w and has_w then return false end
        return true
      end
      if type(val) == "table" and val.type == "source" then return true end
      return false
    elseif is_image then
      local info = __qm_obj_info[val]
      if info and info.kind == "image" then
        if cat_l ~= "" and cat_l ~= "assets" then
          local info_cat = (info.cat_stem:match("^([^/]+)") or ""):lower()
          if info_cat ~= "" and (info_cat == "sprites" or info_cat == "ui" or info_cat == "cheevos") then
            local main_cat = (cat_l:match("^([^/]+)") or ""):lower()
            if main_cat == "sprites" or main_cat == "ui" or main_cat == "cheevos" then
              if main_cat ~= info_cat then return false end
            end
          end
        end
        return true
      end
      if type(val) == "userdata" then
        local ok_w, has_w = pcall(function() return val.getWidth ~= nil end)
        if ok_w and has_w then return true end
        return false
      end
      if type(val) == "table" and val.type == "image" then return true end
      return false
    elseif is_font then
      if type(val) == "userdata" or (type(val) == "table" and val.type == "font") then return true end
      return false
    end
    return true
  end

  local targets = {}
  local seen = {}
  local function add_target(name)
    if name and not seen[name] then
      local current_val = rawget(_G, name)
      if is_compat(current_val) then
        seen[name] = true
        table.insert(targets, name)
      end
    end
  end

  -- 1. Exact category + stem lookup (e.g. "sounds/urblogo" -> "urbLogoSound")
  local by_cs = __qm_by_cat_stem[cat_l .. "/" .. stem_l]
  if by_cs then
    for _, vname in ipairs(by_cs) do add_target(vname) end
  end

  -- 2. Audio suffix variants if no target found yet (e.g. stem .. "Sound", stem .. "Music")
  if #targets == 0 and is_audio then
    if rawget(_G, stem .. "Sound") ~= nil then add_target(stem .. "Sound")
    elseif rawget(_G, stem .. "sound") ~= nil then add_target(stem .. "sound")
    elseif rawget(_G, stem .. "Music") ~= nil then add_target(stem .. "Music")
    elseif rawget(_G, stem .. "music") ~= nil then add_target(stem .. "music")
    end
  end

  -- 3. Stem index lookup across all categories if compatible
  if #targets == 0 then
    local by_s = __qm_by_stem[stem_l]
    if by_s then
      for _, vname in ipairs(by_s) do add_target(vname) end
    end
  end

  -- 4. Direct global stem or case-insensitive matching if compatible
  if #targets == 0 then
    local candidate = nil
    if rawget(_G, stem) ~= nil or __qm_pool_key_map[stem] ~= nil then
      candidate = stem
    elseif __qm_var_lower[stem_l] then
      candidate = __qm_var_lower[stem_l]
    end
    if candidate then
      local old = rawget(_G, candidate)
      local old_info = old and __qm_obj_info[old]
      -- If old was loaded by vanilla from a known asset whose stem does NOT match this stem,
      -- then candidate was created by a DIFFERENT asset (e.g. supplyCrate was loaded from lootCrate.png).
      -- Loose stem matching must not clobber it!
      if not old_info or old_info.stem == stem_l then
        add_target(candidate)
      end
    end
  end

  -- 5. Fallback for brand-new assets not previously registered by vanilla
  if #targets == 0 then
    if rawget(_G, stem) == nil then
      seen[stem] = true
      table.insert(targets, stem)
    elseif is_audio and rawget(_G, stem .. "Sound") == nil then
      local s_name = stem .. "Sound"
      seen[s_name] = true
      table.insert(targets, s_name)
    end
  end

  __qm_path_overrides[cat_l .. "/" .. stem_l .. "." .. ext_l] = { full_path = full_path, fname = fname, mode = default_mode }
  __qm_path_overrides[cat_l .. "/" .. stem_l] = { full_path = full_path, fname = fname, mode = default_mode }

  if ext_l == "wav" or ext_l == "mp3" or ext_l == "ogg" then
    if not orig_newSource then return false end
    for _, var_name in ipairs(targets) do
      local old = rawget(_G, var_name)
      if old == nil and __qm_pool_key_map[var_name] then
        local pfx, idx_s = var_name:match("^(.-[^%d])(%d+)$")
        if pfx and idx_s and __qm_pools_by_prefix[pfx] then
          old = __qm_pools_by_prefix[pfx][tonumber(idx_s)]
        end
      end
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
          __qm_path_overrides[old_info.path] = { full_path = full_path, fname = fname, mode = mode }
          __qm_path_overrides[old_info.cat_stem] = { full_path = full_path, fname = fname, mode = mode }
        end
        rawset(_G, var_name, src)
        __qm_var_lower[var_name:lower()] = var_name
        if source then
          __qm_asset_winners[(old_info and old_info.cat_stem) or (cat_l .. "/" .. stem_l)] = source
        end
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
        __qm_path_overrides[old_info.path] = { full_path = full_path, fname = fname }
        __qm_path_overrides[old_info.cat_stem] = { full_path = full_path, fname = fname }
      end
      -- A file of another resolution is shown at the size of the image it replaces.
      __qm_sprite_inherit(old, img, var_name)
      rawset(_G, var_name, img)
      __qm_var_lower[var_name:lower()] = var_name
      if source then
        __qm_asset_winners[(old_info and old_info.cat_stem) or (cat_l .. "/" .. stem_l)] = source
      end
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
    if source then __qm_asset_winners[cat_l .. "/" .. stem_l] = source end
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

-- `source` is the mod's id (absent in a patch an older launcher built).
_G.__qm_auto_asset_manifest = function(mod_dir, manifest, source)
  if type(manifest) ~= "string" then return false end
  __qm_build_vanilla_index()

  local entries = {}
  local by_key = {}
  for line in manifest:gmatch("[^\r\n]+") do
    local rel_path, category, stem, ext, mode =
      line:match('^%s*"([^"]+)"\t"([^"]*)"\t"([^"]+)"\t"([^"]+)"\t"([^"]*)"%s*$')
    if rel_path and stem and ext then
      local cat_l = category:lower()
      local stem_l = stem:lower()
      local ext_l = ext:lower()
      local key = cat_l .. "/" .. stem_l
      local entry = {
        rel_path = rel_path,
        category = category,
        stem = stem,
        ext = ext,
        mode = mode,
        cat_l = cat_l,
        stem_l = stem_l,
        ext_l = ext_l,
        key = key,
      }
      table.insert(entries, entry)
      by_key[key] = by_key[key] or {}
      table.insert(by_key[key], entry)
    end
  end

  local skip_entry = {}
  for key, group in pairs(by_key) do
    if #group > 1 then
      local vanilla_path = nil
      local target_var = rawget(_G, group[1].stem) and group[1].stem or __qm_var_lower[group[1].stem_l]
      if not target_var and __qm_by_cat_stem[key] then
        target_var = __qm_by_cat_stem[key][1]
      end
      if target_var then
        local old_obj = rawget(_G, target_var)
        local old_info = old_obj and __qm_obj_info[old_obj] or nil
        if old_info and old_info.path then
          vanilla_path = old_info.path
        end
      end

      local best_idx = 1
      local best_score = -1
      for idx, e in ipairs(group) do
        local score = 0
        local candidate_path = e.cat_l .. "/" .. e.stem_l .. "." .. e.ext_l
        if vanilla_path and candidate_path == vanilla_path then
          score = 100
        elseif e.cat_l == "music" then
          if e.ext_l == "mp3" then score = 30
          elseif e.ext_l == "ogg" then score = 20
          elseif e.ext_l == "wav" then score = 10 end
        else
          if e.ext_l == "wav" then score = 30
          elseif e.ext_l == "ogg" then score = 20
          elseif e.ext_l == "mp3" then score = 10 end
        end
        if score > best_score then
          best_score = score
          best_idx = idx
        end
      end

      for idx, e in ipairs(group) do
        if idx ~= best_idx then
          skip_entry[e] = true
        end
      end
    end
  end

  local loaded_count = 0
  for _, e in ipairs(entries) do
    if not skip_entry[e] then
      _G.__qm_auto_asset(mod_dir, e.rel_path, e.category, e.stem, e.ext, e.mode, source)
      loaded_count = loaded_count + 1
      if loaded_count % 100 == 0 and collectgarbage then
        pcall(collectgarbage, "step", 200)
      end
    end
  end

  if _G.__qm_finalize_auto_assets then
    _G.__qm_finalize_auto_assets()
  end
  if loaded_count >= 100 and collectgarbage then
    pcall(collectgarbage, "collect")
  end
  return true
end

-- A global that returns a random member of `list` each time it is read (faction sound pools).
_G.__qm_register_pool = function(name, list)
  if type(name) ~= "string" or type(list) ~= "table" or #list == 0 then return false end
  __qm_pool_key_map[name] = list
  rawset(_G, name, nil)
  return true
end

-- For the faction runtime, called before it sets the file global `name` to `new_value`: the
-- game file `name` was loaded from now resolves to `full_path`, so a later load of the
-- game's own path (a map reload) gives the faction's file too, and the faction is noted as
-- the winner of that file. `fallback_slot` names the slot of a global the game does not have.
_G.__qm_asset_override = function(name, new_value, full_path, source, fallback_slot)
  local old = rawget(_G, name)
  if old == nil and __qm_pool_key_map[name] then old = __qm_pool_key_map[name][1] end
  local info = old and __qm_obj_info[old] or nil
  if info then
    local fname = tostring(full_path):match("([^/]+)$") or "asset"
    local ov = { full_path = full_path, fname = fname, mode = info.mode }
    if info.path then __qm_path_overrides[info.path] = ov end
    if info.cat_stem then __qm_path_overrides[info.cat_stem] = ov end
    if new_value ~= nil then __qm_obj_info[new_value] = info end
  end
  __qm_sprite_inherit(old, new_value, name)
  local slot = (info and info.cat_stem) or fallback_slot
  if source and slot then __qm_asset_winners[slot] = source end
  return slot
end

-- What the launcher passes for the save check below, as Lua source in qm.save_info: the
-- save stamp, and the weapons mods declare in mod.json.
local __qm_save_info
local function __qm_get_save_info()
  if __qm_save_info == nil then
    __qm_save_info = {}
    local ok, qm_mod = pcall(require, "qm")
    local src = ok and type(qm_mod) == "table" and qm_mod.save_info
    if type(src) == "string" and src ~= "" then
      local fn = (loadstring or load)(src, "=qm.save_info")
      local ok_run, info = pcall(fn)
      if ok_run and type(info) == "table" then __qm_save_info = info end
    end
  end
  return __qm_save_info
end

-- Faction save safety. A save remembers which faction ran on each side (player.qmFaction) and
-- the vanilla class unlocks it replaced. When that faction is no longer enabled, put the
-- vanilla unlocks back and turn weapons the game no longer knows into their base weapon, so the
-- save loads instead of crashing on a missing sprite. Runs right after updateProfile.
-- The same is done for the weapons a mod declares in mod.json (save_content) while that mod
-- is not loaded: each becomes the base weapon the mod names for it.
local __qm_vanilla_weapon_tags = {}
for _, t in ipairs({"m1", "g98", "k98", "springfield", "boysRifle", "pz39", "g33", "mas36", "m1Carbine", "matchGarand", "m1941", "t20", "t22", "scopedK98", "1917", "scopedGarand", "scopedSpringfield", "scopedEnfield", "g43", "scopedG43", "svt40", "vg15", "m1CarbinePara", "m2Carbine", "m2CarbineA1", "bren", "chauchat", "zb26", "m1919", "vicker", "mg42", "mg15", "mg34", "tommy", "stenMKVI", "sten", "bar", "johnson", "m3", "m38", "mp40", "mp41", "ppsh41", "stg44", "irstg44", "fg42", "shotgun", "slamShotgun"}) do
  __qm_vanilla_weapon_tags[t] = true
end

local function __qm_deep_copy(v, seen)
  if type(v) ~= "table" then return v end
  seen = seen or {}
  if seen[v] then return seen[v] end
  local out = {}
  seen[v] = out
  for k, x in pairs(v) do out[__qm_deep_copy(k, seen)] = __qm_deep_copy(x, seen) end
  return out
end

_G.__qm_faction_save_cleanup = function(p)
  if type(p) ~= "table" then return end
  local expected = rawget(_G, "__qm_factions_expected") or {}
  local saved = p.qmFaction
  if type(saved) == "table" then
    for side, id in pairs(saved) do
      if expected[side] ~= id then
        local backups = type(p.qmFactionVanilla) == "table" and p.qmFactionVanilla or {}
        local backup = backups[side]
        -- Only allied factions change class unlocks (their classes); restoring an axis
        -- snapshot would undo unlocks the player earned since.
        if side == "allied" and type(backup) == "table" and type(backup.unlock) == "table" then
          p.unlock = __qm_deep_copy(backup.unlock)
        end
        saved[side] = nil
        backups[side] = nil
        print("[QM][Faction] This save used faction '" .. tostring(id) .. "', which is not enabled" .. (side == "allied" and "; vanilla allied classes restored." or "."))
      end
    end
    if next(saved) == nil then
      p.qmFaction = nil
      p.qmFactionVanilla = nil
    end
  end

  local bases = p.qmFactionWeaponBases
  local info = __qm_get_save_info()
  -- Weapon tag -> base weapon, declared by mods that are not loaded.
  local declared = type(info.weapons) == "table" and info.weapons or {}
  -- Declared weapon tags of the mods that are loaded.
  local loaded = type(info.loaded_weapons) == "table" and info.loaded_weapons or {}
  local had_faction = type(bases) == "table"
  if not had_faction and next(declared) == nil then return end
  if not had_faction then bases = {} end
  local extra = rawget(_G, "__qm_factions_expected_weapons") or {}
  -- A weapon the running game builds under its own tag belongs to a mod that is loaded, also
  -- when that mod declares nothing: the game's own builder turns a tag it does not know
  -- into a K98.
  local built = {}
  local function builds(t)
    if built[t] == nil then
      local probe = {}
      built[t] = type(weaponCreator) == "function" and pcall(weaponCreator, probe, t)
        and probe.tag == t and probe.maxAmmo ~= nil or false
    end
    return built[t]
  end
  local function known(t)
    return __qm_vanilla_weapon_tags[t] or extra[t] or loaded[t] or builds(t)
  end
  -- A save that used a faction has every weapon the running game does not know replaced;
  -- any other save only the weapons a mod that is off declared.
  local function gone(t) return not known(t) and (had_faction or declared[t] ~= nil) end
  local function base_of(t)
    local b = bases[t] or declared[t]
    if b and __qm_vanilla_weapon_tags[b] then return b end
    return "k98"
  end
  local fixed = 0
  local seen = {}
  local function walk(t, depth)
    if depth > 12 or seen[t] then return end
    seen[t] = true
    if type(t.tag) == "string" and t.maxAmmo ~= nil and t.magSprite ~= nil and gone(t.tag) then
      if type(weaponCreator) == "function" and pcall(weaponCreator, t, base_of(t.tag)) then
        fixed = fixed + 1
      end
    end
    for _, key in ipairs({"primarySwap", "preScroungeGun"}) do
      local v = t[key]
      if type(v) == "string" and v ~= "" and not known(v) and (bases[v] or declared[v]) then t[key] = base_of(v) end
    end
    for _, v in pairs(t) do
      if type(v) == "table" then walk(v, depth + 1) end
    end
  end
  walk(p, 0)
  if type(p.unlock) == "table" then
    for _, list in pairs(p.unlock) do
      if type(list) == "table" then
        for i, tag in pairs(list) do
          if type(tag) == "string" and gone(tag) then list[i] = base_of(tag) end
        end
      end
    end
  end
  if fixed > 0 then
    print("[QM] " .. fixed .. " weapon(s) from a faction or mod that is not enabled were changed to their base weapon.")
  end
  if had_faction and p.qmFaction == nil and next(extra) == nil then
    p.qmFactionWeaponBases = nil
  end
end

-- The save stamp. Each save that loads is marked with the profile and mods it was loaded
-- with, and with the save fields those mods write, so the launcher can tell later which
-- values a removed mod left behind. The game writes the whole player table, so the stamp is
-- saved with it.
_G.__qm_stamp_save = function(p)
  if type(p) ~= "table" then return end
  local stamp = __qm_get_save_info().stamp
  if type(stamp) ~= "table" then return end
  local meta = __qm_deep_copy(stamp)
  if type(meta.fields) ~= "table" then meta.fields = {} end
  -- A mod that is not loaded now keeps the fields an earlier stamp listed for it, while
  -- the save still has them.
  local old = rawget(p, "_qm_meta")
  if type(old) == "table" and type(old.fields) == "table" then
    for mod_id, list in pairs(old.fields) do
      if meta.fields[mod_id] == nil and type(list) == "table" then
        local keep = {}
        for _, field in ipairs(list) do
          if type(field) == "string" and rawget(p, field) ~= nil then keep[#keep + 1] = field end
        end
        if #keep > 0 then meta.fields[mod_id] = keep end
      end
    end
  end
  meta.timestamp = os.date("!%Y-%m-%dT%H:%M:%SZ")
  p._qm_meta = meta
end

-- QuarterMaster's own wrappers of game functions, by global name. `make(orig)` returns the
-- wrapper's body. They are kept so the end of love.load can put them back over a mod that
-- replaced the function after it was wrapped (__qm_reassert_wrappers, called by the code at
-- the end of main.lua). A wrapper that was put back steps aside: when the mod's function
-- calls the wrapper it saw, the call goes straight on, so QuarterMaster's part runs once.
local __qm_wrappers = {}
_G.__qm_wrap = function(name, make, current)
  local orig = current
  if orig == nil then orig = rawget(_G, name) end
  local state = { retired = false }
  local body = make(orig)
  local wrapper = function(...)
    if state.retired then
      if type(orig) == "function" then return orig(...) end
      return
    end
    return body(...)
  end
  local e = __qm_wrappers[name]
  if not e then
    e = { makes = {}, states = {} }
    __qm_wrappers[name] = e
  end
  e.makes[#e.makes + 1] = make
  e.states[#e.states + 1] = state
  e.wrapper = wrapper
  rawset(_G, name, wrapper)
  return wrapper
end

_G.__qm_reassert_wrappers = function()
  local names = {}
  for name, e in pairs(__qm_wrappers) do
    local now = rawget(_G, name)
    if now ~= e.wrapper and type(now) == "function" then names[#names + 1] = name end
  end
  table.sort(names)
  for _, name in ipairs(names) do
    local e = __qm_wrappers[name]
    for _, s in ipairs(e.states) do s.retired = true end
    __qm_wrappers[name] = nil
    for _, make in ipairs(e.makes) do _G.__qm_wrap(name, make) end
    print("[QM] " .. name .. " was replaced after QuarterMaster wrapped it; QuarterMaster's part was put back around the new one.")
  end
  return names
end

local function __qm_wrap_update_profile(orig)
  return function(p, ...)
    local result = orig(p, ...)
    local save = p or rawget(_G, "player")
    local ok, err = pcall(_G.__qm_faction_save_cleanup, save)
    if not ok then print("[QM][Faction] ERROR in save check: " .. tostring(err)) end
    ok, err = pcall(_G.__qm_stamp_save, save)
    if not ok then print("[QM] ERROR in save stamp: " .. tostring(err)) end
    return result
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
    elseif k == "updateProfile" and type(v) == "function" then
      _G.__qm_wrap(k, __qm_wrap_update_profile, v)
    else
      if type(prev_newindex) == "function" then
        prev_newindex(t, k, v)
      else
        rawset(t, k, v)
      end
    end
  end
})

-- Diagnostic test run only (the launcher's "Trace globals in a test run"). The launcher
-- passes the names to watch as Lua source in qm.diagnostics; a normal launch passes none and
-- nothing below runs. While the game loads, a line hook notes every change of a watched
-- global with the file and line that made it. After love.load each watched function is
-- wrapped once to count the places that call it. The code at the end of main.lua reports
-- both through qm.record_diagnostics.
do
  local ok_qm, qm_mod = pcall(require, "qm")
  local src = ok_qm and type(qm_mod) == "table" and qm_mod.diagnostics
  local cfg = nil
  if type(src) == "string" and src ~= "" then
    local ok_cfg, t = pcall((loadstring or load)(src, "=qm.diagnostics"))
    if ok_cfg and type(t) == "table" then cfg = t end
  end
  if cfg and type(debug) == "table" and type(debug.sethook) == "function" then
    local names = type(cfg.globals) == "table" and cfg.globals or {}
    local hooks = type(cfg.hooks) == "table" and cfg.hooks or {}
    local getinfo = debug.getinfo
    local NONE = {}
    local MAX_CHANGES, MAX_CALLERS = 2000, 200
    local last, changes = {}, {}
    local function clean(s) return (tostring(s):gsub("[\t\r\n]", " ")) end
    for _, n in ipairs(names) do
      local v = rawget(_G, n)
      last[n] = v == nil and NONE or v
    end
    local function check(source, line)
      for i = 1, #names do
        local n = names[i]
        local v = rawget(_G, n)
        local now = v == nil and NONE or v
        -- `now == now` leaves out NaN, which never equals itself.
        if now ~= last[n] and now == now then
          last[n] = now
          if #changes < MAX_CHANGES then
            changes[#changes + 1] = table.concat({
              "G", clean(n), type(v), clean(v), clean(source), tostring(line),
            }, "\t")
          end
        end
      end
    end
    -- A change is seen when the next line starts, so it belongs to the line before.
    local prev_source, prev_line = "?", 0
    local hooked = false
    -- A global set for the first time goes through the startup code's own __newindex; the
    -- line that counts is the one that made the assignment, so those lines are passed over.
    local own_mt = getmetatable(_G) or {}
    local own_newindex, own_index = own_mt.__newindex, own_mt.__index
    local function on_line(_, line)
      local info = getinfo(2, "Sf")
      if info and (info.func == own_newindex or info.func == own_index) then return end
      -- Protected: an error here would surface in whatever game code is running.
      pcall(check, prev_source, prev_line)
      prev_source, prev_line = info and info.source or "?", line or 0
    end
    local jit_was_on = false
    if type(jit) == "table" and type(jit.off) == "function" then
      jit_was_on = type(jit.status) ~= "function" or jit.status() == true
      pcall(jit.off)
    end
    if pcall(debug.sethook, on_line, "l") then hooked = true end

    local calls, dirty = {}, false
    local diag = {}
    -- Wraps each watched function once; a name can be `fn` or `Table.fn`.
    function diag.wrap_hooks()
      for _, name in ipairs(hooks) do
        if not calls[name] then
          local owner, key = _G, name
          local t, k = tostring(name):match("^([%w_]+)%.([%w_]+)$")
          if t then owner, key = rawget(_G, t), k end
          local fn = type(owner) == "table" and rawget(owner, key) or nil
          if type(fn) == "function" then
            local c = { total = 0, by = {}, order = {} }
            calls[name] = c
            rawset(owner, key, function(...)
              c.total = c.total + 1
              dirty = true
              local info = getinfo(2, "Sl")
              local at = (info and info.source or "?") .. "\t" .. tostring(info and info.currentline or 0)
              if c.by[at] then
                c.by[at] = c.by[at] + 1
              elseif #c.order < MAX_CALLERS then
                c.by[at] = 1
                c.order[#c.order + 1] = at
              end
              return fn(...)
            end)
          end
        end
      end
    end
    -- The last look at the watched globals; the line hook comes off.
    function diag.stop()
      if not hooked then return end
      hooked = false
      pcall(debug.sethook)
      pcall(check, prev_source, prev_line)
      if jit_was_on and type(jit.on) == "function" then pcall(jit.on) end
    end
    function diag.checkpoint()
      if hooked then pcall(check, prev_source, prev_line) end
    end
    -- True once since the last report when a watched function was called.
    function diag.take_dirty()
      local d = dirty
      dirty = false
      return d
    end
    function diag.text()
      local out = {}
      for i = 1, #changes do out[#out + 1] = changes[i] end
      for _, name in ipairs(hooks) do
        local c = calls[name]
        if c then
          out[#out + 1] = "H\t" .. clean(name) .. "\t" .. c.total
          for _, at in ipairs(c.order) do
            out[#out + 1] = "C\t" .. clean(name) .. "\t" .. at .. "\t" .. c.by[at]
          end
        else
          out[#out + 1] = "H\t" .. clean(name) .. "\t-1"
        end
      end
      return table.concat(out, "\n")
    end
    _G.__qm_diag = diag
    print("[QM] Diagnostic run: watching " .. #names .. " global(s) and " .. #hooks .. " function(s).")
  end
end
"#,
    include_str!("save_guard.lua")
);

/// Compare two file paths by their lowercase filenames (a path with no file name sorts as "")
fn filename_cmp(first: &Path, second: &Path) -> Ordering {
    let lower = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    lower(first).cmp(&lower(second))
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
    let [first, rest @ ..] = parts.as_slice() else {
        return None;
    };
    if rest.is_empty() || !first.eq_ignore_ascii_case("assets") {
        return None;
    }
    if parts.iter().any(|seg| seg.starts_with('.')) {
        return None;
    }

    let file_name = *parts.last()?;
    let (stem, ext) = file_name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    let stem = stem.to_string();
    let ext = ext.to_ascii_lowercase();

    let is_audio = matches!(ext.as_str(), "wav" | "mp3" | "ogg");
    let is_image = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp");
    let is_font = matches!(ext.as_str(), "ttf" | "otf");
    let is_text = ext == "txt";

    if !is_audio && !is_image && !is_font && !is_text {
        return None;
    }

    // `rest` ends with the file name; the folders between `assets/` and it.
    let subdirs: Vec<String> = rest
        .split_last()
        .map(|(_, dirs)| dirs)
        .unwrap_or_default()
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

    let canonical_rel = format!("assets/{}", rest.join("/"));
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

/// What the vanilla game loads, read from its Lua sources, so an asset can be told apart as an
/// override or an addition without running the game. It mirrors the lookups in
/// `__qm_auto_asset` (category and stem, audio `Sound`/`Music` globals, stem, global name);
/// files the game builds paths for at runtime are not seen, so those show as additions.
#[derive(Debug, Clone, Default)]
pub struct VanillaAssetIndex {
    /// Lowercase `dir/stem` of every asset path literal (`"sounds/garand1.wav"` -> `sounds/garand1`).
    cat_stems: HashSet<String>,
    /// Lowercase file stems of those paths.
    stems: HashSet<String>,
    /// Lowercase names assigned at the start of a line (`weaponText_carcano = ...`).
    globals: HashSet<String>,
}

impl VanillaAssetIndex {
    pub fn from_lua_sources<'a>(sources: impl IntoIterator<Item = &'a str>) -> Self {
        let mut idx = Self::default();
        // Both patterns are constant and valid; a failure only leaves the index empty.
        let (Ok(path_re), Ok(global_re)) = (
            regex_lite::Regex::new(
                r#"["']([^"'\r\n]+)\.(?i:wav|mp3|ogg|png|jpg|jpeg|bmp|ttf|otf)["']"#,
            ),
            regex_lite::Regex::new(r"(?m)^[ \t]*([A-Za-z_][A-Za-z0-9_]*)[ \t]*=[^=]"),
        ) else {
            return idx;
        };
        for src in sources {
            for c in path_re.captures_iter(src) {
                let Some(m) = c.get(1) else { continue };
                let cat_stem = m.as_str().replace('\\', "/").trim_start_matches('/').to_ascii_lowercase();
                if let Some(stem) = cat_stem.rsplit('/').next() {
                    idx.stems.insert(stem.to_string());
                }
                idx.cat_stems.insert(cat_stem);
            }
            for c in global_re.captures_iter(src) {
                if let Some(m) = c.get(1) {
                    idx.globals.insert(m.as_str().to_ascii_lowercase());
                }
            }
        }
        idx
    }

    /// The index from the game archive's file names plus the Lua sources (D79). The archive
    /// lists every asset file, also the ones whose paths the game builds at runtime; the
    /// sources still give the global names.
    pub fn from_archive_paths<'a, 'b>(
        paths: impl IntoIterator<Item = &'a str>,
        sources: impl IntoIterator<Item = &'b str>,
    ) -> Self {
        let mut idx = Self::from_lua_sources(sources);
        for path in paths {
            let p = path.replace('\\', "/");
            let p = p.trim_start_matches('/');
            let Some((cat_stem, ext)) = p.rsplit_once('.') else {
                continue;
            };
            if cat_stem.is_empty() || ext.contains('/') {
                continue;
            }
            let cat_stem = cat_stem.to_ascii_lowercase();
            if let Some(stem) = cat_stem.rsplit('/').next() {
                idx.stems.insert(stem.to_string());
            }
            idx.cat_stems.insert(cat_stem);
        }
        idx
    }

    /// True when the autoloader would replace something the game already has.
    pub fn overrides(&self, asset: &DiscoveredAsset) -> bool {
        let stem = asset.stem.to_ascii_lowercase();
        if asset.ext == "txt" {
            return self.globals.contains(&stem);
        }
        let is_audio = matches!(asset.ext.as_str(), "wav" | "mp3" | "ogg");
        self.cat_stems.contains(&format!("{}/{stem}", asset.category.to_ascii_lowercase()))
            || (is_audio
                && (self.globals.contains(&format!("{stem}sound"))
                    || self.globals.contains(&format!("{stem}music"))))
            || self.stems.contains(&stem)
            || self.globals.contains(&stem)
    }
}

/// Whether `name` can be an image global: a Lua identifier.
fn is_lua_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The Lua table of a mod's `[sprites]` rules and one warning per rule left out. A rule
/// needs exactly one of `size`, `scale` and `like`, with positive numbers.
pub fn sprite_rules_lua(
    rules: &std::collections::BTreeMap<String, crate::patch::SpriteRule>,
) -> (String, Vec<String>) {
    let mut entries: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for (name, rule) in rules {
        if !is_lua_name(name) {
            warnings.push(format!("[sprites] '{name}' is not the name of an image global; the rule is ignored"));
            continue;
        }
        let set = usize::from(rule.size.is_some()) + usize::from(rule.scale.is_some()) + usize::from(rule.like.is_some());
        if set != 1 {
            warnings.push(format!("[sprites.{name}] needs exactly one of size, scale and like; the rule is ignored"));
            continue;
        }
        let good = |n: f64| n.is_finite() && n > 0.0;
        let body = match (&rule.size, rule.scale, &rule.like) {
            (Some([w, h]), _, _) if good(*w) && good(*h) => format!("size = {{ {w}, {h} }}"),
            (_, Some(k), _) if good(k) => format!("scale = {k}"),
            (_, _, Some(other)) if is_lua_name(other) && other != name => format!("like = \"{other}\""),
            _ => {
                warnings.push(format!("[sprites.{name}] has a value that cannot be used (sizes and scales are above 0, like names another image); the rule is ignored"));
                continue;
            }
        };
        entries.push(format!("  {name} = {{ {body} }},"));
    }
    (format!("{{\n{}\n}}", entries.join("\n")), warnings)
}

/// Synthesizes the patch that hands a mod's `[sprites]` rules to the game, at the place the
/// asset patches run. The rules are applied after `love.load`, when every image exists.
/// None when no rule is usable.
pub fn build_sprite_rules_patch(
    mod_id: &str,
    rules: &std::collections::BTreeMap<String, crate::patch::SpriteRule>,
    ordinal: usize,
) -> (Option<Patch>, Vec<String>) {
    let (table, warnings) = sprite_rules_lua(rules);
    let count = table.lines().count().saturating_sub(2);
    if count == 0 {
        return (None, warnings);
    }
    let escaped_id = mod_id.replace('\\', "\\\\").replace('"', "\\\"");
    let payload = format!(
        "-- QuarterMaster sprite sizes ({mod_id}: {count})\nif _G.__qm_sprite_rules then\n  _G.__qm_sprite_rules(\"{escaped_id}\", {table})\nend"
    );
    let id = if ordinal == 0 {
        format!("{mod_id}_sprite_sizes")
    } else {
        format!("{mod_id}_sprite_sizes_{}", ordinal + 1)
    };
    let patch = Patch::Pattern(crate::patch::PatternPatch {
        pattern: "setOptionVolume()".to_string(),
        position: crate::patch::InsertPosition::Before,
        target: crate::patch::Target::Single("main.lua".to_string()),
        payload,
        match_indent: true,
        times: Some(1),
        overwrite: false,
        id: Some(id),
        name: Some(format!("Sprite Sizes ({count})")),
        description: Some(format!("Sets the size {count} image(s) are shown at, from [sprites] in qm.toml.")),
        requires: Vec::new(),
        priority: None,
        before: Vec::new(),
        after: Vec::new(),
        on_failure: crate::patch::OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
    });
    (Some(patch), warnings)
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
    let dir = asset_root_dir.to_string_lossy().replace('\\', "/");
    let (payload, name, description) = auto_asset_texts(mod_id, &dir, assets);

    Some(Patch::Pattern(crate::patch::PatternPatch {
        pattern: "setOptionVolume()".to_string(),
        position: crate::patch::InsertPosition::Before,
        target: crate::patch::Target::Single("main.lua".to_string()),
        payload,
        match_indent: true,
        times: Some(1),
        overwrite: false,
        id: Some(format!("{mod_id}_auto_assets")),
        name: Some(name),
        description: Some(description),
        requires: Vec::new(),
        priority: Some(priority),
        before: Vec::new(),
        after: Vec::new(),
        on_failure: crate::patch::OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id: Some(mod_id.to_string()),
        disabled: false,
        qm: Default::default(),
    }))
}

/// The payload, name and description of an auto-asset patch for `assets` under `dir` (a path
/// with `/` separators). The manifest call names the mod, so the game can report whose file
/// it ended up using (Phase E D77).
fn auto_asset_texts(mod_id: &str, dir: &str, assets: &[DiscoveredAsset]) -> (String, String, String) {
    let escaped_dir = dir.replace('"', "\\\"");
    let escaped_id = mod_id.replace(['\\', '"', '\r', '\n'], "");

    let mut manifest_body = String::new();
    for a in assets {
        let rel_esc = a.rel_path.replace('\\', "/").replace(['\t', '\r', '\n', '"'], "");
        let cat_esc = a.category.replace(['\t', '\r', '\n', '"'], "");
        let stem_esc = a.stem.replace(['\t', '\r', '\n', '"'], "");
        let ext_esc = a.ext.replace(['\t', '\r', '\n', '"'], "");
        let mode_esc = a.default_mode.replace(['\t', '\r', '\n', '"'], "");
        manifest_body.push_str(&format!(
            "\"{rel_esc}\"\t\"{cat_esc}\"\t\"{stem_esc}\"\t\"{ext_esc}\"\t\"{mode_esc}\"\n"
        ));
    }
    let mut eq_len = 3usize;
    while manifest_body.contains(&format!("]{}]", "=".repeat(eq_len))) {
        eq_len += 1;
    }
    let eqs = "=".repeat(eq_len);
    let payload = format!(
        "-- QuarterMaster Auto-Asset Patch ({mod_id}: {} files)\nif _G.__qm_auto_asset_manifest then\n  _G.__qm_auto_asset_manifest(\"{escaped_dir}\", [{eqs}[\n{manifest_body}]{eqs}], \"{escaped_id}\")\nend",
        assets.len()
    );
    (
        payload,
        format!("Auto-Load Assets ({} files)", assets.len()),
        format!(
            "Automatically loads and replaces {} asset file(s) from assets/.",
            assets.len()
        ),
    )
}

/// `patch` without the asset files in `excluded` (`assets/...` paths, compared without
/// case): the plan's `asset_exclusions` for its mod (Phase E D78). The patch itself when it
/// is not an auto-asset patch or nothing of it is excluded; None when every file is.
pub fn without_excluded_assets(patch: &Patch, excluded: &[String]) -> Option<Patch> {
    if excluded.is_empty() {
        return Some(patch.clone());
    }
    let Some(manifest) = auto_asset_manifest(patch) else {
        return Some(patch.clone());
    };
    let norm = |p: &str| p.replace('\\', "/").to_ascii_lowercase();
    let gone: HashSet<String> = excluded.iter().map(|p| norm(p)).collect();
    let kept: Vec<DiscoveredAsset> = manifest
        .assets
        .iter()
        .filter(|a| !gone.contains(&norm(&a.rel_path)))
        .cloned()
        .collect();
    if kept.len() == manifest.assets.len() {
        return Some(patch.clone());
    }
    if kept.is_empty() {
        return None;
    }
    let mut out = patch.clone();
    if let Patch::Pattern(p) = &mut out {
        let (payload, name, description) = auto_asset_texts(&manifest.mod_id, &manifest.dir, &kept);
        p.payload = payload;
        p.name = Some(name);
        p.description = Some(description);
    }
    Some(out)
}

/// The mod, asset folder and files of an auto-asset patch, read back from the patch
/// `build_auto_asset_patch` made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoAssetManifest {
    pub mod_id: String,
    /// The folder the `assets/...` paths are under, with `/` separators.
    pub dir: String,
    pub assets: Vec<DiscoveredAsset>,
}

/// The first line of every auto-asset payload, up to the mod id.
const AUTO_ASSET_HEADER: &str = "-- QuarterMaster Auto-Asset Patch (";

/// Reads an auto-asset patch back into its manifest; None for any other patch.
pub fn auto_asset_manifest(patch: &Patch) -> Option<AutoAssetManifest> {
    let Patch::Pattern(p) = patch else {
        return None;
    };
    if !p.id.as_deref().is_some_and(|id| id.ends_with("_auto_assets"))
        || !p.payload.starts_with(AUTO_ASSET_HEADER)
    {
        return None;
    }
    let mod_id = p.mod_id.clone()?;
    let (_, call) = p.payload.split_once("_G.__qm_auto_asset_manifest(\"")?;
    let (dir, rest) = call.split_once("\", [")?;
    let (_, body) = rest.split_once("[\n")?;
    let mut assets = Vec::new();
    for line in body.lines() {
        if line.starts_with(']') {
            break;
        }
        let mut cols = line.split('\t').map(|c| c.trim_matches('"'));
        let (Some(rel), Some(cat), Some(stem), Some(ext), Some(mode)) =
            (cols.next(), cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        assets.push(DiscoveredAsset {
            rel_path: rel.to_string(),
            category: cat.to_string(),
            stem: stem.to_string(),
            ext: ext.to_string(),
            default_mode: mode.to_string(),
        });
    }
    Some(AutoAssetManifest {
        mod_id,
        dir: dir.replace("\\\"", "\""),
        assets,
    })
}

/// The mods whose auto-asset blocks are in `main_lua`, in the order the game runs them. The
/// last one to load a file wins it.
pub fn auto_asset_run_order(main_lua: &str) -> Vec<String> {
    main_lua
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix(AUTO_ASSET_HEADER))
        .filter_map(|rest| rest.rsplit_once(": ").map(|(id, _)| id.to_string()))
        .collect()
}

/// The mod root inside a zip, with a trailing '/' ("" for the zip root): the folder that
/// holds `qm.toml` or `qm/`, else the folder that holds an `assets/` file. Shared by the
/// patch, asset and mod.json reads, so they agree (D62).
fn zip_mod_root(names: &[String]) -> Option<String> {
    names
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
                        let (parent, rel) = norm.split_at_checked(idx)?;
                        if classify_asset_relative_path(rel).is_some() {
                            return Some(parent.to_string());
                        }
                    }
                }
                None
            })
        })
}

/// The entry names of a zip.
fn zip_names(zip: &mut ZipArchive<fs::File>) -> Vec<String> {
    (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect()
}

/// A mod's mod.json: None when it has none.
type ManifestRead = Option<Result<ModManifest, String>>;

/// Larger mod.json entries in a zip are not read.
const ZIP_MANIFEST_MAX: u64 = 1024 * 1024;

/// A zip's mod.json (D62): `<mod root>mod.json`, else `mod.json` at the zip root.
fn read_zip_manifest(
    zip: &mut ZipArchive<fs::File>,
    names: &[String],
    mod_root: Option<&str>,
) -> ManifestRead {
    let candidates = [mod_root.map(|r| format!("{r}mod.json")), Some("mod.json".to_string())];
    let name = candidates
        .into_iter()
        .flatten()
        .find(|c| names.iter().any(|n| n == c))?;
    let mut raw = String::new();
    let read = zip
        .by_name(&name)
        .map_err(|e| e.to_string())
        .and_then(|entry| {
            entry
                .take(ZIP_MANIFEST_MAX + 1)
                .read_to_string(&mut raw)
                .map_err(|e| e.to_string())
        });
    Some(match read {
        Err(e) => Err(format!("Failed to read {name} from the zip: {e}")),
        Ok(n) if u64::try_from(n).unwrap_or(u64::MAX) > ZIP_MANIFEST_MAX => {
            Err(format!("{name} is larger than 1 MiB"))
        }
        Ok(_) => ModManifest::from_json_str(&raw),
    })
}

/// Extracts and discovers `assets/` inside a `.zip` mod archive if present.
fn discover_zip_assets(zip_file: &Path) -> Result<(PathBuf, Vec<DiscoveredAsset>)> {
    let file = fs::File::open(zip_file)
        .with_context(|| format!("Failed to open zip file at {:?}", zip_file))?;
    let mut zip = ZipArchive::new(file)
        .with_context(|| format!("Failed to open zip archive for {:?}", zip_file))?;

    let names = zip_names(&mut zip);
    let mod_root = zip_mod_root(&names);

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
        if name.ends_with('/') || name.ends_with('\\') {
            continue;
        }
        let Some(rel) = name.strip_prefix(mod_root.as_str()) else {
            continue;
        };
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

/// The patch files of a mod folder: the root `qm.toml` (or `quartermaster.toml`) and every
/// `.toml` under `qm/`, in load order. A folder with none is an unmanaged or asset-only mod.
pub fn dir_patch_files(mod_dir: &Path) -> Vec<PathBuf> {
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
    toml_files
}

/// If a mod folder in `Mods/` does not contain root manifests directly, check if it contains a
/// nested version subdirectory (e.g. `Mods/GCore/1.0.0/` as in the library layout) containing
/// manifests or patches, and resolve to the newest version folder.
pub fn resolve_mod_dir_root(dir: &Path) -> PathBuf {
    if !dir_patch_files(dir).is_empty()
        || dir.join("mod.json").is_file()
        || qm_schema::faction::has_faction_file(dir)
        || !discover_dir_assets(dir).is_empty()
        || dir.join("config.toml").is_file()
        || dir.join("config.lua").is_file()
    {
        return dir.to_path_buf();
    }

    if let Ok(entries) = fs::read_dir(dir) {
        let mut candidates = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let sub = entry.path();
            if sub.is_dir() {
                let name = sub
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if name.starts_with('.') || name == "qm" || name == "assets" {
                    continue;
                }
                if !dir_patch_files(&sub).is_empty()
                    || sub.join("mod.json").is_file()
                    || qm_schema::faction::has_faction_file(&sub)
                    || !discover_dir_assets(&sub).is_empty()
                    || sub.join("config.toml").is_file()
                    || sub.join("config.lua").is_file()
                {
                    candidates.push(sub);
                }
            }
        }
        if !candidates.is_empty() {
            candidates.sort_by(|a, b| {
                let va = ModManifest::from_path(&a.join("mod.json"))
                    .map(|m| m.version)
                    .unwrap_or_else(|_| {
                        a.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string()
                    });
                let vb = ModManifest::from_path(&b.join("mod.json"))
                    .map(|m| m.version)
                    .unwrap_or_else(|_| {
                        b.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string()
                    });
                let pva = ParsedVersion::parse(&va).unwrap_or(ParsedVersion::ZERO);
                let pvb = ParsedVersion::parse(&vb).unwrap_or(ParsedVersion::ZERO);
                pva.cmp(&pvb)
            });
            if let Some(newest) = candidates.pop() {
                return newest;
            }
        }
    }
    dir.to_path_buf()
}

/// Load patch files from the specified mod directory.
fn get_dir_patches(mod_dir: &Path) -> Result<(PathBuf, Vec<IntermediatePatch>)> {
    let toml_files = dir_patch_files(mod_dir);

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
                            let full_path = if mod_dir.join(&x.source).is_file() {
                                mod_dir.join(&x.source)
                            } else if let Some(parent) = toml_path.parent() {
                                parent.join(&x.source)
                            } else {
                                mod_dir.join(&x.source)
                            };
                            if let Ok(source_content) = fs::read_to_string(&full_path) {
                                sources.insert(x.source.clone(), source_content);
                            }
                        }
                        Patch::Copy(x) => {
                            let Some(ref copy_sources) = x.sources else { continue };
                            for source in copy_sources {
                                let full_path = if mod_dir.join(source).is_file() {
                                    mod_dir.join(source)
                                } else if let Some(parent) = toml_path.parent() {
                                    parent.join(source)
                                } else {
                                    mod_dir.join(source)
                                };
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

/// Load the patch files and the mod.json of the specified zip. The mod.json comes back even
/// when the patches fail, so the mod fails under its mod.json id.
fn get_zip_patches(zip_file: &Path) -> (ManifestRead, Result<(PathBuf, Vec<IntermediatePatch>)>) {
    let opened = fs::File::open(zip_file)
        .with_context(|| format!("Failed to open zip file at {:?}", zip_file))
        .and_then(|file| {
            ZipArchive::new(file)
                .with_context(|| format!("Failed to open zip archive for {:?}", zip_file))
        });
    let mut zip = match opened {
        Ok(zip) => zip,
        Err(e) => return (None, Err(e)),
    };
    let names = zip_names(&mut zip);
    let mod_root = zip_mod_root(&names);
    let manifest = read_zip_manifest(&mut zip, &names, mod_root.as_deref());
    (manifest, zip_patches(zip_file, &mut zip, &names, mod_root))
}

/// The patch files of a zip under its mod root.
fn zip_patches(
    zip_file: &Path,
    zip: &mut ZipArchive<fs::File>,
    names: &[String],
    mod_root: Option<String>,
) -> Result<(PathBuf, Vec<IntermediatePatch>)> {
    let no_patches = || {
        log::warn!("No mod root found in zip {:?}. This may happen if the mod does not contain any QuarterMaster (qm.toml) patches (uses another loader)", zip_file);
        Ok((zip_file.to_path_buf(), Vec::new()))
    };
    let Some(mod_root) = mod_root else {
        return no_patches();
    };

    let qm_toml_path = format!("{}qm.toml", mod_root);
    let qm_dir_prefix = format!("{}qm/", mod_root);
    // An asset-only root.
    if !names
        .iter()
        .any(|n| n == &qm_toml_path || n.starts_with(&qm_dir_prefix))
    {
        return no_patches();
    }

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
            // (every toml and source path starts with mod_root; it was built that way)
            let relative_path = toml_path.get(mod_root.len()..).unwrap_or_default();
            let intermediate_path = zip_file.join(relative_path);

            // Build sources map with paths relative to mod_root
            let mut sources: HashMap<PathBuf, String> = HashMap::new();
            for (full_path, content) in &file_contents {
                let relative = full_path.get(mod_root.len()..).unwrap_or_default();
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

/// One loaded patch: the patch, its priority, its toml path relative to the Mods folder and
/// its `[vars]`.
pub type LoadedPatch = (Patch, Priority, PathBuf, HashMap<String, String>);

/// A mod's patch files, asset root and assets, read from disk.
type ModFiles = (Vec<IntermediatePatch>, PathBuf, Vec<DiscoveredAsset>);

/// One mod folder or zip as read from disk. The mod.json sits beside the files, so a mod
/// whose files fail still fails under its mod.json id.
struct ModRead {
    base: PathBuf,
    folder_name: String,
    manifest: ManifestRead,
    files: Result<ModFiles>,
}

/// What the per-mod load needs to know about the mod and the profile.
struct ModCtx<'a> {
    mod_dir: &'a Path,
    base_path: &'a Path,
    mod_id: &'a str,
    folder_name: &'a str,
    manifest_meta: Option<&'a ModManifest>,
    plan: Option<&'a LoadOrderPlan>,
    /// A zip's stem id when its mod.json gives another id (D62).
    stem_id: Option<&'a str>,
}

/// A mod that loaded, before requirer skipping.
struct LoadedMod {
    mod_id: String,
    meta: ModMeta,
    /// Lowercase id and `provides`.
    keys: Vec<String>,
    /// Lowercase `requires` keys from mod.json.
    requires: Vec<String>,
    patches: Vec<plan::RawPatch>,
    faction: Option<crate::patch::faction::LoadedFaction>,
    config: Option<crate::patch::menu::LoadedModConfig>,
    overrides: Vec<Lenient<Override>>,
}

/// What requirer skipping reads of a loaded mod.
struct Reqs<'a> {
    mod_id: &'a str,
    keys: &'a [String],
    requires: &'a [String],
}

/// A mod that failed to load.
struct FailedMod {
    error: LoadError,
    keys: Vec<String>,
}

/// Lowercase id plus `provides`, the names other mods require this one by.
fn mod_keys(mod_id: &str, manifest: Option<&ModManifest>) -> Vec<String> {
    let mut keys = vec![mod_id.to_lowercase()];
    if let Some(m) = manifest {
        keys.extend(m.relationships.provides.iter().map(|p| p.to_lowercase()));
    }
    keys
}

fn mod_requires(manifest: Option<&ModManifest>) -> Vec<String> {
    let mut req: Vec<String> = manifest
        .map(|m| m.relationships.requires.keys().map(|k| k.to_lowercase()).collect())
        .unwrap_or_default();
    req.sort();
    req
}

/// The mod.json data the plan layer keeps (Phase C §3.2).
fn mod_meta(mod_id: &str, folder_name: &str, manifest: Option<&ModManifest>) -> ModMeta {
    let lower = |v: &mut dyn Iterator<Item = &String>| -> BTreeSet<String> {
        v.map(|s| s.to_lowercase()).collect()
    };
    match manifest {
        Some(m) => ModMeta {
            id: mod_id.to_lowercase(),
            folder: folder_name.to_string(),
            name: Some(m.name.clone()).filter(|n| !n.trim().is_empty()),
            version: Some(m.version.clone()),
            requires: lower(&mut m.relationships.requires.keys()),
            load_after: lower(&mut m.relationships.load_after.iter()),
            load_before: lower(&mut m.relationships.load_before.iter()),
            provides: lower(&mut m.relationships.provides.iter()),
            manifest_ok: true,
            loader_index: 0,
            base: PathBuf::new(),
            overrides: m.overrides.clone(),
        },
        None => ModMeta {
            id: mod_id.to_lowercase(),
            folder: folder_name.to_string(),
            ..ModMeta::default()
        },
    }
}

fn failed_mod(
    mod_id: &str,
    folder_name: &str,
    manifest: Option<&ModManifest>,
    err: &anyhow::Error,
) -> FailedMod {
    let message = format!("{err:#}");
    error!("Mod '{mod_id}' ({folder_name}) could not be loaded and is skipped: {message}");
    FailedMod {
        error: LoadError {
            mod_id: mod_id.to_string(),
            folder: folder_name.to_string(),
            message,
            skipped_dependents: Vec::new(),
        },
        keys: mod_keys(mod_id, manifest),
    }
}

/// Load patches from the provided mod directory. This scans for lovely patch files
/// within each subdirectory that matches either:
/// - MOD_DIR/lovely.toml
/// - MOD_DIR/lovely/*.toml
///
/// Zip archives are supported and uniquely support directory nesting
/// (i.e., mod.zip/dir/lovely.toml), but otherwise are treated the same as dir mods.
///
/// A mod that fails to load is skipped (see `load_patches_reporting`).
pub fn load_patches_new(mod_dir: &Path) -> Result<Vec<LoadedPatch>> {
    load_patches_reporting(mod_dir).map(|(patches, _)| patches)
}

/// `load_patches_new` plus the mods it skipped (D15), with the plan read from
/// `Mods/qm/load-order.json`. `Err` when the Mods folder, its blacklist or the plan cannot be
/// read (`PatchTable::load` reports a plan error instead).
pub fn load_patches_reporting(mod_dir: &Path) -> Result<(Vec<LoadedPatch>, Vec<LoadError>)> {
    let read = LoadOrderPlan::read(mod_dir);
    if let Some(e) = read.error {
        bail!("{e}");
    }
    load_patches_with_plan(mod_dir, read.plan.as_ref())
}

/// `load_raw` then the plan's patch layers, as patches with their vars.
pub fn load_patches_with_plan(
    mod_dir: &Path,
    load_order_plan: Option<&LoadOrderPlan>,
) -> Result<(Vec<LoadedPatch>, Vec<LoadError>)> {
    let raw = read_mods(mod_dir, load_order_plan)?;
    let patches = plan::layer_patches(&raw, load_order_plan, &mut Vec::new());
    Ok((patches, raw.load_errors))
}

/// The disk half of the load (`plan::load_raw`): the blacklist, the enabled filter, the mod
/// priorities, every mod's toml files, assets, faction and mod.json data. Each mod loads on
/// its own. One that fails (an unreadable or malformed qm.toml, a missing module or copy
/// source, ...) is skipped together with every enabled mod whose mod.json requires it,
/// directly or through another skipped mod. `Err` only when the Mods folder or its blacklist
/// cannot be read.
pub(crate) fn read_mods(
    mod_dir: &Path,
    load_order_plan: Option<&LoadOrderPlan>,
) -> Result<RawLoad> {
    read_mods_with(mod_dir, load_order_plan, crate::QM_VERSION, crate::QM_FEATURES)
}

/// `read_mods` checking mod.json gates (D61) against this QuarterMaster version and feature
/// list instead of the built-in ones.
fn read_mods_with(
    mod_dir: &Path,
    load_order_plan: Option<&LoadOrderPlan>,
    qm_version: &str,
    qm_features: &[&str],
) -> Result<RawLoad> {
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
    let dir_results: Vec<ModRead> = mod_contents
        .iter()
        .filter(|x| x.is_dir())
        .filter(|x| {
            let ignore_file = x.join(".qmignore");
            let Some(dirname) = x.file_name() else {
                warn!("Skipping {x:?}: it has no directory name.");
                return false;
            };
            let dirname = dirname.to_string_lossy();
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
            let effective_dir = resolve_mod_dir_root(x);
            let files = get_dir_patches(&effective_dir).map(|(_, ips)| {
                let assets = discover_dir_assets(&effective_dir);
                (ips, effective_dir.clone(), assets)
            });
            let manifest_path = if effective_dir.join("mod.json").is_file() {
                effective_dir.join("mod.json")
            } else {
                x.join("mod.json")
            };
            let manifest = manifest_path
                .is_file()
                .then(|| ModManifest::from_path(&manifest_path));
            ModRead {
                base: effective_dir,
                folder_name: x
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                manifest,
                files,
            }
        })
        .collect();

    // Collect zip patches (read TOMLs into IntermediatePatch), mod.json and discovered assets
    let zip_results: Vec<ModRead> = mod_contents
        .iter()
        .filter(|x| x.is_file())
        .filter(|x| x.extension().is_some_and(|ext| ext == "zip"))
        .sorted_by(|a, b| filename_cmp(a, b))
        .map(|x| {
            let (manifest, patches) = get_zip_patches(x);
            let files = patches.and_then(|(_, ips)| {
                let (asset_root, assets) = discover_zip_assets(x)?;
                Ok((ips, asset_root, assets))
            });
            ModRead {
                base: x.to_path_buf(),
                folder_name: x
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string(),
                manifest,
                files,
            }
        })
        .collect();

    let all_results = dir_results.into_iter().chain(zip_results);
    let mut loaded: Vec<LoadedMod> = Vec::new();
    let mut failed: Vec<FailedMod> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for ModRead {
        base: base_path,
        folder_name,
        manifest: manifest_read,
        files,
    } in all_results
    {
        let is_zip = base_path.extension().is_some_and(|e| e == "zip");
        // mod.json, when present, gives the authoritative mod_id and load_priority
        let manifest_meta = manifest_read.as_ref().and_then(|r| r.as_ref().ok()).cloned();
        let mod_id = manifest_meta
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_else(|| folder_name.to_lowercase());
        // D62: a zip's id was its stem before its mod.json was read; old names still match.
        let stem_id = (is_zip && mod_id != folder_name.to_lowercase())
            .then(|| folder_name.to_lowercase());
        let enabled = load_order_plan.is_none_or(|plan| {
            plan.enabled_mods.iter().any(|m| {
                m.eq_ignore_ascii_case(&mod_id) || m.eq_ignore_ascii_case(&folder_name)
            })
        });

        // The names other mods require this one by, the stem included (D62).
        let keys = || {
            let mut keys = mod_keys(&mod_id, manifest_meta.as_ref());
            keys.extend(stem_id.clone());
            keys
        };

        let (ips, asset_root, discovered_assets) = match files {
            Ok(files) => files,
            Err(e) => {
                if enabled {
                    let mut f = failed_mod(&mod_id, &folder_name, manifest_meta.as_ref(), &e);
                    f.keys = keys();
                    failed.push(f);
                } else {
                    warn!("Mod '{mod_id}' ({folder_name}) is not enabled and could not be read: {e:#}");
                }
                continue;
            }
        };

        // A faction-only mod has no qm.toml and no assets/, so look for faction.toml first.
        let faction_root: Option<PathBuf> = if base_path.is_dir() {
            qm_schema::faction::has_faction_file(&base_path).then(|| base_path.clone())
        } else if is_zip {
            crate::patch::faction::extract_zip_faction(&base_path)
        } else {
            None
        };
        // D61: a mod this QuarterMaster cannot run fails, and its requirers are skipped. Checked
        // before the content check: a gated mod with only a mod.json fails too, as the solver says.
        let gate = enabled
            .then_some(manifest_meta.as_ref())
            .flatten()
            .and_then(|m| crate::features::gate_error_with(m, qm_version, qm_features));
        if let Some(why) = gate {
            let mut f = failed_mod(&mod_id, &folder_name, manifest_meta.as_ref(), &anyhow!(why));
            f.keys = keys();
            failed.push(f);
            continue;
        }

        let has_config = if base_path.is_dir() {
            qm_schema::ModConfigSchema::from_mod_dir(&base_path)
                .ok()
                .flatten()
                .is_some()
        } else {
            false
        };

        if ips.is_empty() && discovered_assets.is_empty() && faction_root.is_none() && !has_config {
            continue;
        }

        if !enabled {
            info!(
                "Skipping mod '{}' ({}) as it is not in load-order.json enabled_mods.",
                mod_id, folder_name
            );
            continue;
        }

        if let Some(Err(e)) = &manifest_read {
            let (name, kind) = if is_zip {
                (format!("{folder_name}.zip"), "zip")
            } else {
                (folder_name.clone(), "folder")
            };
            let w = format!(
                "{name}: mod.json unreadable, using the {kind} name as id; fixes by id may not match ({e})"
            );
            warn!("{w}");
            warnings.push(w);
        }

        let ctx = ModCtx {
            mod_dir,
            base_path: &base_path,
            mod_id: &mod_id,
            folder_name: &folder_name,
            manifest_meta: manifest_meta.as_ref(),
            plan: load_order_plan,
            stem_id: stem_id.as_deref(),
        };
        match load_mod(&ctx, ips, &asset_root, &discovered_assets, faction_root.as_deref()) {
            Ok(ModLoad {
                patches,
                faction,
                config,
                warnings: mod_warnings,
                overrides,
            }) => {
                warnings.extend(mod_warnings);
                let mut meta = ModMeta {
                    base: base_path.clone(),
                    ..mod_meta(&mod_id, &folder_name, manifest_meta.as_ref())
                };
                meta.provides.extend(stem_id.clone());
                loaded.push(LoadedMod {
                    mod_id: mod_id.clone(),
                    meta,
                    keys: keys(),
                    requires: mod_requires(manifest_meta.as_ref()),
                    patches,
                    faction,
                    config,
                    overrides,
                })
            }
            Err(e) => {
                let mut f = failed_mod(&mod_id, &folder_name, manifest_meta.as_ref(), &e);
                f.keys = keys();
                failed.push(f);
            }
        }
    }

    let reqs: Vec<Reqs> = loaded
        .iter()
        .map(|m| Reqs {
            mod_id: &m.mod_id,
            keys: &m.keys,
            requires: &m.requires,
        })
        .collect();
    let skipped = skip_requirers(&reqs, &mut failed);
    let mut raw = RawLoad {
        mod_dir: mod_dir.to_path_buf(),
        load_errors: failed.into_iter().map(|f| f.error).collect(),
        warnings,
        ..RawLoad::default()
    };
    for (m, skip) in loaded.into_iter().zip(skipped) {
        if skip {
            continue;
        }
        let ix = u32::try_from(raw.mods.len()).unwrap_or(u32::MAX);
        let mut meta = m.meta;
        meta.loader_index = ix;
        raw.mods.push(meta);
        raw.patches.extend(m.patches.into_iter().map(|mut p| {
            p.mod_ix = ix;
            p
        }));
        raw.factions.extend(m.faction);
        raw.configs.extend(m.config);
        raw.overrides.extend(m.overrides.into_iter().map(|o| (ix, o)));
    }
    Ok(raw)
}

/// `raw` with one more mod read from text (`RawLoad::with_extra_mod`): parsed as `read_mods`
/// parses a mod folder `<mod_dir>/<id>` holding this mod.json and one qm.toml, with no plan
/// (its patches get the mod.json priority). The folder is never read. The mod goes where
/// the loader would list that folder; later mods move down one index.
pub(crate) fn extra_mod(
    raw: &RawLoad,
    manifest_json: &str,
    qm_toml: &str,
) -> Result<RawLoad, String> {
    let manifest = ModManifest::from_json_str(manifest_json)?;
    let mod_id = manifest.id.clone();
    if mod_id.trim().is_empty() {
        return Err("mod.json has no id".to_string());
    }
    if let Some(why) = crate::features::gate_error(&manifest) {
        return Err(format!("the mod {why}"));
    }
    if raw.mods.iter().any(|m| m.id.eq_ignore_ascii_case(&mod_id)) {
        return Err(format!("a mod with the id '{mod_id}' is already loaded"));
    }
    let base_path = raw.mod_dir.join(&mod_id);
    let ip = IntermediatePatch {
        path: base_path.join("qm.toml"),
        content: qm_toml.to_string(),
        sources: HashMap::new(),
    };
    let ctx = ModCtx {
        mod_dir: &raw.mod_dir,
        base_path: &base_path,
        mod_id: &mod_id,
        folder_name: &mod_id,
        manifest_meta: Some(&manifest),
        plan: None,
        stem_id: None,
    };
    let load = load_mod(&ctx, vec![ip], &base_path.join("assets"), &[], None)
        .map_err(|e| format!("{e:#}"))?;

    // Folders sort by lowercase name, before every zip.
    let is_zip = |p: &Path| p.extension().is_some_and(|e| e == "zip");
    let pos = raw
        .mods
        .iter()
        .position(|m| {
            is_zip(&m.base) || filename_cmp(&m.base, &base_path) == Ordering::Greater
        })
        .unwrap_or(raw.mods.len());
    let ix = u32::try_from(pos).unwrap_or(u32::MAX);
    let shift = |i: u32| if i >= ix { i.saturating_add(1) } else { i };

    let mut out = raw.clone();
    for m in &mut out.mods {
        m.loader_index = shift(m.loader_index);
    }
    out.mods.insert(
        pos,
        ModMeta {
            base: base_path.clone(),
            loader_index: ix,
            ..mod_meta(&mod_id, &mod_id, Some(&manifest))
        },
    );
    for p in &mut out.patches {
        p.mod_ix = shift(p.mod_ix);
    }
    let at = out.patches.iter().take_while(|p| p.mod_ix < ix).count();
    let new_patches = load.patches.into_iter().map(|mut p| {
        p.mod_ix = ix;
        p
    });
    out.patches.splice(at..at, new_patches);
    for (i, _) in &mut out.overrides {
        *i = shift(*i);
    }
    let at = out.overrides.iter().take_while(|(i, _)| *i < ix).count();
    let new_overrides = load.overrides.into_iter().map(|o| (ix, o));
    out.overrides.splice(at..at, new_overrides);
    out.warnings.extend(load.warnings);
    Ok(out)
}

/// What `load_mod` read from one mod.
struct ModLoad {
    patches: Vec<plan::RawPatch>,
    faction: Option<crate::patch::faction::LoadedFaction>,
    config: Option<crate::patch::menu::LoadedModConfig>,
    /// The fallback-id collisions it renamed (D29).
    warnings: Vec<String>,
    /// Its `[[overrides]]` entries, in file order (§3.10).
    overrides: Vec<Lenient<Override>>,
}

/// Loads one enabled mod: its auto-asset patch, its faction, every patch in its toml files,
/// with their base priorities (the profile's mod priority included), and its compat
/// overrides. The profile's patch layers come later (`plan::apply_plan`). Any error fails the
/// whole mod; a malformed override entry does not.
fn load_mod(
    ctx: &ModCtx,
    ips: Vec<IntermediatePatch>,
    asset_root: &Path,
    discovered_assets: &[DiscoveredAsset],
    faction_root: Option<&Path>,
) -> Result<ModLoad> {
    let mut patches: Vec<plan::RawPatch> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut overrides: Vec<Lenient<Override>> = Vec::new();
    let mut faction = None;
    // Determine mod-level base priority for synthesized auto-asset patches
    let first_toml_prio = ips.first().and_then(|ip| {
        let fid = format!("{:?}", ip.path);
        parse_patch_file(&ip.content, &fid, ctx.base_path)
            .ok()
            .map(|pf| pf.manifest.priority)
    });
    let mut mod_base_priority = ctx
        .manifest_meta
        .map(|m| m.load_priority)
        .or(first_toml_prio)
        .unwrap_or(50);
    // A zip's old stem id still names its priority (D62).
    let mut prio_keys = vec![ctx.mod_id, ctx.folder_name];
    prio_keys.extend(ctx.stem_id);
    if let Some(plan) = ctx.plan {
        if let Some(override_prio) = plan.mod_priority(&prio_keys) {
            mod_base_priority = override_prio;
        }
    }

    if let Some(root) = faction_root {
        if let Some(lf) =
            crate::patch::faction::load_mod_faction(ctx.mod_id, mod_base_priority, root)
        {
            faction = Some(lf);
        }
    }

    let config = load_mod_config(ctx.base_path, ctx.mod_id, mod_base_priority);

    if let Some(mut auto_patch) =
        build_auto_asset_patch(ctx.mod_id, asset_root, discovered_assets, mod_base_priority)
    {
        if let Some(stem) = ctx.stem_id {
            add_stem_names(&mut auto_patch, ctx.mod_id, stem, None);
            let old_id = format!("{stem}_auto_assets");
            if auto_patch.id() != Some(old_id.as_str()) {
                auto_patch.extra_mut().legacy_ids.insert(0, old_id);
            }
        }
        let rel_assets_path = ctx
            .base_path
            .strip_prefix(ctx.mod_dir)
            .unwrap_or(ctx.base_path)
            .join("assets");
        patches.push(plan::RawPatch {
            patch: auto_patch,
            base_priority: mod_base_priority,
            explicit_priority: None,
            path: rel_assets_path,
            vars: HashMap::new(),
            mod_ix: 0,
            source: "assets".to_string(),
            ordinal: 0,
        });
    }

    // Ids seen in this mod, for the fallback-id collision suffix (D29).
    let mut seen_ids: HashSet<String> = patches
        .iter()
        .filter_map(|p| p.patch.id().map(str::to_string))
        .collect();
    let mut per_target: HashMap<String, u32> = HashMap::new();
    let mut sprite_files = 0usize;

    for ip in ips {
        let file_identifier = format!("{:?}", ip.path);
        let comment_names = extract_patch_comments(&ip.content);
        let mut patch_file: PatchFile =
            parse_patch_file(&ip.content, &file_identifier, ctx.base_path)?;

        // [sprites] becomes one more patch of this file.
        if !patch_file.sprites.is_empty() {
            let (patch, rule_warnings) =
                build_sprite_rules_patch(ctx.mod_id, &patch_file.sprites, sprite_files);
            for w in rule_warnings {
                warnings.push(format!("{}: {w}", ctx.mod_id));
            }
            if let Some(patch) = patch {
                patch_file.patches.push(patch);
                sprite_files += 1;
            }
        }

        // Base priority: load-order.json override > mod.json load_priority > qm.toml [manifest].priority
        let mut base_priority = ctx
            .manifest_meta
            .map(|m| m.load_priority)
            .unwrap_or(patch_file.manifest.priority);
        if let Some(plan) = ctx.plan {
            if let Some(override_prio) = plan.mod_priority(&prio_keys) {
                base_priority = override_prio;
            }
        }

        // For module and copy patches, use preloaded sources
        let mut fallback: Vec<bool> = Vec::with_capacity(patch_file.patches.len());
        for (patch_idx, patch) in patch_file.patches.iter_mut().enumerate() {
            let target_clean = patch
                .primary_target()
                .replace(['/', '\\', '.'], "_")
                .to_lowercase();
            let fallback_id = format!("{}:{}:{}", ctx.mod_id, target_clean, patch_idx + 1);
            let comment_name = comment_names.get(patch_idx).cloned().flatten();
            fallback.push(patch.id().is_none());
            patch.set_metadata(ctx.mod_id.to_string(), fallback_id, comment_name);

            if let Patch::Module(ref mut x) = patch {
                if x.load_now && x.before.is_none() {
                    bail!(
                        "Error at patch file {}:\nModule \"{}\" has \"load_now\" set to true, but does not have required parameter \"before\" set",
                        ip.path.display(),
                        x.name
                    );
                }

                x.display_source = x.source.to_string_lossy().to_string();
                // The DLL passes both to Lua as C strings.
                if x.name.contains('\0') || x.display_source.contains('\0') {
                    bail!(
                        "Error at patch file {}:\nModule \"{}\" has a NUL character in its name or source",
                        ip.path.display(),
                        x.name.escape_default()
                    );
                }
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
        overrides.extend(patch_file.overrides);

        // mod_relative_path: path relative to top-level mod_dir
        let mod_relative_path = ip.path.strip_prefix(ctx.mod_dir).with_context(|| {
            format!(
                "Base mod directory path {} expected to be a prefix of patch file path {}",
                ctx.mod_dir.display(),
                ip.path.display()
            )
        })?;
        // The toml path inside the mod, '/' and lowercase.
        let source = ip
            .path
            .strip_prefix(ctx.base_path)
            .unwrap_or(&ip.path)
            .to_string_lossy()
            .replace('\\', "/")
            .to_lowercase();

        // --- Log output immediately after each mod manifest loads ---
        info!(
            "Loaded manifest: '{}' (mod_id: '{}') | Priority: {} | Patches: {}",
            mod_relative_path.display(),
            ctx.mod_id,
            base_priority,
            patch_file.patches.len()
        );

        for (mut patch, is_fallback) in patch_file.patches.into_iter().zip(fallback) {
            let loader_id = patch.id().filter(|_| is_fallback).map(str::to_string);
            if let Some(id) = patch.id().map(str::to_string) {
                if is_fallback && seen_ids.contains(&id) {
                    // D29: the first keeps the id; legacy entries still match this one.
                    let mut new_id = format!("{id}#{source}");
                    let mut n = 2;
                    while seen_ids.contains(&new_id) {
                        new_id = format!("{id}#{source}#{n}");
                        n += 1;
                    }
                    let w = format!(
                        "{}: two patches have the id '{id}'; the one in {source} is now '{new_id}'",
                        ctx.mod_id
                    );
                    warn!("{w}");
                    warnings.push(w);
                    patch.set_id(new_id.clone());
                    patch.extra_mut().legacy_id = Some(id);
                    seen_ids.insert(new_id);
                } else {
                    seen_ids.insert(id);
                }
            }
            if let Some(stem) = ctx.stem_id {
                add_stem_names(&mut patch, ctx.mod_id, stem, loader_id.as_deref());
            }
            let ordinal = per_target
                .entry(patch.primary_target().to_lowercase())
                .or_insert(0);
            *ordinal += 1;
            patches.push(plan::RawPatch {
                explicit_priority: patch.priority_override(),
                patch,
                base_priority,
                path: mod_relative_path.to_path_buf(),
                vars: vars.clone(),
                mod_ix: 0,
                source: source.clone(),
                ordinal: *ordinal,
            });
        }
    }

    Ok(ModLoad {
        patches,
        faction,
        config,
        warnings,
        overrides,
    })
}

fn load_mod_config(
    base_path: &Path,
    mod_id: &str,
    priority: Priority,
) -> Option<crate::patch::menu::LoadedModConfig> {
    if base_path.is_dir() {
        if let Ok(Some(cfg)) = qm_schema::ModConfigSchema::from_mod_dir(base_path) {
            return Some(crate::patch::menu::LoadedModConfig {
                mod_id: mod_id.to_string(),
                priority,
                config: cfg,
            });
        }
    } else if base_path.extension().is_some_and(|e| e == "zip") {
        if let Ok(file) = std::fs::File::open(base_path) {
            if let Ok(mut zip) = zip::ZipArchive::new(file) {
                // Try config.toml first
                for i in 0..zip.len() {
                    if let Ok(mut entry) = zip.by_index(i) {
                        let name = entry.name().replace('\\', "/");
                        if name.ends_with("config.toml") {
                            let mut buf = String::new();
                            if std::io::Read::read_to_string(&mut entry, &mut buf).is_ok() {
                                if let Ok(cfg) = qm_schema::ModConfigSchema::from_toml_str(&buf) {
                                    return Some(crate::patch::menu::LoadedModConfig {
                                        mod_id: mod_id.to_string(),
                                        priority,
                                        config: cfg,
                                    });
                                }
                            }
                        }
                    }
                }
                // Try qm.toml next
                for i in 0..zip.len() {
                    if let Ok(mut entry) = zip.by_index(i) {
                        let name = entry.name().replace('\\', "/");
                        if name.ends_with("qm.toml") {
                            let mut buf = String::new();
                            if std::io::Read::read_to_string(&mut entry, &mut buf).is_ok() {
                                if let Ok(cfg) = qm_schema::ModConfigSchema::from_toml_str(&buf) {
                                    if !cfg.options.is_empty()
                                        || !cfg.modes.is_empty()
                                        || !cfg.pages.is_empty()
                                    {
                                        return Some(crate::patch::menu::LoadedModConfig {
                                            mod_id: mod_id.to_string(),
                                            priority,
                                            config: cfg,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Records the names a zip's patch had when the zip's id was its stem (D62), so plan entries
/// and rule names written then still match. `loader_id` is the fallback id the loader gave
/// the patch (before any D29 suffix) when it had no id of its own.
fn add_stem_names(patch: &mut Patch, mod_id: &str, stem: &str, loader_id: Option<&str>) {
    let prefix = format!("{mod_id}:");
    let old = |s: &str| s.strip_prefix(&prefix).map(|rest| format!("{stem}:{rest}"));
    let mut names: Vec<String> = Vec::new();
    if loader_id.is_some() {
        names.extend(patch.id().and_then(old));
        names.extend(patch.extra().legacy_id.as_deref().and_then(old));
    }
    // A name the loader made from the fallback id had the stem too.
    let name = patch.name().unwrap_or("");
    let old_name = match loader_id {
        Some(l) if l == name => old(name),
        _ => None,
    }
    .unwrap_or_else(|| name.to_string());
    names.push(format!("{stem}:{old_name}"));

    let current = plan::plan_keys(patch);
    let extra = patch.extra_mut();
    for n in names {
        if !current.contains(&n) && !extra.legacy_ids.contains(&n) {
            extra.legacy_ids.push(n);
        }
    }
    extra.legacy_mod = Some(stem.to_string());
}

/// Marks every loaded mod that requires a failed or skipped mod (by id or `provides`, case
/// insensitive, and not also offered by a mod still loaded) as skipped, until nothing
/// changes. Each skip is listed on the failed mod it traces back to.
fn skip_requirers(loaded: &[Reqs], failed: &mut [FailedMod]) -> Vec<bool> {
    let mut skipped = vec![false; loaded.len()];
    // Lost key -> index of the failed mod it traces back to.
    let mut lost: HashMap<String, usize> = HashMap::new();
    for (i, f) in failed.iter().enumerate() {
        for k in &f.keys {
            lost.entry(k.clone()).or_insert(i);
        }
    }
    loop {
        let available: HashSet<String> = loaded
            .iter()
            .zip(&skipped)
            .filter(|(_, s)| !**s)
            .flat_map(|(m, _)| m.keys.iter().cloned())
            .collect();
        let mut changed = false;
        for (m, skip) in loaded.iter().zip(skipped.iter_mut()) {
            if *skip {
                continue;
            }
            let root = m
                .requires
                .iter()
                .filter(|k| !available.contains(*k))
                .find_map(|k| lost.get(k).copied().map(|root| (k, root)));
            let Some((key, root)) = root else {
                continue;
            };
            *skip = true;
            changed = true;
            if let Some(f) = failed.get_mut(root) {
                warn!(
                    "Mod '{}' requires '{key}', which could not be loaded ('{}'); skipping it too.",
                    m.mod_id, f.error.mod_id
                );
                f.error.skipped_dependents.push(m.mod_id.to_string());
            }
            for k in m.keys {
                lost.entry(k.clone()).or_insert(root);
            }
        }
        if !changed {
            return skipped;
        }
    }
}

/// What `process_patches` returns: the patches, their targets and the merged vars.
pub type ProcessedPatches = (
    Vec<(Patch, Priority, PathBuf)>,
    HashSet<String>,
    HashMap<String, String>,
);

/// Process raw patches to extract targets and consolidate variables
pub fn process_patches(raw_patches: Vec<LoadedPatch>) -> ProcessedPatches {
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

/// One `[vars]` name that two or more mods set to different values (G12).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VarCollision {
    pub name: String,
    /// Each distinct `(mod id, value)`, in the order `process_patches` merges them.
    pub values: Vec<(String, String)>,
    /// The value the merged map keeps (the last one merged) and its mod.
    pub used: (String, String),
}

/// The `[vars]` names that `process_patches` merges from two or more mods with different
/// values. Only reads; the merged map is unchanged.
pub fn var_collisions(raw_patches: &[LoadedPatch]) -> Vec<VarCollision> {
    let mut seen: Vec<VarCollision> = Vec::new();
    for (patch, _, path, vars) in raw_patches {
        let mod_id = patch.mod_id().map_or_else(
            || {
                path.components()
                    .next()
                    .and_then(|c| c.as_os_str().to_str())
                    .unwrap_or("unknown")
                    .to_lowercase()
            },
            str::to_string,
        );
        for (name, value) in vars.iter().sorted() {
            let pair = (mod_id.clone(), value.clone());
            match seen.iter_mut().find(|c| &c.name == name) {
                Some(c) => {
                    if !c.values.contains(&pair) {
                        c.values.push(pair.clone());
                    }
                    c.used = pair;
                }
                None => seen.push(VarCollision {
                    name: name.clone(),
                    values: vec![pair.clone()],
                    used: pair,
                }),
            }
        }
    }
    seen.retain(|c| {
        let mods: HashSet<&str> = c.values.iter().map(|(m, _)| m.as_str()).collect();
        let values: HashSet<&str> = c.values.iter().map(|(_, v)| v.as_str()).collect();
        mods.len() > 1 && values.len() > 1
    });
    seen.sort_by(|a, b| a.name.cmp(&b.name));
    seen
}

/// True for a top-level key of `qm.toml` that the game has no use for and that is no mistake:
/// `[publish]` is read by the site builder.
fn is_key_for_other_tools(key: &str) -> bool {
    key == "publish"
}

/// Parse TOML content into a PatchFile
fn parse_patch_file(content: &str, file_identifier: &str, mod_dir: &Path) -> Result<PatchFile> {
    // HACK: Replace instances of {{qm_hack:patch_dir}} with mod directory.
    let clean_mod_dir = &mod_dir.to_string_lossy().replace("\\", "\\\\");
    let content = content.replace("{{qm_hack:patch_dir}}", clean_mod_dir);

    let ignored_key_callback = |key: serde_ignored::Path| {
        if is_key_for_other_tools(&key.to_string()) {
            return;
        }
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
        .and_then(|i| path.get(..=i))
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

        let (_, patches) = get_zip_patches(&zip).1.unwrap();

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

        let (_, patches) = get_zip_patches(&zip).1.unwrap();

        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].sources.get(Path::new("inject.lua")).unwrap(), "-- nested");
    }

    #[test]
    fn zip_patches_assets_and_mod_json_share_one_root() {
        let temp = TempDir::new().unwrap();
        // An assets/ folder of another root comes first in the zip; qm.toml decides.
        let zip = make_zip(&temp, "Pack.zip", &[
            ("Other/assets/sounds/x.wav", "RIFF_OTHER"),
            ("Other/mod.json", r#"{"id": "other", "name": "O", "version": "1.0.0"}"#),
            ("Main/assets/sounds/y.wav", "RIFF_MAIN"),
            ("Main/qm.toml", PATCH_TOML),
            ("Main/inject.lua", "-- main"),
            ("Main/mod.json", r#"{"id": "main", "name": "M", "version": "1.0.0"}"#),
        ]);
        let mut archive = ZipArchive::new(fs::File::open(&zip).unwrap()).unwrap();
        let names = zip_names(&mut archive);
        assert_eq!(zip_mod_root(&names).as_deref(), Some("Main/"));

        let (manifest, patches) = get_zip_patches(&zip);
        assert_eq!(manifest.unwrap().unwrap().id, "main");
        let (_, patches) = patches.unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].sources.get(Path::new("inject.lua")).unwrap(), "-- main");

        let (root, assets) = discover_zip_assets(&zip).unwrap();
        let rels: Vec<&str> = assets.iter().map(|a| a.rel_path.as_str()).collect();
        assert_eq!(rels, ["assets/sounds/y.wav"]);
        assert_eq!(fs::read_to_string(root.join("assets/sounds/y.wav")).unwrap(), "RIFF_MAIN");

        // With no qm.toml, the assets root is the root for both.
        let zip = make_zip(&temp, "Assets.zip", &[
            ("Snd/assets/sounds/z.wav", "RIFF_Z"),
            ("Snd/mod.json", r#"{"id": "snd", "name": "S", "version": "1.0.0"}"#),
        ]);
        let (manifest, patches) = get_zip_patches(&zip);
        assert_eq!(manifest.unwrap().unwrap().id, "snd");
        assert!(patches.unwrap().1.is_empty());
        assert_eq!(discover_zip_assets(&zip).unwrap().1.len(), 1);

        // No mod.json anywhere.
        let zip = make_zip(&temp, "Bare.zip", &[("qm.toml", PATCH_TOML), ("inject.lua", "")]);
        assert!(get_zip_patches(&zip).0.is_none());
    }

    #[test]
    fn zip_no_mod_root_is_empty() {
        let temp = TempDir::new().unwrap();
        let zip = make_zip(&temp, "empty.zip", &[("random.txt", "")]);

        let (_, patches) = get_zip_patches(&zip).1.unwrap();
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

        // The mod is skipped and reported; the load itself succeeds (D15).
        let (patches, errors) = load_patches_reporting(mods).unwrap();
        assert!(patches.is_empty());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].mod_id, "mod");
        assert!(errors[0].message.contains("not found"), "{}", errors[0].message);
    }

    #[test]
    fn module_name_with_nul_skips_the_mod() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();
        let toml = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.module]\n\
                    source = \"m.lua\"\nname = \"bad\\u0000name\"\n";
        write_mod(mods, "Bad", None, toml);
        fs::write(mods.join("Bad").join("m.lua"), "return {}").unwrap();
        write_mod(mods, "Good", None, &copy_toml("a.lua"));

        let (patches, errors) = load_patches_reporting(mods).unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].folder, "Bad");
        assert!(errors[0].message.contains("NUL character"), "{}", errors[0].message);
    }

    fn write_mod(mods: &Path, folder: &str, mod_json: Option<&str>, qm_toml: &str) {
        let m = mods.join(folder);
        fs::create_dir_all(&m).unwrap();
        fs::write(m.join("qm.toml"), qm_toml).unwrap();
        if let Some(j) = mod_json {
            fs::write(m.join("mod.json"), j).unwrap();
        }
    }

    fn copy_toml(target: &str) -> String {
        format!(
            r#"[manifest]
version = "1.0.0"

[[patches]]
[patches.copy]
target = "{target}"
position = "append"
payload = "-- {target}"
"#
        )
    }

    const MALFORMED_TOML: &str = r#"[manifest]
version = "1.0.0"
[[patches]]
[patches.pattern
"#;

    #[test]
    fn malformed_mod_and_its_requirers_are_skipped() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();
        // base has a malformed qm.toml.
        write_mod(
            mods,
            "Base",
            Some(r#"{"id": "base", "name": "Base", "version": "1.0.0", "relationships": {"provides": ["base-api"]}}"#),
            MALFORMED_TOML,
        );
        // needs_base requires base (by id, any case), needs_api requires it through
        // `provides`, needs_needs requires needs_base only (transitive).
        write_mod(
            mods,
            "NeedsBase",
            Some(r#"{"id": "needs_base", "name": "N", "version": "1.0.0", "relationships": {"requires": {"Base": "*"}}}"#),
            &copy_toml("a.lua"),
        );
        write_mod(
            mods,
            "NeedsApi",
            Some(r#"{"id": "needs_api", "name": "N", "version": "1.0.0", "relationships": {"requires": {"base-api": "*"}}}"#),
            &copy_toml("b.lua"),
        );
        write_mod(
            mods,
            "NeedsNeeds",
            Some(r#"{"id": "needs_needs", "name": "N", "version": "1.0.0", "relationships": {"requires": {"needs_base": "*"}}}"#),
            &copy_toml("c.lua"),
        );
        // Unrelated mods still load, including one whose requirement is simply absent.
        write_mod(mods, "Other", None, &copy_toml("d.lua"));
        write_mod(
            mods,
            "Loose",
            Some(r#"{"id": "loose", "name": "L", "version": "1.0.0", "relationships": {"requires": {"absent": "*"}}}"#),
            &copy_toml("e.lua"),
        );

        let (patches, errors) = load_patches_reporting(mods).unwrap();
        let mut loaded: Vec<String> = patches
            .iter()
            .map(|(p, _, _, _)| p.mod_id().unwrap_or_default().to_string())
            .collect();
        loaded.sort();
        assert_eq!(loaded, ["loose", "other"]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        let e = &errors[0];
        assert_eq!(e.mod_id, "base");
        assert_eq!(e.folder, "Base");
        assert!(e.message.contains("Failed to parse patch file"), "{}", e.message);
        let mut dependents = e.skipped_dependents.clone();
        dependents.sort();
        assert_eq!(dependents, ["needs_api", "needs_base", "needs_needs"]);

        // The table reports the same; only the strict load fails.
        let table = crate::patch::table::PatchTable::load(mods).unwrap();
        // (Its message names the canonical path.)
        assert_eq!(table.load_errors.len(), 1);
        assert_eq!(table.load_errors[0].mod_id, "base");
        assert_eq!(table.load_errors[0].skipped_dependents, e.skipped_dependents);
        let strict = crate::patch::table::PatchTable::load_strict(mods);
        let msg = format!("{:#}", strict.err().unwrap());
        assert!(msg.contains("Failed to parse patch file"), "{msg}");
    }

    #[test]
    fn a_disabled_broken_mod_is_not_an_error() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();
        write_mod(mods, "Broken", None, MALFORMED_TOML);
        write_mod(mods, "Good", None, &copy_toml("a.lua"));
        fs::write(
            mods.join("qm").join("load-order.json"),
            r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["good"]}"#,
        )
        .unwrap();
        let (patches, errors) = load_patches_reporting(mods).unwrap();
        assert_eq!(patches.len(), 1);
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A Mods folder with a gated mod `gated`, a mod requiring it and an unrelated mod.
    fn gated_world(mods: &Path, gates: &str) {
        fs::create_dir_all(mods.join("qm")).unwrap();
        write_mod(
            mods,
            "Gated",
            Some(&format!(r#"{{"id": "gated", "name": "Gated", "version": "1.0.0"{gates}}}"#)),
            &copy_toml("a.lua"),
        );
        write_mod(
            mods,
            "NeedsGated",
            Some(r#"{"id": "needs_gated", "name": "N", "version": "1.0.0", "relationships": {"requires": {"gated": "*"}}}"#),
            &copy_toml("b.lua"),
        );
        write_mod(mods, "Other", None, &copy_toml("c.lua"));
    }

    fn loaded_ids(raw: &RawLoad) -> Vec<String> {
        raw.mods.iter().map(|m| m.id.clone()).collect()
    }

    /// `[publish]` is the site builder's, so it is no unknown key; a key inside another
    /// table of that name, or any other key, still is.
    #[test]
    fn the_publish_table_is_not_reported_as_an_unknown_key() {
        let raw = "[manifest]\nversion = \"1.0.0\"\ntypo = 1\n\n[publish]\nrepository = \"a/b\"\n";
        let mut unknown = Vec::new();
        let _: PatchFile =
            serde_ignored::deserialize(toml::Deserializer::new(raw), |k| unknown.push(k.to_string()))
                .unwrap();
        unknown.retain(|k| !is_key_for_other_tools(k));
        assert_eq!(unknown, ["manifest.typo"]);
    }

    #[test]
    fn an_unknown_feature_fails_the_mod_and_its_requirer() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        gated_world(mods, r#", "qm_features": ["overrides", "teleport"]"#);
        let raw = read_mods(mods, None).unwrap();
        assert_eq!(loaded_ids(&raw), ["other"]);
        assert_eq!(raw.load_errors.len(), 1, "{:?}", raw.load_errors);
        let e = &raw.load_errors[0];
        assert_eq!((e.mod_id.as_str(), e.folder.as_str()), ("gated", "Gated"));
        assert_eq!(
            e.message,
            "needs the QuarterMaster feature 'teleport', which this version does not have"
        );
        assert_eq!(e.skipped_dependents, ["needs_gated"]);
    }

    #[test]
    fn a_met_min_qm_and_known_features_load() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        let gates = format!(r#", "min_qm": "{}", "qm_features": ["overrides"]"#, crate::QM_VERSION);
        gated_world(mods, &gates);
        let raw = read_mods(mods, None).unwrap();
        assert!(raw.load_errors.is_empty(), "{:?}", raw.load_errors);
        assert_eq!(loaded_ids(&raw), ["gated", "needs_gated", "other"]);
    }

    #[test]
    fn min_qm_above_the_version_or_unparsable_fails_the_mod() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        gated_world(mods, r#", "min_qm": "999.0.0""#);
        let raw = read_mods(mods, None).unwrap();
        assert_eq!(loaded_ids(&raw), ["other"]);
        let e = &raw.load_errors[0];
        assert_eq!(
            e.message,
            format!("needs QuarterMaster 999.0.0 or newer (this is {})", crate::QM_VERSION)
        );
        assert_eq!(e.skipped_dependents, ["needs_gated"]);

        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        gated_world(mods, r#", "min_qm": "soon""#);
        let raw = read_mods(mods, None).unwrap();
        assert_eq!(loaded_ids(&raw), ["other"]);
        let e = &raw.load_errors[0];
        assert!(
            e.message.starts_with("has a min_qm that is not a version ('soon': "),
            "{}",
            e.message
        );
    }

    #[test]
    fn an_older_feature_list_fails_a_mod_that_needs_a_newer_feature() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        gated_world(mods, r#", "qm_features": ["apply_as"]"#);
        let now = read_mods(mods, None).unwrap();
        assert_eq!(loaded_ids(&now), ["gated", "needs_gated", "other"]);

        let old: Vec<&str> = crate::QM_FEATURES
            .iter()
            .copied()
            .filter(|f| *f != "apply_as")
            .collect();
        let then = read_mods_with(mods, None, crate::QM_VERSION, &old).unwrap();
        assert_eq!(loaded_ids(&then), ["other"]);
        assert!(then.load_errors[0].message.contains("'apply_as'"));
        assert_eq!(then.load_errors[0].skipped_dependents, ["needs_gated"]);
    }

    #[test]
    fn a_gated_mod_that_is_not_enabled_is_not_an_error() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        gated_world(mods, r#", "qm_features": ["teleport"]"#);
        fs::write(
            mods.join("qm").join("load-order.json"),
            r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["other"]}"#,
        )
        .unwrap();
        let (patches, errors) = load_patches_reporting(mods).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(patches.len(), 1);
    }

    #[test]
    fn unreadable_mods_folder_is_an_error() {
        let temp = TempDir::new().unwrap();
        assert!(load_patches_reporting(&temp.path().join("absent")).is_err());
    }

    #[test]
    fn filename_cmp_handles_paths_without_a_name() {
        assert_eq!(filename_cmp(Path::new("/"), Path::new("a")), Ordering::Less);
        assert_eq!(filename_cmp(Path::new("B"), Path::new("a")), Ordering::Greater);
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
    fn var_collisions_name_both_mods_and_the_value_used() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        fs::create_dir_all(mods.join("qm")).unwrap();
        for (dir, speed, same) in [("a", "1", "x"), ("b", "2", "x")] {
            let m = mods.join(dir);
            fs::create_dir_all(&m).unwrap();
            fs::write(
                m.join("qm.toml"),
                format!(
                    "[manifest]\nversion = \"1.0.0\"\n\n[vars]\nSPEED = \"{speed}\"\nSAME = \"{same}\"\n\n\
                     [[patches]]\n[patches.copy]\ntarget = \"game.lua\"\nposition = \"append\"\npayload = \"-- {dir}\"\n"
                ),
            )
            .unwrap();
        }

        let raw = load_patches_new(mods).unwrap();
        let collisions = var_collisions(&raw);
        let (_, _, vars) = process_patches(raw);

        assert_eq!(collisions.len(), 1, "{collisions:?}");
        let c = &collisions[0];
        assert_eq!(c.name, "SPEED");
        let mut values = c.values.clone();
        values.sort();
        assert_eq!(values, vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())]);
        // The value used is the one the merged map keeps.
        assert_eq!(vars.get("SPEED"), Some(&c.used.1));
    }

    #[test]
    fn get_parent_extracts_dir() {
        assert_eq!(get_parent("a/b/c.txt"), "a/b/");
        assert_eq!(get_parent("file.txt"), "");
        assert_eq!(get_parent(""), "");
    }

    #[test]
    fn vanilla_asset_index_tells_overrides_from_additions() {
        let lua = "garandSound = love.audio.newSource(\"sounds/garand1.wav\", \"static\")\nlocal img = love.graphics.newImage('sprites/weapons/top_carcano.png')\nweaponText_carcano = \"Carcano\"\nurbLogoSound = nil\n";
        let idx = VanillaAssetIndex::from_lua_sources([lua]);
        let a = |p: &str| classify_asset_relative_path(p).unwrap();
        assert!(idx.overrides(&a("assets/sounds/garand1.wav")));
        assert!(idx.overrides(&a("assets/sprites/top_carcano.png")), "stem match across folders");
        assert!(idx.overrides(&a("assets/text/weaponText_carcano.txt")));
        assert!(idx.overrides(&a("assets/sounds/urbLogo.ogg")), "audio Sound global");
        assert!(!idx.overrides(&a("assets/sounds/brand_new.wav")));
        assert!(!idx.overrides(&a("assets/text/newText.txt")));
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
    fn an_auto_asset_patch_reads_back_into_its_manifest() {
        let assets: Vec<DiscoveredAsset> = [
            "assets/sounds/garand1.wav",
            "assets/sounds/vo/axis/axisHurt7.wav",
            "assets/sprites/allied_idle.png",
        ]
        .iter()
        .map(|rel| classify_asset_relative_path(rel).unwrap())
        .collect();
        let patch =
            build_auto_asset_patch("axissw", Path::new("D:\\Mods\\axisSW"), &assets, 50).unwrap();
        let m = auto_asset_manifest(&patch).unwrap();
        assert_eq!(m.mod_id, "axissw");
        assert_eq!(m.dir, "D:/Mods/axisSW");
        assert_eq!(m.assets, assets);

        // The header names the mod, so the patched main.lua gives the run order.
        let Patch::Pattern(p) = &patch else { panic!() };
        let other = p.payload.replace("(axissw:", "(voices: two:");
        let text = format!("x()\n  {}\ny()\n{}\nsetOptionVolume()\n", p.payload, other);
        assert_eq!(auto_asset_run_order(&text), ["axissw", "voices: two"]);
        assert!(auto_asset_run_order("setOptionVolume()\n").is_empty());

        // The manifest call names the mod.
        assert!(p.payload.ends_with("]===], \"axissw\")\nend"), "{}", p.payload);

        // Excluded files are left out; the rest of the patch stays as it is.
        let one = ["ASSETS\\sounds\\garand1.wav".to_string()];
        let less = without_excluded_assets(&patch, &one).unwrap();
        let lm = auto_asset_manifest(&less).unwrap();
        assert_eq!(lm.assets, assets[1..]);
        assert_eq!((lm.mod_id.as_str(), lm.dir.as_str()), ("axissw", "D:/Mods/axisSW"));
        assert_eq!(less.name(), Some("Auto-Load Assets (2 files)"));
        assert_eq!(less.id(), patch.id());
        // Nothing of it excluded: the same patch. Everything excluded: no patch.
        let same = |excluded: &[String]| {
            let out = without_excluded_assets(&patch, excluded).unwrap();
            let Patch::Pattern(o) = &out else { panic!() };
            o.payload == p.payload && o.name == p.name
        };
        assert!(same(&["assets/other.wav".to_string()]));
        assert!(same(&[]));
        let all: Vec<String> = assets.iter().map(|a| a.rel_path.clone()).collect();
        assert!(without_excluded_assets(&patch, &all).is_none());

        // Any other patch is not one.
        let mut plain = p.clone();
        plain.id = Some("axissw:main_lua:1".into());
        assert!(auto_asset_manifest(&Patch::Pattern(plain)).is_none());
    }

    #[test]
    fn the_archive_listing_finds_assets_the_sources_do_not_name() {
        // The game builds this path in a loop, so no literal names it.
        let lua = "for z = 1, 30 do _G[\"axisHurt\"..z] = snd(\"sounds/vo/axis/axisHurt\"..z..\".wav\") end\ngarand1 = snd(\"sounds/garand1.wav\")\n";
        let asset = |rel: &str| classify_asset_relative_path(rel).unwrap();
        let hurt = asset("assets/sounds/vo/axis/axisHurt7.wav");
        let garand = asset("assets/sounds/garand1.wav");
        let new = asset("assets/sounds/brandNew.wav");

        let from_lua = VanillaAssetIndex::from_lua_sources([lua]);
        assert!(!from_lua.overrides(&hurt), "the sources alone miss it");
        assert!(from_lua.overrides(&garand));

        let paths = ["sounds/vo/axis/axisHurt7.wav", "sounds\\garand1.wav", "noext", "a.b/c"];
        let idx = VanillaAssetIndex::from_archive_paths(paths, [lua]);
        assert!(idx.overrides(&hurt));
        assert!(idx.overrides(&garand));
        assert!(!idx.overrides(&new));
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

    #[test]
    fn auto_asset_patch_with_thousands_of_files_uses_compact_manifest_block() {
        let assets: Vec<DiscoveredAsset> = (1..=3500)
            .map(|i| DiscoveredAsset {
                rel_path: format!("assets/sounds/vo/axis/germ{i}.wav"),
                category: "sounds/vo".to_string(),
                stem: format!("germ{i}"),
                ext: "wav".to_string(),
                default_mode: "stream".to_string(),
            })
            .collect();

        let patch = build_auto_asset_patch("bigpack", Path::new("D:/Mods/BigPack"), &assets, 50)
            .expect("should build patch");

        if let Patch::Pattern(p) = patch {
            assert!(p.payload.contains("_G.__qm_auto_asset_manifest("));
            assert!(p.payload.contains("[===["));
            assert!(p.payload.contains("]===], \"bigpack\")"));
            assert_eq!(
                p.payload.matches("_G.__qm_auto_asset(").count(),
                0,
                "must not emit thousands of individual function calls inside if..end block"
            );
        } else {
            panic!("Expected Pattern patch");
        }
    }

    fn pattern_toml(id: &str, payload: &str, extra: &str) -> String {
        format!(
            "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\nid = {id:?}\n\
             target = \"main.lua\"\npattern = \"x()\"\nposition = \"after\"\npayload = {payload:?}\n\n\
             {extra}"
        )
    }

    fn manifest_json(id: &str, requires: &str) -> String {
        format!(
            r#"{{"id": "{id}", "name": "{id}", "version": "1.0.0", "load_priority": 30,
                "relationships": {{"requires": {{{requires}}}}}}}"#
        )
    }

    #[test]
    fn extra_mod_equals_the_installed_folder() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        let dir_payload = "-- {{qm_hack:patch_dir}}";
        let over = "[[overrides]]\npatch = \"zeta:z\"\ndisable = true\n";
        let alpha = pattern_toml("alpha:a", dir_payload, "");
        write_mod(mods, "alpha", Some(&manifest_json("alpha", "")), &alpha);
        let zeta = pattern_toml("zeta:z", "-- z", over);
        write_mod(mods, "zeta", Some(&manifest_json("zeta", "")), &zeta);
        let before = plan::load_raw(mods, None).unwrap();

        let extra_json = manifest_json("compat_x", r#""alpha": ">=1.0.0", "zeta": ">=1.0.0""#);
        let extra_toml = pattern_toml("compat_x:c", dir_payload, over);
        let preview = before.with_extra_mod(&extra_json, &extra_toml).unwrap();

        write_mod(mods, "compat_x", Some(&extra_json), &extra_toml);
        let installed = plan::load_raw(mods, None).unwrap();
        let ids: Vec<&str> = preview.mods.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["alpha", "compat_x", "zeta"], "the loader's folder order");
        assert_eq!(preview.mods, installed.mods);
        assert_eq!(format!("{:?}", preview.patches), format!("{:?}", installed.patches));
        assert_eq!(format!("{:?}", preview.overrides), format!("{:?}", installed.overrides));
        assert_eq!(preview.warnings, installed.warnings);
        let owners: Vec<u32> = preview.overrides.iter().map(|(i, _)| *i).collect();
        assert_eq!(owners, [1, 2]);

        // Nothing is read from disk: the folder is not needed.
        let other = manifest_json("compat_y", "");
        assert!(before.with_extra_mod(&other, &extra_toml).is_ok());
        // A mod that is loaded already, and text that does not parse, are refused.
        assert!(before.with_extra_mod(&manifest_json("ALPHA", ""), &extra_toml).is_err());
        assert!(before.with_extra_mod("{", &extra_toml).is_err());
        assert!(before.with_extra_mod(&other, "[[patches]").is_err());
        // So is a mod this QuarterMaster could not load (D61).
        let gated = r#"{"id": "compat_z", "name": "Z", "version": "1.0.0", "qm_features": ["teleport"]}"#;
        let e = before.with_extra_mod(gated, &extra_toml).unwrap_err();
        assert!(e.contains("'teleport'"), "{e}");
        let met = r#"{"id": "compat_z", "name": "Z", "version": "1.0.0", "qm_features": ["overrides"]}"#;
        assert!(before.with_extra_mod(met, &extra_toml).is_ok());
    }
}
