
-- QuarterMaster: damaged saves
-- A save file that cannot be read must not take the game down, and one bad write must not
-- cost the player the save.
--   Loading: a save that fails to load is set aside as "<name>.damaged", and the game gets
--     the newest copy that does load: backUp.lua (for saveMNB3.lua), then "<name>.prev",
--     then a fresh profile.
--   Writing: before a save file is overwritten, the copy on disk (if it looks like a save)
--     becomes "<name>.prev". A write that never reached the disk leaves only that copy;
--     the game is then told the save is there, and gets the copy.
do
  local fs = love and love.filesystem
  _G.QM = _G.QM or {}
  if fs and fs.load and fs.write and not _G.QM.saveGuard then
    local guard = { used = {} }
    _G.QM.saveGuard = guard
    local origLoad, origWrite, origInfo = fs.load, fs.write, fs.getInfo

    -- The files that hold a save, and so get a ".prev" copy.
    local ROTATED = { ["saveMNB3.lua"] = true, ["saveMNB2.lua"] = true, ["saveMNBclassic.lua"] = true }
    local function guarded(name)
      return type(name) == "string"
        and (name:match("^saveMNB") or name:match("^backUp") or name:match("save.*%.lua$")) ~= nil
    end
    local function exists(name)
      if origInfo then return origInfo(name) ~= nil end
      return true
    end
    local function realPath(name)
      local dir = fs.getSaveDirectory and fs.getSaveDirectory()
      if type(dir) ~= "string" or dir == "" then return nil end
      return dir .. "/" .. name
    end
    -- Renames inside the save folder; false when that is not possible here.
    local function move(from, to)
      local a, b = realPath(from), realPath(to)
      if not a or not b or type(os) ~= "table" or not os.rename then return false end
      pcall(os.remove, b)
      local ok, done = pcall(os.rename, a, b)
      return ok and done and true or false
    end

    -- The table a save file gives, or nil and why not. LÖVE raises an error for a file
    -- that does not compile, so every step is protected.
    local function read(name)
      local ok, chunk, err = pcall(origLoad, name)
      if not ok then return nil, tostring(chunk) end
      if not chunk then return nil, tostring(err) end
      local ran, result = pcall(chunk)
      if not ran then return nil, tostring(result) end
      if type(result) ~= "table" then return nil, "it does not hold a save" end
      return result
    end

    -- What the game gets when no copy of `name` loads. For a profile, a new one made as
    -- the game makes it at first start: its newProfile fills the global `player`, so the
    -- new table stands in as `player` for the call and the profile in play is left alone.
    -- Any other file gets an empty table.
    local function fresh(name)
      local t = {}
      local profile = name:match("^saveMNB") or name:match("^backUp")
      if profile and type(newProfile) == "function" then
        local live = rawget(_G, "player")
        rawset(_G, "player", t)
        pcall(newProfile, t)
        rawset(_G, "player", live)
      end
      return t
    end

    -- A save whose last write never reached the disk: only its previous copy is there.
    local function onlyPrev(name)
      return ROTATED[name] == true and origInfo ~= nil and not exists(name) and exists(name .. ".prev")
    end

    if origInfo then
      fs.getInfo = function(name, ...)
        local info = origInfo(name, ...)
        if info == nil and onlyPrev(name) then return origInfo(name .. ".prev", ...) end
        return info
      end
    end

    fs.load = function(name, ...)
      if not guarded(name) or not (exists(name) or onlyPrev(name)) then
        return origLoad(name, ...)
      end
      local save, why = nil, "the file is missing"
      if exists(name) then
        save, why = read(name)
        if save then
          return function() return save end
        end
        if move(name, name .. ".damaged") then
          print("[QM] Save guard: the damaged file was kept as " .. name .. ".damaged")
        end
      end
      print("[QM] Save guard: " .. name .. " could not be loaded: " .. tostring(why))
      local tries = {}
      if name == "saveMNB3.lua" then tries[#tries + 1] = "backUp.lua" end
      if ROTATED[name] then tries[#tries + 1] = name .. ".prev" end
      for _, other in ipairs(tries) do
        if exists(other) then
          local older, whyNot = read(other)
          if older then
            print("[QM] Save guard: using " .. other .. " in its place")
            guard.used[#guard.used + 1] = { file = name, from = other, why = why }
            return function() return older end
          end
          print("[QM] Save guard: " .. other .. " could not be loaded either: " .. tostring(whyNot))
        end
      end
      print("[QM] Save guard: no usable copy of " .. name .. "; starting it fresh")
      guard.used[#guard.used + 1] = { file = name, from = nil, why = why }
      local made = fresh(name)
      return function() return made end
    end

    -- Whether the file on disk starts like a save (and not with zero bytes or nothing).
    local function looksLikeSave(name)
      if not exists(name) or not fs.read then return false end
      local ok, head = pcall(fs.read, name, 32)
      return ok and type(head) == "string" and head:find("^%s*[%a_]") ~= nil
    end

    fs.write = function(name, ...)
      if ROTATED[name] and looksLikeSave(name) then
        if not move(name, name .. ".prev") then
          local ok, all = pcall(fs.read, name)
          if ok and type(all) == "string" then pcall(origWrite, name .. ".prev", all) end
        end
      end
      return origWrite(name, ...)
    end
  end
end
