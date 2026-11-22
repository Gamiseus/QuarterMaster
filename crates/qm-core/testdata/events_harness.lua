-- Runs QuarterMaster's events runtime and save guard outside the game, with LÖVE's file
-- functions stubbed. Called by qm-core's events tests:
--   lua events_harness.lua <path to events_runtime.lua> <path to save_guard.lua>
-- Prints FAIL lines and exits non-zero when a check fails.

local failures = 0
local function check(cond, msg)
  if not cond then
    failures = failures + 1
    print("FAIL: " .. msg)
  end
end
local function eq(got, want, msg)
  check(got == want, msg .. " (got " .. tostring(got) .. ", want " .. tostring(want) .. ")")
end

-- A save folder in memory.
local files = {}
love = {
  filesystem = {
    load = function(name)
      local text = files[name]
      if not text then return nil, "no file" end
      return function() return { from = name } end
    end,
    write = function(name, data) files[name] = data; return true end,
    getInfo = function(name) return files[name] and {} or nil end,
    read = function(name, len) local t = files[name]; return t and (len and t:sub(1, len) or t) or nil end,
  },
}

dofile(arg[1])
dofile(arg[2])
local E = QM.Events
check(type(E) == "table", "QM.Events exists")

-- on, emit, off.
local seen = {}
local id = E.on("example_mod:ping", function(a, b) seen[#seen + 1] = a .. b end, "example_mod")
E.emit("example_mod:ping", "x", "y")
eq(seen[1], "xy", "a handler gets the event's values")
check(E.off(id), "off() removes the handler")
E.emit("example_mod:ping", "x", "y")
eq(#seen, 1, "a removed handler is not called")
eq(E.on("bad", "not a function"), nil, "on() refuses a handler that is not a function")

-- A failing handler: logged, kept while it recovers, removed after three failures in a row.
local calls, failing = 0, true
E.on("tick_test", function() calls = calls + 1; if failing then error("boom") end end, "broken_mod")
local fine = 0
E.on("tick_test", function() fine = fine + 1 end, "good_mod")
E.emit("tick_test")
E.emit("tick_test")
failing = false
E.emit("tick_test")
failing = true
E.emit("tick_test")
E.emit("tick_test")
eq(calls, 5, "two failures then a success reset the count")
E.emit("tick_test")
eq(calls, 6, "the third failure in a row")
E.emit("tick_test")
eq(calls, 6, "after three failures in a row the handler is removed")
eq(fine, 7, "the other handler keeps running")

-- Battles and waves, watched once a frame.
local log = {}
for _, name in ipairs({ "battle_start", "battle_end", "wave_start", "game_saved", "game_loaded" }) do
  E.on(name, function(v) log[#log + 1] = name .. (v ~= nil and ("=" .. tostring(v)) or "") end)
end
E.tick()
eq(#log, 0, "no player: nothing happens")
player = { gameState = 1, currentWave = 0 }
E.tick()
eq(#log, 0, "in the menus: nothing happens")
player.gameState = 0
E.tick()
eq(log[1], "battle_start", "a battle starts")
E.tick()
player.currentWave = 1
E.tick()
eq(log[2], "wave_start=1", "a wave starts")
player.currentWave = 3
E.tick()
eq(log[3], "wave_start=3", "a jump of waves is one event with the new wave")
player.gameState = 1
E.tick()
eq(log[4], "battle_end", "the battle ends")
eq(#log, 4, "and nothing else")

-- Saves through the save guard.
love.filesystem.write("saveMNB3.lua", "return {}")
eq(log[5], "game_saved=saveMNB3.lua", "a save file written")
love.filesystem.write("saveModSettings.lua", "return {}")
eq(#log, 5, "other files are not saves")
local chunk = love.filesystem.load("saveMNB3.lua")
player = chunk()
eq(#log, 5, "a read save is in play only after the read")
E.tick()
eq(log[6], "game_loaded=saveMNB3.lua", "the next frame says it was loaded")

if failures > 0 then
  print(failures .. " check(s) failed")
  os.exit(1)
end
print("ok")
