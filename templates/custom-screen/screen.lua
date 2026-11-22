-- Example Mod: a screen drawn by the mod. Opened by the card in config.toml.
-- Wiki: Part 6, A fully custom screen.
local ExampleMod = _G.ExampleMod or {}   -- the mod's table, shared by all its files
_G.ExampleMod = ExampleMod

local function draw()
  love.graphics.printf("Example Mod statistics", 50, 200, 800, "left")
  love.graphics.printf("Back", 35, 690, 100, "left")   -- the label of the Back button below
end

local function registerButtons()
  -- Button.new(x, y, width, height, label, status, hoverable, tooltip)
  buttons[#buttons + 1] = Button.new(32, 686, 60, 30, "example_mod_back", "active", true, "Back")
end

function example_mod_backButton()  -- clicking a button calls the global <label>Button
  menuPage = 1                     -- 1 is the main menu
end

local pageNumber = QM.Menu.allocatePage("example_mod", "stats", draw, registerButtons)
-- Open it from anywhere with: menuPage = pageNumber

-- The card in config.toml calls this.
function ExampleMod.openStats()
  menuPage = pageNumber
end

return ExampleMod
