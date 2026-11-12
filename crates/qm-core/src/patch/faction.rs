//! Faction mods: turns checked `faction.toml` files into patches for the game.
//!
//! Three patches come out when at least one faction is active:
//! - `qm_factions_expected` (top of `main.lua`): which faction each side expects, so the save
//!   cleanup in the loader runtime knows what to keep;
//! - `qm_factions` (in `love.load`, after assets and the save have loaded): the runtime from
//!   `faction_runtime.lua` plus the compiled faction data;
//! - `qm_factions_vehicle_label` (`urb_engine.lua`): shows a reskinned vehicle's name in its
//!   tooltip instead of its tag.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use log::*;
use qm_schema::faction::{self, FactionReport, ResolvedFaction, Side, FACTION_FILE};
use qm_schema::DiagnosticSeverity;
use zip::ZipArchive;

use crate::handshake::LoadOrderPlan;
use crate::patch::{
    InsertPosition, OnFailurePolicy, Patch, PatternPatch, Priority, RegexPatch, Target,
};

/// The game-side runtime, shipped inside QuarterMaster.
pub const FACTION_RUNTIME: &str = include_str!("faction_runtime.lua");

/// Runs after every mod patch at the same anchors.
pub const FACTION_PATCH_PRIORITY: Priority = i32::MAX / 2;

/// A faction that passed its checks, ready to compile.
#[derive(Debug, Clone)]
pub struct LoadedFaction {
    pub mod_id: String,
    pub priority: Priority,
    /// Folder holding `faction.toml` and the files it names.
    pub root: PathBuf,
    pub faction: ResolvedFaction,
}

/// Logs a report's diagnostics against a mod.
pub fn log_report(mod_id: &str, report: &FactionReport) {
    for d in &report.diagnostics {
        match d.severity {
            DiagnosticSeverity::Error => error!("[Faction] {mod_id}: {}", d.message),
            DiagnosticSeverity::Warning => warn!("[Faction] {mod_id}: {}", d.message),
            DiagnosticSeverity::Info => info!("[Faction] {mod_id}: {}", d.message),
        }
    }
    if report.has_errors() {
        error!("[Faction] {mod_id}: faction not loaded because of the errors above.");
    }
}

/// For a zip mod with a `faction.toml`, unpacks the folder that holds it next to the zip
/// (`<Mods>/qm/zip_factions/<zip name>/`) so the runtime can read its files.
pub fn extract_zip_faction(zip_file: &Path) -> Option<PathBuf> {
    let file = fs::File::open(zip_file).ok()?;
    let mut zip = ZipArchive::new(file).ok()?;
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().replace('\\', "/")))
        .collect();
    let prefix = names
        .iter()
        .filter(|n| n.rsplit('/').next() == Some(FACTION_FILE))
        .map(|n| n[..n.len() - FACTION_FILE.len()].to_string())
        .min_by_key(|p| p.len())?;
    let stem = zip_file.file_stem()?.to_string_lossy().to_string();
    let out = zip_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("qm")
        .join("zip_factions")
        .join(stem);
    let _ = fs::remove_dir_all(&out);
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let name = entry.name().replace('\\', "/");
        if name.ends_with('/') || !name.starts_with(&prefix) {
            continue;
        }
        let Some(rel) = entry.enclosed_name().and_then(|p| {
            let p = p.to_string_lossy().replace('\\', "/");
            p.strip_prefix(&prefix).map(|s| s.to_string())
        }) else {
            continue;
        };
        let dest = out.join(&rel);
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut buf = Vec::new();
        // A file that fails here would only show up later as "missing file" in the game log.
        match entry.read_to_end(&mut buf) {
            Ok(_) => {
                if let Err(e) = fs::write(&dest, &buf) {
                    warn!(
                        "[Faction] could not unpack {} from {}: {e}",
                        rel,
                        zip_file.display()
                    );
                }
            }
            Err(e) => warn!(
                "[Faction] could not read {} in {}: {e}",
                rel,
                zip_file.display()
            ),
        }
    }
    Some(out)
}

/// Writes a JSON value as a Lua literal. Strings never contain raw newlines, so the result
/// survives being re-indented by a pattern patch.
pub fn to_lua(v: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match v {
        Value::Null => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else {
                let f = n.as_f64().unwrap_or(0.0);
                if f.is_finite() {
                    out.push_str(&format!("{f:?}"));
                } else {
                    out.push('0');
                }
            }
        }
        Value::String(s) => lua_string(s, out),
        Value::Array(items) => {
            out.push('{');
            for item in items {
                to_lua(item, out);
                out.push(',');
            }
            out.push('}');
        }
        Value::Object(map) => {
            out.push('{');
            for (k, item) in map {
                if item.is_null() {
                    continue;
                }
                out.push('[');
                lua_string(k, out);
                out.push_str("]=");
                to_lua(item, out);
                out.push(',');
            }
            out.push('}');
        }
    }
}

fn lua_string(s: &str, out: &mut String) {
    out.push_str(&lua_quote(s));
}

/// A Lua string literal. Control characters are escaped; UTF-8 text passes through.
pub(crate) fn lua_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 || c as u32 == 127 => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn lua_path(p: &Path) -> String {
    dunce::simplified(p).to_string_lossy().replace('\\', "/")
}

/// How the plan's `asset_exclusions` names a faction.
pub fn faction_source(id: &str) -> String {
    format!("faction:{id}")
}

/// The compiled data table for the runtime. With a plan, each faction also gets `excluded`:
/// the globals (and `folder/stem` voice files) a choice gave to another source (D78).
pub fn faction_data_lua(active: &[LoadedFaction], plan: Option<&LoadOrderPlan>) -> String {
    let mut list = Vec::new();
    for lf in active {
        let mut v = serde_json::to_value(&lf.faction).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("root".into(), lua_path(&lf.root).into());
            obj.insert("mod_id".into(), lf.mod_id.clone().into());
            let excluded = plan.map_or(&[][..], |p| {
                p.asset_exclusions_of(&faction_source(&lf.faction.id))
            });
            if !excluded.is_empty() {
                let set: serde_json::Map<String, serde_json::Value> =
                    excluded.iter().map(|k| (k.clone(), true.into())).collect();
                obj.insert("excluded".into(), set.into());
            }
        }
        list.push(v);
    }
    let mut out = String::new();
    to_lua(&serde_json::json!({ "factions": list }), &mut out);
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
        match_indent: false,
        times: Some(1),
        overwrite: false,
        id: Some(id.to_string()),
        name: Some(name.to_string()),
        description: None,
        requires: Vec::new(),
        priority: Some(FACTION_PATCH_PRIORITY),
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

/// The patches for the active factions (at most one per side). Empty when there are none.
/// `plan` gives what each faction leaves out (`asset_exclusions`).
pub fn build_faction_patches(active: &[LoadedFaction], plan: Option<&LoadOrderPlan>) -> Vec<Patch> {
    if active.is_empty() {
        return Vec::new();
    }

    let mut expected = String::from("_G.__qm_factions_expected = {");
    let mut weapons = String::from("_G.__qm_factions_expected_weapons = {");
    for lf in active {
        expected.push_str(&format!(
            "[{}]={},",
            lua_quote(lf.faction.replaces.as_str()),
            lua_quote(&lf.faction.id)
        ));
        for w in lf.faction.weapons.iter().filter(|w| w.mode == "new") {
            weapons.push_str(&format!("[{}]=true,", lua_quote(&w.tag)));
        }
    }
    expected.push('}');
    weapons.push('}');

    let names: Vec<String> = active
        .iter()
        .map(|lf| format!("{} ({})", lf.faction.name, lf.faction.replaces.label()))
        .collect();
    let mut main_payload = format!(
        "-- QuarterMaster factions: {}\nif not _G.__qm_factions_runtime then\n(function()\n{}\nend)()\nend\n",
        names.join(", "),
        FACTION_RUNTIME
    );
    // The data sits in its own function so its strings don't count against love.load's
    // constant limit.
    main_payload.push_str("_G.__qm_factions_apply((function() return ");
    main_payload.push_str(&faction_data_lua(active, plan));
    main_payload.push_str(" end)())\n");

    let mut patches = vec![
        pattern_patch(
            "qm_factions_expected",
            "Faction save check",
            "main.lua",
            "require \"includes/ai\"",
            InsertPosition::Before,
            format!("{expected}\n{weapons}\n"),
        ),
        pattern_patch(
            "qm_factions",
            &format!("Factions: {}", names.join(", ")),
            "main.lua",
            "tensionLoop:setLooping(true)",
            InsertPosition::Before,
            main_payload,
        ),
    ];

    if active
        .iter()
        .any(|lf| lf.faction.vehicles.iter().any(|v| v.name.is_some()))
    {
        patches.push(regex_patch(
            "qm_factions_vehicle_label",
            "Faction vehicle names",
            "includes/urb_engine.lua",
            r#"player\.deployedVehicles\[id\]\.tag\.\."\|""#,
            r#"(__qm_vehicle_label and __qm_vehicle_label(player.deployedVehicles[id]) or player.deployedVehicles[id].tag).."|""#,
        ));
    }

    // The game picks the firing sound of the Garand and Springfield families by tag, not by
    // weapon.sound. When a faction reskins one of them with its own sound, check first.
    let tag_sound_reskin = active.iter().any(|lf| {
        lf.faction.weapons.iter().any(|w| {
            w.mode == "reskin"
                && (w.sound_name.is_some() || !w.sounds.is_empty())
                && faction::vanilla::TAG_PICKED_SOUNDS.contains(&w.tag.as_str())
        })
    });
    if tag_sound_reskin {
        for (id, target) in [
            ("qm_factions_tag_sound_player", "includes/urb_engine.lua"),
            ("qm_factions_tag_sound_ai", "includes/ai.lua"),
        ] {
            patches.push(regex_patch(
                id,
                "Faction weapon sounds",
                target,
                r#"(?m)^(?P<indent>[ \t]*)if\(player\.deployedTroops\[idz\]\.weapon\.tag == "m1""#,
                "${indent}if(__qm_sound_by_tag and __qm_sound_by_tag[player.deployedTroops[idz].weapon.tag]) then\n${indent}  clone_sfx = _G[player.deployedTroops[idz].weapon.sound]:clone()\n${indent}elseif(player.deployedTroops[idz].weapon.tag == \"m1\"",
            ));
        }
    }
    patches
}

fn regex_patch(id: &str, name: &str, target: &str, pattern: &str, payload: &str) -> Patch {
    Patch::Regex(RegexPatch {
        target: Target::Single(target.to_string()),
        pattern: pattern.to_string(),
        position: InsertPosition::At,
        root_capture: None,
        payload: payload.to_string(),
        line_prepend: String::new(),
        times: Some(1),
        verbose: false,
        id: Some(id.to_string()),
        name: Some(name.to_string()),
        description: None,
        requires: Vec::new(),
        priority: Some(FACTION_PATCH_PRIORITY),
        before: Vec::new(),
        after: Vec::new(),
        on_failure: OnFailurePolicy::Warn,
        anchors: Vec::new(),
        sites: Vec::new(),
        stack: None,
        mod_id: Some("quartermaster".to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

/// Checks a mod's faction (if any) and returns it when it loads cleanly. Logs everything.
pub fn load_mod_faction(mod_id: &str, priority: Priority, root: &Path) -> Option<LoadedFaction> {
    if !faction::has_faction_file(root) {
        return None;
    }
    let report = faction::load_faction(root);
    log_report(mod_id, &report);
    report.faction.map(|faction| LoadedFaction {
        mod_id: mod_id.to_string(),
        priority,
        root: root.to_path_buf(),
        faction,
    })
}

/// One faction per side, and why each other one is left out. Logs nothing (previews, D73).
pub fn pick_active_quiet(candidates: Vec<LoadedFaction>) -> (Vec<LoadedFaction>, Vec<String>) {
    let picked = pick_active_for(candidates, None);
    (picked.active, picked.skipped)
}

/// The factions that run, one per side.
#[derive(Debug, Clone, Default)]
pub struct FactionPick {
    pub active: Vec<LoadedFaction>,
    /// Why each other faction is left out.
    pub skipped: Vec<String>,
    /// A pick that could not be honoured; the mod priority decided instead.
    pub notes: Vec<String>,
}

/// One faction per side. A side the plan picks a faction for (`factions`, D82) gets that
/// faction whatever the priorities are; a pick that names no loaded faction of that side is
/// noted and the highest priority wins, as it does without a pick. Logs nothing.
pub fn pick_active_for(candidates: Vec<LoadedFaction>, plan: Option<&LoadOrderPlan>) -> FactionPick {
    let mut out = FactionPick::default();
    let mut pool = candidates;
    for side in [Side::Allied, Side::Axis] {
        let Some(want) = plan.and_then(|p| p.faction_pick(side.as_str())) else {
            continue;
        };
        let is_pick = |lf: &LoadedFaction| lf.faction.replaces == side && lf.faction.id == want;
        let Some(winner) = pool.iter().find(|lf| is_pick(lf)).map(|lf| lf.mod_id.clone()) else {
            if pool.iter().any(|lf| lf.faction.replaces == side) {
                out.notes.push(format!(
                    "The faction picked for the {} ('{want}') is not enabled or did not load; the mod priority decides instead.",
                    side.label()
                ));
            }
            continue;
        };
        pool.retain(|lf| {
            let keep = lf.faction.replaces != side || lf.mod_id == winner;
            if !keep {
                out.skipped.push(format!(
                    "Faction mod '{}' also replaces the {}; '{winner}' is picked for this profile.",
                    lf.mod_id,
                    side.label()
                ));
            }
            keep
        });
    }
    let keyed: Vec<(i32, String, Side, LoadedFaction)> = pool
        .into_iter()
        .map(|lf| (lf.priority, lf.mod_id.clone(), lf.faction.replaces, lf))
        .collect();
    let (active, skipped) = faction::pick_active(&keyed);
    out.active = active.into_iter().map(|(_, lf)| lf).collect();
    out.skipped.extend(skipped);
    out
}

/// One faction per side (the plan's pick, else the highest priority); logs the ones left
/// out. Returns the active factions and the picks that could not be honoured.
pub fn choose_active(
    candidates: Vec<LoadedFaction>,
    plan: Option<&LoadOrderPlan>,
) -> (Vec<LoadedFaction>, Vec<String>) {
    let FactionPick { active, skipped, notes } = pick_active_for(candidates, plan);
    for msg in skipped.iter().chain(&notes) {
        warn!("[Faction] {msg}");
    }
    for lf in &active {
        info!(
            "[Faction] {} ({}) replaces the {}.",
            lf.faction.name,
            lf.mod_id,
            lf.faction.replaces.label()
        );
    }
    (active, notes)
}

#[cfg(test)]
#[path = "faction_tests.rs"]
mod tests;
