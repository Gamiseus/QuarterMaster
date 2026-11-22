/// QM.Events with the save guard, run by a real Lua interpreter with LÖVE's file functions
/// stubbed: handlers, the three-failures rule, battles, waves, saves and loads.
#[test]
fn events_runtime_in_a_lua_interpreter() {
    let Some(lua) = ["luajit", "lua5.1", "lua"].into_iter().find(|exe| {
        std::process::Command::new(exe).arg("-v").output().is_ok()
    }) else {
        eprintln!("skipping events runtime test: needs a Lua interpreter");
        return;
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new(lua)
        .arg(dir.join("testdata/events_harness.lua"))
        .arg(dir.join("src/patch/events_runtime.lua"))
        .arg(dir.join("src/patch/save_guard.lua"))
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success() && text.trim_end().ends_with("ok"), "events harness failed:\n{text}");
}

#[test]
fn events_runtime_runs_before_the_save_guard_in_main_lua() {
    let payload = crate::patch::loader::STEAM_SAFETY_PAYLOAD;
    let events = payload.find("QuarterMaster: game events").unwrap();
    let guard = payload.find("QuarterMaster: damaged saves").unwrap();
    assert!(events < guard);
    assert!(crate::QM_RUNTIME_FOOTER_PAYLOAD.contains("qm_events.tick"));
}
