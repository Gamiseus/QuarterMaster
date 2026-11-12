use std::path::{Path, PathBuf};

use super::*;
use crate::patch::plan::{load_raw, Fingerprints, FpMode};

fn patch(json: serde_json::Value) -> Patch {
    serde_json::from_value(json).unwrap()
}

fn pattern(pattern: &str, payload: &str) -> serde_json::Value {
    serde_json::json!({"pattern": {"target": "main.lua", "pattern": pattern,
        "position": "after", "payload": payload}})
}

fn fp(p: &Patch) -> PatchFingerprint {
    p.fingerprint(Path::new("C:\\Mods\\m"), "qm.toml", 1)
}

#[test]
fn line_ends_and_trailing_spaces_do_not_change_it() {
    let lf = patch(pattern("local a = 1\nlocal b = 2", "-- one\n-- two\n"));
    let crlf = patch(pattern(
        "local a = 1\r\nlocal b = 2",
        "-- one  \r\n-- two\r\n\r\n",
    ));
    let indented = patch(pattern("  local a = 1  \n\tlocal b = 2", "-- one\n-- two"));
    assert_eq!(fp(&lf), fp(&crlf));
    assert_eq!(fp(&lf), fp(&indented), "pattern lines are trimmed");
    // Leading spaces in the payload are code; they count.
    let moved = patch(pattern("local a = 1\nlocal b = 2", "  -- one\n-- two"));
    assert_ne!(fp(&lf).payload, fp(&moved).payload);
}

#[test]
fn ids_names_priorities_and_order_lists_are_not_hashed() {
    let base = patch(pattern("x", "-- p"));
    let mut other = pattern("x", "-- p");
    let o = other["pattern"].as_object_mut().unwrap();
    o.insert("id".into(), "renamed".into());
    o.insert("name".into(), "Some name".into());
    o.insert("priority".into(), 99.into());
    o.insert("before".into(), serde_json::json!(["a:b:1"]));
    o.insert("description".into(), "words".into());
    o.insert("on_failure".into(), "abort".into());
    let mut other = patch(other);
    other.set_metadata("other_mod".into(), "x:y:1".into(), None);
    assert_eq!(fp(&base), fp(&other));
}

#[test]
fn what_it_matches_and_adds_is_hashed() {
    let base = fp(&patch(pattern("x", "-- p")));
    let pat = fp(&patch(pattern("y", "-- p")));
    assert_ne!(base.anchor, pat.anchor);
    assert_eq!(base.payload, pat.payload);
    let pay = fp(&patch(pattern("x", "-- q")));
    assert_eq!(base.anchor, pay.anchor);
    assert_ne!(base.payload, pay.payload);

    let mut v = pattern("x", "-- p");
    v["pattern"]["position"] = "before".into();
    assert_ne!(base.anchor, fp(&patch(v)).anchor, "position");
    let mut v = pattern("x", "-- p");
    v["pattern"]["times"] = 2.into();
    assert_ne!(base.anchor, fp(&patch(v)).anchor, "times");
    let mut v = pattern("x", "-- p");
    v["pattern"]["match_indent"] = true.into();
    assert_ne!(base.anchor, fp(&patch(v)).anchor, "match_indent");
    let mut v = pattern("x", "-- p");
    v["pattern"]["target"] = "conf.lua".into();
    assert_ne!(base.anchor, fp(&patch(v)).anchor, "target");

    // Targets are a set: case, slashes and order do not matter.
    let mut a = pattern("x", "-- p");
    a["pattern"]["target"] = serde_json::json!(["Lib\\A.lua", "b.lua"]);
    let mut b = pattern("x", "-- p");
    b["pattern"]["target"] = serde_json::json!(["b.lua", "lib/a.lua"]);
    assert_eq!(fp(&patch(a)).anchor, fp(&patch(b)).anchor);

    assert_eq!(base.v, 1);
    assert_eq!(base.anchor.len(), 32);
    assert_eq!((base.source.as_str(), base.ordinal), ("qm.toml", 1));
}

#[test]
fn every_kind_has_its_own_inputs() {
    let regex = |pattern: &str, prepend: &str| {
        patch(
            serde_json::json!({"regex": {"target": "main.lua", "pattern": pattern,
            "position": "after", "payload": "-- r", "line_prepend": prepend}}),
        )
    };
    assert_ne!(fp(&regex("a+", "")).anchor, fp(&regex("a*", "")).anchor);
    assert_ne!(fp(&regex("a+", "")).anchor, fp(&regex("a+", "-- ")).anchor);
    assert_ne!(
        fp(&regex("x", "")).anchor,
        fp(&patch(pattern("x", "-- r"))).anchor,
        "the kind is hashed"
    );

    let copy = |position: &str, payload: &str| {
        patch(
            serde_json::json!({"copy": {"target": "main.lua", "position": position,
            "payload": payload}}),
        )
    };
    assert_ne!(
        fp(&copy("append", "a")).anchor,
        fp(&copy("prepend", "a")).anchor
    );
    assert_eq!(
        fp(&copy("append", "a")).anchor,
        fp(&copy("append", "b")).anchor
    );
    assert_ne!(
        fp(&copy("append", "a")).payload,
        fp(&copy("append", "b")).payload
    );
    let Patch::Copy(mut c) = copy("append", "b") else {
        unreachable!()
    };
    c.contents = vec!["-- file\r\n".into()];
    assert_ne!(
        fp(&Patch::Copy(c.clone())).payload,
        fp(&copy("append", "b")).payload
    );
    c.contents = vec!["-- file".into()];
    let lf = fp(&Patch::Copy(c));
    let Patch::Copy(mut c2) = copy("append", "b") else {
        unreachable!()
    };
    c2.contents = vec!["-- file\r\n".into()];
    assert_eq!(lf, fp(&Patch::Copy(c2)));

    let module = |name: &str, content: &str| {
        let Patch::Module(mut m) = patch(serde_json::json!({"module": {"source": "m.lua",
            "name": name, "before": "main.lua", "load_now": true}}))
        else {
            unreachable!()
        };
        m.content = content.into();
        Patch::Module(m)
    };
    assert_ne!(fp(&module("a", "x")).anchor, fp(&module("b", "x")).anchor);
    assert_eq!(fp(&module("a", "x")).anchor, fp(&module("a", "y")).anchor);
    assert_ne!(fp(&module("a", "x")).payload, fp(&module("a", "y")).payload);
}

#[test]
fn regex_rungs_are_in_the_anchor_hash_only_when_present() {
    let regex = |extra: serde_json::Value| {
        let mut v = serde_json::json!({"target": "main.lua", "pattern": "a+",
            "position": "after", "payload": "-- r"});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        patch(serde_json::json!({ "regex": v }))
    };
    // A regex with no rungs keeps the hash it always had (D52).
    let old = hash32(&joined(&[
        TAG, "anchor", "regex", "main.lua", "after", "-", "false", "a+", "", "",
    ]));
    let plain = fp(&regex(serde_json::json!({})));
    assert_eq!(plain.anchor, old);
    let with = fp(&regex(serde_json::json!({"anchors": [{"within": "gcore:b"}]})));
    assert_ne!(with.anchor, old);
    assert_eq!(with.payload, plain.payload);
    let other = fp(&regex(serde_json::json!({"anchors": [{"within": "gcore:c"}]})));
    assert_ne!(with.anchor, other.anchor);
    let sites = fp(&regex(serde_json::json!({"sites": [{"context": "abc"}]})));
    assert_ne!(sites.anchor, old);
}

#[test]
fn auto_assets_hash_the_mod_only() {
    let auto = |mod_id: &str, payload: &str| {
        let mut p = patch(pattern("setOptionVolume()", payload));
        p.set_metadata(mod_id.into(), format!("{mod_id}_auto_assets"), None);
        p
    };
    let a = auto("m", "-- C:/x/assets 3 files");
    let b = auto("m", "-- D:/y/assets 4 files");
    let fa = a.fingerprint(Path::new("C:\\x"), "assets", 0);
    assert_eq!(fa, b.fingerprint(Path::new("D:\\y"), "assets", 0));
    assert_eq!(fa.payload, "");
    assert_ne!(
        fa.anchor,
        auto("n", "").fingerprint(Path::new(""), "assets", 0).anchor
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn patch_dir_payloads_are_equal_across_mod_folders() {
    // A basic string gets single backslashes, a literal string keeps them doubled.
    let toml = "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"x\"\nposition = \"after\"\npayload = \"dofile('{{qm_hack:patch_dir}}/a.lua')\"\n\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = \"y\"\nposition = \"after\"\npayload = '''\ndofile(\"{{qm_hack:patch_dir}}\\b.lua\")\n'''\n";
    let one = tempfile::TempDir::new().unwrap();
    let two = tempfile::TempDir::new().unwrap();
    let mut prints = Vec::new();
    let mut payloads = Vec::new();
    for dir in [one.path(), two.path()] {
        write(dir, "m/qm.toml", toml);
        let raw = load_raw(dir, None).unwrap();
        assert_eq!(raw.mods[0].base, dir.join("m"));
        let mut fps = Fingerprints::new(&raw, FpMode::All);
        prints.push([fps.get(0).cloned().unwrap(), fps.get(1).cloned().unwrap()]);
        payloads.push(
            raw.patches
                .iter()
                .map(|p| match &p.patch {
                    Patch::Pattern(x) => x.payload.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>(),
        );
        let snap = raw.patches[1].patch.snapshot(&raw.mods[0].base);
        assert_eq!(snap.payload, "dofile(\"{{qm_hack:patch_dir}}\\b.lua\")");
    }
    assert_ne!(payloads[0], payloads[1], "the loader put each folder in");
    let base = one.path().join("m").to_string_lossy().to_string();
    assert!(payloads[0][0].contains(&base), "{:?}", payloads[0]);
    assert!(
        payloads[0][1].contains(&base.replace('\\', "\\\\")),
        "{:?}",
        payloads[0]
    );
    assert_eq!(prints[0], prints[1]);
}

#[test]
fn unhack_and_norm() {
    let dir = PathBuf::from("C:\\Mods\\m");
    assert_eq!(
        unhack("a C:\\Mods\\m\\x b", &dir),
        "a {{qm_hack:patch_dir}}\\x b"
    );
    assert_eq!(
        unhack("a C:\\\\Mods\\\\m\\\\x", &dir),
        "a {{qm_hack:patch_dir}}\\\\x"
    );
    assert_eq!(unhack("untouched", Path::new("")), "untouched");
    assert_eq!(norm("a  \r\nb\rc\n\n\n", &dir), "a\nb\nc");
    assert_eq!(norm("\n\n", &dir), "");
}

#[test]
fn snapshot_is_normalized_and_capped() {
    let p = patch(pattern(
        "  x  \r\n",
        "C:\\Mods\\m\\a.lua  \r\n-- two\r\n\r\n",
    ));
    let s = p.snapshot(Path::new("C:\\Mods\\m"));
    assert_eq!(s.kind, "pattern");
    assert_eq!(s.pattern, "  x");
    assert_eq!(s.payload, "{{qm_hack:patch_dir}}\\a.lua\n-- two");
    assert_eq!(s.position, Some(InsertPosition::After));
    assert_eq!(s.times, None);
    assert!(!s.truncated);
    assert!(!s.taken_at.is_empty());

    // Over 64 KiB is cut on a char boundary.
    let big = "é".repeat(SNAPSHOT_CAP);
    let s = patch(pattern("x", &big)).snapshot(Path::new(""));
    assert!(s.truncated);
    assert!(s.payload.len() <= SNAPSHOT_CAP);
    assert!(s.payload.len() > SNAPSHOT_CAP - 4);
    assert!(s.payload.chars().all(|c| c == 'é'));
    let s = patch(pattern(&"y".repeat(SNAPSHOT_CAP + 1), "-- p")).snapshot(Path::new(""));
    assert!(s.truncated);
    assert_eq!(s.pattern.len(), SNAPSHOT_CAP);
    let s = patch(pattern(&"y".repeat(SNAPSHOT_CAP), "-- p")).snapshot(Path::new(""));
    assert!(!s.truncated, "exactly 64 KiB fits");
}

#[test]
fn hashes_and_site_context() {
    assert_eq!(hash32("abc").len(), 32);
    assert_eq!(
        hash32("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"[..32]
    );
    assert_eq!(short_hash("  abc \t"), short_hash("abc"));
    assert_eq!(short_hash("abc").len(), 16);

    let lines = ["a", "b", "  c  ", "d", "e", "f", "g"];
    // [u-2, u+k+2) = lines 0..5 for u=2, k=1.
    let ctx = site_context(&lines, |_| false, 2, 1);
    let lines2 = ["a", "b", "c", "d", "e", "X", "Y"];
    assert_eq!(
        ctx,
        site_context(&lines2, |_| false, 2, 1),
        "outside the window"
    );
    assert_ne!(ctx, site_context(&lines2, |_| false, 3, 1));
    // Clamped at both ends; builtin lines skipped.
    assert_eq!(
        site_context(&lines, |_| false, 0, 1),
        site_context(&["a", "b", "c"], |_| false, 0, 1)
    );
    assert_eq!(
        site_context(&["a", "QM", "b", "c", "d"], |i| i == 1, 2, 1),
        site_context(&["a", "b", "c", "d"], |_| false, 2, 1)
    );
}
