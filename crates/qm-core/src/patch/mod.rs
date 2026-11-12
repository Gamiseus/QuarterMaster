#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub use copy::CopyPatch;
pub use module::ModulePatch;
pub use pattern::PatternPatch;
pub use regex::RegexPatch;

pub mod anchor;
pub mod compat;
pub mod copy;
pub mod edits;
pub mod faction;
pub mod fingerprint;
pub mod linemap;
pub mod loader;
pub mod menu;
pub mod module;
pub mod order;
pub mod origin;
pub mod parity;
pub mod pattern;
pub mod plan;
pub mod regex;
pub mod stack;
pub mod table;
pub mod vars;

#[cfg(test)]
mod menu_runtime_tests;
#[cfg(test)]
mod input_runtime_tests;

pub type Priority = i32;

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFailurePolicy {
    #[default]
    Warn,
    SkipPatch,
    SkipMod,
    Abort,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Manifest {
    pub version: String,
    // Does nothing, kept for legacy compat
    #[serde(default)]
    pub dump_lua: bool,
    #[serde(default)]
    pub priority: Priority,
}

// Represents a single .toml file after deserialization.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatchFile {
    pub manifest: Manifest,
    #[serde(default)]
    pub patches: Vec<Patch>,

    // A table of variable name = value bindings. These are interpolated
    // into injected source code as the *last* step in the patching process.
    #[serde(default)]
    pub vars: HashMap<String, String>,

    /// Compatibility mod changes to other mods' patches (Phase C §3.10). An entry that does
    /// not parse is kept as invalid and reported; the file still loads. Older loaders warn
    /// about the unknown key and apply the patches as authored.
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "crate::lenient::lenient_list"
    )]
    pub overrides: Vec<crate::lenient::Lenient<compat::Override>>,

    /// `[sprites.<image global>]`: the size an image is shown at, whatever its pixel size.
    /// An older loader warns about the unknown key and shows images at their pixel size.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sprites: std::collections::BTreeMap<String, SpriteRule>,
}

/// One `[sprites.<name>]` entry. Exactly one key is used: `size`, `scale` or `like`.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct SpriteRule {
    /// Show the image at this width and height, in the game's pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    /// Show the image at this fraction of its own pixel size (0.25: a 400x400 file shows
    /// at 100x100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// Show the image at the size another image global is shown at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum Patch {
    // A patch which applies some change to a series of line(s) after a line with a match
    // to the provided pattern has been found.
    Pattern(PatternPatch),
    Regex(RegexPatch),
    Copy(CopyPatch),
    Module(ModulePatch),
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Target {
    Single(String),
    Multi(Vec<String>),
}

impl Target {
    pub fn primary(&self) -> &str {
        match self {
            Target::Single(s) => s.as_str(),
            Target::Multi(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
        }
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InsertPosition {
    At,
    Before,
    After,
}

/// What `apply_in` reads besides the rope (plan 2e §4.3): the buffer's `LineMap` before the
/// patch, how to stack code at shared lines, and the patch's rungs.
pub(crate) struct ApplyCtx<'a> {
    pub lines: &'a linemap::LineMap,
    /// `None` places code as today (Legacy).
    pub stack: Option<StackCtx<'a>>,
    /// This patch's index in the table (`linemap::NO_INDEX` without a table).
    pub me: u32,
    /// This patch's anchor ladder; `None` uses its own matches only.
    pub ladder: Option<&'a anchor::Ladder>,
    /// Per patch index, the records it committed in this buffer.
    pub committed: &'a [Vec<u32>],
    /// The buffer before any patch (A's own places, §6.1).
    pub original: Option<&'a anchor::OriginalLines<'a>>,
    /// The plan's `line_rules_v2` (D50): rungs read B's lines by owner.
    pub line_rules: bool,
    /// This `at` patch's normalized form from `prepare` (D47); `None` computes it when needed.
    pub hunks: Option<&'a edits::Normalized>,
    /// This patch's skipped edits (D49).
    pub skips: &'a [edits::SkipAt],
}

impl<'a> ApplyCtx<'a> {
    /// Today's behaviour, for `apply` callers with no table.
    pub fn legacy(lines: &'a linemap::LineMap) -> Self {
        ApplyCtx {
            lines,
            stack: None,
            me: linemap::NO_INDEX,
            ladder: None,
            committed: &[],
            original: None,
            line_rules: false,
            hunks: None,
            skips: &[],
        }
    }
}

/// Stacking for one patch (§5.3). Built when the mode is not Legacy, or when the patch has
/// stack rules (D55), or when a patch of the table has an author hint (D57).
pub(crate) struct StackCtx<'a> {
    pub mode: crate::handshake::StackMode,
    /// The stack rules that name this patch, bound in `prepare`.
    pub rules: &'a [stack::StackRuleBound],
    /// This patch's author hint (D57).
    pub hint: Option<stack::StackHint>,
    pub graph: &'a plan::ModGraph,
    pub mods: &'a [plan::ModMeta],
    /// This patch's mod, an index into `mods` (`linemap::NO_INDEX` when not loaded).
    pub mod_ix: u32,
    /// The record id and mod id of a patch, by patch index.
    pub names: &'a dyn Fn(u32) -> (String, String),
}

impl StackCtx<'_> {
    /// Whether mod `a` depends on mod `b` (indexes into `mods`; unknown mods never do).
    pub fn depends_on(&self, a: u32, b: u32) -> bool {
        match (self.mods.get(a as usize), self.mods.get(b as usize)) {
            (Some(a), Some(b)) => self.graph.depends_on(&a.id, &b.id),
            _ => false,
        }
    }
}

/// `apply_in`'s result: the debug entry (its byte regions are the edits in bytes, in order),
/// the same edits in lines, whether a match touched QuarterMaster's own lines, and the sites
/// stacking placed away from today's slot.
pub(crate) struct ApplyOutcome {
    pub entry: crate::dump::ByteDebugEntry,
    pub edits: Vec<linemap::Edit>,
    pub touches_builtin: bool,
    pub stack_moves: Vec<crate::handshake::StackMove>,
    /// How the rungs resolved (only for a patch with a ladder).
    pub anchoring: Option<crate::handshake::AnchorReport>,
}

impl ApplyOutcome {
    /// An outcome that changed nothing.
    pub fn unchanged(entry: crate::dump::ByteDebugEntry) -> Self {
        ApplyOutcome {
            entry,
            edits: Vec::new(),
            touches_builtin: false,
            stack_moves: Vec::new(),
            anchoring: None,
        }
    }
}

impl Patch {
    pub fn type_name(&self) -> &'static str {
        match self {
            Patch::Module(_) => "Module",
            Patch::Copy(_) => "Copy",
            Patch::Pattern(_) => "Pattern",
            Patch::Regex(_) => "Regex",
        }
    }

    pub fn id(&self) -> Option<&str> {
        match self {
            Patch::Module(m) => m.id.as_deref(),
            Patch::Copy(c) => c.id.as_deref(),
            Patch::Pattern(p) => p.id.as_deref(),
            Patch::Regex(r) => r.id.as_deref(),
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Patch::Module(m) => Some(m.name.as_str()),
            Patch::Copy(c) => c.name.as_deref(),
            Patch::Pattern(p) => p.name.as_deref(),
            Patch::Regex(r) => r.name.as_deref(),
        }
    }

    pub fn description(&self) -> Option<&str> {
        match self {
            Patch::Module(_) => None,
            Patch::Copy(_) => None,
            Patch::Pattern(p) => p.description.as_deref(),
            Patch::Regex(r) => r.description.as_deref(),
        }
    }

    pub fn requires(&self) -> &[String] {
        match self {
            Patch::Module(_) => &[],
            Patch::Copy(_) => &[],
            Patch::Pattern(p) => &p.requires,
            Patch::Regex(r) => &r.requires,
        }
    }

    pub fn mod_id(&self) -> Option<&str> {
        match self {
            Patch::Module(m) => m.mod_id.as_deref(),
            Patch::Copy(c) => c.mod_id.as_deref(),
            Patch::Pattern(p) => p.mod_id.as_deref(),
            Patch::Regex(r) => r.mod_id.as_deref(),
        }
    }

    pub fn priority_override(&self) -> Option<Priority> {
        match self {
            Patch::Module(m) => m.priority,
            Patch::Copy(c) => c.priority,
            Patch::Pattern(p) => p.priority,
            Patch::Regex(r) => r.priority,
        }
    }

    pub fn on_failure(&self) -> OnFailurePolicy {
        match self {
            Patch::Module(m) => m.on_failure,
            Patch::Copy(c) => c.on_failure,
            Patch::Pattern(p) => p.on_failure,
            Patch::Regex(r) => r.on_failure,
        }
    }

    pub fn is_disabled(&self) -> bool {
        match self {
            Patch::Module(m) => m.disabled,
            Patch::Copy(c) => c.disabled,
            Patch::Pattern(p) => p.disabled,
            Patch::Regex(r) => r.disabled,
        }
    }

    pub fn set_disabled(&mut self, disabled: bool) {
        match self {
            Patch::Module(m) => m.disabled = disabled,
            Patch::Copy(c) => c.disabled = disabled,
            Patch::Pattern(p) => p.disabled = disabled,
            Patch::Regex(r) => r.disabled = disabled,
        }
    }

    /// The author's `stack` key (D57), for pattern and regex patches; a bad value is none.
    pub fn stack_hint(&self) -> Option<stack::StackHint> {
        let hint = match self {
            Patch::Pattern(p) => p.stack.as_ref(),
            Patch::Regex(r) => r.stack.as_ref(),
            Patch::Module(_) | Patch::Copy(_) => None,
        };
        hint.and_then(crate::lenient::Lenient::ok).copied()
    }

    /// Replaces the patch id (the loader's D29 suffix).
    pub fn set_id(&mut self, id: String) {
        match self {
            Patch::Module(m) => m.id = Some(id),
            Patch::Copy(c) => c.id = Some(id),
            Patch::Pattern(p) => p.id = Some(id),
            Patch::Regex(r) => r.id = Some(id),
        }
    }

    /// The profile and compat data of this patch.
    pub fn extra(&self) -> &plan::PatchExtra {
        match self {
            Patch::Module(m) => &m.qm,
            Patch::Copy(c) => &c.qm,
            Patch::Pattern(p) => &p.qm,
            Patch::Regex(r) => &r.qm,
        }
    }

    pub fn extra_mut(&mut self) -> &mut plan::PatchExtra {
        match self {
            Patch::Module(m) => &mut m.qm,
            Patch::Copy(c) => &mut c.qm,
            Patch::Pattern(p) => &mut p.qm,
            Patch::Regex(r) => &mut r.qm,
        }
    }

    /// Content hashes of this patch (Phase C §3.7). `mod_dir` is the mod's own folder or zip
    /// (what `{{qm_hack:patch_dir}}` became); `source` and `ordinal` say where it sits.
    pub fn fingerprint(
        &self,
        mod_dir: &std::path::Path,
        source: &str,
        ordinal: u32,
    ) -> fingerprint::PatchFingerprint {
        fingerprint::fingerprint_of(self, mod_dir, source, ordinal)
    }

    /// The normalized text of this patch for drift diffs (D41), each field capped at 64 KiB.
    pub fn snapshot(&self, mod_dir: &std::path::Path) -> fingerprint::PatchSnapshot {
        fingerprint::snapshot_of(self, mod_dir)
    }

    pub fn can_apply_to_target(&self, target: &str) -> bool {
        match self {
            Patch::Module(m) => m.before.as_deref() == Some(target),
            Patch::Copy(c) => c.target.can_apply(target),
            Patch::Pattern(p) => p.target.can_apply(target),
            Patch::Regex(r) => r.target.can_apply(target),
        }
    }

    pub fn expected_count(&self) -> Option<usize> {
        match self {
            Patch::Module(_) => Some(1),
            Patch::Copy(_) => Some(1),
            Patch::Pattern(p) => p.times,
            Patch::Regex(r) => r.times,
        }
    }

    pub fn primary_target(&self) -> &str {
        match self {
            Patch::Module(m) => m.before.as_deref().unwrap_or("main.lua"),
            Patch::Copy(c) => c.target.primary(),
            Patch::Pattern(p) => p.target.primary(),
            Patch::Regex(r) => r.target.primary(),
        }
    }

    pub fn set_metadata(
        &mut self,
        mod_id: String,
        fallback_id: String,
        comment_name: Option<String>,
    ) {
        match self {
            Patch::Module(m) => {
                m.mod_id = Some(mod_id);
                if m.id.is_none() {
                    m.id = Some(fallback_id);
                }
            }
            Patch::Copy(c) => {
                c.mod_id = Some(mod_id);
                if c.id.is_none() {
                    c.id = Some(fallback_id.clone());
                }
                if c.name.is_none() {
                    c.name = comment_name.or(Some(fallback_id));
                }
            }
            Patch::Pattern(p) => {
                p.mod_id = Some(mod_id);
                if p.id.is_none() {
                    p.id = Some(fallback_id.clone());
                }
                if p.name.is_none() {
                    p.name = comment_name.or(Some(fallback_id));
                }
            }
            Patch::Regex(r) => {
                r.mod_id = Some(mod_id);
                if r.id.is_none() {
                    r.id = Some(fallback_id.clone());
                }
                if r.name.is_none() {
                    r.name = comment_name.or(Some(fallback_id));
                }
            }
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Patch::Module(m) => format!(
                "Module: '{}' (before: {:?}, load_now: {})",
                m.name, m.before, m.load_now
            ),
            Patch::Copy(c) => format!("Copy ({:?}) -> target: {:?}", c.position, c.target),
            Patch::Pattern(p) => {
                let snippet = p.pattern.lines().next().unwrap_or("").trim();
                format!(
                    "Pattern ({:?}) -> target: {:?} ['{}']",
                    p.position, p.target, snippet
                )
            }
            Patch::Regex(r) => {
                let snippet = r.pattern.lines().next().unwrap_or("").trim();
                format!(
                    "Regex ({:?}) -> target: {:?} ['{}']",
                    r.position, r.target, snippet
                )
            }
        }
    }
}

#[cfg(test)]
mod publish_table_tests {
    use super::PatchFile;

    /// The `[publish]` table is for the site builder; the game must still load the file.
    #[test]
    fn qm_toml_with_publish_table_still_parses() {
        let raw = "[manifest]\nversion = \"1.0.0\"\n\n[publish]\nrepository = \"Gamiseus/MnB-Mods\"\nsummary = \"Core fixes.\"\n";
        let f: PatchFile = toml::from_str(raw).expect("qm.toml with [publish] parses");
        assert_eq!(f.manifest.version, "1.0.0");
        assert!(f.patches.is_empty());
    }
}
