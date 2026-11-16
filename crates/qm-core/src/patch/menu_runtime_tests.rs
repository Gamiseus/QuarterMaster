#[test]
fn test_menu_runtime_lua_embedded_and_valid() {
    let runtime = include_str!("menu_runtime.lua");
    assert!(!runtime.is_empty(), "menu_runtime.lua should not be empty");
    assert!(runtime.contains("_G.QM = _G.QM or {}"), "Runtime must initialize _G.QM");
    assert!(runtime.contains("QM.Menu"), "Runtime must export QM.Menu");
    assert!(runtime.contains("QM.settings"), "Runtime must export QM.settings");
    assert!(runtime.contains("function menu.applyConfigs"), "Runtime must implement applyConfigs");
    assert!(runtime.contains("PAGE_MOD_MODES = 85"), "Page 85 must be defined");
    assert!(runtime.contains("PAGE_MOD_OPTIONS = 86"), "Page 86 must be defined");
    // Ensure no legacy _G.GMenu references are used
    assert!(!runtime.contains("_G.GMenu"), "Legacy _G.GMenu must not be present");
    // Settings lifecycle and automatic dispatch functions
    assert!(runtime.contains("function _G.QM.syncToPlayer"), "Must implement QM.syncToPlayer");
    assert!(runtime.contains("function _G.QM.syncBattleGlobals"), "Must implement QM.syncBattleGlobals");
    assert!(runtime.contains("_G.loadModSettings ="), "Must export global loadModSettings bridge");
    assert!(runtime.contains("_G.saveModSettings ="), "Must export global saveModSettings bridge");
    assert!(runtime.contains("registerOptionButtonHandlers"), "Must implement automatic op_*Button registration");
    assert!(runtime.contains("applyOptionToTable"), "Must implement option-to-table alias mapping");
}
