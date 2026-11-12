//! Game-update stamps (D74, Phase D Task 38). A fix keeps the SHA-256 of each game file its
//! patches target, taken when it is made or accepted. When a file's hash differs now, the fix
//! needs review; nothing is suspended (D21 and D22 still decide that).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use qm_core::patch::plan::RawLoad;
use qm_core::patch::{Patch, Target};
use qm_core::prepare::canonical_path;

use super::{fixes_mut, hits, FixChange, ProfileFix};
use crate::profiles::ModProfile;

/// The review reason of a fix whose game files changed since it was stamped.
pub const GAME_CHANGED: &str = "The game changed since you made this fix";

/// The hash of a game file now, by its path; None when the game has no such file.
pub type GameHash<'a> = &'a dyn Fn(&str) -> Option<String>;

/// A game file whose hash differs from a fix's stamp.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameFileDrift {
    /// The game's path, as the stamp keys it.
    pub file: String,
    /// The SHA-256 when the fix was made or accepted.
    pub before: String,
    /// The SHA-256 now; None when the game no longer has the file.
    pub after: Option<String>,
}

/// The SHA-256 of `text`, as lowercase hex.
pub fn sha256_hex(text: &str) -> String {
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(text.as_bytes()) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The names a patch targets; a module names the file it replaces.
fn target_names(p: &Patch) -> Vec<&str> {
    fn of(t: &Target) -> Vec<&str> {
        match t {
            Target::Single(s) => vec![s.as_str()],
            Target::Multi(v) => v.iter().map(String::as_str).collect(),
        }
    }
    match p {
        Patch::Module(m) => m.before.as_deref().into_iter().collect(),
        Patch::Copy(c) => of(&c.target),
        Patch::Pattern(x) => of(&x.target),
        Patch::Regex(x) => of(&x.target),
    }
}

/// The patches `fix` names in `raw` (every patch a name hits), and a copy's own patch. None
/// for a `ModOrder` fix: it names mods, not patches.
fn fix_patches<'a>(fix: &'a ProfileFix, raw: Option<&'a RawLoad>) -> Vec<&'a Patch> {
    let mut out: Vec<&Patch> = Vec::new();
    let mut names: Vec<(&str, bool)> = fix.refs().iter().map(|r| (r.id.as_str(), false)).collect();
    match &fix.change {
        FixChange::Order { rule, .. } => {
            names.push((rule.patch.as_str(), true));
            names.extend(
                rule.before
                    .iter()
                    .chain(&rule.after)
                    .map(|n| (n.as_str(), true)),
            );
        }
        FixChange::AdjustedCopy { glue, .. } => out.push(&glue.patch),
        FixChange::ModOrder { .. } => return Vec::new(),
        _ => {}
    }
    if let Some(raw) = raw {
        for (name, rule_name) in names {
            for i in hits(raw, name, rule_name) {
                if let Some(rp) = raw.patches.get(i) {
                    out.push(&rp.patch);
                }
            }
        }
    }
    out
}

/// The game files (from `game`, the game's paths) that the patches of `fix` target. A target
/// names every file whose path ends with it, as the game matches chunks.
pub fn game_files(fix: &ProfileFix, raw: Option<&RawLoad>, game: &[String]) -> BTreeSet<String> {
    let targets: BTreeSet<String> = fix_patches(fix, raw)
        .into_iter()
        .flat_map(target_names)
        .map(canonical_path)
        .filter(|t| !t.is_empty())
        .collect();
    game.iter()
        .filter(|k| {
            let ck = canonical_path(k);
            targets.iter().any(|t| ck.ends_with(t.as_str()))
        })
        .cloned()
        .collect()
}

/// Stamps `fix` with the hashes of its game files now (D74), replacing any earlier stamp.
/// `hash(file)` gives a game file's hash.
pub fn stamp(fix: &mut ProfileFix, raw: Option<&RawLoad>, game: &[String], hash: GameHash<'_>) {
    let files = game_files(fix, raw, game);
    fix.vanilla_hashes = files
        .into_iter()
        .filter_map(|f| hash(&f).map(|h| (f, h)))
        .collect::<BTreeMap<_, _>>();
}

/// Stamps every fix of `p` made or accepted since it was read, or left waiting for a stamp,
/// and clears their marks. A fix that cannot be stamped now (no game files, or no mod load
/// for a fix that names patches) stays marked `stamp_pending` for a later save. Returns how
/// many were stamped.
pub fn stamp_fresh(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    game: Option<(&[String], GameHash<'_>)>,
) -> usize {
    let mut n = 0;
    for f in fixes_mut(p).filter(|f| f.fresh || f.stamp_pending) {
        f.fresh = false;
        let needs_raw = !matches!(f.change, FixChange::ModOrder { .. });
        match game {
            Some((files, hash)) if raw.is_some() || !needs_raw => {
                stamp(f, raw, files, hash);
                f.stamp_pending = false;
                n += 1;
            }
            _ => f.stamp_pending = true,
        }
    }
    n
}

/// Whether `p` has a fix waiting for its stamp.
pub fn has_pending(p: &ModProfile) -> bool {
    super::fixes(p).any(|f| f.stamp_pending)
}

/// The stamped game files of `fix` whose hash differs now. `hash(file)` is None when the game
/// has no such file. Empty for a fix without a stamp, or one waiting for a new stamp.
pub fn game_drift(fix: &ProfileFix, hash: GameHash<'_>) -> Vec<GameFileDrift> {
    if fix.stamp_pending {
        return Vec::new();
    }
    fix.vanilla_hashes
        .iter()
        .filter_map(|(file, before)| {
            let after = hash(file);
            (after.as_deref() != Some(before.as_str())).then(|| GameFileDrift {
                file: file.clone(),
                before: before.clone(),
                after,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "game_stamp_tests.rs"]
mod tests;
