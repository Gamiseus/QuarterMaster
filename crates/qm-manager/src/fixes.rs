//! The profile's fix ledger (Phase C §3.11, §7.3-7.5). Every disable, order rule, priority,
//! adjusted copy and Tier 1 load-order change is a `ProfileFix` with fingerprints, provenance
//! and an undo that survives restarts. Once a profile is migrated (`fixes_version == 1`) the
//! legacy profile fields are the compiled view of the ledger; `compile_plan` writes them, plus
//! the refs that bind each entry to one patch. `mod_priorities` and `user_load_after` stay
//! profile fields: a `ModOrder` fix only records what Tier 1 wrote there (D36).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use qm_core::handshake::{
    AnchorFix, GluePatchEntry, LaunchConfig, LoadOrderPlan, RunAs, SkipHunkFix, StackRuleFix,
};
use qm_core::lenient::Lenient;
use qm_core::patch::anchor::{Rung, SiteKey};
use qm_core::patch::edits::{self, ApplyAs, HunkKey};
use qm_core::patch::fingerprint::{hash32, PatchRef, PatchSnapshot};
use qm_core::patch::linemap::Side;
use qm_core::patch::order::{self, OrderRule};
use qm_core::patch::plan::{self, Applied, Fingerprints, FpMode, RawLoad};
use qm_core::patch::table::PatchTable;
use qm_core::patch::Priority;

use crate::profiles::ModProfile;

pub mod game_stamp;
mod zip_names;
pub use game_stamp::{GameFileDrift, GAME_CHANGED};
pub use zip_names::{rename_zip_stems, zip_stems, ZipRename};

/// The ledger version a migrated profile has.
pub const FIXES_VERSION: u32 = 1;

/// The one-time notice that says earlier changes were imported.
pub const NOTICE_FIXES_IMPORT: &str = "fixes_import_v1";

/// The most journal entries a profile keeps.
pub const JOURNAL_CAP: usize = 50;

/// One fix in the profile's ledger.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFix {
    /// "fx_" and 12 hex chars.
    pub id: String,
    pub label: String,
    pub change: FixChange,
    pub provenance: Provenance,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub undo: UndoRecord,
    /// The text of A and of every patch the fix names, when it was created or accepted (D41),
    /// keyed by patch id. Kept in profile.json only, never in load-order.json.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub snapshots: BTreeMap<String, PatchSnapshot>,
    /// The SHA-256 of each game file the fix's patches target, keyed by the game's path, when
    /// it was created or accepted (D74). In profile.json only. Empty for fixes from before.
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub vanilla_hashes: BTreeMap<String, String>,
    /// Made or accepted in this write: the save stamps `vanilla_hashes` (D74). Never stored.
    #[serde(skip)]
    #[cfg_attr(feature = "specta", specta(skip))]
    pub fresh: bool,
    /// Made or accepted while the game files could not be read: stamped on a later save
    /// once they can. Until then `vanilla_hashes` is not compared (D74).
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[cfg_attr(feature = "specta", specta(skip))]
    pub stamp_pending: bool,
}

/// What a fix changes.
// Stored data: the copy's patch is inline, as in `glue_patches`.
#[allow(clippy::large_enum_variant)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FixChange {
    Disable {
        patch: PatchRef,
    },
    Order {
        rule: OrderRule,
        /// Refs for the names in the rule that named one patch when the fix was made.
        #[serde(default)]
        refs: Vec<PatchRef>,
    },
    Priority {
        patch: PatchRef,
        priority: Priority,
    },
    Anchor {
        patch: PatchRef,
        anchors: Vec<Rung>,
        #[serde(default)]
        sites: Vec<SiteKey>,
        #[serde(default)]
        replace_author_anchors: bool,
    },
    /// A legacy glue patch (D18).
    AdjustedCopy {
        glue: GluePatchEntry,
        /// The patch the copy replaces.
        #[serde(default)]
        base: Option<PatchRef>,
        /// The copy also turns its base off (`glue_patch_create`).
        #[serde(default)]
        disables_base: bool,
    },
    /// How an `at` patch runs: as separate edits or as one replacement (D46).
    ApplyAs {
        patch: PatchRef,
        mode: ApplyAs,
    },
    /// One edit of an `at` patch that does not run, so another patch's change to the same
    /// lines stays (D49). The patch runs as separate edits while the fix runs.
    SkipHunk {
        patch: PatchRef,
        hunk: HunkKey,
    },
    /// At the game's line `site`, on `side`, put `patch`'s block below `below`'s block, or
    /// above `above`'s (D55). It acts in every stacking mode.
    StackRule {
        patch: PatchRef,
        site: SiteKey,
        side: Side,
        #[serde(default)]
        below: Option<PatchRef>,
        #[serde(default)]
        above: Option<PatchRef>,
    },
    /// Who wins an asset group or slot (Phase E D76, D78): a mod id, `faction:<id>` or
    /// `vanilla`. `excludes` is what that leaves out, per source, as the asset ledger worked
    /// it out when the fix was made or last refreshed.
    AssetChoice {
        key: String,
        winner: String,
        #[serde(default)]
        excludes: BTreeMap<String, Vec<String>>,
    },
    /// What Tier 1 "Load X before Y" wrote (D36). Not compiled into the plan.
    ModOrder {
        move_mod_id: String,
        before_mod_id: String,
        /// The priority it gave `move_mod_id`, if any.
        #[serde(default)]
        priority: Option<i32>,
    },
}

/// Where a fix came from.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub created_at: String,
    pub source: FixSource,
    #[serde(default)]
    pub conflict_id: Option<String>,
    #[serde(default)]
    pub conflict_kind: Option<String>,
    #[serde(default)]
    pub option_key: Option<String>,
    /// `(mod id, version)` of the mods the fix names.
    #[serde(default)]
    pub mods: Vec<(String, Option<String>)>,
    #[serde(default)]
    pub approved: Option<ApprovedImpact>,
    #[serde(default)]
    pub history: Vec<FixEvent>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixSource {
    Resolver,
    OrderView,
    PatchToggle,
    Migrated,
    Suggestion,
    AutoFollow,
    CompatExport,
}

/// What the preview showed when the user applied the fix.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedImpact {
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub patch_changes: u32,
    #[serde(default)]
    pub stack_changes: u32,
    #[serde(default)]
    pub resolved: Vec<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixEvent {
    pub at: String,
    pub what: FixEventKind,
    #[serde(default)]
    pub detail: String,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixEventKind {
    Created,
    Pinned,
    Rebound,
    Accepted,
    Paused,
    Resumed,
}

/// How undo reverses a fix (§7.5).
// Stored data, like `FixChange`: a restored skip carries its patch and edit inline.
#[allow(clippy::large_enum_variant)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoRecord {
    #[default]
    Remove,
    /// Put back the order rule this fix replaced.
    RestoreOrder(Option<OrderRule>),
    /// Put back the priority this fix replaced.
    RestorePriority(Option<Priority>),
    /// Remove the copy, keep the disable the user had made before it (if it is still there).
    KeepDisable,
    /// Put back the Anchor fix this fix replaced on the same patch.
    RestoreAnchor {
        label: String,
        anchors: Vec<Rung>,
        #[serde(default)]
        sites: Vec<SiteKey>,
        #[serde(default)]
        replace_author_anchors: bool,
    },
    /// D36: restore `mod_priorities[move]` to `prev_priority` if it still equals the fix's
    /// priority, and remove the `user_load_after` pair if this fix added it and it is still
    /// there. Anything else changed: ask first.
    RestoreModOrder {
        prev_priority: Option<i32>,
        added_load_after: bool,
    },
    /// Put back the ApplyAs fix this fix replaced on the same patch.
    RestoreApplyAs { label: String, mode: ApplyAs },
    /// Put back the AssetChoice fix this fix replaced on the same group or slot.
    RestoreAssetChoice {
        label: String,
        winner: String,
        #[serde(default)]
        excludes: BTreeMap<String, Vec<String>>,
    },
    /// Put back the SkipHunk fix this fix replaced (the same edit, or the other patch's
    /// choice on the same lines).
    RestoreHunk {
        label: String,
        patch: PatchRef,
        hunk: HunkKey,
        /// The other SkipHunk fixes this fix replaced, each a `RestoreHunk` (the same edit
        /// and the other patch's choice can both go). Undo puts every one back.
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        #[cfg_attr(feature = "specta", specta(optional))]
        also: Vec<UndoRecord>,
    },
    /// Put back the StackRule fix this fix replaced on the same line and side: on the same
    /// patch, or on `patch`, the other block, when that rule named this patch.
    RestoreStack {
        label: String,
        #[serde(default)]
        below: Option<PatchRef>,
        #[serde(default)]
        above: Option<PatchRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        patch: Option<PatchRef>,
        /// The other StackRule fixes this fix replaced, each a `RestoreStack` (the same-slot
        /// rule and the other block's rule can both go). Undo puts every one back.
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        #[cfg_attr(feature = "specta", specta(optional))]
        also: Vec<UndoRecord>,
    },
}

/// A batch operation that can be undone as one step.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub at: String,
    /// "migrated" | "compat_export".
    pub op: String,
    pub label: String,
    /// The fixes the operation took out. One of a kind this version does not know
    /// round-trips as it is.
    #[serde(default)]
    pub removed: Vec<Lenient<ProfileFix>>,
    /// Ids of the fixes the operation added.
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub mods_enabled: Vec<String>,
}

impl ProfileFix {
    /// "disable" | "order" | "priority" | "anchor" | "adjusted_copy" | "mod_order" |
    /// "apply_as" | "skip_hunk" | "stack_rule" | "asset_choice".
    pub fn kind(&self) -> &'static str {
        match &self.change {
            FixChange::Disable { .. } => "disable",
            FixChange::Order { .. } => "order",
            FixChange::Priority { .. } => "priority",
            FixChange::Anchor { .. } => "anchor",
            FixChange::AdjustedCopy { .. } => "adjusted_copy",
            FixChange::ModOrder { .. } => "mod_order",
            FixChange::AssetChoice { .. } => "asset_choice",
            FixChange::ApplyAs { .. } => "apply_as",
            FixChange::SkipHunk { .. } => "skip_hunk",
            FixChange::StackRule { .. } => "stack_rule",
        }
    }

    /// The patch the fix is about: the disabled, prioritized or anchored patch, the rule's
    /// patch, or the patch a copy replaces (else the copy itself). None for a `ModOrder`.
    pub fn subject(&self) -> Option<&str> {
        match &self.change {
            FixChange::Disable { patch }
            | FixChange::Priority { patch, .. }
            | FixChange::Anchor { patch, .. }
            | FixChange::ApplyAs { patch, .. }
            | FixChange::SkipHunk { patch, .. }
            | FixChange::StackRule { patch, .. } => Some(&patch.id),
            FixChange::Order { rule, .. } => Some(&rule.patch),
            FixChange::AdjustedCopy { glue, .. } => {
                Some(glue.replaces_patch_id.as_deref().unwrap_or(&glue.id))
            }
            FixChange::ModOrder { .. } | FixChange::AssetChoice { .. } => None,
        }
    }

    /// Every patch ref the fix holds.
    pub fn refs(&self) -> Vec<&PatchRef> {
        match &self.change {
            FixChange::Disable { patch }
            | FixChange::Priority { patch, .. }
            | FixChange::Anchor { patch, .. }
            | FixChange::ApplyAs { patch, .. }
            | FixChange::SkipHunk { patch, .. } => vec![patch],
            FixChange::Order { refs, .. } => refs.iter().collect(),
            FixChange::AdjustedCopy { base, .. } => base.iter().collect(),
            FixChange::StackRule {
                patch,
                below,
                above,
                ..
            } => std::iter::once(patch).chain(below).chain(above).collect(),
            FixChange::ModOrder { .. } | FixChange::AssetChoice { .. } => Vec::new(),
        }
    }

    pub fn refs_mut(&mut self) -> Vec<&mut PatchRef> {
        match &mut self.change {
            FixChange::Disable { patch }
            | FixChange::Priority { patch, .. }
            | FixChange::Anchor { patch, .. }
            | FixChange::ApplyAs { patch, .. }
            | FixChange::SkipHunk { patch, .. } => vec![patch],
            FixChange::Order { refs, .. } => refs.iter_mut().collect(),
            FixChange::AdjustedCopy { base, .. } => base.iter_mut().collect(),
            FixChange::StackRule {
                patch,
                below,
                above,
                ..
            } => std::iter::once(patch).chain(below).chain(above).collect(),
            FixChange::ModOrder { .. } | FixChange::AssetChoice { .. } => Vec::new(),
        }
    }

    /// Adds a history event.
    pub fn log(&mut self, what: FixEventKind, detail: impl Into<String>) {
        self.provenance.history.push(FixEvent {
            at: now(),
            what,
            detail: detail.into(),
        });
    }
}

impl Provenance {
    /// A provenance made now, with a `created` event.
    pub fn new(source: FixSource) -> Self {
        Provenance {
            created_at: now(),
            source,
            conflict_id: None,
            conflict_kind: None,
            option_key: None,
            mods: Vec::new(),
            approved: None,
            history: vec![FixEvent {
                at: now(),
                what: FixEventKind::Created,
                detail: String::new(),
            }],
        }
    }
}

fn now() -> String {
    jiff::Zoned::now().to_string()
}

/// A new fix id: "fx_" and 12 hex chars.
pub fn new_fix_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let seed = format!(
        "{}\0{}\0{:?}",
        jiff::Timestamp::now(),
        N.fetch_add(1, Ordering::Relaxed),
        std::thread::current().id()
    );
    let hex: String = hash32(&seed).chars().take(12).collect();
    format!("fx_{hex}")
}

/// The valid fixes of a profile, in ledger order.
pub fn fixes(p: &ModProfile) -> impl Iterator<Item = &ProfileFix> {
    p.fixes.iter().filter_map(Lenient::ok)
}

fn fixes_mut(p: &mut ModProfile) -> impl Iterator<Item = &mut ProfileFix> {
    p.fixes.iter_mut().filter_map(|f| match f {
        Lenient::Ok(f) => Some(f),
        Lenient::Invalid { .. } => None,
    })
}

/// The fix with this id.
pub fn find(p: &ModProfile, id: &str) -> Option<ProfileFix> {
    fixes(p).find(|f| f.id == id).cloned()
}

/// The fix with this id, to change it in place.
pub fn find_mut<'a>(p: &'a mut ModProfile, id: &str) -> Option<&'a mut ProfileFix> {
    fixes_mut(p).find(|f| f.id == id)
}

/// Removes the fix with this id; the removed fix, if any.
pub fn remove(p: &mut ModProfile, id: &str) -> Option<ProfileFix> {
    let at = p
        .fixes
        .iter()
        .position(|f| f.ok().is_some_and(|f| f.id == id))?;
    p.fixes.remove(at).into_ok()
}

/// Appends a fix.
pub fn push(p: &mut ModProfile, fix: ProfileFix) {
    p.fixes.push(Lenient::Ok(fix));
}

/// What only one running fix may hold: the patch of an order rule, a priority, an anchor or
/// an ApplyAs, one edit of a patch, a patch's block at one line and side, or the id of an
/// adjusted copy. None for a disable or a load order change (they add up).
fn slot(f: &ProfileFix) -> Option<(&'static str, String)> {
    match &f.change {
        FixChange::Order { rule, .. } => Some(("order", rule.patch.clone())),
        FixChange::Priority { patch, .. } => Some(("priority", patch.id.clone())),
        FixChange::Anchor { patch, .. } => Some(("anchor", patch.id.clone())),
        FixChange::ApplyAs { patch, .. } => Some(("apply_as", patch.id.clone())),
        FixChange::SkipHunk { patch, hunk } => {
            Some(("hunk", format!("{}#{}", patch.id, hunk.ordinal)))
        }
        FixChange::AdjustedCopy { glue, .. } => Some(("adjusted_copy", glue.id.clone())),
        FixChange::StackRule {
            patch, site, side, ..
        } => Some(("stack", stack_slot_key(&patch.id, site, *side))),
        FixChange::AssetChoice { key, .. } => Some(("asset", key.clone())),
        FixChange::Disable { .. } | FixChange::ModOrder { .. } => None,
    }
}

/// The slot key of a stack rule: the patch, the line and the side.
fn stack_slot_key(patch: &str, site: &SiteKey, side: Side) -> String {
    let side = match side {
        Side::Before => "before",
        Side::After => "after",
    };
    format!(
        "{patch}#{}#{}#{side}",
        site.context,
        site.ordinal.unwrap_or(0)
    )
}

/// The running fixes (other than `fix`) that hold what `fix` would hold if it ran: the same
/// kind on the same patch, or a copy with the same id. Resuming or restoring `fix` next to them
/// would override them, or leave two rules for one patch. A per-edit choice also clashes with
/// the opposite choice on the same lines (Keep A against Keep B), and a stack rule with the
/// other block's rule naming it at the same line and side.
pub fn running_clashes(p: &ModProfile, fix: &ProfileFix) -> Vec<String> {
    let Some(want) = slot(fix) else {
        return Vec::new();
    };
    fixes(p)
        .filter(|f| {
            !f.paused
                && f.id != fix.id
                && (slot(f).as_ref() == Some(&want) || opposite(fix, f))
        })
        .map(|f| f.id.clone())
        .collect()
}

/// Two choices that exclude each other: per-edit choices where each skips its own patch's edit
/// to keep the other patch's change on the same lines, or stack rules on the two blocks that
/// name each other at the same line and side.
fn opposite(a: &ProfileFix, b: &ProfileFix) -> bool {
    match (&a.change, &b.change) {
        (
            FixChange::StackRule {
                patch: pa,
                site: sa,
                side: da,
                below: ba,
                above: aa,
            },
            FixChange::StackRule {
                patch: pb,
                site: sb,
                side: db,
                below: bb,
                above: ab,
            },
        ) => {
            let names = |b: &Option<PatchRef>, a: &Option<PatchRef>, id: &str| {
                [b, a].into_iter().flatten().any(|r| r.id == id)
            };
            sa == sb && da == db && names(ba, aa, &pb.id) && names(bb, ab, &pa.id)
        }
        (
            FixChange::SkipHunk { patch: pa, hunk: ha },
            FixChange::SkipHunk { patch: pb, hunk: hb },
        ) => {
            ha.keep.as_deref() == Some(pb.id.as_str())
                && hb.keep.as_deref() == Some(pa.id.as_str())
                && same_lines(&ha.vanilla, &hb.vanilla)
        }
        _ => false,
    }
}

/// Whether an adjusted copy uses `glue_id`: in the legacy field, or in any copy fix, paused
/// ones included (a new copy must not take a paused copy's id).
pub fn glue_id_taken(p: &ModProfile, glue_id: &str) -> bool {
    p.glue_patches.iter().any(|g| g.id == glue_id)
        || fixes(p).any(
            |f| matches!(&f.change, FixChange::AdjustedCopy { glue, .. } if glue.id == glue_id),
        )
}

/// Whether two adjusted copies are the same entry: same id and same content.
pub fn same_glue(a: &GluePatchEntry, b: &GluePatchEntry) -> bool {
    a.id == b.id && serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

/// Adds a journal entry, keeping the newest `JOURNAL_CAP`.
pub fn push_journal(p: &mut ModProfile, entry: JournalEntry) {
    p.fix_journal.push(Lenient::Ok(entry));
    let extra = p.fix_journal.len().saturating_sub(JOURNAL_CAP);
    p.fix_journal.drain(..extra);
}

/// Whether the profile has any legacy plan entry or user load-after pair.
pub fn has_legacy_entries(p: &ModProfile) -> bool {
    !p.disabled_patches.is_empty()
        || !p.patch_priorities.is_empty()
        || !p.glue_patches.is_empty()
        || !p.patch_order.is_empty()
        || p.user_load_after.values().any(|v| !v.is_empty())
}

// ---- The compiled view ----

/// The legacy fields the ledger compiles to.
#[derive(Debug, Clone, Default)]
pub struct LegacyFields {
    pub disabled_patches: Vec<String>,
    pub patch_priorities: HashMap<String, i32>,
    pub glue_patches: Vec<GluePatchEntry>,
    pub patch_order: Vec<OrderRule>,
}

/// The legacy fields of the fixes that are not paused, in ledger order.
pub fn ledger_fields(p: &ModProfile) -> LegacyFields {
    let mut out = LegacyFields::default();
    let disable = |id: &str, out: &mut LegacyFields| {
        if !out.disabled_patches.iter().any(|d| d == id) {
            out.disabled_patches.push(id.to_string());
        }
    };
    for f in fixes(p).filter(|f| !f.paused) {
        match &f.change {
            FixChange::Disable { patch } => disable(&patch.id, &mut out),
            FixChange::Priority { patch, priority } => {
                out.patch_priorities.insert(patch.id.clone(), *priority);
            }
            FixChange::AdjustedCopy {
                glue,
                disables_base,
                ..
            } => {
                out.glue_patches.push(glue.clone());
                if *disables_base {
                    if let Some(base) = &glue.replaces_patch_id {
                        disable(base, &mut out);
                    }
                }
            }
            FixChange::Order { rule, .. } => {
                out.patch_order.push(rule.clone());
            }
            FixChange::Anchor { .. }
            | FixChange::ApplyAs { .. }
            | FixChange::SkipHunk { .. }
            | FixChange::StackRule { .. }
            | FixChange::AssetChoice { .. }
            | FixChange::ModOrder { .. } => {}
        }
    }
    out
}

/// Rewrites the legacy fields from the ledger (a migrated profile only).
pub fn sync_legacy(p: &mut ModProfile) {
    if p.fixes_version < FIXES_VERSION {
        return;
    }
    let f = ledger_fields(p);
    p.disabled_patches = f.disabled_patches;
    p.patch_priorities = f.patch_priorities;
    p.glue_patches = f.glue_patches;
    p.patch_order = f.patch_order;
}

/// Pauses fixes `ids` and takes out only the legacy entries those fixes gave. Other entries,
/// hand edits included, stay as they are, so the plan differs from the profile's own in the
/// paused fixes only (a trial run, D68).
pub fn pause_in_place(p: &mut ModProfile, ids: &[String]) {
    let before = ledger_fields(p);
    for id in ids {
        if let Some(f) = find_mut(p, id) {
            f.paused = true;
        }
    }
    if p.fixes_version < FIXES_VERSION {
        return;
    }
    let after = ledger_fields(p);
    p.disabled_patches.retain(|d| {
        !before.disabled_patches.contains(d) || after.disabled_patches.contains(d)
    });
    for (k, v) in &before.patch_priorities {
        let now = after.patch_priorities.get(k);
        if now != Some(v) && p.patch_priorities.get(k) == Some(v) {
            match now {
                Some(w) => {
                    p.patch_priorities.insert(k.clone(), *w);
                }
                None => {
                    p.patch_priorities.remove(k);
                }
            }
        }
    }
    p.glue_patches.retain(|g| {
        !before.glue_patches.iter().any(|b| same_glue(b, g))
            || after.glue_patches.iter().any(|a| same_glue(a, g))
    });
    p.patch_order
        .retain(|r| !before.patch_order.contains(r) || after.patch_order.contains(r));
}

/// Pauses the fixes whose entry is gone from the legacy fields (taken out by hand or by the
/// old launcher), so the next `sync_legacy` does not put it back. A copy whose base was turned
/// back on keeps running but no longer turns it off. Returns how many fixes changed.
pub fn detach_missing(p: &mut ModProfile) -> usize {
    if p.fixes_version < FIXES_VERSION {
        return 0;
    }
    let disabled: HashSet<String> = p.disabled_patches.iter().cloned().collect();
    let prios = p.patch_priorities.clone();
    let glues = p.glue_patches.clone();
    let rules = p.patch_order.clone();
    let mut n = 0;
    for f in fixes_mut(p).filter(|f| !f.paused) {
        let gone = match &mut f.change {
            FixChange::Disable { patch } => !disabled.contains(&patch.id),
            FixChange::Priority { patch, priority } => prios.get(&patch.id) != Some(priority),
            FixChange::Order { rule, .. } => !rules.contains(rule),
            FixChange::AdjustedCopy {
                glue,
                disables_base,
                ..
            } => {
                // A copy edited by hand under the same id counts as gone: `reconcile` adopts
                // the edited one.
                if !glues.iter().any(|g| same_glue(g, glue)) {
                    true
                } else {
                    let base_on = glue
                        .replaces_patch_id
                        .as_ref()
                        .is_some_and(|b| !disabled.contains(b));
                    if *disables_base && base_on {
                        *disables_base = false;
                        n += 1;
                    }
                    false
                }
            }
            FixChange::Anchor { .. }
            | FixChange::ApplyAs { .. }
            | FixChange::SkipHunk { .. }
            | FixChange::StackRule { .. }
            | FixChange::AssetChoice { .. }
            | FixChange::ModOrder { .. } => false,
        };
        if gone {
            f.paused = true;
            f.log(FixEventKind::Paused, "changed outside QuarterMaster");
            n += 1;
        }
    }
    n
}

/// Brings the ledger in line with hand edits of the legacy fields: fixes whose entry was taken
/// out are paused (`detach_missing`), entries no fix accounts for are adopted as fixes, so the
/// next `sync_legacy` keeps the edit. Returns how many were adopted.
pub fn reconcile(p: &mut ModProfile, raw: Option<&RawLoad>) -> usize {
    if p.fixes_version < FIXES_VERSION {
        return 0;
    }
    detach_missing(p);
    let have = ledger_fields(p);
    let orphan = ModProfile {
        disabled_patches: p
            .disabled_patches
            .iter()
            .filter(|d| !have.disabled_patches.contains(d))
            .cloned()
            .collect(),
        patch_priorities: p
            .patch_priorities
            .iter()
            .filter(|(k, v)| have.patch_priorities.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), *v))
            .collect(),
        glue_patches: p
            .glue_patches
            .iter()
            .filter(|g| !have.glue_patches.iter().any(|h| same_glue(h, g)))
            .cloned()
            .collect(),
        patch_order: p
            .patch_order
            .iter()
            .filter(|r| !have.patch_order.contains(r))
            .cloned()
            .collect(),
        user_load_after: HashMap::new(),
        ..p.clone()
    };
    let adopted = from_legacy(&orphan, raw, &mut |_, _| new_fix_id(), FixSource::Migrated);
    let n = adopted.len();
    for f in adopted {
        push(p, f);
    }
    n
}

/// The plan for `profile`, with the resolved mod order and priorities (§7.3). The legacy
/// fields come from the profile; a migrated profile also gets a ref for each entry whose fix
/// pinned a fingerprint, the `fix_of` map and its anchor fixes. Paused fixes are not
/// compiled. Snapshots never go into the plan (D41). Pure apart from the timestamp.
pub fn compile_plan(
    profile: &ModProfile,
    enabled_mods: Vec<String>,
    mod_priorities: HashMap<String, Priority>,
) -> LoadOrderPlan {
    let mut plan = LoadOrderPlan {
        // 4 only when the plan has a Phase E field; set at the end.
        schema_version: qm_core::handshake::PLAN_SCHEMA_PHASE_D,
        generator: Some(format!("quartermaster {}", qm_core::QM_VERSION)),
        profile_id: profile.id.clone(),
        generated_at: now(),
        enabled_mods,
        mod_priorities: mod_priorities
            .into_iter()
            .map(|(k, v)| (k, Lenient::Ok(v)))
            .collect(),
        patch_priorities: profile
            .patch_priorities
            .iter()
            .map(|(k, v)| (k.clone(), Lenient::Ok(*v)))
            .collect(),
        disabled_patches: profile
            .disabled_patches
            .iter()
            .cloned()
            .map(Lenient::Ok)
            .collect(),
        glue_patches: profile
            .glue_patches
            .iter()
            .cloned()
            .map(Lenient::Ok)
            .collect(),
        patch_order: profile
            .patch_order
            .iter()
            .cloned()
            .map(Lenient::Ok)
            .collect(),
        // Written explicitly: a profile that has not seen the stacking notice gets legacy (D3).
        stacking: Some(Lenient::Ok(profile.stack_mode())),
        // Only when on: a profile with no answer keeps the older rules and its plan (D51).
        line_rules_v2: profile.line_rules().then_some(Lenient::Ok(true)),
        // Only when picked (D82): without a pick the plan is as before.
        factions: profile
            .faction_picks
            .iter()
            .map(|(side, id)| (side.clone(), Lenient::Ok(id.clone())))
            .collect(),
        ..LoadOrderPlan::default()
    };
    plan.schema_version = plan.schema_for_fields();
    if profile.fixes_version < FIXES_VERSION {
        return plan;
    }
    let disabled: HashSet<&str> = profile
        .disabled_patches
        .iter()
        .map(String::as_str)
        .collect();
    let add_ref = |plan: &mut LoadOrderPlan, r: &PatchRef| {
        if r.fp.is_some() {
            plan.refs
                .entry(r.id.clone())
                .or_insert_with(|| Lenient::Ok(r.clone()));
        }
    };
    let fix_of = |plan: &mut LoadOrderPlan, key: String, id: &str| {
        plan.fix_of
            .entry(key)
            .or_insert_with(|| Lenient::Ok(id.to_string()));
    };
    for f in fixes(profile).filter(|f| !f.paused) {
        match &f.change {
            FixChange::Disable { patch } if disabled.contains(patch.id.as_str()) => {
                add_ref(&mut plan, patch);
                fix_of(&mut plan, format!("disable:{}", patch.id), &f.id);
            }
            FixChange::Priority { patch, priority }
                if profile.patch_priorities.get(&patch.id) == Some(priority) =>
            {
                add_ref(&mut plan, patch);
                fix_of(&mut plan, format!("priority:{}", patch.id), &f.id);
            }
            FixChange::AdjustedCopy {
                glue,
                base,
                disables_base,
            } if profile.glue_patches.iter().any(|g| g.id == glue.id) => {
                if let (Some(b), Some(rid)) = (base, &glue.replaces_patch_id) {
                    if b.id == *rid {
                        add_ref(&mut plan, b);
                    }
                }
                fix_of(&mut plan, format!("glue:{}", glue.id), &f.id);
                if let (true, Some(rid)) = (*disables_base, &glue.replaces_patch_id) {
                    if disabled.contains(rid.as_str()) {
                        fix_of(&mut plan, format!("disable:{rid}"), &f.id);
                    }
                }
            }
            FixChange::Order { rule, refs } if profile.patch_order.contains(rule) => {
                for r in refs {
                    add_ref(&mut plan, r);
                }
                fix_of(&mut plan, format!("order:{}", rule.patch), &f.id);
            }
            FixChange::Anchor {
                patch,
                anchors,
                sites,
                replace_author_anchors,
            } => {
                plan.anchor_fixes.push(Lenient::Ok(AnchorFix {
                    fix_id: f.id.clone(),
                    patch: patch.clone(),
                    anchors: anchors.clone(),
                    sites: sites.clone(),
                    replace_author_anchors: *replace_author_anchors,
                }));
                fix_of(&mut plan, format!("anchor:{}", patch.id), &f.id);
            }
            FixChange::ApplyAs { patch, mode } => {
                add_ref(&mut plan, patch);
                plan.apply_as
                    .entry(patch.id.clone())
                    .or_insert(Lenient::Ok(*mode));
                fix_of(&mut plan, format!("apply_as:{}", patch.id), &f.id);
            }
            // The first running choice for a key wins; what the choices leave out adds up.
            FixChange::AssetChoice {
                key,
                winner,
                excludes,
            } => {
                plan.asset_choices
                    .entry(key.clone())
                    .or_insert(Lenient::Ok(winner.clone()));
                for (source, files) in excludes {
                    let entry = plan
                        .asset_exclusions
                        .entry(source.clone())
                        .or_insert_with(|| Lenient::Ok(Vec::new()));
                    if let Lenient::Ok(list) = entry {
                        for f in files {
                            if !list.contains(f) {
                                list.push(f.clone());
                            }
                        }
                        list.sort();
                    }
                }
            }
            // The game runs the patch as separate edits while the skip applies (D49).
            FixChange::SkipHunk { patch, hunk } => {
                plan.skip_hunks.push(Lenient::Ok(SkipHunkFix {
                    fix_id: f.id.clone(),
                    patch: patch.clone(),
                    hunk: hunk.clone(),
                }));
            }
            // Acts in every stacking mode, Legacy too (D55).
            FixChange::StackRule {
                patch,
                site,
                side,
                below,
                above,
            } => {
                plan.stack_rules.push(Lenient::Ok(StackRuleFix {
                    fix_id: f.id.clone(),
                    patch: patch.clone(),
                    site: site.clone(),
                    side: *side,
                    below: below.clone(),
                    above: above.clone(),
                }));
            }
            _ => {}
        }
    }
    plan.schema_version = plan.schema_for_fields();
    plan
}

// ---- Pinning ----

/// The raw patches a plan name refers to: by the loader's rule (id, `mod:name`, a D29 old
/// id), or, for an order rule name, as a rule name matches.
pub fn hits(raw: &RawLoad, name: &str, rule_name: bool) -> Vec<usize> {
    raw.patches
        .iter()
        .enumerate()
        .filter(|(_, rp)| {
            if rule_name {
                order::name_matches(name, &rp.patch)
            } else {
                plan::plan_keys(&rp.patch).iter().any(|k| k == name)
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// A ref for plan entry `name`. When it names exactly one loaded patch, the ref carries that
/// patch's mod and fingerprint; otherwise no fingerprint, so the entry keeps matching by
/// name (a D29 duplicate, a mod that is off, a `mod:*` rule name).
pub fn pin(raw: Option<&RawLoad>, name: &str, rule_name: bool) -> PatchRef {
    let guess = name.split(':').next().unwrap_or_default().to_lowercase();
    let Some(raw) = raw else {
        return PatchRef {
            id: name.to_string(),
            mod_id: if name.contains(':') {
                guess
            } else {
                String::new()
            },
            fp: None,
        };
    };
    let found = if name.trim().ends_with(":*") {
        Vec::new()
    } else {
        hits(raw, name, rule_name)
    };
    let mod_of = |i: usize| {
        raw.patches
            .get(i)
            .and_then(|rp| raw.mods.get(rp.mod_ix as usize))
            .map(|m| m.id.clone())
    };
    if let [one] = found.as_slice() {
        let mut fps = Fingerprints::new(raw, FpMode::All);
        return PatchRef {
            id: name.to_string(),
            mod_id: mod_of(*one).unwrap_or(guess),
            fp: fps.get(*one).cloned(),
        };
    }
    let mods: BTreeSet<String> = found.iter().filter_map(|&i| mod_of(i)).collect();
    let mod_id = match (mods.len(), mods.into_iter().next()) {
        (1, Some(m)) => m,
        _ if name.contains(':') => guess,
        _ => String::new(),
    };
    PatchRef {
        id: name.to_string(),
        mod_id,
        fp: None,
    }
}

/// The snapshot of raw patch `ix`, with its mod's version.
pub fn snapshot_at(raw: &RawLoad, ix: usize) -> Option<PatchSnapshot> {
    let rp = raw.patches.get(ix)?;
    let m = raw.mods.get(rp.mod_ix as usize)?;
    let mut s = rp.patch.snapshot(&m.base);
    s.mod_version = m.version.clone();
    Some(s)
}

/// Snapshots of the patches `refs` name, where each names exactly one loaded patch.
pub fn take_snapshots(
    raw: Option<&RawLoad>,
    refs: &[&PatchRef],
    rule_names: bool,
) -> BTreeMap<String, PatchSnapshot> {
    let mut out = BTreeMap::new();
    let Some(raw) = raw else {
        return out;
    };
    for r in refs {
        if let [one] = hits(raw, &r.id, rule_names).as_slice() {
            if let Some(s) = snapshot_at(raw, *one) {
                out.insert(r.id.clone(), s);
            }
        }
    }
    out
}

/// The snapshots of `fix` taken again where its refs name one loaded patch; a ref that does
/// not keeps its stored snapshot (D41), also across a `renames` pair (old id, new id).
fn retake_snapshots(
    fix: &ProfileFix,
    raw: &RawLoad,
    renames: &[(String, String)],
) -> BTreeMap<String, PatchSnapshot> {
    let rule_names = matches!(fix.change, FixChange::Order { .. });
    let refs: Vec<&PatchRef> = fix.refs();
    let mut out = take_snapshots(Some(raw), &refs, rule_names);
    for r in refs {
        if out.contains_key(&r.id) {
            continue;
        }
        let old = fix.snapshots.get(&r.id).or_else(|| {
            renames
                .iter()
                .find(|(_, to)| *to == r.id)
                .and_then(|(from, _)| fix.snapshots.get(from))
        });
        if let Some(s) = old {
            out.insert(r.id.clone(), s.clone());
        }
    }
    out
}

/// The `(mod id, version)` pairs of the mods `refs` name.
pub fn mods_of(raw: Option<&RawLoad>, refs: &[&PatchRef]) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    for r in refs {
        if r.mod_id.is_empty() || out.iter().any(|(m, _)| *m == r.mod_id) {
            continue;
        }
        let version =
            r.fp.as_ref()
                .and_then(|fp| fp.mod_version.clone())
                .or_else(|| {
                    raw.and_then(|raw| {
                        raw.mods
                            .iter()
                            .find(|m| m.id.eq_ignore_ascii_case(&r.mod_id))
                    })
                    .and_then(|m| m.version.clone())
                });
        out.push((r.mod_id.clone(), version));
    }
    out
}

/// A fix as the ledger stores it: refs pinned, snapshots taken, mods listed.
pub fn make_fix(
    raw: Option<&RawLoad>,
    id: String,
    label: String,
    change: FixChange,
    mut provenance: Provenance,
    undo: UndoRecord,
) -> ProfileFix {
    let mut fix = ProfileFix {
        id,
        label,
        change,
        provenance: Provenance {
            mods: Vec::new(),
            ..provenance.clone()
        },
        paused: false,
        undo,
        snapshots: BTreeMap::new(),
        vanilla_hashes: BTreeMap::new(),
        // A migrated fix is stamped only when accepted (D74).
        fresh: provenance.source != FixSource::Migrated,
        stamp_pending: false,
    };
    let rule_names = matches!(fix.change, FixChange::Order { .. });
    let refs: Vec<PatchRef> = fix.refs().into_iter().cloned().collect();
    let refs: Vec<&PatchRef> = refs.iter().collect();
    fix.snapshots = take_snapshots(raw, &refs, rule_names);
    if provenance.mods.is_empty() {
        provenance.mods = mods_of(raw, &refs);
        if let FixChange::ModOrder {
            move_mod_id,
            before_mod_id,
            ..
        } = &fix.change
        {
            provenance.mods = vec![(move_mod_id.clone(), None), (before_mod_id.clone(), None)];
        }
    }
    fix.provenance = provenance;
    fix
}

/// Fixes for the legacy fields of `p`, in field order. A disable that a copy's base names is
/// owned by that copy. Ids come from `id_for(field, n)`.
fn from_legacy(
    p: &ModProfile,
    raw: Option<&RawLoad>,
    id_for: &mut dyn FnMut(&str, usize) -> String,
    source: FixSource,
) -> Vec<ProfileFix> {
    let mut out = Vec::new();
    let migrated = source == FixSource::Migrated;
    let prov = || Provenance::new(source);
    let copy_bases: HashSet<&str> = p
        .glue_patches
        .iter()
        .filter_map(|g| g.replaces_patch_id.as_deref())
        .collect();
    for (n, d) in p.disabled_patches.iter().enumerate() {
        if copy_bases.contains(d.as_str()) {
            continue;
        }
        let label = if migrated {
            "Turned off before this version".to_string()
        } else {
            format!("Turn off {}", patch_text(raw, d, false))
        };
        out.push(make_fix(
            raw,
            id_for("disabled_patches", n),
            label,
            FixChange::Disable {
                patch: pin(raw, d, false),
            },
            prov(),
            UndoRecord::Remove,
        ));
    }
    for (n, g) in p.glue_patches.iter().enumerate() {
        let disables_base = g
            .replaces_patch_id
            .as_ref()
            .is_some_and(|b| p.disabled_patches.contains(b));
        out.push(make_fix(
            raw,
            id_for("glue_patches", n),
            g.name.clone(),
            FixChange::AdjustedCopy {
                glue: g.clone(),
                base: g.replaces_patch_id.as_deref().map(|b| pin(raw, b, false)),
                disables_base,
            },
            prov(),
            UndoRecord::Remove,
        ));
    }
    for (n, rule) in p.patch_order.iter().enumerate() {
        let refs = std::iter::once(&rule.patch)
            .chain(&rule.before)
            .chain(&rule.after)
            .map(|name| pin(raw, name, true))
            .filter(|r| r.fp.is_some())
            .collect();
        out.push(make_fix(
            raw,
            id_for("patch_order", n),
            format!("Order rule for {}", patch_text(raw, &rule.patch, true)),
            FixChange::Order {
                rule: rule.clone(),
                refs,
            },
            prov(),
            UndoRecord::Remove,
        ));
    }
    let mut prios: Vec<(&String, &i32)> = p.patch_priorities.iter().collect();
    prios.sort();
    for (n, (k, v)) in prios.into_iter().enumerate() {
        out.push(make_fix(
            raw,
            id_for("patch_priorities", n),
            format!("Priority {v} for {}", patch_text(raw, k, false)),
            FixChange::Priority {
                patch: pin(raw, k, false),
                priority: *v,
            },
            prov(),
            UndoRecord::Remove,
        ));
    }
    let mut pairs: Vec<(&String, &String)> = p
        .user_load_after
        .iter()
        .flat_map(|(before, moves)| moves.iter().map(move |m| (before, m)))
        .collect();
    pairs.sort();
    for (n, (before, m)) in pairs.into_iter().enumerate() {
        out.push(make_fix(
            raw,
            id_for("user_load_after", n),
            format!(
                "Load {} before {}",
                mod_text(raw, m),
                mod_text(raw, before)
            ),
            FixChange::ModOrder {
                move_mod_id: m.clone(),
                before_mod_id: before.clone(),
                priority: None,
            },
            prov(),
            UndoRecord::RestoreModOrder {
                prev_priority: None,
                added_load_after: true,
            },
        ));
    }
    out
}

/// Read-only rows for a profile that is not migrated yet (D38): one per legacy entry, with
/// ids `legacy:<field>:<n>`.
pub fn legacy_fixes(p: &ModProfile, raw: Option<&RawLoad>) -> Vec<ProfileFix> {
    from_legacy(
        p,
        raw,
        &mut |field, n| format!("legacy:{field}:{n}"),
        FixSource::Migrated,
    )
}

// ---- Migration (§7.4) ----

/// `migration_error` after the user undid the import: the profile is not imported again until
/// "Retry import" clears it.
pub const IMPORT_UNDONE: &str = "You undid the import. Use Retry import to import them again.";

#[cfg(any(test, feature = "test-seams"))]
thread_local! {
    static FAIL_MIGRATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TAMPER_MIGRATION: std::cell::Cell<Option<fn(&mut ModProfile)>> =
        const { std::cell::Cell::new(None) };
}

/// Test seam for other crates' tests: make migration fail on this thread.
#[cfg(any(test, feature = "test-seams"))]
#[doc(hidden)]
pub fn set_fail_migration(on: bool) {
    FAIL_MIGRATION.with(|c| c.set(on));
}

/// Test seam: change the migrated profile before the byte guard checks it, on this thread.
#[cfg(any(test, feature = "test-seams"))]
#[doc(hidden)]
pub fn set_tamper_migration(f: Option<fn(&mut ModProfile)>) {
    TAMPER_MIGRATION.with(|c| c.set(f));
}

/// What a migration did.
#[derive(Debug, Clone)]
pub struct Migration {
    pub profile: ModProfile,
    /// How many fixes were created.
    pub imported: usize,
    /// How many game files the text guard compared (0 without vanilla files).
    pub text_files: usize,
}

/// Turns the legacy fields of an unmigrated profile into ledger fixes (§7.4). Pure. The
/// result is checked to give the same game code: the legacy fields as sets, the applied
/// patches for the mods in `raw` (no game files needed), and, when `vanilla` is given, the
/// patched text of every file. Any difference aborts the migration.
pub fn migrate(
    profile: &ModProfile,
    raw: &RawLoad,
    vanilla: Option<&HashMap<String, String>>,
) -> Result<ModProfile, String> {
    migrate_checked(profile, raw, vanilla).map(|m| m.profile)
}

/// `migrate`, with what it checked.
pub fn migrate_checked(
    profile: &ModProfile,
    raw: &RawLoad,
    vanilla: Option<&HashMap<String, String>>,
) -> Result<Migration, String> {
    if profile.fixes_version >= FIXES_VERSION {
        return Ok(Migration {
            profile: profile.clone(),
            imported: 0,
            text_files: 0,
        });
    }
    if profile.migration_error.as_deref() == Some(IMPORT_UNDONE) {
        return Err(IMPORT_UNDONE.to_string());
    }
    #[cfg(any(test, feature = "test-seams"))]
    if FAIL_MIGRATION.with(std::cell::Cell::get) {
        return Err("the migration was stopped for a test".to_string());
    }
    let created = from_legacy(
        profile,
        Some(raw),
        &mut |_, _| new_fix_id(),
        FixSource::Migrated,
    );
    let imported = created.len();
    let ids: Vec<String> = created.iter().map(|f| f.id.clone()).collect();
    let mut m = profile.clone();
    m.fixes.extend(created.into_iter().map(Lenient::Ok));
    m.fixes_version = FIXES_VERSION;
    m.migration_error = None;
    sync_legacy(&mut m);
    #[cfg(any(test, feature = "test-seams"))]
    if let Some(f) = TAMPER_MIGRATION.with(std::cell::Cell::get) {
        f(&mut m);
    }

    same_fields(profile, &m)?;
    same_applied(profile, &m, raw)?;
    let text_files = match vanilla {
        Some(files) => same_text(profile, &m, raw, files)?,
        None => 0,
    };

    if imported == 0 {
        m.mark_notice_seen(NOTICE_FIXES_IMPORT);
    } else {
        let label = if imported == 1 {
            "Imported 1 earlier change".to_string()
        } else {
            format!("Imported {imported} earlier changes")
        };
        push_journal(
            &mut m,
            JournalEntry {
                at: now(),
                op: "migrated".to_string(),
                label,
                removed: Vec::new(),
                added: ids,
                mods_enabled: Vec::new(),
            },
        );
    }
    Ok(Migration {
        profile: m,
        imported,
        text_files,
    })
}

fn same_fields(old: &ModProfile, new: &ModProfile) -> Result<(), String> {
    same_plan_fields(old, new)?;
    if old.user_load_after != new.user_load_after || old.mod_priorities != new.mod_priorities {
        return Err("the load order would change".to_string());
    }
    Ok(())
}

/// Whether two profiles have the same plan fields (turned-off patches, priorities, copies and
/// order rules), as sets. The load order is not compared.
pub fn same_plan_fields(old: &ModProfile, new: &ModProfile) -> Result<(), String> {
    let set = |v: &[String]| v.iter().cloned().collect::<BTreeSet<String>>();
    if set(&old.disabled_patches) != set(&new.disabled_patches) {
        return Err("the turned-off patches would change".to_string());
    }
    if old.patch_priorities != new.patch_priorities {
        return Err("the patch priorities would change".to_string());
    }
    // Whole entries: a copy edited under the same id is a change too.
    let glues = |p: &ModProfile| {
        p.glue_patches
            .iter()
            .map(|g| serde_json::to_value(g).ok())
            .collect::<Vec<_>>()
    };
    if glues(old) != glues(new) {
        return Err("the adjusted copies would change".to_string());
    }
    let rules = |p: &ModProfile| {
        p.patch_order
            .iter()
            .map(|r| serde_json::to_string(r).unwrap_or_default())
            .collect::<BTreeSet<String>>()
    };
    if rules(old) != rules(new) {
        return Err("the order rules would change".to_string());
    }
    Ok(())
}

/// `plan` laid over `raw` as the game reads it: through JSON.
fn applied_for(raw: &RawLoad, plan: &LoadOrderPlan) -> Result<Applied, String> {
    let json = serde_json::to_string(plan).map_err(|e| e.to_string())?;
    let read = LoadOrderPlan::parse(&json);
    if let Some(e) = &read.error {
        return Err(e.to_string());
    }
    Ok(plan::apply_plan(raw, &read))
}

/// Everything in `Applied` that decides game output (D38). Fix ids are left out: they name
/// the fix, not the code.
pub fn applied_shape(a: &Applied) -> serde_json::Value {
    let patches: Vec<serde_json::Value> = a
        .patches
        .iter()
        .map(|(p, prio, path)| {
            let mut extra = serde_json::to_value(p.extra()).unwrap_or_default();
            if let Some(o) = extra.as_object_mut() {
                o.remove("fix_ids");
            }
            serde_json::json!({
                "patch": p,
                "id": p.id(),
                "mod_id": p.mod_id(),
                "disabled": p.is_disabled(),
                "priority": prio,
                "path": path.to_string_lossy().replace('\\', "/"),
                "extra": extra,
            })
        })
        .collect();
    let targets: BTreeSet<&String> = a.targets.iter().collect();
    let vars: BTreeMap<&String, &String> = a.vars.iter().collect();
    serde_json::json!({
        "patches": patches,
        "targets": targets,
        "vars": vars,
        "order_rules": a.order_rules,
        "compat_rules": a.compat_rules,
        "stack_mode": a.stack_mode,
        "load_errors": a.load_errors,
    })
}

fn same_applied(old: &ModProfile, new: &ModProfile, raw: &RawLoad) -> Result<(), String> {
    let enabled: Vec<String> = raw.mods.iter().map(|m| m.id.clone()).collect();
    let a = applied_for(raw, &compile_plan(old, enabled.clone(), HashMap::new()))?;
    let b = applied_for(raw, &compile_plan(new, enabled, HashMap::new()))?;
    if applied_shape(&a) != applied_shape(&b) {
        return Err("the imported fixes would change the game's patches".to_string());
    }
    Ok(())
}

/// Compares the patched text of every vanilla file either plan touches; the number compared.
fn same_text(
    old: &ModProfile,
    new: &ModProfile,
    raw: &RawLoad,
    vanilla: &HashMap<String, String>,
) -> Result<usize, String> {
    let enabled: Vec<String> = raw.mods.iter().map(|m| m.id.clone()).collect();
    let table = |p: &ModProfile| -> Result<PatchTable, String> {
        let a = applied_for(raw, &compile_plan(p, enabled.clone(), HashMap::new()))?;
        Ok(PatchTable::from_applied(a, raw.mod_dir.clone()))
    };
    let (ta, tb) = (table(old)?, table(new)?);
    let (pa, pb) = (ta.prepare(), tb.prepare());
    let launch = LaunchConfig::default();
    let mut names: Vec<&String> = vanilla.keys().collect();
    names.sort();
    let mut n = 0;
    for name in names {
        if !ta.needs_patching(name) && !tb.needs_patching(name) {
            continue;
        }
        let text = vanilla.get(name).map(String::as_str).unwrap_or_default();
        let run = |t: &PatchTable, p: &qm_core::patch::table::Prepared| {
            qm_core::prepare::simulate_buffer(t, p, &launch, name, text).map(|(_, a)| a.text)
        };
        if run(&ta, &pa) != run(&tb, &pb) {
            return Err(format!("the imported fixes would change {name}"));
        }
        n += 1;
    }
    Ok(n)
}

// ---- Clones ----

/// A cloned profile gets its own fix ids and no journal (D40).
pub fn renew_for_clone(p: &mut ModProfile) {
    for f in fixes_mut(p) {
        f.id = new_fix_id();
    }
    p.fix_journal.clear();
    p.migration_error = None;
}

// ---- Ledger operations ----
//
// Each keeps the legacy behaviour on an unmigrated profile (the legacy fields only) and
// records a fix on a migrated one, then rewrites the legacy fields from the ledger.

/// Turns `id` off unless one of `aliases` already is: the legacy `patch_set_enabled(false)`.
/// Returns the fix that turns it off, if the profile is migrated.
pub fn op_disable(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    id: &str,
    aliases: &HashSet<String>,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        if !p.disabled_patches.iter().any(|x| aliases.contains(x)) {
            p.disabled_patches.push(id.to_string());
        }
        return None;
    }
    let on = |f: &ProfileFix| match &f.change {
        FixChange::Disable { patch } => aliases.contains(&patch.id),
        FixChange::AdjustedCopy {
            glue,
            disables_base: true,
            ..
        } => glue
            .replaces_patch_id
            .as_ref()
            .is_some_and(|b| aliases.contains(b)),
        _ => false,
    };
    if let Some(f) = fixes(p).find(|f| !f.paused && on(f)) {
        return Some(f.id.clone());
    }
    let paused = fixes(p)
        .find(|f| f.paused && matches!(f.change, FixChange::Disable { .. }) && on(f))
        .map(|f| f.id.clone());
    let id_out = match paused {
        Some(fid) => {
            if let Some(f) = find_mut(p, &fid) {
                f.paused = false;
                f.log(FixEventKind::Resumed, "turned off again");
            }
            fid
        }
        None => {
            let fix = make_fix(
                raw,
                new_fix_id(),
                format!("Turn off {}", patch_text(raw, id, false)),
                FixChange::Disable {
                    patch: pin(raw, id, false),
                },
                provenance,
                UndoRecord::Remove,
            );
            let fid = fix.id.clone();
            push(p, fix);
            fid
        }
    };
    sync_legacy(p);
    Some(id_out)
}

/// Turns the patch named by any of `aliases` back on: the legacy `patch_set_enabled(true)`.
pub fn op_enable(p: &mut ModProfile, aliases: &HashSet<String>) {
    if p.fixes_version < FIXES_VERSION {
        p.disabled_patches.retain(|x| !aliases.contains(x));
        return;
    }
    p.fixes.retain(|f| match f.ok().map(|f| &f.change) {
        Some(FixChange::Disable { patch }) => !aliases.contains(&patch.id),
        _ => true,
    });
    for f in fixes_mut(p) {
        if let FixChange::AdjustedCopy {
            glue,
            disables_base,
            ..
        } = &mut f.change
        {
            if glue
                .replaces_patch_id
                .as_ref()
                .is_some_and(|b| aliases.contains(b))
            {
                *disables_base = false;
            }
        }
    }
    sync_legacy(p);
}

/// Sets the profile's order rule for `rule.patch`, replacing an earlier one (legacy
/// `merge_rule`). Returns the new fix's id on a migrated profile.
pub fn op_order_set(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    rule: OrderRule,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        p.patch_order.retain(|r| r.patch != rule.patch);
        p.patch_order.push(rule);
        return None;
    }
    // A paused rule stays as it is (paused): undo of the new one must not turn it on.
    let prev: Vec<String> = fixes(p)
        .filter(|f| !f.paused)
        .filter(|f| matches!(&f.change, FixChange::Order { rule: r, .. } if r.patch == rule.patch))
        .map(|f| f.id.clone())
        .collect();
    let mut prev_rule = None;
    for id in &prev {
        if let Some(FixChange::Order { rule, .. }) = remove(p, id).map(|f| f.change) {
            prev_rule = Some(rule);
        }
    }
    let undo = match prev_rule {
        Some(r) => UndoRecord::RestoreOrder(Some(r)),
        None => UndoRecord::Remove,
    };
    let refs = std::iter::once(&rule.patch)
        .chain(&rule.before)
        .chain(&rule.after)
        .map(|name| pin(raw, name, true))
        .filter(|r| r.fp.is_some())
        .collect();
    let fix = make_fix(
        raw,
        new_fix_id(),
        order_label(raw, &rule),
        FixChange::Order { rule, refs },
        provenance,
        undo,
    );
    let id = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(id)
}

fn order_label(raw: Option<&RawLoad>, rule: &OrderRule) -> String {
    let name = |n: &str| patch_text(raw, n, true);
    match (rule.before.first(), rule.after.first()) {
        (Some(b), _) => format!("Run {} before {}", name(&rule.patch), name(b)),
        (None, Some(a)) => format!("Run {} after {}", name(&rule.patch), name(a)),
        (None, None) => format!("Order rule for {}", name(&rule.patch)),
    }
}

/// A patch as a fix label names it (§11, plain text): its name when `name` is one loaded
/// patch that has one, else `name` as written.
fn patch_text(raw: Option<&RawLoad>, name: &str, rule_name: bool) -> String {
    raw.and_then(|r| match hits(r, name, rule_name).as_slice() {
        [one] => r.patches.get(*one).and_then(|rp| rp.patch.name()),
        _ => None,
    })
    .map(str::trim)
    .filter(|n| !n.is_empty())
    .map_or_else(|| name.to_string(), str::to_string)
}

/// A mod as a fix label names it: its mod.json name when it is loaded, else its id.
fn mod_text(raw: Option<&RawLoad>, id: &str) -> String {
    raw.and_then(|r| r.mods.iter().find(|m| m.id.eq_ignore_ascii_case(id)))
        .and_then(|m| m.name.as_deref())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map_or_else(|| id.to_string(), str::to_string)
}

/// Removes the profile's order rule for `patch` (legacy `patch_order_reset`).
pub fn op_order_reset(p: &mut ModProfile, patch: &str) {
    if p.fixes_version < FIXES_VERSION {
        p.patch_order.retain(|r| r.patch != patch);
        return;
    }
    p.fixes.retain(|f| {
        !matches!(f.ok().map(|f| &f.change), Some(FixChange::Order { rule, .. }) if rule.patch == patch)
    });
    sync_legacy(p);
}

/// Sets a patch priority, replacing an earlier one for the same entry.
pub fn op_priority_set(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    id: &str,
    priority: Priority,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        p.patch_priorities.insert(id.to_string(), priority);
        return None;
    }
    // Every name of the patch (id, `mod:name`, old id), in the loader's key order: a fix on
    // another name of the same patch is replaced too.
    let keys: Vec<String> = match raw.map(|r| (r, hits(r, id, false))) {
        Some((r, found)) if found.len() == 1 => found
            .first()
            .and_then(|&i| r.patches.get(i))
            .map(|rp| plan::plan_keys(&rp.patch))
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let keys = if keys.iter().any(|k| k == id) {
        keys
    } else {
        vec![id.to_string()]
    };
    let rank = |k: &str| keys.iter().position(|x| x == k).unwrap_or(usize::MAX);
    // A paused priority stays as it is (paused): undo of the new one must not turn it on.
    let prev: Vec<String> = fixes(p)
        .filter(|f| !f.paused)
        .filter(|f| matches!(&f.change, FixChange::Priority { patch, .. } if rank(&patch.id) != usize::MAX))
        .map(|f| f.id.clone())
        .collect();
    // The one that took effect: the first key in the loader's order.
    let mut prev_prio: Option<(usize, Priority)> = None;
    for fid in &prev {
        if let Some(FixChange::Priority { patch, priority }) = remove(p, fid).map(|f| f.change) {
            let r = rank(&patch.id);
            if prev_prio.is_none_or(|(best, _)| r <= best) {
                prev_prio = Some((r, priority));
            }
        }
    }
    let prev_prio = prev_prio.map(|(_, v)| v);
    let undo = match prev_prio {
        Some(v) => UndoRecord::RestorePriority(Some(v)),
        None => UndoRecord::Remove,
    };
    let fix = make_fix(
        raw,
        new_fix_id(),
        format!("Priority {priority} for {}", patch_text(raw, id, false)),
        FixChange::Priority {
            patch: pin(raw, id, false),
            priority,
        },
        provenance,
        undo,
    );
    let fid = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(fid)
}

/// Anchors patch `id` on other patches' code (an Anchor fix, §3.11), replacing an earlier
/// Anchor fix on the same patch; undo puts that one back. Each rung on another patch gets
/// that patch's fingerprint when it names exactly one, and the fix keeps a snapshot of every
/// patch it names (D41). `None` on a profile that is not migrated.
pub fn op_anchor_set(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    id: &str,
    anchors: Vec<Rung>,
    sites: Vec<SiteKey>,
    label: Option<String>,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    // A paused anchor stays as it is (paused): undo of the new one must not turn it on.
    let prev: Vec<String> = fixes(p)
        .filter(|f| !f.paused)
        .filter(|f| matches!(&f.change, FixChange::Anchor { patch, .. } if patch.id == id))
        .map(|f| f.id.clone())
        .collect();
    // The newest one took effect; undo restores it.
    let mut undo = UndoRecord::Remove;
    for fid in &prev {
        if let Some(f) = remove(p, fid) {
            if let FixChange::Anchor {
                anchors,
                sites,
                replace_author_anchors,
                ..
            } = f.change
            {
                undo = UndoRecord::RestoreAnchor {
                    label: f.label,
                    anchors,
                    sites,
                    replace_author_anchors,
                };
            }
        }
    }
    let label = label.filter(|l| !l.trim().is_empty());
    let fix = anchor_fix(
        raw,
        pin(raw, id, false),
        anchors,
        sites,
        false,
        label,
        provenance,
        undo,
    );
    let fid = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(fid)
}

/// An Anchor fix on `patch`, with B's fingerprints, snapshots and mods (see `op_anchor_set`).
#[allow(clippy::too_many_arguments)]
fn anchor_fix(
    raw: Option<&RawLoad>,
    patch: PatchRef,
    mut anchors: Vec<Rung>,
    sites: Vec<SiteKey>,
    replace_author_anchors: bool,
    label: Option<String>,
    provenance: Provenance,
    undo: UndoRecord,
) -> ProfileFix {
    let id = patch.id.clone();
    let mut b_refs: Vec<PatchRef> = Vec::new();
    for rung in &mut anchors {
        let Some(within) = rung.within.clone() else {
            continue;
        };
        let b = pin(raw, &within, false);
        if rung.b_fp.is_none() {
            rung.b_fp = b.fp.clone();
        }
        b_refs.push(b);
    }
    let label = label.unwrap_or_else(|| match anchors.first().and_then(|r| r.within.as_deref()) {
        Some(b) => format!("Keep {id} next to {b}"),
        None => format!("Move {id} to another line"),
    });
    let mut fix = make_fix(
        raw,
        new_fix_id(),
        label,
        FixChange::Anchor {
            patch,
            anchors,
            sites,
            replace_author_anchors,
        },
        provenance,
        undo,
    );
    let b_refs: Vec<&PatchRef> = b_refs.iter().collect();
    for (k, v) in take_snapshots(raw, &b_refs, false) {
        fix.snapshots.entry(k).or_insert(v);
    }
    for m in mods_of(raw, &b_refs) {
        if !fix.provenance.mods.iter().any(|(x, _)| *x == m.0) {
            fix.provenance.mods.push(m);
        }
    }
    fix
}

/// Runs `at` patch `id` as separate edits or as one replacement (an ApplyAs fix, D46),
/// replacing an earlier ApplyAs fix on the same patch; undo puts that one back. A paused one
/// stays as it is. `None` on a profile that is not migrated.
pub fn op_apply_as_set(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    id: &str,
    mode: ApplyAs,
    label: Option<String>,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    // A paused one stays as it is (paused): undo of the new one must not turn it on.
    let prev: Vec<String> = fixes(p)
        .filter(|f| !f.paused)
        .filter(|f| matches!(&f.change, FixChange::ApplyAs { patch, .. } if patch.id == id))
        .map(|f| f.id.clone())
        .collect();
    let mut undo = UndoRecord::Remove;
    for fid in &prev {
        if let Some(f) = remove(p, fid) {
            if let FixChange::ApplyAs { mode, .. } = f.change {
                undo = UndoRecord::RestoreApplyAs {
                    label: f.label,
                    mode,
                };
            }
        }
    }
    let label = label
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| apply_as_label(raw, id, mode));
    let fix = make_fix(
        raw,
        new_fix_id(),
        label,
        FixChange::ApplyAs {
            patch: pin(raw, id, false),
            mode,
        },
        provenance,
        undo,
    );
    let fid = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(fid)
}

/// Gives asset group or slot `key` to `winner` (an AssetChoice fix, Phase E D76), replacing
/// an earlier choice for the same key; undo puts that one back. A paused one stays as it
/// is. `excludes` is what the choice leaves out (`assets_ledger::exclusions_of`). `None` on
/// a profile that is not migrated.
pub fn op_asset_choice_set(
    p: &mut ModProfile,
    key: &str,
    winner: &str,
    excludes: BTreeMap<String, Vec<String>>,
    label: Option<String>,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    let prev: Vec<String> = fixes(p)
        .filter(|f| !f.paused)
        .filter(|f| matches!(&f.change, FixChange::AssetChoice { key: k, .. } if k == key))
        .map(|f| f.id.clone())
        .collect();
    let mut undo = UndoRecord::Remove;
    for fid in &prev {
        if let Some(f) = remove(p, fid) {
            if let FixChange::AssetChoice {
                winner, excludes, ..
            } = f.change
            {
                undo = UndoRecord::RestoreAssetChoice {
                    label: f.label,
                    winner,
                    excludes,
                };
            }
        }
    }
    let label = label
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| asset_choice_label(key, winner));
    let fix = make_fix(
        None,
        new_fix_id(),
        label,
        FixChange::AssetChoice {
            key: key.to_string(),
            winner: winner.to_string(),
            excludes,
        },
        provenance,
        undo,
    );
    let fid = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(fid)
}

/// "Use <winner> for <key>", with the game named plainly.
pub fn asset_choice_label(key: &str, winner: &str) -> String {
    let who = match winner {
        "vanilla" => "the game's own files".to_string(),
        w => match w.strip_prefix("faction:") {
            Some(id) => format!("the faction {id}"),
            None => w.to_string(),
        },
    };
    format!("Use {who} for {key}")
}

/// The profile's running asset choices: group or slot key -> winner.
pub fn asset_choices(p: &ModProfile) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for f in fixes(p).filter(|f| !f.paused) {
        if let FixChange::AssetChoice { key, winner, .. } = &f.change {
            out.entry(key.clone()).or_insert_with(|| winner.clone());
        }
    }
    out
}

/// Brings each asset choice's `excludes` in line with `by_key` (key -> what the choice
/// leaves out now, from the asset ledger): a mod that renamed or added files changes the
/// list. A changed fix gets a history event. Returns how many fixes changed.
pub fn refresh_asset_excludes(
    p: &mut ModProfile,
    by_key: &BTreeMap<String, BTreeMap<String, Vec<String>>>,
) -> usize {
    let mut n = 0;
    for f in p.fixes.iter_mut().filter_map(|f| match f {
        Lenient::Ok(f) => Some(f),
        Lenient::Invalid { .. } => None,
    }) {
        let FixChange::AssetChoice { key, excludes, .. } = &mut f.change else {
            continue;
        };
        let now = by_key.get(key).cloned().unwrap_or_default();
        if *excludes != now {
            *excludes = now;
            f.log(FixEventKind::Rebound, "the files this choice leaves out changed");
            n += 1;
        }
    }
    n
}

/// "Run <A> as separate edits" or "Run <A> as one replacement".
pub fn apply_as_label(raw: Option<&RawLoad>, id: &str, mode: ApplyAs) -> String {
    let name = patch_text(raw, id, false);
    match mode {
        ApplyAs::Edits => format!("Run {name} as separate edits"),
        ApplyAs::Literal => format!("Run {name} as one replacement"),
    }
}

/// The key of edit `ordinal` of `at` patch `id` (D49), with the patch's content now and the
/// patch whose change stays. Err when `id` names no one `at` pattern patch that runs as
/// separate edits, or it has no such edit.
pub fn hunk_key(
    raw: &RawLoad,
    id: &str,
    ordinal: u32,
    keep: Option<String>,
) -> Result<HunkKey, String> {
    let ix = match hits(raw, id, false).as_slice() {
        [one] => *one,
        [] => return Err(format!("No loaded patch is called '{id}'.")),
        _ => return Err(format!("'{id}' names more than one patch.")),
    };
    let Some(qm_core::patch::Patch::Pattern(p)) = raw.patches.get(ix).map(|rp| &rp.patch) else {
        return Err(format!("{id} does not replace lines, so it has no separate edits."));
    };
    if p.position != qm_core::patch::InsertPosition::At {
        return Err(format!("{id} does not replace lines, so it has no separate edits."));
    }
    let hunks = p
        .normalized()
        .map_err(|r| format!("{id} cannot run as separate edits: {}.", r.why()))?;
    let Some((_, h)) = edits::edit_hunks(&hunks).find(|(o, _)| *o == ordinal) else {
        return Err(format!("{id} has no change number {}.", ordinal + 1));
    };
    let fp = Fingerprints::new(raw, FpMode::All)
        .get(ix)
        .map(|f| f.lite())
        .ok_or_else(|| format!("{id} could not be read."))?;
    Ok(HunkKey {
        fp,
        vanilla: edits::hunk_vanilla(&p.pattern, h),
        ordinal,
        keep,
    })
}

/// Skips one edit of `at` patch `id` (a SkipHunk fix, D49), so another patch's change to the
/// same lines stays. It replaces a running fix on the same edit, and the other patch's choice
/// on the same lines (Keep A and Keep B exclude each other); undo puts every one back. A
/// paused one stays as it is. `None` on a profile that is not migrated.
pub fn op_hunk_skip(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    id: &str,
    hunk: HunkKey,
    label: Option<String>,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    let other = |f: &ProfileFix| match &f.change {
        FixChange::SkipHunk { patch, hunk: h } => {
            (patch.id == id && h.ordinal == hunk.ordinal)
                || (hunk.keep.as_deref() == Some(patch.id.as_str())
                    && h.keep.as_deref() == Some(id)
                    && same_lines(&h.vanilla, &hunk.vanilla))
        }
        _ => false,
    };
    let prev: Vec<String> = fixes(p)
        .filter(|f| !f.paused && other(f))
        .map(|f| f.id.clone())
        .collect();
    let mut restores = Vec::new();
    for fid in &prev {
        if let Some(f) = remove(p, fid) {
            if let FixChange::SkipHunk { patch, hunk } = f.change {
                restores.push(UndoRecord::RestoreHunk {
                    label: f.label,
                    patch,
                    hunk,
                    also: Vec::new(),
                });
            }
        }
    }
    let undo = undo_all(restores);
    let label = label
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| skip_hunk_label(raw, id, &hunk));
    let fix = make_fix(
        raw,
        new_fix_id(),
        label,
        FixChange::SkipHunk {
            patch: pin(raw, id, false),
            hunk,
        },
        provenance,
        undo,
    );
    let fid = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(fid)
}

/// Whether two patches' edits are about the same game lines, read from their pattern lines
/// (trimmed, possibly wildcards): a line of one is a line of the other. An empty text (an
/// entry without it) counts as the same, as before.
pub fn same_lines(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    let fits = |x: &str, y: &str| {
        x == y
            || wildmatch::WildMatch::new(x).matches(y)
            || wildmatch::WildMatch::new(y).matches(x)
    };
    a.lines().any(|x| b.lines().any(|y| fits(x, y)))
}

/// "Keep <B>'s change to <line>", or "Skip <A>'s change to <line>" when no patch is kept.
pub fn skip_hunk_label(raw: Option<&RawLoad>, id: &str, hunk: &HunkKey) -> String {
    let line = hunk.vanilla.lines().next().unwrap_or("a new line");
    match hunk.keep.as_deref() {
        Some(keep) => format!("Keep {}'s change to {line}", patch_text(raw, keep, false)),
        None => format!("Skip {}'s change to {line}", patch_text(raw, id, false)),
    }
}

/// `hunk` pinned to raw patch `ix` as it is now: its content, and the edit with the same
/// lines when exactly one has them (an edit that moved). Unchanged when none has.
fn repin_hunk(raw: &RawLoad, ix: usize, hunk: &mut HunkKey) {
    let Some(qm_core::patch::Patch::Pattern(p)) = raw.patches.get(ix).map(|rp| &rp.patch) else {
        return;
    };
    if let Some(fp) = Fingerprints::new(raw, FpMode::All).get(ix) {
        hunk.fp = fp.lite();
    }
    let Ok(hunks) = p.normalized() else {
        return;
    };
    let same: Vec<u32> = edits::edit_hunks(&hunks)
        .filter(|(_, h)| edits::hunk_vanilla(&p.pattern, h) == hunk.vanilla)
        .map(|(o, _)| o)
        .collect();
    if let [one] = same.as_slice() {
        hunk.ordinal = *one;
    }
}

/// One undo record that puts back every fix in `restores` (each a `RestoreHunk` or a
/// `RestoreStack`): the last is the record, the others ride in its `also`.
fn undo_all(mut restores: Vec<UndoRecord>) -> UndoRecord {
    let Some(mut last) = restores.pop() else {
        return UndoRecord::Remove;
    };
    if let UndoRecord::RestoreHunk { also, .. } | UndoRecord::RestoreStack { also, .. } = &mut last {
        *also = restores;
    }
    last
}

/// Orders patch `id`'s block at one line (a StackRule fix, D55): at the game's line `site`, on
/// `side`, below `below`'s block or above `above`'s. It replaces a running rule for the same
/// patch, line and side, and the other block's rule there that names this patch; undo puts
/// every one back. A paused one stays as it is. `None` on a
/// profile that is not migrated.
#[allow(clippy::too_many_arguments)]
pub fn op_stack_set(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    id: &str,
    site: SiteKey,
    side: Side,
    below: Option<&str>,
    above: Option<&str>,
    label: Option<String>,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    let want = stack_slot_key(id, &site, side);
    let below = below.map(|b| pin(raw, b, false));
    let above = above.map(|a| pin(raw, a, false));
    let me = pin(raw, id, false).id;
    // The other block's rule at this line and side that names this patch orders the same two
    // blocks: the newer choice replaces it too.
    let opposite = below.as_ref().or(above.as_ref()).map(|o| {
        (
            o.id.clone(),
            stack_slot_key(&o.id, &site, side),
        )
    });
    let names_me = |f: &ProfileFix| match &f.change {
        FixChange::StackRule { below, above, .. } => [below, above]
            .into_iter()
            .flatten()
            .any(|r| r.id == me || r.id == id),
        _ => false,
    };
    let prev: Vec<String> = fixes(p)
        .filter(|f| {
            !f.paused
                && slot(f).is_some_and(|(k, v)| {
                    k == "stack"
                        && (v == want
                            || opposite.as_ref().is_some_and(|(_, o)| *o == v) && names_me(f))
                })
        })
        .map(|f| f.id.clone())
        .collect();
    let mut restores = Vec::new();
    for fid in &prev {
        if let Some(f) = remove(p, fid) {
            if let FixChange::StackRule {
                patch,
                below,
                above,
                ..
            } = f.change
            {
                let other = opposite
                    .as_ref()
                    .filter(|(o, _)| *o == patch.id)
                    .map(|_| patch);
                restores.push(UndoRecord::RestoreStack {
                    label: f.label,
                    below,
                    above,
                    patch: other,
                    also: Vec::new(),
                });
            }
        }
    }
    let undo = undo_all(restores);
    let label = label
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| stack_label(raw, id, below.as_ref(), above.as_ref(), None));
    let fix = make_fix(
        raw,
        new_fix_id(),
        label,
        FixChange::StackRule {
            patch: pin(raw, id, false),
            site,
            side,
            below,
            above,
        },
        provenance,
        undo,
    );
    let fid = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(fid)
}

/// "Put <A>'s block below <B>'s" (or above), with " at <line>" when the line is known.
pub fn stack_label(
    raw: Option<&RawLoad>,
    id: &str,
    below: Option<&PatchRef>,
    above: Option<&PatchRef>,
    line: Option<&str>,
) -> String {
    let name = patch_text(raw, id, false);
    let place = match (below, above) {
        (Some(b), _) => format!("below {}'s", patch_text(raw, &b.id, false)),
        (None, Some(a)) => format!("above {}'s", patch_text(raw, &a.id, false)),
        (None, None) => "in place".to_string(),
    };
    match line.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => format!("Put {name}'s block {place} at {l}"),
        None => format!("Put {name}'s block {place}"),
    }
}

/// Adds an adjusted copy, replacing one with the same id. With `disables_base` the copy also
/// turns its base off (`glue_patch_create`); a disable the user had made before is kept when
/// the copy goes (`KeepDisable`).
pub fn op_glue_add(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    glue: GluePatchEntry,
    disables_base: bool,
    provenance: Provenance,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        p.glue_patches.retain(|g| g.id != glue.id);
        if disables_base {
            if let Some(base) = &glue.replaces_patch_id {
                if !p.disabled_patches.contains(base) {
                    p.disabled_patches.push(base.clone());
                }
            }
        }
        p.glue_patches.push(glue);
        return None;
    }
    let same: Vec<String> = fixes(p)
        .filter(|f| !f.paused)
        .filter(|f| matches!(&f.change, FixChange::AdjustedCopy { glue: g, .. } if g.id == glue.id))
        .map(|f| f.id.clone())
        .collect();
    for id in &same {
        remove(p, id);
    }
    // A paused copy with this id stays (paused); the new copy gets an id of its own.
    // (The legacy field still holds the copy replaced above until `sync_legacy`: only the
    // ledger counts here.)
    let in_ledger = |p: &ModProfile, id: &str| {
        fixes(p).any(|f| matches!(&f.change, FixChange::AdjustedCopy { glue: g, .. } if g.id == id))
    };
    let mut glue = glue;
    if in_ledger(p, &glue.id) {
        let old = glue.id.clone();
        let fresh = (2..)
            .map(|n| format!("{old}:{n}"))
            .find(|id| !in_ledger(p, id))
            .unwrap_or_default();
        if glue.patch.id() == Some(old.as_str()) {
            glue.patch.set_id(fresh.clone());
        }
        glue.id = fresh;
    }
    let already_off = glue
        .replaces_patch_id
        .as_ref()
        .is_some_and(|b| p.disabled_patches.contains(b));
    let undo = if disables_base && already_off {
        UndoRecord::KeepDisable
    } else {
        UndoRecord::Remove
    };
    let base = glue
        .replaces_patch_id
        .as_deref()
        .map(|b| pin(raw, b, false));
    let fix = make_fix(
        raw,
        new_fix_id(),
        glue.name.clone(),
        FixChange::AdjustedCopy {
            glue,
            base,
            disables_base,
        },
        provenance,
        undo,
    );
    let id = fix.id.clone();
    push(p, fix);
    sync_legacy(p);
    Some(id)
}

/// Removes the adjusted copy `glue_id`; false when there is none. Its base comes back on
/// unless another copy or a disable of the user's own keeps it off.
pub fn op_glue_remove(p: &mut ModProfile, glue_id: &str) -> bool {
    if p.fixes_version < FIXES_VERSION {
        let Some(at) = p.glue_patches.iter().position(|g| g.id == glue_id) else {
            return false;
        };
        let removed = p.glue_patches.remove(at);
        if let Some(base) = removed.replaces_patch_id {
            if !p
                .glue_patches
                .iter()
                .any(|g| g.replaces_patch_id.as_deref() == Some(base.as_str()))
            {
                p.disabled_patches.retain(|d| *d != base);
            }
        }
        return true;
    }
    let found: Vec<String> = fixes(p)
        .filter(|f| matches!(&f.change, FixChange::AdjustedCopy { glue, .. } if glue.id == glue_id))
        .map(|f| f.id.clone())
        .collect();
    if found.is_empty() {
        return false;
    }
    for id in &found {
        remove(p, id);
    }
    sync_legacy(p);
    true
}

/// Whether `f` is the adjusted copy `glue_id`.
fn is_copy(f: &ProfileFix, glue_id: &str) -> bool {
    matches!(&f.change, FixChange::AdjustedCopy { glue, .. } if glue.id == glue_id)
}

/// The adjusted copy `glue_id` of a migrated profile: the running one, else a paused one.
pub fn copy_fix<'a>(p: &'a ModProfile, glue_id: &str) -> Option<&'a ProfileFix> {
    fixes(p)
        .find(|f| !f.paused && is_copy(f, glue_id))
        .or_else(|| fixes(p).find(|f| is_copy(f, glue_id)))
}

/// Whether a running newer copy of the copy `glue_id` stands in for it, so it does not run
/// (D59).
pub fn copy_superseded(p: &ModProfile, glue_id: &str) -> bool {
    fixes(p).filter(|f| !f.paused).any(|f| {
        matches!(&f.change, FixChange::AdjustedCopy { glue, .. }
            if glue.replaces_patch_id.as_deref() == Some(glue_id))
    })
}

/// The mod `glue` runs as when it runs like its base mod (D58): the mod of the loaded patch
/// that the first copy of a chain of copies replaces (D59). None when that patch is not loaded:
/// such a copy runs last as `qm_glue` either way.
pub fn copy_base_mod(p: &ModProfile, raw: &RawLoad, glue: &GluePatchEntry) -> Option<String> {
    let older = |id: &str| {
        fixes(p)
            .find_map(|f| match &f.change {
                FixChange::AdjustedCopy { glue: g, .. } if g.id == id => Some(g),
                _ => None,
            })
            .or_else(|| p.glue_patches.iter().find(|g| g.id == id))
            .and_then(|g| g.replaces_patch_id.clone())
    };
    let mut id = glue.replaces_patch_id.clone()?;
    // Bounded: a loop of copies ends where it started.
    for _ in 0..32 {
        match older(&id) {
            Some(next) if next != id && next != glue.id => id = next,
            _ => break,
        }
    }
    raw.patches
        .iter()
        .find(|rp| plan::plan_keys(&rp.patch).contains(&id))
        .and_then(|rp| rp.patch.mod_id())
        .map(str::to_string)
}

/// History detail of a copy switched to run as its base mod (D58).
pub const RUN_LIKE_EVENT: &str = "Runs like its mod";
/// History detail of a copy switched back to run after every mod's patches.
pub const RUN_LAST_EVENT: &str = "Runs after every mod's patches";

/// Sets how the running adjusted copy `glue_id` runs (D58): `Base` runs it as its base mod at
/// the base's place, `Legacy` as `qm_glue` after every mod patch. A `Base` copy with no
/// `base_mod_id` gets the mod it runs as. The fix keeps its id and undo; a switch is logged in
/// its history. None when the profile is not migrated or has no running copy `glue_id`.
pub fn op_glue_run_as(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    glue_id: &str,
    run_as: RunAs,
) -> Option<String> {
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    let f = fixes(p).find(|f| !f.paused && is_copy(f, glue_id))?;
    let id = f.id.clone();
    let base_mod = match (&f.change, raw) {
        (FixChange::AdjustedCopy { glue, .. }, Some(raw)) if run_as == RunAs::Base => {
            copy_base_mod(p, raw, glue)
        }
        _ => None,
    };
    if let Some(f) = find_mut(p, &id) {
        let mut switched = None;
        if let FixChange::AdjustedCopy { glue, .. } = &mut f.change {
            let was = glue.runs_as_base();
            // Absent means Legacy, as old copies have it.
            glue.run_as = (run_as == RunAs::Base).then_some(RunAs::Base);
            if glue.base_mod_id.is_none() {
                glue.base_mod_id = base_mod;
            }
            let now = glue.runs_as_base();
            if was != now {
                switched = Some(if now { RUN_LIKE_EVENT } else { RUN_LAST_EVENT });
            }
        }
        // An existing event kind, so older versions still read the fix.
        if let Some(detail) = switched {
            f.log(FixEventKind::Rebound, detail);
        }
    }
    sync_legacy(p);
    Some(id)
}

/// Whether `f` is a copy "Run it like <mod>" switched to its base mod (D58), so it can go back
/// to running after every mod's patches. A copy made to run as its mod is not.
pub fn copy_switched_to_base(f: &ProfileFix) -> bool {
    let FixChange::AdjustedCopy { glue, .. } = &f.change else {
        return false;
    };
    glue.runs_as_base()
        && f.provenance
            .history
            .iter()
            .rev()
            .find(|e| {
                e.what == FixEventKind::Rebound
                    && (e.detail == RUN_LIKE_EVENT || e.detail == RUN_LAST_EVENT)
            })
            .is_some_and(|e| e.detail == RUN_LIKE_EVENT)
}

/// Tier 1 "Load `move_mod` before `before_mod`": the legacy writes to `mod_priorities` and
/// `user_load_after`, recorded as a `ModOrder` fix on a migrated profile (D36).
pub fn op_mod_order(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    move_mod: &str,
    before_mod: &str,
    priority: i32,
    provenance: Provenance,
) -> Option<String> {
    let prev_priority = p.mod_priorities.get(move_mod).copied();
    p.mod_priorities.insert(move_mod.to_string(), priority);
    let after = p.user_load_after.entry(before_mod.to_string()).or_default();
    let added = !after.iter().any(|m| m == move_mod);
    if added {
        after.push(move_mod.to_string());
    }
    if p.fixes_version < FIXES_VERSION {
        return None;
    }
    let fix = make_fix(
        None,
        new_fix_id(),
        format!(
            "Load {} before {}",
            mod_text(raw, move_mod),
            mod_text(raw, before_mod)
        ),
        FixChange::ModOrder {
            move_mod_id: move_mod.to_string(),
            before_mod_id: before_mod.to_string(),
            priority: Some(priority),
        },
        provenance,
        UndoRecord::RestoreModOrder {
            prev_priority,
            added_load_after: added,
        },
    );
    let id = fix.id.clone();
    push(p, fix);
    Some(id)
}

/// Whether a `ModOrder` fix still matches the profile: its pair is in `user_load_after` and
/// its priority is unchanged. True for other kinds.
pub fn mod_order_intact(p: &ModProfile, fix: &ProfileFix) -> bool {
    let FixChange::ModOrder {
        move_mod_id,
        before_mod_id,
        priority,
    } = &fix.change
    else {
        return true;
    };
    let pair = p
        .user_load_after
        .get(before_mod_id)
        .is_some_and(|v| v.contains(move_mod_id));
    let prio = priority.is_none_or(|v| p.mod_priorities.get(move_mod_id) == Some(&v));
    let needs_pair = !matches!(
        fix.undo,
        UndoRecord::RestoreModOrder {
            added_load_after: false,
            ..
        }
    );
    (pair || !needs_pair) && prio
}

/// What undoing `fix` would leave in place because the profile changed since (the user is
/// asked first). Empty when undo reverses everything.
pub fn undo_asks(p: &ModProfile, fix: &ProfileFix) -> Vec<String> {
    let FixChange::ModOrder {
        move_mod_id,
        before_mod_id,
        priority,
    } = &fix.change
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let UndoRecord::RestoreModOrder {
        added_load_after: true,
        ..
    } = fix.undo
    {
        let pair = p
            .user_load_after
            .get(before_mod_id)
            .is_some_and(|v| v.contains(move_mod_id));
        if !pair {
            out.push(format!(
                "{move_mod_id} no longer has to load before {before_mod_id} in Load order"
            ));
        }
    }
    if let Some(v) = priority {
        if p.mod_priorities.get(move_mod_id) != Some(v) {
            out.push(format!(
                "The priority of {move_mod_id} was changed in Load order; it stays as it is"
            ));
        }
    }
    out
}

/// Fixes made after `fix` on the same patch, newest last.
pub fn later_on_same_patch(p: &ModProfile, fix_id: &str) -> Vec<String> {
    let all: Vec<&ProfileFix> = fixes(p).collect();
    let Some(at) = all.iter().position(|f| f.id == fix_id) else {
        return Vec::new();
    };
    let Some(subject) = all.get(at).and_then(|f| f.subject()) else {
        return Vec::new();
    };
    all.iter()
        .skip(at + 1)
        .filter(|f| f.subject() == Some(subject))
        .map(|f| f.id.clone())
        .collect()
}

/// Undoes one fix by its `UndoRecord` (§7.5). Returns the removed fix.
pub fn undo(p: &mut ModProfile, raw: Option<&RawLoad>, fix_id: &str) -> Option<ProfileFix> {
    let fix = remove(p, fix_id)?;
    match (&fix.undo, &fix.change) {
        (UndoRecord::RestoreOrder(Some(prev)), _) => {
            let prov = Provenance {
                history: Vec::new(),
                ..fix.provenance.clone()
            };
            let refs = std::iter::once(&prev.patch)
                .chain(&prev.before)
                .chain(&prev.after)
                .map(|name| pin(raw, name, true))
                .filter(|r| r.fp.is_some())
                .collect();
            push(
                p,
                make_fix(
                    raw,
                    new_fix_id(),
                    order_label(raw, prev),
                    FixChange::Order {
                        rule: prev.clone(),
                        refs,
                    },
                    prov,
                    UndoRecord::Remove,
                ),
            );
        }
        (UndoRecord::RestorePriority(Some(prev)), FixChange::Priority { patch, .. }) => {
            let prov = Provenance {
                history: Vec::new(),
                ..fix.provenance.clone()
            };
            push(
                p,
                make_fix(
                    raw,
                    new_fix_id(),
                    format!("Priority {prev} for {}", patch_text(raw, &patch.id, false)),
                    FixChange::Priority {
                        patch: patch.clone(),
                        priority: *prev,
                    },
                    prov,
                    UndoRecord::Remove,
                ),
            );
        }
        (
            UndoRecord::RestoreAnchor {
                label,
                anchors,
                sites,
                replace_author_anchors,
            },
            FixChange::Anchor { patch, .. },
        ) => {
            let prov = Provenance {
                history: Vec::new(),
                ..fix.provenance.clone()
            };
            push(
                p,
                anchor_fix(
                    raw,
                    patch.clone(),
                    anchors.clone(),
                    sites.clone(),
                    *replace_author_anchors,
                    Some(label.clone()),
                    prov,
                    UndoRecord::Remove,
                ),
            );
        }
        (UndoRecord::RestoreApplyAs { label, mode }, FixChange::ApplyAs { patch, .. }) => {
            let prov = Provenance {
                history: Vec::new(),
                ..fix.provenance.clone()
            };
            push(
                p,
                make_fix(
                    raw,
                    new_fix_id(),
                    label.clone(),
                    FixChange::ApplyAs {
                        patch: patch.clone(),
                        mode: *mode,
                    },
                    prov,
                    UndoRecord::Remove,
                ),
            );
        }
        (
            UndoRecord::RestoreAssetChoice {
                label,
                winner,
                excludes,
            },
            FixChange::AssetChoice { key, .. },
        ) => {
            let prov = Provenance {
                history: Vec::new(),
                ..fix.provenance.clone()
            };
            push(
                p,
                make_fix(
                    raw,
                    new_fix_id(),
                    label.clone(),
                    FixChange::AssetChoice {
                        key: key.clone(),
                        winner: winner.clone(),
                        excludes: excludes.clone(),
                    },
                    prov,
                    UndoRecord::Remove,
                ),
            );
        }
        (UndoRecord::RestoreStack { .. }, FixChange::StackRule { .. })
        | (UndoRecord::RestoreHunk { .. }, FixChange::SkipHunk { .. }) => {
            restore_replaced(p, raw, &fix.undo, &fix.change, &fix.provenance);
        }
        // KeepDisable: only the copy goes. A disable that still exists stays; one the user
        // took away since is not made again.
        (
            UndoRecord::RestoreModOrder {
                prev_priority,
                added_load_after,
            },
            FixChange::ModOrder {
                move_mod_id,
                before_mod_id,
                priority,
            },
        ) => {
            if *added_load_after {
                if let Some(v) = p.user_load_after.get_mut(before_mod_id) {
                    v.retain(|m| m != move_mod_id);
                    if v.is_empty() {
                        p.user_load_after.remove(before_mod_id);
                    }
                }
            }
            if let Some(v) = priority {
                if p.mod_priorities.get(move_mod_id) == Some(v) {
                    match prev_priority {
                        Some(prev) => {
                            p.mod_priorities.insert(move_mod_id.clone(), *prev);
                        }
                        None => {
                            p.mod_priorities.remove(move_mod_id);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    sync_legacy(p);
    Some(fix)
}

/// Puts back the StackRule or SkipHunk fix `rec` records, and those in its `also`, for the
/// fix `change` that replaced them.
fn restore_replaced(
    p: &mut ModProfile,
    raw: Option<&RawLoad>,
    rec: &UndoRecord,
    change: &FixChange,
    provenance: &Provenance,
) {
    let prov = || Provenance {
        history: Vec::new(),
        ..provenance.clone()
    };
    match (rec, change) {
        (
            UndoRecord::RestoreStack {
                label,
                below,
                above,
                patch: other,
                also,
            },
            FixChange::StackRule {
                patch, site, side, ..
            },
        ) => {
            for r in also {
                restore_replaced(p, raw, r, change, provenance);
            }
            let patch = other.as_ref().unwrap_or(patch);
            push(
                p,
                make_fix(
                    raw,
                    new_fix_id(),
                    label.clone(),
                    FixChange::StackRule {
                        patch: patch.clone(),
                        site: site.clone(),
                        side: *side,
                        below: below.clone(),
                        above: above.clone(),
                    },
                    prov(),
                    UndoRecord::Remove,
                ),
            );
        }
        (
            UndoRecord::RestoreHunk {
                label,
                patch,
                hunk,
                also,
            },
            FixChange::SkipHunk { .. },
        ) => {
            for r in also {
                restore_replaced(p, raw, r, change, provenance);
            }
            push(
                p,
                make_fix(
                    raw,
                    new_fix_id(),
                    label.clone(),
                    FixChange::SkipHunk {
                        patch: patch.clone(),
                        hunk: hunk.clone(),
                    },
                    prov(),
                    UndoRecord::Remove,
                ),
            );
        }
        _ => {}
    }
}

/// Renames patch `from` to `to` everywhere in a change.
fn rename(change: &mut FixChange, from: &str, to: &str) {
    let swap = |s: &mut String| {
        if s == from {
            *s = to.to_string();
        }
    };
    match change {
        FixChange::Disable { patch }
        | FixChange::Priority { patch, .. }
        | FixChange::Anchor { patch, .. }
        | FixChange::ApplyAs { patch, .. } => swap(&mut patch.id),
        FixChange::SkipHunk { patch, hunk } => {
            swap(&mut patch.id);
            if let Some(k) = hunk.keep.as_mut() {
                swap(k);
            }
        }
        FixChange::StackRule {
            patch,
            below,
            above,
            ..
        } => {
            swap(&mut patch.id);
            for r in below.iter_mut().chain(above.iter_mut()) {
                swap(&mut r.id);
            }
        }
        FixChange::Order { rule, refs } => {
            swap(&mut rule.patch);
            rule.before
                .iter_mut()
                .chain(rule.after.iter_mut())
                .for_each(swap);
            refs.iter_mut().for_each(|r| swap(&mut r.id));
        }
        FixChange::AdjustedCopy { glue, base, .. } => {
            if let Some(b) = glue.replaces_patch_id.as_mut() {
                swap(b);
            }
            if let Some(b) = base.as_mut() {
                swap(&mut b.id);
            }
        }
        FixChange::ModOrder { .. } | FixChange::AssetChoice { .. } => {}
    }
}

fn bind_kind(change: &FixChange) -> plan::BindKind {
    match change {
        FixChange::Disable { .. } => plan::BindKind::Disable,
        FixChange::Order { .. } => plan::BindKind::Order,
        FixChange::Priority { .. } => plan::BindKind::Priority,
        FixChange::Anchor { .. } => plan::BindKind::Anchor,
        FixChange::ApplyAs { .. } => plan::BindKind::ApplyAs,
        FixChange::SkipHunk { .. } => plan::BindKind::Hunk,
        FixChange::StackRule { .. } => plan::BindKind::Stack,
        FixChange::AdjustedCopy { .. }
        | FixChange::ModOrder { .. }
        | FixChange::AssetChoice { .. } => plan::BindKind::Glue,
    }
}

/// The current id of raw patch `ix`.
fn raw_id(raw: &RawLoad, ix: usize) -> Option<String> {
    raw.patches
        .get(ix)
        .and_then(|rp| rp.patch.id())
        .map(str::to_string)
}

/// "Accept" (§10): every ref of the fix is pinned again to the patch it binds to now, a
/// re-bound id is followed, and the snapshots are taken again. False when there is no such
/// fix.
pub fn accept(p: &mut ModProfile, raw: &RawLoad, fix_id: &str) -> bool {
    let Some(mut fix) = find(p, fix_id) else {
        return false;
    };
    let kind = bind_kind(&fix.change);
    let mut fps = Fingerprints::new(raw, FpMode::All);
    let mut renames = Vec::new();
    let mut bound = None;
    for r in fix.refs_mut() {
        let b = plan::bind(r, kind, &mut fps);
        let [t] = b.targets.as_slice() else {
            continue;
        };
        bound = Some(*t);
        if let Some(to) = b.new_id.clone().or_else(|| {
            matches!(b.status, qm_core::handshake::BindingStatus::Rebound { .. })
                .then(|| raw_id(raw, *t))
                .flatten()
        }) {
            if to != r.id {
                renames.push((r.id.clone(), to));
            }
        }
        r.fp = fps.get(*t).cloned();
    }
    for (from, to) in &renames {
        rename(&mut fix.change, from, to);
    }
    // A skipped edit is pinned to the patch as it is now too (D49).
    if let (FixChange::SkipHunk { hunk, .. }, Some(t)) = (&mut fix.change, bound) {
        repin_hunk(raw, t, hunk);
    }
    fix.snapshots = retake_snapshots(&fix, raw, &renames);
    let detail = renames
        .iter()
        .map(|(a, b)| format!("{a} is now {b}"))
        .collect::<Vec<_>>()
        .join("; ");
    fix.log(FixEventKind::Accepted, detail);
    // Stamped again against the game as it is now (D74).
    fix.fresh = true;
    if let Some(slot) = find_mut(p, fix_id) {
        *slot = fix;
    }
    sync_legacy(p);
    true
}

/// Points the fix at `patch_id` instead of its current patch (the user picked one of several).
/// Err when the fix or the patch is not there.
pub fn rebind(
    p: &mut ModProfile,
    raw: &RawLoad,
    fix_id: &str,
    patch_id: &str,
) -> Result<(), String> {
    let Some(mut fix) = find(p, fix_id) else {
        return Err("That fix is already gone.".to_string());
    };
    let Some(from) = fix.subject().map(str::to_string) else {
        return Err("This fix does not name a patch.".to_string());
    };
    // A D29 duplicate keeps its old id as a name, so the patch that kept the id is named
    // by both: the exact id picks it.
    let found = hits(raw, patch_id, false);
    let exact: Vec<usize> = found
        .iter()
        .copied()
        .filter(|&i| raw_id(raw, i).as_deref() == Some(patch_id))
        .collect();
    let target = match (found.as_slice(), exact.as_slice()) {
        ([one], _) | (_, [one]) => *one,
        ([], _) => return Err(format!("No loaded patch is called '{patch_id}'.")),
        _ => return Err(format!("'{patch_id}' names more than one patch.")),
    };
    let to = raw_id(raw, target).unwrap_or_else(|| patch_id.to_string());
    rename(&mut fix.change, &from, &to);
    // Pinned to the picked patch itself: its id may name others too.
    let mut fps = Fingerprints::new(raw, FpMode::All);
    let fresh = PatchRef {
        id: to.clone(),
        mod_id: raw
            .patches
            .get(target)
            .and_then(|rp| raw.mods.get(rp.mod_ix as usize))
            .map(|m| m.id.clone())
            .unwrap_or_default(),
        fp: fps.get(target).cloned(),
    };
    let mut pinned = false;
    for r in fix.refs_mut() {
        if r.id == to {
            *r = fresh.clone();
            pinned = true;
        }
    }
    if let (false, FixChange::Order { refs, .. }) = (pinned, &mut fix.change) {
        refs.push(fresh);
    }
    if let FixChange::SkipHunk { hunk, .. } = &mut fix.change {
        repin_hunk(raw, target, hunk);
    }
    // The old patch's snapshot is not carried over to the new one.
    fix.snapshots = retake_snapshots(&fix, raw, &[]);
    fix.log(FixEventKind::Rebound, format!("{from} is now {to}"));
    fix.fresh = true;
    if let Some(slot) = find_mut(p, fix_id) {
        *slot = fix;
    }
    sync_legacy(p);
    Ok(())
}

#[cfg(test)]
#[path = "fixes/tests.rs"]
mod tests;
