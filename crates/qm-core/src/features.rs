//! Feature gates for mods (D61): mod.json `min_qm` and `qm_features`.
//!
//! The DLL loader (`patch::loader`) and the launcher's solver both call [`gate_error`], so
//! they fail the same mods with the same message.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use qm_schema::{ModManifest, ParsedVersion, VersionReq};

use crate::QM_VERSION;

/// The mod.json and qm.toml features this QuarterMaster supports. A mod lists the ones it
/// needs in `qm_features`; a QuarterMaster without one of them skips the mod.
pub const QM_FEATURES: &[&str] = &[
    "overrides",
    "anchors",
    "stack_hint",
    "stack_rules",
    "apply_as",
    "skip_hunks",
    "run_as",
    "regex_anchors",
    "min_qm",
    "sprite_sizes",
];

/// Why this QuarterMaster cannot load the mod, or None when it can. The reason follows the
/// mod's name and has no final period: "needs QuarterMaster 1.2.0 or newer (this is 1.0.0)".
pub fn gate_error(manifest: &ModManifest) -> Option<String> {
    gate_error_with(manifest, QM_VERSION, QM_FEATURES)
}

/// [`gate_error`] for another QuarterMaster version and feature list (tests, older DLLs).
pub fn gate_error_with(
    manifest: &ModManifest,
    qm_version: &str,
    features: &[&str],
) -> Option<String> {
    let mut problems: Vec<String> = Vec::new();
    if let Some(raw) = manifest.min_qm.as_deref() {
        match min_qm_req(raw) {
            Err(e) => problems.push(format!(
                "has a min_qm that is not a version ('{}': {e})",
                raw.trim()
            )),
            Ok(req) => {
                let ok = ParsedVersion::parse(qm_version).is_ok_and(|v| req.matches(&v));
                if !ok {
                    problems.push(format!(
                        "needs QuarterMaster {} (this is {qm_version})",
                        shown_req(raw)
                    ));
                }
            }
        }
    }
    let mut missing: Vec<&str> = Vec::new();
    for f in manifest.qm_features.iter().map(|f| f.trim()) {
        let known = f.is_empty() || features.iter().any(|have| have.eq_ignore_ascii_case(f));
        if !known && !missing.iter().any(|m| m.eq_ignore_ascii_case(f)) {
            missing.push(f);
        }
    }
    match missing.as_slice() {
        [] => {}
        [one] => problems.push(format!(
            "needs the QuarterMaster feature '{one}', which this version does not have"
        )),
        many => problems.push(format!(
            "needs the QuarterMaster features {}, which this version does not have",
            many.iter().map(|f| format!("'{f}'")).collect::<Vec<_>>().join(", ")
        )),
    }
    (!problems.is_empty()).then(|| problems.join("; and "))
}

/// `min_qm` as a requirement: a bare version means that version or newer, anything else is
/// a range such as `">=1.2 <2"`.
fn min_qm_req(raw: &str) -> Result<VersionReq, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("it is empty".to_string());
    }
    match ParsedVersion::parse(trimmed) {
        Ok(_) => VersionReq::parse(&format!(">={trimmed}")),
        Err(_) => VersionReq::parse(trimmed),
    }
}

fn shown_req(raw: &str) -> String {
    let trimmed = raw.trim();
    if ParsedVersion::parse(trimmed).is_ok() {
        format!("{trimmed} or newer")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(min_qm: Option<&str>, features: &[&str]) -> ModManifest {
        let mut m = ModManifest::new("m".into(), "M".into(), "1.0.0".into());
        m.min_qm = min_qm.map(str::to_string);
        m.qm_features = features.iter().map(|f| f.to_string()).collect();
        m
    }

    #[test]
    fn no_gates_loads() {
        assert_eq!(gate_error(&manifest(None, &[])), None);
    }

    #[test]
    fn every_listed_feature_is_met() {
        assert_eq!(gate_error(&manifest(None, QM_FEATURES)), None);
        assert_eq!(gate_error(&manifest(None, &[" Overrides ", ""])), None);
    }

    #[test]
    fn met_min_qm_loads() {
        let all = QM_FEATURES;
        assert_eq!(gate_error_with(&manifest(Some("1.0.0"), &[]), "1.0.0", all), None);
        assert_eq!(gate_error_with(&manifest(Some("0.9"), &[]), "1.0.0", all), None);
        assert_eq!(gate_error_with(&manifest(Some(">=1.0 <2"), &[]), "1.4.2", all), None);
        assert_eq!(gate_error(&manifest(Some(QM_VERSION), &[])), None);
    }

    #[test]
    fn min_qm_above_the_version_fails() {
        let e = gate_error_with(&manifest(Some("1.2.0"), &[]), "1.0.0", QM_FEATURES);
        assert_eq!(e.as_deref(), Some("needs QuarterMaster 1.2.0 or newer (this is 1.0.0)"));
        let e = gate_error_with(&manifest(Some(">=1.0 <2"), &[]), "2.0.0", QM_FEATURES);
        assert_eq!(e.as_deref(), Some("needs QuarterMaster >=1.0 <2 (this is 2.0.0)"));
    }

    #[test]
    fn unparsable_min_qm_fails_with_a_clear_error() {
        let e = gate_error(&manifest(Some("soon"), &[])).unwrap();
        assert!(e.starts_with("has a min_qm that is not a version ('soon': "), "{e}");
        let e = gate_error(&manifest(Some("  "), &[])).unwrap();
        assert_eq!(e, "has a min_qm that is not a version ('': it is empty)");
    }

    #[test]
    fn unknown_feature_fails() {
        let e = gate_error(&manifest(None, &["overrides", "teleport", "Teleport"]));
        assert_eq!(
            e.as_deref(),
            Some("needs the QuarterMaster feature 'teleport', which this version does not have")
        );
        let e = gate_error(&manifest(None, &["teleport", "fly"]));
        assert_eq!(
            e.as_deref(),
            Some(concat!(
                "needs the QuarterMaster features 'teleport', 'fly', ",
                "which this version does not have"
            ))
        );
    }

    #[test]
    fn an_older_feature_list_fails_a_mod_that_needs_a_newer_feature() {
        let old: Vec<&str> = QM_FEATURES.iter().copied().filter(|f| *f != "apply_as").collect();
        let m = manifest(None, &["overrides", "apply_as"]);
        assert_eq!(gate_error_with(&m, QM_VERSION, QM_FEATURES), None);
        let e = gate_error_with(&m, QM_VERSION, &old).unwrap();
        assert!(e.contains("'apply_as'"), "{e}");
    }

    #[test]
    fn both_gates_are_reported_together() {
        let m = manifest(Some("9.0.0"), &["teleport"]);
        let e = gate_error_with(&m, "1.0.0", QM_FEATURES);
        assert_eq!(
            e.as_deref(),
            Some(concat!(
                "needs QuarterMaster 9.0.0 or newer (this is 1.0.0); and needs the QuarterMaster ",
                "feature 'teleport', which this version does not have"
            ))
        );
    }
}
