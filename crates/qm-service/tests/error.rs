mod support;
use qm_service::{ErrorKind, OrKind, QmError};
use support::Fixture;

#[test]
fn error_serializes_to_kind_message_details() {
    let v = serde_json::to_value(QmError::game_running()).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"kind":"game_running","message":"Mud and Blood is running. Close the game first.","details":null})
    );
}

#[test]
fn or_kind_maps_string_errors() {
    let e = Err::<(), String>("boom".into())
        .or_kind(ErrorKind::Io)
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Io);
    assert_eq!(e.message, "boom");
}

#[test]
fn fixture_never_points_at_real_game() {
    let fx = Fixture::new();
    let s = fx.paths.load_settings();
    assert!(s.game_exe_path.unwrap().starts_with(fx.tmp.path()));
    assert!(s.save_dir_path.unwrap().starts_with(fx.tmp.path()));
    assert!(s.workspace_mods_dir.unwrap().starts_with(fx.tmp.path()));
}

#[test]
fn fixture_game_and_mod_are_usable() {
    let fx = Fixture::new();
    let info = fx.paths.ensure_vanilla_cache(&fx.game_exe).unwrap();
    assert!(info.lua_file_count >= 2);
    assert_eq!(info.game_version, "0.5.3b");
    let main_lua = std::fs::read_to_string(info.cache_dir.join("main.lua")).unwrap();
    assert_eq!(
        qm_manager::extract_game_version_from_main_lua(&main_lua).as_deref(),
        Some("0.5.3b")
    );
    fx.add_mod(
        "probe",
        "1.0.0",
        "[mod]
id = \"probe\"
name = \"probe\"
version = \"1.0.0\"
",
    );
    assert!(fx
        .paths
        .list_installed_mods()
        .iter()
        .any(|m| m.manifest.id == "probe"));
}
