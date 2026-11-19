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

-- More options than fit on the screen are split into pages of 14.
local long = {}
for i = 1, 30 do
  long[#long + 1] = { id = "o" .. i, label = "Option " .. i, option_type = "checkbox", default = true }
end
long[#long + 1] = { id = "sub", label = "Sub", option_type = "submenu" }
for i = 1, 16 do
  long[#long + 1] = { id = "s" .. i, label = "Inner " .. i, option_type = "checkbox", default = true, submenu = "sub" }
end
menu.applyConfigs({ long_mod = { name = "Long Mod", options = long } })
for i, m in ipairs(menu.modList) do if m == "long_mod" then menu.activeModIndex = i end end
local function onScreen()
  buttons = {}
  menu.registerModOptionsButtons()
  menu.drawModOptions()
  local list, pageIdx, totalPages = menu.getPagedOptions("long_mod")
  return ids(list), pageIdx, totalPages
end
local shown, pageIdx, totalPages = onScreen()
eq(totalPages, 3, "31 rows make three pages")
eq(shown, "o1,o2,o3,o4,o5,o6,o7,o8,o9,o10,o11,o12,o13,o14", "the first page holds 14 rows")
eq(pageIdx, 1, "page 1")
local hasPrev, hasNext = false, false
for _, b in ipairs(buttons) do
  if b.label == "qm_opt_page_prev" then hasPrev = true end
  if b.label == "qm_opt_page_next" then hasNext = true end
end
check(not hasPrev and hasNext, "page 1 has a Next button and no Previous")
qm_opt_page_nextButton()
shown, pageIdx = onScreen()
eq(pageIdx, 2, "Next turns the page")
eq(shown:match("^[^,]+"), "o15", "page 2 starts at the 15th row")
qm_opt_page_nextButton()
shown, pageIdx = onScreen()
eq(shown, "o29,o30,sub", "the last page holds the rest")
-- The submenu opens on its own first page; Back returns to the page it was opened from.
click(3)
shown, pageIdx, totalPages = onScreen()
eq(totalPages, 2, "the submenu has its own pages")
eq(pageIdx, 1, "and opens on its first")
menu.scrollOptions(-1)
shown, pageIdx = onScreen()
eq(pageIdx, 2, "the wheel down turns to the next page")
eq(shown, "s15,s16", "the submenu's second page")
menu.scrollOptions(-1)
shown, pageIdx = onScreen()
eq(pageIdx, 2, "the wheel stops at the last page")
menu.scrollOptions(1)
shown, pageIdx = onScreen()
eq(pageIdx, 1, "the wheel up turns back")
_G.qm_opt_backButton()
shown, pageIdx = onScreen()
eq(pageIdx, 3, "Back returns to the page the submenu was opened from")
-- Fewer rows than the page held (options hidden): the page moves back to the last one.
for i = 1, 30 do QM.Menu.optionsByMod.long_mod.options[i].show_if = { option_id = "o1", equals = { false } } end
shown, pageIdx, totalPages = onScreen()
eq(totalPages, 1, "with rows hidden there is one page")
eq(pageIdx, 1, "and the page follows")
-- Entering the screen starts on the first page.
menu.optionsPageIndex["long_mod/"] = 3
menu_modOptionsButton()
eq(menu.optionsPageIndex["long_mod/"], nil, "the options screen opens on page 1")

if failures > 0 then
  print(failures .. " check(s) failed")
  os.exit(1)
end
print("ok")
