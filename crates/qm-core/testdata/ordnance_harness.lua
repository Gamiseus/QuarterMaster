-- Runs QuarterMaster's grenade runtime outside the game, with the game's functions stubbed.
-- Called by qm-core's grenade tests:
--   lua ordnance_harness.lua <events_runtime.lua> <skills_runtime.lua> <ordnance_runtime.lua> [<faction_runtime.lua>]
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

-- The game (LuaJIT) has math.atan2; Lua 5.3 and later spell it math.atan.
math.atan2 = math.atan2 or math.atan

-- The game, as far as grenades touch it.
local played = {}
local function snd(name)
  return { clone = function() return { setVolume = function() end, setPitch = function() end,
    play = function() played[#played + 1] = name end } end }
end
fragSound1, fragSound2, fragSound3, tossFrag = snd("fragSound1"), snd("fragSound2"), snd("fragSound3"), snd("tossFrag")
fragReady1 = snd("fragReady1")
neg1 = snd("neg1")
limonka = { getWidth = function() return 40 end, getHeight = function() return 30 end }
function randNbr(n) return 1 end
function stepScale() return 1 end
function distanceCheck(x1, y1, x2, y2) return math.sqrt((x2 - x1) ^ 2 + (y2 - y1) ^ 2) end
function hasPerk() return false end
function convertedId(id) return id end
function insideBunker() return false end
function coveredBuilding() return false end
local craters = 0
function createCrater() craters = craters + 1 end
function addParticle(p, typez, x, y, r)
  p.typez, p.sprite, p.x, p.y, p.rotationz = "fragGrenade", "frag", x, y, r
end
local exploded = {}
function createExplosion(tag, x, y) exploded[#exploded + 1] = tag end
function skillClick(skill) played[#played + 1] = "game skill " .. skill end
tooFarTxt = "too far"
player = {
  screenx = 0, screeny = 0, particles = {}, explosions = {}, textCue = {}, tension = 0,
  deployedTroops = {
    { spec = "light", health = 100, death = 0, pinned = 0, x = 0, y = 0, faction = "allied",
      skill2Timer = 0, frags = 3, grenadeType = 0 },
    { spec = "rifle", health = 100, x = 0, y = 0, faction = "axis", frags = 2, grenadeType = 1,
      fragx = 50, fragy = 0 },
  },
}

dofile(arg[1])
dofile(arg[2])
dofile(arg[3])
local G, X = QM.Ordnance, QM.Explosions

-- An explosion on a preset, then a grenade with it on the light infantry's frag skill.
local blastLog = {}
QM.Events.on("ordnance_blast", function(id) blastLog[#blastLog + 1] = id end)
local e = X.register({ id = "example_mod:limonka_blast", based_on = "frag", fragRadius = 150 })
eq(e.fragRadius, 150, "an explosion takes the number given")
eq(e.blastRadius, 75, "and the preset's for the rest")
eq(X.register({ id = "x", based_on = "mineExplosion" }).atDamage, 100, "a preset by the game's tag")
local g = G.register({
  id = "example_mod:limonka", sprite = "limonka", explosion = "example_mod:limonka_blast",
  range = 500, cooldown = 90, skill = { class = "light", slot = 2 }, replaces_frag = "axis",
})
check(g.type > 14 and g.prompt > 109, "numbers above the game's")
check(g.type ~= g.prompt, "two different numbers")
local g2 = G.register({ id = "other_mod:smoke", sprite = "limonka" })
check(g2.type ~= g.type and g2.prompt ~= g.prompt, "a second grenade gets its own numbers")
eq(G.register({ id = "example_mod:limonka", sprite = "limonka" }), g, "registering twice keeps the first")
eq(G.register({ id = "bad" }), nil, "a grenade needs a sprite")

-- The skill: the light infantry's second skill arms it; other skills stay the game's.
G.install()
specID = 1
skillClick(2)
eq(mousePrompt, g.prompt, "the skill arms the grenade")
eq(mousePrompter, 1, "for the selected soldier")
skillClick(1)
eq(played[#played], "game skill 1", "another skill is the game's own")

-- The click: too far, then in range.
mousePrompt, mousePrompter = g.prompt, 1
G.armedSlot = 2
check(G.click(600, 0), "the click is ours")
eq(player.deployedTroops[1].state, "idle", "too far: no throw")
eq(mousePrompt, 0, "and the prompt is over")
eq(player.textCue[1] and player.textCue[1].name, "too far", "too far is shown")
eq(G.click(1, 1), false, "a click with no grenade armed is the game's")
mousePrompt, mousePrompter = g.prompt, 1
G.armedSlot = 2
G.click(100, 0)
local dude = player.deployedTroops[1]
eq(dude.state, "fragThrow1", "in range: the soldier throws")
eq(dude.grenadeType, g.type, "this grenade")
eq(dude.skill2Timer, 90, "the skill's cooldown starts")

-- The throw, from the soldier's state machine.
check(G.throwFor(1), "the throw is ours")
local part = player.particles[#player.particles]
eq(part.sprite, "limonka", "it flies with its own sprite")
eq(part.xoffset, 20, "centred on it")
eq(part.qmOrdnance, g.id, "and knows what it is")
eq(dude.frags, 2, "it uses a frag")
eq(part.spin, nil, "without spin it turns as the game's frag")

-- spin: its own turn rate, or 0 to fly pointing the way it was thrown.
local stick = G.register({ id = "example_mod:stick", sprite = "limonka", spin = 0 })
local twirl = G.register({ id = "example_mod:twirl", sprite = "limonka", spin = 0.3 })
eq(G.register({ id = "example_mod:spin_rifle", kind = "rifle", sprite = "limonka", spin = 0 }).spin, nil,
  "spin is for a thrown grenade")
G.throw(1, stick, 300, 0)
local sp = player.particles[#player.particles]
eq(sp.spin, 0, "spin 0 does not turn")
eq(sp.innerRotation, sp.rotationz, "and points the way it was thrown")
G.throw(1, twirl, 300, 0)
eq(player.particles[#player.particles].spin, 0.3, "a spin of its own")
eq(player.particles[#player.particles].qmImpact, nil, "a grenade bounces unless impact is set")

-- impact: it goes off where it lands.
local at = G.register({ id = "example_mod:at", sprite = "limonka", impact = true })
check(at.impact, "an impact grenade")
eq(G.register({ id = "example_mod:impact_rifle", kind = "rifle", sprite = "limonka", impact = true }).impact, nil,
  "impact is for a thrown grenade")
G.throw(1, at, 300, 0)
eq(player.particles[#player.particles].qmImpact, true, "its particle goes off where it lands")

-- The AI's frag, swapped for the Axis.
check(G.throwFor(2), "an Axis frag is swapped")
eq(player.deployedTroops[2].grenadeType, g.type, "the frag branch is skipped")
player.deployedTroops[2].grenadeType = 1
player.deployedTroops[2].faction = "allied"
eq(G.throwFor(2), false, "an Allied frag stays the game's")

-- The blast and the explosion.
check(G.blast(part, 10, 20), "the blast is ours")
eq(exploded[#exploded], "example_mod:limonka_blast", "its explosion goes off")
eq(player.tension, 40, "the tension rises as for a frag")
eq(blastLog[1], g.id, "ordnance_blast is sent")
eq(G.blast({ sprite = "frag" }, 0, 0), false, "the game's own frag is not ours")
local sound = X.make("example_mod:limonka_blast", 10, 20, nil)
local ex = player.explosions[#player.explosions]
eq(ex.fragRadius, 150, "the explosion has its numbers")
eq(ex.tag, "example_mod:limonka_blast", "and its tag")
eq(craters, 1, "and a crater")
check(sound and sound.play, "and a sound to play")

-- A failing on_blast is logged and the game goes on.
local g3 = G.register({ id = "broken", sprite = "limonka", on_blast = function() error("boom") end })
check(G.blast({ qmOrdnance = "broken" }, 0, 0), "a failing on_blast still counts as handled")

-- QM.Skills: a fourth skill, a grenade's and one that acts at once.
local drawn, gameDrew = nil, false
love = { graphics = { draw = function(img, x, y) drawn = { x = x, y = y } end, setColor = function() end },
  math = { colorFromBytes = function(...) return ... end } }
Button = { new = function(x, y, w, h, label, status, over, text) return { X = x, Y = y, label = label, text = text } end }
function tellTime(t) return t .. "s left" end
function displayAbilityMenuz(id) gameDrew = true end
function skillTimerUpdate() end
local S = QM.Skills
local light = player.deployedTroops[1]
light.skillUpgrade, light.playerControlled, light.skill4Timer = 2, 1, 0
G.giveSkill("light", 4, g)
S.install()
eq(S.get("light", 4) and S.get("light", 4).ordnance, g, "a grenade on slot 4 is the class's fourth skill")
buttons = {}
specID = 1
S.addButton(1)
eq(#buttons, 0, "no button before the skill level it comes with")
light.skillUpgrade = 3
S.addButton(1)
eq(buttons[1] and buttons[1].label, "skill_4", "then a skill_4 button")
eq(buttons[1] and buttons[1].Y, 300, "above the third")
mousePrompt = 0
skillClick(4)
eq(mousePrompt, g.prompt, "skill 4 arms the grenade")
mousePrompt, mousePrompter = g.prompt, 1
G.armedSlot = 4
G.click(100, 0)
eq(light.skill4Timer, 90, "the fourth skill's cooldown starts")
skillTimerUpdate()
eq(light.skill4Timer, 89, "and counts down with the game's skill timers")
displayAbilityMenuz(1)
check(gameDrew, "the game's skills are drawn")
eq(drawn and drawn.y, 332, "and the fourth icon above the third")
local used = 0
S.add({ class = "medic", name = "Patch up", cooldown = 30, use = function(dude) used = used + 1 end })
player.deployedTroops[2].spec, player.deployedTroops[2].skillUpgrade = "medic", 3
player.deployedTroops[2].health, player.deployedTroops[2].playerControlled = 100, 1
specID = 2
skillClick(4)
eq(used, 1, "a fourth skill that acts at once")
eq(player.deployedTroops[2].skill4Timer, 30, "with its cooldown")
skillClick(4)
eq(used, 1, "not again while it cools down")
eq(S.add({ class = "scout" }), nil, "a fourth skill needs a grenade or a use function")

-- Rifle grenades: fired through the game's rifling, with their own sprite and explosion.
-- The game's rifling, as far as it matters here: it fires only while the soldier aims, and on
-- the Allied side it starts skill 2's cooldown.
function rifling(dude, x, y)
  if dude.state ~= "aim" then return end
  local p = { typez = "rifleGrenade", sprite = "rifleGrenade", x = dude.x, y = dude.y, xoffset = 14, yoffset = 12 }
  table.insert(player.particles, p)
  p.targetx, p.targety = x, y
  if dude.faction == "allied" then dude.skill2Timer = 120 end
  dude.rifleGrenade = (dude.rifleGrenade or 0) - 1
  dude.state = "idle"
end
rifleGrenadeEquip = snd("rifleGrenadeEquip")
local thrownLog = {}
QM.Events.on("ordnance_fired", function(id) thrownLog[#thrownLog + 1] = id end)
local vog = G.register({ id = "example_mod:vog", kind = "rifle", sprite = "limonka",
  skill = { class = "rifle", slot = 2 }, replaces_rifle_grenade = "axis" })
eq(vog.kind, "rifle", "a rifle grenade")
eq(vog.explosion, "rifleGrenadeExplosion", "goes off as the game's unless set")
eq(vog.cooldown, 180, "with the game's rifle grenade cooldown")
eq(vog.tension, nil, "and leaves the tension")
eq(vog.uses, nil, "the game's rifling counts the soldier's rifle grenades")
eq(G.register({ id = "odd", kind = "catapult", sprite = "limonka" }).kind, "thrown", "an unknown kind is thrown")
check(G.rifleInstalled, "registered after love.load, it wraps rifling at once")

-- The AI: the game's delayed action calls _G["rifling"].
local axisDude = { faction = "axis", state = "aim", x = 0, y = 0, rifleGrenade = 2 }
_G["rifling"](axisDude, 300, 0)
local rp = player.particles[#player.particles]
eq(rp.qmOrdnance, vog.id, "an Axis rifle grenade is swapped")
eq(rp.sprite, "limonka", "and flies with its sprite")
eq(rp.xoffset, 20, "centred on it")
eq(axisDude.rifleGrenade, 1, "it uses one of his rifle grenades")
eq(thrownLog[#thrownLog], vog.id, "ordnance_fired is sent")
rifling({ faction = "allied", state = "aim", x = 0, y = 0 }, 10, 0)
eq(player.particles[#player.particles].qmOrdnance, nil, "an Allied rifle grenade stays the game's")
local count = #player.particles
rifling({ faction = "axis", state = "idle", x = 0, y = 0 }, 10, 0)
eq(#player.particles, count, "a soldier who stopped aiming fires nothing")

-- The rifle class's second skill.
player.deployedTroops[3] = { spec = "rifle", health = 100, death = 0, x = 0, y = 0, faction = "allied",
  playerControlled = 1, skill2Timer = 0, xp = 100, rifleGrenade = 1, weapon = { rgCapable = 1 } }
local rifleman = player.deployedTroops[3]
specID, mousePrompt = 3, 0
skillClick(2)
eq(mousePrompt, vog.prompt, "the rifle class's second skill arms the rifle grenade")
eq(rifleman.state, "aim", "and the soldier raises his rifle")
G.click(200, 0)
rp = player.particles[#player.particles]
eq(rp.qmOrdnance, vog.id, "the click fires it")
eq(rifleman.skill2Timer, 180, "and the skill's cooldown starts")
eq(mousePrompt, 0, "the prompt is over")
rifleman.skill2Timer, rifleman.weapon.rgCapable = 0, 0
specID = 3
skillClick(2)
count = #player.particles
G.click(200, 0)
eq(#player.particles, count, "a rifle that takes no rifle grenades fires nothing")
eq(rifleman.skill2Timer, 0, "and the cooldown does not start")
rifleman.weapon.rgCapable = 1
specID = 3
skillClick(2)
count = #player.particles
G.click(5000, 0)
eq(#player.particles, count, "too far: nothing fired")

-- Its impact (the hook in the game's two impact branches).
player.tension = 7
check(G.blast(rp, rp.x, rp.y), "the rifle grenade's impact is ours")
eq(exploded[#exploded], "rifleGrenadeExplosion", "its explosion goes off")
eq(player.tension, 7, "and the tension stays")

-- A rifle grenade as a fourth skill: its own cooldown, and skill 2's is left alone.
local vog4 = G.register({ id = "example_mod:vog4", kind = "rifle", sprite = "limonka", cooldown = 50,
  skill = { class = "sniper", slot = 4 } })
player.deployedTroops[4] = { spec = "sniper", health = 100, death = 0, x = 0, y = 0, faction = "allied",
  playerControlled = 1, skillUpgrade = 3, skill2Timer = 33, skill4Timer = 0, xp = 50,
  weapon = { rgCapable = 1 } }
specID, mousePrompt = 4, 0
skillClick(4)
eq(mousePrompt, vog4.prompt, "the fourth skill arms its rifle grenade")
G.click(100, 0)
eq(player.particles[#player.particles].qmOrdnance, vog4.id, "and fires it")
eq(player.deployedTroops[4].skill4Timer, 50, "with the fourth skill's cooldown")
eq(player.deployedTroops[4].skill2Timer, 33, "skill 2's cooldown is left as it was")

-- Mortars: the soldier's own mortar, or his side's swap, changes each round the game queues.
local floated
function floatyText(dude, text) floated = text end
function insideBuilding() return false end
function underCamNet() return false end
function underTree() return false end
noUnderTreeTxt = "not under a tree"
local mortar = G.register({ id = "example_mod:mortar", kind = "mortar", explosion = "hvyExplosion", shots = 4,
  spread = 50, skill = { class = "rifle", slot = 3 }, replaces_mortar = "axis" })
eq(mortar and mortar.kind, "mortar", "a mortar needs no sprite")
eq(mortar.cooldown, 180, "with the game's mortar cooldown")
eq(mortar.tension, nil, "and leaves the tension")
local act = { action = "createExplosion", arg1 = "mortarExplosion", arg2 = 90, arg3 = 110 }
check(G.mortarRound({ faction = "axis", fragx = 100, fragy = 100, mortar = 5 }, act), "an Axis mortar round is swapped")
eq(act.arg1, "hvyExplosion", "to the mortar's explosion")
eq(act.arg2, 51, "with its own spread")
local alliedAct = { arg1 = "mortarExplosion" }
eq(G.mortarRound({ faction = "allied", fragx = 0, fragy = 0, mortar = 3 }, alliedAct), false, "an Allied mortar stays the game's")
eq(alliedAct.arg1, "mortarExplosion", "and its explosion too")
player.deployedTroops[5] = { spec = "rifle", health = 100, death = 0, x = 0, y = 0, faction = "allied",
  playerControlled = 1, skill3Timer = 0, mortar = 0 }
local mm = player.deployedTroops[5]
specID, mousePrompt = 5, 0
skillClick(3)
eq(mousePrompt, mortar.prompt, "the rifle class's third skill arms the mortar")
G.click(300, 0)
eq(mm.state, "mortar1", "the soldier sets up the mortar")
eq(mm.mortar, 4, "with its rounds")
eq(mm.skill3Timer, 180, "and the skill's cooldown starts")
local a1 = { arg1 = "mortarExplosion" }
G.mortarRound(mm, a1)
eq(a1.arg1, "hvyExplosion", "his rounds are the mortar's")
mm.mortar = 1
G.mortarRound(mm, { arg1 = "mortarExplosion" })
eq(mm.qmMortar, nil, "after the last round his mortar is the game's again")
function underTree() return true end
mm.skill3Timer, mm.state = 0, "idle"
specID = 5
skillClick(3)
G.click(300, 0)
eq(mm.state, "idle", "under a tree: no mortar")
eq(floated, "not under a tree", "and the game's message")
function underTree() return false end

-- Artillery: a barrage on a spot, with the game's delayed actions' clock.
local gameTicks = 0
function delayUpdate(dt) gameTicks = gameTicks + 1 end
function mapReset() end
player.delayedAction = {}
local function incoming(x, y)
  player.delayedAction[#player.delayedAction + 1] = { action = "createExplosion", arg1 = "artyExplosion", arg2 = x, arg3 = y }
end
germanArtyIncoming, alliedArtyIncoming = incoming, incoming
local shell = X.register({ id = "example_mod:heavy_shell", based_on = "artillery", fragRadius = 150 })
eq(shell.crater, 10, "the artillery preset")
eq(X.register({ id = "hm", based_on = "heavy_mortar" }).atRadius, 75, "the heavy mortar preset")
local barrage = G.register({ id = "example_mod:barrage", kind = "artillery", explosion = "example_mod:heavy_shell",
  shots = 3, delay = 1, interval = 2, name = "Heavy barrage", skill = { class = "officer", slot = 4 },
  replaces_artillery = "axis" })
eq(barrage and barrage.cooldown, 300, "a barrage, with the game's radio call cooldown")
check(G.queueInstalled, "artillery wraps the game's delayUpdate")
local shots = #exploded
check(G.callIn(barrage, 500, 300), "a barrage is called in")
delayUpdate(2.9)
eq(#exploded, shots, "nothing lands before the first far shot")
delayUpdate(5.2)
eq(#exploded, shots + 1, "the first shell lands 5 seconds after its far shot")
eq(exploded[#exploded], "example_mod:heavy_shell", "with the barrage's explosion")
delayUpdate(10)
eq(#exploded, shots + 3, "and then the other two")
check(gameTicks >= 3, "the game's own delayed actions still run")
G.callIn(barrage, 0, 0)
mapReset()
delayUpdate(100)
eq(#exploded, shots + 3, "a new battle drops a barrage in the air")
germanArtyIncoming(10, 20, 1)
eq(player.delayedAction[#player.delayedAction].arg1, "example_mod:heavy_shell", "an Axis barrage's shells are swapped")
alliedArtyIncoming(10, 20, 1)
eq(player.delayedAction[#player.delayedAction].arg1, "artyExplosion", "an Allied barrage stays the game's")
player.deployedTroops[6] = { spec = "officer", health = 100, death = 0, x = 0, y = 0, faction = "allied",
  playerControlled = 1, skillUpgrade = 3, skill4Timer = 0 }
specID, mousePrompt = 6, 0
skillClick(4)
eq(mousePrompt, barrage.prompt, "the officer's fourth skill arms the barrage")
comms_skill_1, comms_skill_2, comms_skill_3 = { name = "radio1" }, { name = "radio2" }, { name = "radio3" }
eq(S.icon("officer", 4), comms_skill_3, "with the game's radio icon when it has none")
G.click(400, 0)
eq(player.deployedTroops[6].skill4Timer, 300, "the click calls it, and the cooldown starts")
eq(#G.queue, 9, "three shells, three steps each")
local exploLog = {}
QM.Events.on("explosion", function(tag) exploLog[#exploLog + 1] = tag end)
X.make("example_mod:heavy_shell", 0, 0, nil)
eq(exploLog[#exploLog], "example_mod:heavy_shell", "a registered explosion sends the explosion event")

-- Air strikes: a plane flies over, then its hits on the spot.
G.queue = {}
player.planes, player.topParticles, player.lootables = {}, {}, {}
function addPlane(p, typez, x, y) p.typez, p.sprite, p.x, p.y = typez, typez, x, y end
local realExplosion = createExplosion
local function run(tag)
  return function(foo)
    createExplosion(tag, foo, 0)
    player.delayedAction[#player.delayedAction + 1] = { action = "createExplosion", arg1 = tag, arg2 = foo, arg3 = 0 }
  end
end
alliedBomb, stukaStrafe = run("bombExplosion"), run("stukaExplosion")
alliedStrafe, p47Strafing, axisStrafe = run("flakExplosion"), run("flakExplosion"), run("flakExplosion")
local bigBomb = X.register({ id = "example_mod:big_bomb", based_on = "bomb", blastRadius = 150 })
eq(bigBomb.crater, 15, "the bomb preset")
eq(X.register({ id = "light_hit", based_on = "strafe" }).fragRadius, 50, "the strafe preset")
local bombers = G.register({ id = "example_mod:bombers", kind = "airstrike", explosion = "example_mod:big_bomb",
  shots = 3, spread = 0, interval = 0.5, delay = 1, plane = "limonka", skill = { class = "comms", slot = 4 },
  replaces_bombing = "axis" })
eq(bombers and bombers.cooldown, 240, "an air strike, with the game's bomber call cooldown")
check(G.airInstalled and G.queueInstalled, "it wraps the game's runs and the queue")
local hits = #exploded
check(G.callIn(bombers, 300, 200), "an air strike is called in")
delayUpdate(1.5)
eq(#player.planes, 0, "the plane is heard first")
delayUpdate(0.6)
eq(#player.planes, 1, "then flies over")
eq(player.planes[1].sprite, "limonka", "as the strike's own plane")
eq(player.planes[1].x, 300 - 1800, "from the left, as the game's")
delayUpdate(1)
eq(#exploded, hits + 1, "the first bomb")
delayUpdate(1)
eq(#exploded, hits + 3, "and the other two, half a second apart")
eq(exploded[#exploded], "example_mod:big_bomb", "with the strike's explosion")
local strafer = G.register({ id = "example_mod:strafer", kind = "airstrike", explosion = "light_hit",
  replaces_strafing = "allied" })
stukaStrafe(100)
eq(exploded[#exploded], "example_mod:big_bomb", "an Axis Stuka's bomb set off at once is the swap's")
eq(player.delayedAction[#player.delayedAction].arg1, "example_mod:big_bomb", "and its queued ones")
check(createExplosion == realExplosion, "createExplosion is the game's again")
alliedBomb(100)
eq(player.delayedAction[#player.delayedAction].arg1, "bombExplosion", "the Allied bomber stays the game's")
p47Strafing(100)
eq(player.delayedAction[#player.delayedAction].arg1, "light_hit", "an Allied strafing run's hits are the swap's")
eq(exploded[#exploded], "light_hit", "the first one too")
axisStrafe(100)
eq(player.delayedAction[#player.delayedAction].arg1, "flakExplosion", "the Axis strafe stays the game's")
eq(G.register({ id = "x:strafe_both", kind = "airstrike", replaces_frag = "axis" }).kind, "airstrike",
  "replaces_frag on an air strike is left out, with a line in the log")
eq(G.swaps.axis, g, "and the frag swap stays")

-- Supply drops: the cargo plane, a parachute, and a crate that holds what the drop says.
function cargoCratezButton()
  for i, l in ipairs(player.lootables) do
    if l.name == "cargoCrate" then
      table.remove(player.lootables, i)
      for _ = 1, (tonumber(l.argument) or 0) do player.lootables[#player.lootables + 1] = { name = "loot_beer" } end
      return
    end
  end
end
function cargoCrateOpening(dude) cargoCratezButton() end
local function lootCount(name)
  local n = 0
  for _, l in ipairs(player.lootables) do if l.name == name then n = n + 1 end end
  return n
end
local medDrop = G.register({ id = "example_mod:med_drop", kind = "drop", contents = { "loot_medPack" }, shots = 2,
  delay = 1, skill = { class = "medic", slot = 4 } })
eq(medDrop and medDrop.explosion, nil, "a drop has no explosion")
eq(medDrop.cooldown, 300, "with the default cooldown")
eq(S.icon("medic", 4), comms_skill_1, "and the game's radio icon on a fourth skill")
check(G.dropInstalled, "a drop wraps the game's crate opening")
G.callIn(medDrop, 400, 300)
delayUpdate(1.1)
eq(player.planes[#player.planes].typez, "alliedCargoSprite", "the cargo plane flies over")
delayUpdate(2)
local chute = player.topParticles[#player.topParticles]
eq(chute and chute.qmDrop, medDrop.id, "then its parachute")
eq(chute.faction, 2, "with the crate's item count")
eq(chute.x, 400, "on the spot")
table.remove(player.topParticles)
player.lootables[#player.lootables + 1] = { name = "cargoCrate", x = chute.x, y = chute.y, argument = chute.faction }
delayUpdate(0.1)
eq(player.lootables[#player.lootables].qmDrop, medDrop.id, "the crate that landed is the drop's")
cargoCratezButton()
eq(lootCount("loot_medPack"), 2, "opened, it holds the drop's contents")
eq(lootCount("loot_beer"), 0, "and not the game's own")
local opened = 0
G.register({ id = "example_mod:supply", kind = "drop", contents = { "loot_ration" }, replaces_drop = "allied",
  on_open = function(x, y) opened = opened + 1 end })
player.lootables = { { name = "cargoCrate", x = 0, y = 0, argument = 3 }, { name = "cargoCrate", x = 5, y = 5, argument = 1 } }
cargoCrateOpening({})
eq(lootCount("loot_ration"), 3, "the Allies' own crate holds the swap's contents, as many as the game's")
eq(opened, 1, "and on_open runs")
eq(player.lootables[1].argument, 1, "a crate not opened keeps its count")

-- A faction's [[explosions]] and [[ordnance]], through the faction runtime.
if arg[4] then
  dofile(arg[4])
  local R = __qm_factions_runtime
  R.image = function() return limonka end
  R.load_ordnance({
    id = "red_army", replaces = "axis", root = "mods/RedArmy",
    explosions = { { id = "red_army:blast", based_on = "contact", numbers = { fragRadius = 120 } } },
    ordnance = { { id = "red_army:limonka", sprite = "weapons/limonka.png", explosion = "red_army:blast",
      range = 800, spin = 0, impact = true, skill_class = "light", skill_slot = 1, replaces_frag = true },
      { id = "red_army:vog", kind = "rifle", sprite = "weapons/vog.png", explosion = "rifleGrenadeExplosion",
        replaces_rifle_grenade = true },
      { id = "red_army:hvy", kind = "mortar", explosion = "hvyExplosion", shots = 3, replaces_mortar = true },
      { id = "red_army:katyusha", kind = "artillery", explosion = "artyExplosion", shots = 12,
        replaces_artillery = true },
      { id = "red_army:il2", kind = "airstrike", explosion = "bombExplosion", plane = "planes/il2.png",
        replaces_bombing = true, replaces_strafing = true },
      { id = "red_army:lend_lease", kind = "drop", explosion = "", contents = { "loot_ration" }, replaces_drop = true } },
  })
  local il2 = G.byId["red_army:il2"]
  eq(il2 and il2.plane, "qmf_red_army_il2_plane", "a faction air strike's plane has its own global")
  eq(qmf_red_army_il2_plane, limonka, "set to the faction's image")
  check(G.bombSwaps.axis == il2 and G.strafeSwaps.axis == il2, "its bombing and strafing runs are the faction's")
  local ll = G.byId["red_army:lend_lease"]
  eq(ll and ll.explosion, nil, "a faction drop has no explosion")
  eq(ll and ll.contents[1], "loot_ration", "and its contents")
  -- A faction's [[skills]].
  R.load_skills({ id = "red_army", root = "mods/RedArmy", skills = {
    { class = "rifle", slot = 1, name = "Flare", icon = "icons/flare.png", cooldown = 30 },
    { class = "scout", slot = 2, remove = true },
    { class = "sapper", slot = 9, game_class = "light", game_slot = 3, unlock = 4 } } })
  eq(S.get("rifle", 1) and S.get("rifle", 1).icon, "qmf_red_army_skill_rifle_1", "a faction skill's icon has its own global")
  eq(qmf_red_army_skill_rifle_1, limonka, "set to the faction's image")
  check(S.get("scout", 2) and S.get("scout", 2).remove, "a faction removes a skill")
  eq(S.get("sapper", 9) and S.get("sapper", 9).game.class, "light", "and adds one that is another class's")
  eq(S.maxLevel("sapper"), 4, "at a level past 3")
  local fm = G.byId["red_army:hvy"]
  eq(fm and fm.sprite, nil, "a faction mortar with no sprite")
  eq(fm and fm.shots, 3, "and its rounds")
  eq(G.mortarSwaps.axis, fm, "the faction's mortar men fire it")
  eq(G.artySwaps.axis, G.byId["red_army:katyusha"], "and its barrages are the faction's")
  local fv = G.byId["red_army:vog"]
  eq(fv and fv.kind, "rifle", "a faction's rifle grenade")
  eq(G.rifleSwaps.axis, fv, "the faction's side fires it for rifle grenades")
  local fe = X.defs["red_army:blast"]
  eq(fe and fe.fragRadius, 120, "a faction explosion is registered with its numbers")
  eq(fe and fe.blastRadius, 0, "on its preset")
  local fg = G.byId["red_army:limonka"]
  eq(fg and fg.sprite, "qmf_red_army_limonka", "a faction grenade's image has its own global")
  eq(qmf_red_army_limonka, limonka, "set to the faction's image")
  eq(fg and fg.range, 800, "with its range")
  eq(fg and fg.spin, 0, "and its spin")
  check(fg and fg.impact, "and impact")
  eq(G.skills["light:1"], fg, "it takes the class skill")
  eq(G.swaps.axis, fg, "and the faction's side throws it for frags")
end

if failures > 0 then
  print(failures .. " check(s) failed")
  os.exit(1)
end
print("ok")
