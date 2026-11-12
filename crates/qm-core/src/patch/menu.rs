//! Built-in menu framework: synthesizes patches for QM.Menu and compiles mod configuration.

use qm_schema::ModConfigSchema;

use crate::patch::{
    InsertPosition, OnFailurePolicy, Patch, PatternPatch, Priority, Target,
};

pub const MENU_RUNTIME: &str = include_str!("menu_runtime.lua");
pub const INPUT_RUNTIME: &str = include_str!("input_runtime.lua");
pub const MENU_PATCH_PRIORITY: Priority = i32::MAX / 2 + 10;

#[derive(Debug, Clone)]
pub struct LoadedModConfig {
    pub mod_id: String,
    pub priority: Priority,
    pub config: ModConfigSchema,
}

/// Compiles loaded mod configurations into a Lua table literal string for the game runtime.
pub fn compile_menu_configs(configs: &[LoadedModConfig]) -> String {
    let mut map = serde_json::Map::new();
    for cfg in configs {
        let mut val = serde_json::to_value(&cfg.config).unwrap_or_default();
        if let Some(obj) = val.as_object_mut() {
            obj.insert("priority".into(), serde_json::json!(cfg.priority));
        }
        map.insert(cfg.mod_id.clone(), val);
    }
    let mut out = String::new();
    crate::patch::faction::to_lua(&serde_json::Value::Object(map), &mut out);
    out
}

fn pattern_patch(
    id: &str,
    name: &str,
    target: &str,
    pattern: &str,
    position: InsertPosition,
    payload: String,
) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: pattern.to_string(),
        position,
        target: Target::Single(target.to_string()),
        payload,
        match_indent: true,
        times: Some(1),
        overwrite: false,
        id: Some(id.to_string()),
        name: Some(name.to_string()),
        description: None,
        requires: Vec::new(),
        priority: Some(MENU_PATCH_PRIORITY),
        before: Vec::new(),
        after: Vec::new(),
        on_failure: OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        apply_as: None,
        mod_id: Some("quartermaster".to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

/// Builds the 6 synthetic patches that inject QM.Menu and register active mod options/modes/pages.
pub fn build_menu_patches(configs: &[LoadedModConfig]) -> Vec<Patch> {
    if configs.is_empty() {
        return Vec::new();
    }

    let compiled_lua = compile_menu_configs(configs);

    let runtime_payload = format!(
        "if not _G.QM then _G.QM = {{}} end\nif not _G.QM.Menu then\n(function()\n{}\nend)()\nend\nif not _G.QM.Input then\n(function()\n{}\nend)()\nend\n",
        MENU_RUNTIME,
        INPUT_RUNTIME
    );

    let init_payload = format!(
        "if QM and QM.Menu and QM.Menu.applyConfigs then\n  QM.Menu.applyConfigs((function() return {} end)())\nend\nif QM and QM.Input and QM.Input.applyConfigs then\n  QM.Input.applyConfigs((function() return {} end)())\nend\n",
        compiled_lua,
        compiled_lua
    );

    let urb_button_pattern = r#"    else
      buttons[#buttons+1] = Button.new(32, 686, 60, 30, "pause_yes","active", true, "Quit the game")
    end"#;

    let urb_button_payload = r#"    if QM and QM.Menu and QM.Menu.registerMainMenuButton then QM.Menu.registerMainMenuButton() end"#;

    let urb_screens_pattern = r#"  elseif(menuPage == 10) then 
    buttons[#buttons+1] = Button.new(32, 686, 60, 30, "menu_back","active", true, mouseOverBack)"#;

    let urb_screens_payload = r#"  elseif(menuPage == 85) then 
    if QM and QM.Menu and QM.Menu.registerModModesButtons then QM.Menu.registerModModesButtons() end
  elseif(menuPage == 86) then 
    if QM and QM.Menu and QM.Menu.registerModOptionsButtons then QM.Menu.registerModOptionsButtons() end
  elseif(menuPage == 87) then 
    if QM and QM.Input and QM.Input.registerControlsButtons then QM.Input.registerControlsButtons() end
  elseif QM and QM.Menu and QM.Menu.customPages and QM.Menu.customPages[menuPage] then
    if QM.Menu.customPages[menuPage].registerButtons then QM.Menu.customPages[menuPage].registerButtons() end"#;

    let draw_button_pattern = r#"    else
      love.graphics.printf({{love.math.colorFromBytes(211,211,211,uiFade)}, "Quit"}, 50, 685, 280, "left")
    end"#;

    let draw_button_payload = r#"    if QM and QM.Menu and QM.Menu.drawMainMenuButton then QM.Menu.drawMainMenuButton() end"#;

    let draw_screens_pattern = r#"    if(player.borderLess == 1) then
      love.graphics.setColor( love.math.colorFromBytes(101, 24, 24, uiFade))
      love.graphics.rectangle( "fill", 1026, 276, 18, 18 )
    end
    --
  end"#;

    let draw_screens_payload = r#"    if(player.borderLess == 1) then
      love.graphics.setColor( love.math.colorFromBytes(101, 24, 24, uiFade))
      love.graphics.rectangle( "fill", 1026, 276, 18, 18 )
    end
    --
  elseif(menuPage == 85) then
    if QM and QM.Menu and QM.Menu.drawModModes then QM.Menu.drawModModes() end
  elseif(menuPage == 86) then
    if QM and QM.Menu and QM.Menu.drawModOptions then QM.Menu.drawModOptions() end
  elseif(menuPage == 87) then
    if QM and QM.Input and QM.Input.drawControlsMenu then QM.Input.drawControlsMenu() end
  elseif QM and QM.Menu and QM.Menu.customPages and QM.Menu.customPages[menuPage] then
    if QM.Menu.customPages[menuPage].draw then QM.Menu.customPages[menuPage].draw() end
  end"#;

    // Three lines: the last two alone are also in `updateProfile`, and this patch is for
    // `newProfile` only.
    let urb_new_profile_pattern =
        "player.learningDays = 0\n  player.broomTimer = 0\n  player.borderLess = 0";
    let urb_new_profile_payload = "if QM and QM.syncToPlayer then QM.syncToPlayer(player) end";

    let urb_update_profile_pattern = r#"print("check for profile updates..")"#;
    let urb_update_profile_payload = "if QM and QM.syncToPlayer then QM.syncToPlayer(player) end";

    let main_battle_sync_pattern = "mnb3Tomnb2_maxWave = player.maxWave";
    let main_battle_sync_payload = "if QM and QM.syncBattleGlobals then QM.syncBattleGlobals() end";

    vec![
        pattern_patch(
            "qm_menu_runtime",
            "QuarterMaster Menu Runtime",
            "main.lua",
            r#"require "includes/directives""#,
            InsertPosition::After,
            runtime_payload,
        ),
        pattern_patch(
            "qm_menu_init",
            "QuarterMaster Menu Init & Settings",
            "main.lua",
            "tensionLoop:setLooping(true)",
            InsertPosition::Before,
            init_payload,
        ),
        pattern_patch(
            "qm_settings_battle_sync",
            "QuarterMaster Settings Battle Transition Sync",
            "main.lua",
            main_battle_sync_pattern,
            InsertPosition::After,
            main_battle_sync_payload.to_string(),
        ),
        pattern_patch(
            "qm_menu_urb_button",
            "QuarterMaster Main Menu Button",
            "includes/urb_engine.lua",
            urb_button_pattern,
            InsertPosition::After,
            urb_button_payload.to_string(),
        ),
        pattern_patch(
            "qm_menu_urb_screens",
            "QuarterMaster Menu Buttons Dispatcher",
            "includes/urb_engine.lua",
            urb_screens_pattern,
            InsertPosition::After,
            urb_screens_payload.to_string(),
        ),
        pattern_patch(
            "qm_settings_profile_new",
            "QuarterMaster Settings Default Seeding",
            "includes/urb_engine.lua",
            urb_new_profile_pattern,
            InsertPosition::After,
            urb_new_profile_payload.to_string(),
        ),
        pattern_patch(
            "qm_settings_profile_update",
            "QuarterMaster Settings Profile Update Sync",
            "includes/urb_engine.lua",
            urb_update_profile_pattern,
            InsertPosition::After,
            urb_update_profile_payload.to_string(),
        ),
        pattern_patch(
            "qm_menu_draw_button",
            "QuarterMaster Main Menu Button Render",
            "includes/drawFiles.lua",
            draw_button_pattern,
            InsertPosition::After,
            draw_button_payload.to_string(),
        ),
        pattern_patch(
            "qm_menu_draw_screens",
            "QuarterMaster Menu Screens Render",
            "includes/drawFiles.lua",
            draw_screens_pattern,
            InsertPosition::At,
            draw_screens_payload.to_string(),
        ),
        pattern_patch(
            "qm_input_hooks",
            "QuarterMaster Input Shim Hooks",
            "main.lua",
            "function love.textinput(t)",
            InsertPosition::Before,
            "if QM and QM.Input and QM.Input.installHooks then QM.Input.installHooks() end\n\n".to_string(),
        ),
    ]
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod menu_tests;
