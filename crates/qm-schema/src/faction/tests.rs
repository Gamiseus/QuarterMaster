use super::*;
use std::path::PathBuf;
use tempfile::TempDir;

/// A mod folder with the given files (contents don't matter for checking).
fn mod_with(files: &[&str]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for f in files {
        let p = dir.path().join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, b"x").unwrap();
    }
    dir
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p: PathBuf = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn errors(r: &FactionReport) -> Vec<String> {
    r.diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .map(|d| d.message.clone())
        .collect()
}

fn warnings(r: &FactionReport) -> Vec<String> {
    r.diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Warning)
        .map(|d| d.message.clone())
        .collect()
}

const HEADER: &str = "schema = 1\nid = \"red_army\"\nname = \"Red Army\"\nreplaces = \"allied\"\n";

#[test]
fn full_faction_resolves() {
    let dir = mod_with(&[
        "units/base/idle.png",
        "units/base/aim.png",
        "units/base/reload1.png",
        "units/base/notes.png",
        "units/sarge/idle.png",
        "units/sarge/smoking1.png",
        "weapons/mosin_top.png",
        "weapons/mosin_side.png",
        "weapons/mosin1.wav",
        "weapons/mosin2.wav",
        "weapons/mosin10.wav",
        "weapons/pps43_top.png",
        "weapons/pps43_side.png",
        "weapons/pps43.ogg",
        "voices/contact1.wav",
        "voices/contact2.wav",
        "voices/hurt1.wav",
        "voices/advance.wav",
        "voices_fr/frenchKill1.wav",
        "vehicles/t34/body.png",
        "vehicles/t34/turret.png",
        "vehicles/t34/debris1.png",
        "vehicles/t34/debris2.png",
        "vehicles/t34/second.png",
        "icons/rifle.png",
        "flag.png",
        "horn.wav",
        "hooks.lua",
    ]);
    write(
        dir.path(),
        "names/last.txt",
        "\u{feff}Ivanov\n# comment\n\nPetrov\r\n",
    );
    let toml = format!(
        r#"{HEADER}side_name = "Soviet"

[units.base]
folder = "units/base"

[units.sarge]
folder = "units/sarge"
copy_from = "base"

[[weapons]]
reskin = "m1"
name = "Mosin"
description = "Bolt rifle."
top_sprite = "weapons/mosin_top.png"
side_sprite = "weapons/mosin_side.png"
sound = "weapons/mosin"
stats = {{ action = "bolt", ammo = 5, maxAmmo = 5 }}

[[weapons]]
tag = "pps43"
based_on = "m3"
name = "PPS-43"
top_sprite = "weapons/pps43_top.png"
side_sprite = "weapons/pps43_side.png"
sound = "weapons/pps43"
loot_side = "allied"
pools = ["smg"]

[spawn_swap]
tommy = "ppsh41"

[pools.grunt]
add = ["pps43"]
remove = ["springfield"]

[classes.rifle]
name = "Rifleman"
description = "Strelok."
start_weapon = "m1"
unlocks = ["svt40", "ppsh41", "pps43"]
icon = "icons/rifle.png"

[classes.gunner]
name = "Machine Gunner"
bonus = {{ speed = -10, detect_range = 1.2 }}

[classes.commando]
description = "Razvedchik."
skills = ["a", "b", "c"]

[names]
last_file = "names/last.txt"
vehicles = ["Za Rodinu"]

[voices]
folder = "voices"
map = {{ hurt = "alliedHurt", advance = "advanceOrder" }}
dirs = {{ frenchies = "voices_fr" }}

[text]
mouseOverLightSpec = "Scout"

[sprites]
parachuteAllied = "flag.png"

[sounds]
airRaid = "horn.wav"

[[vehicles]]
reskin = "sherman"
name = "T-34/76"
body = "vehicles/t34/body.png"
turret = "vehicles/t34/turret.png"
debris = "vehicles/t34/debris"
parts = {{ SecondTurret1 = "vehicles/t34/second.png" }}

[hooks]
lua = "hooks.lua"
"#
    );
    let r = check_faction(&toml, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", r.diagnostics);
    let f = r.faction.clone().expect("resolves");

    // Frames are keyed by vanilla unit tag; stray files are reported, copy_from fills gaps.
    let base = &f.frames["allied"];
    assert_eq!(base["idle"], "units/base/idle.png");
    assert_eq!(base["reload1"], "units/base/reload1.png");
    assert!(!base.contains_key("notes"));
    assert!(warnings(&r).iter().any(|w| w.contains("notes.png")));
    let sarge = &f.frames["sarge"];
    assert_eq!(sarge["idle"], "units/sarge/idle.png");
    assert_eq!(sarge["aim"], "units/base/aim.png");
    assert_eq!(sarge["smoking1"], "units/sarge/smoking1.png");

    // Weapons: numbered sound pool in number order, single file pool, text folded in.
    let mosin = &f.weapons[0];
    assert_eq!(mosin.mode, "reskin");
    assert_eq!(
        mosin.sounds,
        vec![
            "weapons/mosin1.wav",
            "weapons/mosin2.wav",
            "weapons/mosin10.wav"
        ]
    );
    assert_eq!(f.weapons[1].sounds, vec!["weapons/pps43.ogg"]);
    assert_eq!(f.text["weaponText_m1"], "Bolt rifle.");
    assert_eq!(f.text["weaponText_pps43"], "PPS-43");

    // Pools: loot_side and pools merge with [pools].
    assert_eq!(f.pools["allied"].add, vec!["pps43"]);
    assert_eq!(f.pools["smg"].add, vec!["pps43"]);
    assert_eq!(f.pools["grunt"].remove, vec!["springfield"]);

    // Classes: full text, name-only and description-only markers, skills, commando alias.
    assert_eq!(f.text["mouseOverRifleSpec"], "Rifleman, Strelok.");
    assert_eq!(f.text["mouseOverGunnerSpec::name"], "Machine Gunner");
    assert_eq!(f.text["mouseOverCommandoSpec::description"], "Razvedchik.");
    assert_eq!(f.text["mouseOverSOFSpec::description"], "Razvedchik.");
    assert_eq!(f.text["commando_skillText_2"], "b");
    assert_eq!(f.classes["rifle"].icon.as_deref(), Some("icons/rifle.png"));
    assert_eq!(f.text["mouseOverLightSpec"], "Scout");

    // Names: comments, blank lines, BOM and CRLF are dropped.
    let names = f.names.as_ref().unwrap();
    assert_eq!(
        names.soldiers.as_ref().unwrap().last,
        vec!["Ivanov", "Petrov"]
    );
    assert_eq!(names.vehicles, vec!["Za Rodinu"]);

    // Voices: renamed numbered pools and singles, extra folder.
    let main = f
        .voices
        .iter()
        .find(|v| v.vanilla_dir == "sounds/vo/allied")
        .unwrap();
    assert_eq!(main.exact["alliedHurt1"], "voices/hurt1.wav");
    assert_eq!(main.exact["advanceOrder"], "voices/advance.wav");
    assert_eq!(
        main.pools["contact"],
        vec!["voices/contact1.wav", "voices/contact2.wav"]
    );
    assert!(f
        .voices
        .iter()
        .any(|v| v.vanilla_dir == "sounds/vo/frenchies"));

    // Vehicles: named parts and free parts become game globals.
    let t34 = &f.vehicles[0];
    assert_eq!(t34.parts["shermanBody"], "vehicles/t34/body.png");
    assert_eq!(t34.parts["shermanSecondTurret1"], "vehicles/t34/second.png");
    assert_eq!(t34.debris.len(), 2);
    assert_eq!(f.hooks.as_deref(), Some("hooks.lua"));
    assert_eq!(f.sprites["parachuteAllied"], "flag.png");
}

#[test]
fn unknown_keys_and_bad_toml_are_errors() {
    let dir = mod_with(&[]);
    let r = check_faction(&format!("{HEADER}colour = \"red\"\n"), dir.path());
    assert!(r.faction.is_none());
    assert!(errors(&r)[0].contains("colour"), "{:?}", r.diagnostics);

    let r = check_faction("schema = 1\nid = ", dir.path());
    assert!(r.faction.is_none());
    assert_eq!(errors(&r).len(), 1);

    let r = check_faction(&HEADER.replace("schema = 1", "schema = 2"), dir.path());
    assert!(errors(&r)[0].contains("schema = 2"));

    let r = check_faction(&HEADER.replace("red_army", "Red Army"), dir.path());
    assert!(errors(&r)[0].contains("lowercase"));
}

#[test]
fn weapon_rules() {
    let dir = mod_with(&["a.png"]);
    let cases = [
        ("[[weapons]]\nreskin = \"m1\"\ntag = \"x\"\n", "not both"),
        ("[[weapons]]\nname = \"x\"\n", "needs reskin"),
        ("[[weapons]]\nreskin = \"m99\"\n", "not a vanilla weapon"),
        (
            "[[weapons]]\ntag = \"m1\"\nbased_on = \"m1\"\n",
            "already a vanilla weapon",
        ),
        ("[[weapons]]\ntag = \"new\"\n", "needs based_on"),
        (
            "[[weapons]]\ntag = \"new\"\nbased_on = \"nope\"\n",
            "based_on \"nope\"",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\nloot_side = \"allied\"\n",
            "only apply to new weapons",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\n[[weapons]]\nreskin = \"m1\"\n",
            "more than once",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\nstats = { action = \"pump\" }\n",
            "stats.action",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\nstats = { ammo = 0 }\n",
            "at least 1",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\nstats = { range = -5.0 }\n",
            "can't be negative",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\nsound = \"weapons/none\"\n",
            "no wav/ogg/mp3 files",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\ntop_sprite = \"../outside.png\"\n",
            "inside the mod folder",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\ntop_sprite = \"C:/x.png\"\n",
            "inside the mod folder",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\ntop_sprite = \"missing.png\"\n",
            "not found",
        ),
        (
            "[[weapons]]\nreskin = \"m1\"\ntop_sprite = \"a.txt\"\n",
            "must be one of",
        ),
        (
            "[[weapons]]\ntag = \"x\"\nbased_on = \"m3\"\npools = [\"bogus\"]\n",
            "not a weapon pool",
        ),
    ];
    for (body, expect) in cases {
        let r = check_faction(&format!("{HEADER}{body}"), dir.path());
        assert!(
            errors(&r).iter().any(|e| e.contains(expect)),
            "case {body:?}: expected {expect:?}, got {:?}",
            r.diagnostics
        );
        assert!(r.faction.is_none());
    }
}

#[test]
fn weapon_sound_can_name_a_game_sound() {
    let dir = mod_with(&["weapons/mosin1.wav"]);
    let r = check_faction(
        &format!(
            "{HEADER}[[weapons]]
reskin = \"m1\"
sound = \"springfieldSound\"
[[weapons]]
reskin = \"bar\"
sound = \"weapons/mosin\"
"
        ),
        dir.path(),
    );
    assert!(errors(&r).is_empty(), "{:?}", r.diagnostics);
    let f = r.faction.unwrap();
    assert_eq!(f.weapons[0].sound_name.as_deref(), Some("springfieldSound"));
    assert!(f.weapons[0].sounds.is_empty());
    assert_eq!(f.weapons[1].sound_name, None);
    assert_eq!(f.weapons[1].sounds, vec!["weapons/mosin1.wav"]);
}

#[test]
fn weapon_magazine_names() {
    let dir = mod_with(&["weapons/pan.png", "misc/drum.png"]);
    let r = check_faction(
        &format!(
            "{HEADER}[[weapons]]
reskin = \"m1\"
mag_sprite = \"ppshMag\"
[[weapons]]
reskin = \"bar\"
mag_sprite = \"weapons/pan.png\"
[[weapons]]
reskin = \"bren\"
mag_sprite = \"drumMag\"
[[weapons]]
reskin = \"sten\"
mag_sprite = \"noSuchMag\"
[sprites]
drumMag = \"misc/drum.png\"
"
        ),
        dir.path(),
    );
    assert!(errors(&r).is_empty(), "{:?}", r.diagnostics);
    let w = warnings(&r);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("noSuchMag") && w[0].contains("[sprites]"), "{w:?}");
    let f = r.faction.unwrap();
    assert_eq!(f.weapons[0].mag_name.as_deref(), Some("ppshMag"));
    assert_eq!(f.weapons[1].mag_file.as_deref(), Some("weapons/pan.png"));
    assert_eq!(f.weapons[2].mag_name.as_deref(), Some("drumMag"));
    for m in vanilla::MAGAZINES {
        assert!(vanilla::is_magazine(m));
    }
}

#[test]
fn spawn_swap_and_pools() {
    let dir = mod_with(&[]);
    let r = check_faction(
        &format!("{HEADER}[spawn_swap]\nk98 = \"svt40\"\n"),
        dir.path(),
    );
    assert!(errors(&r).is_empty());
    assert!(warnings(&r).iter().any(|w| w.contains("14 place(s)")));

    let r = check_faction(
        &format!("{HEADER}[spawn_swap]\ntommy = \"made_up\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("made_up"));

    let r = check_faction(
        &format!("{HEADER}[pools.allied]\nremove = [\"made_up\"]\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("not a vanilla weapon"));

    let r = check_faction(
        &format!("{HEADER}[pools.navy]\nadd = [\"m1\"]\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("not a weapon pool"));
}

#[test]
fn class_rules() {
    let dir = mod_with(&[]);
    let r = check_faction(
        &format!("{HEADER}[classes.pilot]\nname = \"x\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("not a class slot"));
    let r = check_faction(
        &format!("{HEADER}[classes.rifle]\nunlocks = [\"m1\"]\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("exactly 3"));
    let r = check_faction(
        &format!("{HEADER}[classes.rifle]\nstart_weapon = \"laser\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("laser"));
    let r = check_faction(
        &format!("{HEADER}[classes.rifle]\nbonus = {{ detect_range = 0 }}\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("multiplier"));
    let axis = HEADER.replace("\"allied\"", "\"axis\"");
    let r = check_faction(
        &format!("{axis}[classes.rifle]\nname = \"x\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("only the Allied side"));
}

#[test]
fn unit_rules() {
    let dir = mod_with(&["u/idle.png"]);
    let r = check_faction(
        &format!("{HEADER}[units.volks]\nfolder = \"u\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("no \"volks\" unit"));
    let r = check_faction(
        &format!("{HEADER}[units.base]\nframes = {{ jagerMove = \"u/idle.png\" }}\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("jagerMove"));
    let r = check_faction(
        &format!("{HEADER}[units.base]\nfolder = \"u\"\ncopy_from = \"base\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("itself"));
    let r = check_faction(
        &format!("{HEADER}[units.para]\ncopy_from = \"sarge\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("no [units.sarge]"));
    let r = check_faction(
        &format!("{HEADER}[units.base]\nfolder = \"missing\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("folder \"missing\" not found"));

    // Axis side: base maps to the vanilla "axis" tag, officers to "officer".
    let axis = HEADER.replace("\"allied\"", "\"axis\"");
    let r = check_faction(
        &format!("{axis}[units.base]\nfolder = \"u\"\n[units.officer]\ncopy_from = \"base\"\n"),
        dir.path(),
    );
    assert!(errors(&r).is_empty(), "{:?}", r.diagnostics);
    let f = r.faction.clone().unwrap();
    assert_eq!(f.frames["axis"]["idle"], "u/idle.png");
    assert_eq!(f.frames["officer"]["idle"], "u/idle.png");
}

#[test]
fn vehicle_names_and_voice_rules() {
    let dir = mod_with(&["v/b.png", "voices/x.wav"]);
    let r = check_faction(
        &format!("{HEADER}[[vehicles]]\nreskin = \"tiger\"\nbody = \"v/b.png\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("belongs to the Germans"));
    let r = check_faction(
        &format!("{HEADER}[[vehicles]]\nreskin = \"ufo\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("not a vanilla vehicle"));

    let r = check_faction(
        &format!("{HEADER}[names]\nfirst = [\"Ivan\"]\nformat = \"full\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("needs last names"));
    let r = check_faction(
        &format!("{HEADER}[names]\nlast = [\"Ivanov\"]\nlast_file = \"n.txt\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("not both"));
    let r = check_faction(
        &format!("{HEADER}[names]\nlast = [\"Ivanov\"]\nformat = \"full\"\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("needs first names"));

    let r = check_faction(
        &format!("{HEADER}[voices]\nfolder = \"voices\"\ndirs = {{ axis = \"voices\" }}\n"),
        dir.path(),
    );
    assert!(
        errors(&r)[0].contains("the Allies have no \"axis\" voice folder"),
        "{:?}",
        r.diagnostics
    );
    let r = check_faction(
        &format!("{HEADER}[voices]\nfill_missing = false\n"),
        dir.path(),
    );
    assert!(errors(&r)[0].contains("needs folder"));
}

#[test]
fn frame_lists_match_the_game() {
    let base = vanilla::frames(Side::Allied, "base");
    assert!(base.contains(&"flare7") && base.contains(&"deadBBQ") && base.contains(&"reload10"));
    let sarge = vanilla::frames(Side::Allied, "sarge");
    assert!(
        sarge.contains(&"smoking2") && !sarge.contains(&"deadBBQ") && !sarge.contains(&"flare1")
    );
    let axis = vanilla::frames(Side::Axis, "base");
    assert!(
        !axis.contains(&"jagerMove") && !axis.contains(&"analyze") && axis.contains(&"bazooka3")
    );
    assert!(vanilla::frames(Side::Axis, "volks").contains(&"jagerMove"));
    // No duplicates in any list.
    for side in [Side::Allied, Side::Axis] {
        for (v, _) in vanilla::variants(side) {
            let f = vanilla::frames(side, v);
            let set: BTreeSet<_> = f.iter().collect();
            assert_eq!(set.len(), f.len(), "{v}");
        }
    }
}

/// The editor schema repeats the vanilla lists; keep them in step.
#[test]
fn json_schema_matches_vanilla_lists() {
    let schema: serde_json::Value = serde_json::from_str(FACTION_JSON_SCHEMA).unwrap();
    let defs = &schema["definitions"];
    let list = |key: &str| -> BTreeSet<String> {
        defs[key]["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("{key} enum"))
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    let set =
        |items: Vec<&str>| -> BTreeSet<String> { items.into_iter().map(String::from).collect() };

    assert_eq!(
        list("vanilla_weapon"),
        set(vanilla::WEAPONS.iter().map(|(t, _)| *t).collect())
    );
    assert_eq!(
        list("spec"),
        set(vanilla::SPECS.iter().map(|(t, _)| *t).collect())
    );
    assert_eq!(
        list("pool"),
        set(vanilla::POOLS.iter().map(|(t, _)| *t).collect())
    );
    assert_eq!(list("variant"), set(vanilla::ALL_VARIANTS.to_vec()));
    let mut vehicles: Vec<&str> = vanilla::vehicles(Side::Allied).to_vec();
    vehicles.extend(vanilla::vehicles(Side::Axis));
    assert_eq!(list("vanilla_vehicle"), set(vehicles));

    // Every frame passes the schema's frame pattern, and nothing else does.
    let pattern = defs["frame"]["pattern"].as_str().unwrap();
    let re = regex_lite::Regex::new(pattern).unwrap();
    for f in vanilla::all_frames() {
        assert!(re.is_match(f), "{f} rejected by schema");
    }
    for bad in ["reload11", "flare8", "walk", "smoking3"] {
        assert!(!re.is_match(bad), "{bad} accepted by schema");
    }
}

#[test]
fn one_faction_per_side() {
    let candidates = vec![
        (50, "red_a".to_string(), Side::Allied, 1),
        (60, "red_b".to_string(), Side::Allied, 2),
        (50, "axis_a".to_string(), Side::Axis, 3),
        (60, "red_c".to_string(), Side::Allied, 4),
    ];
    let (active, skipped) = pick_active(&candidates);
    assert_eq!(
        active,
        vec![("red_c".to_string(), 4), ("axis_a".to_string(), 3)]
    );
    assert_eq!(skipped.len(), 2);
    assert!(skipped.iter().all(|s| s.contains("red_c")));
}
