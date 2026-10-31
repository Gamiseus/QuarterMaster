use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use qm_core::patch::table::PatchTable;
use qm_manager::{
    decode_profile_code, lint_mod_directory, package_mod_release, register_windows_protocols,
    resolve_load_order, scaffold_new_mod, simulate_and_analyze_patches,
    ConflictResolutionSuggestion, LaunchMethod, QuarterMasterPaths,
};
use qm_schema::{compare_saves, generate_mod_json_schema, generate_qm_toml_schema};
use qm_service::testing::NullSink;
use qm_service::{GameStage, LaunchRequest, QmError, Service, Win32Probe};

fn print_help() {
    println!(
        r#"QuarterMaster (QM) v{} — Tactical Mod Launcher & Patch Engine CLI

USAGE:
  qm <COMMAND> [OPTIONS]

COMMANDS:
  status                              Show game path, version, library, profiles, and save status
  ui                                  Launch the QuarterMaster Desktop Mod Launcher GUI
  launch [--profile <id>] [--vanilla] [--steam|--direct]
                                      Activate profile and launch Mud and Blood
  validate [--mod-dir <dir>] [--json] Resolve load order, run offline patch simulation & 3-tier conflict resolver
  test-patches [--target <file.lua>] [--mod-dir <dir>]
                                      Simulate patches offline and display attributed code lines
  import <path> [--unmanaged]         Import a mod (.zip, folder, or multi-mod pack) into the pristine library
  sync-workspace [<dir>]              Import/sync all mods from MnB-Mods workspace folder into library
  verify-library                      Verify SHA-256 integrity of all mods in the pristine library
  profile list                        List all mod profiles
  profile activate <id>               Atomically activate a mod profile (junction swap + save slot swap)
  profile clone <src> <new_id> <name> Clone an existing mod profile
  profile diff <id_a> <id_b>          Compare two mod profiles (mods, priorities, settings)
  profile export-code <id>            Export a shareable offline profile code (QM1-...)
  profile import-code <code> <new_id> Import a shareable profile code (QM1-...)
  save list                           List all save profiles and backups
  save inspect [<id>]                 Inspect squad, rank, kills, and mod fingerprint of a save slot
  save backup <id> [<label>]          Create a timestamped backup of a save slot
  save restore <id> <backup_id>       Restore a save slot from a backup snapshot
  save compare <id_a> <id_b>          Compare two save profiles side-by-side
  lint <mod_dir>                      Lint mod.json, qm.toml, dry-run patches against vanilla, and scan security
  pack <mod_dir> [--out <dir>]        Package a mod into a release .zip and output its SHA-256 index entry
  new <id> <name> [--template <t>]    Scaffold a new mod (templates: empty, gmenu_config, gameplay_patch)
  watch [--relaunch]                  Watch MnB-Mods/ for file changes, auto-sync, validate & hot-reload
  register-protocols                  Register qm://, nxm://, and .qmpack associations in Windows Registry (HKCU)
  schema [--out <dir>]                Write official JSON Schemas (mod.schema.json, qm.schema.json)
"#,
        qm_core::QM_VERSION
    );
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" || args[0] == "help" {
        print_help();
        return ExitCode::SUCCESS;
    }

    // Handle `qm://` or `nxm://` URL protocol invocation or `--url` / `--import-pack` flags
    if args[0].starts_with("qm://") || args[0].starts_with("nxm://") {
        return handle_url_scheme(&args[0]);
    }
    if args[0] == "--url" {
        if let Some(u) = args.get(1) {
            return handle_url_scheme(u);
        }
    }
    if args[0] == "--import-pack" {
        if let Some(pack_path) = args.get(1) {
            let paths = QuarterMasterPaths::resolve(None);
            match paths.import_qmpack_file(Path::new(pack_path)) {
                Ok(preview) => {
                    let id = preview.manifest.name.to_lowercase().replace(' ', "_");
                    let _ = paths.apply_modpack_as_profile(&preview.manifest, &id);
                    println!("Imported .qmpack '{}' as profile '{}'", pack_path, id);
                    return ExitCode::SUCCESS;
                }
                Err(e) => {
                    eprintln!("Failed to import .qmpack: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    let paths = QuarterMasterPaths::resolve(None);
    let settings = paths.load_settings();

    // Auto-seed library from workspace MnB-Mods if library is empty
    if paths.list_installed_mods().is_empty() {
        if let Some(ref ws_mods) = settings.workspace_mods_dir {
            let _ = paths.sync_workspace_mods(ws_mods);
        }
    }

    let cmd = args[0].as_str();
    let rest = &args[1..];

    let result = match cmd {
        "status" => cmd_status(&paths),
        "ui" | "gui" => cmd_ui(rest),
        "launch" => cmd_launch(&paths, rest),
        "validate" => cmd_validate(&paths, rest),
        "test-patches" => cmd_test_patches(&paths, rest),
        "import" => cmd_import(&paths, rest),
        "sync-workspace" => cmd_sync_workspace(&paths, rest),
        "verify-library" => cmd_verify_library(&paths),
        "profile" => cmd_profile(&paths, rest),
        "save" => cmd_save(&paths, rest),
        "lint" => cmd_lint(&paths, rest),
        "pack" => cmd_pack(rest),
        "new" => cmd_new(rest),
        "watch" => cmd_watch(&paths, rest),
        "register-protocols" => cmd_register_protocols(),
        "schema" => cmd_schema(rest),
        other => Err(format!(
            "Unknown command '{other}'. Run `qm --help` for usage."
        )),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("[ERROR] {e}");
            ExitCode::FAILURE
        }
    }
}

fn handle_url_scheme(url: &str) -> ExitCode {
    println!("[QM URL Handler] Received URL: {url}");
    let paths = QuarterMasterPaths::resolve(None);
    if let Some(code_rest) = url.strip_prefix("qm://import-profile?code=") {
        match decode_profile_code(code_rest) {
            Ok(pack) => {
                let id = pack.name.to_lowercase().replace(' ', "_");
                match paths.apply_modpack_as_profile(&pack, &id) {
                    Ok(p) => {
                        println!("Imported profile '{}' (id: {})", p.name, p.id);
                        return ExitCode::SUCCESS;
                    }
                    Err(e) => {
                        eprintln!("Failed to apply profile code: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            Err(e) => {
                eprintln!("Invalid profile code in URL: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Some(install_url) = url.strip_prefix("qm://install?url=") {
        let p = Path::new(install_url);
        if p.exists() {
            match paths.import_mod_from_path(p, Some(install_url.to_string()), false) {
                Ok(sum) => {
                    println!("Installed {} mod(s) from URL.", sum.imported_mods.len());
                    return ExitCode::SUCCESS;
                }
                Err(e) => {
                    eprintln!("Failed to install from URL: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        println!("Queued remote install URL for QuarterMaster: {install_url}");
        return ExitCode::SUCCESS;
    }
    println!("URL recorded for QuarterMaster Launcher.");
    ExitCode::SUCCESS
}

fn cmd_ui(args: &[String]) -> Result<(), String> {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let launcher_exe = exe_dir.join("qm-launcher.exe");
    let status = std::process::Command::new(&launcher_exe)
        .args(args)
        .status()
        .map_err(|e| format!("Failed to start {}: {e}", launcher_exe.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err("qm-launcher exited with non-zero status".to_string())
    }
}

fn cmd_status(paths: &QuarterMasterPaths) -> Result<(), String> {
    let settings = paths.load_settings();
    println!("========================================================");
    println!("QuarterMaster Field Command — System Status");
    println!("========================================================");
    println!(
        "Data Root      : {} (Portable: {})",
        paths.root_dir.display(),
        paths.is_portable
    );
    println!(
        "Game Executable: {}",
        settings
            .game_exe_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Not found".to_string())
    );
    if let Some(ref exe) = settings.game_exe_path {
        if let Ok(info) = paths.ensure_vanilla_cache(exe) {
            println!(
                "Game Version   : {} (SHA-256: {}..., {} vanilla Lua files cached)",
                info.game_version,
                &info.exe_sha256[..12],
                info.lua_file_count
            );
        }
    }
    println!(
        "Save Directory : {}",
        settings
            .save_dir_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Not found".to_string())
    );

    let mods = paths.list_installed_mods();
    println!("\nInstalled Mods in Library ({}):", mods.len());
    for m in &mods {
        println!(
            "  - {:<20} v{:<8} [Priority {:>3}] ({} patches, category: {})",
            m.manifest.id,
            m.manifest.version,
            m.manifest.load_priority,
            m.patch_count,
            m.manifest.category.as_deref().unwrap_or("Other")
        );
    }

    let profiles = paths.list_profiles();
    println!("\nMod Profiles ({}):", profiles.len());
    for p in &profiles {
        let active_marker = if p.id == settings.active_profile_id {
            "* ACTIVE"
        } else {
            " "
        };
        println!(
            "  {} {:<16} — {} ({} mods enabled, save slot: '{}')",
            active_marker,
            p.id,
            p.name,
            p.enabled_mods.len(),
            p.linked_save_profile_id
        );
    }

    let crashes = paths.list_crash_reports();
    println!("\nRecorded Crash Reports: {}", crashes.len());
    for c in crashes.iter().take(3) {
        println!(
            "  - [{}] {} | Suspected Mod: {:?}",
            c.id, c.error_message, c.suspected_mod_id
        );
    }
    Ok(())
}

fn cmd_launch(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let settings = paths.load_settings();
    let mut profile_id = settings.active_profile_id.clone();
    let mut method_override = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--profile" | "-p" => {
                i += 1;
                if let Some(val) = args.get(i) {
                    profile_id = val.clone();
                }
            }
            "--vanilla" | "-v" => method_override = Some(LaunchMethod::Vanilla),
            "--steam" => method_override = Some(LaunchMethod::SteamUrl),
            "--direct" => method_override = Some(LaunchMethod::DirectExe),
            _ => {}
        }
        i += 1;
    }

    let svc = new_service(paths);
    start_launch(&svc, &profile_id, method_override)?;
    wait_for_game(&svc)
}

fn new_service(paths: &QuarterMasterPaths) -> Service {
    Service::new(paths.clone(), Arc::new(NullSink), Arc::new(Win32Probe))
}

/// Recovers any interrupted game-folder transaction, then launches through the service.
fn start_launch(
    svc: &Service,
    profile_id: &str,
    method: Option<LaunchMethod>,
) -> Result<(), String> {
    // Never recover under a live game; game_status also attaches to a running game.
    if svc.game_status().is_busy() {
        return Err(fmt_err(&QmError::game_running()));
    }
    match svc.recover() {
        Ok(Some(msg)) => println!("{msg}"),
        Ok(None) => {}
        Err(e) => return Err(fmt_err(&e)),
    }
    svc.game_launch(LaunchRequest {
        profile_id: profile_id.to_string(),
        method,
    })
    .map(|_| ())
    .map_err(|e| fmt_err(&e))
}

fn fmt_err(e: &QmError) -> String {
    match &e.details {
        Some(d) if !d.is_empty() => format!("{} ({d})", e.message),
        _ => e.message.clone(),
    }
}

fn describe_stage(stage: &GameStage) -> String {
    match stage {
        GameStage::Idle => "Game closed; game folder is clean.".to_string(),
        GameStage::Deploying => "Preparing the game folder...".to_string(),
        GameStage::WaitingForSteam => "Waiting for Steam to start the game...".to_string(),
        GameStage::Running { pid } => format!("Mud and Blood is running (PID: {pid})."),
        GameStage::CleaningUp => "Game exited; cleaning up the game folder...".to_string(),
        GameStage::Failed { .. } => String::new(),
    }
}

/// Polls the game status every 500 ms, printing each stage change once, until Idle or Failed.
fn wait_for_game(svc: &Service) -> Result<(), String> {
    let mut last: Option<GameStage> = None;
    loop {
        let st = svc.game_status();
        if last.as_ref() != Some(&st.stage) {
            // Failed is reported once by the caller as the error.
            if !matches!(st.stage, GameStage::Failed { .. }) {
                println!("{}", describe_stage(&st.stage));
            }
            last = Some(st.stage.clone());
        }
        match st.stage {
            GameStage::Idle => return Ok(()),
            GameStage::Failed { reason } => return Err(reason),
            _ => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
}

fn cmd_validate(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let settings = paths.load_settings();
    let mut mod_dir_override: Option<PathBuf> = None;
    let mut json_output = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mod-dir" => {
                i += 1;
                if let Some(p) = args.get(i) {
                    mod_dir_override = Some(PathBuf::from(p));
                }
            }
            "--json" => json_output = true,
            _ => {}
        }
        i += 1;
    }

    let (game_ver, vanilla_files) =
        paths.load_vanilla_lua_files(settings.game_exe_path.as_deref())?;

    let target_mod_dir = if let Some(d) = mod_dir_override {
        d
    } else if let Some(ref ws) = settings.workspace_mods_dir {
        ws.clone()
    } else {
        paths
            .profiles_dir
            .join(&settings.active_profile_id)
            .join("Mods")
    };

    let installed = paths.list_installed_mods();
    let manifests: Vec<_> = installed.into_iter().map(|m| m.manifest).collect();
    let load_order = resolve_load_order(
        &manifests,
        &Default::default(),
        &Default::default(),
        Some(&game_ver),
    );

    let table = PatchTable::load(&target_mod_dir).map_err(|e| {
        format!(
            "Failed to load patches from {}: {e:#}",
            target_mod_dir.display()
        )
    })?;
    let sim = simulate_and_analyze_patches(&table, &vanilla_files);

    if json_output {
        let combined = serde_json::json!({
            "game_version": game_ver,
            "load_order": load_order,
            "simulation": sim,
        });
        println!("{}", serde_json::to_string_pretty(&combined).unwrap());
        return Ok(());
    }

    println!("========================================================");
    println!(
        "QuarterMaster Offline Validation & Conflict Report (Game v{})",
        game_ver
    );
    println!("Mod Directory: {}", target_mod_dir.display());
    println!("========================================================");
    println!("\nResolved Topological Load Order:");
    for entry in &load_order.ordered_mods {
        println!(
            "  #{:<2} [Priority {:>3}] {:<20} v{:<8} | {}",
            entry.order_index,
            entry.effective_priority,
            entry.mod_id,
            entry.version,
            entry.reasons.join("; ")
        );
    }

    if !load_order.diagnostics.is_empty() {
        println!("\nDependency & Compatibility Diagnostics:");
        for d in &load_order.diagnostics {
            println!("  [{:?}] {}: {}", d.severity, d.mod_id, d.message);
        }
    }

    println!(
        "\nOffline Patch Simulation Summary: {}/{} patches applied cleanly across {} files ({} broken anchors)",
        sim.applied_patches,
        sim.total_patches,
        sim.files_touched.len(),
        sim.broken_anchors
    );

    for f in &sim.files_touched {
        println!(
            "  - {:<24} ({:>5} -> {:>5} lines) | Mods: {}",
            f.target_file,
            f.original_line_count,
            f.patched_line_count,
            f.mods_touching.join(", ")
        );
    }

    if !sim.conflicts.is_empty() {
        println!(
            "\nDetected Conflicts & 3-Tier Resolutions ({}):",
            sim.conflicts.len()
        );
        for c in &sim.conflicts {
            println!(
                "  [{}] {} — {}",
                c.severity.to_uppercase(),
                c.target_file,
                c.description
            );
            for sug in &c.suggestions {
                match sug {
                    ConflictResolutionSuggestion::SwapModOrder { explanation, .. } => {
                        println!("    -> [Tier 1 Auto-Order Fix] {explanation}");
                    }
                    ConflictResolutionSuggestion::GenerateGluePatch { explanation, .. } => {
                        println!("    -> [Tier 2 1-Click Glue Patch] {explanation}");
                    }
                    ConflictResolutionSuggestion::SelectWinner { explanation, .. } => {
                        println!("    -> [Tier 3 Winner Picker] {explanation}");
                    }
                }
            }
        }
    } else {
        println!("\n✓ Zero broken anchors or errors detected!");
    }

    Ok(())
}

fn cmd_test_patches(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let settings = paths.load_settings();
    let mut target_filter: Option<String> = None;
    let mut mod_dir_override: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--target" | "-t" => {
                i += 1;
                target_filter = args.get(i).cloned();
            }
            "--mod-dir" => {
                i += 1;
                mod_dir_override = args.get(i).map(PathBuf::from);
            }
            _ => {}
        }
        i += 1;
    }

    let (_game_ver, vanilla_files) =
        paths.load_vanilla_lua_files(settings.game_exe_path.as_deref())?;
    let target_mod_dir = mod_dir_override
        .or(settings.workspace_mods_dir)
        .unwrap_or_else(|| {
            paths
                .profiles_dir
                .join(&settings.active_profile_id)
                .join("Mods")
        });

    let table = PatchTable::load(&target_mod_dir).map_err(|e| format!("{e:#}"))?;
    let sim = simulate_and_analyze_patches(&table, &vanilla_files);

    for f in &sim.files_touched {
        if let Some(ref tf) = target_filter {
            if &f.target_file != tf && !f.target_file.ends_with(tf) {
                continue;
            }
        }
        println!(
            "=== {} (Mods: {}) ===",
            f.target_file,
            f.mods_touching.join(", ")
        );
        for rec in &f.records {
            println!(
                "  [{:?}] Prio {:>3} | {:<18} | {:<32} (matched: {}, regions: {:?})",
                rec.status,
                rec.priority,
                rec.mod_id,
                rec.patch_name,
                rec.matched_count,
                rec.regions
            );
        }
    }
    Ok(())
}

fn cmd_import(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let source = args
        .first()
        .ok_or_else(|| "Usage: qm import <path-to-zip-or-folder> [--unmanaged]".to_string())?;
    let allow_unmanaged = args.iter().any(|a| a == "--unmanaged");
    let summary = paths.import_mod_from_path(Path::new(source), None, allow_unmanaged)?;
    println!(
        "Successfully imported {} mod(s) (Multi-mod pack: {}):",
        summary.imported_mods.len(),
        summary.was_multi_mod_pack
    );
    for m in &summary.imported_mods {
        println!(
            "  - {} ({}) v{} -> {}",
            m.manifest.name,
            m.manifest.id,
            m.manifest.version,
            m.install_path.display()
        );
    }
    Ok(())
}

fn cmd_sync_workspace(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let settings = paths.load_settings();
    let dir = args
        .first()
        .map(PathBuf::from)
        .or(settings.workspace_mods_dir)
        .ok_or_else(|| "Workspace mods directory not specified or detected.".to_string())?;
    let synced = paths.sync_workspace_mods(&dir)?;
    println!(
        "Synced {} mod(s) from {} into library:",
        synced.len(),
        dir.display()
    );
    for m in synced {
        println!(
            "  - {} ({}) v{}",
            m.manifest.name, m.manifest.id, m.manifest.version
        );
    }
    Ok(())
}

fn cmd_verify_library(paths: &QuarterMasterPaths) -> Result<(), String> {
    let results = paths.verify_library();
    println!(
        "Verifying SHA-256 integrity of {} library mod(s):",
        results.len()
    );
    for (id, ver, res) in results {
        if res.is_pristine {
            println!("  ✓ {:<20} v{:<8} — PRISTINE", id, ver);
        } else {
            println!(
                "  ! {:<20} v{:<8} — MODIFIED: {:?}, MISSING: {:?}, UNTRACKED: {:?}",
                id, ver, res.modified, res.missing, res.untracked
            );
        }
    }
    Ok(())
}

fn cmd_profile(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" => {
            let settings = paths.load_settings();
            for p in paths.list_profiles() {
                let mark = if p.id == settings.active_profile_id {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{} {:<16} | {:<28} | Mods: {:?} | Save: {}",
                    mark, p.id, p.name, p.enabled_mods, p.linked_save_profile_id
                );
            }
            Ok(())
        }
        "activate" => {
            let id = args
                .get(1)
                .ok_or_else(|| "Usage: qm profile activate <profile_id>".to_string())?;
            let plan = paths.activate_profile(id)?;
            println!(
                "Activated profile '{}' ({} mods linked into game Mods junction).",
                plan.profile_id,
                plan.enabled_mods.len()
            );
            Ok(())
        }
        "clone" => {
            let src = args.get(1).ok_or("Missing source profile ID")?;
            let new_id = args.get(2).ok_or("Missing new profile ID")?;
            let new_name = args.get(3).map(|s| s.as_str()).unwrap_or(new_id);
            let p = paths.clone_profile(src, new_id, new_name)?;
            println!("Cloned profile '{}' -> '{}' ({})", src, p.id, p.name);
            Ok(())
        }
        "diff" => {
            let a = args.get(1).ok_or("Missing profile A ID")?;
            let b = args.get(2).ok_or("Missing profile B ID")?;
            let diff = paths.diff_profiles(a, b)?;
            println!("{}", serde_json::to_string_pretty(&diff).unwrap());
            Ok(())
        }
        "export-code" => {
            let id = args
                .get(1)
                .ok_or("Usage: qm profile export-code <profile_id>")?;
            let code = paths.export_profile_code(id)?;
            println!("{code}");
            Ok(())
        }
        "import-code" => {
            let code = args
                .get(1)
                .ok_or("Usage: qm profile import-code <QM1-...> <new_id>")?;
            let new_id = args.get(2).map(|s| s.as_str()).unwrap_or("imported");
            let manifest = decode_profile_code(code)?;
            let p = paths.apply_modpack_as_profile(&manifest, new_id)?;
            println!(
                "Imported profile '{}' (id: {}) with {} mods.",
                p.name,
                p.id,
                p.enabled_mods.len()
            );
            Ok(())
        }
        other => Err(format!("Unknown profile subcommand '{other}'")),
    }
}

fn cmd_save(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");
    match sub {
        "list" => {
            let settings = paths.load_settings();
            for sp in paths.list_save_profiles() {
                let mark = if sp.id == settings.active_save_profile_id {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{} {:<16} | {:<32} | Vanilla Clean: {:<5} | Backups: {}",
                    mark,
                    sp.id,
                    sp.name,
                    sp.is_vanilla_clean,
                    sp.backups.len()
                );
            }
            Ok(())
        }
        "inspect" => {
            let settings = paths.load_settings();
            let id = args
                .get(1)
                .map(|s| s.as_str())
                .unwrap_or(&settings.active_save_profile_id);
            let summary = paths.inspect_save_profile(id)?;
            println!("========================================================");
            println!("Save Slot Inspection: '{}' ({})", id, summary.file_name);
            println!("========================================================");
            println!(
                "Area of Operations : {} (Day {})",
                summary.area_of_operations, summary.day
            );
            println!("German Commander   : {}", summary.german_commander);
            println!(
                "Campaign Progress  : Rank {} | XP {} | TP {} (Total Spent: {}) | Command {}",
                summary.rank,
                summary.experience,
                summary.tactical_points,
                summary.total_tp_spent,
                summary.command_points
            );
            println!(
                "Campaign Combat    : {} Kills | {} Casualties | Win Streak: {} | Missions: {}",
                summary.campaign_kills,
                summary.campaign_casualties,
                summary.win_streak,
                summary.missions_completed
            );
            println!(
                "Classic Progress   : Rank {} | Max Wave {} | Total Kills {} | Casualties {}",
                summary.classic_rank,
                summary.classic_max_wave,
                summary.classic_total_kills,
                summary.classic_total_casualties
            );
            println!(
                "Deployed Squad ({} soldiers):",
                summary.deployed_squad.len()
            );
            for s in &summary.deployed_squad {
                println!(
                    "  - {:<16} (Age {}, Rank {}, Lvl {}) | Weapon: {:<12} | Kills: {:>3} | HP: {:.0}/{:.0} | Skill: {}",
                    s.name, s.age, s.rank, s.level, s.weapon_tag, s.kills, s.health, s.max_health, s.skill
                );
            }
            if !summary.unknown_mod_keys.is_empty() {
                println!("Mod Option Keys in Save: {:?}", summary.unknown_mod_keys);
            }
            Ok(())
        }
        "backup" => {
            let id = args
                .get(1)
                .ok_or("Usage: qm save backup <save_id> [label]")?;
            let label = args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or("Manual CLI Backup");
            let entry = paths.create_save_backup(id, label)?;
            println!(
                "Created backup '{}' for save slot '{}' ({} bytes)",
                entry.id, id, entry.size_bytes
            );
            Ok(())
        }
        "restore" => {
            let id = args
                .get(1)
                .ok_or("Usage: qm save restore <save_id> <backup_id>")?;
            let bak_id = args
                .get(2)
                .ok_or("Usage: qm save restore <save_id> <backup_id>")?;
            paths.restore_save_backup(id, bak_id)?;
            println!("Restored backup '{}' into save slot '{}'.", bak_id, id);
            Ok(())
        }
        "compare" => {
            let a = args
                .get(1)
                .ok_or("Usage: qm save compare <save_id_a> <save_id_b>")?;
            let b = args
                .get(2)
                .ok_or("Usage: qm save compare <save_id_a> <save_id_b>")?;
            let sa = paths.inspect_save_profile(a)?;
            let sb = paths.inspect_save_profile(b)?;
            let cmp = compare_saves(&sa, &sb);
            println!("{}", serde_json::to_string_pretty(&cmp).unwrap());
            Ok(())
        }
        other => Err(format!("Unknown save subcommand '{other}'")),
    }
}

fn cmd_lint(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let dir = args.first().ok_or("Usage: qm lint <mod_dir>")?;
    let settings = paths.load_settings();
    let (_gv, vanilla_files) = paths.load_vanilla_lua_files(settings.game_exe_path.as_deref())?;
    let report = lint_mod_directory(Path::new(dir), &vanilla_files)?;

    println!("========================================================");
    println!(
        "Mod Lint Report: {} ({}) — {}",
        report.mod_name,
        report.mod_id,
        if report.passed { "PASSED" } else { "FAILED" }
    );
    println!("========================================================");
    println!(
        "Patches Matched: {}/{} (Zero-match anchors: {})",
        report.matched_patches,
        report.patch_count,
        report.zero_match_patches.len()
    );
    for d in &report.diagnostics {
        println!("  [{:?}] {}", d.severity, d.message);
    }
    if !report.security_report.capabilities.is_empty() {
        println!("Security Capabilities Detected:");
        for f in &report.security_report.findings {
            println!(
                "  - [{}] {} ({}:{}) -> `{}`",
                f.severity.to_uppercase(),
                f.capability.label(),
                f.file,
                f.line,
                f.snippet
            );
        }
    } else {
        println!("Security Scan: Pure sandboxed Lua (no OS/FFI/raw-IO capabilities)");
    }

    if !report.passed {
        return Err("Mod failed lint checks.".to_string());
    }
    Ok(())
}

fn cmd_pack(args: &[String]) -> Result<(), String> {
    let mod_dir = args
        .first()
        .ok_or("Usage: qm pack <mod_dir> [--out <dir>]")?;
    let mut out_dir = PathBuf::from(".");
    if args.len() >= 3 && args[1] == "--out" {
        out_dir = PathBuf::from(&args[2]);
    }
    let (zip_path, entry) = package_mod_release(Path::new(mod_dir), &out_dir)?;
    println!("Packaged release archive: {}", zip_path.display());
    println!("{}", serde_json::to_string_pretty(&entry).unwrap());
    Ok(())
}

fn cmd_new(args: &[String]) -> Result<(), String> {
    let mod_id = args.first().ok_or(
        "Usage: qm new <mod_id> <mod_name> [--template <empty|gmenu_config|gameplay_patch>]",
    )?;
    let mod_name = args.get(1).map(|s| s.as_str()).unwrap_or(mod_id);
    let mut template = "gmenu_config";
    let mut author = "Community Modder";
    let mut parent_dir = PathBuf::from(".");

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                if let Some(t) = args.get(i) {
                    template = t.as_str();
                }
            }
            "--author" | "-a" => {
                i += 1;
                if let Some(a) = args.get(i) {
                    author = a.as_str();
                }
            }
            "--dir" | "-d" => {
                i += 1;
                if let Some(d) = args.get(i) {
                    parent_dir = PathBuf::from(d);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let created = scaffold_new_mod(&parent_dir, mod_id, mod_name, author, template)?;
    println!(
        "Scaffolded new mod '{}' (template: '{}') at {}",
        mod_name,
        template,
        created.display()
    );
    Ok(())
}

fn cmd_schema(args: &[String]) -> Result<(), String> {
    let out_dir = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("Quartermaster/schemas"));
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let mod_schema = serde_json::to_string_pretty(&generate_mod_json_schema()).unwrap();
    let qm_schema = serde_json::to_string_pretty(&generate_qm_toml_schema()).unwrap();

    std::fs::write(out_dir.join("mod.schema.json"), mod_schema).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("qm.schema.json"), qm_schema).map_err(|e| e.to_string())?;
    println!(
        "Wrote mod.schema.json and qm.schema.json to {}",
        out_dir.display()
    );
    Ok(())
}

fn cmd_register_protocols() -> Result<(), String> {
    let registered = register_windows_protocols()?;
    println!("Registered Windows URL & File Associations in HKCU\\Software\\Classes:");
    for r in registered {
        println!("  ✓ {r}");
    }
    Ok(())
}

fn cmd_watch(paths: &QuarterMasterPaths, args: &[String]) -> Result<(), String> {
    let once = args.iter().any(|a| a == "--once");
    let relaunch = args.iter().any(|a| a == "--relaunch");
    println!("[QM Watch] Monitoring workspace mods directory for changes...");
    let mut last_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    if once {
        let res = paths.poll_dev_watch_changes(last_ms.saturating_sub(1000));
        println!("{}", serde_json::to_string_pretty(&res).unwrap_or_default());
        return Ok(());
    }

    let relaunch_svc = relaunch.then(|| new_service(paths));
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let res = paths.poll_dev_watch_changes(last_ms);
        if let Some(now_ms) = res["now_epoch_ms"].as_u64() {
            last_ms = now_ms;
        }
        if res["changed"].as_bool() == Some(true) {
            println!("[QM Watch] Change detected: {:?}", res["changed_files"]);
            let _ = cmd_validate(paths, &[]);
            if relaunch {
                let settings = paths.load_settings();
                if let Some(svc) = relaunch_svc.as_ref() {
                    if let Err(e) = start_launch(svc, &settings.active_profile_id, None) {
                        eprintln!("[QM Watch] Relaunch failed: {e}");
                    }
                }
            }
        }
    }
}
