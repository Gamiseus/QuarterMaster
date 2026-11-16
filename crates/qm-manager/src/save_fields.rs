//! Which mod owns which save field (Phase E D84).
//!
//! The game keeps its save in the `player` table and writes all of it. A mod that sets
//! `player.<field> = ...` therefore leaves that field in every save, also after the mod is
//! removed. This registry lists those fields with the mods that write them, from the same
//! index the semantic checks use. A field the game's own code sets is never listed, and
//! neither are QuarterMaster's own keys.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Save field -> the mods that write it, sorted. Built once per simulation.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveFieldRegistry {
    pub fields: BTreeMap<String, Vec<String>>,
}

/// Keys QuarterMaster itself keeps in a save (the faction marker and the stamp); the game's
/// loader cleans those up on its own.
pub fn is_own_key(field: &str) -> bool {
    field.starts_with("qm") || field.starts_with("_qm")
}

impl SaveFieldRegistry {
    pub(crate) fn from_owners(owners: BTreeMap<String, BTreeSet<String>>) -> Self {
        Self {
            fields: owners
                .into_iter()
                .filter(|(field, mods)| !is_own_key(field) && !mods.is_empty())
                .map(|(field, mods)| (field, mods.into_iter().collect()))
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The mods that write `field`; empty when no mod does.
    pub fn owners(&self, field: &str) -> &[String] {
        self.fields.get(field).map_or(&[], Vec::as_slice)
    }

    /// The mod that alone writes `field`.
    pub fn single_owner(&self, field: &str) -> Option<&str> {
        match self.owners(field) {
            [only] => Some(only.as_str()),
            _ => None,
        }
    }

    /// Mod -> the fields it writes, sorted: the `fields` of a save stamp (D85).
    pub fn by_mod(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (field, mods) in &self.fields {
            for m in mods {
                out.entry(m.clone()).or_default().push(field.clone());
            }
        }
        out
    }
}

/// What a save holds from mods that are not in a profile, from the save's stamp.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeftoverFields {
    /// Absent mods with at least one field in the save, sorted.
    pub mods: Vec<String>,
    /// Every field of an absent mod that is in the save, sorted.
    pub fields: Vec<String>,
    /// The fields Clean up may remove: in the save, and every mod the stamp lists for the
    /// field is absent and it lists exactly one (D86). Sorted.
    pub cleanable: Vec<String>,
    /// Leftover field -> the mods the stamp lists for it.
    #[serde(default)]
    pub owners: BTreeMap<String, Vec<String>>,
}

impl LeftoverFields {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// The leftovers of one save. `stamp_fields` is the save stamp's mod -> fields, `in_save`
/// says whether the save's root table has a key, and `enabled` whether a mod is in the
/// profile (by id, ignoring case).
pub fn leftovers(
    stamp_fields: &BTreeMap<String, Vec<String>>,
    in_save: impl Fn(&str) -> bool,
    enabled: impl Fn(&str) -> bool,
) -> LeftoverFields {
    // Field -> every mod the stamp lists for it.
    let mut writers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (m, fields) in stamp_fields {
        for f in fields {
            writers.entry(f.as_str()).or_default().push(m.as_str());
        }
    }
    let mut out = LeftoverFields::default();
    let mut mods: BTreeSet<&str> = BTreeSet::new();
    for (field, owners) in &writers {
        if is_own_key(field) || !in_save(field) || owners.iter().any(|m| enabled(m)) {
            continue;
        }
        out.fields.push((*field).to_string());
        out.owners.insert(
            (*field).to_string(),
            owners.iter().map(|m| (*m).to_string()).collect(),
        );
        mods.extend(owners.iter().copied());
        if owners.len() == 1 {
            out.cleanable.push((*field).to_string());
        }
    }
    out.mods = mods.into_iter().map(str::to_string).collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owners(list: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
        list.iter()
            .map(|(f, mods)| ((*f).to_string(), mods.iter().map(|m| (*m).to_string()).collect()))
            .collect()
    }

    #[test]
    fn the_registry_lists_owners_and_leaves_out_quartermasters_keys() {
        let r = SaveFieldRegistry::from_owners(owners(&[
            ("option_a", &["gcore"]),
            ("option_shared", &["tactical_overhaul", "gcore"]),
            ("qmFaction", &["somemod"]),
            ("_qm_meta", &["somemod"]),
            ("empty", &[]),
        ]));
        assert_eq!(r.fields.len(), 2);
        assert_eq!(r.single_owner("option_a"), Some("gcore"));
        assert_eq!(r.single_owner("option_shared"), None);
        assert_eq!(r.owners("option_shared"), ["gcore", "tactical_overhaul"]);
        assert!(r.owners("nope").is_empty());
        let by_mod = r.by_mod();
        assert_eq!(by_mod["gcore"], ["option_a", "option_shared"]);
        assert_eq!(by_mod["tactical_overhaul"], ["option_shared"]);
    }

    #[test]
    fn leftovers_are_fields_whose_writers_are_all_absent() {
        let stamp: BTreeMap<String, Vec<String>> = BTreeMap::from([
            ("gcore".to_string(), vec!["option_a".to_string(), "option_shared".to_string()]),
            ("to".to_string(), vec!["option_shared".to_string(), "option_t".to_string()]),
            ("gone".to_string(), vec!["option_gone".to_string(), "not_in_save".to_string()]),
        ]);
        let in_save = |f: &str| f != "not_in_save";

        // Only `gone` is absent.
        let l = leftovers(&stamp, in_save, |m| m != "gone");
        assert_eq!(l.mods, ["gone"]);
        assert_eq!(l.fields, ["option_gone"]);
        assert_eq!(l.cleanable, ["option_gone"]);

        // gcore absent too: its own field is cleanable; the shared one stays (TO is there).
        let l = leftovers(&stamp, in_save, |m| m == "to");
        assert_eq!(l.fields, ["option_a", "option_gone"]);
        assert_eq!(l.cleanable, ["option_a", "option_gone"]);

        // Everything absent: the shared field is a leftover, but two mods wrote it, so it
        // is never offered for clean up.
        let l = leftovers(&stamp, in_save, |_| false);
        assert_eq!(l.mods, ["gcore", "gone", "to"]);
        assert_eq!(l.fields, ["option_a", "option_gone", "option_shared", "option_t"]);
        assert_eq!(l.cleanable, ["option_a", "option_gone", "option_t"]);
        assert_eq!(l.owners["option_shared"], ["gcore", "to"]);

        assert!(leftovers(&stamp, in_save, |_| true).is_empty());
    }
}
