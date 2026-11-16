use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use qm_schema::{DiagnosticSeverity, ModManifest, ParsedVersion, VersionReq};

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedModOrderEntry {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub effective_priority: i32,
    pub order_index: usize,
    /// Human-readable "Why is this mod at position #N?" explanation strings.
    pub reasons: Vec<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverDiagnostic {
    pub severity: DiagnosticSeverity,
    pub mod_id: String,
    pub kind: String, // "missing_dependency" | "version_mismatch" | "conflict" | "game_version" | "cycle" | "recommendation" | "patch_cycle" | "qm_gate" | "qm_gate_requirer"
    pub message: String,
    pub related_mod_id: Option<String>,
}

/// One `patch_cycle` warning per order cycle the engine broke (`qm_core::patch::order`).
/// `cycle` holds patch names (`mod_id:patch_id`) in order: each was meant to run before the next.
pub fn patch_cycle_diagnostics(cycles: &[Vec<String>]) -> Vec<SolverDiagnostic> {
    let mod_of = |name: &str| name.split(':').next().unwrap_or(name).to_string();
    cycles
        .iter()
        .filter(|c| !c.is_empty())
        .map(|c| SolverDiagnostic {
            severity: DiagnosticSeverity::Warning,
            mod_id: mod_of(&c[0]),
            kind: "patch_cycle".to_string(),
            message: format!(
                "These patches each have to run before the next, which cannot all hold: {} (and back to the first). One of these order rules was ignored.",
                c.join(", then ")
            ),
            related_mod_id: c.get(1).map(|n| mod_of(n)),
        })
        .collect()
}

/// D61: an error for every enabled mod whose mod.json `min_qm` or `qm_features` this
/// QuarterMaster cannot meet (`qm_core::features`), and one for every enabled mod that
/// requires such a mod, directly or through another skipped mod (by id or `provides`, case
/// insensitive, unless a mod still loaded offers it). The DLL's loader skips the same mods.
pub fn gate_diagnostics(
    enabled: &[ModManifest],
    qm_version: &str,
    features: &[&str],
) -> Vec<SolverDiagnostic> {
    let keys = |m: &ModManifest| -> Vec<String> {
        std::iter::once(&m.id)
            .chain(&m.relationships.provides)
            .map(|k| k.to_lowercase())
            .collect()
    };
    let mut out = Vec::new();
    let mut skipped = vec![false; enabled.len()];
    // Lost key -> the gated mod it traces back to.
    let mut lost: HashMap<String, &ModManifest> = HashMap::new();
    for (m, skip) in enabled.iter().zip(skipped.iter_mut()) {
        let Some(why) = qm_core::features::gate_error_with(m, qm_version, features) else {
            continue;
        };
        *skip = true;
        for k in keys(m) {
            lost.entry(k).or_insert(m);
        }
        out.push(SolverDiagnostic {
            severity: DiagnosticSeverity::Error,
            mod_id: m.id.clone(),
            kind: "qm_gate".to_string(),
            message: format!("'{}' {why}. The game skips it.", m.name),
            related_mod_id: None,
        });
    }
    loop {
        let available: HashSet<String> = enabled
            .iter()
            .zip(&skipped)
            .filter(|(_, s)| !**s)
            .flat_map(|(m, _)| keys(m))
            .collect();
        let mut changed = false;
        for (m, skip) in enabled.iter().zip(skipped.iter_mut()) {
            if *skip {
                continue;
            }
            let mut requires: Vec<String> =
                m.relationships.requires.keys().map(|k| k.to_lowercase()).collect();
            requires.sort();
            let Some(root) = requires
                .iter()
                .filter(|k| !available.contains(*k))
                .find_map(|k| lost.get(k).copied())
            else {
                continue;
            };
            *skip = true;
            changed = true;
            for k in keys(m) {
                lost.entry(k).or_insert(root);
            }
            out.push(SolverDiagnostic {
                severity: DiagnosticSeverity::Error,
                mod_id: m.id.clone(),
                kind: "qm_gate_requirer".to_string(),
                message: format!(
                    "'{}' requires '{}', which this QuarterMaster cannot run, so the game skips it too.",
                    m.name, root.name
                ),
                related_mod_id: Some(root.id.clone()),
            });
        }
        if !changed {
            return out;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityRule {
    pub mod_id: String,
    pub load_after: Vec<String>,
    pub recommended_priority: Option<i32>,
    pub note: String,
}

/// Built-in LOOT-style Community Masterlist for known Mud and Blood mods (Section 1 & 7).
/// Sits below explicit user rules and manifest dependencies, and above default mod list order.
pub fn default_community_masterlist() -> Vec<CommunityRule> {
    vec![
        CommunityRule {
            mod_id: "gcore".to_string(),
            load_after: vec![],
            recommended_priority: Some(5),
            note: "Core shared utility framework; must load first.".to_string(),
        },
        CommunityRule {
            mod_id: "gmenu".to_string(),
            load_after: vec!["gcore".to_string()],
            recommended_priority: Some(10),
            note: "In-game Mod Configuration Menu; loads immediately after GCore.".to_string(),
        },
        CommunityRule {
            mod_id: "gparticles".to_string(),
            load_after: vec!["gcore".to_string(), "gmenu".to_string()],
            recommended_priority: Some(20),
            note: "Particle & visual effects library; loads before gameplay event mods.".to_string(),
        },
        CommunityRule {
            mod_id: "g_special_events".to_string(),
            load_after: vec!["gcore".to_string(), "gmenu".to_string()],
            recommended_priority: Some(30),
            note: "Campaign & Classic special events expansion.".to_string(),
        },
        CommunityRule {
            mod_id: "gconsole".to_string(),
            load_after: vec!["gcore".to_string(), "gmenu".to_string()],
            recommended_priority: Some(90),
            note: "Developer console hooks; loads late so it can inspect all loaded modules.".to_string(),
        },
        CommunityRule {
            mod_id: "gdumper".to_string(),
            load_after: vec!["gcore".to_string(), "gconsole".to_string()],
            recommended_priority: Some(95),
            note: "Diagnostic file dumper; loads last.".to_string(),
        },
    ]
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadOrderResolution {
    pub ordered_mods: Vec<ResolvedModOrderEntry>,
    pub diagnostics: Vec<SolverDiagnostic>,
    pub has_cycles: bool,
    pub cycle_paths: Vec<Vec<String>>,
    pub rule_hierarchy: String,
}

impl Default for LoadOrderResolution {
    fn default() -> Self {
        Self {
            ordered_mods: Vec::new(),
            diagnostics: Vec::new(),
            has_cycles: false,
            cycle_paths: Vec::new(),
            rule_hierarchy:
                "patch-level override > explicit user rule > mod dependency > community masterlist > mod list order"
                    .to_string(),
        }
    }
}

/// Resolves the topological load order and validates dependencies, virtual `provides`,
/// conflicts, and game-version compatibility.
pub fn resolve_load_order(
    enabled_manifests: &[ModManifest],
    user_priorities: &HashMap<String, i32>,
    user_load_after: &HashMap<String, Vec<String>>,
    detected_game_version: Option<&str>,
) -> LoadOrderResolution {
    let mut diagnostics = Vec::new();
    let parsed_game_ver = detected_game_version.and_then(|v| ParsedVersion::parse(v).ok());

    // Map mod_id -> manifest, and virtual provides -> provider mod_id
    let mut by_id: HashMap<String, &ModManifest> = HashMap::new();
    let mut providers: HashMap<String, String> = HashMap::new();
    for m in enabled_manifests {
        by_id.insert(m.id.clone(), m);
        providers.insert(m.id.clone(), m.id.clone());
        for virt in &m.relationships.provides {
            providers.entry(virt.clone()).or_insert_with(|| m.id.clone());
        }
    }

    // 1. Validate QuarterMaster gates, dependencies, conflicts, and game_version
    diagnostics.extend(gate_diagnostics(
        enabled_manifests,
        qm_core::QM_VERSION,
        qm_core::QM_FEATURES,
    ));
    for m in enabled_manifests {
        // Check game_version compatibility
        if let (Some(ref req_str), Some(ref actual_gv)) = (&m.game_version, &parsed_game_ver) {
            if let Ok(req) = VersionReq::parse(req_str) {
                if !req.matches(actual_gv) {
                    diagnostics.push(SolverDiagnostic {
                        severity: DiagnosticSeverity::Warning,
                        mod_id: m.id.clone(),
                        kind: "game_version".to_string(),
                        message: format!(
                            "'{}' expects game version '{}', but installed Mud and Blood is '{}'.",
                            m.name, req_str, actual_gv
                        ),
                        related_mod_id: None,
                    });
                }
            }
        }

        // Check hard dependencies (`requires`)
        for (dep_key, range_str) in &m.relationships.requires {
            match providers.get(dep_key).and_then(|id| by_id.get(id)) {
                None => {
                    diagnostics.push(SolverDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        mod_id: m.id.clone(),
                        kind: "missing_dependency".to_string(),
                        message: format!(
                            "'{}' requires '{}' ({}), which is not enabled.",
                            m.name, dep_key, range_str
                        ),
                        related_mod_id: Some(dep_key.clone()),
                    });
                }
                Some(dep_manifest) => {
                    if let (Ok(req), Ok(dep_ver)) = (
                        VersionReq::parse(range_str),
                        ParsedVersion::parse(&dep_manifest.version),
                    ) {
                        if !req.matches(&dep_ver) {
                            diagnostics.push(SolverDiagnostic {
                                severity: DiagnosticSeverity::Error,
                                mod_id: m.id.clone(),
                                kind: "version_mismatch".to_string(),
                                message: format!(
                                    "'{}' requires '{}' {}, but version {} is enabled.",
                                    m.name, dep_manifest.name, range_str, dep_manifest.version
                                ),
                                related_mod_id: Some(dep_manifest.id.clone()),
                            });
                        }
                    }
                }
            }
        }

        // Check soft recommendations (`recommends`)
        for (rec_key, range_str) in &m.relationships.recommends {
            if !providers.contains_key(rec_key) {
                diagnostics.push(SolverDiagnostic {
                    severity: DiagnosticSeverity::Info,
                    mod_id: m.id.clone(),
                    kind: "recommendation".to_string(),
                    message: format!(
                        "'{}' recommends enabling '{}' ({}).",
                        m.name, rec_key, range_str
                    ),
                    related_mod_id: Some(rec_key.clone()),
                });
            }
        }

        // Check incompatibilities (`conflicts`)
        for (conf_key, range_str) in &m.relationships.conflicts {
            if let Some(conf_manifest) = providers.get(conf_key).and_then(|id| by_id.get(id)) {
                let matches_range = match (
                    VersionReq::parse(range_str),
                    ParsedVersion::parse(&conf_manifest.version),
                ) {
                    (Ok(req), Ok(ver)) => req.matches(&ver),
                    _ => true,
                };
                if matches_range {
                    diagnostics.push(SolverDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        mod_id: m.id.clone(),
                        kind: "conflict".to_string(),
                        message: format!(
                            "'{}' conflicts with '{}' ({}).",
                            m.name, conf_manifest.name, range_str
                        ),
                        related_mod_id: Some(conf_manifest.id.clone()),
                    });
                }
            }
        }
    }

    // 2. Build directed graph: edge `A -> B` means `A` MUST load before `B`
    let mut adj: HashMap<String, HashSet<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut edge_reasons: HashMap<(String, String), String> = HashMap::new();

    for m in enabled_manifests {
        adj.entry(m.id.clone()).or_default();
        in_degree.entry(m.id.clone()).or_insert(0);
    }

    let mut add_edge = |before_id: &str, after_id: &str, reason: String| {
        if before_id == after_id || !by_id.contains_key(before_id) || !by_id.contains_key(after_id)
        {
            return;
        }
        let targets = adj.entry(before_id.to_string()).or_default();
        if targets.insert(after_id.to_string()) {
            *in_degree.entry(after_id.to_string()).or_insert(0) += 1;
            edge_reasons.insert((before_id.to_string(), after_id.to_string()), reason);
        }
    };

    for m in enabled_manifests {
        for after_dep in &m.relationships.load_after {
            if let Some(provider_id) = providers.get(after_dep) {
                let is_req = m.relationships.requires.contains_key(after_dep);
                let reason = if is_req {
                    format!("Loads after '{}' (required dependency)", provider_id)
                } else {
                    format!("Loads after '{}' (manifest load_after rule)", provider_id)
                };
                add_edge(provider_id, &m.id, reason);
            }
        }
        for before_dep in &m.relationships.load_before {
            if let Some(provider_id) = providers.get(before_dep) {
                add_edge(
                    &m.id,
                    provider_id,
                    format!("Loads after '{}' (requested by '{}' load_before)", m.id, m.name),
                );
            }
        }
    }

    // Apply LOOT-style Community Masterlist rules (below explicit user rules and manifest dependencies)
    let masterlist = default_community_masterlist();
    for rule in &masterlist {
        if by_id.contains_key(&rule.mod_id) {
            for after_dep in &rule.load_after {
                if let Some(provider_id) = providers.get(after_dep) {
                    // Only add if user_load_after didn't explicitly reverse it
                    let user_reversed = user_load_after
                        .get(provider_id)
                        .map(|list| list.contains(&rule.mod_id))
                        .unwrap_or(false);
                    if !user_reversed {
                        add_edge(
                            provider_id,
                            &rule.mod_id,
                            format!(
                                "Loads after '{}' (community masterlist: {})",
                                provider_id, rule.note
                            ),
                        );
                    }
                }
            }
        }
    }

    // Apply user custom `load_after` rules (highest precedence)
    for (mod_id, after_list) in user_load_after {
        for target_before in after_list {
            add_edge(
                target_before,
                mod_id,
                format!("Loads after '{}' (explicit user rule)", target_before),
            );
        }
    }

    // Compute effective priority per mod
    let mut effective_prio: HashMap<String, i32> = HashMap::new();
    for m in enabled_manifests {
        let p = user_priorities
            .get(&m.id)
            .copied()
            .unwrap_or(m.load_priority);
        effective_prio.insert(m.id.clone(), p);
    }

    // 3. Kahn's topological sort with min-heap keyed by `(effective_priority, mod_id)`
    let mut heap: BinaryHeap<Reverse<(i32, String)>> = BinaryHeap::new();
    for (id, &deg) in &in_degree {
        if deg == 0 {
            heap.push(Reverse((effective_prio[id], id.clone())));
        }
    }

    let mut ordered_ids: Vec<String> = Vec::with_capacity(enabled_manifests.len());
    while let Some(Reverse((_, curr))) = heap.pop() {
        ordered_ids.push(curr.clone());
        if let Some(neighbors) = adj.get(&curr) {
            let mut sorted_neighbors: Vec<&String> = neighbors.iter().collect();
            sorted_neighbors.sort();
            for next in sorted_neighbors {
                if let Some(deg) = in_degree.get_mut(next) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        heap.push(Reverse((effective_prio[next], next.clone())));
                    }
                }
            }
        }
    }

    // 4. Handle cycles if `ordered_ids.len() < enabled_manifests.len()`
    let mut has_cycles = false;
    let mut cycle_paths = Vec::new();
    if ordered_ids.len() < enabled_manifests.len() {
        has_cycles = true;
        let remaining: Vec<String> = enabled_manifests
            .iter()
            .map(|m| m.id.clone())
            .filter(|id| !ordered_ids.contains(id))
            .collect();

        cycle_paths.push(remaining.clone());
        diagnostics.push(SolverDiagnostic {
            severity: DiagnosticSeverity::Error,
            mod_id: remaining.first().cloned().unwrap_or_default(),
            kind: "cycle".to_string(),
            message: format!(
                "Dependency cycle detected among mods: {}. Breaking cycle by base priority.",
                remaining.join(" -> ")
            ),
            related_mod_id: remaining.get(1).cloned(),
        });

        let mut fallback = remaining;
        fallback.sort_by_key(|id| (effective_prio[id], id.clone()));
        ordered_ids.extend(fallback);
    }

    // 5. Build `ResolvedModOrderEntry` with "Why is this mod here?" provenance
    let mut ordered_mods = Vec::with_capacity(ordered_ids.len());
    for (idx, id) in ordered_ids.iter().enumerate() {
        let m = by_id[id];
        let prio = effective_prio[id];
        let mut reasons = Vec::new();
        if user_priorities.contains_key(id) {
            reasons.push(format!(
                "User priority override: {} (manifest default: {})",
                prio, m.load_priority
            ));
        } else {
            reasons.push(format!("Base priority: {} (ascending)", prio));
        }

        for ((_from, to), edge_reason) in &edge_reasons {
            if to == id {
                reasons.push(edge_reason.clone());
            }
        }

        ordered_mods.push(ResolvedModOrderEntry {
            mod_id: id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            effective_priority: prio,
            order_index: idx + 1,
            reasons,
        });
    }

    LoadOrderResolution {
        ordered_mods,
        diagnostics,
        has_cycles,
        cycle_paths,
        rule_hierarchy:
            "patch-level override > explicit user rule > mod dependency > community masterlist > mod list order"
                .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_mnb_mods_topologically_and_explains_provenance() {
        let gcore = ModManifest::from_json_str(
            r#"{"id":"gcore","name":"GCore","version":"1.0.0","load_priority":5,"game_version":">=0.5.3"}"#,
        )
        .unwrap();
        let gmenu = ModManifest::from_json_str(
            r#"{"id":"gmenu","name":"GMenu","version":"1.0.0","load_priority":10,"relationships":{"load_after":["gcore"]}}"#,
        )
        .unwrap();
        let gse = ModManifest::from_json_str(
            r#"{"id":"g_special_events","name":"GSpecialEvents","version":"1.0.0","load_priority":30,"relationships":{"requires":{"gcore":">=1.0.0","gmenu":">=1.0.0"}}}"#,
        )
        .unwrap();

        let res = resolve_load_order(
            &[gse, gmenu, gcore],
            &HashMap::new(),
            &HashMap::new(),
            Some("0.5.3b"),
        );

        assert!(!res.has_cycles);
        assert!(res.diagnostics.is_empty());
        let ids: Vec<&str> = res.ordered_mods.iter().map(|m| m.mod_id.as_str()).collect();
        assert_eq!(ids, vec!["gcore", "gmenu", "g_special_events"]);
        assert!(res.ordered_mods[2]
            .reasons
            .iter()
            .any(|r| r.contains("Loads after 'gcore'")));
    }

    const GATED: &[(&str, &str)] = &[
        ("Gated", r#"{"id":"gated","name":"Gated","version":"1.0.0","relationships":{"provides":["gated-api"]}GATES}"#),
        ("NeedsGated", r#"{"id":"needs_gated","name":"Needs","version":"1.0.0","relationships":{"requires":{"Gated":"*"}}}"#),
        ("NeedsApi", r#"{"id":"needs_api","name":"Api","version":"1.0.0","relationships":{"requires":{"gated-api":"*"}}}"#),
        ("NeedsNeeds", r#"{"id":"needs_needs","name":"Deep","version":"1.0.0","relationships":{"requires":{"needs_gated":"*"}}}"#),
        ("Other", r#"{"id":"other","name":"Other","version":"1.0.0"}"#),
    ];

    fn gated_manifests(gates: &str) -> Vec<(&'static str, String)> {
        GATED
            .iter()
            .map(|(folder, json)| (*folder, json.replace("GATES", gates)))
            .collect()
    }

    /// Mod id -> (kind, related) of every gate diagnostic.
    fn gates_of(diags: &[SolverDiagnostic]) -> Vec<(String, String, Option<String>)> {
        let mut v: Vec<_> = diags
            .iter()
            .filter(|d| d.kind.starts_with("qm_gate"))
            .map(|d| (d.mod_id.clone(), d.kind.clone(), d.related_mod_id.clone()))
            .collect();
        v.sort();
        v
    }

    fn resolve(manifests: &[ModManifest]) -> LoadOrderResolution {
        resolve_load_order(manifests, &HashMap::new(), &HashMap::new(), Some("0.5.3b"))
    }

    #[test]
    fn the_solver_reports_the_same_gates_as_the_loader() {
        // A gated mod with no content (only a mod.json, a library others require) fails the
        // same way as one with patches.
        for (gates, contentless) in [
            r#","qm_features":["overrides","teleport"]"#,
            r#","min_qm":"999.0.0""#,
            r#","min_qm":"soon""#,
            r#","min_qm":"1.0.0","qm_features":["anchors"]"#,
        ]
        .into_iter()
        .flat_map(|g| [(g, false), (g, true)])
        {
            let manifests = gated_manifests(gates);
            let temp = tempfile::tempdir().unwrap();
            for (folder, json) in &manifests {
                let dir = temp.path().join(folder);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join("mod.json"), json).unwrap();
                if contentless && *folder == "Gated" {
                    continue;
                }
                let toml = format!(
                    "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.copy]\ntarget = \"{folder}.lua\"\nposition = \"append\"\npayload = \"-- x\"\n"
                );
                std::fs::write(dir.join("qm.toml"), toml).unwrap();
            }
            let raw = qm_core::patch::plan::load_raw(temp.path(), None).unwrap();
            let mut dll: Vec<(String, String, Option<String>)> = Vec::new();
            for e in &raw.load_errors {
                dll.push((e.mod_id.clone(), "qm_gate".into(), None));
                for d in &e.skipped_dependents {
                    dll.push((d.clone(), "qm_gate_requirer".into(), Some(e.mod_id.clone())));
                }
            }
            dll.sort();

            let parsed: Vec<ModManifest> = manifests
                .iter()
                .map(|(_, j)| ModManifest::from_json_str(j).unwrap())
                .collect();
            let res = resolve(&parsed);
            assert_eq!(gates_of(&res.diagnostics), dll, "{gates} {contentless}");

            // The messages agree too.
            if let Some(e) = raw.load_errors.first() {
                let d = res.diagnostics.iter().find(|d| d.kind == "qm_gate").unwrap();
                assert_eq!(d.severity, DiagnosticSeverity::Error);
                assert_eq!(d.message, format!("'Gated' {}. The game skips it.", e.message));
            }
        }
    }

    #[test]
    fn a_failed_gate_fails_the_mod_and_its_requirers() {
        let manifests: Vec<ModManifest> = gated_manifests(r#","qm_features":["teleport"]"#)
            .iter()
            .map(|(_, j)| ModManifest::from_json_str(j).unwrap())
            .collect();
        let res = resolve(&manifests);
        let g = |id: &str, kind: &str, rel: Option<&str>| {
            (id.to_string(), kind.to_string(), rel.map(str::to_string))
        };
        assert_eq!(
            gates_of(&res.diagnostics),
            [
                g("gated", "qm_gate", None),
                g("needs_api", "qm_gate_requirer", Some("gated")),
                g("needs_gated", "qm_gate_requirer", Some("gated")),
                g("needs_needs", "qm_gate_requirer", Some("gated")),
            ]
        );
        let d = res.diagnostics.iter().find(|d| d.kind == "qm_gate").unwrap();
        assert_eq!(
            d.message,
            "'Gated' needs the QuarterMaster feature 'teleport', which this version does not have. The game skips it."
        );
    }

    #[test]
    fn an_older_feature_list_fails_a_mod_that_needs_a_newer_feature() {
        let m = ModManifest::from_json_str(
            r#"{"id":"c","name":"C","version":"1.0.0","qm_features":["overrides","apply_as"]}"#,
        )
        .unwrap();
        let now = gate_diagnostics(std::slice::from_ref(&m), qm_core::QM_VERSION, qm_core::QM_FEATURES);
        assert!(now.is_empty(), "{now:?}");
        let old: Vec<&str> = qm_core::QM_FEATURES
            .iter()
            .copied()
            .filter(|f| *f != "apply_as")
            .collect();
        let then = gate_diagnostics(&[m], qm_core::QM_VERSION, &old);
        assert_eq!(then.len(), 1);
        assert!(then[0].message.contains("'apply_as'"), "{}", then[0].message);
    }

    #[test]
    fn a_requirer_with_another_provider_still_loads() {
        let gated = ModManifest::from_json_str(
            r#"{"id":"gated","name":"G","version":"1.0.0","qm_features":["teleport"],"relationships":{"provides":["api"]}}"#,
        )
        .unwrap();
        let alt = ModManifest::from_json_str(
            r#"{"id":"alt","name":"A","version":"1.0.0","relationships":{"provides":["api"]}}"#,
        )
        .unwrap();
        let user = ModManifest::from_json_str(
            r#"{"id":"user","name":"U","version":"1.0.0","relationships":{"requires":{"api":"*"}}}"#,
        )
        .unwrap();
        let d = gate_diagnostics(&[gated, alt, user], qm_core::QM_VERSION, qm_core::QM_FEATURES);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].mod_id, "gated");
    }
}
