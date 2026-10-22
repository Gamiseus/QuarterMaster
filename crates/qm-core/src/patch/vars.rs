use std::{collections::HashMap, sync::LazyLock};
use regex_lite::{Captures, Regex};

/// Apply valid var interpolations to the provided string buffer.
/// Supports {{qm:VAR_NAME}} and {{quartermaster:VAR_NAME}}.
pub fn apply_var_interp(buffer: &mut String, vars: &HashMap<String, String>) {
    if vars.is_empty() || !buffer.contains("{{") {
        return;
    }

    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\{\{(?:qm|quartermaster):(\w+)\}\}").unwrap()
    });

    let replaced = RE.replace_all(buffer, |captures: &Captures| {
        let var = &captures[1];
        match vars.get(var) {
            Some(val) => val.clone(),
            None => {
                log::warn!("QuarterMaster: Unregistered variable interpolation target '{{{var}}}' encountered");
                captures[0].to_string()
            }
        }
    });

    *buffer = replaced.into_owned();
}
