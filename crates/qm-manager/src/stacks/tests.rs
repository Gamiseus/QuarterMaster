use super::*;
use crate::conflict::simulate_and_analyze_patches;
use qm_core::patch::plan::ModMeta;
use qm_core::patch::table::PatchTable;
use qm_core::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};
use std::collections::HashMap;
use std::path::PathBuf;

fn meta(id: &str, requires: &[&str], ix: u32) -> ModMeta {
    ModMeta {
        id: id.to_string(),
        folder: id.to_string(),
        requires: requires.iter().map(|s| s.to_string()).collect(),
        load_after: requires.iter().map(|s| s.to_string()).collect(),
        manifest_ok: true,
        loader_index: ix,
        ..ModMeta::default()
    }
}

fn pat(target: &str, id: &str, anchor: &str, position: InsertPosition, payload: &str) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: anchor.to_string(),
        position,
        target: Target::Single(target.to_string()),
        payload: payload.to_string(),
        match_indent: false,
        times: None,
        overwrite: false,
        id: Some(id.to_string()),
        name: Some(format!("name of {id}")),
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

/// Patches run in the order given.
fn table(patches: Vec<Patch>, mode: StackMode) -> PatchTable {
    let mut t = PatchTable::default();
    for (i, p) in patches.into_iter().enumerate() {
        if let Patch::Pattern(x) = &p {
            if let Target::Single(f) = &x.target {
                t.targets.insert(f.clone());
            }
        }
        let folder = p.mod_id().unwrap_or("m").to_string();
        t.patches.push((
            p,
            10 * (i as i32 + 1),
            PathBuf::from(folder).join("qm.toml"),
        ));
    }
    t.mods = vec![meta("gcore", &[], 0), meta("to", &["gcore"], 1)];
    t.stack_mode = mode;
    t
}

fn stacks(t: &PatchTable, file: &str, src: &str) -> Vec<SharedStack> {
    let vanilla = HashMap::from([(file.to_string(), src.to_string())]);
    let r = simulate_and_analyze_patches(t, &vanilla);
    r.files_touched
        .into_iter()
        .find(|f| f.target_file == file)
        .map(|f| f.stacks)
        .unwrap_or_default()
}

fn ids(blocks: &[StackBlock]) -> Vec<&str> {
    blocks.iter().map(|b| b.patch_id.as_str()).collect()
}

const SRC: &str = "function love.load()\n  setup()\nend\n";

fn borderless_like() -> Vec<Patch> {
    vec![
        pat(
            "conf.lua",
            "gcore:1",
            "  setup()",
            InsertPosition::After,
            "  gcore_after()",
        ),
        pat(
            "conf.lua",
            "to:1",
            "  setup()",
            InsertPosition::After,
            "  to_after()",
        ),
        pat(
            "conf.lua",
            "to:2",
            "  setup()",
            InsertPosition::Before,
            "  to_before()",
        ),
        pat(
            "conf.lua",
            "gcore:2",
            "  setup()",
            InsertPosition::Before,
            "  gcore_before()",
        ),
    ]
}

#[test]
fn legacy_lists_stacks_in_the_older_order() {
    let s = stacks(
        &table(borderless_like(), StackMode::Legacy),
        "conf.lua",
        SRC,
    );
    assert_eq!(s.len(), 2, "{s:#?}");
    let before = &s[0];
    assert_eq!(before.side, Side::Before);
    assert_eq!(ids(&before.blocks), ["to:2", "gcore:2"]);
    assert_eq!(before.legacy, ["to:2", "gcore:2"]);
    assert!(!before.moved);
    let after = &s[1];
    assert_eq!(after.side, Side::After);
    assert_eq!(after.mode, StackMode::Legacy);
    assert_eq!(after.target_file, "conf.lua");
    assert_eq!(after.anchor_text, "setup()");
    assert_eq!(after.site_label, "in love.load");
    assert_eq!(ids(&after.blocks), ["to:1", "gcore:1"]);
    assert_eq!(after.legacy, ["to:1", "gcore:1"]);
    assert!(!after.moved);
    assert!(after.blocks.iter().all(|b| b.reason.is_none()));
    // The before stack, the shared line, the after stack.
    assert_eq!((before.file_line, after.file_line), (4, 4));
    assert_eq!(
        (before.blocks[0].first_line, before.blocks[1].last_line),
        (2, 3)
    );
    assert_eq!(
        (after.blocks[0].first_line, after.blocks[1].last_line),
        (5, 6)
    );
    assert_eq!(after.blocks[0].patch_name, "name of to:1");
    assert_eq!(after.blocks[0].mod_id, "to");
}

#[test]
fn dependencies_mark_moved_stacks_with_reasons() {
    let s = stacks(
        &table(borderless_like(), StackMode::Dependencies),
        "conf.lua",
        SRC,
    );
    assert_eq!(s.len(), 2, "{s:#?}");
    // to requires gcore: gcore's code goes above to's on both sides.
    let before = &s[0];
    assert_eq!(ids(&before.blocks), ["gcore:2", "to:2"]);
    assert_eq!(before.legacy, ["to:2", "gcore:2"]);
    assert!(before.moved);
    assert_eq!(
        before.blocks[0].reason,
        Some(StackReason::AboveDependent { of: "to".into() })
    );
    assert_eq!(before.blocks[1].reason, None);
    let after = &s[1];
    assert_eq!(after.mode, StackMode::Dependencies);
    assert_eq!(ids(&after.blocks), ["gcore:1", "to:1"]);
    assert_eq!(after.legacy, ["to:1", "gcore:1"]);
    assert!(after.moved);
    assert_eq!(
        after.blocks[1].reason,
        Some(StackReason::BelowDependency { of: "gcore".into() })
    );
    assert_eq!(
        (after.blocks[0].first_line, after.blocks[1].last_line),
        (5, 6)
    );
}

#[test]
fn one_item_and_replacements_are_not_stacks() {
    let patches = vec![
        pat(
            "conf.lua",
            "gcore:1",
            "  setup()",
            InsertPosition::After,
            "  gcore_after()",
        ),
        pat("conf.lua", "to:1", "end", InsertPosition::At, "end -- to"),
        pat(
            "conf.lua",
            "gcore:2",
            "end -- to",
            InsertPosition::At,
            "end -- gcore",
        ),
    ];
    for mode in [
        StackMode::Legacy,
        StackMode::Dependencies,
        StackMode::LoadOrder,
    ] {
        let s = stacks(&table(patches.clone(), mode), "conf.lua", SRC);
        assert!(s.is_empty(), "{mode:?}: {s:#?}");
    }
}

#[test]
fn code_after_another_mods_new_line_is_its_own_stack() {
    // Two inserts after a line gcore added: a stack on that line, inside gcore's item.
    let patches = vec![
        pat(
            "conf.lua",
            "gcore:1",
            "  setup()",
            InsertPosition::After,
            "  hook()",
        ),
        pat(
            "conf.lua",
            "to:1",
            "  hook()",
            InsertPosition::After,
            "  to_hook()",
        ),
        pat(
            "conf.lua",
            "gcore:2",
            "  hook()",
            InsertPosition::After,
            "  gcore_hook()",
        ),
    ];
    let s = stacks(&table(patches, StackMode::Dependencies), "conf.lua", SRC);
    assert_eq!(s.len(), 1, "{s:#?}");
    assert_eq!(s[0].anchor_text, "hook()");
    assert_eq!(s[0].file_line, 3);
    assert_eq!(ids(&s[0].blocks), ["gcore:2", "to:1"]);
    assert_eq!(s[0].legacy, ["gcore:2", "to:1"]);
    assert!(!s[0].moved);
}

#[test]
fn a_stack_after_a_function_header_is_inside_the_function() {
    let patches = || {
        vec![
            pat("main.lua", "gcore:1", "function love.load()", InsertPosition::After, "  g()"),
            pat("main.lua", "to:1", "function love.load()", InsertPosition::After, "  t()"),
            pat("main.lua", "gcore:2", "function love.load()", InsertPosition::Before, "g2()"),
            pat("main.lua", "to:2", "function love.load()", InsertPosition::Before, "t2()"),
        ]
    };
    let s = stacks(&table(patches(), StackMode::Dependencies), "main.lua", SRC);
    assert_eq!(s.len(), 2, "{s:#?}");
    assert_eq!(s[0].side, Side::Before);
    assert_eq!(s[0].site_label, "outside any function");
    assert_eq!(s[1].side, Side::After);
    assert_eq!(s[1].site_label, "in love.load");
}

#[test]
fn a_file_with_no_functions_is_labelled_outside_any_function() {
    let patches = vec![
        pat("data.lua", "gcore:1", "b", InsertPosition::After, "g"),
        pat("data.lua", "to:1", "b", InsertPosition::After, "t"),
    ];
    let s = stacks(
        &table(patches, StackMode::Dependencies),
        "data.lua",
        "a
b
c
",
    );
    assert_eq!(s.len(), 1, "{s:#?}");
    assert_eq!(s[0].site_label, "outside any function");
    assert_eq!(ids(&s[0].blocks), ["gcore:1", "to:1"]);
    assert_eq!(s[0].legacy, ["to:1", "gcore:1"]);
}

#[test]
fn stacked_comments_are_not_anchors_in_comments() {
    // Each mod's code has a comment on the side away from its anchor. In a stack that comment
    // sits between the next patch and its anchor, which is not a placement warning.
    let patches = vec![
        pat(
            "conf.lua",
            "gcore:1",
            "  setup()",
            InsertPosition::After,
            "  g()\n  -- gcore",
        ),
        pat(
            "conf.lua",
            "to:1",
            "  setup()",
            InsertPosition::After,
            "  t()\n  -- to",
        ),
        pat(
            "conf.lua",
            "gcore:2",
            "  setup()",
            InsertPosition::Before,
            "  -- gcore\n  g0()",
        ),
        pat(
            "conf.lua",
            "to:2",
            "  setup()",
            InsertPosition::Before,
            "  -- to\n  t0()",
        ),
    ];
    for mode in [
        StackMode::Legacy,
        StackMode::Dependencies,
        StackMode::LoadOrder,
    ] {
        let t = table(patches.clone(), mode);
        let vanilla = HashMap::from([("conf.lua".to_string(), SRC.to_string())]);
        let r = simulate_and_analyze_patches(&t, &vanilla);
        assert!(r.conflicts.is_empty(), "{mode:?}: {:#?}", r.conflicts);
        assert_eq!(r.files_touched[0].stacks.len(), 2, "{mode:?}");
    }
}

/// x's insert-only `at` puts its line under `L`; GCore and TO add code after `L` (the L5 shape).
fn l5_like() -> Vec<Patch> {
    vec![
        pat("l5.lua", "gcore:1", "L", InsertPosition::After, "g"),
        pat("l5.lua", "x:1", "L", InsertPosition::At, "L\nr"),
        pat("l5.lua", "to:1", "L", InsertPosition::After, "t"),
    ]
}

#[test]
fn implicit_anchors_make_a_stack_only_while_implicit_chains_are_on() {
    // Legacy with the line rules on: chains ignore implicit anchors, so the stack view lists
    // nothing the older rules do not (D51).
    for mode in [StackMode::Legacy, StackMode::Dependencies] {
        let mut t = table(l5_like(), mode);
        let off = stacks(&t, "l5.lua", "L\nE\n");
        t.line_rules_v2 = true;
        let on = stacks(&t, "l5.lua", "L\nE\n");
        if mode == StackMode::Legacy {
            assert_eq!(on, off, "Legacy");
            assert!(on.is_empty(), "{on:#?}");
        } else {
            assert!(off.is_empty(), "{off:#?}");
            assert_eq!(on.len(), 1, "{on:#?}");
            assert_eq!(ids(&on[0].blocks), ["x:1", "gcore:1", "to:1"]);
        }
    }
}

fn cp(id: &str, position: qm_core::patch::copy::CopyPosition, payload: &str) -> Patch {
    Patch::Copy(qm_core::patch::copy::CopyPatch {
        position,
        target: Target::Single("conf.lua".to_string()),
        sources: None,
        payload: Some(payload.to_string()),
        id: Some(id.to_string()),
        name: None,
        priority: None,
        before: Vec::new(),
        after: Vec::new(),
        on_failure: OnFailurePolicy::Warn,
        mod_id: id.split(':').next().map(str::to_string),
        disabled: false,
        qm: Default::default(),
        contents: Vec::new(),
    })
}

#[test]
fn copies_stack_at_the_far_end_in_the_older_order() {
    // D53: a copy prepend hangs before the first line and goes on top; a pattern insert
    // before that line goes right above it. Two appends hang after the last line, newest at
    // the bottom. Legacy shows them in the older order, not moved.
    use qm_core::patch::copy::CopyPosition::{Append, Prepend};
    let patches = || {
        vec![
            cp("to:c1", Prepend, "to_top()"),
            cp("gcore:c2", Append, "gcore_tail()"),
            cp("to:c3", Append, "to_tail()"),
            pat(
                "conf.lua",
                "gcore:1",
                "function love.load()",
                InsertPosition::Before,
                "gcore_before()",
            ),
        ]
    };
    let mut t = table(patches(), StackMode::Legacy);
    t.targets.insert("conf.lua".to_string());
    let s = stacks(&t, "conf.lua", SRC);
    assert_eq!(s.len(), 2, "{s:#?}");
    assert_eq!(ids(&s[0].blocks), ["to:c1", "gcore:1"]);
    assert_eq!(s[0].legacy, ["to:c1", "gcore:1"]);
    assert!(!s[0].moved);
    assert_eq!(ids(&s[1].blocks), ["gcore:c2", "to:c3"]);
    assert_eq!(s[1].legacy, ["gcore:c2", "to:c3"]);
    assert!(!s[1].moved);

    // Dependencies puts GCore's pattern above TO's copy: moved from the older order.
    let mut t = table(patches(), StackMode::Dependencies);
    t.targets.insert("conf.lua".to_string());
    let s = stacks(&t, "conf.lua", SRC);
    assert_eq!(ids(&s[0].blocks), ["gcore:1", "to:c1"]);
    assert_eq!(s[0].legacy, ["to:c1", "gcore:1"]);
    assert!(s[0].moved);
    assert_eq!(
        s[0].blocks[0].reason,
        Some(StackReason::AboveDependent { of: "to".into() })
    );
    assert!(!s[1].moved);
}

/// D55: a user rule moves a block in Legacy; the stack names its line by key and the rule.
#[test]
fn a_stack_rule_shows_its_fix_and_the_line_key() {
    use qm_core::patch::stack::{line_key, RuleSource, StackRuleSpec};
    let lines: Vec<&str> = SRC.lines().collect();
    let key = line_key(&lines, |_| false, 1);
    let mut t = table(borderless_like(), StackMode::Legacy);
    // to:1 runs last and goes on top today; the rule on gcore:1 puts gcore's block above it,
    // so it moves to:1 when to:1 runs.
    for (p, _, _) in &mut t.patches {
        if p.id() == Some("gcore:1") {
            p.extra_mut().stack_rules.push(StackRuleSpec {
                site: Some(key.clone()),
                side: Some(Side::After),
                below: None,
                above: Some("to:1".into()),
                source: RuleSource::Profile {
                    fix_id: "fx_1".into(),
                },
            });
        }
    }
    let s = stacks(&t, "conf.lua", SRC);
    let after = s.iter().find(|x| x.side == Side::After).expect("after stack");
    assert_eq!(ids(&after.blocks), ["gcore:1", "to:1"]);
    assert!(after.moved);
    assert_eq!(after.site, key);
    assert_eq!(
        after.stack_key,
        format!("conf.lua#{}#1#after", key.context)
    );
    assert_eq!(after.blocks[1].rule_fix_id.as_deref(), Some("fx_1"));
    assert_eq!(
        after.blocks[1].reason,
        Some(StackReason::UserRule {
            fix_id: "fx_1".into()
        })
    );
    assert!(after.blocks[0].rule_fix_id.is_none());
    assert!(after.blocks.iter().all(|b| b.locked_reason.is_none()));
    // The before stack has its own key.
    let before = s.iter().find(|x| x.side == Side::Before).expect("before stack");
    assert_eq!(before.site, key);
    assert_ne!(before.stack_key, after.stack_key);
}
