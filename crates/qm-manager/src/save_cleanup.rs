//! Values a removed mod left in a save, and their removal on request (Phase E D86).
//!
//! A save's stamp lists the save fields each mod wrote (D85). A field is a leftover when the
//! save still has it and no profile that uses the slot enables any mod the stamp lists for
//! it. Clean up removes only the leftovers one mod alone wrote, and only when asked: a
//! backup first, the entries cut out of the text byte for byte, the result parsed again and
//! loaded with the game's own Lua, and only then written.

use std::collections::BTreeMap;
use std::fs;

use qm_core::compile::LuaCompiler;
use qm_core::handshake::write_file_atomic;
use qm_schema::{SaveSummary, SerDocument};
use serde::{Deserialize, Serialize};

use crate::save_fields::{leftovers, LeftoverFields};
use crate::storage::QuarterMasterPaths;

/// The label of the backup Clean up makes first.
pub const CLEANUP_BACKUP_LABEL: &str = "Before clean up";

/// What Clean up did.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveCleanup {
    /// The fields taken out of the save, in file order.
    pub removed: Vec<String>,
    /// The backup made before the save was changed.
    pub backup_id: String,
}

/// From a save's text: the stamp's mod -> fields, keeping only the fields the save's root
/// table still has as entries of its own. Empty for a save without a stamp.
pub fn stamped_fields(raw: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    let summary = SaveSummary::from_ser_str("saveMNB3.lua", raw)?;
    let Some(meta) = summary.qm_meta else {
        return Ok(BTreeMap::new());
    };
    let keys: Vec<String> = SerDocument::root_entry_spans(raw)?
        .into_iter()
        .filter_map(|e| e.key)
        .collect();
    Ok(meta
        .fields
        .into_iter()
        .map(|(m, fields)| {
            let kept: Vec<String> = fields.into_iter().filter(|f| keys.contains(f)).collect();
            (m, kept)
        })
        .filter(|(_, fields)| !fields.is_empty())
        .collect())
}

/// The leftovers among `stamped` (from `stamped_fields`) for a slot whose profiles enable
/// `enabled`.
pub fn leftovers_of(stamped: &BTreeMap<String, Vec<String>>, enabled: &[String]) -> LeftoverFields {
    leftovers(
        stamped,
        |_| true,
        |m| enabled.iter().any(|e| e.eq_ignore_ascii_case(m)),
    )
}

impl QuarterMasterPaths {
    /// The mods enabled in any profile that uses save slot `slot_id`. A value of a mod one
    /// of them still loads is not a leftover.
    pub fn slot_enabled_mods(&self, slot_id: &str) -> Vec<String> {
        let mut mods: Vec<String> = self
            .list_profiles()
            .into_iter()
            .filter(|p| p.linked_save_profile_id == slot_id)
            .flat_map(|p| p.enabled_mods)
            .collect();
        mods.sort();
        mods.dedup();
        mods
    }

    /// The stamp fields of slot `slot_id`'s save (`stamped_fields`); empty without a save.
    pub fn slot_stamped_fields(&self, slot_id: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
        match self.slot_save_file(slot_id) {
            Some(file) => {
                let raw = fs::read_to_string(&file)
                    .map_err(|e| format!("Failed to read {}: {e}", file.display()))?;
                stamped_fields(&raw)
            }
            None => Ok(BTreeMap::new()),
        }
    }

    /// Removes `fields` from slot `slot_id`'s save (D86). Every field must be one Clean up
    /// may remove. The save is written only after a complete backup, a parse of the new
    /// text and a load with the game's Lua (`compiler`); any failure leaves it as it was.
    pub fn clean_save(
        &self,
        slot_id: &str,
        fields: &[String],
        compiler: Option<&LuaCompiler>,
    ) -> Result<SaveCleanup, String> {
        if fields.is_empty() {
            return Err("No value was chosen to clean up.".to_string());
        }
        let file = self
            .slot_save_file(slot_id)
            .ok_or_else(|| "This save slot has no save file.".to_string())?;
        let raw = fs::read_to_string(&file)
            .map_err(|e| format!("Failed to read {}: {e}", file.display()))?;
        let left = leftovers_of(&stamped_fields(&raw)?, &self.slot_enabled_mods(slot_id));
        if let Some(f) = fields.iter().find(|f| !left.cleanable.contains(f)) {
            return Err(format!(
                "'{f}' cannot be cleaned up: it is not a value one removed mod alone left in this save."
            ));
        }
        let compiler = compiler.ok_or_else(|| {
            "The game's Lua could not be loaded, so the cleaned save could not be checked."
                .to_string()
        })?;

        let backup = self.create_save_backup_strict(slot_id, CLEANUP_BACKUP_LABEL)?;

        let keys: Vec<&str> = fields.iter().map(String::as_str).collect();
        let (text, removed) = SerDocument::remove_root_keys(&raw, &keys)?;
        if removed.len() != fields.len() {
            return Err("Not every chosen value was found in the save; nothing was changed.".to_string());
        }
        // The new text must be the old one without exactly those entries.
        SerDocument::parse(&text).map_err(|e| format!("The cleaned save does not parse: {e}"))?;
        let keys_of = |t: &str| -> Result<Vec<Option<String>>, String> {
            Ok(SerDocument::root_entry_spans(t)?.into_iter().map(|e| e.key).collect())
        };
        let expected: Vec<Option<String>> = keys_of(&raw)?
            .into_iter()
            .filter(|k| !k.as_deref().is_some_and(|k| keys.contains(&k)))
            .collect();
        if keys_of(&text)? != expected {
            return Err("The cleaned save lost or kept the wrong values; nothing was changed.".to_string());
        }
        compiler
            .compile("saveMNB3.lua", &text)
            .map_err(|e| format!("The game's Lua does not load the cleaned save: {}", e.message))?;

        write_file_atomic(&file, text.as_bytes())?;
        Ok(SaveCleanup {
            removed,
            backup_id: backup.id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVE: &str = "local _={}\n_[2]={\"gone_a\",\"gone_b\",\"vanished\"}\n_[3]={\"shared\"}\n_[4]={\"shared\",\"kept_a\"}\n_[1]={gone=_[2],other=_[3],kept=_[4]}\n_[5]={x=1}\nreturn {rank=4,gone_a=1,gone_b=_[5],shared=2,kept_a=3,_qm_meta={profile_id=\"p\",fields=_[1]},dayz=9}";

    fn game_lua() -> Option<LuaCompiler> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        [root.join("../../../game"), root.join("../../game")]
            .iter()
            .find_map(|d| LuaCompiler::find(d))
            .and_then(|p| LuaCompiler::load(&p).ok())
    }

    /// A slot `s` with a save, used by profile `p`, which enables `kept`.
    fn slot(temp: &std::path::Path) -> (QuarterMasterPaths, std::path::PathBuf) {
        let paths = QuarterMasterPaths::resolve(Some(temp));
        paths.list_save_profiles();
        paths.create_save_profile("S").unwrap();
        let slot_id = paths
            .list_save_profiles()
            .into_iter()
            .find(|s| s.name == "S")
            .unwrap()
            .id;
        let mut p = paths.create_profile("P", None, Some(&slot_id), None).unwrap();
        p.enabled_mods = vec!["Kept".into()];
        paths.save_profile(&p).unwrap();
        let file = paths.saves_dir.join(&slot_id).join("active").join("saveMNB3.lua");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, SAVE).unwrap();
        (paths, file)
    }

    fn slot_id(paths: &QuarterMasterPaths) -> String {
        paths.list_save_profiles().into_iter().find(|s| s.name == "S").unwrap().id
    }

    #[test]
    fn the_stamp_lists_only_fields_the_save_still_has() {
        let s = stamped_fields(SAVE).unwrap();
        assert_eq!(s["gone"], ["gone_a", "gone_b"]);
        assert_eq!(s["other"], ["shared"]);
        assert_eq!(s["kept"], ["kept_a", "shared"]);
        assert!(stamped_fields("return {dayz=3}").unwrap().is_empty());

        let left = leftovers_of(&s, &["KEPT".to_string()]);
        assert_eq!(left.mods, ["gone"]);
        assert_eq!(left.cleanable, ["gone_a", "gone_b"]);
        // With no profile on the slot the shared field is a leftover, but never cleanable.
        let left = leftovers_of(&s, &[]);
        assert_eq!(left.fields, ["gone_a", "gone_b", "kept_a", "shared"]);
        assert_eq!(left.cleanable, ["gone_a", "gone_b", "kept_a"]);
    }

    #[test]
    fn clean_up_cuts_exactly_the_chosen_fields_after_a_backup() {
        let Some(lua) = game_lua() else {
            eprintln!("lua51.dll not found; skipped");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let (paths, file) = slot(temp.path());
        let id = slot_id(&paths);
        assert_eq!(paths.slot_enabled_mods(&id), ["Kept"]);

        let done = paths.clean_save(&id, &["gone_b".to_string()], Some(&lua)).unwrap();
        assert_eq!(done.removed, ["gone_b"]);
        let now = fs::read_to_string(&file).unwrap();
        assert_eq!(now, SAVE.replace("gone_b=_[5],", ""), "every other byte stays");
        let backup = paths.saves_dir.join(&id).join("backups").join(&done.backup_id);
        assert_eq!(fs::read_to_string(backup.join("saveMNB3.lua")).unwrap(), SAVE);
        let left = leftovers_of(&paths.slot_stamped_fields(&id).unwrap(), &paths.slot_enabled_mods(&id));
        assert_eq!(left.cleanable, ["gone_a"]);
    }

    #[test]
    fn clean_up_refuses_what_it_may_not_remove_and_leaves_the_save() {
        let temp = tempfile::tempdir().unwrap();
        let (paths, file) = slot(temp.path());
        let id = slot_id(&paths);
        let backups = || {
            fs::read_dir(paths.saves_dir.join(&id).join("backups"))
                .map(|d| d.count())
                .unwrap_or(0)
        };
        let lua = game_lua();
        // A field of an enabled mod, a field two mods wrote, a game field, and nothing.
        for f in ["kept_a", "shared", "rank", "vanished"] {
            let err = paths.clean_save(&id, &[f.to_string()], lua.as_ref()).unwrap_err();
            assert!(err.contains("cannot be cleaned up"), "{err}");
        }
        assert!(paths.clean_save(&id, &[], lua.as_ref()).is_err());
        assert_eq!(backups(), 0, "a refused request makes no backup");
        // Without the game's Lua the result cannot be checked, so nothing is written.
        let err = paths.clean_save(&id, &["gone_a".to_string()], None).unwrap_err();
        assert!(err.contains("could not be checked"), "{err}");
        assert_eq!(fs::read_to_string(&file).unwrap(), SAVE);
    }
}
