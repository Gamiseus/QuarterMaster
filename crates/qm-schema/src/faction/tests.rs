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
    let mut presets: Vec<&str> = vanilla::EXPLOSION_PRESETS.iter().map(|(n, _)| *n).collect();
    presets.extend(vanilla::EXPLOSION_PRESETS.iter().map(|(_, t)| *t));
    assert_eq!(list("explosion_preset"), set(presets));
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

#[test]
fn grenades_and_explosions() {
    let dir = mod_with(&["weapons/limonka.png"]);
    let ok = format!(
        "{HEADER}\n[[explosions]]\nid = \"limonka_blast\"\nbased_on = \"frag\"\nfrag_radius = 150\nstun_time = 6\n\n\
         [[ordnance]]\nid = \"limonka\"\nsprite = \"weapons/limonka.png\"\nexplosion = \"limonka_blast\"\nrange = 900\nspin = 0\nimpact = true\n\
         skill = {{ class = \"light\", slot = 2 }}\nreplaces_frag = true\n\n\
         [[ordnance]]\nid = \"spare\"\nsprite = \"weapons/limonka.png\"\nexplosion = \"contact\"\n"
    );
    let r = check_faction(&ok, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let f = r.faction.unwrap();
    assert_eq!(f.explosions[0].id, "red_army:limonka_blast");
    assert_eq!(f.explosions[0].numbers.get("fragRadius"), Some(&150.0));
    assert_eq!(f.explosions[0].numbers.get("stunTime"), Some(&6.0));
    let g = &f.ordnance[0];
    assert_eq!((g.id.as_str(), g.explosion.as_str()), ("red_army:limonka", "red_army:limonka_blast"));
    assert_eq!((g.skill_class.as_deref(), g.skill_slot), (Some("light"), Some(2)));
    assert!(g.replaces_frag);
    assert_eq!(g.spin, Some(0.0));
    assert!(g.impact);
    assert!(!f.ordnance[1].impact);
    assert_eq!(f.ordnance[1].explosion, "contactExplosion", "a preset by name is its game tag");
    assert_eq!(f.ordnance[1].spin, None);

    // spin is a thrown grenade's: never negative, left out for other kinds.
    let spun = format!(
        "{HEADER}\n[[ordnance]]\nid = \"neg\"\nsprite = \"weapons/limonka.png\"\nspin = -1\n\n\
         [[ordnance]]\nid = \"rg\"\nkind = \"rifle\"\nsprite = \"weapons/limonka.png\"\nspin = 0\n"
    );
    let r = check_faction(&spun, dir.path());
    assert!(errors(&r).iter().any(|m| m.contains("spin can't be negative")), "{:?}", errors(&r));
    assert!(warnings(&r).iter().any(|m| m.contains("spin is for a thrown grenade")), "{:?}", warnings(&r));

    let bad = format!(
        "{HEADER}\n[[explosions]]\nid = \"Bad Id\"\n\n[[explosions]]\nid = \"x\"\nbased_on = \"nuke\"\n\n\
         [[explosions]]\nid = \"y\"\nfrag_damage = -1\n\n\
         [[ordnance]]\nid = \"a\"\nsprite = \"weapons/missing.png\"\n\n\
         [[ordnance]]\nid = \"b\"\nsprite = \"weapons/limonka.png\"\nexplosion = \"nowhere\"\n\n\
         [[ordnance]]\nid = \"c\"\nsprite = \"weapons/limonka.png\"\nskill = {{ class = \"wizard\", slot = 2 }}\n\n\
         [[ordnance]]\nid = \"d\"\nsprite = \"weapons/limonka.png\"\nskill = {{ class = \"light\", slot = 0 }}\n\n\
         [[ordnance]]\nid = \"e\"\nsprite = \"weapons/limonka.png\"\nreplaces_frag = true\n\n\
         [[ordnance]]\nid = \"f\"\nsprite = \"weapons/limonka.png\"\nreplaces_frag = true\n\n\
         [[ordnance]]\nid = \"e\"\nsprite = \"weapons/limonka.png\"\nrange = 0\n"
    );
    let e = errors(&check_faction(&bad, dir.path()));
    let has = |s: &str| e.iter().any(|m| m.contains(s));
    assert!(has("explosions[0] (Bad Id): id must use only"), "{e:?}");
    assert!(has("based_on \"nuke\" is not one of the game's explosions"), "{e:?}");
    assert!(has("frag_damage can't be negative"), "{e:?}");
    assert!(has("weapons/missing.png\" not found"), "{e:?}");
    assert!(has("explosion \"nowhere\" is not an explosion of this faction"), "{e:?}");
    assert!(has("\"wizard\" is not a class"), "{e:?}");
    assert!(has("slot must be 1 or more"), "{e:?}");

    // A fourth skill: its own icon and the level it comes with.
    let fourth = format!(
        "{HEADER}\n[[ordnance]]\nid = \"g4\"\nname = \"Limonka\"\nsprite = \"weapons/limonka.png\"\n\
         skill = {{ class = \"rifle\", slot = 4, unlock = 2, icon = \"weapons/limonka.png\" }}\n"
    );
    let r = check_faction(&fourth, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let g4 = &r.faction.unwrap().ordnance[0];
    assert_eq!((g4.skill_slot, g4.skill_unlock), (Some(4), Some(2)));
    assert_eq!(g4.skill_icon.as_deref(), Some("weapons/limonka.png"));
    assert_eq!(g4.name.as_deref(), Some("Limonka"));
    let bad4 = format!("{HEADER}\n[[ordnance]]\nid = \"g\"\nsprite = \"weapons/limonka.png\"\nskill = {{ class = \"rifle\", slot = 4, unlock = 0 }}\n");
    assert!(errors(&check_faction(&bad4, dir.path())).iter().any(|m| m.contains("unlock must be 1 or more")));
    // Any slot, any level: slot 7 at level 6, and the game's slot 2 at level 1 with its own icon.
    let any = format!(
        "{HEADER}\n[[ordnance]]\nid = \"g7\"\nsprite = \"weapons/limonka.png\"\nskill = {{ class = \"rifle\", slot = 7, unlock = 6 }}\n\n\
         [[ordnance]]\nid = \"g2\"\nsprite = \"weapons/limonka.png\"\nskill = {{ class = \"rifle\", slot = 2, unlock = 1, icon = \"weapons/limonka.png\" }}\n"
    );
    let r = check_faction(&any, dir.path());
    assert!(errors(&r).is_empty() && warnings(&r).is_empty(), "{:?} {:?}", errors(&r), warnings(&r));
    let o = r.faction.unwrap().ordnance;
    assert_eq!((o[0].skill_slot, o[0].skill_unlock), (Some(7), Some(6)));
    assert_eq!((o[1].skill_unlock, o[1].skill_icon.as_deref()), (Some(1), Some("weapons/limonka.png")));
    assert!(has("e already replaces the frag"), "{e:?}");
    assert!(has("another [[ordnance]] entry has this id"), "{e:?}");

    // Rifle grenades: the game's rifle grenade explosion unless set, and their own swap.
    let rifle = format!(
        "{HEADER}\n[[ordnance]]\nid = \"vog\"\nkind = \"rifle\"\nsprite = \"weapons/limonka.png\"\n\
         skill = {{ class = \"rifle\", slot = 2 }}\nreplaces_rifle_grenade = true\n"
    );
    let r = check_faction(&rifle, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let rg = &r.faction.unwrap().ordnance[0];
    assert_eq!((rg.kind.as_str(), rg.explosion.as_str()), ("rifle", "rifleGrenadeExplosion"));
    assert!(rg.replaces_rifle_grenade && !rg.replaces_frag);
    let thrown = format!("{HEADER}\n[[ordnance]]\nid = \"t\"\nsprite = \"weapons/limonka.png\"\n");
    assert_eq!(check_faction(&thrown, dir.path()).faction.unwrap().ordnance[0].kind, "thrown");
    let bad_rifle = format!(
        "{HEADER}\n[[ordnance]]\nid = \"k\"\nkind = \"catapult\"\nsprite = \"weapons/limonka.png\"\n\n\
         [[ordnance]]\nid = \"r1\"\nkind = \"rifle\"\nsprite = \"weapons/limonka.png\"\nreplaces_frag = true\n\n\
         [[ordnance]]\nid = \"t1\"\nsprite = \"weapons/limonka.png\"\nreplaces_rifle_grenade = true\n\n\
         [[ordnance]]\nid = \"r2\"\nkind = \"rifle\"\nsprite = \"weapons/limonka.png\"\nreplaces_rifle_grenade = true\n\n\
         [[ordnance]]\nid = \"r3\"\nkind = \"rifle\"\nsprite = \"weapons/limonka.png\"\nreplaces_rifle_grenade = true\n"
    );
    let e = errors(&check_faction(&bad_rifle, dir.path()));
    let has = |s: &str| e.iter().any(|m| m.contains(s));
    assert!(has("kind must be \"thrown\""), "{e:?}");
    assert!(has("replaces_frag is for kind = \"thrown\"; kind = \"rifle\""), "{e:?}");
    assert!(has("replaces_rifle_grenade is for kind = \"rifle\"; kind = \"thrown\""), "{e:?}");
    assert!(has("r2 already replaces the rifle grenade"), "{e:?}");

    // Mortars and artillery: no sprite needed, the game's own explosion unless set, their
    // rounds and spread, and their own swaps.
    let heavy = format!(
        "{HEADER}\n[[ordnance]]\nid = \"hvy\"\nkind = \"mortar\"\nshots = 4\nspread = 60\nreplaces_mortar = true\n\n\
         [[ordnance]]\nid = \"katyusha\"\nkind = \"artillery\"\nshots = 12\nreplaces_artillery = true\n\n\
         [[ordnance]]\nid = \"hvy2\"\nkind = \"mortar\"\nexplosion = \"heavy_mortar\"\n"
    );
    let r = check_faction(&heavy, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let gs = r.faction.unwrap().ordnance;
    assert_eq!((gs[0].kind.as_str(), gs[0].explosion.as_str()), ("mortar", "mortarExplosion"));
    assert_eq!((gs[0].shots, gs[0].spread, gs[0].sprite.as_deref()), (Some(4), Some(60.0), None));
    assert!(gs[0].replaces_mortar && gs[1].replaces_artillery);
    assert_eq!((gs[1].kind.as_str(), gs[1].explosion.as_str()), ("artillery", "artyExplosion"));
    assert_eq!(gs[2].explosion, "hvyExplosion", "the heavy mortar preset");
    let bad_heavy = format!(
        "{HEADER}\n[[ordnance]]\nid = \"t\"\n\n\
         [[ordnance]]\nid = \"m1\"\nkind = \"mortar\"\nshots = 0\nspread = -5\nreplaces_artillery = true\n\n\
         [[ordnance]]\nid = \"m2\"\nkind = \"mortar\"\nreplaces_mortar = true\n\n\
         [[ordnance]]\nid = \"m3\"\nkind = \"mortar\"\nreplaces_mortar = true\n"
    );
    let e = errors(&check_faction(&bad_heavy, dir.path()));
    let has = |s: &str| e.iter().any(|m| m.contains(s));
    assert!(has("a thrown grenade needs a sprite"), "{e:?}");
    assert!(has("shots must be 1 or more"), "{e:?}");
    assert!(has("spread can't be negative"), "{e:?}");
    assert!(has("replaces_artillery is for kind = \"artillery\""), "{e:?}");
    assert!(has("m2 already replaces the mortar"), "{e:?}");
    let odd = format!("{HEADER}\n[[ordnance]]\nid = \"t\"\nsprite = \"weapons/limonka.png\"\nshots = 3\n");
    assert!(warnings(&check_faction(&odd, dir.path())).iter().any(|m| m.contains("shots and spread are for a mortar")));

    // Air strikes and drops: their own plane, a drop's contents and no explosion, the bombing
    // and strafing swaps apart.
    let air = format!(
        "{HEADER}\n[[ordnance]]\nid = \"il2\"\nkind = \"airstrike\"\nplane = \"weapons/limonka.png\"\n\
         replaces_bombing = true\n\n\
         [[ordnance]]\nid = \"yak\"\nkind = \"airstrike\"\nexplosion = \"strafe\"\nreplaces_strafing = true\n\n\
         [[ordnance]]\nid = \"lend_lease\"\nkind = \"drop\"\ncontents = [\"loot_ration\", \"loot_medPack\"]\n\
         shots = 4\nreplaces_drop = true\n"
    );
    let r = check_faction(&air, dir.path());
    assert!(errors(&r).is_empty() && warnings(&r).is_empty(), "{:?} {:?}", errors(&r), warnings(&r));
    let gs = r.faction.unwrap().ordnance;
    assert_eq!((gs[0].explosion.as_str(), gs[0].plane.as_deref()), ("bombExplosion", Some("weapons/limonka.png")));
    assert!(gs[0].replaces_bombing && !gs[0].replaces_strafing);
    assert_eq!(gs[1].explosion, "flakExplosion", "the strafe preset");
    assert!(gs[1].replaces_strafing);
    assert_eq!((gs[2].explosion.as_str(), gs[2].contents.len(), gs[2].shots), ("", 2, Some(4)));
    assert!(gs[2].replaces_drop);
    let bad_air = format!(
        "{HEADER}\n[[ordnance]]\nid = \"d\"\nkind = \"drop\"\nexplosion = \"bomb\"\ncontents = [\"medkit\"]\nspread = 5\n\n\
         [[ordnance]]\nid = \"t\"\nsprite = \"weapons/limonka.png\"\nplane = \"weapons/limonka.png\"\ncontents = [\"loot_ration\"]\n\n\
         [[ordnance]]\nid = \"m\"\nkind = \"mortar\"\nreplaces_drop = true\n\n\
         [[ordnance]]\nid = \"a1\"\nkind = \"airstrike\"\nreplaces_bombing = true\n\n\
         [[ordnance]]\nid = \"a2\"\nkind = \"airstrike\"\nreplaces_bombing = true\n"
    );
    let r = check_faction(&bad_air, dir.path());
    let (e, w) = (errors(&r), warnings(&r));
    let has = |l: &[String], s: &str| l.iter().any(|m| m.contains(s));
    assert!(has(&w, "a drop has no explosion"), "{w:?}");
    assert!(has(&w, "\"medkit\" is not one of the game's loot names"), "{w:?}");
    assert!(has(&w, "a drop lands on the spot"), "{w:?}");
    assert!(has(&w, "plane is for an air strike or a drop"), "{w:?}");
    assert!(has(&w, "contents are for a drop"), "{w:?}");
    assert!(has(&e, "replaces_drop is for kind = \"drop\"; kind = \"mortar\" has replaces_mortar"), "{e:?}");
    assert!(has(&e, "a1 already replaces the bombing runs"), "{e:?}");

    // On the Axis side a class skill is never used: a warning.
    let axis = "schema = 1\nid = \"axis_army\"\nname = \"Axis Army\"\nreplaces = \"axis\"\n\n\
         [[ordnance]]\nid = \"g\"\nsprite = \"weapons/limonka.png\"\nskill = { class = \"light\", slot = 2 }\n";
    let r = check_faction(axis, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    assert!(warnings(&r).iter().any(|w| w.contains("never used")), "{:?}", warnings(&r));
}

#[test]
fn skills_change_replace_move_remove_and_add() {
    let dir = mod_with(&["weapons/limonka.png", "icons/flare.png"]);
    let ok = format!(
        "{HEADER}\n[[ordnance]]\nid = \"limonka\"\nsprite = \"weapons/limonka.png\"\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 1\nname = \"Flare\"\ndescription = \"light up the night\"\nicon = \"icons/flare.png\"\ncooldown = 30\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 3\ngame = \"light:3\"\n\n\
         [[skills]]\nclass = \"light\"\nslot = 2\nremove = true\n\n\
         [[skills]]\nclass = \"light\"\nslot = 3\nunlock = 1\n\n\
         [[skills]]\nclass = \"medic\"\nslot = 4\ngame = 1\nunlock = 4\n\n\
         [[skills]]\nclass = \"medic\"\nslot = 5\nordnance = \"limonka\"\nunlock = 5\n"
    );
    let r = check_faction(&ok, dir.path());
    assert!(errors(&r).is_empty() && warnings(&r).is_empty(), "{:?} {:?}", errors(&r), warnings(&r));
    let s = r.faction.unwrap().skills;
    assert_eq!(s.len(), 6);
    assert_eq!((s[0].name.as_deref(), s[0].icon.as_deref(), s[0].cooldown), (Some("Flare"), Some("icons/flare.png"), Some(30.0)));
    assert_eq!((s[1].game_class.as_deref(), s[1].game_slot), (Some("light"), Some(3)));
    assert!(s[2].remove);
    assert_eq!(s[3].unlock, Some(1));
    assert_eq!((s[4].game_class.as_deref(), s[4].game_slot, s[4].unlock), (Some("medic"), Some(1), Some(4)));
    assert_eq!(s[5].ordnance.as_deref(), Some("red_army:limonka"));

    let bad = format!(
        "{HEADER}\n[[ordnance]]\nid = \"g\"\nsprite = \"weapons/limonka.png\"\nskill = {{ class = \"rifle\", slot = 2 }}\n\n\
         [[skills]]\nclass = \"wizard\"\nslot = 1\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 0\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 2\nname = \"x\"\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 4\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 5\ngame = 1\nremove = true\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 6\ngame = \"light:9\"\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 7\nordnance = \"nowhere\"\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 8\ngame = 1\nunlock = 0\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 9\ngame = \"comms:1\"\n\n\
         [[skills]]\nclass = \"rifle\"\nslot = 1\nremove = true\ncooldown = 5\n"
    );
    let r = check_faction(&bad, dir.path());
    let (e, w) = (errors(&r), warnings(&r));
    let has = |l: &[String], s: &str| l.iter().any(|m| m.contains(s));
    assert!(has(&e, "\"wizard\" is not a class"), "{e:?}");
    assert!(has(&e, "slot must be 1 or more"), "{e:?}");
    assert!(has(&e, "already takes rifle skill 2"), "{e:?}");
    assert!(has(&e, "skill 4 is a new skill; it needs game or ordnance"), "{e:?}");
    assert!(has(&e, "game, ordnance and remove are one or the other"), "{e:?}");
    assert!(has(&e, "a game skill is 1, 2 or 3"), "{e:?}");
    assert!(has(&e, "\"nowhere\" is not an [[ordnance]] entry"), "{e:?}");
    assert!(has(&e, "unlock must be 1 or more"), "{e:?}");
    assert!(has(&w, "the radio operator's calls go through only for radio operators"), "{w:?}");
    assert!(has(&w, "a removed skill shows nothing"), "{w:?}");
}


#[test]
fn rank_names() {
    let dir = mod_with(&[]);
    let ok = format!(
        "{HEADER}\n[ranks]\nsoldier = [\"Gvardeyets\", \"Gv. Yefreitor\", \"Gv. Ml. Serzhant\", \"Gv. Serzhant\"]\n\
         officer = [\"Ml. Leytenant\"]\nrecruit = \"Prizyvnik\"\n"
    );
    let r = check_faction(&ok, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let ranks = r.faction.unwrap().ranks.unwrap();
    assert_eq!(ranks.soldier.len(), 4);
    assert_eq!(ranks.officer, vec!["Ml. Leytenant".to_string()]);
    assert_eq!(ranks.recruit.as_deref(), Some("Prizyvnik"));
    assert_eq!(ranks.cadet, None);
    let none = check_faction(HEADER, dir.path());
    assert!(none.faction.unwrap().ranks.is_none());

    let bad = format!("{HEADER}\n[ranks]\nsoldier = [\"a\", \"b\", \"c\", \"d\", \"e\"]\nofficer = [\" \"]\ncadet = \"\"\n");
    let e = errors(&check_faction(&bad, dir.path()));
    let has = |s: &str| e.iter().any(|m| m.contains(s));
    assert!(has("ranks.soldier: the game has four ranks"), "{e:?}");
    assert!(has("ranks.officer: a rank name can't be empty"), "{e:?}");
    assert!(has("ranks.cadet: a rank name can't be empty"), "{e:?}");
    let unknown = format!("{HEADER}\n[ranks]\nprivate = \"x\"\n");
    assert!(!errors(&check_faction(&unknown, dir.path())).is_empty(), "an unknown key is an error");
}

#[test]
fn sprite_sizes() {
    let dir = mod_with(&["misc/rank_1.png", "misc/flag.png"]);
    let ok = format!(
        "{HEADER}\n[sprites]\nflag = \"misc/flag.png\"\nrank_1 = {{ file = \"misc/rank_1.png\", size = [18, 22] }}\n\
         spec_rifle = {{ scale = 0.5 }}\nrank_2 = {{ like = \"rank_1\" }}\nrank_3 = {{ file = \"misc/rank_1.png\" }}\n"
    );
    let r = check_faction(&ok, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let f = r.faction.unwrap();
    assert_eq!(f.sprites.get("flag").map(String::as_str), Some("misc/flag.png"));
    assert_eq!(f.sprites.get("rank_1").map(String::as_str), Some("misc/rank_1.png"));
    assert_eq!(f.sprites.get("rank_3").map(String::as_str), Some("misc/rank_1.png"), "a table with only a file");
    assert!(!f.sprites.contains_key("spec_rifle"), "a size alone replaces no file");
    assert_eq!(f.sprite_sizes.get("rank_1").and_then(|s| s.size), Some([18.0, 22.0]));
    assert_eq!(f.sprite_sizes.get("spec_rifle").and_then(|s| s.scale), Some(0.5));
    assert_eq!(f.sprite_sizes.get("rank_2").and_then(|s| s.like.clone()).as_deref(), Some("rank_1"));
    assert!(!f.sprite_sizes.contains_key("flag") && !f.sprite_sizes.contains_key("rank_3"));

    let bad = format!(
        "{HEADER}\n[sprites]\na = {{ size = [18, 22], scale = 2.0 }}\nb = {{}}\nc = {{ size = [0, 22] }}\n\
         d = {{ like = \"d\" }}\ne = {{ file = \"misc/missing.png\", size = [1, 1] }}\n"
    );
    let e = errors(&check_faction(&bad, dir.path()));
    let has = |s: &str| e.iter().any(|m| m.contains(s));
    assert!(has("sprites.a: give one of size, scale and like"), "{e:?}");
    assert!(has("sprites.b: needs a file, a size or a place"), "{e:?}");
    assert!(has("sprites.c: sizes and scales must be more than 0"), "{e:?}");
    assert!(has("sprites.d: like must name another image"), "{e:?}");
    assert!(has("misc/missing.png"), "{e:?}");
}

#[test]
fn sprite_places() {
    let dir = mod_with(&["misc/rank_1.png"]);
    let ok = format!(
        "{HEADER}\n[sprites]\nrank_1 = {{ file = \"misc/rank_1.png\", offset = [3, -2], rotate = 90, flip = \"x\" }}\n\
         spec_rifle = {{ scale = 0.5, rotate = -12.5 }}\nflag = {{ flip = \"xy\" }}\nrank_2 = {{ draw_size = [20, 24] }}\n"
    );
    let r = check_faction(&ok, dir.path());
    assert!(errors(&r).is_empty(), "{:?}", errors(&r));
    let f = r.faction.unwrap();
    let one = f.sprite_sizes.get("rank_1").expect("a file with a place has a rule");
    assert_eq!((one.offset, one.rotate, one.flip.as_deref(), one.size), (Some([3.0, -2.0]), Some(90.0), Some("x"), None));
    assert_eq!(f.sprites.get("rank_1").map(String::as_str), Some("misc/rank_1.png"));
    let rifle = f.sprite_sizes.get("spec_rifle").unwrap();
    assert_eq!((rifle.scale, rifle.rotate), (Some(0.5), Some(-12.5)));
    assert_eq!(f.sprite_sizes.get("flag").and_then(|s| s.flip.clone()).as_deref(), Some("xy"), "a place alone");
    assert!(!f.sprites.contains_key("flag"));
    assert_eq!(f.sprite_sizes.get("rank_2").and_then(|s| s.draw_size), Some([20.0, 24.0]), "a draw size alone");

    let bad = format!("{HEADER}\n[sprites]\na = {{ flip = \"z\" }}\n");
    let e = errors(&check_faction(&bad, dir.path()));
    assert!(e.iter().any(|m| m.contains("sprites.a: flip is \"x\", \"y\" or \"xy\", not \"z\"")), "{e:?}");
    for shape in ["offset = [1]", "rotate = \"left\"", "draw_size = [0, 4]"] {
        let bad = format!("{HEADER}\n[sprites]\nb = {{ {shape} }}\n");
        assert!(!errors(&check_faction(&bad, dir.path())).is_empty(), "{shape} is refused");
    }
}

#[test]
fn sarge_title() {
    let dir = mod_with(&[]);
    let ok = format!("{HEADER}\n[ranks]\nsarge = \"Starshina\"\n");
    let r = check_faction(&ok, dir.path());
    assert!(errors(&r).is_empty() && warnings(&r).is_empty(), "{:?} {:?}", errors(&r), warnings(&r));
    assert_eq!(r.faction.unwrap().ranks.unwrap().sarge.as_deref(), Some("Starshina"));
    let empty = format!("{HEADER}\n[ranks]\nsarge = \"\"\n");
    assert!(errors(&check_faction(&empty, dir.path())).iter().any(|m| m.contains("ranks.sarge: a rank name can't be empty")));
    let axis = "schema = 1\nid = \"heer\"\nname = \"Heer\"\nreplaces = \"axis\"\n\n[ranks]\nsarge = \"Feldwebel\"\n";
    let r = check_faction(axis, dir.path());
    assert!(warnings(&r).iter().any(|m| m.contains("the Sarge is the Allies'")), "{:?}", warnings(&r));
    assert_eq!(r.faction.unwrap().ranks.unwrap().sarge, None);
}
