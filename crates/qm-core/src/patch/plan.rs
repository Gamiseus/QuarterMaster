#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
//! The plan layer (Phase C §3.2, §3.3). `load_raw` reads the Mods folder (disk only);
//! `apply_plan` lays the profile's plan over it (pure). The DLL, the launcher's simulation and
//! its previews all build their tables this way, so they cannot drift apart.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use log::*;
use serde::Serialize;

use crate::handshake::{
    AnchorFix, BindingStatus, FixBinding, GluePatchEntry, InvalidRung, LoadError, LoadOrderPlan,
    OverrideReport, PlanLoad, StackMode, StackRuleFix, SuspendedRung,
};
use crate::lenient::Lenient;
use crate::patch::anchor::{AnchorSource, Rung, SiteKey};
use crate::patch::compat::{self, CompatLayer, Override, ProfileTouch};
use crate::patch::edits::{self, ApplyAs, HunkKey};
use crate::patch::faction::LoadedFaction;
use crate::patch::fingerprint::{PatchFingerprint, PatchRef};
use crate::patch::loader::{self, LoadedPatch};
use crate::patch::order::{self, OrderRule};
use crate::patch::stack::{RuleSource, StackRuleSpec};
use crate::patch::{InsertPosition, Patch, Priority};

/// What reading the plan found. Copied into patch-report.json and the simulation report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanReport {
    /// A hard error: no mods were loaded (D16).
    pub plan_error: Option<String>,
    /// Skipped entries and other notes about the plan (D17).
    pub plan_warnings: Vec<String>,
    /// Notes about the mods: renamed duplicate ids (D29), unreadable mod.json files and mods
    /// that depend on each other (their blocks keep the legacy order).
    pub warnings: Vec<String>,
    /// How plan fixes met the loaded patches: one per entry with a ref (§7.1), and an
    /// adjusted copy whose mod did not load is `Waiting` (D19, D60).
    pub bindings: Vec<FixBinding>,
    /// Order rules a stronger rule replaced or ignored. This and the three below are filled
    /// by `PatchTable::prepare` (§3.9).
    pub overridden_rules: Vec<String>,
    /// Names in order rules that match no loaded patch.
    pub unknown_ids: Vec<String>,
    /// Rungs suspended to break an order cycle.
    pub suspended_rungs: Vec<SuspendedRung>,
    /// Rungs that are never used, with why.
    pub invalid_rungs: Vec<InvalidRung>,
    /// What became of each compat override (§8.1), filled by `apply_plan`.
    pub overrides: Vec<OverrideReport>,
}

/// The mod id a `Legacy` adjusted copy runs as (D18).
pub const GLUE_MOD: &str = "qm_glue";

/// One loaded mod's mod.json data, in loader order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModMeta {
    /// Lowercase.
    pub id: String,
    pub folder: String,
    /// The display name from mod.json.
    pub name: Option<String>,
    pub version: Option<String>,
    /// `relationships.requires` keys, lowercase.
    pub requires: BTreeSet<String>,
    /// Already includes `requires`.
    pub load_after: BTreeSet<String>,
    pub load_before: BTreeSet<String>,
    pub provides: BTreeSet<String>,
    /// False when mod.json was missing or unreadable and the id fell back to the folder name.
    pub manifest_ok: bool,
    pub loader_index: u32,
    /// The mod's folder or zip file: what `{{qm_hack:patch_dir}}` became in its patches.
    pub base: PathBuf,
    /// mod.json `overrides`: launcher check findings the author silences (D69). The game
    /// never reads it.
    pub overrides: Vec<String>,
}

/// Per-patch data from the profile and compat layers. Never read from or written to a patch
/// file (`#[serde(skip)]` on the patch), so every table cloned from patches keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct PatchExtra {
    /// Profile and compat rungs; author rungs stay on the pattern or regex patch.
    pub rungs: Vec<(Rung, AnchorSource)>,
    /// From the winning profile or compat layer.
    pub sites: Vec<SiteKey>,
    pub replace_author_anchors: bool,
    pub fix_ids: Vec<String>,
    /// Compat mod ids.
    pub changed_by: Vec<String>,
    /// The fallback id this patch had before a duplicate got the `#<toml>` suffix (D29).
    /// Legacy plan entries and rule names still match it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    /// For an adjusted copy: the id of the patch it replaces, so a rung on that patch finds
    /// the copy (§4.5, glue alias).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
    /// For a zip whose mod.json gives another id than its file stem (D62): the names the
    /// patch had under the stem id (its id, its D29 legacy id and `mod:name`, where they
    /// differ). Plan entries and rule names still match them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub legacy_ids: Vec<String>,
    /// That zip's stem id (D62), so `<stem>:*` and `<stem>:<id>` rule names still match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_mod: Option<String>,
    /// How an `at` patch runs, from the profile, else a compat mod (D13, D46). `None` leaves
    /// the author's `apply_as`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply_as: Option<ApplyAs>,
    /// The edits of this `at` patch that do not run, from the profile, else compat mods (D49).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skip_hunks: Vec<HunkSkip>,
    /// Stack rules this patch carries, from the profile and compat mods (D55). The other
    /// patch named in each gets the rule too, in `PatchTable::prepare`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stack_rules: Vec<StackRuleSpec>,
    /// An adjusted copy from the profile (D58). It belongs to the profile whichever mod it
    /// runs as: no compat mod may name it, and a crash in it points at its fix.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub adjusted_copy: bool,
    /// For an exact copy (D59): its pattern lines, trimmed. They match only their own text,
    /// so `*` and `?` in a Lua line are not wildcards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_lines: Option<Vec<String>>,
}

/// One edit of an `at` patch that does not run (D49), bound to the patch as it is now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct HunkSkip {
    /// Among the patch's edits, from 0 (`edits::edit_hunks`).
    pub ordinal: u32,
    /// The patch's lines, trimmed (`HunkKey.vanilla`), for the record's warning.
    pub vanilla: String,
    /// The patch whose change to these lines stays (`HunkKey.keep`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep: Option<String>,
}

impl HunkSkip {
    /// Adds `skip` unless one with its ordinal is there.
    pub(crate) fn add_to(list: &mut Vec<HunkSkip>, skip: HunkSkip) {
        if !list.iter().any(|s| s.ordinal == skip.ordinal) {
            list.push(skip);
        }
    }
}

/// The edit `key` names in raw patch `t` (D49), or why it names none: the patch is not an
/// `at` pattern patch, its content changed since the key was made, or the edit is gone.
pub(crate) fn check_hunk(
    raw: &RawLoad,
    t: usize,
    key: &HunkKey,
    fps: &mut Fingerprints,
) -> Result<HunkSkip, String> {
    let Some(rp) = raw.patches.get(t) else {
        return Err("the patch is not loaded".to_string());
    };
    let id = rp.patch.id().unwrap_or_default().to_string();
    let p = match &rp.patch {
        Patch::Pattern(p) if p.position == InsertPosition::At => p,
        _ => return Err(format!("{id} has no separate edits")),
    };
    let same = fps
        .get(t)
        .is_some_and(|f| f.anchor == key.fp.anchor && f.payload == key.fp.payload);
    if !same {
        return Err(format!("{id} changed since the choice was made"));
    }
    edits::find_hunk(&p.pattern, &p.payload, key.ordinal, &key.vanilla)
        .map_err(|why| format!("{id}: {why}"))?;
    Ok(HunkSkip {
        ordinal: key.ordinal,
        vanilla: key.vanilla.clone(),
        keep: key.keep.clone(),
    })
}

/// `check_hunk` for callers outside qm-core (the compat export): whether `key` still names
/// an edit of raw patch `t`, else why not.
pub fn check_hunk_key(
    raw: &RawLoad,
    t: usize,
    key: &HunkKey,
    fps: &mut Fingerprints,
) -> Result<(), String> {
    check_hunk(raw, t, key, fps).map(|_| ())
}

impl PatchExtra {
    /// The D29 legacy id, then the D62 stem names.
    pub fn legacy_names(&self) -> impl Iterator<Item = &str> {
        self.legacy_id
            .as_deref()
            .into_iter()
            .chain(self.legacy_ids.iter().map(String::as_str))
    }
}

/// One patch as read from disk, before the profile's layers.
#[derive(Clone, Debug)]
pub struct RawPatch {
    pub patch: Patch,
    /// The mod's priority for this patch's file (the plan's mod priority included).
    pub base_priority: Priority,
    /// The patch's own `priority`.
    pub explicit_priority: Option<Priority>,
    /// The toml path relative to the Mods folder (`<mod>/assets` for the asset patch).
    pub path: PathBuf,
    pub vars: HashMap<String, String>,
    /// Index into `RawLoad::mods`.
    pub mod_ix: u32,
    /// The toml path inside the mod, '/' and lowercase ("assets" for the asset patch).
    pub source: String,
    /// 1-based among the mod's toml patches with this primary target (0 for the asset patch).
    pub ordinal: u32,
}

/// Everything read from the Mods folder. Built once per staged profile; previews reuse it.
#[derive(Clone, Debug, Default)]
pub struct RawLoad {
    pub mod_dir: PathBuf,
    /// The mods that loaded, in loader order.
    pub mods: Vec<ModMeta>,
    /// Their patches, in loader order (mod order, then file order).
    pub patches: Vec<RawPatch>,
    /// Their `[[overrides]]` entries with the index of the mod (into `mods`), in loader order.
    pub overrides: Vec<(u32, Lenient<Override>)>,
    /// Faction candidates of the loaded mods.
    pub factions: Vec<LoadedFaction>,
    /// Configurations of the loaded mods for the built-in menu.
    pub configs: Vec<crate::patch::menu::LoadedModConfig>,
    /// Mods skipped because they failed to load, or require one that did (D15).
    pub load_errors: Vec<LoadError>,
    pub warnings: Vec<String>,
}

/// Reads the Mods folder: the blacklist, the plan's enabled filter and mod priorities, every
/// enabled mod's toml files, assets, faction and mod.json data. A mod that fails to load is
/// skipped with its requirers and listed in `load_errors`. `Err` only when the Mods folder or
/// its blacklist cannot be read.
pub fn load_raw(mod_dir: &Path, plan: Option<&LoadOrderPlan>) -> Result<RawLoad> {
    loader::read_mods(mod_dir, plan)
}

impl RawLoad {
    /// Pure, no disk: this load plus one mod given as text (the export preview, §8.2).
    /// `manifest_json` is parsed by `ModManifest::from_json_str` and `qm_toml` as a qm.toml in
    /// the folder `<mod_dir>/<id>`, which is never read. The mod takes the place the loader
    /// gives that folder; its patches and overrides are added as `load_raw` adds them, with
    /// the mod.json priority. Err when either text does not parse, a patch needs a file, or
    /// a mod with that id is already loaded.
    pub fn with_extra_mod(&self, manifest_json: &str, qm_toml: &str) -> Result<RawLoad, String> {
        loader::extra_mod(self, manifest_json, qm_toml)
    }
}

/// `apply_plan`'s result: what a `PatchTable` is built from (`PatchTable::from_applied`).
#[derive(Clone, Debug, Default)]
pub struct Applied {
    pub patches: Vec<(Patch, Priority, PathBuf)>,
    pub targets: HashSet<String>,
    pub vars: HashMap<String, String>,
    /// `[vars]` names two mods set to different values (G12); `vars` keeps the last.
    pub var_collisions: Vec<loader::VarCollision>,
    pub mods: Vec<ModMeta>,
    pub order_rules: Vec<OrderRule>,
    pub compat_rules: Vec<(OrderRule, String)>,
    pub stack_mode: StackMode,
    /// The plan's `line_rules_v2` (D50, D51); false when absent.
    pub line_rules_v2: bool,
    pub load_errors: Vec<LoadError>,
    pub report: PlanReport,
}

/// Lays the plan over what was read. Pure: no disk. Layers, in order: compat overrides (§8.1;
/// the profile wins over them, D13), the profile layer (refs bound, glue-replaced patches
/// dropped, disables, patch priorities), factions, legacy glue (a copy whose base mod did not
/// load is skipped, D19, D60), then the order rules, compat rules and stack mode. A plan with
/// a hard error gives no patches (D16). Fingerprints are computed as the DLL does (`FpMode::Lazy`).
pub fn apply_plan(raw: &RawLoad, plan: &PlanLoad) -> Applied {
    apply_plan_with(raw, plan, FpMode::Lazy)
}

/// `apply_plan` with a choice of how many fingerprints binding computes. The output is the
/// same in both modes; `FpMode::All` also reports drift the output does not depend on.
pub fn apply_plan_with(raw: &RawLoad, plan: &PlanLoad, mode: FpMode) -> Applied {
    let mut report = PlanReport {
        plan_error: plan.error.as_ref().map(ToString::to_string),
        plan_warnings: plan.warnings.clone(),
        warnings: raw.warnings.clone(),
        ..PlanReport::default()
    };
    if report.plan_error.is_some() {
        return Applied {
            report,
            ..Applied::default()
        };
    }
    let p = plan.plan.as_ref();
    let layered = layer_patches_with(raw, p, mode, &mut report.bindings);
    report.overrides = layered.compat.reports;
    report.warnings.extend(layered.notes);
    let var_collisions = loader::var_collisions(&layered.patches);
    let (patches, targets, vars) = loader::process_patches(layered.patches);
    for rp in &raw.patches {
        let (id, stack) = match &rp.patch {
            Patch::Pattern(x) => (x.id.as_deref(), x.stack.as_ref()),
            Patch::Regex(x) => (x.id.as_deref(), x.stack.as_ref()),
            _ => (None, None),
        };
        if let Some(e) = stack.and_then(Lenient::error) {
            report.warnings.push(format!(
                "{}: `stack` must be \"first\" or \"last\"; it is ignored: {e}",
                id.unwrap_or("a patch")
            ));
        }
        if let Patch::Pattern(x) = &rp.patch {
            if let Some(e) = x.apply_as.as_ref().and_then(Lenient::error) {
                report.warnings.push(format!(
                    "{}: `apply_as` must be \"edits\" or \"literal\"; it runs literal: {e}",
                    x.id.as_deref().unwrap_or("a patch")
                ));
            }
        }
    }
    let graph = ModGraph::build(&raw.mods);
    for (a, b) in graph.mutual_pairs() {
        report.warnings.push(format!(
            "stack_cycle: '{a}' and '{b}' depend on each other; their blocks keep the legacy order"
        ));
    }
    Applied {
        patches,
        targets,
        vars,
        var_collisions,
        mods: raw.mods.clone(),
        order_rules: layered.order_rules,
        compat_rules: layered.compat.rules,
        stack_mode: p.map(|p| p.stack_mode()).unwrap_or_default(),
        line_rules_v2: p.is_some_and(LoadOrderPlan::line_rules_v2),
        load_errors: raw.load_errors.clone(),
        report,
    }
}

/// The names a plan entry may give a patch: its id, `mod:name`, its D29 legacy id and its
/// D62 stem names.
pub fn plan_keys(patch: &Patch) -> Vec<String> {
    let mut keys = vec![
        patch.id().unwrap_or_default().to_string(),
        format!(
            "{}:{}",
            patch.mod_id().unwrap_or_default(),
            patch.name().unwrap_or("")
        ),
    ];
    keys.extend(patch.extra().legacy_names().map(str::to_string));
    keys
}

/// Whether the plan enables mod `m` (by id or folder).
fn mod_enabled(plan: &LoadOrderPlan, m: &str) -> bool {
    plan.enabled_mods.iter().any(|e| e.eq_ignore_ascii_case(m))
}

/// Whether mod `m` loaded (by id or folder).
fn mod_loaded(raw: &RawLoad, m: &str) -> bool {
    raw.mods
        .iter()
        .any(|x| x.id.eq_ignore_ascii_case(m) || x.folder.eq_ignore_ascii_case(m))
}

/// Why a copy's base mod `m` is not on (D19, D60), as the end of the Waiting detail: it
/// failed to load, it was skipped as a requirer of one that did (D15), it is enabled with
/// no folder, or it is off.
fn mod_absence(raw: &RawLoad, plan: &LoadOrderPlan, m: &str) -> String {
    let failed = raw
        .load_errors
        .iter()
        .any(|e| e.mod_id.eq_ignore_ascii_case(m) || e.folder.eq_ignore_ascii_case(m));
    let root = raw.load_errors.iter().find(|e| {
        e.skipped_dependents
            .iter()
            .any(|d| d.eq_ignore_ascii_case(m))
    });
    if failed {
        format!("because {m} failed to load")
    } else if let Some(e) = root {
        format!("because {m} needs {}, which failed to load", e.mod_id)
    } else if mod_enabled(plan, m) {
        format!("while {m} is missing")
    } else {
        format!("while {m} is off")
    }
}

/// The mod a glue's base patch belongs to, when that mod did not load (D19, D60). The mod is
/// `base_mod_id`, else the `<mod>` of a base id in the loader's fallback form
/// `<mod>:<target>:<n>` (an explicit id's prefix need not be a mod). None when the base patch
/// loaded, when the mod loaded or when the copy names no mod: it keeps running.
fn glue_base_mod_off(raw: &RawLoad, glue: &GluePatchEntry) -> Option<String> {
    let base = glue.replaces_patch_id.as_deref()?;
    if raw
        .patches
        .iter()
        .any(|rp| plan_keys(&rp.patch).iter().any(|k| k == base))
    {
        return None;
    }
    let m = glue
        .base_mod_id
        .clone()
        .or_else(|| fallback_id_mod(base).map(str::to_string))
        // An old copy of a copy named `qm_glue`, which never loads (D59).
        .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case(GLUE_MOD))?;
    (!mod_loaded(raw, &m)).then_some(m)
}

/// The first copy of a chain of copies of copies (D59): follows `replaces_patch_id` through
/// the plan's copy ids. A loop ends at the copy it started from.
fn root_glue<'a>(glues: &[&'a GluePatchEntry], glue: &'a GluePatchEntry) -> &'a GluePatchEntry {
    let mut at = glue;
    for _ in 0..glues.len() {
        let Some(next) = at
            .replaces_patch_id
            .as_deref()
            .and_then(|b| glues.iter().find(|g| g.id == b))
        else {
            return at;
        };
        if next.id == glue.id {
            return glue;
        }
        at = next;
    }
    at
}

/// The `<mod>` of a loader fallback id `<mod>:<target>:<n>`, with or without the D29
/// `#<source>` suffix.
fn fallback_id_mod(id: &str) -> Option<&str> {
    let id = id.split_once('#').map_or(id, |(base, _)| base);
    let mut parts = id.split(':');
    let (m, target, n) = (parts.next()?, parts.next()?, parts.next()?);
    let numbered = !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    (parts.next().is_none() && !m.is_empty() && !target.is_empty() && numbered).then_some(m)
}

/// Which kind of plan entry a ref belongs to (§7.1, §7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    Disable,
    Order,
    Priority,
    /// An adjusted copy's base patch.
    Glue,
    Anchor,
    /// How an `at` patch runs (D46).
    ApplyAs,
    /// An edit of an `at` patch that does not run (D49).
    Hunk,
    /// A patch a stack rule names (D55).
    Stack,
}

impl BindKind {
    /// Whether drift changes what the entry does, so even a single id hit is fingerprinted
    /// in the DLL (D32). For the others a drifted single hit applies as a bound one.
    fn drift_relevant(self) -> bool {
        matches!(self, BindKind::Anchor | BindKind::Hunk)
    }

    /// The `fix_of` key prefix.
    fn key(self) -> &'static str {
        match self {
            BindKind::Disable => "disable",
            BindKind::Order => "order",
            BindKind::Priority => "priority",
            BindKind::Glue => "glue",
            BindKind::Anchor => "anchor",
            BindKind::ApplyAs => "apply_as",
            BindKind::Hunk => "hunk",
            BindKind::Stack => "stack",
        }
    }
}

/// How many fingerprints binding computes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FpMode {
    /// The DLL (D32): only the ref's mod's patches when the id finds none or several, only
    /// the hit when it finds one and the kind is drift-relevant, else none.
    #[default]
    Lazy,
    /// The simulation: every hit too, so drift is always reported.
    All,
}

/// Fingerprints of the raw patches, each computed on first use.
pub struct Fingerprints<'a> {
    raw: &'a RawLoad,
    mode: FpMode,
    cache: HashMap<usize, PatchFingerprint>,
}

impl<'a> Fingerprints<'a> {
    pub fn new(raw: &'a RawLoad, mode: FpMode) -> Self {
        Fingerprints {
            raw,
            mode,
            cache: HashMap::new(),
        }
    }

    /// The fingerprint of raw patch `ix`, with its mod's version.
    pub fn get(&mut self, ix: usize) -> Option<&PatchFingerprint> {
        if !self.cache.contains_key(&ix) {
            let rp = self.raw.patches.get(ix)?;
            let m = self.raw.mods.get(rp.mod_ix as usize);
            let base = m.map(|m| m.base.as_path()).unwrap_or(Path::new(""));
            let mut fp = rp.patch.fingerprint(base, &rp.source, rp.ordinal);
            fp.mod_version = m.and_then(|m| m.version.clone());
            self.cache.insert(ix, fp);
        }
        self.cache.get(&ix)
    }
}

/// A ref bound to the raw patches: its status, and the raw patch indexes the entry applies
/// to under the drift policy (§7.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub status: BindingStatus,
    pub targets: Vec<usize>,
    /// The id the fix moved to (Rebound, or re-bound and drifted).
    pub new_id: Option<String>,
}

/// Binds a ref to the raw patches (§7.1). The id lookup is the loader's rule (id, `mod:name`,
/// a D29 old id; an Order name matches as a rule name does), within the ref's mod.
/// - Mod not loaded: Waiting.
/// - One hit: Bound, or Drifted when its content differs (checked per `FpMode`).
/// - Several: narrowed by content, then by `(source, ordinal)`; Bound, else Ambiguous. When
///   no hit has the content, the `(source, ordinal)` match is Drifted.
/// - None: the mod's patch with the same anchor and payload is Rebound (narrowed by place);
///   an anchor-only match is Rebound and Drifted, except for a Disable or a copy's base,
///   which never widen (D20) and are Ambiguous. Else Missing.
///
/// Ambiguous applies to the one hit whose id is exactly the ref's, if there is one.
pub fn bind(r: &PatchRef, kind: BindKind, fps: &mut Fingerprints) -> Bound {
    let raw = fps.raw;
    let id_of = |i: usize| {
        raw.patches
            .get(i)
            .and_then(|rp| rp.patch.id())
            .unwrap_or_default()
            .to_string()
    };
    let at = |status: BindingStatus, targets: Vec<usize>, new_id: Option<String>| Bound {
        status,
        targets,
        new_id,
    };
    let Some(mod_ix) = raw.mods.iter().position(|m| {
        m.id.eq_ignore_ascii_case(&r.mod_id) || m.folder.eq_ignore_ascii_case(&r.mod_id)
    }) else {
        let mods = vec![r.mod_id.clone()];
        return at(BindingStatus::Waiting { mods }, Vec::new(), None);
    };
    let in_mod: Vec<usize> = raw
        .patches
        .iter()
        .enumerate()
        .filter(|(_, rp)| rp.mod_ix as usize == mod_ix)
        .map(|(i, _)| i)
        .collect();
    let named = |p: &Patch| match kind {
        BindKind::Order => order::name_matches(&r.id, p),
        _ => plan_keys(p).contains(&r.id),
    };
    let hits: Vec<usize> = in_mod
        .iter()
        .copied()
        .filter(|&i| raw.patches.get(i).is_some_and(|rp| named(&rp.patch)))
        .collect();
    let ambiguous = |cands: &[usize], from_hits: bool| {
        let exact: Vec<usize> = if from_hits {
            cands
                .iter()
                .copied()
                .filter(|&i| raw.patches.get(i).and_then(|rp| rp.patch.id()) == Some(&r.id))
                .collect()
        } else {
            Vec::new()
        };
        let candidates = cands.iter().map(|&i| id_of(i)).collect();
        let targets = if exact.len() == 1 { exact } else { Vec::new() };
        at(BindingStatus::Ambiguous { candidates }, targets, None)
    };
    let placed = |cands: &[usize], fp: &PatchFingerprint| -> Option<usize> {
        let v: Vec<usize> = cands
            .iter()
            .copied()
            .filter(|&i| {
                raw.patches
                    .get(i)
                    .is_some_and(|rp| rp.source == fp.source && rp.ordinal == fp.ordinal)
            })
            .collect();
        match v.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    };
    let drift = |fps: &mut Fingerprints, i: usize, fp: &PatchFingerprint| {
        fps.get(i).map_or((false, false), |f| {
            (f.anchor != fp.anchor, f.payload != fp.payload)
        })
    };
    let Some(fp) = r.fp.as_ref() else {
        return match hits.as_slice() {
            [] => at(BindingStatus::Missing, Vec::new(), None),
            [one] => at(BindingStatus::Bound, vec![*one], None),
            _ => ambiguous(&hits, true),
        };
    };

    match hits.as_slice() {
        [one] => {
            let check = fps.mode == FpMode::All || kind.drift_relevant();
            let (a, p) = if check {
                drift(fps, *one, fp)
            } else {
                (false, false)
            };
            let status = if a || p {
                BindingStatus::Drifted {
                    anchor: a,
                    payload: p,
                }
            } else {
                BindingStatus::Bound
            };
            at(status, vec![*one], None)
        }
        [] => {
            let mut same = Vec::new();
            let mut anchor_only = Vec::new();
            for &i in &in_mod {
                let (a, p) = drift(fps, i, fp);
                if !a && !p {
                    same.push(i);
                } else if !a {
                    anchor_only.push(i);
                }
            }
            let rebound = |i: usize| {
                let to = id_of(i);
                let status = BindingStatus::Rebound {
                    from: r.id.clone(),
                    to: to.clone(),
                };
                at(status, vec![i], Some(to))
            };
            match same.as_slice() {
                [one] => return rebound(*one),
                [_, _, ..] => {
                    return match placed(&same, fp) {
                        Some(i) => rebound(i),
                        None => ambiguous(&same, false),
                    }
                }
                [] => {}
            }
            let pick = match anchor_only.as_slice() {
                [one] => Some(*one),
                [] => None,
                _ => placed(&anchor_only, fp),
            };
            match pick {
                // Never widen (D20): another patch is not run another way on a guess.
                Some(i)
                    if matches!(
                        kind,
                        BindKind::Disable
                            | BindKind::Glue
                            | BindKind::ApplyAs
                            | BindKind::Hunk
                            | BindKind::Stack
                    ) =>
                {
                    ambiguous(&[i], false)
                }
                Some(i) => {
                    let status = BindingStatus::Drifted {
                        anchor: false,
                        payload: true,
                    };
                    at(status, vec![i], Some(id_of(i)))
                }
                None if !anchor_only.is_empty() => ambiguous(&anchor_only, false),
                None => at(BindingStatus::Missing, Vec::new(), None),
            }
        }
        _ => {
            let same: Vec<usize> = hits
                .iter()
                .copied()
                .filter(|&i| drift(fps, i, fp) == (false, false))
                .collect();
            match same.as_slice() {
                [one] => at(BindingStatus::Bound, vec![*one], None),
                [_, _, ..] => match placed(&same, fp) {
                    Some(i) => at(BindingStatus::Bound, vec![i], None),
                    None => ambiguous(&same, true),
                },
                [] => match placed(&hits, fp) {
                    Some(i) => {
                        let (a, p) = drift(fps, i, fp);
                        let status = BindingStatus::Drifted {
                            anchor: a,
                            payload: p,
                        };
                        at(status, vec![i], None)
                    }
                    None => ambiguous(&hits, true),
                },
            }
        }
    }
}

/// The plain-text detail of a binding.
fn binding_detail(kind: BindKind, entry: &str, r: &PatchRef, b: &Bound) -> String {
    let what = match kind {
        BindKind::Disable => "it stays off until you review it",
        BindKind::Order => "the order rule still applies",
        BindKind::Priority => "the priority still applies",
        BindKind::Glue => "your copy is older and still runs",
        BindKind::Anchor => "review the fix",
        BindKind::ApplyAs => "it still runs the way you chose",
        BindKind::Hunk => "nothing is skipped until you review the fix",
        BindKind::Stack => "the rule still applies",
    };
    match &b.status {
        BindingStatus::Bound => format!("{entry} found"),
        BindingStatus::Rebound { from, to } => format!("{from} is now {to}; the fix follows it"),
        BindingStatus::Drifted { .. } => match &b.new_id {
            Some(to) => format!("{entry} is now {to} and its code changed; {what}"),
            None => format!("{entry} changed in {}; {what}", r.mod_id),
        },
        BindingStatus::Ambiguous { candidates } if b.targets.is_empty() => format!(
            "{} patches could be {entry}; nothing is applied until you pick one",
            candidates.len()
        ),
        BindingStatus::Ambiguous { candidates } => format!(
            "{} patches could be {entry}; only the one with that exact id is used",
            candidates.len()
        ),
        BindingStatus::Missing if kind == BindKind::Glue => {
            format!("{entry} is no longer in {}; your copy still runs", r.mod_id)
        }
        BindingStatus::Missing => format!("{entry} is no longer in {}", r.mod_id),
        BindingStatus::Waiting { mods } => format!("Waiting for {}", mods.join(", ")),
        BindingStatus::Suspended { reason } => reason.clone(),
    }
}

/// The plan's profile layer, bound to the raw patches.
struct Layer<'a> {
    /// Raw indexes turned off through a ref.
    disabled: HashSet<usize>,
    /// `disabled_patches` entries with no ref: they match by name as before.
    legacy_disabled: Vec<&'a str>,
    /// Raw index -> (rank of the entry's key in the patch's `plan_keys`, priority), through a
    /// ref. The lowest rank wins, as the legacy lookup takes the first key that has one.
    priorities: HashMap<usize, (usize, Priority)>,
    /// `patch_priorities` entries with no ref.
    legacy_priorities: HashMap<&'a str, Priority>,
    /// The adjusted copies that run, each with the raw indexes it replaces.
    glues: Vec<(&'a GluePatchEntry, Vec<usize>)>,
    /// Raw indexes a running copy replaces, through a ref.
    replaced: HashSet<usize>,
    /// Base ids of running copies with no ref.
    legacy_replaced: HashSet<&'a str>,
    order_rules: Vec<OrderRule>,
    /// Raw index -> the anchor fixes that apply to it, in plan order.
    anchors: HashMap<usize, Vec<&'a AnchorFix>>,
    /// Raw index -> (rank of the entry's key, mode, entry name), through a ref (D46). The
    /// lowest rank wins, as for priorities.
    apply_as: HashMap<usize, (usize, ApplyAs, &'a str)>,
    /// `apply_as` entries with no ref, by patch name.
    legacy_apply_as: HashMap<&'a str, ApplyAs>,
    /// Raw index -> the edits it skips, each with its fix id (D49).
    skips: HashMap<usize, Vec<(HunkSkip, String)>>,
    /// Raw index -> the stack rules it carries (D55).
    stack_rules: HashMap<usize, Vec<StackRuleSpec>>,
}

/// Adds a binding; its fix id is `fix_of[key]`, else `own_id`.
fn push_binding(
    bindings: &mut Vec<FixBinding>,
    plan: &LoadOrderPlan,
    key: String,
    own_id: &str,
    status: BindingStatus,
    detail: String,
    new_id: Option<String>,
) {
    info!("Plan fix {key}: {detail}");
    let fix_id = plan
        .fix_of
        .get(&key)
        .and_then(Lenient::ok)
        .cloned()
        .unwrap_or_else(|| own_id.to_string());
    bindings.push(FixBinding {
        fix_id,
        key,
        status,
        detail,
        new_id,
    });
}

/// Adds the binding of a ref'd entry: `subject` names the fix (`<kind>:<subject>`), `entry`
/// is the plan string the ref is for.
fn record(
    bindings: &mut Vec<FixBinding>,
    plan: &LoadOrderPlan,
    (kind, subject, entry): (BindKind, &str, &str),
    r: &PatchRef,
    b: &Bound,
) {
    let key = format!("{}:{subject}", kind.key());
    let detail = binding_detail(kind, entry, r, b);
    push_binding(
        bindings,
        plan,
        key,
        subject,
        b.status.clone(),
        detail,
        b.new_id.clone(),
    );
}

/// The binding of an adjusted copy skipped while its base mod `m` did not load (D19, D60).
fn glue_waiting(
    bindings: &mut Vec<FixBinding>,
    raw: &RawLoad,
    plan: &LoadOrderPlan,
    glue: &GluePatchEntry,
    m: &str,
) {
    let detail = format!(
        "Waiting for {m}: the adjusted copy '{}' is skipped {}",
        glue.name,
        mod_absence(raw, plan, m)
    );
    let status = BindingStatus::Waiting {
        mods: vec![m.to_string()],
    };
    let key = format!("glue:{}", glue.id);
    push_binding(bindings, plan, key, &glue.id, status, detail, None);
}

/// The ref of a legacy entry, when the plan has a valid one.
fn ref_of<'a>(plan: &'a LoadOrderPlan, entry: &str) -> Option<&'a PatchRef> {
    plan.refs.get(entry).and_then(Lenient::ok)
}

/// Binds the plan's legacy fields through their refs and records each binding (§7.1, §7.2).
/// An entry with no ref matches by name as before (D24, D29).
fn profile_layer<'a>(
    raw: &RawLoad,
    plan: &'a LoadOrderPlan,
    mode: FpMode,
    bindings: &mut Vec<FixBinding>,
) -> Layer<'a> {
    let mut fps = Fingerprints::new(raw, mode);
    let mut layer = Layer {
        disabled: HashSet::new(),
        legacy_disabled: Vec::new(),
        priorities: HashMap::new(),
        legacy_priorities: HashMap::new(),
        glues: Vec::new(),
        replaced: HashSet::new(),
        legacy_replaced: HashSet::new(),
        order_rules: Vec::new(),
        anchors: HashMap::new(),
        apply_as: HashMap::new(),
        legacy_apply_as: HashMap::new(),
        skips: HashMap::new(),
        stack_rules: HashMap::new(),
    };
    for d in plan.disabled_patches.iter().filter_map(Lenient::ok) {
        match ref_of(plan, d) {
            None => layer.legacy_disabled.push(d.as_str()),
            Some(r) => {
                let b = bind(r, BindKind::Disable, &mut fps);
                layer.disabled.extend(b.targets.iter().copied());
                record(bindings, plan, (BindKind::Disable, d, d), r, &b);
            }
        }
    }

    let mut prios: Vec<(&String, Priority)> = plan
        .patch_priorities
        .iter()
        .filter_map(|(k, v)| v.ok().map(|p| (k, *p)))
        .collect();
    prios.sort();
    for (k, prio) in prios {
        match ref_of(plan, k) {
            None => {
                layer.legacy_priorities.insert(k.as_str(), prio);
            }
            Some(r) => {
                let b = bind(r, BindKind::Priority, &mut fps);
                for &t in &b.targets {
                    // A re-bound id is not among the patch's keys: it ranks last.
                    let rank = raw
                        .patches
                        .get(t)
                        .and_then(|rp| plan_keys(&rp.patch).iter().position(|x| x == k))
                        .unwrap_or(usize::MAX);
                    let slot = layer.priorities.entry(t).or_insert((rank, prio));
                    if rank < slot.0 {
                        *slot = (rank, prio);
                    }
                }
                record(bindings, plan, (BindKind::Priority, k, k), r, &b);
            }
        }
    }

    // How `at` patches run (D46): bound like priorities.
    for (k, mode) in plan
        .apply_as
        .iter()
        .filter_map(|(k, v)| v.ok().map(|a| (k, *a)))
    {
        match ref_of(plan, k) {
            None => {
                layer.legacy_apply_as.insert(k.as_str(), mode);
            }
            Some(r) => {
                let b = bind(r, BindKind::ApplyAs, &mut fps);
                for &t in &b.targets {
                    let rank = raw
                        .patches
                        .get(t)
                        .and_then(|rp| plan_keys(&rp.patch).iter().position(|x| x == k))
                        .unwrap_or(usize::MAX);
                    let slot = layer.apply_as.entry(t).or_insert((rank, mode, k.as_str()));
                    if rank < slot.0 {
                        *slot = (rank, mode, k.as_str());
                    }
                }
                record(bindings, plan, (BindKind::ApplyAs, k, k), r, &b);
            }
        }
    }

    // A copy of a copy names the older copy as its base (D59): the older copy no longer
    // runs, and the newer one stands in for the first copy's base.
    let glues: Vec<&GluePatchEntry> = plan.glues().collect();
    let superseded: HashSet<&str> = glues
        .iter()
        .filter_map(|g| g.replaces_patch_id.as_deref())
        .filter(|b| glues.iter().any(|o| o.id == *b))
        .collect();
    for &glue in &glues {
        if superseded.contains(glue.id.as_str()) {
            info!("Adjusted copy '{}' is replaced by a newer copy of it", glue.id);
            continue;
        }
        let root = root_glue(&glues, glue);
        let base = root.replaces_patch_id.as_deref();
        let Some((base, r)) = base.and_then(|b| ref_of(plan, b).map(|r| (b, r))) else {
            if let Some(m) = glue_base_mod_off(raw, root) {
                glue_waiting(bindings, raw, plan, glue, &m);
                continue;
            }
            layer.legacy_replaced.extend(base);
            let targets = base.map_or_else(Vec::new, |b| {
                raw.patches
                    .iter()
                    .enumerate()
                    .filter(|(_, rp)| plan_keys(&rp.patch).iter().any(|k| k == b))
                    .map(|(i, _)| i)
                    .collect()
            });
            layer.glues.push((glue, targets));
            continue;
        };
        let b = bind(r, BindKind::Glue, &mut fps);
        if matches!(b.status, BindingStatus::Waiting { .. }) {
            // The base mod did not load: off, failed or missing (D19, D60).
            glue_waiting(bindings, raw, plan, glue, &r.mod_id);
            continue;
        }
        layer.replaced.extend(b.targets.iter().copied());
        layer.glues.push((glue, b.targets.clone()));
        record(bindings, plan, (BindKind::Glue, &glue.id, base), r, &b);
    }

    for rule in plan.order_rules() {
        let subject = rule.patch.clone();
        let mut bind_name = |name: &str| -> Option<Vec<String>> {
            if name.trim().ends_with(":*") {
                return None;
            }
            let r = ref_of(plan, name)?;
            let b = bind(r, BindKind::Order, &mut fps);
            record(bindings, plan, (BindKind::Order, &subject, name), r, &b);
            Some(
                b.targets
                    .iter()
                    .map(|&t| order_name(raw, name, t))
                    .collect(),
            )
        };
        let patch = match bind_name(&rule.patch) {
            None => vec![rule.patch.clone()],
            Some(v) => v,
        };
        let mut names = |list: &[String]| -> Vec<String> {
            list.iter()
                .flat_map(|n| bind_name(n).unwrap_or_else(|| vec![n.clone()]))
                .collect()
        };
        let before = names(&rule.before);
        let after = names(&rule.after);
        for patch in patch {
            layer.order_rules.push(OrderRule {
                patch,
                before: before.clone(),
                after: after.clone(),
            });
        }
    }

    // Anchor fixes (§7.2): applied when bound or when only A's payload changed; suspended
    // (A as authored) when what A matches changed; nothing when ambiguous.
    for fix in plan.anchor_fixes.iter().filter_map(Lenient::ok) {
        let r = &fix.patch;
        let mut b = bind(r, BindKind::Anchor, &mut fps);
        let apply = match b.status {
            BindingStatus::Bound | BindingStatus::Rebound { .. } => true,
            BindingStatus::Drifted { anchor: false, .. } => true,
            BindingStatus::Drifted { anchor: true, .. } => {
                b.status = BindingStatus::Suspended {
                    reason: format!(
                        "{} changed where it attaches; the fix is off until you review it",
                        r.id
                    ),
                };
                false
            }
            _ => false,
        };
        if apply {
            for &t in &b.targets {
                layer.anchors.entry(t).or_default().push(fix);
            }
        }
        let detail = binding_detail(BindKind::Anchor, &r.id, r, &b);
        info!("Plan fix anchor:{}: {detail}", r.id);
        bindings.push(FixBinding {
            fix_id: fix.fix_id.clone(),
            key: format!("anchor:{}", r.id),
            status: b.status.clone(),
            detail,
            new_id: b.new_id.clone(),
        });
    }

    // Skipped edits (D49): a key that no longer names one of the patch's edits skips nothing
    // and needs review (never widen).
    for fix in plan.skip_hunks.iter().filter_map(Lenient::ok) {
        let r = &fix.patch;
        let mut b = bind(r, BindKind::Hunk, &mut fps);
        let mut detail = binding_detail(BindKind::Hunk, &r.id, r, &b);
        if matches!(b.status, BindingStatus::Bound | BindingStatus::Rebound { .. }) {
            for &t in &b.targets {
                match check_hunk(raw, t, &fix.hunk, &mut fps) {
                    Ok(skip) => layer
                        .skips
                        .entry(t)
                        .or_default()
                        .push((skip, fix.fix_id.clone())),
                    Err(why) => {
                        b.status = BindingStatus::Drifted {
                            anchor: false,
                            payload: true,
                        };
                        detail = format!("{why}; nothing is skipped until you review the fix");
                    }
                }
            }
        }
        let key = format!("hunk:{}:{}", r.id, fix.hunk.ordinal);
        info!("Plan fix {key}: {detail}");
        bindings.push(FixBinding {
            fix_id: fix.fix_id.clone(),
            key,
            status: b.status.clone(),
            detail,
            new_id: b.new_id.clone(),
        });
    }

    // Stack rules (D55): both patches must bind to one patch each; a rule whose other patch
    // is not loaded waits for it.
    for fix in plan.stack_rules.iter().filter_map(Lenient::ok) {
        stack_rule_binding(raw, fix, &mut fps, &mut layer, bindings);
    }
    layer
}

/// Binds one stack rule (D55) and records a binding per patch it names.
fn stack_rule_binding(
    raw: &RawLoad,
    fix: &StackRuleFix,
    fps: &mut Fingerprints,
    layer: &mut Layer,
    bindings: &mut Vec<FixBinding>,
) {
    let one = |b: &Bound| match (&b.status, b.targets.as_slice()) {
        (
            BindingStatus::Bound | BindingStatus::Rebound { .. } | BindingStatus::Drifted { .. },
            [t],
        ) => Some(*t),
        _ => None,
    };
    let mut push = |r: &PatchRef, b: &Bound| {
        let detail = binding_detail(BindKind::Stack, &r.id, r, b);
        let key = format!("stack:{}", r.id);
        info!("Plan fix {key}: {detail}");
        bindings.push(FixBinding {
            fix_id: fix.fix_id.clone(),
            key,
            status: b.status.clone(),
            detail,
            new_id: b.new_id.clone(),
        });
    };
    let b = bind(&fix.patch, BindKind::Stack, fps);
    push(&fix.patch, &b);
    let me = one(&b);
    let others = [(fix.below.as_ref(), true), (fix.above.as_ref(), false)];
    for (r, below) in others {
        let Some(r) = r else { continue };
        let ob = bind(r, BindKind::Stack, fps);
        push(r, &ob);
        let (Some(t), Some(o)) = (me, one(&ob)) else {
            continue;
        };
        let Some(other_id) = raw.patches.get(o).and_then(|rp| rp.patch.id()) else {
            continue;
        };
        let name = order_name(raw, other_id, o);
        let spec = StackRuleSpec {
            site: Some(fix.site.clone()),
            side: Some(fix.side),
            below: below.then(|| name.clone()),
            above: (!below).then_some(name),
            source: RuleSource::Profile {
                fix_id: fix.fix_id.clone(),
            },
        };
        layer.stack_rules.entry(t).or_default().push(spec);
    }
}

/// The rule name for raw patch `t`: the entry's own name when it names only `t`, else `t`'s
/// id, else the exact form (`=<id>`, which a D29 duplicate's old id does not match).
pub(crate) fn order_name(raw: &RawLoad, name: &str, t: usize) -> String {
    let Some(id) = raw.patches.get(t).and_then(|rp| rp.patch.id()) else {
        return name.to_string();
    };
    let only_t = |n: &str| {
        let hits: Vec<usize> = raw
            .patches
            .iter()
            .enumerate()
            .filter(|(_, rp)| order::name_matches(n, &rp.patch))
            .map(|(i, _)| i)
            .collect();
        hits == [t]
    };
    let exact = format!("{}{id}", order::EXACT);
    [name.to_string(), id.to_string(), exact]
        .into_iter()
        .find(|n| only_t(n))
        .unwrap_or_else(|| id.to_string())
}

/// The patch layers of `apply_plan`, as patches with their vars, in loader order.
pub(crate) fn layer_patches(
    raw: &RawLoad,
    plan: Option<&LoadOrderPlan>,
    bindings: &mut Vec<FixBinding>,
) -> Vec<LoadedPatch> {
    layer_patches_with(raw, plan, FpMode::Lazy, bindings).patches
}

/// What the patch layers give `apply_plan`.
struct Layered {
    /// Notes for the load report: a faction pick that could not be honoured (D82).
    notes: Vec<String>,
    patches: Vec<LoadedPatch>,
    /// The plan's order rules with their refs bound.
    order_rules: Vec<OrderRule>,
    compat: CompatLayer,
}

/// The raw patches the profile layer changes (D13: they win over compat overrides).
fn profile_touch(raw: &RawLoad, layer: Option<&Layer>) -> ProfileTouch {
    let mut touch = ProfileTouch::default();
    let Some(l) = layer else {
        return touch;
    };
    touch.copies = l.glues.iter().map(|(g, _)| g.id.to_lowercase()).collect();
    for (ix, rp) in raw.patches.iter().enumerate() {
        let keys = plan_keys(&rp.patch);
        let any = |set: &dyn Fn(&str) -> bool| keys.iter().any(|k| set(k));
        if l.replaced.contains(&ix) || any(&|k| l.legacy_replaced.contains(k)) {
            touch.replaced.insert(ix);
        }
        if l.priorities.contains_key(&ix) || any(&|k| l.legacy_priorities.contains_key(k)) {
            touch.priority.insert(ix);
        }
        if l.anchors.contains_key(&ix) {
            touch.anchors.insert(ix);
        }
        if l.apply_as.contains_key(&ix) || any(&|k| l.legacy_apply_as.contains_key(k)) {
            touch.apply_as.insert(ix);
        }
        if l.skips.contains_key(&ix) {
            touch.skip_hunks.insert(ix);
        }
    }
    touch
}

/// `layer_patches`, plus the order rules and the compat layer.
fn layer_patches_with(
    raw: &RawLoad,
    plan: Option<&LoadOrderPlan>,
    mode: FpMode,
    bindings: &mut Vec<FixBinding>,
) -> Layered {
    let layer = plan.map(|plan| profile_layer(raw, plan, mode, bindings));
    let compat = compat::compat_layer(raw, &profile_touch(raw, layer.as_ref()));
    // A `Base` copy takes the place of the first patch it replaces (D58); the others, and a
    // `Base` copy whose base is gone, run last (D18).
    let mut at_slot: HashMap<usize, Vec<&GluePatchEntry>> = HashMap::new();
    let mut last: Vec<&GluePatchEntry> = Vec::new();
    for (glue, targets) in layer.iter().flat_map(|l| &l.glues) {
        match targets.iter().min() {
            Some(&t) if glue.runs_as_base() => at_slot.entry(t).or_default().push(*glue),
            _ => last.push(*glue),
        }
    }
    let mut out: Vec<LoadedPatch> = Vec::with_capacity(raw.patches.len());
    for (ix, rp) in raw.patches.iter().enumerate() {
        let keys = plan_keys(&rp.patch);
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let patch_id = keys.first().copied().unwrap_or_default();
        let mut patch = rp.patch.clone();
        let mut priority = rp.explicit_priority.unwrap_or(rp.base_priority);
        // Compat overrides first, so the profile layer below wins (D13).
        if let Some(edit) = compat.edits.get(&ix) {
            if edit.disable {
                info!("Disabling patch '{patch_id}' per a compatibility mod");
                patch.set_disabled(true);
            }
            if let Some(p) = edit.priority {
                priority = p;
            }
            let extra = patch.extra_mut();
            if let Some(a) = &edit.anchors {
                let source = AnchorSource::Compat {
                    mod_id: a.mod_id.clone(),
                };
                extra
                    .rungs
                    .extend(a.rungs.iter().map(|r| (r.clone(), source.clone())));
                if !a.sites.is_empty() {
                    extra.sites = a.sites.clone();
                }
                extra.replace_author_anchors |= a.replace;
            }
            if edit.apply_as.is_some() {
                extra.apply_as = edit.apply_as;
            }
            if !edit.skip_hunks.is_empty() {
                extra.skip_hunks = edit.skip_hunks.clone();
                // A skipped edit needs edits mode (D49).
                extra.apply_as = Some(ApplyAs::Edits);
            }
            // Compat stack rules add up; profile rules come next and are stronger (D56).
            extra.stack_rules.extend(edit.stack_rules.iter().cloned());
            extra.changed_by.extend(edit.changed_by.iter().cloned());
        }
        if let Some(layer) = &layer {
            if layer.replaced.contains(&ix)
                || keys.iter().any(|k| layer.legacy_replaced.contains(k))
            {
                info!("Replacing patch '{patch_id}' with profile glue patch from load-order.json");
                for glue in at_slot.get(&ix).into_iter().flatten() {
                    let mod_id = rp.patch.mod_id().unwrap_or(GLUE_MOD);
                    let p = copy_patch(glue, plan, mod_id);
                    out.push((p, glue.priority, rp.path.clone(), rp.vars.clone()));
                }
                continue;
            }
            if layer.disabled.contains(&ix)
                || keys.iter().any(|k| layer.legacy_disabled.contains(k))
            {
                info!("Marking disabled patch '{patch_id}' per load-order.json");
                patch.set_disabled(true);
            }
            // The first key with a priority wins, through a ref or not (legacy key order).
            let legacy = keys
                .iter()
                .enumerate()
                .find_map(|(n, k)| layer.legacy_priorities.get(k).map(|p| (n, *p)));
            let pinned = layer.priorities.get(&ix).copied();
            let best = match (pinned, legacy) {
                (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
                (a, b) => a.or(b),
            };
            if let Some((_, p)) = best {
                priority = p;
            }
            // The first key with an `apply_as` entry wins, through a ref or not, as for
            // priorities (D46).
            let legacy = keys
                .iter()
                .enumerate()
                .find_map(|(n, k)| layer.legacy_apply_as.get(k).map(|a| (n, *a, *k)));
            let pinned = layer.apply_as.get(&ix).copied();
            let best = match (pinned, legacy) {
                (Some(a), Some(b)) => Some(if b.0 < a.0 { b } else { a }),
                (a, b) => a.or(b),
            };
            if let Some((_, a, k)) = best {
                let extra = patch.extra_mut();
                extra.apply_as = Some(a);
                if let Some(id) = plan
                    .and_then(|p| p.fix_of.get(&format!("apply_as:{k}")))
                    .and_then(Lenient::ok)
                {
                    extra.fix_ids.push(id.clone());
                }
            }
            if let Some(skips) = layer.skips.get(&ix) {
                let extra = patch.extra_mut();
                extra.skip_hunks.clear();
                for (skip, fix_id) in skips {
                    HunkSkip::add_to(&mut extra.skip_hunks, skip.clone());
                    if !extra.fix_ids.contains(fix_id) {
                        extra.fix_ids.push(fix_id.clone());
                    }
                }
                // A skipped edit needs edits mode, so the fix adds it (D49).
                extra.apply_as = Some(ApplyAs::Edits);
            }
            for spec in layer.stack_rules.get(&ix).into_iter().flatten() {
                let extra = patch.extra_mut();
                extra.stack_rules.push(spec.clone());
                if let RuleSource::Profile { fix_id } = &spec.source {
                    if !extra.fix_ids.contains(fix_id) {
                        extra.fix_ids.push(fix_id.clone());
                    }
                }
            }
            for fix in layer.anchors.get(&ix).into_iter().flatten() {
                let extra = patch.extra_mut();
                let source = AnchorSource::Profile {
                    fix_id: fix.fix_id.clone(),
                };
                extra
                    .rungs
                    .extend(fix.anchors.iter().map(|r| (r.clone(), source.clone())));
                if !fix.sites.is_empty() {
                    extra.sites = fix.sites.clone();
                }
                extra.replace_author_anchors |= fix.replace_author_anchors;
                extra.fix_ids.push(fix.fix_id.clone());
            }
        }
        // Asset files a choice leaves out are not loaded (D78).
        let excluded = plan
            .zip(patch.mod_id())
            .map_or(&[][..], |(plan, id)| plan.asset_exclusions_of(id));
        if !excluded.is_empty() {
            match loader::without_excluded_assets(&patch, excluded) {
                Some(p) => patch = p,
                None => continue,
            }
        }
        out.push((patch, priority, rp.path.clone(), rp.vars.clone()));
    }

    // Factions: at most one per side, compiled into patches QuarterMaster owns.
    let (active, faction_notes) =
        crate::patch::faction::choose_active(raw.factions.clone(), plan);
    for p in crate::patch::faction::build_faction_patches(&active, plan) {
        out.push((
            p,
            crate::patch::faction::FACTION_PATCH_PRIORITY,
            PathBuf::from("qm/factions"),
            HashMap::new(),
        ));
    }

    // Menu framework: builds synthetic patches QuarterMaster owns to inject QM.Menu and compile active mod configs
    for p in crate::patch::menu::build_menu_patches(&raw.configs) {
        out.push((
            p,
            crate::patch::menu::MENU_PATCH_PRIORITY,
            PathBuf::from("qm/menu"),
            HashMap::new(),
        ));
    }

    // Profile-local glue patches, appended last (D18).
    for glue in last {
        out.push((
            copy_patch(glue, plan, GLUE_MOD),
            glue.priority,
            PathBuf::from("qm/load-order.json"),
            HashMap::new(),
        ));
    }
    let order_rules = layer.map(|l| l.order_rules).unwrap_or_default();
    Layered {
        notes: faction_notes,
        patches: out,
        order_rules,
        compat,
    }
}

/// The patch an adjusted copy runs as, with mod `mod_id` (`qm_glue`, or the base mod for a
/// `Base` copy, D58). It answers to the replaced id (§4.5), carries its fix, and an exact
/// copy gets its literal lines (D59).
fn copy_patch(glue: &GluePatchEntry, plan: Option<&LoadOrderPlan>, mod_id: &str) -> Patch {
    let mut p = glue.patch.clone();
    p.set_metadata(
        mod_id.to_string(),
        glue.id.clone(),
        Some(glue.name.clone()),
    );
    let exact = match &p {
        Patch::Pattern(x) if glue.exact => {
            Some(x.pattern.lines().map(|l| l.trim().to_string()).collect())
        }
        _ => None,
    };
    let fix = plan
        .and_then(|p| p.fix_of.get(&format!("glue:{}", glue.id)))
        .and_then(Lenient::ok)
        .cloned();
    let extra = p.extra_mut();
    extra.replaces = glue.replaces_patch_id.clone();
    extra.adjusted_copy = true;
    extra.exact_lines = exact;
    extra.fix_ids.extend(fix);
    p
}

/// Which mods depend on which (D4): the transitive closure over `requires`, `load_after` and
/// the reverse of `load_before`, with ids resolved through `provides` among the loaded mods,
/// case-insensitively. Mutually dependent pairs are removed: they count as unrelated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModGraph {
    /// Lowercase id or `provides` key -> mod id (an id beats a `provides`; among providers the
    /// first mod in loader order wins).
    names: BTreeMap<String, String>,
    deps: BTreeMap<String, BTreeSet<String>>,
    requires: BTreeMap<String, BTreeSet<String>>,
    mutual: Vec<(String, String)>,
}

impl ModGraph {
    pub fn build(mods: &[ModMeta]) -> ModGraph {
        let mut names: BTreeMap<String, String> = BTreeMap::new();
        for m in mods {
            names.insert(m.id.to_lowercase(), m.id.to_lowercase());
        }
        for m in mods {
            for p in &m.provides {
                names
                    .entry(p.to_lowercase())
                    .or_insert_with(|| m.id.to_lowercase());
            }
        }
        let resolve = |k: &str| names.get(&k.to_lowercase()).cloned();

        let mut direct: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut requires: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for m in mods {
            let id = m.id.to_lowercase();
            let entry = direct.entry(id.clone()).or_default();
            for d in m.requires.iter().chain(&m.load_after) {
                if let Some(d) = resolve(d).filter(|d| *d != id) {
                    entry.insert(d);
                }
            }
            let req: BTreeSet<String> = m.requires.iter().filter_map(|d| resolve(d)).collect();
            requires.insert(id, req);
        }
        for m in mods {
            let id = m.id.to_lowercase();
            for x in &m.load_before {
                if let Some(x) = resolve(x).filter(|x| *x != id) {
                    direct.entry(x).or_default().insert(id.clone());
                }
            }
        }

        // Transitive closure, one DFS per mod.
        let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for start in direct.keys() {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut stack: Vec<&String> = direct.get(start).into_iter().flatten().collect();
            while let Some(d) = stack.pop() {
                if d == start || !seen.insert(d.clone()) {
                    continue;
                }
                stack.extend(direct.get(d).into_iter().flatten());
            }
            deps.insert(start.clone(), seen);
        }

        let mut mutual: Vec<(String, String)> = Vec::new();
        for (a, da) in &deps {
            for b in da {
                if a < b && deps.get(b).is_some_and(|db| db.contains(a)) {
                    mutual.push((a.clone(), b.clone()));
                }
            }
        }
        for (a, b) in &mutual {
            if let Some(d) = deps.get_mut(a) {
                d.remove(b);
            }
            if let Some(d) = deps.get_mut(b) {
                d.remove(a);
            }
        }
        ModGraph {
            names,
            deps,
            requires,
            mutual,
        }
    }

    /// The mod id `name` (an id or a `provides` key) resolves to, lowercase.
    pub(crate) fn resolve(&self, name: &str) -> String {
        let lower = name.to_lowercase();
        self.names.get(&lower).cloned().unwrap_or(lower)
    }

    /// Whether mod `a` depends on mod `b`, directly or not.
    pub fn depends_on(&self, a: &str, b: &str) -> bool {
        let b = self.resolve(b);
        self.deps
            .get(&self.resolve(a))
            .is_some_and(|d| d.contains(&b))
    }

    /// Every mod `a` depends on.
    pub fn deps(&self, a: &str) -> BTreeSet<String> {
        self.deps.get(&self.resolve(a)).cloned().unwrap_or_default()
    }

    /// The loaded mods that `compat`'s `requires` keys name, through `provides`.
    pub fn requires_resolved(&self, compat: &str) -> BTreeSet<String> {
        self.requires
            .get(&self.resolve(compat))
            .cloned()
            .unwrap_or_default()
    }

    /// The pairs that depend on each other and so count as unrelated, sorted.
    pub fn mutual_pairs(&self) -> &[(String, String)] {
        &self.mutual
    }
}

#[cfg(test)]
#[path = "plan/bind_tests.rs"]
mod bind_tests;

#[cfg(test)]
#[path = "plan/copy_tests.rs"]
mod copy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::order::matching_patches;
    use crate::patch::table::PatchTable;
    use std::fs;
    use tempfile::TempDir;

    fn meta(id: &str, f: impl FnOnce(&mut ModMeta)) -> ModMeta {
        let mut m = ModMeta {
            id: id.to_string(),
            folder: id.to_string(),
            manifest_ok: true,
            ..ModMeta::default()
        };
        f(&mut m);
        m
    }

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mod_graph_closure_provides_load_before_and_case() {
        // GMenu requires GCore through its `provides` key, GDumper loads after GMenu, and
        // GCore declares `load_before` on Late.
        let mods = vec![
            meta("gcore", |m| {
                m.provides = set(&["gcore-api"]);
                m.load_before = set(&["Late"]);
            }),
            meta("gmenu", |m| {
                m.requires = set(&["GCore-API"]);
                m.load_after = set(&["gcore-api"]);
            }),
            meta("gdumper", |m| m.load_after = set(&["GMENU"])),
            meta("late", |_| {}),
            meta("loner", |m| m.requires = set(&["absent"])),
        ];
        let g = ModGraph::build(&mods);
        assert!(g.depends_on("gmenu", "gcore"));
        assert!(g.depends_on("GDumper", "GCore"), "transitive and case-insensitive");
        assert!(g.depends_on("gdumper", "gcore-api"), "a provides key names its mod");
        assert!(g.depends_on("late", "gcore"), "reverse of load_before");
        assert!(!g.depends_on("gcore", "gmenu"));
        assert!(!g.depends_on("loner", "absent"), "a mod that is not loaded");
        assert_eq!(g.deps("gdumper"), set(&["gcore", "gmenu"]));
        assert_eq!(g.requires_resolved("GMenu"), set(&["gcore"]));
        assert!(g.requires_resolved("gdumper").is_empty(), "load_after is not requires");
        assert!(g.mutual_pairs().is_empty());
    }

    #[test]
    fn mod_graph_mutual_pairs_are_unrelated() {
        let mods = vec![
            meta("a", |m| m.load_after = set(&["b"])),
            meta("b", |m| m.load_after = set(&["a"])),
            meta("c", |m| m.requires = set(&["a"])),
        ];
        let g = ModGraph::build(&mods);
        assert!(!g.depends_on("a", "b"));
        assert!(!g.depends_on("b", "a"));
        assert!(g.depends_on("c", "a"));
        assert!(g.depends_on("c", "b"), "c still depends on what a depends on");
        assert_eq!(g.mutual_pairs(), [("a".to_string(), "b".to_string())]);

        let raw = RawLoad {
            mods,
            ..RawLoad::default()
        };
        let applied = apply_plan(&raw, &PlanLoad::default());
        assert!(
            applied.report.warnings.iter().any(|w| w.starts_with("stack_cycle: 'a' and 'b'")),
            "{:?}",
            applied.report.warnings
        );
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap_or(dir)).unwrap_or_default();
        fs::write(p, text).unwrap_or_default();
    }

    const ONE: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"x\"\nposition = \"after\"\npayload = \"-- one\"\n";
    const TWO: &str = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"x\"\nposition = \"after\"\npayload = \"-- two\"\n";

    fn ids(patches: &[(Patch, Priority, PathBuf)]) -> Vec<String> {
        patches
            .iter()
            .map(|(p, _, _)| p.id().unwrap_or_default().to_string())
            .collect()
    }

    #[test]
    fn duplicate_fallback_ids_get_a_suffix_and_legacy_entries_match_both() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        write(mods, "m/mod.json", r#"{"id": "m", "name": "M", "version": "1.0.0"}"#);
        write(mods, "m/qm.toml", ONE);
        write(mods, "m/qm/More.toml", TWO);
        write(mods, "m/inject.lua", "");
        let raw = load_raw(mods, None).unwrap();
        let got: Vec<(&str, &str, u32)> = raw
            .patches
            .iter()
            .map(|p| (p.patch.id().unwrap_or_default(), p.source.as_str(), p.ordinal))
            .collect();
        assert_eq!(
            got,
            [
                ("m:main_lua:1", "qm.toml", 1),
                ("m:main_lua:1#qm/more.toml", "qm/more.toml", 2)
            ]
        );
        assert_eq!(
            raw.patches[1].patch.extra().legacy_id.as_deref(),
            Some("m:main_lua:1")
        );
        assert!(
            raw.warnings.iter().any(|w| w.contains("m:main_lua:1#qm/more.toml")),
            "{:?}",
            raw.warnings
        );
        // The name stays the old one, so names in the UI do not change.
        assert_eq!(raw.patches[1].patch.name(), Some("m:main_lua:1"));

        // A legacy entry with the base id still hits both patches.
        let plan = LoadOrderPlan::parse(
            r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["m"],
                "disabled_patches": ["m:main_lua:1"],
                "patch_priorities": {"m:main_lua:1": 77}}"#,
        );
        let applied = apply_plan(&raw, &plan);
        assert_eq!(
            ids(&applied.patches),
            ["m:main_lua:1", "m:main_lua:1#qm/more.toml"]
        );
        assert!(applied.patches.iter().all(|(p, prio, _)| p.is_disabled() && *prio == 77));
        assert!(applied.report.warnings.iter().any(|w| w.contains("#qm/more.toml")));
        // So does a rule name, bare or with the mod.
        assert_eq!(matching_patches("m:main_lua:1", &applied.patches), [0, 1]);
        // The new id names only the duplicate.
        assert_eq!(
            matching_patches("m:main_lua:1#qm/more.toml", &applied.patches),
            [1]
        );

        // A glue that replaces the base id replaces both.
        let plan = LoadOrderPlan::parse(
            r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["m"],
                "glue_patches": [{"id": "glue:x", "name": "G", "replaces_patch_id": "m:main_lua:1",
                  "priority": 1, "patch": {"pattern": {"target": "main.lua", "pattern": "x",
                  "position": "after", "payload": "-- g"}}}]}"#,
        );
        assert_eq!(ids(&apply_plan(&raw, &plan).patches), ["glue:x"]);
    }

    /// Mods `core` (base patch `core:main_lua:1`) and `other`, and a plan that enables
    /// `enabled` with an adjusted copy of the core patch (its base turned off, as
    /// `glue_patch_create` writes it).
    fn glue_world(enabled: &[&str], base_mod_id: Option<&str>) -> (TempDir, PlanLoad) {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        write(mods, "core/qm.toml", ONE);
        write(mods, "other/qm.toml", TWO);
        let mut glue = serde_json::json!({
            "id": "glue:x", "name": "Copy", "replaces_patch_id": "core:main_lua:1",
            "priority": 1, "patch": {"pattern": {"target": "main.lua", "pattern": "x",
            "position": "before", "payload": "-- glue"}}
        });
        if let Some(m) = base_mod_id {
            glue["base_mod_id"] = serde_json::json!(m);
        }
        let plan = serde_json::json!({
            "profile_id": "p", "generated_at": "now", "enabled_mods": enabled,
            "disabled_patches": ["core:main_lua:1"],
            "glue_patches": [glue],
            "fix_of": {"glue:glue:x": "fx_1"},
        });
        let plan = LoadOrderPlan::parse(&plan.to_string());
        assert!(plan.error.is_none() && plan.warnings.is_empty(), "{plan:?}");
        (temp, plan)
    }

    fn main_text(applied: Applied, mods: &Path) -> String {
        let table = PatchTable::from_applied(applied, mods.to_path_buf());
        table.apply_patches_offline("main.lua", "x\n").unwrap().0
    }

    #[test]
    fn glue_is_skipped_while_its_base_mod_is_off() {
        for (base_mod_id, named) in [(None, "core"), (Some("Core"), "Core")] {
            let (temp, plan) = glue_world(&["other"], base_mod_id);
            let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
            let applied = apply_plan(&raw, &plan);
            assert_eq!(ids(&applied.patches), ["other:main_lua:1"], "{base_mod_id:?}");
            assert_eq!(
                applied.report.bindings,
                [FixBinding {
                    fix_id: "fx_1".to_string(),
                    key: "glue:glue:x".to_string(),
                    status: BindingStatus::Waiting {
                        mods: vec![named.to_string()]
                    },
                    detail: format!(
                        "Waiting for {named}: the adjusted copy 'Copy' is skipped while {named} is off"
                    ),
                    new_id: None,
                }]
            );
            assert_eq!(main_text(applied, temp.path()), "x\n-- two\n");
        }

        // The table and the DLL report carry it.
        let (temp, plan) = glue_world(&["other"], None);
        write(temp.path(), "qm/load-order.json", &plan_json(&plan));
        let table = PatchTable::load(temp.path()).unwrap();
        assert_eq!(table.plan_report.bindings.len(), 1);
    }

    /// D60: a base mod that failed to load, was skipped as a requirer or is enabled with no
    /// folder is not on; the copy waits, with the reason, and does not run.
    #[test]
    fn glue_waits_while_its_base_mod_did_not_load() {
        let broken = "[manifest\nversion = ";
        // (case, enabled mods, how the Mods folder is broken, the detail's end)
        type Case = (&'static str, &'static [&'static str], fn(&Path), &'static str);
        let cases: [Case; 3] = [
            (
                "failed",
                &["core", "other"],
                |mods| write(mods, "core/qm.toml", "[manifest\nversion = "),
                "because core failed to load",
            ),
            (
                "requirer",
                &["base", "core", "other"],
                |mods| {
                    write(mods, "base/qm.toml", "[manifest\nversion = ");
                    write(
                        mods,
                        "core/mod.json",
                        r#"{"id": "core", "name": "Core", "version": "1.0.0", "relationships": {"requires": {"base": "*"}}}"#,
                    );
                },
                "because core needs base, which failed to load",
            ),
            (
                "missing",
                &["core", "other"],
                |mods| fs::remove_dir_all(mods.join("core")).unwrap_or_default(),
                "while core is missing",
            ),
        ];
        for (case, enabled, edit, why) in cases {
            for base_mod_id in [None, Some("core")] {
                let (temp, plan) = glue_world(enabled, base_mod_id);
                edit(temp.path());
                let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
                assert!(!raw.mods.iter().any(|m| m.id == "core"), "{case}");
                let applied = apply_plan(&raw, &plan);
                assert_eq!(ids(&applied.patches), ["other:main_lua:1"], "{case}");
                assert_eq!(
                    applied.report.bindings,
                    [FixBinding {
                        fix_id: "fx_1".to_string(),
                        key: "glue:glue:x".to_string(),
                        status: BindingStatus::Waiting {
                            mods: vec!["core".to_string()]
                        },
                        detail: format!(
                            "Waiting for core: the adjusted copy 'Copy' is skipped {why}"
                        ),
                        new_id: None,
                    }],
                    "{case}"
                );
                assert_eq!(main_text(applied, temp.path()), "x\n-- two\n", "{case}");
            }
        }

        // The ref'd branch (a copy made by the ledger) waits the same way.
        let (temp, plan) = glue_world(&["core", "other"], Some("core"));
        write(temp.path(), "core/qm.toml", broken);
        let mut p = plan.plan.clone().unwrap();
        p.refs.insert(
            "core:main_lua:1".to_string(),
            Lenient::Ok(PatchRef {
                id: "core:main_lua:1".to_string(),
                mod_id: "core".to_string(),
                fp: None,
            }),
        );
        let plan = PlanLoad {
            plan: Some(p),
            ..PlanLoad::default()
        };
        let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
        let applied = apply_plan(&raw, &plan);
        assert_eq!(ids(&applied.patches), ["other:main_lua:1"]);
        let glue: Vec<&FixBinding> = applied
            .report
            .bindings
            .iter()
            .filter(|b| b.key == "glue:glue:x")
            .collect();
        assert_eq!(glue.len(), 1, "{:?}", applied.report.bindings);
        assert_eq!(
            glue[0].detail,
            "Waiting for core: the adjusted copy 'Copy' is skipped because core failed to load"
        );

        // The DLL reads the same plan and reports the same binding.
        let (temp, plan) = glue_world(&["core", "other"], None);
        write(temp.path(), "core/qm.toml", broken);
        write(temp.path(), "qm/load-order.json", &plan_json(&plan));
        let table = PatchTable::load(temp.path()).unwrap();
        assert_eq!(table.plan_report.bindings.len(), 1);
        assert!(table.plan_report.bindings[0]
            .detail
            .ends_with("because core failed to load"));
    }

    fn plan_json(plan: &PlanLoad) -> String {
        serde_json::to_string(plan.plan.as_ref().unwrap()).unwrap()
    }

    #[test]
    fn glue_with_its_base_mod_on_gives_todays_output() {
        let (temp, plan) = glue_world(&["core", "other"], Some("core"));
        let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
        let applied = apply_plan(&raw, &plan);
        // The copy replaces its base and runs last, as before.
        assert_eq!(ids(&applied.patches), ["other:main_lua:1", "glue:x"]);
        assert!(applied.report.bindings.is_empty());
        assert_eq!(main_text(applied, temp.path()), "-- glue\nx\n-- two\n");

        // A copy whose base is gone from a mod that loaded keeps running. So does a copy that
        // names no mod (no base_mod_id and a base id not in the fallback form): this is the
        // case D60 leaves, as nothing says which mod it waits for.
        let (temp, plan) = glue_world(&["core", "other"], None);
        let mut p = plan.plan.clone().unwrap();
        let mut a = p.glues().next().cloned().unwrap();
        a.replaces_patch_id = Some("core:main_lua:9".to_string());
        let mut b = a.clone();
        b.id = "glue:y".to_string();
        b.replaces_patch_id = Some("nomod".to_string());
        let mut c = a.clone();
        c.id = "glue:z".to_string();
        c.replaces_patch_id = None;
        // An explicit base id whose prefix is not a mod, gone from an enabled mod.
        let mut d = a.clone();
        d.id = "glue:w".to_string();
        d.replaces_patch_id = Some("pack:halt_fix".to_string());
        p.glue_patches = vec![a.into(), b.into(), c.into(), d.into()];
        let plan = PlanLoad {
            plan: Some(p),
            ..PlanLoad::default()
        };
        let raw = load_raw(temp.path(), plan.plan.as_ref()).unwrap();
        let applied = apply_plan(&raw, &plan);
        assert_eq!(
            ids(&applied.patches),
            ["core:main_lua:1", "other:main_lua:1", "glue:x", "glue:y", "glue:z", "glue:w"]
        );
        assert!(applied.report.bindings.is_empty());
    }

    #[test]
    fn only_a_fallback_id_names_the_base_mod() {
        assert_eq!(fallback_id_mod("core:main_lua:1"), Some("core"));
        assert_eq!(fallback_id_mod("core:main_lua:12#qm/more.toml"), Some("core"));
        assert_eq!(fallback_id_mod("pack:halt_fix"), None);
        assert_eq!(fallback_id_mod("a:b:c"), None);
        assert_eq!(fallback_id_mod("a:b:1:2"), None);
        assert_eq!(fallback_id_mod(":main_lua:1"), None);
        assert_eq!(fallback_id_mod("nomod"), None);
    }

    #[test]
    fn explicit_ids_never_move() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        let explicit = ONE.replace("[patches.pattern]\n", "[patches.pattern]\nid = \"same\"\n");
        write(mods, "m/qm.toml", &explicit);
        write(mods, "m/qm/b.toml", &explicit);
        let raw = load_raw(mods, None).unwrap();
        let got: Vec<&str> = raw.patches.iter().filter_map(|p| p.patch.id()).collect();
        assert_eq!(got, ["same", "same"]);
        assert!(raw.warnings.is_empty(), "{:?}", raw.warnings);
    }

    #[test]
    fn mod_meta_keeps_the_mod_json_relations() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        write(
            mods,
            "Core/mod.json",
            r#"{"id": "GCore", "name": "G", "version": "2.1.0", "relationships": {"provides": ["Core-API"], "load_before": ["Late"]}}"#,
        );
        write(mods, "Core/qm.toml", ONE);
        write(
            mods,
            "Menu/mod.json",
            r#"{"id": "gmenu", "name": "M", "version": "1.0.0", "relationships": {"requires": {"GCore": "*"}}}"#,
        );
        write(mods, "Menu/qm.toml", TWO);
        write(mods, "Plain/qm.toml", TWO);
        write(mods, "Torn/mod.json", "{ nope");
        write(mods, "Torn/qm.toml", TWO);
        let raw = load_raw(mods, None).unwrap();
        let core = &raw.mods[0];
        assert_eq!(core.id, "gcore");
        assert_eq!(core.folder, "Core");
        assert_eq!(core.version.as_deref(), Some("2.1.0"));
        assert_eq!(core.provides, set(&["core-api"]));
        assert_eq!(core.load_before, set(&["late"]));
        let menu = &raw.mods[1];
        assert_eq!(menu.requires, set(&["gcore"]));
        assert_eq!(menu.load_after, set(&["gcore"]), "requires are in load_after");
        assert_eq!(menu.loader_index, 1);
        assert!(!raw.mods[2].manifest_ok);
        assert_eq!(raw.mods[2].id, "plain");
        assert!(!raw.mods[3].manifest_ok);
        assert!(raw.patches.iter().all(|p| raw.mods.get(p.mod_ix as usize).is_some()));
        // Only the unreadable mod.json is reported, not the missing one.
        assert_eq!(raw.warnings.len(), 1, "{:?}", raw.warnings);
        assert!(raw.warnings[0].starts_with("Torn: mod.json unreadable"));

        // The table keeps them, and its prepared graph reads them.
        let table = PatchTable::load(mods).unwrap();
        // `load` resolves the Mods folder, so only the base paths differ.
        let unbased = |v: &[ModMeta]| -> Vec<ModMeta> {
            v.iter()
                .map(|m| ModMeta {
                    base: PathBuf::new(),
                    ..m.clone()
                })
                .collect()
        };
        assert_eq!(unbased(&table.mods), unbased(&raw.mods));
        assert_eq!(raw.mods[0].base, mods.join("Core"));
        assert!(table.prepare().graph().depends_on("gmenu", "core-api"));
    }

    #[test]
    fn from_applied_is_what_load_builds() {
        let temp = TempDir::new().unwrap();
        let mods = temp.path();
        write(mods, "a/qm.toml", ONE);
        write(mods, "b/qm.toml", TWO);
        write(
            mods,
            "qm/load-order.json",
            r#"{"profile_id": "p", "generated_at": "now", "enabled_mods": ["a", "b"],
                "disabled_patches": ["a:main_lua:1"], "stacking": "dependencies",
                "patch_order": [{"patch": "b:main_lua:1", "before": ["a:main_lua:1"]}]}"#,
        );
        let (table, raw) = PatchTable::load_with_raw(mods).unwrap();
        let raw = raw.expect("read");
        let plan = LoadOrderPlan::read(&table.mod_dir);
        let again = PatchTable::from_applied(apply_plan(&raw, &plan), table.mod_dir.clone());
        assert_eq!(again.stamp(), table.stamp());
        assert_eq!(again.stack_mode, StackMode::Dependencies);
        assert_eq!(again.order_rules.len(), 1);
        assert_eq!(ids(&again.patches), ids(&table.patches));

        // A plan error reads no mods.
        write(mods, "qm/load-order.json", "{");
        let (table, raw) = PatchTable::load_with_raw(mods).unwrap();
        assert!(raw.is_none());
        assert!(table.patches.is_empty());
        assert!(table.plan_report.plan_error.is_some());
    }
}
