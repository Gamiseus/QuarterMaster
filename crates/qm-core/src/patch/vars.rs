#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use std::{collections::HashMap, sync::OnceLock};
use regex_lite::{Captures, Regex};

/// The interpolation regex, or `None` when it fails to build (interpolation is then off).
fn var_regex() -> Option<&'static Regex> {
    static RE: OnceLock<Option<Regex>> = OnceLock::new();
    RE.get_or_init(|| match Regex::new(r"\{\{(?:qm|quartermaster):(\w+)\}\}") {
        Ok(re) => Some(re),
        Err(e) => {
            log::warn!("QuarterMaster: Variable interpolation is off: {e}");
            None
        }
    })
    .as_ref()
}

/// Apply valid var interpolations to the provided string buffer.
/// Supports {{qm:VAR_NAME}} and {{quartermaster:VAR_NAME}}.
pub fn apply_var_interp(buffer: &mut String, vars: &HashMap<String, String>) {
    if vars.is_empty() || !buffer.contains("{{") {
        return;
    }
    let Some(re) = var_regex() else {
        return;
    };

    let replaced = re.replace_all(buffer, |captures: &Captures| {
        let whole = captures.get(0).map_or("", |m| m.as_str());
        let var = captures.get(1).map_or("", |m| m.as_str());
        match vars.get(var) {
            Some(val) => val.clone(),
            None => {
                log::warn!("QuarterMaster: Unregistered variable interpolation target '{{{var}}}' encountered");
                whole.to_string()
            }
        }
    });

    *buffer = replaced.into_owned();
}
