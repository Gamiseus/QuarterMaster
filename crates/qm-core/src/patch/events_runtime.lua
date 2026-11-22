-- QuarterMaster: game events (QM.Events)
-- A mod reacts to what happens in the game without patching the game's code:
--   QM.Events.on(name, handler[, owner]) -> id     runs handler(...) when the event happens
--   QM.Events.off(id)                              stops it
--   QM.Events.emit(name, ...)                      sends an event (a mod's own: "<mod_id>:<name>")
-- Built-in events, found by watching the game once a frame:
--   battle_start, battle_end    a battle (or mission) begins, and ends back in the menus
--   wave_start(wave)            a new wave in a wave mode (classic and the like)
--   game_saved(file)            the game wrote a save file (saveMNB3.lua, saveMNB2.lua, ...)
--   game_loaded(file)           the game read a save file into play
-- Every handler runs protected. An error is logged with the handler's owner; a handler that
-- fails three times in a row is removed, so one broken mod never floods the log.
do
  _G.QM = _G.QM or {}
  if not _G.QM.Events then
    local E = { handlers = {}, nextId = 0, MAX_FAILURES = 3 }
    _G.QM.Events = E

    -- Who a handler belongs to, for the log: the owner given, else where it was written.
    local function ownerOf(fn)
      if debug and debug.getinfo then
        local ok, info = pcall(debug.getinfo, fn, "S")
        if ok and info then
          return tostring(info.short_src) .. ":" .. tostring(info.linedefined)
        end
      end
      return "a handler"
    end

    function E.on(name, fn, owner)
      if type(name) ~= "string" or type(fn) ~= "function" then
        print("[QM][Events] on() needs an event name and a function")
        return nil
      end
      E.nextId = E.nextId + 1
      E.handlers[#E.handlers + 1] = {
        id = E.nextId,
        name = name,
        fn = fn,
        owner = type(owner) == "string" and owner or ownerOf(fn),
        failures = 0,
      }
      return E.nextId
    end

    function E.off(id)
      for i = #E.handlers, 1, -1 do
        if E.handlers[i].id == id then
          table.remove(E.handlers, i)
          return true
        end
      end
      return false
    end

    function E.emit(name, ...)
      -- A copy: a handler may add or remove handlers while it runs.
      local list = {}
      for _, h in ipairs(E.handlers) do
        if h.name == name then list[#list + 1] = h end
      end
      for _, h in ipairs(list) do
        local ok, err = pcall(h.fn, ...)
        if ok then
          h.failures = 0
        else
          h.failures = h.failures + 1
          print("[QM][Events] " .. name .. ": " .. h.owner .. " failed: " .. tostring(err))
          if h.failures >= E.MAX_FAILURES then
            E.off(h.id)
            print("[QM][Events] " .. name .. ": " .. h.owner .. " failed " .. E.MAX_FAILURES
              .. " times in a row and was removed")
          end
        end
      end
    end

    -- Called by the save guard.
    function E.saved(file)
      E.emit("game_saved", file)
    end

    -- A save was read; the game puts it in play after the read, so the event waits a frame.
    function E.loading(file)
      E.pendingLoad = file
    end

    -- Once a frame (QuarterMaster wraps love.update after love.load).
    function E.tick()
      if E.pendingLoad then
        local file = E.pendingLoad
        E.pendingLoad = nil
        E.emit("game_loaded", file)
      end
      local p = rawget(_G, "player")
      if type(p) ~= "table" then return end
      local state = p.gameState
      if state ~= E.lastState then
        local was = E.lastState
        E.lastState = state
        if state == 0 then
          E.lastWave = tonumber(p.currentWave)
          E.emit("battle_start")
        elseif was == 0 then
          E.lastWave = nil
          E.emit("battle_end")
        end
      end
      if state == 0 then
        local wave = tonumber(p.currentWave)
        if wave and E.lastWave and wave > E.lastWave then
          E.lastWave = wave
          E.emit("wave_start", wave)
        elseif wave then
          E.lastWave = wave
        end
      end
    end
  end
end
