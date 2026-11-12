#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use crate::dump::PatchRegion;
use crate::lenient::Lenient;
use crate::patch::anchor::{Rung, SiteKey};
use crate::patch::edits::{ApplyAs, HunkKey};
use crate::patch::fingerprint::PatchRef;
use crate::patch::linemap::Side;
use crate::patch::order::OrderRule;
use crate::patch::{OnFailurePolicy, Patch, Priority};

/// Written by the QuarterMaster Launcher to `Mods/qm/launch-config.json` before launching
/// `mnb.exe` (via `steam://rungameid/1391530` or Direct Launch).
/// Allows `winmm.dll` to receive launch flags without passing custom CLI args through Steam.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchConfig {
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    #[serde(default)]
    pub vanilla_mode: bool,
    #[serde(default)]
    pub disable_console: bool,
    #[serde(default)]
    pub dump_all: bool,
    /// Debug mode: the console window, every loaded file dumped and the dump commands. Also
    /// turned on by `-debug` on the game's command line or `QM_DEBUG=1`.
    #[serde(default)]
    pub debug: bool,
    /// Unlocks the game's own developer commands. Also `-dev` or `QM_DEV=1`.
    #[serde(default)]
    pub dev_commands: bool,
    /// Optional LÖVE `t.identity` override for isolated save sandboxing away from Steam Cloud.
    pub save_identity_override: Option<String>,
    /// Active `mod_id -> version` map for save fingerprinting and crash reports.
    #[serde(default)]
    pub active_mods: HashMap<String, String>,
    /// Whether live dev hot-reload watching (`Mods/qm/hot-reload.signal`) is enabled.
    #[serde(default)]
    pub dev_hot_reload: bool,
    /// A new id per launch (D67). The DLL copies it into patch-report.json and crash reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// What the game stamps into each save it loads (D85). A broken value is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_stamp: Option<Lenient<SaveStamp>>,
    /// Save content each installed mod declares, by mod id, enabled or not (D87).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub save_content: BTreeMap<String, Lenient<SaveContent>>,
    /// Set only for a diagnostic test run (D89, D91). A broken value means no diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Lenient<Diagnostics>>,
}

/// The data behind a save's `_qm_meta` stamp (D85).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveStamp {
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub profile_name: String,
    /// Enabled `mod_id -> version`.
    #[serde(default)]
    pub mods: BTreeMap<String, String>,
    /// `mod_id -> save fields` the mod writes and the game never sets (D84).
    #[serde(default)]
    pub fields: BTreeMap<String, Vec<String>>,
}

/// Save content a mod declares in mod.json, so a save still loads without the mod (D87).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveContent {
    /// The mod's weapon tag -> the vanilla weapon it becomes when the mod is off.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weapons: BTreeMap<String, String>,
}

/// What a diagnostic test run watches (D89, D90).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics {
    /// Global names whose changes the probes record.
    #[serde(default)]
    pub globals: Vec<String>,
    /// Global functions whose callers are counted.
    #[serde(default)]
    pub hooks: Vec<String>,
}

/// Whether the game was started with `-name` or `--name`, or with the environment variable
/// `env` set to 1. `args` is the game's command line.
pub fn launch_flag(args: &[String], name: &str, env: &str) -> bool {
    args.iter()
        .any(|a| a.strip_prefix("--").or_else(|| a.strip_prefix('-')) == Some(name))
        || std::env::var(env).map(|v| v == "1").unwrap_or(false)
}

impl LaunchConfig {
    /// The save stamp; None when absent or invalid.
    pub fn save_stamp(&self) -> Option<&SaveStamp> {
        self.save_stamp.as_ref().and_then(Lenient::ok)
    }

    /// Lua source of the table the startup code's save check reads as `qm.save_info`.
    /// `stamp` is the save stamp (D85). `weapons` is weapon tag -> base weapon for the
    /// declared weapons of mods that are not loaded, and `loaded_weapons` the declared tags
    /// of mods that are (D87); `loaded` says whether a mod id is loaded. Empty when there is
    /// nothing to pass.
    pub fn save_info_lua(&self, loaded: &dyn Fn(&str) -> bool) -> String {
        use crate::patch::faction::lua_quote;
        let map = |entries: Vec<String>| format!("{{{}}}", entries.join(","));
        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = self.save_stamp() {
            parts.push(format!("stamp={}", Self::stamp_lua(s)));
        }
        let mut gone: BTreeMap<&str, &str> = BTreeMap::new();
        let mut here: BTreeSet<&str> = BTreeSet::new();
        for (id, content) in &self.save_content {
            let Some(content) = content.ok() else { continue };
            for (tag, base) in &content.weapons {
                if loaded(id) {
                    here.insert(tag);
                } else {
                    gone.insert(tag, base);
                }
            }
        }
        // A tag a loaded mod also declares is not gone.
        gone.retain(|tag, _| !here.contains(tag));
        if !gone.is_empty() {
            let entries = gone
                .iter()
                .map(|(tag, base)| format!("[{}]={}", lua_quote(tag), lua_quote(base)))
                .collect();
            parts.push(format!("weapons={}", map(entries)));
        }
        if !here.is_empty() {
            let entries = here.iter().map(|t| format!("[{}]=true", lua_quote(t))).collect();
            parts.push(format!("loaded_weapons={}", map(entries)));
        }
        if parts.is_empty() {
            return String::new();
        }
        format!("return {}", map(parts))
    }

    fn stamp_lua(s: &SaveStamp) -> String {
        use crate::patch::faction::lua_quote;
        let map = |entries: Vec<String>| format!("{{{}}}", entries.join(","));
        let mods = s
            .mods
            .iter()
            .map(|(id, version)| format!("[{}]={}", lua_quote(id), lua_quote(version)))
            .collect();
        let fields = s
            .fields
            .iter()
            .map(|(id, list)| {
                let names = list.iter().map(|f| lua_quote(f)).collect();
                format!("[{}]={}", lua_quote(id), map(names))
            })
            .collect();
        format!(
            "{{profile_id={},profile_name={},mods={},fields={}}}",
            lua_quote(&s.profile_id),
            lua_quote(&s.profile_name),
            map(mods),
            map(fields)
        )
    }

    /// Lua source of what a diagnostic test run watches, read by the startup code as
    /// `qm.diagnostics`. Empty for a normal launch.
    pub fn diagnostics_lua(&self) -> String {
        use crate::patch::faction::lua_quote;
        let Some(d) = self.diagnostics() else {
            return String::new();
        };
        let list = |names: &[String]| {
            let quoted: Vec<String> = names.iter().map(|n| lua_quote(n)).collect();
            format!("{{{}}}", quoted.join(","))
        };
        format!("return {{globals={},hooks={}}}", list(&d.globals), list(&d.hooks))
    }

    /// The diagnostics of a test run; None when absent, invalid or empty.
    pub fn diagnostics(&self) -> Option<&Diagnostics> {
        self.diagnostics
            .as_ref()
            .and_then(Lenient::ok)
            .filter(|d| !d.globals.is_empty() || !d.hooks.is_empty())
    }

    /// One line per value that could not be read and was ignored.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(e) = self.save_stamp.as_ref().and_then(Lenient::error) {
            out.push(format!(
                "launch-config.json save_stamp is invalid, saves are not stamped: {e}"
            ));
        }
        for (id, c) in &self.save_content {
            if let Some(e) = c.error() {
                out.push(format!(
                    "launch-config.json save_content of '{id}' is invalid and ignored: {e}"
                ));
            }
        }
        if let Some(e) = self.diagnostics.as_ref().and_then(Lenient::error) {
            out.push(format!("launch-config.json diagnostics is invalid and ignored: {e}"));
        }
        out
    }

    /// Reads `<mod_dir>/qm/launch-config.json`. Ok(None) when there is no file; Err when it
    /// cannot be read or parsed (the DLL then loads no mods and keeps its safety edits, D65).
    pub fn load_from_mod_dir(mod_dir: &Path) -> Result<Option<Self>, String> {
        read_json(&mod_dir.join("qm").join("launch-config.json"), "launch-config.json")
    }

    pub fn save_to_mod_dir(&self, mod_dir: &Path) -> Result<(), String> {
        let qm_dir = mod_dir.join("qm");
        std::fs::create_dir_all(&qm_dir)
            .map_err(|e| format!("Failed to create {}: {e}", qm_dir.display()))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize launch-config.json: {e}"))?;
        write_file_atomic(&qm_dir.join("launch-config.json"), json.as_bytes())
    }
}

/// The newest plan schema this version reads (4: the Phase E fields). Older ones read as they
/// are; a newer one still applies its known fields.
pub const PLAN_SCHEMA_VERSION: u32 = 4;

/// The schema of a plan with no Phase E field. A plan is written as 4 only when it has one
/// (`asset_choices`, `asset_exclusions`, `factions`), so a Phase D build reads other plans
/// without a warning.
pub const PLAN_SCHEMA_PHASE_D: u32 = 3;

fn plan_schema_one() -> u32 {
    1
}

/// Written by the QuarterMaster Launcher to `Mods/qm/load-order.json`.
/// Specifies the exact resolved mod load order, priority overrides, and any profile-local
/// conflict-resolution glue patches.
///
/// `profile_id`, `generated_at` and `enabled_mods` are strict: a bad one is a hard error and
/// the DLL loads no mods (D16). Every list and map entry is lenient: a bad one is skipped
/// and reported, and the rest of the plan applies (D17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadOrderPlan {
    /// 1 when absent (written before Phase C).
    #[serde(default = "plan_schema_one")]
    pub schema_version: u32,
    /// "quartermaster <version>".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    pub profile_id: String,
    pub generated_at: String,
    /// Ordered list of enabled mod folder names / IDs in ascending execution order.
    pub enabled_mods: Vec<String>,
    /// Per-mod priority overrides (`mod_folder_or_id -> priority`).
    #[serde(default)]
    pub mod_priorities: HashMap<String, Lenient<Priority>>,
    /// Per-patch priority overrides (`patch_id -> priority`).
    #[serde(default)]
    pub patch_priorities: HashMap<String, Lenient<Priority>>,
    /// Explicitly disabled patch IDs (e.g. from a user conflict winner selection).
    #[serde(default)]
    pub disabled_patches: Vec<Lenient<String>>,
    /// Profile-local glue patches generated by the 1-Click Re-Anchoring Conflict Resolver.
    #[serde(default)]
    pub glue_patches: Vec<Lenient<GluePatchEntry>>,
    /// Profile patch order rules; they beat the authors' `before`/`after` (see `patch::order`).
    #[serde(default)]
    pub patch_order: Vec<Lenient<OrderRule>>,
    /// Absent or invalid means Legacy (D2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stacking: Option<Lenient<StackMode>>,
    /// Key: a legacy entry as written in the fields above. An entry with a ref binds to one
    /// patch (§7.1); one without matches by name as before.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub refs: BTreeMap<String, Lenient<PatchRef>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchor_fixes: Vec<Lenient<AnchorFix>>,
    /// "disable:<id>" | "order:<patch>" | "priority:<id>" | "glue:<id>" | "anchor:<id>" |
    /// "apply_as:<id>" -> fix id. Stack rules carry their own fix id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fix_of: BTreeMap<String, Lenient<String>>,
    /// The Phase D line rules (D50 owner-based rungs, D51 implicit anchors). Absent, false or
    /// invalid: today's rules, so an older plan keeps them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_rules_v2: Option<Lenient<bool>>,
    /// Patch name -> how that `at` patch runs (D46): `"edits"` or `"literal"`. Beats a compat
    /// mod and the author (D13).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub apply_as: BTreeMap<String, Lenient<ApplyAs>>,
    /// Edits of `at` patches the profile skips, so another patch's change to the same lines
    /// stays (D49). The patch runs as separate edits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_hunks: Vec<Lenient<SkipHunkFix>>,
    /// Profile rules for the order of blocks at one line (D55). They act in every stacking
    /// mode and beat compat rules and the dependency order (D56).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stack_rules: Vec<Lenient<StackRuleFix>>,
    /// Asset group or slot key -> its winner: a mod id, `faction:<id>` or `vanilla` (D76,
    /// D78). Absent: the last source to load an asset wins, as before.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub asset_choices: BTreeMap<String, Lenient<String>>,
    /// What the choices leave out, worked out by the launcher against the game's files: a
    /// mod id -> its asset files (`assets/...` paths) that are not loaded, and
    /// `faction:<id>` -> the game globals and voice files that faction leaves alone (D78). The
    /// game acts on this list; `asset_choices` says why.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub asset_exclusions: BTreeMap<String, Lenient<Vec<String>>>,
    /// Side (`allied`, `axis`) -> the faction picked for it (D82). Absent: priority decides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub factions: BTreeMap<String, Lenient<String>>,
}

impl Default for LoadOrderPlan {
    fn default() -> Self {
        Self {
            schema_version: plan_schema_one(),
            generator: None,
            profile_id: String::new(),
            generated_at: String::new(),
            enabled_mods: Vec::new(),
            mod_priorities: HashMap::new(),
            patch_priorities: HashMap::new(),
            disabled_patches: Vec::new(),
            glue_patches: Vec::new(),
            patch_order: Vec::new(),
            stacking: None,
            refs: BTreeMap::new(),
            anchor_fixes: Vec::new(),
            fix_of: BTreeMap::new(),
            line_rules_v2: None,
            apply_as: BTreeMap::new(),
            skip_hunks: Vec::new(),
            stack_rules: Vec::new(),
            asset_choices: BTreeMap::new(),
            asset_exclusions: BTreeMap::new(),
            factions: BTreeMap::new(),
        }
    }
}

/// How patches that share an anchor line are stacked.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StackMode {
    #[default]
    Legacy,
    Dependencies,
    LoadOrder,
}

/// A profile fix that gives patch A new anchors.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AnchorFix {
    pub fix_id: String,
    pub patch: PatchRef,
    pub anchors: Vec<Rung>,
    #[serde(default)]
    pub sites: Vec<SiteKey>,
    #[serde(default)]
    pub replace_author_anchors: bool,
}

/// A profile fix that orders two blocks at one line (D55): at the game's line `site`, on
/// `side`, `patch`'s block goes below `below`'s block, or above `above`'s.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StackRuleFix {
    pub fix_id: String,
    pub patch: PatchRef,
    pub site: SiteKey,
    pub side: Side,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub below: Option<PatchRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub above: Option<PatchRef>,
}

/// A profile fix that skips one edit of an `at` patch (D49).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SkipHunkFix {
    pub fix_id: String,
    pub patch: PatchRef,
    pub hunk: HunkKey,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GluePatchEntry {
    pub id: String,
    pub name: String,
    pub replaces_patch_id: Option<String>,
    pub priority: Priority,
    pub patch: Patch,
    /// The mod of the replaced patch. A `Legacy` copy runs as `qm_glue` (D18); a `Base`
    /// copy runs as this mod (D58).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub base_mod_id: Option<String>,
    /// Where the copy runs and as which mod (D58). Absent means `Legacy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub run_as: Option<RunAs>,
    /// The copy's pattern lines match only their own text: `*` and `?` are not wildcards
    /// (D59). Old copies keep wildcards.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub exact: bool,
}

/// How an adjusted copy runs (D58).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunAs {
    /// As `qm_glue`, after every mod patch (D18).
    #[default]
    Legacy,
    /// As its base mod, at the replaced patch's place in the order.
    Base,
}

impl GluePatchEntry {
    /// Whether the copy runs as its base mod (D58).
    pub fn runs_as_base(&self) -> bool {
        self.run_as == Some(RunAs::Base)
    }
}

/// Why load-order.json could not be used. Any of these means no mods (D16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    Io(String),
    Malformed {
        line: usize,
        column: usize,
        message: String,
    },
    Invalid(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::Io(m) => write!(f, "load-order.json could not be read: {m}"),
            PlanError::Malformed {
                line,
                column,
                message,
            } => write!(
                f,
                "load-order.json is not valid JSON at line {line}, column {column}: {message}"
            ),
            PlanError::Invalid(m) => write!(f, "load-order.json is invalid: {m}"),
        }
    }
}

/// The result of reading load-order.json. No file: `plan` and `error` are both None.
#[derive(Debug, Clone, Default)]
pub struct PlanLoad {
    pub plan: Option<LoadOrderPlan>,
    pub error: Option<PlanError>,
    /// Skipped entries and a newer schema.
    pub warnings: Vec<String>,
}

/// One warning per invalid entry of a plan field.
fn skipped<'a, T: 'a>(
    field: &str,
    entries: impl Iterator<Item = (String, &'a Lenient<T>)>,
    out: &mut Vec<String>,
) {
    for (key, entry) in entries {
        if let Some(e) = entry.error() {
            out.push(format!("load-order.json {field}{key} is invalid and was skipped: {e}"));
        }
    }
}

fn by_index<T>(v: &[Lenient<T>]) -> impl Iterator<Item = (String, &Lenient<T>)> {
    v.iter().enumerate().map(|(i, e)| (format!("[{i}]"), e))
}

fn by_key<'a, T: 'a>(
    m: impl Iterator<Item = (&'a String, &'a Lenient<T>)>,
) -> impl Iterator<Item = (String, &'a Lenient<T>)> {
    let mut v: Vec<_> = m.map(|(k, e)| (format!("[{k:?}]"), e)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v.into_iter()
}

impl LoadOrderPlan {
    /// Reads `<mod_dir>/qm/load-order.json`. Never fails: errors and skipped entries are in
    /// the result.
    pub fn read(mod_dir: &Path) -> PlanLoad {
        let path = mod_dir.join("qm").join("load-order.json");
        match std::fs::read_to_string(&path) {
            Ok(raw) => Self::parse(&raw),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PlanLoad::default(),
            Err(e) => PlanLoad {
                error: Some(PlanError::Io(e.to_string())),
                ..PlanLoad::default()
            },
        }
    }

    /// Parses the text of load-order.json (see `read`).
    pub fn parse(raw: &str) -> PlanLoad {
        let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
        match serde_json::from_str::<LoadOrderPlan>(raw) {
            Ok(plan) => PlanLoad {
                warnings: plan.warnings(),
                plan: Some(plan),
                error: None,
            },
            Err(e) => {
                use serde_json::error::Category;
                let error = match e.classify() {
                    Category::Syntax | Category::Eof => PlanError::Malformed {
                        line: e.line(),
                        column: e.column(),
                        message: e.to_string(),
                    },
                    Category::Data => PlanError::Invalid(e.to_string()),
                    Category::Io => PlanError::Io(e.to_string()),
                };
                PlanLoad {
                    error: Some(error),
                    ..PlanLoad::default()
                }
            }
        }
    }

    /// One line per skipped entry, plus a newer schema.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.schema_version > PLAN_SCHEMA_VERSION {
            out.push(format!(
                "load-order.json was written by a newer QuarterMaster (schema {}); only the parts this version knows are used",
                self.schema_version
            ));
        }
        skipped("mod_priorities", by_key(self.mod_priorities.iter()), &mut out);
        skipped("patch_priorities", by_key(self.patch_priorities.iter()), &mut out);
        skipped("disabled_patches", by_index(&self.disabled_patches), &mut out);
        skipped("glue_patches", by_index(&self.glue_patches), &mut out);
        skipped("patch_order", by_index(&self.patch_order), &mut out);
        if let Some(e) = self.stacking.as_ref().and_then(Lenient::error) {
            out.push(format!("load-order.json stacking is invalid, using legacy: {e}"));
        }
        skipped("refs", by_key(self.refs.iter()), &mut out);
        skipped("anchor_fixes", by_index(&self.anchor_fixes), &mut out);
        skipped("fix_of", by_key(self.fix_of.iter()), &mut out);
        if let Some(e) = self.line_rules_v2.as_ref().and_then(Lenient::error) {
            out.push(format!(
                "load-order.json line_rules_v2 is invalid, using the older rules: {e}"
            ));
        }
        skipped("apply_as", by_key(self.apply_as.iter()), &mut out);
        skipped("skip_hunks", by_index(&self.skip_hunks), &mut out);
        skipped("stack_rules", by_index(&self.stack_rules), &mut out);
        skipped("asset_choices", by_key(self.asset_choices.iter()), &mut out);
        skipped("asset_exclusions", by_key(self.asset_exclusions.iter()), &mut out);
        skipped("factions", by_key(self.factions.iter()), &mut out);
        out
    }

    /// The first valid priority among `keys` (an invalid entry counts as absent).
    fn priority_in(map: &HashMap<String, Lenient<Priority>>, keys: &[&str]) -> Option<Priority> {
        keys.iter()
            .find_map(|k| map.get(*k).and_then(Lenient::ok).copied())
    }

    /// The profile's priority for a mod, by id then folder name.
    pub fn mod_priority(&self, keys: &[&str]) -> Option<Priority> {
        Self::priority_in(&self.mod_priorities, keys)
    }

    /// The profile's priority for a patch, by id then `mod:name`.
    pub fn patch_priority(&self, keys: &[&str]) -> Option<Priority> {
        Self::priority_in(&self.patch_priorities, keys)
    }

    /// Whether any of `ids` is in `disabled_patches`.
    pub fn is_disabled(&self, ids: &[&str]) -> bool {
        self.disabled_patches
            .iter()
            .filter_map(Lenient::ok)
            .any(|d| ids.contains(&d.as_str()))
    }

    /// The valid glue patches, in plan order.
    pub fn glues(&self) -> impl Iterator<Item = &GluePatchEntry> {
        self.glue_patches.iter().filter_map(Lenient::ok)
    }

    /// The valid patch order rules, in plan order.
    pub fn order_rules(&self) -> Vec<OrderRule> {
        self.patch_order
            .iter()
            .filter_map(Lenient::ok)
            .cloned()
            .collect()
    }

    /// The stacking mode; Legacy when absent or invalid (D2).
    pub fn stack_mode(&self) -> StackMode {
        self.stacking
            .as_ref()
            .and_then(Lenient::ok)
            .copied()
            .unwrap_or_default()
    }

    /// Whether the Phase D line rules are on (D50, D51); off when absent or invalid.
    pub fn line_rules_v2(&self) -> bool {
        self.line_rules_v2
            .as_ref()
            .and_then(Lenient::ok)
            .copied()
            .unwrap_or(false)
    }

    /// The winner chosen for an asset group or slot key; None when absent or invalid.
    pub fn asset_choice(&self, key: &str) -> Option<&str> {
        self.asset_choices
            .get(key)
            .and_then(Lenient::ok)
            .map(String::as_str)
    }

    /// What a source (a mod id, or `faction:<id>`) leaves out; empty when absent or invalid.
    pub fn asset_exclusions_of(&self, source: &str) -> &[String] {
        self.asset_exclusions
            .get(source)
            .and_then(Lenient::ok)
            .map_or(&[], Vec::as_slice)
    }

    /// The faction picked for a side (`allied`, `axis`); None when absent or invalid.
    pub fn faction_pick(&self, side: &str) -> Option<&str> {
        self.factions
            .get(side)
            .and_then(Lenient::ok)
            .map(String::as_str)
    }

    /// The schema a plan with these fields is written as: 4 only with a Phase E field.
    pub fn schema_for_fields(&self) -> u32 {
        if self.asset_choices.is_empty()
            && self.asset_exclusions.is_empty()
            && self.factions.is_empty()
        {
            PLAN_SCHEMA_PHASE_D
        } else {
            PLAN_SCHEMA_VERSION
        }
    }

    /// Writes `<mod_dir>/qm/load-order.json` through a temp file and a rename.
    pub fn save_to_mod_dir_atomic(&self, mod_dir: &Path) -> Result<(), String> {
        let qm_dir = mod_dir.join("qm");
        std::fs::create_dir_all(&qm_dir)
            .map_err(|e| format!("Failed to create {}: {e}", qm_dir.display()))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize load-order.json: {e}"))?;
        write_file_atomic(&qm_dir.join("load-order.json"), json.as_bytes())
    }
}

/// Reads a handshake file: Ok(None) when it is missing, Err naming `file` when it cannot be
/// read or parsed.
fn read_json<T: serde::de::DeserializeOwned>(path: &Path, file: &str) -> Result<Option<T>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{file} could not be read: {e}")),
    };
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    serde_json::from_str(raw)
        .map(Some)
        .map_err(|e| format!("{file} is not valid: {e}"))
}

/// Writes `path` through `<path>.tmp`: write, flush to disk, then rename over the old file.
/// A crash leaves the old file or the new one, never a half-written one.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_name);
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.flush()?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Failed to write {}: {e}", tmp.display()));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Failed to replace {}: {e}", path.display())
    })
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOutcomeStatus {
    Applied,
    ZeroMatches,
    CountMismatch,
    SkippedByPolicy,
    SkippedDisabled,
    Aborted,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Telemetry record for a single patch execution on a target file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchExecutionRecord {
    pub patch_id: String,
    pub patch_name: String,
    pub mod_id: String,
    pub source_file: String,
    pub target_file: String,
    pub patch_type: String,
    pub priority: Priority,
    pub matched_count: usize,
    pub expected_count: Option<usize>,
    pub on_failure: OnFailurePolicy,
    pub status: PatchOutcomeStatus,
    pub warnings: Vec<String>,
    pub regions: Vec<PatchRegion>,
    /// Sites where stacking put this patch's code away from today's slot (§5.3).
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub stack_moves: Vec<StackMove>,
    /// How the patch's anchors resolved, site by site (only for a patch with rungs).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub anchoring: Option<AnchorReport>,
    /// The profile fixes that changed this patch.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub fix_ids: Vec<String>,
    /// The compatibility mods that changed this patch.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub changed_by: Vec<String>,
    /// Why a rung of this patch is suspended, when one is.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub suspended: Option<String>,
}

/// How a patch's anchor ladder resolved in one file (Phase C §3.9).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorReport {
    pub sites: Vec<SiteResolution>,
    /// Where the rungs came from: "author", "profile:<fix id>" or "compat:<mod id>".
    pub source: String,
}

/// One of the patch's own places, and where its code went.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteResolution {
    /// Index among the patch's matches in the unpatched file (for a lost site, of the site key).
    pub site: u32,
    /// 1-based line in the unpatched file, in the file's own numbering (QuarterMaster's added
    /// lines are not counted). The DLL and the simulation write the same value.
    pub vanilla_line: Option<u32>,
    /// The rung that placed the code, or none when the patch used its own line.
    pub rung: Option<u32>,
    /// The patch the code went next to (`mod:patch`), or "text" for a text rung.
    pub anchored_on: Option<String>,
    /// Why the rungs did not apply, when they did not.
    pub fallback: Option<String>,
    /// The rung found its text although the other patch changed.
    pub followed: bool,
    /// The other patch changed since the fix was made.
    pub b_drift: bool,
    /// A chosen place that is no longer in the file.
    pub lost: bool,
}

/// A rung whose order edge had to be dropped to break a cycle: the patch uses its own line.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspendedRung {
    pub patch_id: String,
    pub rung: u32,
    pub reason: String,
    /// "author", "profile:<fix id>" or "compat:<mod id>".
    pub source: String,
}

/// What became of one compatibility mod's `[[overrides]]` entry (§3.10, §8.1). An entry can
/// give several reports: one for the changes that apply and one for each set of changes a
/// stronger layer replaced.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverrideReport {
    /// The compatibility mod's id.
    pub from_mod: String,
    /// The patch the entry names, as written.
    pub target: String,
    /// The changes this report is about: "disable", "priority", "order" and "anchors",
    /// joined with ", ".
    pub action: String,
    /// "applied" | "rejected_not_required" | "target_missing" | "suspended_drift" |
    /// "superseded" | "superseded_by_profile" | "waiting" | "invalid".
    pub status: String,
    /// The same in plain words.
    pub detail: String,
}

/// A rung that is never used, with why. The patch still loads and uses its own line.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidRung {
    pub patch_id: String,
    pub rung: u32,
    /// "not_required" | "at_patch" | "edge_with_pattern" | "edge_without_within" |
    /// "no_target" | "mod_wildcard" | "position_at" | "module_patch" | "not_pattern" |
    /// "malformed" | "needs_capture" | "regex_position".
    pub reason: String,
    /// The same in plain words.
    pub detail: String,
    pub source: String,
}

/// One site where a patch's code was not put in the legacy slot of a shared line, or was
/// kept there by the guard (§5.3). Slots count the items on that side, top to bottom.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackMove {
    pub site: u32,
    /// 1-based, in the text the patch was applied to.
    pub anchor_line: u32,
    pub side: Side,
    pub legacy_slot: u32,
    pub slot: u32,
    pub reason: StackReason,
    /// The patch whose code is right above this one, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub below: Option<String>,
    /// The patch whose code is right below this one, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub above: Option<String>,
    /// Stack rules a stronger rule beat here (D56), in plain words.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub overridden: Vec<String>,
}

/// Why a stack slot differs from the legacy one. Ids are mod ids.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StackReason {
    /// Below the code of a mod this one depends on.
    BelowDependency { of: String },
    /// Above the code of a mod that depends on this one.
    AboveDependent { of: String },
    LoadOrder,
    /// A dependency sits below a dependent, so no slot fits both; the dependency wins.
    Unsatisfiable { below: String, above: String },
    /// Kept in the legacy slot: the move would cross this patch's next match.
    GuardClamped,
    /// Placed by the user's stack rule (D55).
    UserRule { fix_id: String },
    /// Placed by a compatibility mod's stack rule (D55).
    CompatRule { mod_id: String },
    /// Placed by an author's `stack` key (D57): this block's own, or the hint of mod `of`'s
    /// block next to it.
    AuthorHint {
        hint: crate::patch::stack::StackHint,
        of: String,
    },
}

/// Written by `winmm.dll` (and offline validation) to `Mods/qm/patch-report.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatchReport {
    pub qm_version: String,
    pub timestamp: String,
    pub profile_id: Option<String>,
    pub total_patches_applied: usize,
    pub total_warnings: usize,
    pub total_failures: usize,
    pub records: Vec<PatchExecutionRecord>,
    /// Patch order cycles that were broken to apply patches (patch names in cycle order).
    #[serde(default)]
    pub order_cycles: Vec<Vec<String>>,
    /// Mods that failed to load, each skipped with the mods that require it (D15).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_errors: Vec<LoadError>,
    /// The patch order hit an impossible state and placed the rest by priority.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub order_degraded: bool,
    /// Set by the first caught panic in the DLL; from then on every chunk gets only
    /// QuarterMaster's builtin edits (D34).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dll_degraded: Option<String>,
    /// load-order.json could not be used, so no mods were loaded (D16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_error: Option<String>,
    /// Plan entries that were skipped, and other notes about the plan (D17).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_warnings: Vec<String>,
    /// How plan fixes met the loaded patches: one per plan entry with a ref (§7.1), and an
    /// adjusted copy waiting for its mod (D19).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fix_bindings: Vec<FixBinding>,
    /// Patch records whose match touched lines QuarterMaster adds or rewrites at startup
    /// (`matched_qm_startup_code`): a match next to a line it rewrote, or one it refused
    /// (`matched_qm_startup_code: skipped`, D42). Not a conflict.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub builtin_matches: usize,
    /// Order rules a stronger rule replaced or ignored (§4.5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overridden_rules: Vec<String>,
    /// Names in order rules that match no loaded patch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_ids: Vec<String>,
    /// Rungs suspended to break an order cycle.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suspended_rungs: Vec<SuspendedRung>,
    /// Rungs that are never used, with why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalid_rungs: Vec<InvalidRung>,
    /// What became of each compatibility mod's overrides (§8.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<OverrideReport>,
    /// launch-config.json could not be used, so no mods were loaded and the game used its
    /// default save identity (D65).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_config_error: Option<String>,
    /// launch-config.json's run id (D67).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The game quit through love.quit and the quit went ahead (D66). False after a crash, a
    /// cancelled quit, or a close that skipped love.quit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clean_exit: bool,
    /// The source that ended up owning each asset slot in the game (D77).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asset_winners: Vec<AssetWinner>,
    /// What the probes of a diagnostic test run saw (D89).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub globals_trace: Vec<GlobalChange>,
    /// Who called the watched hooks in a diagnostic test run (D90).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hook_calls: Vec<HookCalls>,
}

/// One asset slot and the source whose file the game ended up using.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetWinner {
    /// Lowercase `category/stem`.
    pub slot: String,
    /// A mod id, or `faction:<id>`.
    pub source: String,
}

/// One change of a watched global in a diagnostic test run, with the line that made it.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalChange {
    pub name: String,
    /// The patch that owns the line that made the change; empty for the game's own code.
    #[serde(default)]
    pub patch_id: String,
    /// That patch's mod; empty for the game's own code.
    #[serde(default)]
    pub mod_id: String,
    /// The chunk the line is in, as Lua names it (`@main.lua`).
    #[serde(default)]
    pub source: String,
    /// The line in the file the game loaded (patched).
    #[serde(default)]
    pub line: u32,
    /// The Lua type of the new value (`nil` when it was removed).
    pub value_type: String,
    /// Tells two values of one type apart (`function: 0x...`); the value itself for scalars.
    #[serde(default)]
    pub identity: String,
    /// The order the changes were seen in, from 1.
    #[serde(default)]
    pub seq: u32,
}

/// The distinct places that called one watched hook.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookCalls {
    pub name: String,
    #[serde(default)]
    pub total: u32,
    /// No function of that name existed after love.load, so nothing was counted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
    #[serde(default)]
    pub callers: Vec<HookCaller>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookCaller {
    /// The chunk name as Lua reports it.
    pub source: String,
    pub line: u32,
    #[serde(default)]
    pub calls: u32,
    /// The patch that owns the line, when it is patched code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub patch_id: Option<String>,
    /// That patch's mod.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub mod_id: Option<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// How one plan fix met the loaded patches (§3.9, §7.2).
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixBinding {
    /// The ledger fix id from `fix_of`, else the plan entry's own id.
    pub fix_id: String,
    /// The `fix_of` key: "glue:<id>", "disable:<id>", ...
    pub key: String,
    pub status: BindingStatus,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub new_id: Option<String>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BindingStatus {
    Bound,
    Rebound { from: String, to: String },
    Drifted { anchor: bool, payload: bool },
    Ambiguous { candidates: Vec<String> },
    Missing,
    /// The fix's mods are off, so it does nothing until they are on again.
    Waiting { mods: Vec<String> },
    Suspended { reason: String },
}

/// A mod that could not be loaded. It is skipped, together with every enabled mod that
/// requires it, directly or through another skipped mod.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadError {
    /// The mod's id; empty when the Mods folder itself could not be read.
    pub mod_id: String,
    /// The mod's folder or zip name (the Mods folder path for a Mods folder error).
    pub folder: String,
    pub message: String,
    /// Ids of the enabled mods skipped because they require this one.
    #[serde(default)]
    pub skipped_dependents: Vec<String>,
}

impl PatchReport {
    /// Reads `<mod_dir>/qm/patch-report.json`. Ok(None) when there is no file; Err when it
    /// cannot be read or parsed.
    pub fn load_from_mod_dir(mod_dir: &Path) -> Result<Option<Self>, String> {
        read_json(&mod_dir.join("qm").join("patch-report.json"), "patch-report.json")
    }

    /// Writes `<mod_dir>/qm/patch-report.json` through a temp file and a rename.
    pub fn save_to_mod_dir(&self, mod_dir: &Path) -> Result<(), String> {
        let qm_dir = mod_dir.join("qm");
        std::fs::create_dir_all(&qm_dir)
            .map_err(|e| format!("Failed to create {}: {e}", qm_dir.display()))?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize patch-report.json: {e}"))?;
        write_file_atomic(&qm_dir.join("patch-report.json"), json.as_bytes())
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
/// Structured crash diagnostic report written to `Mods/qm/log/crashes/crash-{timestamp}.json`
/// whenever a Rust panic or in-game Lua `love.errorhandler` blue-screen occurs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashReport {
    pub id: String,
    pub timestamp: String,
    pub crash_kind: String, // "lua_runtime" | "lua_syntax" | "rust_panic"
    pub error_message: String,
    pub traceback: String,
    pub crashing_file: Option<String>,
    pub crashing_line: Option<usize>,
    pub suspected_mod_id: Option<String>,
    pub suspected_patch_id: Option<String>,
    pub suspected_patch_name: Option<String>,
    pub code_snippet: Vec<CrashSnippetLine>,
    pub active_profile_id: Option<String>,
    pub active_mods: HashMap<String, String>,
    /// launch-config.json's run id (D67); absent in reports from an older DLL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub run_id: Option<String>,
    /// The profile fixes of the suspected patch (D58). For an adjusted copy, the fix that
    /// made it.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub suspected_fix_ids: Vec<String>,
    /// The suspected patch is an adjusted copy: its fix is the suspect, not its base mod.
    // `default` last: specta reads the serde list in order and takes it as optional.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub suspected_adjusted_copy: bool,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashSnippetLine {
    pub line_number: usize,
    pub text: String,
    pub is_crash_line: bool,
    pub attributed_mod_id: Option<String>,
    pub attributed_patch_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qm_file(dir: &Path, name: &str) -> std::path::PathBuf {
        let qm = dir.join("qm");
        std::fs::create_dir_all(&qm).unwrap();
        qm.join(name)
    }

    #[test]
    fn the_phase_e_plan_fields_read_leniently_and_set_the_schema() {
        // A plan from before Phase E has neither field and stays schema 3.
        let old = LoadOrderPlan::default();
        assert_eq!(old.schema_for_fields(), PLAN_SCHEMA_PHASE_D);
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("asset_choices") && !json.contains("factions"), "{json}");

        let plan: LoadOrderPlan = serde_json::from_str(
            r#"{"schema_version": 4, "profile_id": "p", "generated_at": "t", "enabled_mods": [],
                "asset_choices": {"sounds/vo/axis/axishurt#": "axissw", "sounds/garand1": 5},
                "asset_exclusions": {"voices2": ["assets/vo/axis/axisHurt1.wav"], "bad": "x"},
                "factions": {"allied": "eastern_front", "axis": ["x"]}}"#,
        )
        .unwrap();
        assert_eq!(plan.asset_choice("sounds/vo/axis/axishurt#"), Some("axissw"));
        assert_eq!(plan.asset_choice("sounds/garand1"), None, "an invalid entry is ignored");
        assert_eq!(plan.faction_pick("allied"), Some("eastern_front"));
        assert_eq!(plan.faction_pick("axis"), None);
        assert_eq!(plan.asset_exclusions_of("voices2"), ["assets/vo/axis/axisHurt1.wav"]);
        assert!(plan.asset_exclusions_of("bad").is_empty());
        assert!(plan.asset_exclusions_of("other").is_empty());
        assert_eq!(plan.schema_for_fields(), PLAN_SCHEMA_VERSION);
        let only_exclusions = LoadOrderPlan {
            asset_exclusions: plan.asset_exclusions.clone(),
            ..LoadOrderPlan::default()
        };
        assert_eq!(only_exclusions.schema_for_fields(), PLAN_SCHEMA_VERSION);
        let w = plan.warnings();
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w.iter().any(|l| l.contains("asset_exclusions")), "{w:?}");
        assert!(w.iter().any(|l| l.contains("asset_choices")), "{w:?}");
        assert!(w.iter().any(|l| l.contains("factions")), "{w:?}");
        // The invalid entries round-trip as they were written.
        let back = serde_json::to_value(&plan).unwrap();
        assert_eq!(back["asset_choices"]["sounds/garand1"], 5);
        assert_eq!(back["factions"]["axis"][0], "x");
    }

    #[test]
    fn schemas_up_to_four_read_without_a_newer_warning() {
        for v in 1..=PLAN_SCHEMA_VERSION {
            let plan = LoadOrderPlan {
                schema_version: v,
                ..LoadOrderPlan::default()
            };
            assert!(plan.warnings().is_empty(), "schema {v}");
        }
        let newer = LoadOrderPlan {
            schema_version: PLAN_SCHEMA_VERSION + 1,
            ..LoadOrderPlan::default()
        };
        assert_eq!(newer.warnings().len(), 1);
    }

    #[test]
    fn a_launch_flag_takes_one_or_two_dashes_and_the_whole_word() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(launch_flag(&args(&["mnb.exe", "-debug"]), "debug", "QM_TEST_UNSET"));
        assert!(launch_flag(&args(&["mnb.exe", "--debug"]), "debug", "QM_TEST_UNSET"));
        assert!(!launch_flag(&args(&["mnb.exe", "-d", "-dev"]), "debug", "QM_TEST_UNSET"));
        assert!(launch_flag(&args(&["mnb.exe", "-d", "-dev"]), "dev", "QM_TEST_UNSET"));
        assert!(!launch_flag(&args(&["mnb.exe", "debug"]), "debug", "QM_TEST_UNSET"));
        let cfg: LaunchConfig = serde_json::from_str("{}").unwrap();
        assert!(!cfg.debug && !cfg.dev_commands);
        let cfg: LaunchConfig =
            serde_json::from_str(r#"{"debug": true, "dev_commands": true}"#).unwrap();
        assert!(cfg.debug && cfg.dev_commands);
    }

    #[test]
    fn the_phase_e_launch_config_fields_are_lenient() {
        let cfg: LaunchConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.save_stamp().is_none() && cfg.diagnostics().is_none());
        assert!(cfg.save_content.is_empty() && cfg.warnings().is_empty());
        let json = serde_json::to_string(&cfg).unwrap();
        for key in ["save_stamp", "save_content", "diagnostics"] {
            assert!(!json.contains(key), "{key} is written only when set: {json}");
        }

        let cfg: LaunchConfig = serde_json::from_str(
            r#"{"save_stamp": {"profile_id": "p", "profile_name": "P",
                               "mods": {"gcore": "1.0.0"}, "fields": {"gcore": ["option_x"]}},
                "save_content": {"m": {"weapons": {"carcano": "k98"}}, "bad": 3},
                "diagnostics": {"globals": ["loadModSettings"]}}"#,
        )
        .unwrap();
        let stamp = cfg.save_stamp().unwrap();
        assert_eq!(stamp.fields["gcore"], ["option_x"]);
        assert_eq!(cfg.save_content["m"].ok().unwrap().weapons["carcano"], "k98");
        assert_eq!(cfg.diagnostics().unwrap().globals, ["loadModSettings"]);
        let w = cfg.warnings();
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("save_content of 'bad'"), "{w:?}");

        // A broken value does not fail the file, and empty diagnostics mean none.
        let cfg: LaunchConfig = serde_json::from_str(
            r#"{"save_stamp": "no", "diagnostics": {"globals": [], "hooks": []}}"#,
        )
        .unwrap();
        assert!(cfg.save_stamp().is_none() && cfg.diagnostics().is_none());
        assert_eq!(cfg.warnings().len(), 1);
        let cfg: LaunchConfig = serde_json::from_str(r#"{"diagnostics": 7}"#).unwrap();
        assert!(cfg.diagnostics().is_none());
        assert_eq!(cfg.warnings().len(), 1);
    }

    #[test]
    fn the_phase_e_report_fields_are_written_only_when_set() {
        let empty: PatchReport = serde_json::from_str(
            r#"{"qm_version": "1", "timestamp": "t", "profile_id": null,
                "total_patches_applied": 0, "total_warnings": 0, "total_failures": 0,
                "records": []}"#,
        )
        .unwrap();
        assert!(empty.asset_winners.is_empty() && empty.globals_trace.is_empty());
        let json = serde_json::to_string(&empty).unwrap();
        for key in ["asset_winners", "globals_trace", "hook_calls"] {
            assert!(!json.contains(key), "{key}: {json}");
        }
        let mut full = empty;
        full.asset_winners.push(AssetWinner {
            slot: "sounds/garand1".into(),
            source: "axissw".into(),
        });
        full.hook_calls.push(HookCalls {
            name: "onDeath".into(),
            total: 2,
            missing: false,
            callers: vec![HookCaller {
                source: "@includes/urb_engine.lua".into(),
                line: 42133,
                calls: 1,
                patch_id: None,
                mod_id: None,
            }],
        });
        let back: PatchReport =
            serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
        assert_eq!(back.asset_winners, full.asset_winners);
        assert_eq!(back.hook_calls, full.hook_calls);
    }

    #[test]
    fn a_missing_launch_config_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(LaunchConfig::load_from_mod_dir(dir.path()).unwrap().is_none());
    }

    #[test]
    fn a_broken_launch_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = qm_file(dir.path(), "launch-config.json");
        std::fs::write(&path, "{\"vanilla_mode\": tru").unwrap();
        let err = LaunchConfig::load_from_mod_dir(dir.path()).unwrap_err();
        assert!(err.starts_with("launch-config.json is not valid: "), "{err}");

        // A field of the wrong type is an error too, not the defaults.
        std::fs::write(&path, "{\"vanilla_mode\": 1}").unwrap();
        assert!(LaunchConfig::load_from_mod_dir(dir.path()).is_err());
    }

    #[test]
    fn an_unreadable_launch_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be: it is there but cannot be read.
        std::fs::create_dir_all(qm_file(dir.path(), "launch-config.json")).unwrap();
        let err = LaunchConfig::load_from_mod_dir(dir.path()).unwrap_err();
        assert!(err.starts_with("launch-config.json could not be read: "), "{err}");
    }

    #[test]
    fn a_launch_config_reads_back_also_with_a_bom() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = LaunchConfig {
            vanilla_mode: true,
            save_identity_override: Some("mnb_x".to_string()),
            ..LaunchConfig::default()
        };
        cfg.save_to_mod_dir(dir.path()).unwrap();
        let read = LaunchConfig::load_from_mod_dir(dir.path()).unwrap().unwrap();
        assert!(read.vanilla_mode);
        assert_eq!(read.save_identity_override.as_deref(), Some("mnb_x"));

        let path = qm_file(dir.path(), "launch-config.json");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("\u{feff}{text}")).unwrap();
        assert!(LaunchConfig::load_from_mod_dir(dir.path()).unwrap().unwrap().vanilla_mode);
    }

    #[test]
    fn patch_report_load_tells_missing_from_broken() {
        let dir = tempfile::tempdir().unwrap();
        assert!(PatchReport::load_from_mod_dir(dir.path()).unwrap().is_none());
        std::fs::write(qm_file(dir.path(), "patch-report.json"), "{").unwrap();
        let err = PatchReport::load_from_mod_dir(dir.path()).unwrap_err();
        assert!(err.starts_with("patch-report.json is not valid: "), "{err}");
    }

    #[test]
    fn patch_report_save_writes_a_temp_file_and_renames_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = PatchReport {
            qm_version: "1".to_string(),
            launch_config_error: Some("broken".to_string()),
            ..PatchReport::default()
        };
        first.save_to_mod_dir(dir.path()).unwrap();
        let tmp = qm_file(dir.path(), "patch-report.json.tmp");
        assert!(!tmp.exists(), "the temp file was renamed");
        let read = PatchReport::load_from_mod_dir(dir.path()).unwrap().unwrap();
        assert_eq!(read.launch_config_error.as_deref(), Some("broken"));

        // With the temp path blocked the save fails, and the old report stays whole.
        std::fs::create_dir_all(&tmp).unwrap();
        let second = PatchReport {
            qm_version: "2".to_string(),
            ..PatchReport::default()
        };
        let err = second.save_to_mod_dir(dir.path()).unwrap_err();
        assert!(err.contains("patch-report.json.tmp"), "{err}");
        let kept = PatchReport::load_from_mod_dir(dir.path()).unwrap().unwrap();
        assert_eq!(kept.qm_version, "1");
    }

    #[test]
    fn reports_from_before_run_ids_still_read() {
        let report: PatchReport = serde_json::from_str(
            r#"{"qm_version": "1", "timestamp": "t", "profile_id": null, "total_patches_applied": 0,
                "total_warnings": 0, "total_failures": 0, "records": []}"#,
        )
        .unwrap();
        assert_eq!(report.run_id, None);
        assert!(!report.clean_exit);
        // Neither field is written while unset, so old readers see the same file.
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("run_id") && !json.contains("clean_exit"), "{json}");

        let crash: CrashReport = serde_json::from_str(
            r#"{"id": "c", "timestamp": "t", "crash_kind": "lua_runtime", "error_message": "e",
                "traceback": "t", "crashing_file": null, "crashing_line": null,
                "suspected_mod_id": null, "suspected_patch_id": null, "suspected_patch_name": null,
                "code_snippet": [], "active_profile_id": null, "active_mods": {}}"#,
        )
        .unwrap();
        assert_eq!(crash.run_id, None);

        let cfg: LaunchConfig = serde_json::from_str(r#"{"profile_id": "p", "run_id": "r1"}"#).unwrap();
        assert_eq!(cfg.run_id.as_deref(), Some("r1"));
        assert!(!serde_json::to_string(&LaunchConfig::default()).unwrap().contains("run_id"));
    }
}
