#[test]
fn test_input_runtime_lua_embedded_and_valid() {
    let runtime = include_str!("input_runtime.lua");
    assert!(!runtime.is_empty(), "input_runtime.lua should not be empty");
    assert!(runtime.contains("_G.QM = _G.QM or {}"), "Runtime must initialize _G.QM");
    assert!(runtime.contains("_G.QM.Input = input"), "Runtime must export QM.Input");
    assert!(runtime.contains("PAGE_CONTROLS = 87"), "Page 87 must be defined");

    // Action and Keybinding Registry
    assert!(runtime.contains("function input.registerAction"), "Must implement registerAction");
    assert!(runtime.contains("function input.loadSavedKeybinds"), "Must implement loadSavedKeybinds");
    assert!(runtime.contains("function input.saveCurrentKeybinds"), "Must implement saveCurrentKeybinds");
    assert!(runtime.contains("function input.resetAction"), "Must implement resetAction");
    assert!(runtime.contains("function input.resetTab"), "Must implement resetTab");

    // Vanilla Core Controls Registration
    assert!(runtime.contains("camera_left"), "Must register camera_left");
    assert!(runtime.contains("camera_right"), "Must register camera_right");
    assert!(runtime.contains("squad_halt"), "Must register squad_halt");
    assert!(runtime.contains("grouping"), "Must register grouping");
    assert!(runtime.contains("alt_move"), "Must register alt_move");
    assert!(runtime.contains("selection_mod"), "Must register selection_mod");
    assert!(runtime.contains("deselect"), "Must register deselect");
    assert!(runtime.contains("tactical_view"), "Must register tactical_view");
    assert!(runtime.contains("pause"), "Must register pause");
    assert!(runtime.contains("toggle_faces"), "Must register toggle_faces");

    // LÖVE Engine Input Shim & Mouse Support
    assert!(runtime.contains("love.keyboard.isDown ="), "Must wrap love.keyboard.isDown");
    assert!(runtime.contains("love.keypressed ="), "Must wrap love.keypressed");
    assert!(runtime.contains("love.keyreleased ="), "Must wrap love.keyreleased");
    assert!(runtime.contains("love.mousepressed ="), "Must wrap love.mousepressed");
    assert!(runtime.contains("love.mousereleased ="), "Must wrap love.mousereleased");
    assert!(runtime.contains("love.wheelmoved ="), "Must wrap love.wheelmoved");
    assert!(runtime.contains("input.capturingAction"), "Must support input capture mode");
    assert!(runtime.contains("input.capturingSlot"), "Must support dual-slot capture (primary and secondary)");
    assert!(runtime.contains("input.waitingForRelease"), "Must wait for key release before finalizing combo");

    // Dual-Bind Slots & Uncategorized Tab
    assert!(runtime.contains("primaryKey"), "Must support primaryKey");
    assert!(runtime.contains("secondaryKey"), "Must support secondaryKey");
    assert!(runtime.contains("uncategorized"), "Must support uncategorized tab");
    assert!(runtime.contains("input.getVisibleTabs"), "Must dynamically evaluate visible tabs");

    // Escape-to-clear Invariant
    assert!(runtime.contains("key == \"escape\""), "Must handle Escape key specifically in capture mode");
    assert!(runtime.contains("input.clearBinding"), "Escape must clear binding to [NONE]");

    // Conflict Resolution with User Override
    assert!(runtime.contains("input.findConflict"), "Must implement conflict detection");
    assert!(runtime.contains("input.pendingConflict"), "Must track pending conflicts");
    assert!(runtime.contains("qm_conf_replace"), "Must provide conflict Replace option");
    assert!(runtime.contains("qm_conf_both"), "Must provide conflict Keep Both option");

    // Screen 87 Controls UI & Pagination
    assert!(runtime.contains("function input.registerControlsButtons"), "Must implement registerControlsButtons");
    assert!(runtime.contains("function input.drawControlsMenu"), "Must implement drawControlsMenu");
    assert!(runtime.contains("qm_page_prev"), "Must provide row pagination prev button");
    assert!(runtime.contains("qm_page_next"), "Must provide row pagination next button");
    assert!(runtime.contains("_G.menu_controlsButton"), "Must export global menu_controlsButton");

    // Modifiers and Title-Case Button Styling
    assert!(runtime.contains("def.modifiers"), "Must support declarative modifiers list");
    assert!(runtime.contains(r#"drawCleanShadowText("Back""#), "Must use Title Case 'Back'");
    assert!(runtime.contains(r#"drawCleanShadowText("Reset Tab to Defaults""#), "Must use Title Case 'Reset Tab to Defaults'");
}
