use super::*;
use qm_schema::{ConfigOptionDef, ConfigOptionType, ConfigValue, ModConfigSchema};

#[test]
fn test_compile_menu_configs_generates_valid_lua_table() {
    let schema = ModConfigSchema {
        mod_id: Some("tactical_overhaul".into()),
        name: "Tactical Overhaul".into(),
        options: vec![ConfigOptionDef {
            id: "smart_move".into(),
            label: "Smart Move".into(),
            option_type: ConfigOptionType::Checkbox,
            default: Some(ConfigValue::Bool(true)),
            description: Some("Cover distribution".into()),
            options: Vec::new(),
            labels: Vec::new(),
            min: None,
            max: None,
            step: None,
            show_if: None,
            button_text: None,
            aliases: Vec::new(),
            on_click: None,
        }],
        modes: Vec::new(),
        pages: Vec::new(),
        keybinds: Vec::new(),
    };

    let loaded = LoadedModConfig {
        mod_id: "tactical_overhaul".into(),
        priority: 50,
        config: schema,
    };

    let lua = compile_menu_configs(&[loaded]);
    assert!(lua.starts_with('{'), "Compiled config must be a Lua table literal");
    assert!(lua.contains(r#"["tactical_overhaul"]"#), "Must contain mod_id key");
    assert!(lua.contains(r#"["id"]="smart_move""#), "Must contain option id");
    assert!(lua.contains(r#"["default"]=true"#), "Must contain default value");
}

#[test]
fn test_build_menu_patches_generates_synthetic_patches() {
    assert!(build_menu_patches(&[]).is_empty());

    let loaded = LoadedModConfig {
        mod_id: "test_mod".into(),
        priority: 50,
        config: ModConfigSchema {
            mod_id: Some("test_mod".into()),
            name: "Test Mod".into(),
            options: Vec::new(),
            modes: Vec::new(),
            pages: Vec::new(),
            keybinds: Vec::new(),
        },
    };
    let patches = build_menu_patches(&[loaded]);
    assert_eq!(patches.len(), 10, "Must produce exactly 10 core menu and settings patches");
    let ids: Vec<&str> = patches.iter().filter_map(|p| p.id()).collect();
    assert!(ids.contains(&"qm_menu_runtime"));
    assert!(ids.contains(&"qm_menu_init"));
    assert!(ids.contains(&"qm_settings_battle_sync"));
    assert!(ids.contains(&"qm_menu_urb_button"));
    assert!(ids.contains(&"qm_menu_urb_screens"));
    assert!(ids.contains(&"qm_settings_profile_new"));
    assert!(ids.contains(&"qm_settings_profile_update"));
    assert!(ids.contains(&"qm_menu_draw_button"));
    assert!(ids.contains(&"qm_menu_draw_screens"));
    assert!(ids.contains(&"qm_input_hooks"));
}

#[test]
fn test_menu_runtime_button_labels_and_style() {
    let runtime = MENU_RUNTIME;
    assert!(runtime.contains(r#"drawCleanShadowText("Mod Operations""#), "Must use Title Case 'Mod Operations'");
    assert!(runtime.contains(r#"drawCleanShadowText("Mod Settings""#), "Must use Title Case 'Mod Settings'");
    assert!(runtime.contains(r#"drawCleanShadowText("Keybinds""#), "Must use Title Case 'Keybinds'");
    assert!(runtime.contains(r#"drawCleanShadowText("Back""#), "Must use Title Case 'Back'");
    assert!(!runtime.contains("MOD SETTINGS >"), "Must not contain all-caps arrow 'MOD SETTINGS >'");
    assert!(!runtime.contains("KEYBINDS >"), "Must not contain all-caps arrow 'KEYBINDS >'");
    assert!(!runtime.contains("MOD OPERATIONS >"), "Must not contain all-caps arrow 'MOD OPERATIONS >'");
}
