-- Runs QuarterMaster's skills runtime outside the game, with the game's skill functions stubbed.
-- Called by qm-core's ordnance tests:
--   lua skills_harness.lua <events_runtime.lua> <skills_runtime.lua>
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

-- The game, as far as skills touch it.
local function img(name) return { name = name, getWidth = function() return 46 end, getHeight = function() return 46 end } end
for _, class in ipairs({ "rifle", "light", "medic", "sniper" }) do
  for n = 1, 3 do
    _G[class .. "_skill_" .. n] = img(class .. n)
    _G[class .. "_skillText_" .. n] = class .. " skill " .. n
  end
end
flare_icon, medkit_icon, para_icon, whiteCircle = img("flare"), img("medkit"), img("para"), img("circle")
function randNbr(n) return 1 end
function convertedId(id) return id end
function tellTime(t) return t .. "s left" end
local drawn = {}
love = {
  graphics = {
    draw = function(i, x, y) drawn[#drawn + 1] = { name = i and i.name, x = x, y = y } end,
    setColor = function() end,
    newImage = function() return img("blank") end,
  },
  image = { newImageData = function() return {} end },
  math = { colorFromBytes = function(...) return ... end },
}
Button = { new = function(x, y, w, h, label, status, over, text)
  return { X = x, Y = y, label = label, mouseOverText = text or "" } end }
player = { screenx = 0, screeny = 0, deployedTroops = {
  { spec = "rifle", health = 100, playerControlled = 1, skillUpgrade = 3, skill1Timer = 0, skill2Timer = 0, skill3Timer = 0 },
  { spec = "light", health = 100, playerControlled = 1, skillUpgrade = 1, skill1Timer = 0, skill2Timer = 0, skill3Timer = 0 },
  { spec = "medic", health = 100, playerControlled = 1, skillUpgrade = 3, skill1Timer = 0, skill2Timer = 0, skill3Timer = 0 },
  { spec = "sniper", health = 100, playerControlled = 1, skillUpgrade = 3, skill1Timer = 7, skill2Timer = 0, skill3Timer = 0 },
} }
local gameCalls = {}
function skillClick(n)
  local d = player.deployedTroops[convertedId(specID)]
  gameCalls[#gameCalls + 1] = d.spec .. ":" .. n
  if d.spec == "rifle" and n == 1 then d.skill1Timer = 120 end
  if d.spec == "rifle" and n == 2 then mousePrompt = 3 end
  if d.spec == "light" and n == 3 then d.skill3Timer = 60 end
end
function displayAbilityMenuz(id)
  local d = player.deployedTroops[id]
  for n = 1, math.min(3, d.skillUpgrade) do
    love.graphics.draw(_G[d.spec .. "_skill_" .. n], 0, ({ 632, 532, 432 })[n])
  end
end
function displaySkillMenu(id)
  local d = player.deployedTroops[id]
  if d.skillUpgrade < 3 then love.graphics.draw(_G[d.spec .. "_skill_" .. (d.skillUpgrade + 1)], 0, 632) end
end
function skillTimerUpdate()
  for _, d in ipairs(player.deployedTroops) do
    for n = 1, 3 do
      if d["skill" .. n .. "Timer"] > 0 then d["skill" .. n .. "Timer"] = d["skill" .. n .. "Timer"] - 1 end
    end
  end
end
-- The game's skill buttons for the selected soldier, then QuarterMaster's hook.
local function makeButtons(id)
  specID = id
  buttons = {}
  local d = player.deployedTroops[id]
  for n = 3, 1, -1 do
    if d.skillUpgrade > n - 1 then
      local t = d["skill" .. n .. "Timer"]
      buttons[#buttons + 1] = Button.new(60 * id, ({ 610, 510, 400 })[n], 45, 45, "skill_" .. n, "active", true,
        t <= 0 and _G[d.spec .. "_skillText_" .. n] or tellTime(t))
    end
  end
  QM.Skills.addButton(id)
  local byLabel = {}
  for _, b in ipairs(buttons) do byLabel[b.label] = b end
  return byLabel
end
local function drawnNames()
  local t = {}
  for _, d in ipairs(drawn) do t[d.name] = d.y end
  return t
end

dofile(arg[1])
dofile(arg[2])
local S = QM.Skills

-- What the checks below set up, before love.load.
-- Edit a game skill: its icon, text and cooldown; the game's own action stays.
S.set({ class = "rifle", slot = 1, name = "Flare", description = "light up the night", icon = "flare_icon", cooldown = 30 })
-- Replace a game skill with code.
local used = 0
S.set({ class = "rifle", slot = 2, use = function(d) used = used + 1 end, cooldown = 45 })
-- Another class's game skill on a slot.
S.set({ class = "rifle", slot = 3, game = "light:3" })
-- Remove a game skill; give another an earlier level.
S.remove("light", 2)
S.set({ class = "light", slot = 3, unlock = 1 })
-- Add new skills, as many as wanted, some at levels past 3.
local s4 = S.add({ class = "medic", name = "Medkit drop", use = function() end, icon = "medkit_icon" })
local s5 = S.add({ class = "medic", name = "Airborne", use = function() end, icon = "para_icon", unlock = 4 })
local s6 = S.add({ class = "medic", name = "Last stand", use = function() end, unlock = 5, cooldown = 10 })
local s8 = S.set({ class = "medic", slot = 8, game = 1, unlock = 3 })
eq(s4 and s4.slot, 4, "add takes the next free slot")
eq(s5 and s5.slot, 5, "and the one after")
eq(s6 and s6.slot, 6, "and the one after that")
eq(s8 and s8.game.slot, 1, "a new slot can be a game skill of the class")
-- Borrow a skill into another slot (its timer moves), and one that waits for a click.
S.set({ class = "sniper", slot = 5, game = "rifle:1", cooldown = 25 })
S.set({ class = "sniper", slot = 2, game = "rifle:2", cooldown = 99 })

-- What is refused.
eq(S.set({ class = "rifle" }), nil, "a skill needs a slot")
eq(S.set({ class = "rifle", slot = 4 }), nil, "a new slot needs something to do")
eq(S.set({ class = "rifle", slot = 1, use = function() end, remove = true }), nil, "one action at a time")
eq(S.set({ class = "rifle", slot = 1, game = "light:9" }), nil, "a game skill is 1 to 3")
eq(S.set({ slot = 1 }), nil, "a skill needs a class")

S.install()

-- Edits reach the game's globals: the icon, the text for the button and the promotion.
eq(rifle_skill_1, flare_icon, "an edited icon is the class's")
eq(rifle_skillText_1, "Flare: light up the night", "and so is its text")
eq(rifle_skill_3, light_skill_3, "a borrowed skill shows its own icon")
eq(rifle_skillText_3, "light skill 3", "and its text")
eq(light_skillText_1, "light skill 1 / light skill 3", "a skill level lists what it brings")
eq(light_skillText_3, "", "a level that brings nothing says so")
eq(medic_skillText_4, "Airborne", "a level past 3 has its text")
eq(S.maxLevel("medic"), 5, "and the class goes that high")

-- Edit: the game's own action, with its cooldown.
specID = 1
skillClick(1)
eq(gameCalls[#gameCalls], "rifle:1", "an edited skill is the game's own")
eq(player.deployedTroops[1].skill1Timer, 30, "with its own cooldown")

-- Replace with code.
local before = #gameCalls
skillClick(2)
eq(used, 1, "a replaced skill runs its code")
eq(#gameCalls, before, "and not the game's")
eq(player.deployedTroops[1].skill2Timer, 45, "with its cooldown")
skillClick(2)
eq(used, 1, "not while it cools down")

-- Borrow: the light infantry's charge on a rifleman.
skillClick(3)
eq(gameCalls[#gameCalls], "light:3", "a borrowed skill is the other class's")
eq(player.deployedTroops[1].spec, "rifle", "and the soldier stays a rifleman")
eq(player.deployedTroops[1].skill3Timer, 60, "with that skill's cooldown")

-- Remove and earlier levels: the light infantry at level 1.
specID = 2
local b = makeButtons(2)
check(b.skill_1, "a level 1 skill keeps the game's button")
eq(b.skill_2, nil, "a removed skill has none")
eq(b.skill_3 and b.skill_3.Y, 400, "a skill given level 1 has its button there")
before = #gameCalls
skillClick(2)
eq(#gameCalls, before, "a removed skill does nothing")
drawn = {}
displayAbilityMenuz(2)
local d = drawnNames()
eq(d.light1, 632, "the game draws its own icon")
eq(d.light3, 432, "and QuarterMaster the one it moved")
skillClick(3)
eq(gameCalls[#gameCalls], "light:3", "a moved skill is still the game's own")
eq(player.deployedTroops[2].skill3Timer, 60, "with the game's cooldown")
player.deployedTroops[2].skillUpgrade = 3
drawn = {}
displayAbilityMenuz(2)
d = drawnNames()
eq(d.light2, nil, "a removed skill's icon is not drawn")
local blankAt = {}
for _, e in ipairs(drawn) do if e.name == "blank" then blankAt[e.y] = true end end
check(blankAt[532], "the game draws nothing there")

-- New skills: buttons above the third, columns past the seventh, levels past 3.
specID = 3
local m = player.deployedTroops[3]
b = makeButtons(3)
eq(b.skill_4 and b.skill_4.Y, 300, "slot 4 above the third")
eq(b.skill_4 and b.skill_4.mouseOverText, "Medkit drop", "with its text")
eq(b.skill_5, nil, "a level 4 skill waits for level 4")
eq(b.skill_8 and b.skill_8.X, 180 + 55, "slot 8 starts a column to the right")
eq(b.skill_8 and b.skill_8.Y, 610, "at the bottom")
eq(b.skill_3 and b.skill_3.mouseOverText, "medic skill 3", "the game's buttons keep their own text")
buttons = {}
S.addSelect(3)
eq(buttons[1] and buttons[1].label, "skillSelect", "promotion offers level 4")
eq(buttons[1] and buttons[1].mouseOverText, "Airborne", "with what it brings")
drawn = {}
displaySkillMenu(3)
eq(drawnNames().para, 632, "and its icon")
m.skillUpgrade = 5
buttons = {}
S.addSelect(3)
eq(#buttons, 0, "no level past the class's last")
b = makeButtons(3)
eq(b.skill_6 and b.skill_6.Y, 100, "slot 6 at level 5")
drawn = {}
displayAbilityMenuz(3)
eq(drawnNames().circle, 132, "a new skill with no icon shows the game's plain circle")
m.skill4Timer = 3
skillTimerUpdate()
eq(m.skill4Timer, 2, "new skills cool down with the game's")
check(S.place(7) == 0 and select(2, S.place(7)) == 0, "slot 7 at the top")

-- Borrowed into another slot: the timer moves there, and the game's is put back.
specID = 4
local sn = player.deployedTroops[4]
local own1 = sn.skill1Timer
skillClick(5)
eq(gameCalls[#gameCalls], "rifle:1", "a sniper fires the rifleman's skill 1")
eq(sn.skill5Timer, 25, "on his slot 5's cooldown")
eq(sn.skill1Timer, own1, "and his own skill 1 is as it was")
-- A borrowed skill that waits for a click: its cooldown comes when the game sets it.
mousePrompt = 0
skillClick(2)
eq(mousePrompt, 3, "the rifleman's rifle grenade is armed for the sniper")
sn.skill2Timer = 180
skillTimerUpdate()
eq(sn.skill2Timer, 99, "and takes the slot's cooldown when fired")

-- Set later (after love.load): applied at once.
S.set({ class = "sniper", slot = 1, name = "Steady aim" })
eq(sniper_skillText_1, "Steady aim", "a skill set later reaches the game's text at once")

if failures > 0 then
  print(failures .. " check(s) failed")
  os.exit(1)
end
print("ok")
