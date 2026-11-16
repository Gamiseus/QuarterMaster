//! Simulation checks that need no line map (plan 2e Phase D §6.1): modules that replace a
//! game file or each other, targets the game resolves by the end of the path, patches that
//! abort their file, and `[vars]` collisions; modules that do not compile and patches that
//! target a module (§6.2). None of them changes game output.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sha2::{Digest, Sha256};

use qm_core::compile::LuaCompiler;
use qm_core::handshake::{LaunchConfig, PatchExecutionRecord, PatchOutcomeStatus};
use qm_core::patch::module::ModulePatch;
use qm_core::patch::order::OrderResult;
use qm_core::patch::table::{PatchTable, TracedApply};
use qm_core::patch::{OnFailurePolicy, Patch, Target};
use qm_core::prepare::{canonical_path, simulate_buffer, PreparedBuffer};

use crate::conflict::{ConflictResolutionSuggestion, DetectedConflict};

/// The warning code on every record of a patch whose file a module replaces (G1).
pub const SHADOWED_BY_MODULE: &str = "shadowed_by_module";

fn conflict(id: String, severity: &str, kind: &str, target_file: &str) -> DetectedConflict {
    DetectedConflict {
        id,
        severity: severity.to_string(),
        kind: kind.to_string(),
        target_file: target_file.to_string(),
        mod_a_id: String::new(),
        mod_b_id: None,
        patch_a_id: String::new(),
        patch_b_id: None,
        description: String::new(),
        suggestions: Vec::new(),
        consumer: None,
        sites_lost: None,
        same_line: None,
    }
}

/// One enabled module patch, in the order the DLL writes it to `package.preload`.
pub(crate) struct ModuleUse<'a> {
    pub name: String,
    pub mod_id: String,
    pub patch_id: String,
    sha: [u8; 32],
    pub module: &'a ModulePatch,
}

pub(crate) fn module_uses<'a>(table: &'a PatchTable, order: &OrderResult) -> Vec<ModuleUse<'a>> {
    // The DLL preloads in `order`; a patch missing from it (never expected) goes last.
    let mut indices: Vec<usize> = order.order.clone();
    let listed: BTreeSet<usize> = indices.iter().copied().collect();
    indices.extend((0..table.patches.len()).filter(|i| !listed.contains(i)));
    // A load_now module is written when its `before` chunk loads, after main.lua preloads
    // the others, so it comes after them (in `order` among themselves).
    let load_now = |i: &usize| {
        matches!(table.patches.get(*i), Some((Patch::Module(m), _, _)) if m.load_now)
    };
    let (later, mut indices): (Vec<usize>, Vec<usize>) = indices.into_iter().partition(load_now);
    indices.extend(later);
    indices
        .into_iter()
        .filter_map(|i| table.patches.get(i))
        .filter_map(|(p, _, path)| match p {
            Patch::Module(m) if !m.disabled => {
                let mod_id = m.mod_id.clone().unwrap_or_else(|| {
                    path.components()
                        .next()
                        .and_then(|c| c.as_os_str().to_str())
                        .unwrap_or("unknown")
                        .to_lowercase()
                });
                let patch_id =
                    m.id.clone()
                        .unwrap_or_else(|| format!("{mod_id}:module:{}", m.name));
                Some(ModuleUse {
                    name: m.name.clone(),
                    mod_id,
                    patch_id,
                    sha: Sha256::digest(m.content.as_bytes()).into(),
                    module: m,
                })
            }
            _ => None,
        })
        .collect()
}

/// A game file a module replaces: `require` gets the module, so the file never runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shadow {
    /// The vanilla file key.
    pub file: String,
    pub module: String,
    pub mod_id: String,
    pub patch_id: String,
}

/// What the module checks found.
pub(crate) struct ModuleReport {
    /// Module name to the mods that register it, in the order the DLL preloads them.
    pub registrations: BTreeMap<String, Vec<String>>,
    pub conflicts: Vec<DetectedConflict>,
    pub shadows: Vec<Shadow>,
}

/// `duplicate_module` (G7: the winner is the module the DLL preloads last; identical modules
/// are not a conflict) and `shadowed_file` (G1: a module named like a game file).
pub(crate) fn module_report(
    table: &PatchTable,
    order: &OrderResult,
    vanilla: &HashMap<String, String>,
) -> ModuleReport {
    let uses = module_uses(table, order);
    let mut by_name: BTreeMap<&str, Vec<&ModuleUse>> = BTreeMap::new();
    for u in &uses {
        by_name.entry(u.name.as_str()).or_default().push(u);
    }
    let mut registrations = BTreeMap::new();
    let mut conflicts = Vec::new();
    let mut shadows = Vec::new();
    for (name, list) in &by_name {
        registrations.insert(
            name.to_string(),
            list.iter().map(|u| u.mod_id.clone()).collect::<Vec<_>>(),
        );
        let Some(winner) = list.last() else {
            continue;
        };
        // The last module with other content than the winner's: the one it replaces.
        if let Some(loser) = list.iter().rev().find(|u| u.sha != winner.sha) {
            let mut c = conflict(
                format!("module-collision:{name}"),
                "warning",
                "duplicate_module",
                name,
            );
            c.mod_a_id = loser.mod_id.clone();
            c.patch_a_id = loser.patch_id.clone();
            c.mod_b_id = Some(winner.mod_id.clone());
            c.patch_b_id = Some(winner.patch_id.clone());
            let owners: Vec<&str> = list.iter().map(|u| u.mod_id.as_str()).collect();
            c.description = format!(
                "Module '{name}' is registered by several mods ({}) with different code; '{}' loads last and replaces the others.",
                owners.join(", "),
                winner.mod_id
            );
            c.suggestions = vec![ConflictResolutionSuggestion::SelectWinner {
                patch_a_id: loser.patch_id.clone(),
                mod_a_id: loser.mod_id.clone(),
                patch_b_id: winner.patch_id.clone(),
                mod_b_id: winner.mod_id.clone(),
                explanation: "Choose which mod provides this Lua module.".to_string(),
            }];
            conflicts.push(c);
        }
        let file = format!("{name}.lua");
        if vanilla.contains_key(&file) {
            let mut c = conflict(
                format!("shadowed:{file}"),
                "warning",
                "shadowed_file",
                &file,
            );
            c.mod_a_id = winner.mod_id.clone();
            c.patch_a_id = winner.patch_id.clone();
            c.description = format!(
                "'{file}' won't run: '{}' replaces this file with its module '{name}'. Patches on this file do not run in the game.",
                winner.mod_id
            );
            conflicts.push(c);
            shadows.push(Shadow {
                file,
                module: name.to_string(),
                mod_id: winner.mod_id.clone(),
                patch_id: winner.patch_id.clone(),
            });
        }
    }
    ModuleReport {
        registrations,
        conflicts,
        shadows,
    }
}

/// Adds the shadow note to every record of the shadowed file.
pub(crate) fn mark_shadowed(records: &mut [PatchExecutionRecord], shadow: &Shadow) {
    let note = format!(
        "{SHADOWED_BY_MODULE}:{}: '{}' from '{}' replaces this file, so this patch does not run in the game",
        shadow.mod_id, shadow.module, shadow.mod_id
    );
    for rec in records {
        if !rec.warnings.contains(&note) {
            rec.warnings.push(note.clone());
        }
    }
}

/// Whether `rec` carries the shadow note.
pub fn is_shadowed(rec: &PatchExecutionRecord) -> bool {
    rec.warnings.iter().any(|w| {
        w.strip_prefix(SHADOWED_BY_MODULE)
            .is_some_and(|r| r.starts_with(':'))
    })
}

/// The conflict kind for a module that does not compile but that another module of the same
/// name replaces, so the game never uses it. A warning: it does not block a launch.
pub const REPLACED_MODULE_COMPILE_ERROR: &str = "replaced_module_compile_error";

/// Compiles every enabled module as the game loads it (§6.2). A module the game uses gives a
/// blocking `compile_error`; one another module replaces gives a warning.
pub(crate) fn module_compile_conflicts(
    compiler: &LuaCompiler,
    table: &PatchTable,
    order: &OrderResult,
) -> Vec<DetectedConflict> {
    let uses = module_uses(table, order);
    let mut conflicts = Vec::new();
    for (i, u) in uses.iter().enumerate() {
        // `require` gets the last plain module of a name; a load_now module always runs.
        let winner = (!u.module.load_now)
            .then(|| {
                uses.get(i + 1..)
                    .unwrap_or_default()
                    .iter()
                    .rev()
                    .find(|w| !w.module.load_now && w.name == u.name)
            })
            .flatten();
        // A replaced copy of the winner's own text: the winner's check covers it.
        if winner.is_some_and(|w| w.sha == u.sha) {
            continue;
        }
        let Err(err) = qm_core::compile::check_module(compiler, u.module) else {
            continue;
        };
        let place = err.line.map_or_else(String::new, |l| format!("line {l}: "));
        let what = format!(
            "Module '{}' from '{}' ({}) does not compile: {place}{}.",
            u.name, u.mod_id, u.module.display_source, err.message
        );
        let mut c = match winner {
            None => {
                let mut c = conflict(
                    format!("compile-module:{}:{}", u.name, u.patch_id),
                    "error",
                    "compile_error",
                    &u.name,
                );
                let when = match u.module.before.as_deref().filter(|_| u.module.load_now) {
                    Some(before) => format!("when it loads '{before}'"),
                    None => "when a mod requires this module".to_string(),
                };
                c.description = format!("{what} The game would stop with a Lua error {when}.");
                c
            }
            Some(w) => {
                let mut c = conflict(
                    format!("compile-module-replaced:{}:{}", u.name, u.patch_id),
                    "warning",
                    REPLACED_MODULE_COMPILE_ERROR,
                    &u.name,
                );
                c.mod_b_id = Some(w.mod_id.clone());
                c.patch_b_id = Some(w.patch_id.clone());
                c.description = format!(
                    "{what} The game does not use it: '{}' replaces it with its own module '{}'.",
                    w.mod_id, w.name
                );
                c
            }
        };
        c.mod_a_id = u.mod_id.clone();
        c.patch_a_id = u.patch_id.clone();
        conflicts.push(c);
    }
    conflicts
}

/// Whether `path` is `name` or ends with `/name`.
fn ends_on_boundary(path: &str, name: &str) -> bool {
    path.strip_suffix(name)
        .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
}

/// `target_is_module`: an enabled patch whose target names no game file but a module's source
/// file (or `<name>.lua`). Modules are not patch targets (D44), so the patch never runs.
pub(crate) fn module_target_warnings(
    table: &PatchTable,
    vanilla: &HashMap<String, String>,
) -> Vec<DetectedConflict> {
    let modules: Vec<(&ModulePatch, String)> = table
        .patches
        .iter()
        .filter_map(|(p, _, path)| match p {
            Patch::Module(m) if !m.disabled => Some((m, owner_of(p, path))),
            _ => None,
        })
        .collect();
    if modules.is_empty() {
        return Vec::new();
    }
    let game_keys: Vec<String> = vanilla.keys().map(|k| canonical_path(k)).collect();
    let mut out = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for (p, _, path) in &table.patches {
        if matches!(p, Patch::Module(_)) || p.is_disabled() {
            continue;
        }
        let mod_id = owner_of(p, path);
        for raw in target_names(p) {
            let t = canonical_path(raw);
            // A target that names a game file patches it, as in the game.
            if t.is_empty() || game_keys.iter().any(|k| k.ends_with(t.as_str())) {
                continue;
            }
            let hit = modules.iter().find(|(m, _)| {
                let source = canonical_path(&m.display_source);
                ends_on_boundary(&source, &t)
                    || (!source.is_empty() && ends_on_boundary(&t, &source))
                    || ends_on_boundary(&format!("{}.lua", m.name), &t)
            });
            let Some((m, module_mod)) = hit else {
                continue;
            };
            let patch_id = p.id().map_or_else(|| format!("{mod_id}:{t}"), str::to_string);
            if !seen.insert((patch_id.clone(), t.clone())) {
                continue;
            }
            let mut c = conflict(
                format!("target-module:{patch_id}:{t}"),
                "warning",
                "target_is_module",
                &t,
            );
            c.mod_a_id = mod_id.clone();
            c.patch_a_id = patch_id.clone();
            c.mod_b_id = Some(module_mod.clone());
            c.patch_b_id = Some(
                m.id.clone()
                    .unwrap_or_else(|| format!("{module_mod}:module:{}", m.name)),
            );
            c.description = format!(
                "The target '{t}' of '{patch_id}' (from '{mod_id}') is the module '{}' ('{}' in '{module_mod}'), not a game file. Modules are not patch targets, so this patch never runs.",
                m.name, m.display_source
            );
            out.push(c);
        }
    }
    out
}

/// The mod of patch `p` loaded from `path`.
fn owner_of(p: &Patch, path: &std::path::Path) -> String {
    p.mod_id().map_or_else(
        || {
            path.components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("unknown")
                .to_lowercase()
        },
        str::to_string,
    )
}

fn names_of(t: &Target) -> Vec<&str> {
    match t {
        Target::Single(s) => vec![s.as_str()],
        Target::Multi(v) => v.iter().map(String::as_str).collect(),
    }
}

fn target_names(p: &Patch) -> Vec<&str> {
    let of = names_of;
    match p {
        Patch::Module(m) => m.before.as_deref().into_iter().collect(),
        Patch::Copy(c) => of(&c.target),
        Patch::Pattern(x) => of(&x.target),
        Patch::Regex(x) => of(&x.target),
    }
}

/// The first patch that names target `t` (canonical), for a conflict's patch A.
fn first_naming(table: &PatchTable, t: &str) -> Option<(String, String)> {
    table.patches.iter().find_map(|(p, _, path)| {
        target_names(p)
            .into_iter()
            .any(|n| canonical_path(n) == t)
            .then(|| {
                let mod_id = owner_of(p, path);
                let patch_id = p
                    .id()
                    .map_or_else(|| format!("{mod_id}:{t}"), str::to_string);
                (mod_id, patch_id)
            })
    })
}

/// The vanilla files the table's targets name, each once (G2, D45): a target names every
/// file whose path ends with it, as the DLL matches chunks. Also the `target_matches_several`
/// and `target_without_boundary` warnings.
pub(crate) fn resolve_targets(
    table: &PatchTable,
    vanilla: &HashMap<String, String>,
) -> (Vec<String>, Vec<DetectedConflict>) {
    let targets: BTreeSet<String> = table
        .targets
        .iter()
        .map(|t| canonical_path(t))
        .filter(|t| !t.is_empty())
        .collect();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let mut warnings = Vec::new();
    for t in &targets {
        let mut hits: Vec<(&String, bool)> = vanilla
            .keys()
            .filter_map(|k| {
                let ck = canonical_path(k);
                ck.strip_suffix(t.as_str())
                    .map(|rest| (k, rest.is_empty() || rest.ends_with('/')))
            })
            .collect();
        hits.sort();
        keys.extend(hits.iter().map(|(k, _)| (*k).clone()));
        let Some((mod_id, patch_id)) = first_naming(table, t) else {
            continue;
        };
        if hits.len() > 1 {
            let files: Vec<&str> = hits.iter().map(|(k, _)| k.as_str()).collect();
            let mut c = conflict(
                format!("target-several:{t}"),
                "warning",
                "target_matches_several",
                t,
            );
            c.mod_a_id = mod_id.clone();
            c.patch_a_id = patch_id.clone();
            c.description = format!(
                "The target '{t}' of '{patch_id}' (from '{mod_id}') names {} game files: {}. The game patches each of them.",
                files.len(),
                files.join(", ")
            );
            warnings.push(c);
        }
        for (k, on_boundary) in &hits {
            if *on_boundary {
                continue;
            }
            let mut c = conflict(
                format!("target-boundary:{t}:{k}"),
                "warning",
                "target_without_boundary",
                k,
            );
            c.mod_a_id = mod_id.clone();
            c.patch_a_id = patch_id.clone();
            c.description = format!(
                "The target '{t}' of '{patch_id}' (from '{mod_id}') also names '{k}', because the game matches the end of the file path. Write the full path to patch only the file you mean."
            );
            warnings.push(c);
        }
    }
    (keys.into_iter().collect(), warnings)
}

/// A run of `target` with every `abort` patch set to `warn`, for the report of a file the game
/// fails to load: the records, and the index of the record that aborts (now `Aborted`).
pub(crate) fn rerun_without_abort(
    table: &PatchTable,
    launch: &LaunchConfig,
    target: &str,
    vanilla: &str,
) -> Option<(PreparedBuffer, TracedApply, Option<usize>)> {
    let mut t = table.clone();
    let mut aborting: Vec<(String, String)> = Vec::new();
    for (p, _, _) in &mut t.patches {
        let names = (
            p.mod_id().unwrap_or_default().to_string(),
            p.id().unwrap_or_default().to_string(),
        );
        let policy = match p {
            Patch::Pattern(x) => Some(&mut x.on_failure),
            Patch::Regex(x) => Some(&mut x.on_failure),
            _ => None,
        };
        if let Some(policy) = policy.filter(|f| **f == OnFailurePolicy::Abort) {
            *policy = OnFailurePolicy::Warn;
            aborting.push(names);
        }
    }
    let prep = t.prepare();
    let (buf, mut traced) = simulate_buffer(&t, &prep, launch, target, vanilla).ok()?;
    let is_aborting = |r: &PatchExecutionRecord| {
        aborting
            .iter()
            .any(|(m, id)| *m == r.mod_id && *id == r.patch_id)
    };
    let mut first = None;
    for (i, rec) in traced.records.iter_mut().enumerate() {
        if !is_aborting(rec) {
            continue;
        }
        rec.on_failure = OnFailurePolicy::Abort;
        let failed = matches!(
            rec.status,
            PatchOutcomeStatus::ZeroMatches | PatchOutcomeStatus::CountMismatch
        );
        if failed && first.is_none() {
            rec.status = PatchOutcomeStatus::Aborted;
            first = Some(i);
        }
    }
    Some((buf, traced, first))
}

/// `patch_aborts_file` (G3): a patch with `on_failure = "abort"` failed, so the game fails to
/// load the file with a Lua error. An error: it blocks a launch.
pub(crate) fn abort_conflict(
    target: &str,
    message: &str,
    rec: Option<&PatchExecutionRecord>,
) -> DetectedConflict {
    let mut c = conflict(
        format!(
            "abort:{target}:{}",
            rec.map_or("?", |r| r.patch_id.as_str())
        ),
        "error",
        "patch_aborts_file",
        target,
    );
    if let Some(r) = rec {
        c.mod_a_id = r.mod_id.clone();
        c.patch_a_id = r.patch_id.clone();
        c.description = format!(
            "Patch '{}' (from '{}') does not match in '{target}' and has on_failure = \"abort\", so the game fails to load '{target}' and stops with a Lua error.",
            r.patch_name, r.mod_id
        );
    } else {
        c.description =
            format!("The game fails to load '{target}' and stops with a Lua error: {message}.");
    }
    c
}

/// `var_collision` (G12): two mods set one `[vars]` name to different values.
pub(crate) fn var_conflicts(table: &PatchTable) -> Vec<DetectedConflict> {
    table
        .var_collisions
        .iter()
        .map(|v| {
            let mut c = conflict(format!("var-collision:{}", v.name), "warning", "var_collision", "");
            let (used_mod, used_value) = &v.used;
            let other = v
                .values
                .iter()
                .find(|(m, val)| m != used_mod && val != used_value)
                .or_else(|| v.values.iter().find(|(m, _)| m != used_mod));
            c.mod_a_id = other.map(|(m, _)| m.clone()).unwrap_or_default();
            c.mod_b_id = Some(used_mod.clone());
            c.patch_a_id = format!("vars:{}", v.name);
            let all: Vec<String> = v
                .values
                .iter()
                .map(|(m, val)| format!("'{val}' ({m})"))
                .collect();
            c.description = format!(
                "Mods set the variable '{}' to different values: {}. The game uses '{used_value}' from '{used_mod}' in every patch.",
                v.name,
                all.join(", ")
            );
            c
        })
        .collect()
}

#[cfg(test)]
#[path = "diagnostics_extra_tests.rs"]
mod tests;
