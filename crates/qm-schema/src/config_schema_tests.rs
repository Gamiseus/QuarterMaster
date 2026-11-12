use super::*;

#[test]
fn test_parse_declarative_config_toml_with_mixed_options() {
    let toml = r#"
[config]
id = "tactical_overhaul"
name = "Tactical Overhaul"

[[config.options]]
id = "squad_limit"
label = "Squad Size Limit"
type = "cycle"
options = [6, 8, 10, 12, "OFF"]
labels = ["6", "8", "10", "12", "OFF"]
default = 6
description = "Exposed soldier threshold"

[[config.options]]
id = "smart_move"
label = "Smart Squad Move"
type = "checkbox"
default = true
description = "Cover distribution and overwatch"
aliases = ["smartMoveEnabled"]

[[config.modes]]
id = "custom_op"
title = "Custom Operation"
subtitle = "Assault on Carentan"
description = "Airborne drop"
priority = 80
on_click = "page:briefing"

[[config.pages]]
id = "briefing"
title = "Operation Briefing"
type = "info"
content = "Intel reports strong resistance."
action_label = "DEPLOY"
action_on_click = "startCustomOperation"
"#;

    let schema = ModConfigSchema::from_toml_str(toml).expect("Failed to parse config TOML");
    assert_eq!(schema.mod_id.as_deref(), Some("tactical_overhaul"));
    assert_eq!(schema.name, "Tactical Overhaul");
    assert_eq!(schema.options.len(), 2);
    assert_eq!(schema.modes.len(), 1);
    assert_eq!(schema.pages.len(), 1);

    // Option 1: Cycle with mixed numbers and strings
    let opt1 = &schema.options[0];
    assert_eq!(opt1.id, "squad_limit");
    assert_eq!(opt1.option_type, ConfigOptionType::Cycle);
    assert_eq!(opt1.options.len(), 5);
    assert_eq!(opt1.options[0], ConfigValue::Number(6.0));
    assert_eq!(opt1.options[4], ConfigValue::String("OFF".into()));
    assert_eq!(opt1.default, Some(ConfigValue::Number(6.0)));

    // Option 2: Checkbox with aliases
    let opt2 = &schema.options[1];
    assert_eq!(opt2.id, "smart_move");
    assert_eq!(opt2.aliases, vec!["smartMoveEnabled".to_string()]);

    // Mode 1
    let mode1 = &schema.modes[0];
    assert_eq!(mode1.id, "custom_op");
    assert_eq!(mode1.title, "Custom Operation");
    assert_eq!(mode1.priority, Some(80));

    // Page 1
    let page1 = &schema.pages[0];
    assert_eq!(page1.id, "briefing");
    assert_eq!(page1.action_label.as_deref(), Some("DEPLOY"));
}

#[test]
fn test_parse_declarative_keybinds_toml() {
    let toml = r#"
id = "g_console"
name = "GConsole"

[[keybinds]]
id = "toggle_ai_debug"
name = "Toggle AI Debug"
category = "Diagnostics"
default = "f1"
modifiers = ["shift"]
tab = "GConsole"
description = "Toggles AI navigation overlay"

[[keybinds]]
id = "quick_halt"
name = "Halt All Troops"
default = "h"
secondary = "x"
on_trigger = "GDumper.dumpLua"
"#;

    let schema = ModConfigSchema::from_toml_str(toml).expect("Failed to parse keybinds TOML");
    assert_eq!(schema.keybinds.len(), 2);

    let k1 = &schema.keybinds[0];
    assert_eq!(k1.id, "toggle_ai_debug");
    assert_eq!(k1.name, "Toggle AI Debug");
    assert_eq!(k1.category, "Diagnostics");
    assert_eq!(k1.default, "f1");
    assert_eq!(k1.secondary, None);
    assert_eq!(k1.modifiers, vec!["shift".to_string()]);
    assert_eq!(k1.tab.as_deref(), Some("GConsole"));
    assert_eq!(
        k1.description.as_deref(),
        Some("Toggles AI navigation overlay")
    );
    assert_eq!(k1.on_trigger, None);

    let k2 = &schema.keybinds[1];
    assert_eq!(k2.id, "quick_halt");
    assert_eq!(k2.name, "Halt All Troops");
    assert_eq!(k2.category, "General");
    assert_eq!(k2.default, "h");
    assert_eq!(k2.secondary.as_deref(), Some("x"));
    assert!(k2.modifiers.is_empty());
    assert_eq!(k2.tab, None);
    assert_eq!(k2.on_trigger.as_deref(), Some("GDumper.dumpLua"));
}

#[test]
fn test_parse_declarative_show_if_variants() {
    let toml = r#"
id = "g_special_events"
name = "Special Events"

[[options]]
id = "event_select"
label = "Active Event"
type = "cycle"
options = ["Auto", "Halloween", "Xmas"]
default = "Auto"

[[options]]
id = "halloween_opt"
label = "Zombies Attack"
type = "checkbox"
default = true
show_if = { id = "event_select", value = "Halloween" }
"#;

    let schema = ModConfigSchema::from_toml_str(toml).expect("Failed to parse show_if TOML");
    assert_eq!(schema.options.len(), 2);
    let opt2 = &schema.options[1];
    let show_if = opt2
        .show_if
        .as_ref()
        .expect("show_if rule should be present");
    assert_eq!(show_if.option_id, "event_select");
    assert_eq!(
        show_if.equals,
        vec![ConfigValue::String("Halloween".into())]
    );
}

#[test]
fn test_from_mod_dir_precedence() {
    let tmp = tempfile::tempdir().unwrap();
    let mod_dir = tmp.path().join("test_mod");
    std::fs::create_dir_all(&mod_dir).unwrap();

    // 1. Initially empty directory
    assert_eq!(ModConfigSchema::from_mod_dir(&mod_dir).unwrap(), None);

    // 2. Add config.toml
    std::fs::write(
        mod_dir.join("config.toml"),
        r#"id = "mod_a"
name = "Mod A (config.toml)"
[[options]]
id = "opt_a"
label = "Option A"
type = "checkbox"
"#,
    )
    .unwrap();

    // Also add qm.toml with [config]
    std::fs::write(
        mod_dir.join("qm.toml"),
        r#"[config]
id = "mod_b"
name = "Mod B (qm.toml)"
"#,
    )
    .unwrap();

    // config.toml must win precedence over qm.toml
    let schema = ModConfigSchema::from_mod_dir(&mod_dir).unwrap().unwrap();
    assert_eq!(schema.name, "Mod A (config.toml)");

    // 3. Remove config.toml -> qm.toml wins
    std::fs::remove_file(mod_dir.join("config.toml")).unwrap();
    let schema2 = ModConfigSchema::from_mod_dir(&mod_dir).unwrap().unwrap();
    assert_eq!(schema2.name, "Mod B (qm.toml)");

    // 4. Remove qm.toml, place config inside 1.0.0/ version subdirectory
    std::fs::remove_file(mod_dir.join("qm.toml")).unwrap();
    let ver_dir = mod_dir.join("1.0.0");
    std::fs::create_dir_all(&ver_dir).unwrap();
    std::fs::write(
        ver_dir.join("config.toml"),
        r#"id = "mod_c"
name = "Mod C (versioned)"
[[options]]
id = "opt_c"
label = "Option C"
type = "checkbox"
"#,
    )
    .unwrap();
    let schema3 = ModConfigSchema::from_mod_dir(&mod_dir).unwrap().unwrap();
    assert_eq!(schema3.name, "Mod C (versioned)");
}
