use super::*;
use crate::handshake::PatchOutcomeStatus;
use crate::patch::{InsertPosition, OnFailurePolicy, Patch, PatternPatch, Target};
use std::path::PathBuf;

/// The DLL's inline code before Task 6 (`lib.rs:511-548`), kept to prove the move. The file
/// test is the root-file rule of D44 (it was a suffix test).
fn old_inline(name: &str, buf_str: &str, launch_config: &LaunchConfig) -> (String, bool) {
    let root = |file: &str| chunk_file(name).is_root(file);
    let mut current_buf = buf_str.to_string();
    let mut was_safety_patched = false;
    if root(CONF_LUA) {
        if let Some(ref custom_identity) = launch_config.save_identity_override {
            if !custom_identity.trim().is_empty() {
                current_buf = current_buf.replace(
                    "t.identity = nil",
                    &format!("t.identity = {:?}", custom_identity),
                );
                was_safety_patched = true;
            }
        }
    }
    if root(MAIN_LUA) {
        current_buf.insert_str(0, STEAM_SAFETY_PAYLOAD);
        current_buf.push('\n');
        current_buf.push_str(QM_RUNTIME_FOOTER_PAYLOAD);
        was_safety_patched = true;
    }
    if root(EXTERNAL_LUA) && current_buf.contains("function uploadScoresHelper") {
        current_buf = current_buf.replace(
            "function uploadScoresHelper(handle, scorez)",
            "function uploadScoresHelper(handle, scorez) return false end function _disabled_uploadScoresHelper(handle, scorez)",
        );
        was_safety_patched = true;
    }
    (current_buf, was_safety_patched)
}

const MAIN: &str = "local x = 1\nfunction love.load()\nend\n";
const CONF: &str = "function love.conf(t)\n  t.identity = nil\n  t.window = nil\nend\n";
const EXTERNAL: &str = "local a = 1\nfunction uploadScoresHelper(handle, scorez)\n  send()\nend\n";

fn launch(identity: Option<&str>) -> LaunchConfig {
    LaunchConfig {
        save_identity_override: identity.map(str::to_string),
        ..Default::default()
    }
}

fn prefix_lines() -> usize {
    STEAM_SAFETY_PAYLOAD.matches('\n').count()
}

#[test]
fn prepare_buffer_equals_the_old_inline_code() {
    let chunks = [
        ("@conf.lua", CONF),
        ("conf.lua", CONF),
        ("@main.lua", MAIN),
        ("main.lua", MAIN),
        ("@main.lua", "local x = 1"),
        ("@main.lua", ""),
        ("@includes/external.lua", EXTERNAL),
        ("@includes/external.lua", "function uploadScoresHelper(h, s)\nend\n"),
        ("@includes/external.lua", "local nothing = 1\n"),
        ("@includes/other.lua", MAIN),
        ("=[love \"main.lua\"]", MAIN),
        ("@./main.lua", MAIN),
        (".\\main.lua", MAIN),
        ("@C:/Games/MnB/main.lua", MAIN),
        ("@./includes/external.lua", EXTERNAL),
        (".\\conf.lua", CONF),
        ("@domain.lua", MAIN),
        ("@x/main.lua", MAIN),
        ("=stdin", MAIN),
    ];
    for identity in [None, Some("mnb_save2"), Some("  ")] {
        let cfg = launch(identity);
        for (name, text) in chunks {
            let new = prepare_buffer(name, text, &cfg);
            let (old, touched) = old_inline(name, text, &cfg);
            assert_eq!(new.text, old, "{name} with {identity:?}");
            assert_eq!(new.touched, touched, "{name} with {identity:?}");
        }
    }
}

#[test]
fn main_lua_builtin_ranges_cover_prefix_and_footer() {
    for vanilla in [MAIN, "local x = 1", ""] {
        let p = prepare_buffer("@main.lua", vanilla, &LaunchConfig::default());
        let lines: Vec<&str> = p.text.lines().collect();
        let n = prefix_lines();
        let v = vanilla.lines().count();
        assert_eq!(p.builtin, vec![0..n, n + v..lines.len()], "{vanilla:?}");
        assert_eq!(
            lines.get(n..n + v).unwrap(),
            vanilla.lines().collect::<Vec<_>>()
        );
        assert!(p.builtin_edited.is_empty());
        assert!(p.has_builtin());
        assert!(p.is_builtin(0) && !p.is_builtin(n) || v == 0);
    }
}

#[test]
fn builtin_edited_lists_the_identity_and_upload_lines() {
    let conf = prepare_buffer("@conf.lua", CONF, &launch(Some("mnb_save2")));
    assert_eq!(conf.builtin_edited, vec![1]);
    assert!(conf.builtin.is_empty());
    assert_eq!(conf.text.lines().count(), CONF.lines().count());
    assert_eq!(
        conf.text.lines().nth(1),
        Some("  t.identity = \"mnb_save2\"")
    );

    let plain = prepare_buffer("@conf.lua", CONF, &LaunchConfig::default());
    assert!(plain.builtin_edited.is_empty() && !plain.touched && !plain.has_builtin());

    let ext = prepare_buffer("@includes/external.lua", EXTERNAL, &LaunchConfig::default());
    assert_eq!(ext.builtin_edited, vec![1]);
    assert_eq!(ext.text.lines().count(), EXTERNAL.lines().count());
}

fn pattern(
    id: &str,
    target: &str,
    pattern: &str,
    position: InsertPosition,
    payload: &str,
) -> Patch {
    Patch::Pattern(PatternPatch {
        pattern: pattern.to_string(),
        position,
        target: Target::Single(target.to_string()),
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
        mod_id: Some("mod_a".to_string()),
        disabled: false,
        qm: Default::default(),
    })
}

fn table(patches: Vec<Patch>) -> PatchTable {
    let mut t = PatchTable::default();
    for p in patches {
        if let Patch::Pattern(x) = &p {
            x.target.insert_into(&mut t.targets);
        }
        t.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
    }
    t
}

fn records_of(outcome: &DllOutcome) -> &[PatchExecutionRecord] {
    match outcome {
        DllOutcome::Patched { records, .. } => records,
        other => panic!("expected a patched outcome, got {other:?}"),
    }
}

#[test]
fn a_match_on_upload_scores_helper_counts_as_a_builtin_match() {
    let t = table(vec![
        pattern(
            "hook_upload",
            "external.lua",
            "function uploadScoresHelper*",
            InsertPosition::Before,
            "-- before upload",
        ),
        pattern(
            "plain",
            "external.lua",
            "local a = 1",
            InsertPosition::After,
            "-- after a",
        ),
    ]);
    let prep = t.prepare();
    let out = dll_transform(
        &t,
        &prep,
        &LaunchConfig::default(),
        "@includes/external.lua",
        EXTERNAL,
        false,
    );
    let recs = records_of(&out);
    let hook = recs.iter().find(|r| r.patch_id == "hook_upload").unwrap();
    assert!(is_builtin_match(hook), "{:?}", hook.warnings);
    let plain = recs.iter().find(|r| r.patch_id == "plain").unwrap();
    assert!(!is_builtin_match(plain), "{:?}", plain.warnings);
    assert_eq!(recs.iter().filter(|r| is_builtin_match(r)).count(), 1);
}

fn regex(id: &str, re: &str, position: &str, payload: &str) -> Patch {
    let mut p: Patch = serde_json::from_value(serde_json::json!({"regex": {
        "target": "main.lua", "pattern": re, "position": position, "payload": payload,
        "id": id, "name": null, "priority": null, "root_capture": null
    }}))
    .unwrap();
    p.set_metadata("mod_a".to_string(), id.to_string(), None);
    p
}

fn regex_table(patches: Vec<Patch>) -> PatchTable {
    let mut t = PatchTable::default();
    Target::Single("main.lua".to_string()).insert_into(&mut t.targets);
    for p in patches {
        t.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
    }
    t
}

#[test]
fn regex_inserts_at_the_vanilla_edges_are_not_builtin_matches() {
    let t = regex_table(vec![
        regex("first", r"(?m)^local x = 1\n", "before", "-- first\n"),
        // "end" alone would also match QuarterMaster's own code.
        regex(
            "last",
            r"(?m)^function love\.load\(\)\nend\n",
            "after",
            "-- last\n",
        ),
    ]);
    let prep = t.prepare();
    let out = dll_transform(&t, &prep, &LaunchConfig::default(), "@main.lua", MAIN, false);
    let recs = records_of(&out);
    for id in ["first", "last"] {
        let r = recs.iter().find(|r| r.patch_id == id).unwrap();
        assert!(r.status == crate::handshake::PatchOutcomeStatus::Applied, "{id}: {r:?}");
        assert!(!is_builtin_match(r), "{id}: {:?}", r.warnings);
    }
}

#[test]
fn identity_line_and_payload_lines_count_as_builtin_matches() {
    let t = table(vec![
        pattern(
            "on_identity",
            "conf.lua",
            "t.identity = *",
            InsertPosition::After,
            "-- after identity",
        ),
        pattern(
            "on_window",
            "conf.lua",
            "t.window = nil",
            InsertPosition::At,
            "  t.window = {}",
        ),
    ]);
    let prep = t.prepare();
    let out = dll_transform(
        &t,
        &prep,
        &launch(Some("mnb_save2")),
        "@conf.lua",
        CONF,
        false,
    );
    let recs = records_of(&out);
    assert!(is_builtin_match(
        recs.iter().find(|r| r.patch_id == "on_identity").unwrap()
    ));
    assert!(!is_builtin_match(
        recs.iter().find(|r| r.patch_id == "on_window").unwrap()
    ));

    // A match inside the Steam safety prefix of main.lua.
    let first_payload_line = STEAM_SAFETY_PAYLOAD
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap();
    let t = table(vec![
        pattern(
            "on_payload",
            "main.lua",
            first_payload_line.trim(),
            InsertPosition::After,
            "-- after qm",
        ),
        pattern(
            "on_vanilla",
            "main.lua",
            "local x = 1",
            InsertPosition::After,
            "-- after x",
        ),
    ]);
    let prep = t.prepare();
    let out = dll_transform(
        &t,
        &prep,
        &LaunchConfig::default(),
        "@main.lua",
        MAIN,
        false,
    );
    let recs = records_of(&out);
    // D42: the Steam block is never matched. The record says so and counts as a builtin match.
    let on_payload = recs.iter().find(|r| r.patch_id == "on_payload").unwrap();
    assert!(is_builtin_match(on_payload));
    assert_eq!(on_payload.status, PatchOutcomeStatus::ZeroMatches);
    assert!(
        on_payload
            .warnings
            .iter()
            .any(|w| w.starts_with(BUILTIN_MATCH_SKIPPED)),
        "{:?}",
        on_payload.warnings
    );
    assert!(!is_builtin_match(
        recs.iter().find(|r| r.patch_id == "on_vanilla").unwrap()
    ));
    match &out {
        DllOutcome::Patched { text, .. } => {
            assert!(text.starts_with(STEAM_SAFETY_PAYLOAD), "nothing inside the block");
            assert!(!text.contains("-- after qm"));
        }
        other => panic!("{other:?}"),
    }
}

/// An `at` on the save identity line would undo the save sandbox: refused, and the override
/// stays (D42). An insert next to it still applies.
#[test]
fn an_at_on_the_identity_line_is_refused() {
    let t = table(vec![
        pattern(
            "at_identity",
            "conf.lua",
            "t.identity = *",
            InsertPosition::At,
            "  t.identity = nil",
        ),
        pattern(
            "after_identity",
            "conf.lua",
            "t.identity = *",
            InsertPosition::After,
            "  -- after identity",
        ),
    ]);
    let prep = t.prepare();
    let cfg = launch(Some("mnb_save2"));
    let out = dll_transform(&t, &prep, &cfg, "@conf.lua", CONF, false);
    let recs = records_of(&out);
    let at = recs.iter().find(|r| r.patch_id == "at_identity").unwrap();
    assert_eq!(at.status, PatchOutcomeStatus::ZeroMatches);
    let skip = at
        .warnings
        .iter()
        .find(|w| w.starts_with(BUILTIN_MATCH_SKIPPED))
        .unwrap_or_else(|| panic!("{:?}", at.warnings));
    assert!(skip.contains("line 2 of 'conf.lua'"), "{skip}");
    let after = recs.iter().find(|r| r.patch_id == "after_identity").unwrap();
    assert_eq!(after.status, PatchOutcomeStatus::Applied);
    match &out {
        DllOutcome::Patched { text, .. } => assert!(
            text.contains("  t.identity = \"mnb_save2\"\n  -- after identity\n"),
            "{text}"
        ),
        other => panic!("{other:?}"),
    }
    let (_, sim) = simulate_buffer(&t, &prep, &cfg, "conf.lua", CONF).unwrap();
    assert_eq!(
        serde_json::to_string(&sim.records).unwrap(),
        serde_json::to_string(recs).unwrap()
    );
    // Without an override the line is the game's own, so the `at` applies.
    let plain = dll_transform(&t, &prep, &LaunchConfig::default(), "@conf.lua", CONF, false);
    let at = records_of(&plain).iter().find(|r| r.patch_id == "at_identity").unwrap();
    assert_eq!(at.status, PatchOutcomeStatus::Applied);
}

fn ext_regex(id: &str, re: &str, position: &str, payload: &str) -> Patch {
    let mut p: Patch = serde_json::from_value(serde_json::json!({"regex": {
        "target": "external.lua", "pattern": re, "position": position, "payload": payload,
        "id": id, "name": null, "priority": null, "root_capture": null
    }}))
    .unwrap();
    p.set_metadata("mod_a".to_string(), id.to_string(), None);
    p
}

/// A regex edit on the defused line is dropped, an insert inside it too; a whole-line insert
/// next to it applies (D42).
#[test]
fn regex_edits_on_the_defused_line() {
    let mut t = PatchTable::default();
    Target::Single("external.lua".to_string()).insert_into(&mut t.targets);
    for p in [
        ext_regex("rewrite", r"uploadScoresHelper\(handle", "at", "uploadScoresHelper(h"),
        ext_regex(
            "inline",
            r"function uploadScoresHelper\(handle, scorez\)",
            "after",
            " send(scorez)",
        ),
        ext_regex(
            "whole_line",
            r"(?m)^function uploadScoresHelper",
            "before",
            "-- regex before\n",
        ),
    ] {
        t.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
    }
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    let out = dll_transform(&t, &prep, &cfg, "@includes/external.lua", EXTERNAL, false);
    let recs = records_of(&out);
    let status = |id: &str| recs.iter().find(|r| r.patch_id == id).unwrap().status;
    assert_eq!(status("rewrite"), PatchOutcomeStatus::ZeroMatches);
    assert_eq!(status("inline"), PatchOutcomeStatus::ZeroMatches);
    assert_eq!(status("whole_line"), PatchOutcomeStatus::Applied);
    match &out {
        DllOutcome::Patched { text, .. } => {
            let defused = "function uploadScoresHelper(handle, scorez) return false end function _disabled_uploadScoresHelper(handle, scorez)";
            assert!(text.contains(&format!("-- regex before\n{defused}\n")), "{text}");
            assert!(!text.contains("send(scorez)"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn dll_transform_outcomes() {
    let t = table(vec![pattern(
        "m",
        "main.lua",
        "local x = 1",
        InsertPosition::After,
        "-- mod",
    )]);
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    // Not targeted and nothing to prepare: passthrough, unless dumping everything.
    assert!(matches!(
        dll_transform(&t, &prep, &cfg, "@includes/other.lua", "x\n", false),
        DllOutcome::Passthrough
    ));
    match dll_transform(&t, &prep, &cfg, "@includes/other.lua", "x\n", true) {
        DllOutcome::Prepared(text) => assert_eq!(text, "x\n"),
        other => panic!("{other:?}"),
    }
    // Not targeted but prepared: QuarterMaster's edits only.
    match dll_transform(&t, &prep, &cfg, "@includes/external.lua", EXTERNAL, false) {
        DllOutcome::Prepared(text) => assert!(text.contains("return false end")),
        other => panic!("{other:?}"),
    }
    // Targeted: the mod patches the prepared text.
    match dll_transform(&t, &prep, &cfg, "@main.lua", MAIN, false) {
        DllOutcome::Patched { text, .. } => {
            assert!(text.starts_with(STEAM_SAFETY_PAYLOAD));
            assert!(text.contains("local x = 1\n-- mod\n"));
            assert!(text.ends_with(QM_RUNTIME_FOOTER_PAYLOAD));
        }
        other => panic!("{other:?}"),
    }
    // An abort fails the chunk.
    let mut abort = pattern(
        "a",
        "main.lua",
        "no such line",
        InsertPosition::After,
        "-- x",
    );
    if let Patch::Pattern(p) = &mut abort {
        p.on_failure = OnFailurePolicy::Abort;
    }
    let t = table(vec![abort]);
    let prep = t.prepare();
    assert!(matches!(
        dll_transform(&t, &prep, &cfg, "@main.lua", MAIN, false),
        DllOutcome::SyntaxError(_)
    ));
}

#[test]
fn pretty_names() {
    assert_eq!(pretty_name("@main.lua"), "main.lua");
    assert_eq!(pretty_name("main.lua"), "main.lua");
    assert_eq!(
        pretty_name("@includes/directives.lua"),
        "includes/directives.lua"
    );
    assert_eq!(pretty_name("=[love \"main.lua\"]"), "love//main.lua");
    assert_eq!(pretty_name("=[qm mod.sub \"x.lua\"]"), "qm/mod/sub/x.lua");
}

#[test]
fn file_view_hides_builtin_lines_and_keeps_file_numbers() {
    let t = table(vec![pattern(
        "m",
        "main.lua",
        "local x = 1",
        InsertPosition::After,
        "-- mod",
    )]);
    let prep = t.prepare();
    let (buf, traced) =
        simulate_buffer(&t, &prep, &LaunchConfig::default(), "main.lua", MAIN).unwrap();
    let view = FileView::new(&traced.trace, &buf);
    assert_eq!(
        view.trace.text,
        "local x = 1\n-- mod\nfunction love.load()\nend\n"
    );
    assert_eq!(view.trace.lines.len(), 4);
    assert_eq!(view.trace.lines[0], LineOrigin::Vanilla(1));
    assert!(matches!(view.trace.lines[1], LineOrigin::Inserted { .. }));
    assert_eq!(view.trace.lines[2], LineOrigin::Vanilla(2));
    assert_eq!(view.trace.runs_of_record(0), vec![(2, 2)]);
    let n = prefix_lines();
    assert_eq!(view.line(1), None);
    assert_eq!(view.line(n + 1), Some(1));
    assert_eq!(view.line(n + 2), Some(2));
    assert_eq!(view.line(traced.trace.lines.len()), None);
    assert_eq!(view.line_at_or_after(1), 1);
    assert_eq!(view.line_at_or_before(traced.trace.lines.len()), 4);
    assert_eq!(
        view.regions(&[PatchRegion {
            start_line: n + 2,
            end_line: n + 2
        }]),
        vec![PatchRegion {
            start_line: 2,
            end_line: 2
        }]
    );

    // In-place edits stay, with their file numbers.
    let t = table(vec![pattern(
        "w",
        "conf.lua",
        "t.window = nil",
        InsertPosition::After,
        "-- w",
    )]);
    let prep = t.prepare();
    let (buf, traced) =
        simulate_buffer(&t, &prep, &launch(Some("mnb_save2")), "conf.lua", CONF).unwrap();
    let view = FileView::new(&traced.trace, &buf);
    assert_eq!(view.trace.text, traced.text);
    assert_eq!(view.trace.lines[1], LineOrigin::Vanilla(2));
    assert_eq!(view.line(2), Some(2));
}

#[test]
fn simulate_buffer_equals_dll_transform() {
    let t = table(vec![
        pattern(
            "m",
            "main.lua",
            "local x = 1",
            InsertPosition::After,
            "-- mod",
        ),
        pattern(
            "e",
            "external.lua",
            "function uploadScoresHelper*",
            InsertPosition::Before,
            "-- e",
        ),
    ]);
    let prep = t.prepare();
    let cfg = launch(Some("mnb_save2"));
    for (file, text) in [("main.lua", MAIN), ("includes/external.lua", EXTERNAL)] {
        let (_, sim) = simulate_buffer(&t, &prep, &cfg, file, text).unwrap();
        for name in [format!("@{file}"), file.to_string()] {
            match dll_transform(&t, &prep, &cfg, &name, text, false) {
                DllOutcome::Patched { text, records, .. } => {
                    assert_eq!(text, sim.text, "{name}");
                    assert_eq!(
                        serde_json::to_string(&records).unwrap(),
                        serde_json::to_string(&sim.records).unwrap(),
                        "{name}"
                    );
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }
}

#[test]
fn chunk_names_name_one_file() {
    let game = |p: &str| ChunkFile::Game(p.to_string());
    for (name, want) in [
        ("@main.lua", game("main.lua")),
        ("main.lua", game("main.lua")),
        ("@./main.lua", game("main.lua")),
        (".\\main.lua", game("main.lua")),
        ("=main.lua", game("main.lua")),
        ("@includes\\x.lua", game("includes/x.lua")),
        ("@C:\\Games\\MnB\\main.lua", game("C:/Games/MnB/main.lua")),
        ("@domain.lua", game("domain.lua")),
        ("@x/main.lua", game("x/main.lua")),
        ("=[love \"main.lua\"]", ChunkFile::Love("main.lua".to_string())),
        (
            "=[qm includes/ser \"lib\\ser.lua\"]",
            ChunkFile::Module {
                name: "includes/ser".to_string(),
                source: "lib\\ser.lua".to_string(),
            },
        ),
        ("=stdin", ChunkFile::Other),
        ("=(load)", ChunkFile::Other),
        ("=[string \"main.lua\"]", ChunkFile::Other),
        ("local f = 1\n-- main.lua", ChunkFile::Other),
        ("return 1", ChunkFile::Other),
    ] {
        assert_eq!(chunk_file(name), want, "{name:?}");
    }
    assert_eq!(canonical_path("@././a.lua"), "a.lua");
    assert_eq!(canonical_path(".\\a\\b.lua"), "a/b.lua");
    assert_eq!(canonical_path("@@a.lua"), "@a.lua", "one @ only");
}

#[test]
fn root_files_match_by_root_path() {
    for path in ["main.lua", "C:/Games/MnB/main.lua", "/game/main.lua", "d:/main.lua"] {
        assert!(is_root_file(path, MAIN_LUA), "{path}");
    }
    for path in ["domain.lua", "x/main.lua", "C:/Games/MnB/domain.lua", "1:/main.lua", ""] {
        assert!(!is_root_file(path, MAIN_LUA), "{path}");
    }
    assert!(is_root_file("includes/external.lua", EXTERNAL_LUA));
    assert!(is_root_file("C:/MnB/includes/external.lua", EXTERNAL_LUA));
    assert!(!is_root_file("external.lua", EXTERNAL_LUA));
    assert!(!is_root_file("x/includes/external.lua", EXTERNAL_LUA));
}

fn main_table(target: &str) -> PatchTable {
    table(vec![pattern(
        "m",
        target,
        "local x = 1",
        InsertPosition::After,
        "-- mod",
    )])
}

/// Every name of the game's main.lua gets the same text and records, with the Steam block
/// and the footer (D44). An absolute name differs only in the record's `target_file`.
#[test]
fn every_main_lua_name_gets_the_builtins_and_the_patches() {
    let t = main_table("main.lua");
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    let run = |name: &str| match dll_transform(&t, &prep, &cfg, name, MAIN, false) {
        DllOutcome::Patched {
            text, mut records, ..
        } => {
            for r in &mut records {
                r.target_file = "main.lua".to_string();
            }
            (text, serde_json::to_string(&records).unwrap())
        }
        other => panic!("{name}: {other:?}"),
    };
    let want = run("@main.lua");
    assert!(want.0.starts_with(STEAM_SAFETY_PAYLOAD));
    assert!(want.0.ends_with(QM_RUNTIME_FOOTER_PAYLOAD));
    assert!(want.0.contains("local x = 1\n-- mod\n"));
    for name in [
        "main.lua",
        "@./main.lua",
        ".\\main.lua",
        "@C:/Games/MnB/main.lua",
        "@/game/main.lua",
    ] {
        assert_eq!(run(name), want, "{name}");
    }
}

/// Chunks that are not the game's main.lua get no Steam block or footer, and chunks that are
/// not game files get no mod patch either (D44).
#[test]
fn other_chunk_names_get_no_builtins() {
    let t = main_table("main.lua");
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    let code = "local x = 1\n-- main.lua";
    for (name, buf) in [("=stdin", MAIN), ("=(load)", MAIN), (code, code)] {
        assert!(
            matches!(
                dll_transform(&t, &prep, &cfg, name, buf, false),
                DllOutcome::Passthrough
            ),
            "{name:?}"
        );
        match dll_transform(&t, &prep, &cfg, name, buf, true) {
            DllOutcome::Prepared(text) => assert_eq!(text, buf, "{name:?}"),
            other => panic!("{name:?}: {other:?}"),
        }
    }
    // Game files, but not the root main.lua: no builtins. Mod targets keep the legacy
    // suffix rule (D45), so a `main.lua` target still patches them.
    for name in ["@domain.lua", "@x/main.lua"] {
        match dll_transform(&t, &prep, &cfg, name, MAIN, false) {
            DllOutcome::Patched { text, .. } => {
                assert_eq!(
                    text, "local x = 1\n-- mod\nfunction love.load()\nend\n",
                    "{name}"
                );
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    // With no target on them, they load as they are.
    let conf = table(vec![pattern(
        "c",
        "conf.lua",
        "t.window = nil",
        InsertPosition::After,
        "-- c",
    )]);
    let prep = conf.prepare();
    for name in ["@domain.lua", "@x/main.lua", "=stdin", "=(load)", code] {
        assert!(
            matches!(
                dll_transform(&conf, &prep, &cfg, name, MAIN, false),
                DllOutcome::Passthrough
            ),
            "{name:?}"
        );
    }
}

/// LÖVE's own `=[love "main.lua"]` is not a game file: it loads as it is, as the simulation
/// shows it, and is kept only for a full dump (D44).
#[test]
fn the_love_form_is_passthrough() {
    let t = main_table("main.lua");
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    let love = "=[love \"main.lua\"]";
    assert!(matches!(
        dll_transform(&t, &prep, &cfg, love, MAIN, false),
        DllOutcome::Passthrough
    ));
    match dll_transform(&t, &prep, &cfg, love, MAIN, true) {
        DllOutcome::Prepared(text) => assert_eq!(text, MAIN),
        other => panic!("{other:?}"),
    }
    assert!(!prepare_buffer(love, MAIN, &cfg).touched);
    let (buf, sim) = simulate_buffer(&t, &prep, &cfg, love, MAIN).unwrap();
    assert_eq!((buf.text.as_str(), sim.text.as_str()), (MAIN, MAIN));
    assert!(sim.records.is_empty() && !buf.has_builtin());
}

/// A table target in another form of the same path still names the chunk (D44).
#[test]
fn a_dot_slash_target_patches_at_main_lua() {
    for target in ["./main.lua", "@./main.lua", ".\\main.lua", "main.lua"] {
        let t = main_table(target);
        assert!(t.needs_patching("@main.lua"), "{target}");
        let prep = t.prepare();
        match dll_transform(&t, &prep, &LaunchConfig::default(), "@main.lua", MAIN, false) {
            DllOutcome::Patched { text, records, .. } => {
                assert!(text.contains("local x = 1\n-- mod\n"), "{target}");
                assert_eq!(records.len(), 1, "{target}");
                assert_eq!(
                    records[0].status,
                    crate::handshake::PatchOutcomeStatus::Applied,
                    "{target}"
                );
            }
            other => panic!("{target}: {other:?}"),
        }
    }
}

fn copy(id: &str, position: &str, payload: &str) -> Patch {
    let mut p: Patch = serde_json::from_value(serde_json::json!({"copy": {
        "target": "main.lua", "position": position, "payload": payload,
        "sources": null, "id": id, "name": null, "priority": null
    }}))
    .unwrap();
    p.set_metadata("mod_a".to_string(), id.to_string(), None);
    p
}

fn copy_table(patches: Vec<Patch>) -> PatchTable {
    let mut t = PatchTable::default();
    Target::Single("main.lua".to_string()).insert_into(&mut t.targets);
    for p in patches {
        t.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
    }
    t
}

/// The game's lua51.dll, when it is there (compile checks are skipped without it).
fn compiler() -> Option<crate::compile::LuaCompiler> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        root.join("../../../game"),
        root.join("../../game"),
        PathBuf::from("D:/WindowsGames/SteamLibrary/steamapps/common/MudandBlood"),
    ]
    .iter()
    .find_map(|d| crate::compile::LuaCompiler::find(d))
    .and_then(|p| crate::compile::LuaCompiler::load(&p).ok())
}

/// D43: a main.lua prepend lands right after the Steam safety block and an append right
/// before the footer, as whole lines, and the file still compiles.
#[test]
fn main_lua_copies_land_inside_the_frame() {
    let t = copy_table(vec![
        copy("pre", "prepend", "local pre = 1"),
        copy("app", "append", "local app = 2"),
    ]);
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    let (_, sim) = simulate_buffer(&t, &prep, &cfg, "main.lua", MAIN).unwrap();
    let want = format!(
        "{STEAM_SAFETY_PAYLOAD}local pre = 1\n{MAIN}local app = 2\n\n{QM_RUNTIME_FOOTER_PAYLOAD}"
    );
    assert_eq!(sim.text, want);
    assert!(!sim.lines.resized(), "whole lines: the map never resized");
    for r in &sim.records {
        assert_eq!(r.status, PatchOutcomeStatus::Applied, "{r:?}");
        assert!(r.warnings.is_empty() && !is_builtin_match(r), "{r:?}");
    }
    match dll_transform(&t, &prep, &cfg, "@main.lua", MAIN, false) {
        DllOutcome::Patched { text, .. } => assert_eq!(text, want),
        other => panic!("{other:?}"),
    }
    // Other files have no frame: the bytes stay as they were (byte 0, and EOF after a '\n').
    let mut other = copy_table(vec![]);
    for (id, pos, payload) in [
        ("pre", "prepend", "local pre = 1"),
        ("app", "append", "local app = 2"),
    ] {
        let mut p = copy(id, pos, payload);
        if let Patch::Copy(c) = &mut p {
            c.target = Target::Single("includes/other.lua".to_string());
        }
        other.patches.push((p, 10, PathBuf::from("ModA/qm.toml")));
    }
    Target::Single("includes/other.lua".to_string()).insert_into(&mut other.targets);
    let prep = other.prepare();
    let (_, sim) = simulate_buffer(&other, &prep, &cfg, "includes/other.lua", MAIN).unwrap();
    assert_eq!(sim.text, format!("local pre = 1\n{MAIN}\nlocal app = 2"));

    let Some(c) = compiler() else {
        eprintln!("lua51.dll not found; compile check skipped");
        return;
    };
    assert!(c.compile("main.lua", &want).is_ok());
}

/// A main.lua append that ends in a top-level `return` keeps today's place after the footer,
/// with a warning, so the file still compiles (D43, §1.3 item 5).
#[test]
fn a_main_lua_append_ending_in_return_stays_after_the_footer() {
    let t = copy_table(vec![copy("ret", "append", "local M = {}\nreturn M")]);
    let prep = t.prepare();
    let cfg = LaunchConfig::default();
    let (_, sim) = simulate_buffer(&t, &prep, &cfg, "main.lua", MAIN).unwrap();
    let want = format!(
        "{STEAM_SAFETY_PAYLOAD}{MAIN}\n{QM_RUNTIME_FOOTER_PAYLOAD}\nlocal M = {{}}\nreturn M"
    );
    assert_eq!(sim.text, want);
    let rec = &sim.records[0];
    assert_eq!(rec.status, PatchOutcomeStatus::Applied);
    assert!(
        rec.warnings.iter().any(|w| w.starts_with(COPY_AFTER_FOOTER_RETURN)),
        "{:?}",
        rec.warnings
    );
    match dll_transform(&t, &prep, &cfg, "@main.lua", MAIN, false) {
        DllOutcome::Patched { text, records, .. } => {
            assert_eq!(text, want);
            assert_eq!(
                serde_json::to_string(&records).unwrap(),
                serde_json::to_string(&sim.records).unwrap()
            );
        }
        other => panic!("{other:?}"),
    }
    let Some(c) = compiler() else {
        eprintln!("lua51.dll not found; compile check skipped");
        return;
    };
    assert!(c.compile("main.lua", &want).is_ok());
    // Inside the frame it would not compile: code follows the return.
    let inside = format!(
        "{STEAM_SAFETY_PAYLOAD}{MAIN}local M = {{}}\nreturn M\n\n{QM_RUNTIME_FOOTER_PAYLOAD}"
    );
    assert!(c.compile("main.lua", &inside).is_err());
}
