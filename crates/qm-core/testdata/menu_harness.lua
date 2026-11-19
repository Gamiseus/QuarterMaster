-- Runs QuarterMaster's menu runtime outside the game, with the game's drawing and buttons
-- stubbed. Called by qm-core's menu runtime tests:
--   lua menu_harness.lua <path to menu_runtime.lua>
-- Prints FAIL lines and exits non-zero when a check fails.

local runtime_path = arg[1]
loadstring = loadstring or load

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

-- Game stubs: drawing does nothing, buttons are recorded.
local noop = function() end
love = {
  graphics = setmetatable({}, { __index = function() return noop end }),
  math = { colorFromBytes = function(r, g, b, a) return r, g, b, a end },
  mouse = { getPosition = function() return 0, 0 end },
}
local font = { getWidth = function() return 10 end }
bigFont, cardFont, nonoFont = font, font, font
Button = { new = function(x, y, w, h, label) return { label = label } end }
buttons = {}
menuPage = 86

local menu = dofile(runtime_path)
menu.savedSettings = { example_mod = { advanced = true, flag = false } }

menu.applyConfigs({
  example_mod = {
    name = "Example Mod",
    options = {
      { id = "speed", label = "Speed", option_type = "checkbox", default = true },
      { id = "advanced", label = "Advanced", option_type = "submenu" },
      { id = "flag", label = "Flag", option_type = "checkbox", default = false, submenu = "advanced" },
      { id = "extra", label = "Extra", option_type = "checkbox", default = true, submenu = "advanced",
        show_if = { option_id = "flag", equals = { true } } },
      { id = "inner", label = "Inner", option_type = "submenu", submenu = "advanced",
        show_if = { option_id = "speed", equals = { true } } },
      { id = "deep", label = "Deep", option_type = "cycle", options = { "a", "b" }, default = "a", submenu = "inner" },
      { id = "stray", label = "Stray", option_type = "checkbox", default = true, submenu = "nowhere" },
    },
  },
})

local function ids(list)
  local out = {}
  for _, o in ipairs(list) do out[#out + 1] = o.id end
  return table.concat(out, ",")
end
local function page()
  buttons = {}
  menu.registerModOptionsButtons()
  menu.drawModOptions()
  return ids(menu.getVisibleOptions("example_mod"))
end
local function click(n) _G["qm_opt_" .. n .. "Button"]() end

-- The top page: a submenu is one row; an option naming no submenu stays on the top page.
eq(page(), "speed,advanced,stray", "top page")
eq(QM.settings.example_mod.advanced, nil, "a submenu holds no value")
eq(menu.savedSettings.example_mod.advanced, nil, "a value saved under a submenu's id is dropped")
eq(menu.getOptionValue("example_mod", { id = "advanced", option_type = "submenu" }), nil, "and none is read back")

-- Opening it shows its own options; show_if hides one until its switch is on.
click(2)
eq(menu.currentSubmenu("example_mod"), "advanced", "the row opens the submenu")
eq(page(), "flag,inner", "submenu page, extra hidden while flag is off")
click(1)
eq(QM.settings.example_mod.flag, true, "a switch in a submenu changes its option")
eq(page(), "flag,extra,inner", "extra shows once flag is on")
QM.settings.example_mod.flag = 1
eq(page(), "flag,extra,inner", "show_if = true also matches a switch saved as 1")
eq(ids(menu.getVisibleOptions("example_mod", nil)), "flag,extra,inner", "no submenu given: the open one")

-- A submenu inside a submenu; Back goes up one level at a time.
click(3)
eq(page(), "deep", "a nested submenu")
_G.qm_opt_backButton()
eq(page(), "flag,extra,inner", "Back goes up to the parent submenu")
click(3)
-- The open submenu hidden by show_if: the page falls back to the one it sits in.
QM.settings.example_mod.speed = false
eq(menu.currentSubmenu("example_mod"), "advanced", "a hidden submenu closes to its parent")
eq(page(), "flag,extra", "and its row is gone")
_G.qm_opt_backButton()
eq(menu.currentSubmenu("example_mod"), nil, "Back from a submenu reaches the top page")
eq(menuPage, 86, "without leaving the options screen")
_G.qm_opt_backButton()
eq(menuPage, 1, "Back on the top page leaves the screen")

-- Entering the screen again starts on the top page.
menu.openSubmenu.example_mod = "advanced"
menu_modOptionsButton()
eq(menu.currentSubmenu("example_mod"), nil, "the options screen opens on the top page")

-- Without submenus, getVisibleOptions behaves as before.
menu.registerOption("plain_mod", "Plain", { id = "a", label = "A", option_type = "checkbox", default = true })
menu.registerOption("plain_mod", "Plain", { id = "b", label = "B", option_type = "checkbox", default = true,
  show_if = { option_id = "a", equals = { false } } })
eq(ids(menu.getVisibleOptions("plain_mod")), "a", "show_if still hides on a mod with no submenus")

if failures > 0 then
  print(failures .. " check(s) failed")
  os.exit(1)
end
print("ok")
