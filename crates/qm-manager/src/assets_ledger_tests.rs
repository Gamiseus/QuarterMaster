use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use qm_core::patch::faction::LoadedFaction;
use qm_core::patch::loader::{
    build_auto_asset_patch, classify_asset_relative_path, AutoAssetManifest,
};
use qm_core::patch::table::PatchTable;
use qm_schema::faction::{ResolvedFaction, ResolvedVehicle, ResolvedVoiceDir, ResolvedWeapon, Side};

use super::*;
use crate::vanilla_assets::{VanillaAsset, VanillaAssetListing};

fn game(paths: &[&str]) -> VanillaAssetListing {
    VanillaAssetListing::new(
        paths
            .iter()
            .map(|p| VanillaAsset {
                path: (*p).to_string(),
                size: 1,
                width: p.ends_with(".png").then_some(32),
                height: p.ends_with(".png").then_some(48),
            })
            .collect(),
    )
}

fn manifest(mod_id: &str, dir: &Path, rels: &[&str]) -> AutoAssetManifest {
    AutoAssetManifest {
        mod_id: mod_id.to_string(),
        dir: dir.to_string_lossy().replace('\\', "/"),
        assets: rels
            .iter()
            .map(|r| classify_asset_relative_path(r).unwrap())
            .collect(),
    }
}

fn faction(id: &str, root: &Path) -> LoadedFaction {
    LoadedFaction {
        mod_id: id.to_string(),
        priority: 50,
        root: root.to_path_buf(),
        faction: ResolvedFaction {
            id: id.to_string(),
            name: id.to_string(),
            replaces: Side::Allied,
            side_name: None,
            description: None,
            frames: BTreeMap::new(),
            weapons: Vec::new(),
            spawn_swap: BTreeMap::new(),
            pools: BTreeMap::new(),
            classes: BTreeMap::new(),
            names: None,
            voices: Vec::new(),
            text: BTreeMap::new(),
            sprites: BTreeMap::new(),
            sounds: BTreeMap::new(),
            vehicles: Vec::new(),
            hooks: None,
        },
    }
}

fn ledger(
    manifests: &[AutoAssetManifest],
    factions: &[LoadedFaction],
    listing: Option<&VanillaAssetListing>,
    choices: &[(&str, &str)],
) -> AssetLedger {
    let choices: BTreeMap<String, String> = choices
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    build(&LedgerInput {
        manifests,
        factions,
        listing,
        choices: &choices,
        last_run: &BTreeMap::new(),
    })
}

fn codes(notes: &[LedgerNote]) -> Vec<&str> {
    notes.iter().map(|n| n.code.as_str()).collect()
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

/// The shape of the user's axisSW: 29 of 30 hurt voices, five garand sounds where the game
/// has four, one sprite of another size, and one brand-new sound.
#[test]
fn groups_are_numbered_sets_and_folders_where_sources_meet() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("assets/sprites")).unwrap();
    std::fs::write(dir.path().join("assets/sprites/allied_idle.png"), png(64, 48)).unwrap();

    let mut paths: Vec<String> = (1..=30).map(|n| format!("sounds/vo/axis/axisHurt{n}.wav")).collect();
    paths.extend((1..=4).map(|n| format!("sounds/garand{n}.wav")));
    paths.extend(["sprites/allied_idle.png", "sprites/allied_aim.png", "fonts/impact.ttf"].map(String::from));
    let listing = game(&paths.iter().map(String::as_str).collect::<Vec<_>>());

    let mut rels: Vec<String> = (1..=30)
        .filter(|n| *n != 11)
        .map(|n| format!("assets/sounds/vo/axis/axisHurt{n}.wav"))
        .collect();
    rels.extend((1..=5).map(|n| format!("assets/sounds/garand{n}.wav")));
    rels.extend(["assets/sprites/allied_idle.png", "assets/sounds/brandNew.wav", "assets/fonts/impact.ttf"].map(String::from));
    let m = manifest("axissw", dir.path(), &rels.iter().map(String::as_str).collect::<Vec<_>>());

    let l = ledger(&[m], &[], Some(&listing), &[]);
    assert!(l.has_game_files);
    let keys: Vec<_> = l.groups.iter().map(|g| (g.key.as_str(), g.kind)).collect();
    assert_eq!(
        keys,
        [
            ("fonts", GroupKind::Folder),
            ("sounds/garand#", GroupKind::Set),
            ("sounds/vo/axis/axishurt#", GroupKind::Set),
            ("sprites", GroupKind::Folder),
        ]
    );

    // 29 of the game's 30: the group says so, and the mod's coarser folder still matches.
    let hurt = l.group("sounds/vo/axis/axishurt#").unwrap();
    assert_eq!(hurt.label, "sounds/vo/axis/axisHurt#");
    assert_eq!((hurt.game_count, hurt.slots.len()), (30, 29));
    assert_eq!(hurt.claimants, [GroupClaimant { source: "axissw".into(), files: 29 }]);
    assert_eq!(codes(&hurt.notes), ["only_some_of_set"]);
    assert_eq!(hurt.notes[0].text, "axissw has 29 of the game's 30; it has no file for the other 1.");
    assert!(hurt.slots.iter().all(|s| s.winner == "axissw" && s.notes.is_empty()));
    assert_eq!(hurt.slots[0].number, Some(1));
    assert_eq!(hurt.slots[10].number, Some(12), "sorted by number; 11 is not claimed");
    assert_eq!(hurt.slots[0].name, "axisHurt1");

    // Five where the game has four: the fifth is an addition and nothing is renumbered.
    let garand = l.group("sounds/garand#").unwrap();
    assert_eq!((garand.game_count, garand.slots.len()), (4, 5));
    assert!(garand.notes.is_empty(), "all four of the game's are replaced");
    let fifth = l.slot("sounds/garand5").unwrap();
    assert!(fifth.game.is_none());
    assert_eq!(codes(&fifth.notes), ["added_number"]);

    // A folder group lists only the slots where two sources meet.
    let sprites = l.group("sprites").unwrap();
    assert_eq!(sprites.slots.len(), 1);
    let idle = &sprites.slots[0];
    assert_eq!((idle.key.as_str(), idle.kind), ("sprites/allied_idle", AssetKind::Image));
    assert_eq!(idle.claims[0].width, Some(64));
    assert_eq!(codes(&idle.notes), ["asset_size_differs"]);
    assert!(idle.notes[0].text.contains("64x48") && idle.notes[0].text.contains("32x48"));
    assert_eq!(codes(&l.slot("fonts/impact").unwrap().notes), ["font_all_sizes"]);

    // A brand-new file is no overlap.
    assert!(l.slot("sounds/brandnew").is_none());
    assert!(exclusions(&l).is_empty(), "no choice leaves nothing out");
}

#[test]
fn the_last_source_wins_until_a_choice_names_another() {
    let d = Path::new("D:/Mods");
    let listing = game(&["sounds/garand1.wav", "sounds/garand2.wav", "sounds/garand3.wav"]);
    let a = manifest("a", d, &["assets/sounds/garand1.wav", "assets/sounds/garand2.wav"]);
    let b = manifest("b", d, &["assets/sounds/garand1.wav", "assets/sounds/garand3.wav"]);
    let ms = [a, b];

    let l = ledger(&ms, &[], Some(&listing), &[]);
    let g = l.group("sounds/garand#").unwrap();
    assert_eq!(
        g.claimants,
        [
            GroupClaimant { source: "a".into(), files: 2 },
            GroupClaimant { source: "b".into(), files: 2 }
        ]
    );
    let winners = |l: &AssetLedger| -> Vec<String> {
        l.group("sounds/garand#").unwrap().slots.iter().map(|s| s.winner.clone()).collect()
    };
    assert_eq!(winners(&l), ["b", "a", "b"]);

    // A group choice: `a` wins where it has a file; where it has none, today's winner stays.
    let l = ledger(&ms, &[], Some(&listing), &[("sounds/garand#", "a")]);
    assert_eq!(winners(&l), ["a", "a", "b"]);
    assert_eq!(codes(&l.slot("sounds/garand3").unwrap().notes), ["filled_from"]);
    assert_eq!(
        exclusions(&l),
        BTreeMap::from([("b".to_string(), vec!["assets/sounds/garand1.wav".to_string()])])
    );

    // A slot choice beats the group's; `vanilla` leaves every file of the slot out.
    let l = ledger(
        &ms,
        &[],
        Some(&listing),
        &[("sounds/garand#", "a"), ("sounds/garand1", "b"), ("sounds/garand2", "vanilla")],
    );
    assert_eq!(winners(&l), ["b", "vanilla", "b"]);
    assert_eq!(l.slot("sounds/garand1").unwrap().choice.as_deref(), Some("b"));
    assert_eq!(
        exclusions(&l),
        BTreeMap::from([(
            "a".to_string(),
            vec!["assets/sounds/garand1.wav".to_string(), "assets/sounds/garand2.wav".to_string()]
        )])
    );

    // Vanilla for the whole group.
    let l = ledger(&ms, &[], Some(&listing), &[("sounds/garand#", "vanilla")]);
    assert_eq!(winners(&l), ["vanilla", "vanilla", "vanilla"]);
    let ex = exclusions(&l);
    assert_eq!(ex["a"].len(), 2);
    assert_eq!(ex["b"].len(), 2);
    assert!(l.has_key("sounds/garand#") && l.has_key("sounds/garand2") && !l.has_key("sounds"));
}

#[test]
fn a_faction_loads_after_every_mod_and_is_left_out_by_slot() {
    let d = Path::new("D:/Mods");
    let listing = game(&[
        "sprites/allied_idle.png",
        "sprites/top_m1.png",
        "sounds/vo/allied/contact1.wav",
        "sounds/vo/allied/contact2.wav",
        "sounds/vo/allied/contact3.wav",
        "sounds/vo/allied/hurt1.wav",
        "sounds/vo/allied/deep/contact9.wav",
        "sounds/springfield.wav",
        "sprites/sherman_debris1.png",
        "sprites/sherman_debris2.png",
        "sprites/sherman_debris3.png",
    ]);
    let m = manifest(
        "axissw",
        d,
        &["assets/sprites/allied_idle.png", "assets/sounds/vo/allied/contact1.wav"],
    );
    let mut f = faction("eastern_front", Path::new("D:/Mods/EasternFront"));
    f.faction
        .frames
        .insert("allied".into(), BTreeMap::from([("idle".to_string(), "units/base/idle.png".to_string())]));
    f.faction.weapons.push(ResolvedWeapon {
        tag: "m1".into(),
        mode: "reskin".into(),
        based_on: None,
        top: Some("weapons/mosin_top.png".into()),
        side: None,
        sounds: Vec::new(),
        sound_name: None,
        mag_name: None,
        mag_file: None,
        stats: Default::default(),
    });
    f.faction.sounds.insert("springfieldSound".into(), "weapons/mosin.wav".into());
    f.faction.voices.push(ResolvedVoiceDir {
        vanilla_dir: "sounds/vo/allied".into(),
        fill_missing: true,
        exact: BTreeMap::from([
            ("contact1".to_string(), "voices/contact1.wav".to_string()),
            ("notInTheGame".to_string(), "voices/x.wav".to_string()),
        ]),
        pools: BTreeMap::from([("contact".to_string(), vec!["voices/a.wav".to_string(), "voices/b.wav".to_string()])]),
    });
    f.faction.vehicles.push(ResolvedVehicle {
        tag: "sherman".into(),
        name: None,
        parts: BTreeMap::new(),
        debris: vec!["t34/d1.png".to_string(), "t34/d2.png".to_string()],
        call_signs: Vec::new(),
        spawner: None,
    });
    let fs = [f];

    let l = ledger(std::slice::from_ref(&m), &fs, Some(&listing), &[]);
    let src = "faction:eastern_front";
    let idle = l.slot("sprites/allied_idle").unwrap();
    assert_eq!(idle.claims.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["axissw", src]);
    assert_eq!(idle.winner, src, "a faction loads after the mods");
    assert_eq!(l.slot("sprites/top_m1").unwrap().winner, src);
    assert_eq!(l.slot("sounds/springfield").unwrap().winner, src);
    // Voices, as the runtime spreads them: the exact name first, then (fill_missing) the
    // faction's own lines in turn over the game's other numbered lines of that folder.
    let voice = |n: u32| l.slot(&format!("sounds/vo/allied/contact{n}")).unwrap();
    assert_eq!(voice(1).claims.len(), 2);
    assert_eq!(voice(1).claims[1].file, "voices/contact1.wav");
    assert_eq!(voice(1).claims[1].target.as_deref(), Some("sounds/vo/allied/contact1"));
    assert_eq!(voice(2).claims[0].file, "voices/b.wav");
    assert_eq!(voice(3).claims[0].file, "voices/a.wav");
    assert!(l.slot("sounds/vo/allied/hurt1").is_none(), "no pool for that name");
    assert!(l.slot("sounds/vo/allied/deep/contact9").is_none(), "another folder");
    assert!(l.slot("sounds/vo/allied/notinthegame").is_none(), "only the game's files are replaced");
    // Two debris images over the game's three frames.
    let debris: Vec<_> = (1..=3)
        .map(|i| l.slot(&format!("sprites/sherman_debris{i}")).unwrap().claims[0].file.clone())
        .collect();
    assert_eq!(debris, ["t34/d1.png", "t34/d2.png", "t34/d1.png"]);
    assert_eq!(idle.claims[1].target.as_deref(), Some("allied_idle"));
    assert_eq!(idle.claims[0].target, None, "a mod's file is named by its path");
    assert_eq!(
        l.slot("sounds/springfield").unwrap().claims[0].target.as_deref(),
        Some("springfieldSound"),
        "a sound is left out by its global"
    );
    let voices = l.group("sounds/vo/allied/contact#").unwrap();
    assert_eq!(voices.claimants.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["axissw", src]);

    // The mod chosen over the faction: the faction leaves the global alone.
    let l = ledger(
        std::slice::from_ref(&m),
        &fs,
        Some(&listing),
        &[("sprites/allied_idle", "axissw"), ("sounds/vo/allied/contact1", "vanilla")],
    );
    assert_eq!(l.slot("sprites/allied_idle").unwrap().winner, "axissw");
    assert_eq!(
        exclusions(&l),
        BTreeMap::from([
            ("axissw".to_string(), vec!["assets/sounds/vo/allied/contact1.wav".to_string()]),
            (
                src.to_string(),
                vec!["allied_idle".to_string(), "sounds/vo/allied/contact1".to_string()]
            ),
        ])
    );
    // The faction chosen over the mod: the mod's file is left out by its path.
    let l = ledger(std::slice::from_ref(&m), &fs, Some(&listing), &[("sprites/allied_idle", src)]);
    assert_eq!(
        exclusions(&l),
        BTreeMap::from([("axissw".to_string(), vec!["assets/sprites/allied_idle.png".to_string()])])
    );
}

#[test]
fn without_the_games_file_list_only_files_two_sources_ship_are_listed() {
    let d = Path::new("D:/Mods");
    let a = manifest("a", d, &["assets/sounds/garand1.wav", "assets/sounds/only_a.wav"]);
    let b = manifest("b", d, &["assets/sounds/garand1.wav", "assets/text/weaponText_m1.txt"]);
    let c = manifest("c", d, &["assets/text/weaponText_m1.txt"]);
    let l = ledger(&[a, b, c], &[], None, &[]);
    assert!(!l.has_game_files);
    let slots: Vec<_> = l.groups.iter().flat_map(|g| &g.slots).map(|s| (s.key.as_str(), s.winner.as_str())).collect();
    assert_eq!(slots, [("sounds/garand1", "b"), ("text/weapontext_m1", "c")]);
    assert_eq!(l.slot("text/weapontext_m1").unwrap().kind, AssetKind::Text);
    assert!(l.groups.iter().all(|g| g.kind == GroupKind::Folder && g.game_count == 0));
}

#[test]
fn a_file_name_the_game_has_in_several_folders_fills_them_all() {
    let d = Path::new("D:/Mods");
    let listing = game(&["sprites/icon.png", "ui/icon.png", "sounds/icon.wav"]);
    // `assets/gfx/...` is the sprites folder; `assets/extra/...` has no folder of its own.
    let exact = manifest("exact", d, &["assets/gfx/icon.png"]);
    let loose = manifest("loose", d, &["assets/cheevos/icon.png"]);
    let l = ledger(&[exact, loose], &[], Some(&listing), &[]);
    let sprites = l.slot("sprites/icon").unwrap();
    assert_eq!(sprites.claims.len(), 2);
    assert_eq!(codes(&sprites.notes), ["matches_several"]);
    let ui = l.slot("ui/icon").unwrap();
    assert_eq!(ui.claims.iter().map(|c| c.source.as_str()).collect::<Vec<_>>(), ["loose"]);
    assert!(l.slot("sounds/icon").is_none(), "an image never fills a sound");
}

#[test]
fn the_last_runs_winner_is_shown_when_it_differs() {
    let d = Path::new("D:/Mods");
    let listing = game(&["sounds/garand1.wav"]);
    let ms = [
        manifest("a", d, &["assets/sounds/garand1.wav"]),
        manifest("b", d, &["assets/sounds/garand1.wav"]),
    ];
    let run = |winner: &str| {
        build(&LedgerInput {
            manifests: &ms,
            factions: &[],
            listing: Some(&listing),
            choices: &BTreeMap::new(),
            last_run: &BTreeMap::from([("sounds/garand1".to_string(), winner.to_string())]),
        })
    };
    assert!(run("b").slot("sounds/garand1").unwrap().notes.is_empty());
    let l = run("a");
    let s = l.slot("sounds/garand1").unwrap();
    assert_eq!(s.winner, "b");
    assert_eq!(s.last_run_winner.as_deref(), Some("a"));
    assert_eq!(codes(&s.notes), ["last_run_differs"]);
}

#[test]
fn manifests_follow_the_order_of_their_blocks_in_main_lua() {
    let asset = |rel: &str| classify_asset_relative_path(rel).unwrap();
    let mut table = PatchTable::default();
    let mut payloads = Vec::new();
    for id in ["first", "second", "third"] {
        let patch = build_auto_asset_patch(
            id,
            Path::new("D:/Mods").join(id).as_path(),
            &[asset("assets/sounds/garand1.wav")],
            50,
        )
        .unwrap();
        if let qm_core::patch::Patch::Pattern(p) = &patch {
            payloads.push(p.payload.clone());
        }
        table.patches.push((patch, 50, PathBuf::from(format!("{id}/qm.toml"))));
    }
    // Without the patched text: the table's order.
    let ids = |ms: Vec<AutoAssetManifest>| ms.into_iter().map(|m| m.mod_id).collect::<Vec<_>>();
    assert_eq!(ids(manifests_in_run_order(&table, None)), ["first", "second", "third"]);
    // With it: the order of the blocks; a block that did not land loads nothing.
    let text = format!("{}\n{}\nsetOptionVolume()\n", payloads[2], payloads[0]);
    assert_eq!(ids(manifests_in_run_order(&table, Some(&text))), ["third", "first"]);
}
