//! The binding matrix (§7.1) and the drift policy for the legacy fields (§7.2).

use super::*;
use crate::handshake::SkipHunkFix;
use crate::patch::fingerprint::FINGERPRINTS;

/// A pattern patch of mod `m` with id `id`.
fn pat(m: &str, id: &str, pattern: &str, payload: &str) -> Patch {
    let mut p: Patch = serde_json::from_value(serde_json::json!({"pattern": {
        "id": id, "target": "main.lua", "pattern": pattern, "position": "after",
        "payload": payload}}))
    .unwrap();
    p.set_metadata(m.to_string(), id.to_string(), None);
    p
}

/// One raw patch: the patch, its toml and its ordinal.
struct P(Patch, &'static str, u32);

/// A raw load of `mods`, each with its patches, in order.
fn raw_of(mods: Vec<(&str, Vec<P>)>) -> RawLoad {
    let mut raw = RawLoad::default();
    for (ix, (id, patches)) in mods.into_iter().enumerate() {
        raw.mods.push(ModMeta {
            id: id.to_string(),
            folder: id.to_string(),
            manifest_ok: true,
            loader_index: ix as u32,
            version: Some("1.0.0".into()),
            ..ModMeta::default()
        });
        for P(patch, source, ordinal) in patches {
            raw.patches.push(RawPatch {
                patch,
                base_priority: 10,
                explicit_priority: None,
                path: PathBuf::from(format!("{id}/{source}")),
                vars: HashMap::new(),
                mod_ix: ix as u32,
                source: source.to_string(),
                ordinal,
            });
        }
    }
    raw
}

/// A ref to `id` of mod `m` with the fingerprint of `like` (None: no fingerprint).
fn rf(m: &str, id: &str, like: Option<(&Patch, &str, u32)>) -> PatchRef {
    PatchRef {
        id: id.to_string(),
        mod_id: m.to_string(),
        fp: like.map(|(p, s, o)| p.fingerprint(Path::new(""), s, o)),
    }
}

fn bind_all(raw: &RawLoad, r: &PatchRef, kind: BindKind) -> Bound {
    bind(r, kind, &mut Fingerprints::new(raw, FpMode::All))
}

fn bind_lazy(raw: &RawLoad, r: &PatchRef, kind: BindKind) -> Bound {
    bind(r, kind, &mut Fingerprints::new(raw, FpMode::Lazy))
}

fn counted<T>(f: impl FnOnce() -> T) -> (T, usize) {
    FINGERPRINTS.with(|c| c.set(0));
    let out = f();
    (out, FINGERPRINTS.with(|c| c.get()))
}

const KINDS: [BindKind; 8] = [
    BindKind::Disable,
    BindKind::Order,
    BindKind::Priority,
    BindKind::Glue,
    BindKind::Anchor,
    BindKind::ApplyAs,
    BindKind::Hunk,
    BindKind::Stack,
];

fn ambiguous(ids: &[&str]) -> BindingStatus {
    BindingStatus::Ambiguous {
        candidates: ids.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn mod_off_waits() {
    let raw = raw_of(vec![(
        "a",
        vec![P(pat("a", "a:1", "x", "-- a"), "qm.toml", 1)],
    )]);
    for kind in KINDS {
        let b = bind_all(&raw, &rf("gone", "gone:1", None), kind);
        assert_eq!(
            b.status,
            BindingStatus::Waiting {
                mods: vec!["gone".into()]
            }
        );
        assert!(b.targets.is_empty());
    }
}

#[test]
fn one_hit() {
    let a = pat("a", "a:1", "x", "-- a");
    let raw = raw_of(vec![("a", vec![P(a.clone(), "qm.toml", 1)])]);
    let changed_anchor = pat("a", "a:1", "y", "-- a");
    let changed_payload = pat("a", "a:1", "x", "-- b");
    let changed_both = pat("a", "a:1", "y", "-- b");
    for kind in KINDS {
        // No fingerprint, or an equal one: Bound.
        for r in [
            rf("a", "a:1", None),
            rf("a", "a:1", Some((&a, "qm.toml", 1))),
        ] {
            assert_eq!(bind_all(&raw, &r, kind).status, BindingStatus::Bound);
            assert_eq!(bind_all(&raw, &r, kind).targets, [0]);
        }
        // A moved patch (another toml or ordinal) with the same content is still Bound.
        let moved = rf("a", "a:1", Some((&a, "qm/other.toml", 7)));
        assert_eq!(bind_all(&raw, &moved, kind).status, BindingStatus::Bound);
        // Differs: Drifted, with which half changed. The entry still applies to the hit.
        for (like, anchor, payload) in [
            (&changed_anchor, true, false),
            (&changed_payload, false, true),
            (&changed_both, true, true),
        ] {
            let r = rf("a", "a:1", Some((like, "qm.toml", 1)));
            let b = bind_all(&raw, &r, kind);
            assert_eq!(
                b.status,
                BindingStatus::Drifted { anchor, payload },
                "{kind:?}"
            );
            assert_eq!(b.targets, [0]);
            assert_eq!(b.new_id, None);
        }
    }
    // The mod's folder and any case name it too.
    let r = rf("A", "a:1", None);
    assert_eq!(bind_all(&raw, &r, BindKind::Disable).targets, [0]);
}

#[test]
fn several_hits_narrow_by_content_then_place() {
    // Two patches answer to "m:1": the first and a D29 duplicate.
    let first = pat("m", "m:1", "x", "-- first");
    let mut dup = pat("m", "m:1#qm/b.toml", "x", "-- dup");
    dup.extra_mut().legacy_id = Some("m:1".into());
    let mut twin = pat("m", "m:1#qm/c.toml", "x", "-- first");
    twin.extra_mut().legacy_id = Some("m:1".into());
    let other = pat("o", "m:1", "x", "-- first");
    let raw = raw_of(vec![
        (
            "m",
            vec![
                P(first.clone(), "qm.toml", 1),
                P(dup.clone(), "qm/b.toml", 2),
                P(twin, "qm/c.toml", 3),
            ],
        ),
        ("o", vec![P(other, "qm.toml", 1)]),
    ]);
    for kind in KINDS {
        // Content picks the duplicate.
        let r = rf("m", "m:1", Some((&dup, "qm/b.toml", 2)));
        assert_eq!(
            bind_all(&raw, &r, kind),
            Bound {
                status: BindingStatus::Bound,
                targets: vec![1],
                new_id: None
            }
        );
        // Two with the same content: the place picks.
        let r = rf("m", "m:1", Some((&first, "qm/c.toml", 3)));
        assert_eq!(bind_all(&raw, &r, kind).targets, [2], "{kind:?}");
        assert_eq!(bind_all(&raw, &r, kind).status, BindingStatus::Bound);
        // Same content, no place match: Ambiguous; only the exact id applies.
        let r = rf("m", "m:1", Some((&first, "qm/z.toml", 9)));
        let b = bind_all(&raw, &r, kind);
        assert_eq!(b.status, ambiguous(&["m:1", "m:1#qm/c.toml"]));
        assert_eq!(b.targets, [0]);
        // No content match: the place match is Drifted.
        let changed = pat("m", "m:1", "x", "-- changed");
        let r = rf("m", "m:1", Some((&changed, "qm/b.toml", 2)));
        let b = bind_all(&raw, &r, kind);
        assert_eq!(
            b.status,
            BindingStatus::Drifted {
                anchor: false,
                payload: true
            }
        );
        assert_eq!(b.targets, [1]);
        // ... and with no place match either: Ambiguous.
        let r = rf("m", "m:1", Some((&changed, "qm/z.toml", 9)));
        let b = bind_all(&raw, &r, kind);
        assert_eq!(
            b.status,
            ambiguous(&["m:1", "m:1#qm/b.toml", "m:1#qm/c.toml"])
        );
        assert_eq!(b.targets, [0]);
        // No fingerprint: Ambiguous, the exact id only. The other mod's "m:1" is not a hit.
        let b = bind_all(&raw, &rf("m", "m:1", None), kind);
        assert_eq!(
            b.status,
            ambiguous(&["m:1", "m:1#qm/b.toml", "m:1#qm/c.toml"])
        );
        assert_eq!(b.targets, [0]);
    }
    // Explicit ids that repeat give no exact match: nothing applies.
    let same = |payload| pat("e", "same", "x", payload);
    let raw = raw_of(vec![(
        "e",
        vec![
            P(same("-- 1"), "qm.toml", 1),
            P(same("-- 1"), "qm/b.toml", 1),
        ],
    )]);
    let b = bind_all(&raw, &rf("e", "same", None), BindKind::Disable);
    assert_eq!(b.status, ambiguous(&["same", "same"]));
    assert!(b.targets.is_empty());
    let b = bind_all(
        &raw,
        &rf("e", "same", Some((&same("-- 1"), "qm/b.toml", 1))),
        BindKind::Disable,
    );
    assert_eq!((b.status, b.targets), (BindingStatus::Bound, vec![1]));
}

#[test]
fn no_hit_rebinds_within_the_mod() {
    let a = pat("m", "m:main_lua:2", "x", "-- a");
    let b2 = pat("m", "m:main_lua:3", "y", "-- b");
    let raw = raw_of(vec![
        ("m", vec![P(a.clone(), "qm.toml", 2), P(b2, "qm.toml", 3)]),
        ("o", vec![P(pat("o", "o:1", "x", "-- a"), "qm.toml", 1)]),
    ]);
    // The old id is gone; the same content under a new id.
    let old = pat("m", "m:main_lua:1", "x", "-- a");
    for kind in KINDS {
        let r = rf("m", "m:main_lua:1", Some((&old, "qm.toml", 1)));
        let b = bind_all(&raw, &r, kind);
        assert_eq!(
            b.status,
            BindingStatus::Rebound {
                from: "m:main_lua:1".into(),
                to: "m:main_lua:2".into()
            }
        );
        assert_eq!(
            (b.targets, b.new_id),
            (vec![0], Some("m:main_lua:2".into()))
        );
        // No fingerprint: Missing.
        let b = bind_all(&raw, &rf("m", "m:main_lua:1", None), kind);
        assert_eq!((b.status, b.targets), (BindingStatus::Missing, vec![]));
        // Nothing alike: Missing.
        let r = rf(
            "m",
            "m:main_lua:1",
            Some((&pat("m", "q", "z", "-- z"), "qm.toml", 1)),
        );
        assert_eq!(bind_all(&raw, &r, kind).status, BindingStatus::Missing);
    }

    // Only the anchor is the same: Rebound and Drifted for an order, a priority or an anchor
    // fix; a disable, a copy's base, an ApplyAs, a skip or a stack rule never widens (D20).
    let edited = pat("m", "m:main_lua:1", "x", "-- edited");
    let r = rf("m", "m:main_lua:1", Some((&edited, "qm.toml", 1)));
    for kind in [BindKind::Order, BindKind::Priority, BindKind::Anchor] {
        let b = bind_all(&raw, &r, kind);
        assert_eq!(
            b.status,
            BindingStatus::Drifted {
                anchor: false,
                payload: true
            }
        );
        assert_eq!(
            (b.targets, b.new_id),
            (vec![0], Some("m:main_lua:2".into()))
        );
    }
    for kind in [
        BindKind::Disable,
        BindKind::Glue,
        BindKind::ApplyAs,
        BindKind::Hunk,
        BindKind::Stack,
    ] {
        let b = bind_all(&raw, &r, kind);
        assert_eq!(b.status, ambiguous(&["m:main_lua:2"]));
        assert!(b.targets.is_empty());
    }
}

#[test]
fn no_hit_with_several_alike_narrows_by_place() {
    let p = |id: &str| pat("m", id, "x", "-- a");
    let raw = raw_of(vec![(
        "m",
        vec![
            P(p("m:2"), "qm.toml", 2),
            P(p("m:3"), "qm/b.toml", 1),
            P(pat("m", "m:4", "x", "-- other"), "qm/c.toml", 1),
            P(pat("m", "m:5", "x", "-- other 2"), "qm/d.toml", 1),
        ],
    )]);
    let r = rf("m", "m:1", Some((&p("m:1"), "qm/b.toml", 1)));
    let b = bind_all(&raw, &r, BindKind::Disable);
    assert_eq!(b.targets, [1]);
    assert!(matches!(b.status, BindingStatus::Rebound { .. }));
    let r = rf("m", "m:1", Some((&p("m:1"), "qm/z.toml", 1)));
    let b = bind_all(&raw, &r, BindKind::Disable);
    assert_eq!((b.status, b.targets), (ambiguous(&["m:2", "m:3"]), vec![]));
    // Several anchor-only matches: the place picks, else Ambiguous.
    let edited = pat("m", "m:1", "x", "-- edited");
    let r = rf("m", "m:1", Some((&edited, "qm/c.toml", 1)));
    let b = bind_all(&raw, &r, BindKind::Priority);
    assert_eq!((b.targets, b.new_id), (vec![2], Some("m:4".into())));
    let r = rf("m", "m:1", Some((&edited, "qm/z.toml", 1)));
    let b = bind_all(&raw, &r, BindKind::Priority);
    assert_eq!(b.status, ambiguous(&["m:2", "m:3", "m:4", "m:5"]));
    assert!(b.targets.is_empty());
}

#[test]
fn order_names_match_as_rule_names() {
    // A rule name does not match `mod:name`; a disable does.
    let mut p: Patch = serde_json::from_value(serde_json::json!({"pattern": {
        "id": "a:1", "name": "Halt", "target": "main.lua", "pattern": "x",
        "position": "after", "payload": "-- a"}}))
    .unwrap();
    p.set_metadata("a".into(), "a:1".into(), None);
    let raw = raw_of(vec![("a", vec![P(p, "qm.toml", 1)])]);
    let r = rf("a", "a:Halt", None);
    assert_eq!(bind_all(&raw, &r, BindKind::Disable).targets, [0]);
    assert_eq!(
        bind_all(&raw, &r, BindKind::Order).status,
        BindingStatus::Missing
    );
    let r = rf("a", "a:a:1", None);
    assert_eq!(bind_all(&raw, &r, BindKind::Order).targets, [0]);
}

#[test]
fn auto_assets_bind_by_id() {
    let auto = |payload: &str| {
        let mut p = pat("snd", "snd_auto_assets", "setOptionVolume()", payload);
        p.set_metadata("snd".into(), "snd_auto_assets".into(), None);
        P(p, "assets", 0)
    };
    let before = raw_of(vec![("snd", vec![auto("-- C:/old/assets: 3 files")])]);
    let after = raw_of(vec![("snd", vec![auto("-- D:/new/assets: 5 files")])]);
    let fp = Fingerprints::new(&before, FpMode::All).get(0).cloned();
    let r = PatchRef {
        id: "snd_auto_assets".into(),
        mod_id: "snd".into(),
        fp,
    };
    for kind in KINDS {
        let b = bind_all(&after, &r, kind);
        assert_eq!((b.status, b.targets), (BindingStatus::Bound, vec![0]));
    }
}

#[test]
fn the_dll_fingerprints_lazily() {
    let m_patches = || {
        vec![
            P(pat("m", "m:1", "x", "-- 1"), "qm.toml", 1),
            P(pat("m", "m:2", "y", "-- 2"), "qm.toml", 2),
            P(pat("m", "m:3", "z", "-- 3"), "qm.toml", 3),
        ]
    };
    let raw = raw_of(vec![
        ("m", m_patches()),
        (
            "o",
            vec![
                P(pat("o", "o:1", "x", "-- 1"), "qm.toml", 1),
                P(pat("o", "o:2", "x", "-- 1"), "qm.toml", 2),
            ],
        ),
    ]);
    let like = pat("m", "m:1", "x", "-- 1");
    let with_fp = |id: &str| rf("m", id, Some((&like, "qm.toml", 1)));

    // No refs: no fingerprint.
    let plan = PlanLoad {
        plan: Some(LoadOrderPlan {
            enabled_mods: vec!["m".into(), "o".into()],
            disabled_patches: vec!["m:1".to_string().into()],
            patch_priorities: [("m:2".to_string(), 5.into())].into(),
            patch_order: vec![OrderRule {
                patch: "m:3".into(),
                before: vec!["m:1".into()],
                after: Vec::new(),
            }
            .into()],
            ..LoadOrderPlan::default()
        }),
        ..PlanLoad::default()
    };
    let (_, n) = counted(|| apply_plan(&raw, &plan));
    assert_eq!(n, 0);

    // A disable, a priority and an order ref with one hit each: none.
    let mut p = plan.plan.clone().unwrap();
    for id in ["m:1", "m:2", "m:3"] {
        p.refs.insert(id.into(), with_fp(id).into());
    }
    let with_refs = PlanLoad {
        plan: Some(p),
        ..PlanLoad::default()
    };
    let (applied, n) = counted(|| apply_plan(&raw, &with_refs));
    assert_eq!(n, 0);
    assert_eq!(
        applied.report.bindings.len(),
        4,
        "{:?}",
        applied.report.bindings
    );
    let r2 = with_fp("m:2");
    for kind in [
        BindKind::Disable,
        BindKind::Order,
        BindKind::Priority,
        BindKind::Glue,
    ] {
        let (b, n) = counted(|| bind_lazy(&raw, &r2, kind));
        assert_eq!((b.status, n), (BindingStatus::Bound, 0), "{kind:?}");
    }
    // The simulation computes them all, so it sees the drift.
    let (applied, n) = counted(|| apply_plan_with(&raw, &with_refs, FpMode::All));
    assert_eq!(n, 3, "one per hit, each once");
    let drifted = applied
        .report
        .bindings
        .iter()
        .filter(|b| matches!(b.status, BindingStatus::Drifted { .. }))
        .count();
    assert_eq!(drifted, 2, "m:2 (the priority) and m:3 (the order rule)");

    // An anchor ref with one hit: exactly one.
    let (b, n) = counted(|| bind_lazy(&raw, &r2, BindKind::Anchor));
    assert_eq!(n, 1);
    assert!(matches!(b.status, BindingStatus::Drifted { .. }));

    // No hit: only the ref's mod's patches.
    let gone = with_fp("m:gone");
    let (b, n) = counted(|| bind_lazy(&raw, &gone, BindKind::Disable));
    assert_eq!(n, 3);
    assert!(matches!(b.status, BindingStatus::Rebound { .. }));

    // Several hits: only those, all of the ref's mod.
    let mut dup_raw = raw_of(vec![
        ("m", m_patches()),
        ("o", vec![P(pat("o", "m:1", "x", "-- 1"), "qm.toml", 1)]),
    ]);
    let mut dup = pat("m", "m:1#qm/b.toml", "x", "-- 1");
    dup.extra_mut().legacy_id = Some("m:1".into());
    dup_raw.patches.insert(
        1,
        RawPatch {
            patch: dup,
            source: "qm/b.toml".into(),
            ordinal: 1,
            ..dup_raw.patches[0].clone()
        },
    );
    let r1 = with_fp("m:1");
    let (b, n) = counted(|| bind_lazy(&dup_raw, &r1, BindKind::Disable));
    assert_eq!(n, 2);
    assert_eq!((b.status, b.targets), (BindingStatus::Bound, vec![0]));
}

/// Mods `m` (patches m:1, m:2, m:3 at "a", "b", "c") and `o` (o:1 at "d"), and a plan.
fn world() -> (RawLoad, LoadOrderPlan) {
    let raw = raw_of(vec![
        (
            "m",
            vec![
                P(pat("m", "m:1", "a", "-- 1"), "qm.toml", 1),
                P(pat("m", "m:2", "b", "-- 2"), "qm.toml", 2),
                P(pat("m", "m:3", "c", "-- 3"), "qm.toml", 3),
            ],
        ),
        ("o", vec![P(pat("o", "o:1", "d", "-- o"), "qm.toml", 1)]),
    ]);
    let plan = LoadOrderPlan {
        profile_id: "p".into(),
        generated_at: "now".into(),
        enabled_mods: vec!["m".into(), "o".into()],
        ..LoadOrderPlan::default()
    };
    (raw, plan)
}

fn applied(raw: &RawLoad, plan: &LoadOrderPlan) -> Applied {
    apply_plan_with(
        raw,
        &PlanLoad {
            plan: Some(plan.clone()),
            ..PlanLoad::default()
        },
        FpMode::All,
    )
}

fn disabled(a: &Applied) -> Vec<String> {
    a.patches
        .iter()
        .filter(|(p, _, _)| p.is_disabled())
        .map(|(p, _, _)| p.id().unwrap_or_default().to_string())
        .collect()
}

fn ids(a: &Applied) -> Vec<String> {
    a.patches
        .iter()
        .map(|(p, _, _)| p.id().unwrap_or_default().to_string())
        .collect()
}

fn prio(a: &Applied, id: &str) -> Priority {
    a.patches
        .iter()
        .find(|(p, _, _)| p.id() == Some(id))
        .map(|(_, p, _)| *p)
        .unwrap()
}

/// The refs of `cases`: (entry, ref).
fn with_refs(plan: &LoadOrderPlan, cases: &[(&str, PatchRef)]) -> LoadOrderPlan {
    let mut p = plan.clone();
    for (k, r) in cases {
        p.refs.insert(k.to_string(), r.clone().into());
    }
    p
}

fn status(a: &Applied, key: &str) -> Vec<BindingStatus> {
    a.report
        .bindings
        .iter()
        .filter(|b| b.key == key)
        .map(|b| b.status.clone())
        .collect()
}

/// A ref whose fingerprint matches m:2 under the old id `m:old`.
fn moved_ref(payload_changed: bool) -> PatchRef {
    let like = if payload_changed {
        pat("m", "m:old", "b", "-- 2 old")
    } else {
        pat("m", "m:old", "b", "-- 2")
    };
    rf("m", "m:old", Some((&like, "qm.toml", 2)))
}

#[test]
fn drift_policy_disable() {
    let (raw, mut plan) = world();
    let m2 = pat("m", "m:2", "b", "-- 2");
    let drifted_like = pat("m", "m:2", "b", "-- 2 old");

    // No ref: by name, as before.
    plan.disabled_patches = vec!["m:2".to_string().into()];
    let a = applied(&raw, &plan);
    assert_eq!(disabled(&a), ["m:2"]);
    assert!(a.report.bindings.is_empty());

    // Bound.
    let p = with_refs(&plan, &[("m:2", rf("m", "m:2", Some((&m2, "qm.toml", 2))))]);
    let a = applied(&raw, &p);
    assert_eq!(disabled(&a), ["m:2"]);
    assert_eq!(status(&a, "disable:m:2"), [BindingStatus::Bound]);

    // Drifted: stays off, flagged.
    let p = with_refs(
        &plan,
        &[("m:2", rf("m", "m:2", Some((&drifted_like, "qm.toml", 2))))],
    );
    let a = applied(&raw, &p);
    assert_eq!(disabled(&a), ["m:2"]);
    assert_eq!(
        status(&a, "disable:m:2"),
        [BindingStatus::Drifted {
            anchor: false,
            payload: true
        }]
    );
    assert!(a.report.bindings[0].detail.contains("stays off"));

    // Rebound: the new id is off.
    plan.disabled_patches = vec!["m:old".to_string().into()];
    let p = with_refs(&plan, &[("m:old", moved_ref(false))]);
    let a = applied(&raw, &p);
    assert_eq!(disabled(&a), ["m:2"]);
    assert!(matches!(
        status(&a, "disable:m:old")[..],
        [BindingStatus::Rebound { .. }]
    ));
    assert_eq!(a.report.bindings[0].new_id.as_deref(), Some("m:2"));

    // Anchor-only: nothing new is turned off (D20).
    let p = with_refs(&plan, &[("m:old", moved_ref(true))]);
    let a = applied(&raw, &p);
    assert!(disabled(&a).is_empty());
    assert_eq!(status(&a, "disable:m:old"), [ambiguous(&["m:2"])]);

    // Missing: nothing.
    let gone = rf(
        "m",
        "m:old",
        Some((&pat("m", "q", "zz", "-- zz"), "qm.toml", 9)),
    );
    let p = with_refs(&plan, &[("m:old", gone)]);
    let a = applied(&raw, &p);
    assert!(disabled(&a).is_empty());
    assert_eq!(status(&a, "disable:m:old"), [BindingStatus::Missing]);

    // Mod off: Waiting, and a fix id from fix_of.
    plan.disabled_patches = vec!["x:1".to_string().into()];
    let mut p = with_refs(&plan, &[("x:1", rf("x", "x:1", None))]);
    p.fix_of
        .insert("disable:x:1".into(), "fx_1".to_string().into());
    let a = applied(&raw, &p);
    assert!(disabled(&a).is_empty());
    assert_eq!(a.report.bindings[0].fix_id, "fx_1");
    assert_eq!(a.report.bindings[0].detail, "Waiting for x");
    assert_eq!(
        status(&a, "disable:x:1"),
        [BindingStatus::Waiting {
            mods: vec!["x".into()]
        }]
    );
}

#[test]
fn drift_policy_disable_ambiguous_keeps_the_exact_id() {
    // A legacy entry on a duplicated fallback id hits both; a ref with no fingerprint
    // only the exact id (and a ledger ref with a place, only its own duplicate).
    let first = pat("m", "m:1", "a", "-- 1");
    let mut dup = pat("m", "m:1#qm/b.toml", "a", "-- 1");
    dup.extra_mut().legacy_id = Some("m:1".into());
    let raw = raw_of(vec![(
        "m",
        vec![P(first.clone(), "qm.toml", 1), P(dup, "qm/b.toml", 1)],
    )]);
    let (_, mut plan) = world();
    plan.disabled_patches = vec!["m:1".to_string().into()];
    assert_eq!(disabled(&applied(&raw, &plan)), ["m:1", "m:1#qm/b.toml"]);
    let p = with_refs(&plan, &[("m:1", rf("m", "m:1", None))]);
    let a = applied(&raw, &p);
    assert_eq!(disabled(&a), ["m:1"]);
    assert_eq!(
        status(&a, "disable:m:1"),
        [ambiguous(&["m:1", "m:1#qm/b.toml"])]
    );
    let p = with_refs(
        &plan,
        &[("m:1", rf("m", "m:1", Some((&first, "qm/b.toml", 1))))],
    );
    assert_eq!(disabled(&applied(&raw, &p)), ["m:1#qm/b.toml"]);
}

#[test]
fn drift_policy_priority() {
    let (raw, mut plan) = world();
    let m2 = pat("m", "m:2", "b", "-- 2");
    plan.patch_priorities = [("m:2".to_string(), 77.into())].into();
    assert_eq!(prio(&applied(&raw, &plan), "m:2"), 77);

    // Bound and Drifted both apply.
    for like in [m2.clone(), pat("m", "m:2", "b", "-- 2 old")] {
        let p = with_refs(
            &plan,
            &[("m:2", rf("m", "m:2", Some((&like, "qm.toml", 2))))],
        );
        let a = applied(&raw, &p);
        assert_eq!(prio(&a, "m:2"), 77);
        assert_eq!(status(&a, "priority:m:2").len(), 1);
    }
    // Rebound, and Rebound with a changed payload: the new id gets it.
    plan.patch_priorities = [("m:old".to_string(), 77.into())].into();
    for changed in [false, true] {
        let p = with_refs(&plan, &[("m:old", moved_ref(changed))]);
        let a = applied(&raw, &p);
        assert_eq!(prio(&a, "m:2"), 77, "{changed}");
        let s = status(&a, "priority:m:old");
        if changed {
            assert_eq!(
                s,
                [BindingStatus::Drifted {
                    anchor: false,
                    payload: true
                }]
            );
            assert!(a.report.bindings[0].detail.contains("is now m:2"));
        } else {
            assert!(matches!(s[..], [BindingStatus::Rebound { .. }]));
        }
    }
    // Missing and Waiting: ignored.
    let gone = rf(
        "m",
        "m:old",
        Some((&pat("m", "q", "zz", "-- zz"), "qm.toml", 9)),
    );
    let p = with_refs(&plan, &[("m:old", gone)]);
    let a = applied(&raw, &p);
    assert!(a.patches.iter().all(|(_, p, _)| *p == 10));
    plan.patch_priorities = [("x:1".to_string(), 77.into())].into();
    let p = with_refs(&plan, &[("x:1", rf("x", "x:1", None))]);
    let a = applied(&raw, &p);
    assert!(a.patches.iter().all(|(_, p, _)| *p == 10));
    assert!(matches!(
        status(&a, "priority:x:1")[..],
        [BindingStatus::Waiting { .. }]
    ));

    // Ambiguous: the exact id only.
    let first = pat("m", "m:1", "a", "-- 1");
    let mut dup = pat("m", "m:1#qm/b.toml", "a", "-- 1");
    dup.extra_mut().legacy_id = Some("m:1".into());
    let raw = raw_of(vec![(
        "m",
        vec![P(first, "qm.toml", 1), P(dup, "qm/b.toml", 1)],
    )]);
    plan.patch_priorities = [("m:1".to_string(), 77.into())].into();
    let a = applied(&raw, &plan);
    assert_eq!((prio(&a, "m:1"), prio(&a, "m:1#qm/b.toml")), (77, 77));
    let p = with_refs(&plan, &[("m:1", rf("m", "m:1", None))]);
    let a = applied(&raw, &p);
    assert_eq!((prio(&a, "m:1"), prio(&a, "m:1#qm/b.toml")), (77, 10));
}

#[test]
fn drift_policy_order() {
    let (raw, mut plan) = world();
    let m2 = pat("m", "m:2", "b", "-- 2");
    let rule = |patch: &str, before: &[&str]| OrderRule {
        patch: patch.into(),
        before: before.iter().map(|s| s.to_string()).collect(),
        after: Vec::new(),
    };
    plan.patch_order = vec![rule("o:1", &["m:2"]).into()];
    assert_eq!(applied(&raw, &plan).order_rules, [rule("o:1", &["m:2"])]);

    // Bound and Drifted keep the rule as written.
    for like in [m2.clone(), pat("m", "m:2", "x", "-- 2")] {
        let p = with_refs(
            &plan,
            &[("m:2", rf("m", "m:2", Some((&like, "qm.toml", 2))))],
        );
        let a = applied(&raw, &p);
        assert_eq!(a.order_rules, [rule("o:1", &["m:2"])]);
        assert_eq!(status(&a, "order:o:1").len(), 1);
    }
    // Rebound: the rule names the new id.
    plan.patch_order = vec![rule("m:old", &["o:1"]).into()];
    for changed in [false, true] {
        let p = with_refs(&plan, &[("m:old", moved_ref(changed))]);
        let a = applied(&raw, &p);
        assert_eq!(a.order_rules, [rule("m:2", &["o:1"])], "{changed}");
    }
    // Missing or Waiting: the rule is dropped; a missing `before` name is dropped alone.
    let gone = rf(
        "m",
        "m:old",
        Some((&pat("m", "q", "zz", "-- zz"), "qm.toml", 9)),
    );
    let p = with_refs(&plan, &[("m:old", gone.clone())]);
    let a = applied(&raw, &p);
    assert!(a.order_rules.is_empty());
    assert_eq!(status(&a, "order:m:old"), [BindingStatus::Missing]);
    plan.patch_order = vec![rule("o:1", &["m:old", "m:3"]).into()];
    let p = with_refs(&plan, &[("m:old", gone)]);
    assert_eq!(applied(&raw, &p).order_rules, [rule("o:1", &["m:3"])]);
    plan.patch_order = vec![rule("x:1", &["m:3"]).into()];
    let p = with_refs(&plan, &[("x:1", rf("x", "x:1", None))]);
    let a = applied(&raw, &p);
    assert!(a.order_rules.is_empty());
    assert!(matches!(
        status(&a, "order:x:1")[..],
        [BindingStatus::Waiting { .. }]
    ));

    // Ambiguous: the exact id only, which a D29 duplicate's old id does not match.
    let first = pat("m", "m:1", "a", "-- 1");
    let mut dup = pat("m", "m:1#qm/b.toml", "a", "-- 1");
    dup.extra_mut().legacy_id = Some("m:1".into());
    let raw = raw_of(vec![
        ("m", vec![P(first, "qm.toml", 1), P(dup, "qm/b.toml", 1)]),
        ("o", vec![P(pat("o", "o:1", "d", "-- o"), "qm.toml", 1)]),
    ]);
    plan.patch_order = vec![rule("o:1", &["m:1"]).into()];
    let p = with_refs(&plan, &[("m:1", rf("m", "m:1", None))]);
    let a = applied(&raw, &p);
    assert_eq!(a.order_rules, [rule("o:1", &["=m:1"])]);
    assert_eq!(order::matching_patches("=m:1", &a.patches), [0]);
    assert_eq!(order::matching_patches("m:1", &a.patches), [0, 1]);
    // The duplicate alone keeps its own unique id.
    let first = pat("m", "m:1", "a", "-- 1");
    let p = with_refs(
        &plan,
        &[("m:1", rf("m", "m:1", Some((&first, "qm/b.toml", 1))))],
    );
    assert_eq!(
        applied(&raw, &p).order_rules,
        [rule("o:1", &["m:1#qm/b.toml"])]
    );
    // The order itself follows: o:1 runs before only the exact m:1.
    let table = crate::patch::table::PatchTable::from_applied(
        applied(&raw, &with_refs(&plan, &[("m:1", rf("m", "m:1", None))])),
        PathBuf::new(),
    );
    let order = table.ordered();
    assert!(order.unknown_ids.is_empty());
    let pos = |i: usize| order.order.iter().position(|&x| x == i).unwrap();
    assert!(pos(2) < pos(0), "{:?}", order.order);
    assert!(
        pos(1) < pos(2),
        "the duplicate keeps its loader place: {:?}",
        order.order
    );
}

#[test]
fn drift_policy_adjusted_copy() {
    let (raw, mut plan) = world();
    let glue = |base: &str| GluePatchEntry {
        id: "glue:g".into(),
        name: "Copy".into(),
        replaces_patch_id: Some(base.into()),
        priority: 1,
        patch: serde_json::from_value(serde_json::json!({"pattern": {"target": "main.lua",
            "pattern": "b", "position": "before", "payload": "-- copy"}}))
        .unwrap(),
        base_mod_id: None,
        run_as: None,
        exact: false,
    };
    let m2 = pat("m", "m:2", "b", "-- 2");
    plan.glue_patches = vec![glue("m:2").into()];
    let legacy = ids(&applied(&raw, &plan));
    assert_eq!(legacy, ["m:1", "m:3", "o:1", "glue:g"]);

    // Bound, and a drifted base: the copy replaces it and runs (D22).
    for (like, drifted) in [
        (m2.clone(), false),
        (pat("m", "m:2", "b", "-- 2 old"), true),
    ] {
        let p = with_refs(
            &plan,
            &[("m:2", rf("m", "m:2", Some((&like, "qm.toml", 2))))],
        );
        let a = applied(&raw, &p);
        assert_eq!(ids(&a), legacy);
        let s = status(&a, "glue:glue:g");
        assert_eq!(s.len(), 1);
        assert_eq!(matches!(s[0], BindingStatus::Drifted { .. }), drifted);
        if drifted {
            assert!(a.report.bindings[0].detail.contains("your copy is older"));
        }
    }
    // Rebound: the copy replaces the new id.
    plan.glue_patches = vec![glue("m:old").into()];
    let p = with_refs(&plan, &[("m:old", moved_ref(false))]);
    let a = applied(&raw, &p);
    assert_eq!(ids(&a), legacy);
    // Anchor-only: Ambiguous; the copy runs and nothing new is replaced.
    let p = with_refs(&plan, &[("m:old", moved_ref(true))]);
    let a = applied(&raw, &p);
    assert_eq!(ids(&a), ["m:1", "m:2", "m:3", "o:1", "glue:g"]);
    assert_eq!(status(&a, "glue:glue:g"), [ambiguous(&["m:2"])]);
    // Missing: the copy keeps running.
    let gone = rf(
        "m",
        "m:old",
        Some((&pat("m", "q", "zz", "-- zz"), "qm.toml", 9)),
    );
    let p = with_refs(&plan, &[("m:old", gone)]);
    let a = applied(&raw, &p);
    assert_eq!(ids(&a), ["m:1", "m:2", "m:3", "o:1", "glue:g"]);
    assert_eq!(status(&a, "glue:glue:g"), [BindingStatus::Missing]);
    assert!(a.report.bindings[0].detail.contains("still runs"));

    // The base mod off: the copy is skipped, Waiting (D19).
    plan.glue_patches = vec![glue("x:1").into()];
    plan.enabled_mods = vec!["m".into(), "o".into()];
    let p = with_refs(&plan, &[("x:1", rf("x", "x:1", None))]);
    let a = applied(&raw, &p);
    assert_eq!(ids(&a), ["m:1", "m:2", "m:3", "o:1"]);
    assert_eq!(
        status(&a, "glue:glue:g"),
        [BindingStatus::Waiting {
            mods: vec!["x".into()]
        }]
    );
    assert!(a.report.bindings[0].detail.ends_with("while x is off"));
    // Enabled but not loaded: the copy waits too (D60).
    plan.enabled_mods.push("x".into());
    let p = with_refs(&plan, &[("x:1", rf("x", "x:1", None))]);
    let a = applied(&raw, &p);
    assert_eq!(ids(&a), ["m:1", "m:2", "m:3", "o:1"]);
    assert_eq!(
        status(&a, "glue:glue:g"),
        [BindingStatus::Waiting {
            mods: vec!["x".into()]
        }]
    );
    assert!(a.report.bindings[0].detail.ends_with("while x is missing"));
    // Failed to load, or skipped as a requirer of one that did: it waits (D60).
    let mut failed = raw.clone();
    failed.load_errors = vec![LoadError {
        mod_id: "x".into(),
        folder: "x".into(),
        message: "bad qm.toml".into(),
        skipped_dependents: vec!["y".into()],
    }];
    let a = applied(&failed, &p);
    assert_eq!(ids(&a), ["m:1", "m:2", "m:3", "o:1"]);
    assert_eq!(
        a.report.bindings[0].detail,
        "Waiting for x: the adjusted copy 'Copy' is skipped because x failed to load"
    );
    plan.glue_patches = vec![glue("y:1").into()];
    let p = with_refs(&plan, &[("y:1", rf("y", "y:1", None))]);
    let a = applied(&failed, &p);
    assert_eq!(ids(&a), ["m:1", "m:2", "m:3", "o:1"]);
    assert_eq!(
        status(&a, "glue:glue:g"),
        [BindingStatus::Waiting {
            mods: vec!["y".into()]
        }]
    );
    assert!(a.report.bindings[0]
        .detail
        .ends_with("because y needs x, which failed to load"));
}

#[test]
fn a_ref_on_one_field_binds_every_field_with_that_entry() {
    let (raw, mut plan) = world();
    plan.disabled_patches = vec!["m:old".to_string().into()];
    plan.patch_priorities = [("m:old".to_string(), 3.into())].into();
    let p = with_refs(&plan, &[("m:old", moved_ref(false))]);
    let a = applied(&raw, &p);
    assert_eq!(disabled(&a), ["m:2"]);
    assert_eq!(prio(&a, "m:2"), 3);
    assert_eq!(a.report.bindings.len(), 2);
    // An invalid ref counts as none.
    let mut p = plan.clone();
    p.refs.insert(
        "m:old".into(),
        Lenient::Invalid {
            raw: serde_json::json!(5),
            error: "bad".into(),
        },
    );
    let a = applied(&raw, &p);
    assert!(disabled(&a).is_empty());
    assert!(a.report.bindings.is_empty());
}

/// How an `at` patch runs (D46): the profile's `apply_as` beats a compat mod's, which beats
/// the author's (D13). A profile entry carries its fix id.
#[test]
fn apply_as_layers_profile_over_compat_over_author() {
    let mut at: Patch = serde_json::from_value(serde_json::json!({"pattern": {
        "id": "m:at", "target": "main.lua", "pattern": "x", "position": "at",
        "payload": "x\ny", "apply_as": "edits"}}))
    .unwrap();
    at.set_metadata("m".to_string(), "m:at".to_string(), None);
    let mut raw = raw_of(vec![("m", vec![P(at, "qm.toml", 1)]), ("c", Vec::new())]);
    let mode = |a: &Applied| -> (ApplyAs, Option<ApplyAs>, Vec<String>) {
        let Some((Patch::Pattern(p), _, _)) = a.patches.first() else {
            panic!("no pattern patch");
        };
        (p.asked_apply_as(), p.qm.apply_as, p.qm.fix_ids.clone())
    };
    let none = apply_plan(&raw, &PlanLoad::default());
    assert_eq!(mode(&none), (ApplyAs::Edits, None, Vec::new()));

    // A compat mod that requires m runs it literal.
    raw.mods[1].requires.insert("m".into());
    raw.overrides.push((
        1,
        Lenient::Ok(Override {
            patch: "m:at".into(),
            apply_as: Some(ApplyAs::Literal),
            ..Override::default()
        }),
    ));
    let compat = apply_plan(&raw, &PlanLoad::default());
    assert_eq!(
        mode(&compat),
        (ApplyAs::Literal, Some(ApplyAs::Literal), Vec::new())
    );
    assert_eq!(compat.report.overrides[0].status, "applied");
    assert_eq!(compat.report.overrides[0].action, "apply_as");

    // The profile wins and names its fix.
    let mut plan = LoadOrderPlan {
        enabled_mods: vec!["m".into(), "c".into()],
        ..LoadOrderPlan::default()
    };
    plan.apply_as
        .insert("m:at".into(), Lenient::Ok(ApplyAs::Edits));
    plan.fix_of
        .insert("apply_as:m:at".into(), Lenient::Ok("f1".into()));
    let profile = applied(&raw, &plan);
    assert_eq!(
        mode(&profile),
        (ApplyAs::Edits, Some(ApplyAs::Edits), vec!["f1".to_string()])
    );
    assert_eq!(profile.report.overrides[0].status, "superseded_by_profile");

    // An entry that does not parse is skipped with a warning.
    plan.apply_as.insert(
        "m:at".into(),
        Lenient::Invalid {
            raw: serde_json::json!("sometimes"),
            error: "unknown variant".into(),
        },
    );
    assert!(plan.warnings().iter().any(|w| w.contains("apply_as")));
    let skipped = applied(&raw, &plan);
    assert_eq!(mode(&skipped).0, ApplyAs::Literal);
}

/// A profile `apply_as` entry with a ref binds like a priority: it follows a renamed patch,
/// records its binding under `apply_as:<entry>`, and never lands on a patch that only shares
/// the anchor (D20).
#[test]
fn apply_as_entries_bind_through_their_ref() {
    let at = |id: &str, payload: &str| -> Patch {
        let mut p: Patch = serde_json::from_value(serde_json::json!({"pattern": {
            "id": id, "target": "main.lua", "pattern": "x", "position": "at",
            "payload": payload}}))
        .unwrap();
        p.set_metadata("m".to_string(), id.to_string(), None);
        p
    };
    let made = at("m:at", "x\ny");
    let mode = |a: &Applied| -> Vec<(String, ApplyAs, Vec<String>)> {
        a.patches
            .iter()
            .filter_map(|(p, _, _)| match p {
                Patch::Pattern(x) => Some((
                    x.id.clone().unwrap_or_default(),
                    x.asked_apply_as(),
                    x.qm.fix_ids.clone(),
                )),
                _ => None,
            })
            .collect()
    };
    let mut plan = LoadOrderPlan {
        enabled_mods: vec!["m".into()],
        ..LoadOrderPlan::default()
    };
    plan.apply_as
        .insert("m:at".into(), Lenient::Ok(ApplyAs::Edits));
    plan.refs.insert(
        "m:at".into(),
        Lenient::Ok(rf("m", "m:at", Some((&made, "qm.toml", 1)))),
    );
    plan.fix_of
        .insert("apply_as:m:at".into(), Lenient::Ok("f1".into()));

    // The author renamed it: the entry follows the content.
    let raw = raw_of(vec![("m", vec![P(at("m:renamed", "x\ny"), "qm.toml", 1)])]);
    let a = applied(&raw, &plan);
    assert_eq!(
        mode(&a),
        [("m:renamed".to_string(), ApplyAs::Edits, vec!["f1".to_string()])]
    );
    let b = &a.report.bindings[0];
    assert_eq!((b.key.as_str(), b.fix_id.as_str()), ("apply_as:m:at", "f1"));
    assert!(matches!(b.status, BindingStatus::Rebound { .. }), "{b:?}");

    // Only the anchor is the same: nothing runs another way until the user picks.
    let raw = raw_of(vec![("m", vec![P(at("m:other", "x\nz"), "qm.toml", 1)])]);
    let a = applied(&raw, &plan);
    assert_eq!(mode(&a)[0].1, ApplyAs::Literal);
    assert_eq!(a.report.bindings[0].status, ambiguous(&["m:other"]));

    // The mod is off: it waits, and the patch is left alone.
    let raw = raw_of(vec![("o", vec![P(at("o:at", "x\ny"), "qm.toml", 1)])]);
    let a = applied(&raw, &plan);
    assert_eq!(mode(&a)[0].1, ApplyAs::Literal);
    assert!(matches!(
        a.report.bindings[0].status,
        BindingStatus::Waiting { .. }
    ));
}

/// An `at` patch of mod `m` that changes `b` and adds `d` (Keep, Change, Keep, Insert).
fn hunk_world() -> (RawLoad, Patch) {
    let mut at: Patch = serde_json::from_value(serde_json::json!({"pattern": {
        "id": "m:at", "target": "main.lua", "pattern": "a\nb\nc", "position": "at",
        "payload": "a\nb2\nc\nd"}}))
    .unwrap();
    at.set_metadata("m".to_string(), "m:at".to_string(), None);
    let raw = raw_of(vec![
        ("m", vec![P(at.clone(), "qm.toml", 1)]),
        ("c", Vec::new()),
    ]);
    (raw, at)
}

fn hunk_key(at: &Patch, ordinal: u32, vanilla: &str) -> HunkKey {
    HunkKey {
        fp: at.fingerprint(Path::new(""), "qm.toml", 1).lite(),
        vanilla: vanilla.to_string(),
        ordinal,
        keep: Some("o:at".to_string()),
    }
}

/// What the plan made of patch `m:at`: its skips, its mode and its fixes.
fn skips_of(a: &Applied) -> (Vec<HunkSkip>, ApplyAs, Vec<String>) {
    let Some((Patch::Pattern(p), _, _)) = a.patches.first() else {
        panic!("no pattern patch");
    };
    (p.qm.skip_hunks.clone(), p.asked_apply_as(), p.qm.fix_ids.clone())
}

/// A profile skip binds through its ref, turns edits mode on and names its fix. A key whose
/// patch changed, or whose edit is gone, skips nothing and needs review (D49).
#[test]
fn skip_hunk_entries_bind_and_a_drifted_key_skips_nothing() {
    let (raw, at) = hunk_world();
    let mut plan = LoadOrderPlan {
        enabled_mods: vec!["m".into(), "c".into()],
        ..LoadOrderPlan::default()
    };
    let fix = |hunk: HunkKey| {
        Lenient::Ok(SkipHunkFix {
            fix_id: "f1".into(),
            patch: rf("m", "m:at", Some((&at, "qm.toml", 1))),
            hunk,
        })
    };
    plan.skip_hunks = vec![fix(hunk_key(&at, 0, "b"))];
    let a = applied(&raw, &plan);
    let want = HunkSkip {
        ordinal: 0,
        vanilla: "b".into(),
        keep: Some("o:at".into()),
    };
    assert_eq!(
        skips_of(&a),
        (vec![want], ApplyAs::Edits, vec!["f1".to_string()])
    );
    let b = &a.report.bindings[0];
    assert_eq!((b.key.as_str(), b.fix_id.as_str()), ("hunk:m:at:0", "f1"));
    assert_eq!(b.status, BindingStatus::Bound);

    // The edit is gone (another ordinal, or other text): nothing is skipped.
    for key in [hunk_key(&at, 1, "b"), hunk_key(&at, 0, "x")] {
        plan.skip_hunks = vec![fix(key)];
        let a = applied(&raw, &plan);
        assert_eq!(skips_of(&a), (Vec::new(), ApplyAs::Literal, Vec::new()));
        let b = &a.report.bindings[0];
        assert!(matches!(b.status, BindingStatus::Drifted { .. }), "{b:?}");
        assert!(b.detail.contains("nothing is skipped"), "{}", b.detail);
    }

    // The patch changed since the key was made.
    let mut key = hunk_key(&at, 0, "b");
    key.fp.payload = "0".repeat(32);
    plan.skip_hunks = vec![fix(key)];
    let a = applied(&raw, &plan);
    assert_eq!(skips_of(&a).0, Vec::new());
    assert!(a.report.bindings[0].detail.contains("changed since"));

    // A compat mod's skip applies when the profile has none; the profile's replaces it.
    let mut raw = raw;
    raw.mods[1].requires.insert("m".into());
    raw.overrides.push((
        1,
        Lenient::Ok(Override {
            patch: "m:at".into(),
            skip_hunks: vec![Lenient::Ok(HunkKey {
                keep: None,
                ..hunk_key(&at, 0, "b")
            })],
            ..Override::default()
        }),
    ));
    let compat = apply_plan(&raw, &PlanLoad::default());
    let (skips, mode, _) = skips_of(&compat);
    assert_eq!(
        (skips.len(), skips[0].keep.clone(), mode),
        (1, None, ApplyAs::Edits)
    );
    assert_eq!(compat.report.overrides[0].status, "applied");
    assert_eq!(compat.report.overrides[0].action, "skip_hunks");
    plan.skip_hunks = vec![fix(hunk_key(&at, 0, "b"))];
    let both = applied(&raw, &plan);
    assert_eq!(skips_of(&both).0[0].keep.as_deref(), Some("o:at"));
    assert_eq!(both.report.overrides[0].status, "superseded_by_profile");

    // The kept patch's mod must be required too (D12).
    raw.overrides[0].1 = Lenient::Ok(Override {
        patch: "m:at".into(),
        skip_hunks: vec![Lenient::Ok(hunk_key(&at, 0, "b"))],
        ..Override::default()
    });
    let gated = apply_plan(&raw, &PlanLoad::default());
    assert!(skips_of(&gated).0.is_empty());
    assert_eq!(gated.report.overrides[0].status, "rejected_not_required");

    // A compat key whose edit is gone is reported and skips nothing.
    raw.overrides[0].1 = Lenient::Ok(Override {
        patch: "m:at".into(),
        skip_hunks: vec![Lenient::Ok(HunkKey {
            keep: None,
            ..hunk_key(&at, 3, "b")
        })],
        ..Override::default()
    });
    let gone = apply_plan(&raw, &PlanLoad::default());
    assert!(skips_of(&gone).0.is_empty());
    let r = &gone.report.overrides[0];
    assert_eq!(
        (r.status.as_str(), r.action.as_str()),
        ("suspended_drift", "skip_hunks")
    );

    // An entry that does not parse is skipped with a warning.
    plan.skip_hunks = vec![Lenient::Invalid {
        raw: serde_json::json!({"fix_id": 1}),
        error: "bad".into(),
    }];
    assert!(plan.warnings().iter().any(|w| w.contains("skip_hunks")));
}
