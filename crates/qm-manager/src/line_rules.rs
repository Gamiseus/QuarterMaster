//! The line rules comparison (D51): whether the Phase D line rules (D50 owner-based rungs,
//! D51 implicit anchors) change a profile's game output, and where.
//!
//! The launcher runs it once per simulation, in the background, only for a profile that has
//! not answered the `stacking_v2` notice and whose output the rules can change. The notice
//! shows only when something differs; nothing is written without the user's answer.

use std::collections::{BTreeMap, HashMap};

use qm_core::handshake::{LaunchConfig, PatchExecutionRecord, SiteResolution, StackMode};
use qm_core::patch::table::{PatchTable, TracedApply};
use qm_core::patch::Patch;
use qm_core::prepare::{simulate_buffer, FileView};
use serde::{Deserialize, Serialize};

use crate::stacks::shared_stacks;

/// What the line rules change in one profile, from the older rules to the newer ones.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRulesDiff {
    /// Files whose game text differs.
    pub files: Vec<String>,
    /// Shared lines whose order differs: `<file> line <n> <side> (<where>): <old> -> <new>`.
    pub stacks: Vec<String>,
    /// Patches whose places on another patch's code differ: `<file> <patch>: <old> -> <new>`.
    pub rungs: Vec<String>,
}

impl LineRulesDiff {
    /// Both rules give the same output.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.stacks.is_empty() && self.rungs.is_empty()
    }

    /// The game output differs, so the notice shows (Task 13). Stack and rung changes that
    /// move no text are Review detail only.
    pub fn changes_output(&self) -> bool {
        !self.files.is_empty()
    }
}

/// Whether any patch that runs has a rung (author anchors, a compat rung or a profile anchor
/// fix): the rules change rung resolution in every mode (D50).
pub fn has_rung(table: &PatchTable) -> bool {
    table.patches.iter().any(|(p, _, _)| {
        !p.is_disabled()
            && (!p.extra().rungs.is_empty()
                || matches!(p, Patch::Pattern(x) if !x.anchors.is_empty())
                || matches!(p, Patch::Regex(x) if !x.anchors.is_empty()))
    })
}

/// Whether the comparison runs (D51): the profile has no answer yet, and the rules can change
/// its output (a mode other than Legacy, or a rung). A Legacy profile with no rung never runs
/// it.
pub fn needs_comparison(answer: Option<bool>, table: &PatchTable) -> bool {
    answer.is_none() && (table.stack_mode != StackMode::Legacy || has_rung(table))
}

/// Simulates every file `table` patches under both rules and lists what differs. `table` may
/// carry either value; the result always reads from the older rules to the newer ones.
pub fn compare(
    table: &PatchTable,
    vanilla: &HashMap<String, String>,
    launch: &LaunchConfig,
) -> LineRulesDiff {
    let mut flipped = table.clone();
    flipped.line_rules_v2 = !table.line_rules_v2;
    let (old, new) = if table.line_rules_v2 {
        (&flipped, table)
    } else {
        (table, &flipped)
    };
    let (old_prep, new_prep) = (old.prepare(), new.prepare());
    let mut keys: Vec<&String> = vanilla.keys().filter(|f| table.needs_patching(f)).collect();
    keys.sort();
    let mut out = LineRulesDiff::default();
    for file in keys {
        let Some(src) = vanilla.get(file) else {
            continue;
        };
        let a = simulate_buffer(old, &old_prep, launch, file, src);
        let b = simulate_buffer(new, &new_prep, launch, file, src);
        match (a, b) {
            (Ok((buf_a, a)), Ok((buf_b, b))) => {
                if a.text != b.text {
                    out.files.push(file.clone());
                }
                let sa = stacks_of(file, table.stack_mode, &a, &buf_a);
                let sb = stacks_of(file, table.stack_mode, &b, &buf_b);
                diff_stacks(file, &sa, &sb, &mut out.stacks);
                diff_maps(
                    file,
                    &rungs_of(&a.records),
                    &rungs_of(&b.records),
                    &mut out.rungs,
                );
            }
            (Err(x), Err(y)) if x == y => {}
            _ => out.files.push(file.clone()),
        }
    }
    out
}

/// The shared stacks of one file, keyed by where the line is (the game line it hangs on and
/// the side, or its text, function and side on another patch's code), so the two runs compare
/// even when line numbers move. The value is the shown label (with the line) and the order.
fn stacks_of(
    file: &str,
    mode: StackMode,
    t: &TracedApply,
    buf: &qm_core::prepare::PreparedBuffer,
) -> BTreeMap<String, (String, String)> {
    let view = FileView::new(&t.trace, buf);
    let mut out = BTreeMap::new();
    for s in shared_stacks(file, mode, &t.lines, &t.records, &t.trace.text, &view, buf) {
        let side = match s.side {
            qm_core::patch::linemap::Side::Before => "before",
            qm_core::patch::linemap::Side::After => "after",
        };
        let base = if s.site.context.is_empty() {
            format!("{side} {:?} ({})", s.anchor_text, s.site_label)
        } else {
            format!(
                "{side} {}#{}",
                s.site.context,
                s.site.ordinal.unwrap_or(0)
            )
        };
        // The same text twice in one function: number the repeats in file order.
        let mut key = base.clone();
        let mut n = 1;
        while out.contains_key(&key) {
            n += 1;
            key = format!("{base} #{n}");
        }
        let label = format!(
            "line {} {side} {:?} ({})",
            s.file_line, s.anchor_text, s.site_label
        );
        let ids: Vec<&str> = s.blocks.iter().map(|b| b.patch_id.as_str()).collect();
        out.insert(key, (label, ids.join(", ")));
    }
    out
}

/// Lists the stacks whose order differs, labelled as the newer run shows them.
fn diff_stacks(
    file: &str,
    old: &BTreeMap<String, (String, String)>,
    new: &BTreeMap<String, (String, String)>,
    out: &mut Vec<String>,
) {
    let none = String::from("(none)");
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (o, n) = (old.get(k), new.get(k));
        let a = o.map_or(&none, |x| &x.1);
        let b = n.map_or(&none, |x| &x.1);
        if a != b {
            let label = n.or(o).map_or(k, |x| &x.0);
            out.push(format!("{file} {label}: {a} -> {b}"));
        }
    }
}

/// Per patch with rungs, where each of its places went.
fn rungs_of(records: &[PatchExecutionRecord]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in records {
        let Some(a) = &r.anchoring else { continue };
        let sites: Vec<String> = a.sites.iter().map(site_text).collect();
        out.insert(r.patch_id.clone(), sites.join("; "));
    }
    out
}

fn site_text(s: &SiteResolution) -> String {
    match (s.rung, &s.fallback) {
        (Some(r), _) => format!("place {} by rung {}", s.site + 1, r + 1),
        (None, Some(why)) => format!("place {} on its own line ({why})", s.site + 1),
        (None, None) => format!("place {} on its own line", s.site + 1),
    }
}

fn diff_maps(
    file: &str,
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
    out: &mut Vec<String>,
) {
    let none = String::from("(none)");
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (a, b) = (old.get(k).unwrap_or(&none), new.get(k).unwrap_or(&none));
        if a != b {
            out.push(format!("{file} {k}: {a} -> {b}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qm_core::patch::plan::ModMeta;
    use qm_core::patch::{InsertPosition, OnFailurePolicy, PatternPatch, Target};
    use std::path::PathBuf;

    fn pat(id: &str, pattern: &str, position: InsertPosition, payload: &str) -> Patch {
        Patch::Pattern(PatternPatch {
            pattern: pattern.to_string(),
            position,
            target: Target::Single("test.lua".to_string()),
            payload: payload.to_string(),
            match_indent: false,
            times: None,
            overwrite: false,
            id: Some(id.to_string()),
            name: None,
            description: None,
            requires: Vec::new(),
            priority: None,
            before: Vec::new(),
            after: Vec::new(),
            on_failure: OnFailurePolicy::Warn,
            anchors: Vec::new(),
            sites: Vec::new(),
            stack: None,
            apply_as: None,
            mod_id: id.split(':').next().map(str::to_string),
            disabled: false,
            qm: Default::default(),
        })
    }

    fn meta(id: &str, requires: &[&str]) -> ModMeta {
        ModMeta {
            id: id.to_string(),
            folder: id.to_string(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
            load_after: requires.iter().map(|s| s.to_string()).collect(),
            manifest_ok: true,
            ..ModMeta::default()
        }
    }

    /// The L5 repro: x's insert-only `at` and TO's insert after the same line.
    fn l5(mode: StackMode) -> PatchTable {
        let mut t = PatchTable::default();
        t.targets.insert("test.lua".to_string());
        let patches = [
            pat("gcore:1", "L", InsertPosition::After, "g"),
            pat("x:1", "L", InsertPosition::At, "L\nr"),
            pat("to:1", "L", InsertPosition::After, "t"),
        ];
        for (i, p) in patches.into_iter().enumerate() {
            let folder = p.mod_id().unwrap_or("m").to_string();
            t.patches.push((
                p,
                10 * (i as i32 + 1),
                PathBuf::from(folder).join("qm.toml"),
            ));
        }
        t.mods = vec![meta("gcore", &[]), meta("x", &[]), meta("to", &["gcore"])];
        t.stack_mode = mode;
        t
    }

    fn vanilla() -> HashMap<String, String> {
        HashMap::from([("test.lua".to_string(), "L\nE\n".to_string())])
    }

    #[test]
    fn a_changed_stack_is_listed() {
        let d = compare(
            &l5(StackMode::Dependencies),
            &vanilla(),
            &LaunchConfig::default(),
        );
        assert_eq!(d.files, ["test.lua"]);
        assert_eq!(d.stacks.len(), 1, "{:?}", d.stacks);
        // Today `r` ends the chain, so there is no shared line to list before.
        assert!(
            d.stacks[0].ends_with(": (none) -> x:1, gcore:1, to:1"),
            "{:?}",
            d.stacks
        );
        assert!(d.rungs.is_empty());
        // The same from a table on the newer rules.
        let mut on = l5(StackMode::Dependencies);
        on.line_rules_v2 = true;
        assert_eq!(compare(&on, &vanilla(), &LaunchConfig::default()), d);
    }

    #[test]
    fn a_stack_change_that_moves_no_text_is_detail_only() {
        // x rewrites L and adds r; y adds a line after L. Both rules give "L\ny\nr\nE\n".
        let mut t = PatchTable::default();
        t.targets.insert("test.lua".to_string());
        let patches = [
            pat("x:1", "L", InsertPosition::At, "L\nr"),
            pat("y:1", "L", InsertPosition::After, "y"),
        ];
        for (i, p) in patches.into_iter().enumerate() {
            let folder = p.mod_id().unwrap_or("m").to_string();
            t.patches.push((
                p,
                10 * (i as i32 + 1),
                PathBuf::from(folder).join("qm.toml"),
            ));
        }
        t.mods = vec![meta("x", &[]), meta("y", &[])];
        t.stack_mode = StackMode::Dependencies;
        let d = compare(&t, &vanilla(), &LaunchConfig::default());
        assert!(d.files.is_empty(), "{d:?}");
        assert!(!d.stacks.is_empty(), "{d:?}");
        assert!(!d.is_empty());
        assert!(!d.changes_output(), "no notice for a stack-only difference");
        assert!(d.stacks[0].contains("line 1 after"), "{:?}", d.stacks);
        assert!(compare(&l5(StackMode::Dependencies), &vanilla(), &LaunchConfig::default()).changes_output());
    }

    #[test]
    fn legacy_without_rungs_never_compares_and_would_find_nothing() {
        let t = l5(StackMode::Legacy);
        assert!(!has_rung(&t));
        assert!(!needs_comparison(None, &t));
        assert!(compare(&t, &vanilla(), &LaunchConfig::default()).is_empty());
        let deps = l5(StackMode::Dependencies);
        assert!(needs_comparison(None, &deps));
        // An answer, either way, ends it.
        assert!(!needs_comparison(Some(false), &deps));
        assert!(!needs_comparison(Some(true), &deps));
        // A rung makes a Legacy profile compare (D50 changes rung resolution there).
        let mut rung = l5(StackMode::Legacy);
        rung.patches[2].0.extra_mut().rungs = vec![(
            qm_core::patch::anchor::Rung {
                within: Some("x:1".into()),
                pattern: Some("r".into()),
                ..Default::default()
            },
            qm_core::patch::anchor::AnchorSource::Author,
        )];
        assert!(needs_comparison(None, &rung));
        // So do author rungs on a regex patch (D52).
        let mut regex = l5(StackMode::Legacy);
        let text = "[[patches]]\n[patches.regex]\ntarget = \"test.lua\"\npattern = 'r'\n\
                    position = \"after\"\npayload = 'x'\nanchors = [{ within = \"x:1\" }]\n";
        let file: qm_core::patch::PatchFile =
            toml::from_str(&format!("[manifest]\nversion = \"1.0.0\"\n{text}")).unwrap();
        let p = file.patches.into_iter().next().unwrap();
        assert!(matches!(&p, Patch::Regex(_)));
        regex.patches.push((p, 99, std::path::PathBuf::from("r/qm.toml")));
        assert!(has_rung(&regex));
    }
}
