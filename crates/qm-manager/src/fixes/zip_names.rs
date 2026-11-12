//! Zip stem names (D62, Phase D Task 9b). A zip's mod id was its file stem until the loader
//! read the mod.json inside it. The loader still matches the stem names (`legacy_ids`,
//! `legacy_mod`); on the next save the profile's stored references move to the mod.json id.

use std::collections::{BTreeSet, HashMap};

use qm_core::patch::order;
use qm_core::patch::plan::{self, RawLoad};
use qm_core::patch::Patch;

use super::{rename, FixChange, FixEventKind, ProfileFix};
use crate::profiles::ModProfile;

/// A zip mod whose mod.json gives another id than its file stem (D62).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipRename {
    /// The zip's file stem, lowercase: its id before its mod.json was read.
    pub stem: String,
    /// The id in its mod.json.
    pub id: String,
}

/// The loaded zips of `raw` whose mod.json id is not their stem. A stem that also names
/// another mod (loaded, failed to load, or a folder in the Mods folder) is left out: a
/// reference to it may mean that mod.
pub fn zip_stems(raw: &RawLoad) -> Vec<ZipRename> {
    let folders: Vec<String> = std::fs::read_dir(&raw.mod_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for (i, m) in raw.mods.iter().enumerate() {
        let zip = m
            .base
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let stem = m.folder.to_lowercase();
        if !zip || stem.is_empty() || stem == m.id {
            continue;
        }
        let names = |id: &str, folder: &str| {
            id.eq_ignore_ascii_case(&stem) || folder.eq_ignore_ascii_case(&stem)
        };
        let taken = raw
            .mods
            .iter()
            .enumerate()
            .any(|(j, o)| j != i && (names(&o.id, &o.folder) || o.provides.contains(&stem)))
            || raw.load_errors.iter().any(|e| names(&e.mod_id, &e.folder))
            || folders.contains(&stem);
        if !taken {
            out.push(ZipRename {
                stem,
                id: m.id.clone(),
            });
        }
    }
    out
}

/// Renames the profile's stored references to a zip's stem id to its mod.json id (D62):
/// enabled mods, mod priorities, load-after pairs, disabled patches, patch priorities, order
/// rules, adjusted copies, the ledger (refs, rule names, rungs, snapshots, mod ids) and the
/// fixes the journal keeps. A ledger fix that changes gets one `Rebound` event per renamed
/// mod. A patch name moves only to a name that matches the same loaded patches without the
/// stem names; one with no such name, or a patch priority another entry for the same patch
/// could beat, keeps its stem name, which the loader still matches. The legacy fields and the
/// ledger get the same names, so a migrated profile's compiled view stays in step. Returns
/// the zips whose references changed.
pub fn rename_zip_stems(p: &mut ModProfile, raw: &RawLoad) -> Vec<ZipRename> {
    let zips = zip_stems(raw);
    if zips.is_empty() {
        return Vec::new();
    }
    let mut prio_keys: Vec<String> = p.patch_priorities.keys().cloned().collect();
    prio_keys.extend(super::fixes(p).filter_map(|f| match &f.change {
        FixChange::Priority { patch, .. } => Some(patch.id.clone()),
        _ => None,
    }));
    let mut n = StemNames::new(raw, &zips, prio_keys);

    // Mod ids.
    p.enabled_mods = rename_list(&p.enabled_mods, true, |m| n.mod_id(m));
    rename_keys(&mut p.mod_priorities, |m| n.mod_id(m));
    let load_after = std::mem::take(&mut p.user_load_after);
    for (k, v) in load_after {
        let k = n.mod_id(&k).unwrap_or(k);
        let v = rename_list(&v, true, |m| n.mod_id(m));
        let e = p.user_load_after.entry(k).or_default();
        for m in v {
            if !e.iter().any(|x| x.eq_ignore_ascii_case(&m)) {
                e.push(m);
            }
        }
    }

    // Patch names in the legacy fields.
    p.disabled_patches = rename_list(&p.disabled_patches, false, |s| n.patch_name(s));
    rename_keys(&mut p.patch_priorities, |s| n.patch_name(s));
    for rule in &mut p.patch_order {
        rename_rule(rule, &mut n);
    }
    for g in &mut p.glue_patches {
        rename_opt(&mut g.replaces_patch_id, |s| n.patch_name(s));
        rename_opt(&mut g.base_mod_id, |m| n.mod_id(m));
    }

    // The ledger.
    for f in super::fixes_mut(p) {
        n.local.clear();
        rename_fix(f, &mut n);
        for z in std::mem::take(&mut n.local) {
            if let Some(zr) = zips.get(z) {
                f.log(
                    FixEventKind::Rebound,
                    format!("{} is now {}", zr.stem, zr.id),
                );
            }
        }
    }
    for e in p.fix_journal.iter_mut().filter_map(|e| match e {
        qm_core::lenient::Lenient::Ok(e) => Some(e),
        qm_core::lenient::Lenient::Invalid { .. } => None,
    }) {
        for f in e.removed.iter_mut().filter_map(|f| match f {
            qm_core::lenient::Lenient::Ok(f) => Some(f),
            qm_core::lenient::Lenient::Invalid { .. } => None,
        }) {
            rename_fix(f, &mut n);
        }
        e.mods_enabled = rename_list(&e.mods_enabled, true, |m| n.mod_id(m));
    }

    let hit = n.hit;
    zips.into_iter()
        .enumerate()
        .filter(|(i, _)| hit.contains(i))
        .map(|(_, z)| z)
        .collect()
}

/// The patches a name matches: as a plan key (`plan_keys`), and as a rule name.
type Hits = (Vec<usize>, Vec<usize>);

/// The new names of a profile's references to zip stem ids.
struct StemNames<'a> {
    raw: &'a RawLoad,
    zips: &'a [ZipRename],
    /// The loaded patches without their stem names (`None`: the patch has none).
    current: Vec<Option<Patch>>,
    /// The profile's patch priority keys and Priority fix names.
    prio_keys: Vec<String>,
    names: HashMap<String, Option<(String, usize)>>,
    /// The zips (indexes into `zips`) a rename was made for.
    hit: BTreeSet<usize>,
    /// The same, since the last clear (one fix).
    local: BTreeSet<usize>,
}

impl<'a> StemNames<'a> {
    fn new(raw: &'a RawLoad, zips: &'a [ZipRename], prio_keys: Vec<String>) -> Self {
        let current = raw
            .patches
            .iter()
            .map(|rp| {
                rp.patch.extra().legacy_mod.as_ref()?;
                let mut p = rp.patch.clone();
                let extra = p.extra_mut();
                extra.legacy_ids.clear();
                extra.legacy_mod = None;
                Some(p)
            })
            .collect();
        StemNames {
            raw,
            zips,
            current,
            prio_keys,
            names: HashMap::new(),
            hit: BTreeSet::new(),
            local: BTreeSet::new(),
        }
    }

    fn mark(&mut self, z: usize) {
        self.hit.insert(z);
        self.local.insert(z);
    }

    /// The mod.json id for a reference to a zip's stem id.
    fn mod_id(&mut self, m: &str) -> Option<String> {
        let (z, to) = self
            .zips
            .iter()
            .enumerate()
            .find(|(_, z)| z.stem.eq_ignore_ascii_case(m))
            .map(|(i, z)| (i, z.id.clone()))?;
        self.mark(z);
        Some(to)
    }

    /// The new name for a patch name that matches only through a zip's stem names.
    fn patch_name(&mut self, name: &str) -> Option<String> {
        let found = match self.names.get(name) {
            Some(found) => found.clone(),
            None => {
                let found = self.find_name(name);
                self.names.insert(name.to_string(), found.clone());
                found
            }
        };
        let (to, z) = found?;
        self.mark(z);
        Some(to)
    }

    fn hits(&self, name: &str, with_stems: bool) -> Hits {
        let mut keys = Vec::new();
        let mut rules = Vec::new();
        for (i, rp) in self.raw.patches.iter().enumerate() {
            let p = match self.current.get(i) {
                Some(Some(p)) if !with_stems => p,
                _ => &rp.patch,
            };
            if plan::plan_keys(p).iter().any(|k| k == name) {
                keys.push(i);
            }
            if order::name_matches(name, p) {
                rules.push(i);
            }
        }
        (keys, rules)
    }

    fn find_name(&self, name: &str) -> Option<(String, usize)> {
        let lower = name.to_ascii_lowercase();
        let zips: Vec<(usize, &ZipRename)> = self
            .zips
            .iter()
            .enumerate()
            .filter(|(_, z)| lower.contains(&z.stem))
            .collect();
        if zips.is_empty() {
            return None;
        }
        let before = self.hits(name, true);
        if (before.0.is_empty() && before.1.is_empty()) || self.hits(name, false) == before {
            return None;
        }
        if self.prio_keys.iter().any(|k| k == name) && self.prio_clash(name, &before) {
            return None;
        }
        for (z, zip) in zips {
            for c in self.candidates(name, zip, &before) {
                if c != name && self.hits(&c, false) == before {
                    return Some((c, z));
                }
            }
        }
        None
    }

    /// Whether another patch priority key names a patch `name` names: which entry wins would
    /// depend on the key order, so the stem name stays.
    fn prio_clash(&self, name: &str, before: &Hits) -> bool {
        self.prio_keys.iter().filter(|k| *k != name).any(|k| {
            let (a, b) = self.hits(k, true);
            a.iter()
                .chain(&b)
                .any(|i| before.0.contains(i) || before.1.contains(i))
        })
    }

    /// The names `name` may become: the stem prefix swapped for the id (twice for the
    /// `mod:name` of a patch named by its fallback id), the asset patch's id, and the names
    /// of the one patch it matches.
    fn candidates(&self, name: &str, z: &ZipRename, before: &Hits) -> Vec<String> {
        let n = z.stem.len() + 1;
        let swap = |s: &str| -> Option<String> {
            let head = s.get(..n)?;
            let rest = s.get(n..)?;
            (head.strip_suffix(':')?.eq_ignore_ascii_case(&z.stem))
                .then(|| format!("{}:{rest}", z.id))
        };
        let mut out = Vec::new();
        if let Some(c) = swap(name) {
            out.push(c);
            if let Some(rest) = name.get(n..).and_then(swap) {
                out.push(format!("{}:{rest}", z.id));
            }
        }
        if name.eq_ignore_ascii_case(&format!("{}_auto_assets", z.stem)) {
            out.push(format!("{}_auto_assets", z.id));
        }
        let one = match (before.0.as_slice(), before.1.as_slice()) {
            ([t], _) | (_, [t]) => Some(*t),
            _ => None,
        };
        if let Some(p) = one
            .and_then(|t| self.current.get(t))
            .and_then(Option::as_ref)
        {
            out.extend(plan::plan_keys(p).into_iter().take(2));
        }
        out
    }
}

/// `list` with each renamed entry swapped; one whose new name is in the list already goes.
fn rename_list(
    list: &[String],
    mods: bool,
    mut f: impl FnMut(&str) -> Option<String>,
) -> Vec<String> {
    let same = |a: &str, b: &str| {
        if mods {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for s in list {
        match f(s) {
            Some(to) => {
                let there = list.iter().chain(out.iter()).any(|x| same(x, &to));
                if !there {
                    out.push(to);
                }
            }
            None => out.push(s.clone()),
        }
    }
    out
}

/// `map` with each renamed key swapped; an entry under the new key already there wins (the
/// loader looks the new key up first).
fn rename_keys<V>(map: &mut HashMap<String, V>, mut f: impl FnMut(&str) -> Option<String>) {
    let mut moved = Vec::new();
    for (k, v) in std::mem::take(map) {
        match f(&k) {
            Some(to) => moved.push((to, v)),
            None => {
                map.insert(k, v);
            }
        }
    }
    for (to, v) in moved {
        map.entry(to).or_insert(v);
    }
}

fn rename_opt(s: &mut Option<String>, f: impl FnOnce(&str) -> Option<String>) {
    if let Some(to) = s.as_deref().and_then(f) {
        *s = Some(to);
    }
}

fn rename_rule(rule: &mut order::OrderRule, n: &mut StemNames) {
    for s in std::iter::once(&mut rule.patch)
        .chain(rule.before.iter_mut())
        .chain(rule.after.iter_mut())
    {
        if let Some(to) = n.patch_name(s) {
            *s = to;
        }
    }
}

/// Renames the stem names and stem mod ids of one fix.
fn rename_fix(f: &mut ProfileFix, n: &mut StemNames) {
    let mut names: Vec<String> = f.refs().iter().map(|r| r.id.clone()).collect();
    match &f.change {
        FixChange::Order { rule, .. } => {
            names.push(rule.patch.clone());
            names.extend(rule.before.iter().chain(&rule.after).cloned());
        }
        FixChange::SkipHunk { hunk, .. } => names.extend(hunk.keep.clone()),
        FixChange::AdjustedCopy { glue, .. } => names.extend(glue.replaces_patch_id.clone()),
        _ => {}
    }
    names.extend(f.snapshots.keys().cloned());
    names.sort();
    names.dedup();
    for name in names {
        let Some(to) = n.patch_name(&name) else {
            continue;
        };
        rename(&mut f.change, &name, &to);
        if let Some(s) = f.snapshots.remove(&name) {
            f.snapshots.entry(to).or_insert(s);
        }
    }
    match &mut f.change {
        // A rung names the patch it anchors in.
        FixChange::Anchor { anchors, .. } => {
            for r in anchors {
                rename_opt(&mut r.within, |s| n.patch_name(s));
            }
        }
        FixChange::ModOrder {
            move_mod_id,
            before_mod_id,
            ..
        } => {
            for m in [move_mod_id, before_mod_id] {
                if let Some(to) = n.mod_id(m) {
                    *m = to;
                }
            }
        }
        FixChange::AdjustedCopy { glue, .. } => {
            rename_opt(&mut glue.base_mod_id, |m| n.mod_id(m));
        }
        _ => {}
    }
    for r in f.refs_mut() {
        if let Some(to) = n.mod_id(&r.mod_id) {
            r.mod_id = to;
        }
    }
    for (m, _) in &mut f.provenance.mods {
        if let Some(to) = n.mod_id(m) {
            *m = to;
        }
    }
}

#[cfg(test)]
#[path = "zip_names_tests.rs"]
mod tests;
