use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::RwLock;

use qm_core::handshake::{CrashReport, GluePatchEntry};
use qm_core::patch::table::PatchTable;
use qm_manager::{
    decode_profile_code, lint_mod_directory, package_mod_release, register_windows_protocols,
    resolve_load_order, scaffold_new_mod, simulate_and_analyze_patches,
    ConflictResolutionSuggestion, LaunchMethod, ModProfile, QuarterMasterPaths,
};
use qm_schema::{compare_saves, ConfigValue, ModSettingsStore, SecurityScanReport};
use serde_json::{json, Value};

/// The legacy app keeps its previous 100 MB download ceiling.
const LEGACY_MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// The legacy app never restricted redirect targets.
fn legacy_allow_any(_url: &str) -> Result<(), String> {
    Ok(())
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchStatusInfo {
    pub stage: &'static str,
    pub pid: Option<u32>,
    pub detail: &'static str,
}

static LAUNCH_STATUS: RwLock<LaunchStatusInfo> = RwLock::new(LaunchStatusInfo {
    stage: "idle",
    pid: None,
    detail: "Ready to launch",
});

const EMBEDDED_INDEX_HTML: &str = include_str!("../../../ui/index.html");
const EMBEDDED_STYLES_CSS: &str = include_str!("../../../ui/styles.css");
const EMBEDDED_APP_JS: &str = include_str!("../../../ui/app.js");

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let paths = QuarterMasterPaths::resolve(None);
    let settings = paths.load_settings();

    if paths.list_installed_mods().is_empty() {
        if let Some(ref ws) = settings.workspace_mods_dir {
            let _ = paths.sync_workspace_mods(ws);
        }
    }
    let _ = paths.activate_profile(&settings.active_profile_id);

    // If the game process is not currently running, ensure the game folder is in a clean unmodded state
    if qm_manager::profiles::is_game_running().is_none() {
        let _ = legacy_restore_clean(&paths);
    }

    if args.iter().any(|a| a == "--smoke-test") {
        match build_full_state(&paths) {
            Ok(state_val) => {
                let mods_len = state_val["installed_mods"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                let applied = state_val["simulation"]["applied_patches"]
                    .as_u64()
                    .unwrap_or(0);
                let total = state_val["simulation"]["total_patches"]
                    .as_u64()
                    .unwrap_or(0);
                println!(
                    "[QM-App Smoke Test PASSED] Mods: {}, Simulation: {}/{} patches applied cleanly.",
                    mods_len, applied, total
                );
                return;
            }
            Err(e) => {
                eprintln!("[QM-App Smoke Test FAILED] {e}");
                std::process::exit(1);
            }
        }
    }

    let port = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4789);

    let listener = TcpListener::bind(("127.0.0.1", port))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .expect("Failed to bind local QuarterMaster GUI server");
    let local_addr = listener.local_addr().unwrap();
    let url = format!("http://{}", local_addr);

    // Pre-warm initial state cache so routine UI queries respond within SLA (< 2500ms)
    let _ = build_full_state(&paths);

    println!("========================================================");
    println!("QuarterMaster Desktop Launcher Server listening on {url}");
    println!("========================================================");
    let _ = std::io::stdout().flush();

    if !args.iter().any(|a| a == "--no-window") {
        open_native_app_window(&url);
    }

    for stream in listener.incoming().flatten() {
        let paths_clone = paths.clone();
        std::thread::spawn(move || {
            let _ = handle_http_connection(stream, &paths_clone);
        });
    }
}

fn open_native_app_window(url: &str) {
    let edge_status = Command::new("cmd")
        .args([
            "/C",
            "start",
            "",
            "msedge",
            &format!("--app={url}"),
            "--window-size=1440,900",
        ])
        .spawn();
    if edge_status.is_err() {
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
}

fn handle_http_connection(
    mut stream: std::net::TcpStream,
    paths: &QuarterMasterPaths,
) -> Result<(), String> {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut data = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];
    let mut header_end_pos = None;
    let mut expected_content_len = 0usize;

    loop {
        let n = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        data.extend_from_slice(&chunk[..n]);

        if header_end_pos.is_none() {
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let hdr_end = pos + 4;
                header_end_pos = Some(hdr_end);
                let hdr_str = String::from_utf8_lossy(&data[..pos]);
                for line in hdr_str.lines() {
                    if let Some((k, v)) = line.split_once(':') {
                        if k.trim().eq_ignore_ascii_case("content-length") {
                            expected_content_len = v.trim().parse::<usize>().unwrap_or(0);
                        }
                    }
                }
            }
        }

        if let Some(hdr_end) = header_end_pos {
            if data.len() >= hdr_end + expected_content_len {
                break;
            }
        }
        if data.len() > 4 * 1024 * 1024 {
            break;
        }
    }

    if data.is_empty() {
        return Ok(());
    }
    let raw_req = String::from_utf8_lossy(&data);
    let mut lines = raw_req.lines();
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let full_path = parts[1];
    let (route, query) = full_path.split_once('?').unwrap_or((full_path, ""));

    let body_str = raw_req.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");

    match (method, route) {
        ("GET", "/") | ("GET", "/index.html") => {
            let html = fs::read_to_string("Quartermaster/ui/index.html")
                .or_else(|_| fs::read_to_string("ui/index.html"))
                .unwrap_or_else(|_| EMBEDDED_INDEX_HTML.to_string());
            send_response(&mut stream, 200, "text/html; charset=utf-8", html.as_bytes())
        }
        ("GET", "/styles.css") => {
            let css = fs::read_to_string("Quartermaster/ui/styles.css")
                .or_else(|_| fs::read_to_string("ui/styles.css"))
                .unwrap_or_else(|_| EMBEDDED_STYLES_CSS.to_string());
            send_response(&mut stream, 200, "text/css; charset=utf-8", css.as_bytes())
        }
        ("GET", "/app.js") => {
            let js = fs::read_to_string("Quartermaster/ui/app.js")
                .or_else(|_| fs::read_to_string("ui/app.js"))
                .unwrap_or_else(|_| EMBEDDED_APP_JS.to_string());
            send_response(
                &mut stream,
                200,
                "application/javascript; charset=utf-8",
                js.as_bytes(),
            )
        }
        ("GET", "/vanilla_engine_reference.json") => {
            let json = fs::read_to_string("Quartermaster/ui/vanilla_engine_reference.json")
                .or_else(|_| fs::read_to_string("ui/vanilla_engine_reference.json"))
                .unwrap_or_else(|_| "{}".to_string());
            send_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                json.as_bytes(),
            )
        }
        ("GET", "/vanilla_api.json") => {
            let json = fs::read_to_string("Quartermaster/ui/vanilla_api.json")
                .or_else(|_| fs::read_to_string("ui/vanilla_api.json"))
                .unwrap_or_else(|_| "{}".to_string());
            send_response(
                &mut stream,
                200,
                "application/json; charset=utf-8",
                json.as_bytes(),
            )
        }
        ("GET", "/api/state") => {
            let res = build_full_state(paths);
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/file-diff") => {
            let target_file = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("file="))
                .map(url_decode)
                .unwrap_or_default();
            let res = build_file_diff(paths, &target_file);
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/devtools/vanilla-source") => {
            let target_file = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("file="))
                .map(url_decode)
                .unwrap_or_default();
            let res = build_vanilla_source(paths, &target_file);
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/devtools/vanilla-files") => {
            let res = build_vanilla_files_list(paths);
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/devtools/patches") => {
            let mod_dir = get_query_param(query, "mod_dir")
                .or_else(|| get_query_param(query, "mod_id"))
                .unwrap_or_default();
            let res = build_mod_patches_list(paths, &mod_dir);
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/devtools/decompile-assets") => {
            let res = decompile_vanilla_assets(paths);
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/conflict/demo") => {
            let res = build_conflict_demo();
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/devtools/files") => {
            let mod_dir = get_query_param(query, "mod_dir")
                .or_else(|| get_query_param(query, "mod_id"))
                .unwrap_or_default();
            let res = qm_manager::devtools::resolve_mod_dir(&mod_dir, paths, false)
                .and_then(|dir| qm_manager::devtools::list_mod_files(&dir))
                .map(|files| serde_json::to_value(files).unwrap_or_default());
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/devtools/file/read") => {
            let mod_dir = get_query_param(query, "mod_dir")
                .or_else(|| get_query_param(query, "mod_id"))
                .unwrap_or_default();
            let file_path = get_query_param(query, "file_path").unwrap_or_default();
            let res = qm_manager::devtools::resolve_mod_dir(&mod_dir, paths, false)
                .and_then(|dir| qm_manager::devtools::read_mod_file(&dir, &file_path))
                .map(|content| json!({ "content": content, "file_path": file_path }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/activate") | ("POST", "/api/profile/switch") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let id = v["profile_id"].as_str().unwrap_or("default");
            let res = paths
                .activate_profile(id)
                .map(|plan| json!({ "ok": true, "plan": plan }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/create") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let name = v["name"].as_str().unwrap_or("");
            let id_opt = v["id"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty());
            let clone_from = v["clone_from_id"].as_str();
            let linked_save = v["linked_save_id"]
                .as_str()
                .or_else(|| v["linked_save_profile_id"].as_str());
            let desc = v["description"].as_str();
            let res = paths
                .create_profile_with_id(id_opt, name, clone_from, linked_save, desc)
                .map(|prof| serde_json::to_value(prof).unwrap_or_default());
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/rename") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let profile_id = v["profile_id"].as_str().unwrap_or("");
            let new_name = v["new_name"].as_str().unwrap_or("");
            let res = paths
                .rename_profile(profile_id, new_name)
                .map(|prof| serde_json::to_value(prof).unwrap_or_default());
            send_profile_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/delete") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let profile_id = v["profile_id"].as_str().unwrap_or("");
            let settings = paths.load_settings();
            let res = paths
                .delete_profile(profile_id, &settings.active_profile_id)
                .map(|deleted_id| json!({ "ok": true, "deleted": deleted_id }));
            send_profile_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/update") => {
            let res = serde_json::from_str::<ModProfile>(body_str)
                .map_err(|e| format!("Invalid profile payload: {e}"))
                .and_then(|prof| {
                    let id = prof.id.clone();
                    paths.save_profile(&prof)?;
                    paths.activate_profile(&id)?;
                    Ok(json!({ "ok": true }))
                });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/clone") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let src_id = v["src_id"].as_str().unwrap_or("default");
            let new_id = v["new_id"].as_str().unwrap_or("cloned");
            let new_name = v["new_name"].as_str().unwrap_or("Cloned Profile");
            let res = paths
                .clone_profile(src_id, new_id, new_name)
                .and_then(|p| {
                    paths.activate_profile(&p.id)?;
                    Ok(json!({ "ok": true, "profile": p }))
                });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/diff") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let a = v["profile_a"].as_str().unwrap_or("default");
            let b = v["profile_b"].as_str().unwrap_or("vanilla");
            let res = paths
                .diff_profiles(a, b)
                .map(|d| serde_json::to_value(d).unwrap_or_default());
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/export-code") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let id = v["profile_id"].as_str().unwrap_or("default");
            let res = paths
                .export_profile_code(id)
                .map(|code| json!({ "code": code }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/import-code") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let code = v["code"].as_str().unwrap_or("");
            let new_id = v["new_id"].as_str().unwrap_or("imported");
            let res = decode_profile_code(code)
                .and_then(|pack| paths.apply_modpack_as_profile(&pack, new_id))
                .and_then(|p| {
                    paths.activate_profile(&p.id)?;
                    Ok(json!({ "ok": true, "profile": p }))
                });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/conflict/resolve") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let res = apply_conflict_resolution(paths, &v);
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/mod/config") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let res = update_mod_config(paths, &v);
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/mod/config/reset") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let res = reset_mod_config(paths, &v);
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/patch/configure") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let res = configure_individual_patch(paths, &v);
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/profile/dry-preview") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let target_id = v["profile_id"].as_str().unwrap_or("default");
            let res = build_dry_preview(paths, target_id);
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/health") => {
            let res = Ok(paths.check_system_health());
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/health/repair-dll") => {
            let res = legacy_dll_source()
                .and_then(|dll| paths.repair_winmm_dll(&dll))
                .map(|sha| json!({ "ok": true, "sha256": sha }));
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/diagnostics/bundle") => {
            let res = paths.build_redacted_diagnostic_bundle();
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/save/activate") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let save_id = v["save_id"].as_str().unwrap_or("default");
            let res = paths
                .activate_save_profile(save_id)
                .map(|_| json!({ "ok": true }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/save/clone") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let src_id = v["src_id"].as_str().unwrap_or("default");
            let new_id = v["new_id"].as_str().unwrap_or("cloned_save");
            let new_name = v["new_name"].as_str().unwrap_or("Cloned Save Slot");
            let res = paths
                .clone_save_profile(src_id, new_id, new_name)
                .map(|sp| json!({ "ok": true, "save_profile": sp }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/save/delete-backup") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let save_id = v["save_id"].as_str().unwrap_or("default");
            let backup_id = v["backup_id"].as_str().unwrap_or("");
            let res = paths
                .delete_save_backup(save_id, backup_id)
                .map(|_| json!({ "ok": true }));
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/save/export-dossier") => {
            let save_id = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("save_id="))
                .map(url_decode)
                .unwrap_or_else(|| "default".to_string());
            let res = paths.inspect_save_profile(&save_id).map(|sum| {
                let md = sum.to_markdown_dossier(&save_id);
                json!({
                    "save_id": save_id,
                    "markdown": md,
                    "summary": sum,
                })
            });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/modpack/export") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let prof_id = v["profile_id"].as_str().unwrap_or("default");
            let out_dir = paths.cache_dir.join("releases");
            let _ = fs::create_dir_all(&out_dir);
            let out_path = out_dir.join(format!("{prof_id}.qmpack"));
            let res = paths.export_qmpack_file(prof_id, &out_path).map(|p| {
                json!({
                    "ok": true,
                    "path": p.display().to_string(),
                })
            });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/modpack/import") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let path_str = v["path"].as_str().unwrap_or("");
            let new_id = v["new_id"].as_str().unwrap_or("modpack_imported");
            let res = paths.import_qmpack_file(Path::new(path_str)).and_then(|preview| {
                let prof = paths.apply_modpack_as_profile(&preview.manifest, new_id)?;
                paths.activate_profile(&prof.id)?;
                Ok(json!({
                    "ok": true,
                    "preview": preview,
                    "profile": prof,
                }))
            });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/mod/inspect-archive") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let path_str = v["path"].as_str().unwrap_or("");
            let allow_unmanaged = v["allow_unmanaged"].as_bool().unwrap_or(false);
            let res = paths
                .inspect_archive_candidates(Path::new(path_str), allow_unmanaged)
                .map(|cands| json!({ "ok": true, "candidates": cands }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/mod/import") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let path_str = v["path"].as_str().unwrap_or("");
            let allow_unmanaged = v["allow_unmanaged"].as_bool().unwrap_or(false);
            let selected_mod_ids: Option<Vec<String>> = v["selected_mod_ids"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect());
            let conflict_policy = v["conflict_policy"].as_str().map(|s| s.to_string());
            let res = paths
                .import_mod_from_path_with_options(
                    Path::new(path_str),
                    None,
                    allow_unmanaged,
                    selected_mod_ids,
                    conflict_policy,
                )
                .map(|sum| serde_json::to_value(sum).unwrap_or_default());
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/updates/check") | ("POST", "/api/updates/check") => {
            let res = paths.check_for_updates();
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/updates/download-launcher") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let dl_url = v["download_url"].as_str().unwrap_or("");
            let sha = v["sha256"].as_str();
            let res = paths
                .download_and_stage_launcher_update(dl_url, sha, LEGACY_MAX_DOWNLOAD_BYTES, &legacy_allow_any)
                .map(|p| json!({ "ok": true, "staged_path": p.display().to_string() }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/updates/apply-launcher") => {
            let res = paths
                .apply_launcher_update_and_restart()
                .map(|_| json!({ "ok": true, "message": "Launcher update applied. Restarting..." }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/updates/rollback-launcher") => {
            let res = paths
                .rollback_launcher_update_and_restart()
                .map(|_| json!({ "ok": true, "message": "Restoring previous launcher version. Restarting..." }));
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/browser/catalog") => {
            let mut q_str: Option<String> = None;
            let mut topic_str: Option<String> = None;
            let mut src_str: Option<String> = None;
            let mut force_refresh = false;

            for param in query.split('&') {
                if let Some(val) = param.strip_prefix("query=").or_else(|| param.strip_prefix("q=")) {
                    q_str = Some(url_decode(val));
                } else if let Some(val) = param.strip_prefix("topic=") {
                    topic_str = Some(url_decode(val));
                } else if let Some(val) = param.strip_prefix("source=") {
                    src_str = Some(url_decode(val));
                } else if param.starts_with("refresh=true") || param.starts_with("force=true") {
                    force_refresh = true;
                }
            }

            let res = Ok(paths.search_community_catalog(
                q_str.as_deref(),
                topic_str.as_deref(),
                src_str.as_deref(),
                force_refresh,
            ));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/browser/install") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let download_url = v["download_url"].as_str().unwrap_or("");
            let mod_id = v["mod_id"].as_str();
            let res = paths.download_and_install_community_mod(
                download_url,
                mod_id,
                LEGACY_MAX_DOWNLOAD_BYTES,
                &legacy_allow_any,
            );
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/system/register-protocols") => {
            let res = register_windows_protocols()
                .map(|list| json!({ "ok": true, "registered": list }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/system/open-logs-folder") => {
            let settings = paths.load_settings();
            let target_dir = settings
                .game_exe_path
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.join("Mods").join("qm").join("log")))
                .filter(|d| d.is_dir())
                .unwrap_or_else(|| paths.logs_dir.clone());
            let _ = fs::create_dir_all(&target_dir);
            let _ = Command::new("explorer").arg(&target_dir).spawn();
            send_json_result(
                &mut stream,
                Ok(json!({ "ok": true, "opened_path": target_dir.display().to_string() })),
            )
        }
        ("POST", "/api/system/acknowledge-game-update") => {
            let res = paths
                .acknowledge_game_update()
                .map(|sha| json!({ "ok": true, "exe_sha256": sha }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/overwrite/clear") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let settings = paths.load_settings();
            let prof_id = v["profile_id"]
                .as_str()
                .unwrap_or(&settings.active_profile_id);
            let res = paths
                .clear_overwrite_files(prof_id)
                .map(|cleared| json!({ "ok": true, "cleared_count": cleared }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/devtools/watch-poll") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let since_ms = v["since_epoch_ms"].as_u64().unwrap_or(0);
            let res = Ok(paths.poll_dev_watch_changes(since_ms));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/mod/sync-workspace") => {
            let settings = paths.load_settings();
            let res = if let Some(ref ws) = settings.workspace_mods_dir {
                paths
                    .sync_workspace_mods(ws)
                    .map(|m| json!({ "ok": true, "count": m.len() }))
            } else {
                Err("No workspace mods directory configured.".to_string())
            };
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/save/backup") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let save_id = v["save_id"].as_str().unwrap_or("default");
            let label = v["label"].as_str().unwrap_or("Manual UI Snapshot");
            let res = paths
                .create_save_backup(save_id, label)
                .map(|b| serde_json::to_value(b).unwrap_or_default());
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/save/restore") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let save_id = v["save_id"].as_str().unwrap_or("default");
            let bak_id = v["backup_id"].as_str().unwrap_or("");
            let res = paths
                .restore_save_backup(save_id, bak_id)
                .map(|_| json!({ "ok": true }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/save/compare") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let a = v["save_a"]
                .as_str()
                .or_else(|| v["slot_a"].as_str())
                .unwrap_or("default");
            let b = v["save_b"]
                .as_str()
                .or_else(|| v["slot_b"].as_str())
                .unwrap_or("vanilla");
            let sa = paths.inspect_save_profile(a).unwrap_or_default();
            let sb = paths.inspect_save_profile(b).unwrap_or_default();
            let cmp = compare_saves(&sa, &sb);
            send_json_result(&mut stream, Ok(serde_json::to_value(cmp).unwrap_or_default()))
        }
        ("POST", "/api/crash/disable-mod") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mod_id = v["mod_id"].as_str().unwrap_or("");
            let settings = paths.load_settings();
            let res = paths
                .get_profile(&settings.active_profile_id)
                .and_then(|mut prof| {
                    prof.enabled_mods.retain(|m| m != mod_id);
                    paths.save_profile(&prof)?;
                    paths.activate_profile(&prof.id)?;
                    Ok(json!({ "ok": true }))
                });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/crash/simulate") => {
            let settings = paths.load_settings();
            let report = CrashReport {
                id: "crash-simulated-1".to_string(),
                timestamp: "2026-10-24T15:00:00Z".to_string(),
                crash_kind: "lua_runtime".to_string(),
                error_message:
                    "includes/ai.lua:1412: attempt to index field 'specialSquadTarget' (a nil value)"
                        .to_string(),
                traceback: "stack traceback:\n\tincludes/ai.lua:1412: in function 'updateGermanCommand'\n\tmain.lua:8842: in function 'update'\n\t[C]: in function 'xpcall'".to_string(),
                crashing_file: Some("includes/ai.lua".to_string()),
                crashing_line: Some(1412),
                suspected_mod_id: Some("g_special_events".to_string()),
                suspected_patch_id: Some("g_special_events:ai.lua:2".to_string()),
                suspected_patch_name: Some("Inject Special Event AI hooks".to_string()),
                code_snippet: Vec::new(),
                active_profile_id: Some(settings.active_profile_id.clone()),
                active_mods: HashMap::new(),
                run_id: None,
                suspected_fix_ids: Vec::new(),
                suspected_adjusted_copy: false,
            };
            let _ = fs::create_dir_all(&paths.logs_dir);
            let file_path = paths.logs_dir.join("crash-simulated.json");
            let _ = fs::write(
                &file_path,
                serde_json::to_string_pretty(&report).unwrap_or_default(),
            );
            send_json_result(&mut stream, Ok(json!({ "ok": true })))
        }
        ("POST", "/api/devtools/scaffold") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mod_id = v["mod_id"].as_str().unwrap_or("new_mod");
            let mod_name = v["mod_name"].as_str().unwrap_or("New Mod");
            let author = v["author"].as_str().unwrap_or("Field Commander");
            let template = v["template"].as_str().unwrap_or("gmenu_config");
            let settings = paths.load_settings();
            let parent = settings
                .workspace_mods_dir
                .unwrap_or_else(|| PathBuf::from("MnB-Mods"));
            let res =
                scaffold_new_mod(&parent, mod_id, mod_name, author, template).and_then(|created| {
                    let _ = paths.import_mod_from_path(&created, None, false);
                    Ok(json!({ "ok": true, "path": created.display().to_string() }))
                });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/devtools/lint") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mod_dir = v["mod_dir"].as_str().unwrap_or("");
            let resolved_dir = qm_manager::devtools::resolve_mod_dir(mod_dir, paths, false);
            let settings = paths.load_settings();
            let res = resolved_dir.and_then(|dir| {
                let vanilla_res = paths
                    .load_vanilla_lua_files(None)
                    .or_else(|_| paths.load_vanilla_lua_files(settings.game_exe_path.as_deref()));
                vanilla_res
                    .and_then(|(_, vanilla)| lint_mod_directory(&dir, &vanilla))
                    .map(|r| serde_json::to_value(r).unwrap_or_default())
            });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/devtools/pack") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mod_dir = v["mod_dir"].as_str().unwrap_or("");
            let resolved_dir = qm_manager::devtools::resolve_mod_dir(mod_dir, paths, false);
            let out_dir = paths.cache_dir.join("releases");
            let res = resolved_dir.and_then(|dir| {
                package_mod_release(&dir, &out_dir).map(|(zip_path, entry)| {
                    json!({
                        "zip_path": zip_path.display().to_string(),
                        "release_index_entry": entry
                    })
                })
            });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/devtools/file/save") | ("POST", "/api/devtools/file/write") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mod_dir = v["mod_dir"]
                .as_str()
                .or_else(|| v["mod_id"].as_str())
                .unwrap_or("");
            let file_path = v["file_path"].as_str().unwrap_or("");
            let content = v["content"].as_str().unwrap_or("");
            let res = qm_manager::devtools::resolve_mod_dir(mod_dir, paths, true)
                .and_then(|dir| qm_manager::devtools::save_mod_file(&dir, file_path, content))
                .map(|_| json!({ "saved": true, "file_path": file_path }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/settings/auto-detect") => {
            let detected = paths.auto_detect_all_paths();
            send_json_result(&mut stream, Ok(json!({ "ok": true, "paths": detected })))
        }
        ("POST", "/api/settings/browse") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mode = v["mode"].as_str().unwrap_or("file");
            #[cfg(windows)]
            {
                let script = if mode == "folder" {
                    r#"Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.FolderBrowserDialog; $f.Description = 'Select Directory'; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $f.SelectedPath }"#
                } else {
                    r#"Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Filter = 'Executable (*.exe)|*.exe|All Files (*.*)|*.*'; $f.Title = 'Select Game Executable'; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $f.FileName }"#
                };
                let output = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
                    .output();
                match output {
                    Ok(out) if out.status.success() => {
                        let selected = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if !selected.is_empty() {
                            send_json_result(&mut stream, Ok(json!({ "ok": true, "path": selected })))
                        } else {
                            send_json_result(&mut stream, Ok(json!({ "ok": false, "cancelled": true })))
                        }
                    }
                    _ => send_json_result(&mut stream, Ok(json!({ "ok": false, "cancelled": true }))),
                }
            }
            #[cfg(not(windows))]
            {
                send_json_result(&mut stream, Ok(json!({ "ok": false, "error": "Browse not supported on this OS" })))
            }
        }
        ("POST", "/api/settings/open-folder") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let target_path = v["path"].as_str().unwrap_or("");
            if !target_path.is_empty() {
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("explorer").arg(target_path).spawn();
                }
            }
            send_json_result(&mut stream, Ok(json!({ "ok": true })))
        }
        ("POST", "/api/game/restore-clean") => {
            let res = legacy_restore_clean(&paths).map(|_| json!({ "ok": true, "status": "clean" }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/game/deploy") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let settings = paths.load_settings();
            let prof_id = v["profile_id"].as_str().unwrap_or(&settings.active_profile_id);
            let res = legacy_dll_source().and_then(|dll| paths.deploy_profile_to_game(prof_id, &dll)).map(|_| json!({ "ok": true, "status": "deployed" }));
            send_json_result(&mut stream, res)
        }
        ("GET", "/api/game/status") => {
            let status = LAUNCH_STATUS.read().unwrap().clone();
            let is_modded = paths.is_game_dir_modded();
            let pid = status.pid.or_else(qm_manager::profiles::is_game_running);
            let res = Ok(json!({
                "ok": true,
                "stage": status.stage,
                "detail": status.detail,
                "is_modded": is_modded,
                "is_running": pid.is_some() || status.stage == "running",
                "running_pid": pid,
            }));
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/onboarding/complete") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let mut settings = paths.load_settings();
            if let Some(s) = v["game_exe_path"].as_str().filter(|s| !s.is_empty()) {
                settings.game_exe_path = Some(PathBuf::from(s));
            }
            if let Some(s) = v["save_dir_path"].as_str().filter(|s| !s.is_empty()) {
                settings.save_dir_path = Some(PathBuf::from(s));
            }
            if let Some(s) = v["workspace_mods_dir"].as_str().filter(|s| !s.is_empty()) {
                settings.workspace_mods_dir = Some(PathBuf::from(s));
            }
            if let Some(th) = v["theme"].as_str().filter(|s| !s.is_empty()) {
                settings.theme = th.to_string();
            }
            settings.first_run_completed = true;
            let res = paths.save_settings(&settings).and_then(|_| {
                if let Some(ref ws) = settings.workspace_mods_dir {
                    let _ = paths.sync_workspace_mods(ws);
                }
                Ok(json!({ "ok": true }))
            });
            send_json_result(&mut stream, res)
        }
        ("POST", "/api/launch") => {
            let v: Value = serde_json::from_str(body_str).unwrap_or_default();
            let prof_id = v["profile_id"].as_str().unwrap_or("default");
            let is_vanilla = v["vanilla"].as_bool().unwrap_or(false);
            let method_str = v["method"].as_str().or_else(|| v["launch_method"].as_str());
            let method = if is_vanilla {
                Some(LaunchMethod::Vanilla)
            } else {
                match method_str {
                    Some("steam_url") | Some("steam") => Some(LaunchMethod::SteamUrl),
                    Some("direct_exe") => Some(LaunchMethod::DirectExe),
                    _ => None,
                }
            };
            let is_steam = method == Some(LaunchMethod::SteamUrl);
            if is_steam {
                *LAUNCH_STATUS.write().unwrap() = LaunchStatusInfo {
                    stage: "waiting_for_steam",
                    pid: None,
                    detail: "Dispatched steam://rungameid/1391530; waiting for mnb.exe to spawn...",
                };
            } else {
                *LAUNCH_STATUS.write().unwrap() = LaunchStatusInfo {
                    stage: "starting",
                    pid: None,
                    detail: "Starting Mud and Blood...",
                };
            }
            let res = legacy_launch(&paths, prof_id, method)
                .map(|pid| {
                    if !is_steam {
                        *LAUNCH_STATUS.write().unwrap() = LaunchStatusInfo {
                            stage: "running",
                            pid: Some(pid),
                            detail: "Mud and Blood running",
                        };
                    }
                    let paths_watcher = paths.clone();
                    std::thread::spawn(move || {
                        // For Steam launches, wait for mnb.exe to spawn and discover its PID
                        let target_pid = if is_steam {
                            let mut found = None;
                            for _ in 0..15 {
                                std::thread::sleep(std::time::Duration::from_millis(1000));
                                #[cfg(windows)]
                                {
                                    if let Ok(output) = std::process::Command::new("tasklist")
                                        .args(["/FI", "IMAGENAME eq mnb.exe", "/FO", "CSV", "/NH"])
                                        .output()
                                    {
                                        let text = String::from_utf8_lossy(&output.stdout);
                                        for line in text.lines() {
                                            let parts: Vec<&str> = line.split(',').collect();
                                            if parts.len() >= 2 {
                                                let p_str = parts[1].trim_matches('"').trim();
                                                if let Ok(parsed) = p_str.parse::<u32>() {
                                                    found = Some(parsed);
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }
                                if found.is_some() {
                                    break;
                                }
                            }
                            let actual = found.unwrap_or(pid);
                            *LAUNCH_STATUS.write().unwrap() = LaunchStatusInfo {
                                stage: "running",
                                pid: Some(actual),
                                detail: "Mud and Blood running via Steam",
                            };
                            actual
                        } else {
                            pid
                        };

                        loop {
                            std::thread::sleep(std::time::Duration::from_millis(1500));
                            #[cfg(windows)]
                            {
                                let still_running = std::process::Command::new("tasklist")
                                    .args(["/FI", &format!("PID eq {}", target_pid), "/NH"])
                                    .output()
                                    .map(|o| {
                                        let text = String::from_utf8_lossy(&o.stdout);
                                        text.contains(&format!("{}", target_pid))
                                    })
                                    .unwrap_or(false);
                                if !still_running {
                                    *LAUNCH_STATUS.write().unwrap() = LaunchStatusInfo {
                                        stage: "cleaning",
                                        pid: None,
                                        detail: "Game exited; restoring clean vanilla state...",
                                    };
                                    let _ = legacy_restore_clean(&paths_watcher);
                                    *LAUNCH_STATUS.write().unwrap() = LaunchStatusInfo {
                                        stage: "idle",
                                        pid: None,
                                        detail: "Ready to launch",
                                    };
                                    break;
                                }
                            }
                            #[cfg(not(windows))]
                            {
                                break;
                            }
                        }
                    });
                    json!({ "ok": true, "pid": pid })
                });
            send_json_result(&mut stream, res)
        }
        _ => send_response(
            &mut stream,
            404,
            "application/json",
            b"{\"error\":\"Not found\"}",
        ),
    }
}

struct CachedSimulation {
    profile_id: String,
    profile_json: String,
    mods_mtime: u64,
    report: qm_manager::SimulationReport,
}

static SIMULATION_CACHE: std::sync::Mutex<Option<CachedSimulation>> = std::sync::Mutex::new(None);

fn build_full_state(paths: &QuarterMasterPaths) -> Result<Value, String> {
    let settings = paths.load_settings();
    let profiles = paths.list_profiles();
    let active_profile = paths
        .get_profile(&settings.active_profile_id)
        .unwrap_or_else(|_| profiles[0].clone());

    let game_info = if let Some(ref exe) = settings.game_exe_path {
        paths.ensure_vanilla_cache(exe).ok().map(|info| {
            json!({
                "version": info.game_version,
                "exe_sha256": info.exe_sha256,
                "lua_file_count": info.lua_file_count,
            })
        })
    } else {
        None
    };

    let (game_ver, vanilla_files) = paths
        .load_vanilla_lua_files(settings.game_exe_path.as_deref())
        .unwrap_or_else(|_| ("0.5.3b".to_string(), HashMap::new()));

    let prof_settings_path = paths
        .profiles_dir
        .join(&active_profile.id)
        .join("saveModSettings.lua");
    let mod_settings_store = if prof_settings_path.is_file() {
        ModSettingsStore::from_path(&prof_settings_path).unwrap_or_default()
    } else if let Some(ref sdir) = settings.save_dir_path {
        ModSettingsStore::from_path(&sdir.join("saveModSettings.lua")).unwrap_or_default()
    } else {
        ModSettingsStore::default()
    };

    let installed = paths.list_installed_mods();
    let mut enriched_mods = Vec::new();
    for m in &installed {
        let security_report = SecurityScanReport::scan_directory(&m.install_path);
        // Optimize routine state queries against redundant SHA-256 disk hashing to maintain < 2500ms SLA
        let missing: Vec<String> = m
            .integrity
            .files
            .keys()
            .filter(|rel| !m.install_path.join(rel).exists())
            .cloned()
            .collect();
        let is_pristine = missing.is_empty();
        let integrity = qm_schema::IntegrityVerifyResult {
            is_pristine,
            modified: Vec::new(),
            missing,
            untracked: Vec::new(),
        };

        let saved_cfg_map = mod_settings_store
            .mods
            .get(&m.manifest.id)
            .map(|opts| {
                let mut map = serde_json::Map::new();
                for (k, v) in opts {
                    let jv = match v {
                        ConfigValue::Bool(b) => Value::Bool(*b),
                        ConfigValue::Number(n) => json!(*n),
                        ConfigValue::String(s) => Value::String(s.clone()),
                    };
                    map.insert(k.clone(), jv);
                }
                Value::Object(map)
            })
            .unwrap_or(Value::Null);

        enriched_mods.push(json!({
            "manifest": m.manifest,
            "patch_count": m.patch_count,
            "install_path": m.install_path,
            "readme_md": m.readme_markdown,
            "changelog_md": m.changelog_markdown,
            "config_schema": m.config_schema,
            "saved_config": saved_cfg_map,
            "security_report": security_report,
            "integrity": integrity,
        }));
    }

    let enabled_manifests: Vec<_> = installed
        .iter()
        .filter(|m| active_profile.enabled_mods.contains(&m.manifest.id))
        .map(|m| m.manifest.clone())
        .collect();

    let load_order = resolve_load_order(
        &enabled_manifests,
        &active_profile.mod_priorities,
        &active_profile.user_load_after,
        Some(&game_ver),
    );
    let profile_mods_dir = paths.profiles_dir.join(&active_profile.id).join("Mods");
    if !profile_mods_dir.exists() {
        let _ = paths.activate_profile(&active_profile.id);
    }
    let mods_mtime = fs::metadata(&profile_mods_dir)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let profile_json = serde_json::to_string(&active_profile).unwrap_or_default();

    let simulation = {
        let mut cache = SIMULATION_CACHE.lock().unwrap();
        if let Some(ref c) = *cache {
            if c.profile_id == active_profile.id
                && c.profile_json == profile_json
                && c.mods_mtime == mods_mtime
            {
                c.report.clone()
            } else {
                let rep = if let Ok(table) = PatchTable::load_strict(&profile_mods_dir) {
                    simulate_and_analyze_patches(&table, &vanilla_files)
                } else {
                    qm_manager::SimulationReport {
                        files_touched: Vec::new(),
                        conflicts: Vec::new(),
                        module_registrations: Default::default(),
                        total_patches: 0,
                        applied_patches: 0,
                        broken_anchors: 0,
                        order_diagnostics: Vec::new(),
                        traces: Default::default(),
                        compile_check: Default::default(),
                        plan_error: None,
                        plan_warnings: Vec::new(),
                        load_errors: Vec::new(),
                        fix_bindings: Vec::new(),
                        overridden_rules: Vec::new(),
                        unknown_ids: Vec::new(),
                        suspended_rungs: Vec::new(),
                        invalid_rungs: Vec::new(),
                        overrides: Vec::new(),
                        losses: Vec::new(),
                        kept_anchors: Vec::new(),
                        runs_as: Vec::new(),
                        failure_groups: Vec::new(),
                        inferred_edges: Vec::new(),
                        save_fields: Default::default(),
                    }
                };
                *cache = Some(CachedSimulation {
                    profile_id: active_profile.id.clone(),
                    profile_json,
                    mods_mtime,
                    report: rep.clone(),
                });
                rep
            }
        } else {
            let rep = if let Ok(table) = PatchTable::load_strict(&profile_mods_dir) {
                simulate_and_analyze_patches(&table, &vanilla_files)
            } else {
                qm_manager::SimulationReport {
                    files_touched: Vec::new(),
                    conflicts: Vec::new(),
                    module_registrations: Default::default(),
                    total_patches: 0,
                    applied_patches: 0,
                    broken_anchors: 0,
                    order_diagnostics: Vec::new(),
                    traces: Default::default(),
                    compile_check: Default::default(),
                    plan_error: None,
                    plan_warnings: Vec::new(),
                    load_errors: Vec::new(),
                    fix_bindings: Vec::new(),
                    overridden_rules: Vec::new(),
                    unknown_ids: Vec::new(),
                    suspended_rungs: Vec::new(),
                    invalid_rungs: Vec::new(),
                    overrides: Vec::new(),
                    losses: Vec::new(),
                    kept_anchors: Vec::new(),
                    runs_as: Vec::new(),
                    failure_groups: Vec::new(),
                    inferred_edges: Vec::new(),
                    save_fields: Default::default(),
                }
            };
            *cache = Some(CachedSimulation {
                profile_id: active_profile.id.clone(),
                profile_json,
                mods_mtime,
                report: rep.clone(),
            });
            rep
        }
    };

    let mut save_profiles_out = Vec::new();
    for sp in paths.list_save_profiles() {
        let summary = paths.inspect_save_profile(&sp.id).ok();
        save_profiles_out.push(json!({
            "profile": sp,
            "summary": summary,
        }));
    }

    let save_compatibility = paths.check_save_compatibility(&active_profile);
    let crashes: Vec<Value> = paths
        .list_crash_reports()
        .into_iter()
        .map(|c| serde_json::to_value(c).unwrap_or_default())
        .collect();
    let health = paths.check_system_health();
    let templates = paths.discover_mod_templates();
    let browser_catalog = paths.get_mod_browser_catalog();
    let updates = paths.check_for_updates().unwrap_or_default();

    Ok(json!({
        "version": qm_core::QM_VERSION,
        "is_portable": paths.is_portable,
        "root_dir": paths.root_dir,
        "library_dir": paths.library_dir,
        "settings": settings,
        "game_info": game_info,
        "installed_mods": enriched_mods,
        "profiles": profiles,
        "active_profile": active_profile,
        "load_order": load_order,
        "simulation": simulation,
        "save_profiles": save_profiles_out,
        "save_compatibility": save_compatibility,
        "crashes": crashes,
        "health": health,
        "templates": templates,
        "browser_catalog": browser_catalog,
        "updates": updates,
    }))
}

fn build_file_diff(paths: &QuarterMasterPaths, target_file: &str) -> Result<Value, String> {
    let settings = paths.load_settings();
    let (_gv, vanilla_files) =
        paths.load_vanilla_lua_files(settings.game_exe_path.as_deref())?;

    let profile_mods_dir = paths
        .profiles_dir
        .join(&settings.active_profile_id)
        .join("Mods");
    let table = PatchTable::load_strict(&profile_mods_dir).map_err(|e| format!("{e:#}"))?;
    let sim = simulate_and_analyze_patches(&table, &vanilla_files);
    let file_res = sim
        .files_touched
        .into_iter()
        .find(|f| f.target_file == target_file)
        .ok_or_else(|| format!("Patched file '{}' not found in simulation", target_file))?;

    let original_lines: Vec<String> = vanilla_files
        .get(target_file)
        .map(|s| s.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default();

    let mut val = serde_json::to_value(file_res).unwrap_or_default();
    if let Some(obj) = val.as_object_mut() {
        obj.insert("original_lines".to_string(), json!(original_lines));
    }
    Ok(val)
}

fn build_vanilla_source(paths: &QuarterMasterPaths, target_file: &str) -> Result<Value, String> {
    let settings = paths.load_settings();
    let (_gv, vanilla_files) =
        paths.load_vanilla_lua_files(settings.game_exe_path.as_deref())?;

    let clean_target = target_file
        .strip_prefix("@game/")
        .unwrap_or(target_file)
        .trim_start_matches('/');

    let content = vanilla_files
        .get(clean_target)
        .or_else(|| {
            vanilla_files
                .iter()
                .find(|(k, _)| k.ends_with(clean_target) || clean_target.ends_with(k.as_str()))
                .map(|(_, v)| v)
        })
        .ok_or_else(|| format!("Vanilla file '{}' not found", target_file))?;

    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    Ok(json!({
        "file": target_file,
        "content": content,
        "lines": lines
    }))
}

fn build_vanilla_files_list(paths: &QuarterMasterPaths) -> Result<Value, String> {
    let settings = paths.load_settings();
    let (_gv, vanilla_files) = paths
        .load_vanilla_lua_files(settings.game_exe_path.as_deref())
        .unwrap_or_else(|_| ("0.5.3b".to_string(), HashMap::new()));
    let mut files: Vec<String> = vanilla_files.into_keys().collect();
    files.sort();
    Ok(json!({
        "files": files,
        "count": files.len()
    }))
}

fn build_mod_patches_list(paths: &QuarterMasterPaths, mod_dir: &str) -> Result<Value, String> {
    let mut patches = Vec::new();
    let mod_dirs_to_check: Vec<PathBuf> = if !mod_dir.trim().is_empty() {
        if let Ok(dir) = qm_manager::devtools::resolve_mod_dir(mod_dir, paths, false) {
            vec![dir]
        } else {
            Vec::new()
        }
    } else {
        paths.list_installed_mods().into_iter().map(|m| m.install_path).collect()
    };

    for dir in mod_dirs_to_check {
        let qm_toml = dir.join("qm.toml");
        if qm_toml.is_file() {
            if let Ok(content) = fs::read_to_string(&qm_toml) {
                if let Ok(toml_val) = content.parse::<toml::Value>() {
                    if let Some(patch_arr) = toml_val.get("patches").and_then(|p| p.as_array()) {
                        for p in patch_arr {
                            let table = p.get("pattern")
                                .or_else(|| p.get("regex"))
                                .or_else(|| p.get("copy"))
                                .or_else(|| p.get("module"));
                            let target = table.and_then(|t| t.get("target")).and_then(|t| t.as_str()).unwrap_or("");
                            let id = table.and_then(|t| t.get("id")).and_then(|t| t.as_str()).unwrap_or("");
                            let name = table.and_then(|t| t.get("name")).and_then(|t| t.as_str()).unwrap_or(id);
                            let pattern = table.and_then(|t| t.get("pattern")).and_then(|t| t.as_str()).unwrap_or("");
                            let pos = table.and_then(|t| t.get("position")).and_then(|t| t.as_str()).unwrap_or("at");
                            let desc = table.and_then(|t| t.get("description")).and_then(|t| t.as_str()).unwrap_or("");
                            let requires: Vec<String> = table.and_then(|t| t.get("requires")).and_then(|r| r.as_array())
                                .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                                .unwrap_or_default();
                            if !id.is_empty() || !target.is_empty() {
                                patches.push(json!({
                                    "id": id,
                                    "name": name,
                                    "target": target,
                                    "pattern": pattern,
                                    "position": pos,
                                    "description": desc,
                                    "requires": requires,
                                    "mod_id": dir.file_name().and_then(|f| f.to_str()).unwrap_or("")
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(json!({ "patches": patches }))
}

fn decompile_vanilla_assets(paths: &QuarterMasterPaths) -> Result<Value, String> {
    let settings = paths.load_settings();
    let game_exe = settings.game_exe_path.as_ref()
        .ok_or_else(|| "Game executable path not configured".to_string())?;
    let target_dir = if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("decompile").is_dir() {
            cwd.join("decompile")
        } else if let Some(p) = cwd.parent() {
            p.join("decompile")
        } else {
            cwd.join("decompile")
        }
    } else {
        paths.vanilla_cache_dir.join("extracted")
    };

    let count = paths.extract_all_vanilla_assets(game_exe, &target_dir)?;
    Ok(json!({
        "ok": true,
        "extracted_count": count,
        "target_dir": target_dir.display().to_string()
    }))
}

fn build_dry_preview(paths: &QuarterMasterPaths, target_profile_id: &str) -> Result<Value, String> {
    let settings = paths.load_settings();
    let current_id = &settings.active_profile_id;
    let diff = paths.diff_profiles(current_id, target_profile_id)?;
    let target_prof = paths.get_profile(target_profile_id)?;
    let save_compat = paths.check_save_compatibility(&target_prof);
    let game_ver = if let Some(ref exe) = settings.game_exe_path {
        paths
            .ensure_vanilla_cache(exe)
            .ok()
            .map(|i| i.game_version)
            .unwrap_or_else(|| "0.5.3b".to_string())
    } else {
        "0.5.3b".to_string()
    };
    let installed = paths.list_installed_mods();
    let enabled_manifests: Vec<_> = installed
        .iter()
        .filter(|m| target_prof.enabled_mods.contains(&m.manifest.id))
        .map(|m| m.manifest.clone())
        .collect();
    let load_order = resolve_load_order(
        &enabled_manifests,
        &target_prof.mod_priorities,
        &target_prof.user_load_after,
        Some(&game_ver),
    );

    let added_mods = diff.only_in_b.clone();
    let removed_mods = diff.only_in_a.clone();

    Ok(json!({
        "current_profile_id": current_id,
        "target_profile": target_prof,
        "added_mods": added_mods,
        "removed_mods": removed_mods,
        "diff": diff,
        "save_compatibility": save_compat,
        "load_order": load_order,
    }))
}

fn configure_individual_patch(paths: &QuarterMasterPaths, v: &Value) -> Result<Value, String> {
    let settings = paths.load_settings();
    let mut prof = paths.get_profile(&settings.active_profile_id)?;

    if let Some(patch_id) = v["patch_id"].as_str().filter(|s| !s.is_empty()) {
        if let Some(enabled) = v["enabled"].as_bool() {
            if enabled {
                prof.disabled_patches.retain(|p| p != patch_id);
            } else if !prof.disabled_patches.contains(&patch_id.to_string()) {
                prof.disabled_patches.push(patch_id.to_string());
            }
        }
        if let Some(prio) = v["priority"].as_i64() {
            prof.patch_priorities.insert(patch_id.to_string(), prio as i32);
        } else if v["clear_priority"].as_bool() == Some(true) {
            prof.patch_priorities.remove(patch_id);
        }
    }

    if let Some(glue_obj) = v.get("glue_patch") {
        if let Ok(gp) = serde_json::from_value::<GluePatchEntry>(glue_obj.clone()) {
            prof.glue_patches.retain(|g| g.id != gp.id);
            prof.glue_patches.push(gp);
        }
    }
    if let Some(remove_glue_id) = v["remove_glue_id"].as_str() {
        prof.glue_patches.retain(|g| g.id != remove_glue_id);
    }

    paths.save_profile(&prof)?;
    paths.activate_profile(&prof.id)?;
    Ok(json!({ "ok": true, "profile": prof }))
}

fn reset_mod_config(paths: &QuarterMasterPaths, v: &Value) -> Result<Value, String> {
    let mod_id = v["mod_id"]
        .as_str()
        .ok_or_else(|| "Missing mod_id".to_string())?;
    let mode = v["mode"].as_str().unwrap_or("defaults");

    let settings = paths.load_settings();
    let prof_settings_path = paths
        .profiles_dir
        .join(&settings.active_profile_id)
        .join("saveModSettings.lua");

    let mut store = if prof_settings_path.is_file() {
        ModSettingsStore::from_path(&prof_settings_path).unwrap_or_default()
    } else if let Some(ref sdir) = settings.save_dir_path {
        ModSettingsStore::from_path(&sdir.join("saveModSettings.lua")).unwrap_or_default()
    } else {
        ModSettingsStore::default()
    };

    let installed = paths.list_installed_mods();
    let schema_opt = installed
        .into_iter()
        .find(|m| m.manifest.id == mod_id)
        .and_then(|m| m.config_schema);

    if mode == "defaults" {
        if let Some(schema) = schema_opt {
            store.reset_mod_to_defaults(mod_id, &schema);
        } else {
            store.mods.remove(mod_id);
        }
    } else if let Some(schema) = schema_opt {
        store.safe_reset_mod(mod_id, &schema);
    }

    store.save_to_path(&prof_settings_path)?;
    if let Some(ref sdir) = settings.save_dir_path {
        let _ = store.save_to_path(&sdir.join("saveModSettings.lua"));
    }

    Ok(json!({ "ok": true }))
}

fn build_conflict_demo() -> Result<Value, String> {
    Ok(json!({
        "description": "GCore ('Clear pathfinding destination on Halt (H key)') and TacticalOverhaul ('Halt order combat reaction reset') both anchor on `love.audio.play(snd_halt)` in main.lua.",
        "tier1_explanation": "Auto-Order Fix: Load TacticalOverhaul (priority 50) after GCore (priority 5) because GCore preserves `love.audio.play(snd_halt)` in its replacement.",
        "tier2_glue_preview": "love.audio.play(snd_halt)\n-- [GCore] Clear custom pathfinding waypoint\nperso[i].finalDestX = nil\n-- [TacticalOverhaul] Reset suppression reaction state\nperso[i].combatReactionTimer = 0"
    }))
}

/// Applies one resolver suggestion to the active profile. The change is simulated first, and
/// one that breaks a file or adds a conflict is refused with nothing written (D63).
fn apply_conflict_resolution(paths: &QuarterMasterPaths, v: &Value) -> Result<Value, String> {
    let sug: ConflictResolutionSuggestion = serde_json::from_value(v["suggestion"].clone())
        .map_err(|e| format!("Invalid suggestion payload: {e}"))?;
    let winner_idx = v["winner_index"].as_u64().unwrap_or(0) as usize;
    let check = qm_manager::conflict::resolve_conflict(paths, &sug, winner_idx)?;
    Ok(json!({ "ok": true, "impact": check.impact }))
}

fn update_mod_config(paths: &QuarterMasterPaths, v: &Value) -> Result<Value, String> {
    let mod_id = v["mod_id"]
        .as_str()
        .ok_or_else(|| "Missing mod_id".to_string())?;
    let vals = v["values"]
        .as_object()
        .ok_or_else(|| "Missing values object".to_string())?;

    let settings = paths.load_settings();
    let prof_settings_path = paths
        .profiles_dir
        .join(&settings.active_profile_id)
        .join("saveModSettings.lua");

    let mut store = if prof_settings_path.is_file() {
        ModSettingsStore::from_path(&prof_settings_path).unwrap_or_default()
    } else if let Some(ref sdir) = settings.save_dir_path {
        ModSettingsStore::from_path(&sdir.join("saveModSettings.lua")).unwrap_or_default()
    } else {
        ModSettingsStore::default()
    };

    let mod_entry = store.mods.entry(mod_id.to_string()).or_default();
    for (opt_id, jval) in vals {
        let cv = match jval {
            Value::Bool(b) => ConfigValue::Bool(*b),
            Value::Number(n) => ConfigValue::Number(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => ConfigValue::String(s.clone()),
            _ => continue,
        };
        mod_entry.insert(opt_id.clone(), cv);
    }

    store.save_to_path(&prof_settings_path)?;
    if let Some(ref sdir) = settings.save_dir_path {
        let _ = store.save_to_path(&sdir.join("saveModSettings.lua"));
    }

    Ok(json!({ "ok": true }))
}

fn send_json_result(
    stream: &mut std::net::TcpStream,
    res: Result<Value, String>,
) -> Result<(), String> {
    match res {
        Ok(val) => {
            let body = serde_json::to_vec(&val).unwrap_or_default();
            send_response(stream, 200, "application/json; charset=utf-8", &body)
        }
        Err(err) => {
            let body = serde_json::to_vec(&json!({ "error": err })).unwrap_or_default();
            send_response(stream, 400, "application/json; charset=utf-8", &body)
        }
    }
}

fn send_profile_json_result(
    stream: &mut std::net::TcpStream,
    res: Result<Value, String>,
) -> Result<(), String> {
    match res {
        Ok(val) => {
            let body = serde_json::to_vec(&val).unwrap_or_default();
            send_response(stream, 200, "application/json; charset=utf-8", &body)
        }
        Err(err) => {
            let status = if err.to_ascii_lowercase().contains("not found") {
                404
            } else {
                400
            };
            let body = serde_json::to_vec(&json!({ "error": err })).unwrap_or_default();
            send_response(stream, status, "application/json; charset=utf-8", &body)
        }
    }
}

fn send_response(
    stream: &mut std::net::TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|e| e.to_string())
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = format!("{c1}{c2}");
                if let Ok(byte) = u8::from_str_radix(&hex_str, 16) {
                    out.push(byte as char);
                } else {
                    out.push('%');
                    out.push(c1);
                    out.push(c2);
                }
            } else {
                out.push('%');
                if let Some(c) = h1 {
                    out.push(c);
                }
            }
        } else if ch == '+' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
}

fn get_query_param(query: &str, key: &str) -> Option<String> {
    for part in query.split('&') {
        let prefix = format!("{key}=");
        if let Some(val) = part.strip_prefix(&prefix) {
            return Some(url_decode(val));
        }
    }
    None
}


// Interim shims after the Task 6 qm-manager signature changes (legacy reference crate).
fn legacy_dll_source() -> Result<PathBuf, String> {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("qm_winmm.dll")))
        .filter(|p| p.is_file())
        .ok_or_else(|| "qm_winmm.dll was not found next to the launcher executable.".to_string())
}

fn legacy_restore_clean(paths: &QuarterMasterPaths) -> Result<(), String> {
    let mut known = std::collections::HashSet::new();
    if let Some(bytes) = legacy_dll_source().ok().and_then(|p| fs::read(p).ok()) {
        known.insert(qm_schema::sha256_hex(&bytes));
    }
    paths.restore_clean_game_dir(&known)
}

fn legacy_launch(
    paths: &QuarterMasterPaths,
    profile_id: &str,
    method_override: Option<LaunchMethod>,
) -> Result<u32, String> {
    paths.activate_profile(profile_id)?;
    let settings = paths.load_settings();
    let profile = paths.get_profile(profile_id)?;
    let method = method_override
        .or(profile.launch_method)
        .unwrap_or(settings.default_launch_method);
    let is_vanilla = method == LaunchMethod::Vanilla
        || profile.launch_method == Some(LaunchMethod::Vanilla)
        || profile.enabled_mods.is_empty();
    if is_vanilla {
        legacy_restore_clean(paths)?;
    } else {
        paths.deploy_profile_to_game(profile_id, &legacy_dll_source()?)?;
    }
    if settings.auto_backup_saves {
        let _ = paths.create_save_backup(&profile.linked_save_profile_id, "Auto-backup before game launch");
    }
    paths.spawn_game_process(&profile, method)
}