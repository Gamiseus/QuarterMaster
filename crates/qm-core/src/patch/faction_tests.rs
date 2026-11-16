use super::*;
use crate::patch::loader::{load_patches_new, STEAM_SAFETY_PAYLOAD};
use std::process::Command;
use tempfile::TempDir;

/// Writes files whose content is their own relative path, so the Lua harness can tell which
/// file ended up where.
fn put(root: &Path, files: &[&str]) {
    for f in files {
        let p = root.join(f);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, f.as_bytes()).unwrap();
    }
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

const RED_ARMY: &str = r#"#:schema ../schemas/faction.schema.json
schema = 1
id = "red_army"
name = "Red Army"
replaces = "allied"
side_name = "Soviet"

[units.base]
folder = "units/base"

[units.sarge]
copy_from = "base"

[[weapons]]
reskin = "m1"
description = "Mosin-Nagant \"M91/30\".\nBolt action."
top_sprite = "weapons/mosin_top.png"
sound = "weapons/mosin"
stats = { action = "bolt", ammo = 5, maxAmmo = 5, damage = 110 }

[[weapons]]
tag = "pps43"
based_on = "m3"
name = "PPS-43"
sound = "weapons/pps43"
mag_sprite = "weapons/pps43_mag.png"
stats = { maxAmmo = 35 }
loot_side = "allied"

[[weapons]]
reskin = "svt40"
sound = "springfieldSound"
mag_sprite = "ppshMag"

[spawn_swap]
tommy = "ppsh41"

[pools.grunt]
remove = ["springfield"]
add = ["pps43"]

[classes.rifle]
name = "Rifleman"
description = "Strelok."
start_weapon = "m1"
unlocks = ["svt40", "ppsh41", "pps43"]

[classes.gunner]
name = "Machine Gunner"
start_weapon = "pps43"
bonus = { speed = -5, detect_range = 2.0, max_health = 10 }

[names]
last = ["Ivanov", "Petrov"]
vehicles = ["Za Rodinu"]

[voices]
folder = "voices"
map = { hurt = "alliedHurt" }

[text]
mouseOverLightSpec = "Scout, fast."

[[vehicles]]
reskin = "sherman"
name = "T-34/76"
body = "vehicles/t34/body.png"
debris = "vehicles/t34/debris"
call_signs = ["Za Stalina"]

[hooks]
lua = "hooks.lua"
"#;

const HOOKS: &str = r#"
local H = {}
function H.on_enable(info) _G.TEST_HOOK_ENABLED = info.id end
function H.on_weapon_created(weapon, tag) _G.TEST_WEAPONS_CREATED = (_G.TEST_WEAPONS_CREATED or 0) + 1 end
return H
"#;

fn red_army_mod(mods: &Path) -> PathBuf {
    let root = mods.join("RedArmy");
    put(
        &root,
        &[
            "units/base/idle.png",
            "units/base/aim.png",
            "weapons/mosin_top.png",
            "weapons/mosin1.wav",
            "weapons/mosin2.wav",
            "weapons/pps43.wav",
            "weapons/pps43_mag.png",
            "voices/contact1.wav",
            "voices/contact2.wav",
            "voices/hurt1.wav",
            "vehicles/t34/body.png",
            "vehicles/t34/debris1.png",
            "vehicles/t34/debris2.png",
        ],
    );
    write(&root, "faction.toml", RED_ARMY);
    write(&root, "hooks.lua", HOOKS);
    root
}

fn loaded(root: &Path) -> LoadedFaction {
    load_mod_faction("redarmy", 50, root).expect("faction loads")
}

#[test]
fn lua_literals_escape_and_keep_utf8() {
    let v = serde_json::json!({
        "s": "a \"q\" \\ b\nc\r\t\u{1}",
        "u": "Ivanov · Petrov",
        "n": 3,
        "f": 1.5,
        "list": [1, "x", true, null],
        "nested": { "k": false },
        "skip": null
    });
    let mut out = String::new();
    to_lua(&v, &mut out);
    assert!(out.contains(r#"["s"]="a \"q\" \\ b\nc\r\t\001""#), "{out}");
    assert!(out.contains("Ivanov · Petrov"));
    assert!(out.contains(r#"["n"]=3,"#) && out.contains(r#"["f"]=1.5,"#));
    assert!(out.contains(r#"{1,"x",true,nil,}"#));
    assert!(!out.contains("skip"));
    assert!(!out.contains('\n'));
}

#[test]
fn no_factions_means_no_patches() {
    assert!(build_faction_patches(&[], None).is_empty());
}

#[test]
fn patches_for_an_active_faction() {
    let dir = TempDir::new().unwrap();
    let lf = loaded(&red_army_mod(dir.path()));
    let patches = build_faction_patches(&[lf], None);
    let ids: Vec<&str> = patches.iter().filter_map(|p| p.id()).collect();
    assert_eq!(
        ids,
        vec![
            "qm_factions_expected",
            "qm_factions",
            "qm_factions_vehicle_label",
            "qm_factions_magazines",
            "qm_factions_tag_sound_player",
            "qm_factions_tag_sound_ai"
        ]
    );
    let Patch::Pattern(expected) = &patches[0] else {
        panic!()
    };
    assert!(expected.payload.contains(r#"["allied"]="red_army""#));
    assert!(expected.payload.contains(r#"["pps43"]=true"#));
    let Patch::Pattern(main) = &patches[1] else {
        panic!()
    };
    assert!(main
        .payload
        .contains("__qm_factions_apply((function() return {"));
    assert!(main
        .payload
        .contains(FACTION_RUNTIME.lines().nth(1).unwrap()));
    assert!(!main.match_indent, "payload must not be re-indented");
}

#[test]
fn loader_picks_up_faction_only_mods() {
    let dir = TempDir::new().unwrap();
    let mods = dir.path();
    red_army_mod(mods);

    // A second Allied faction with higher priority wins; a broken one never loads.
    let other = mods.join("Poles");
    write(
        &other,
        "faction.toml",
        "schema = 1\nid = \"poles\"\nname = \"Polish Army\"\nreplaces = \"allied\"\n",
    );
    write(
        &other,
        "mod.json",
        r#"{"id":"poles","name":"Polish Army","version":"1.0.0","load_priority":90}"#,
    );
    let broken = mods.join("Broken");
    write(&broken, "faction.toml", "schema = 1\nid = \"x\"\nname = \"X\"\nreplaces = \"axis\"\n[[weapons]]\nreskin = \"nope\"\n");

    let patches = load_patches_new(mods).unwrap();
    let main = patches
        .iter()
        .find_map(|(p, ..)| match p {
            Patch::Pattern(pp) if pp.id.as_deref() == Some("qm_factions") => Some(pp),
            _ => None,
        })
        .expect("faction patch");
    assert!(
        main.payload.contains(r#"["id"]="poles""#),
        "higher priority faction wins"
    );
    assert!(!main.payload.contains(r#"["id"]="red_army""#));
    assert!(!main.payload.contains(r#"["id"]="x""#));
}

#[test]
fn zip_faction_mods_are_unpacked() {
    let dir = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    let root = red_army_mod(src.path());
    let zip_path = dir.path().join("RedArmy.zip");
    {
        let file = fs::File::create(&zip_path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts: zip::write::SimpleFileOptions = Default::default();
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.path().is_file() {
                let rel = entry
                    .path()
                    .strip_prefix(src.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                zw.start_file(rel, opts).unwrap();
                std::io::Write::write_all(&mut zw, &fs::read(entry.path()).unwrap()).unwrap();
            }
        }
        zw.finish().unwrap();
    }
    let out = extract_zip_faction(&zip_path).expect("unpacked");
    assert!(out.join("faction.toml").is_file());
    assert!(out.join("units/base/idle.png").is_file());
    let patches = load_patches_new(dir.path()).unwrap();
    assert!(patches.iter().any(|(p, ..)| p.id() == Some("qm_factions")));
}

// ---------------------------------------------------------------------------------------------
// The runtime, run by a real Lua interpreter against vanilla functions from the decompiled game
// ---------------------------------------------------------------------------------------------

fn lua_exe() -> Option<String> {
    for exe in ["luajit", "lua5.1", "lua"] {
        if Command::new(exe).arg("-v").output().is_ok() {
            return Some(exe.to_string());
        }
    }
    None
}

fn decompiled_engine() -> Option<PathBuf> {
    let p =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../decompile/includes/urb_engine.lua");
    p.is_file().then_some(p)
}

fn run_harness(scenario: &str) -> Option<String> {
    run_harness_with(scenario, None)
}

fn plan_of(extra: &str) -> crate::handshake::LoadOrderPlan {
    serde_json::from_str(&format!(
        r#"{{"profile_id": "p", "generated_at": "t", "enabled_mods": []{extra}}}"#
    ))
    .unwrap()
}

fn run_harness_with(scenario: &str, plan: Option<&crate::handshake::LoadOrderPlan>) -> Option<String> {
    let (Some(lua), Some(engine)) = (lua_exe(), decompiled_engine()) else {
        eprintln!(
            "skipping Lua runtime test: needs a Lua interpreter and the decompiled game sources"
        );
        return None;
    };
    let dir = TempDir::new().unwrap();
    let lf = loaded(&red_army_mod(dir.path()));
    let patches = build_faction_patches(&[lf], plan);
    let payload_of = |id: &str| -> String {
        patches
            .iter()
            .find_map(|p| match p {
                Patch::Pattern(pp) if pp.id.as_deref() == Some(id) => Some(pp.payload.clone()),
                _ => None,
            })
            .unwrap()
    };
    let work = dir.path().join("lua");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join("payload.lua"), STEAM_SAFETY_PAYLOAD).unwrap();
    fs::write(
        work.join("expected.lua"),
        payload_of("qm_factions_expected"),
    )
    .unwrap();
    fs::write(work.join("factions.lua"), payload_of("qm_factions")).unwrap();
    // Not under tests/: that folder is git-ignored for the stress test.
    let harness = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/faction_harness.lua");
    let out = Command::new(lua)
        .arg(&harness)
        .arg(scenario)
        .arg(&work)
        .arg(&engine)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "harness {scenario} failed:\n{text}");
    assert!(text.contains("HARNESS OK"), "{text}");
    Some(text)
}

#[test]
fn runtime_applies_a_faction() {
    if let Some(out) = run_harness("apply") {
        assert!(!out.contains("ERROR"), "{out}");
    }
}

/// Phase E D83: a pool another mod changed keeps that mod's weapons under the faction.
#[test]
fn runtime_keeps_another_mods_pool_edits() {
    if let Some(out) = run_harness("pools") {
        assert!(!out.contains("FAIL"), "{out}");
    }
}

/// Phase E D78: what a choice gave to another source, the faction leaves alone.
#[test]
fn runtime_leaves_excluded_files_alone() {
    let plan = plan_of(
        r#", "asset_exclusions": {"faction:red_army": ["allied_idle", "top_m1", "shermanBody",
            "sherman_debris2", "sounds/vo/allied/contact1"], "faction:other": ["allied_aim"]}"#,
    );
    if let Some(out) = run_harness_with("excluded", Some(&plan)) {
        assert!(!out.contains("ERROR"), "{out}");
    }
}

#[test]
fn excluded_names_go_into_the_factions_data() {
    let dir = TempDir::new().unwrap();
    let lf = loaded(&red_army_mod(dir.path()));
    let none = faction_data_lua(std::slice::from_ref(&lf), None);
    assert!(!none.contains("excluded"), "no plan, no list");
    let other = plan_of(r#", "asset_exclusions": {"faction:other": ["allied_idle"], "redarmy": ["x"]}"#);
    assert_eq!(faction_data_lua(std::slice::from_ref(&lf), Some(&other)), none, "another source's list is not this faction's");
    let plan = plan_of(r#", "asset_exclusions": {"faction:red_army": ["allied_idle", "sounds/vo/allied/contact1"]}"#);
    let data = faction_data_lua(std::slice::from_ref(&lf), Some(&plan));
    assert!(
        data.contains(r#"["excluded"]={["allied_idle"]=true,["sounds/vo/allied/contact1"]=true,}"#),
        "{data}"
    );
}

fn candidate(dir: &Path, folder: &str, id: &str, side: &str, priority: i32) -> LoadedFaction {
    let root = dir.join(folder);
    write(
        &root,
        "faction.toml",
        &format!("schema = 1\nid = \"{id}\"\nname = \"{id}\"\nreplaces = \"{side}\"\n"),
    );
    load_mod_faction(&folder.to_lowercase(), priority, &root).expect("faction loads")
}

/// Phase E D82: a picked faction wins its side whatever the priorities are.
#[test]
fn a_picked_faction_beats_the_priority() {
    let dir = TempDir::new().unwrap();
    let all = || {
        vec![
            candidate(dir.path(), "Red", "red_army", "allied", 10),
            candidate(dir.path(), "Polish", "polish_army", "allied", 90),
            candidate(dir.path(), "Volks", "volkssturm", "axis", 50),
        ]
    };
    let ids = |p: &FactionPick| -> Vec<String> {
        let mut v: Vec<String> = p.active.iter().map(|lf| lf.faction.id.clone()).collect();
        v.sort();
        v
    };

    // No pick: the highest priority wins, as before.
    let p = pick_active_for(all(), None);
    assert_eq!(ids(&p), ["polish_army", "volkssturm"]);
    assert!(p.notes.is_empty());
    assert_eq!(p.skipped.len(), 1);
    let (quiet, skipped) = pick_active_quiet(all());
    assert_eq!(quiet.len(), 2);
    assert_eq!(skipped, p.skipped);

    // The pick wins against a higher priority; the other side is untouched.
    let plan = plan_of(r#", "factions": {"allied": "red_army"}"#);
    let p = pick_active_for(all(), Some(&plan));
    assert_eq!(ids(&p), ["red_army", "volkssturm"]);
    assert!(p.notes.is_empty());
    assert_eq!(p.skipped.len(), 1);
    assert!(p.skipped[0].contains("'polish' also replaces the Allies; 'red' is picked"), "{:?}", p.skipped);

    // A pick that names no loaded faction of that side: the priority decides, with a note.
    let plan = plan_of(r#", "factions": {"allied": "gone", "axis": "red_army"}"#);
    let p = pick_active_for(all(), Some(&plan));
    assert_eq!(ids(&p), ["polish_army", "volkssturm"]);
    assert_eq!(p.notes.len(), 2, "{:?}", p.notes);
    assert!(p.notes[0].contains("picked for the Allies ('gone')"), "{:?}", p.notes);
    // A pick for a side no faction replaces says nothing.
    let plan = plan_of(r#", "factions": {"axis": "gone"}"#);
    let only_allied = vec![candidate(dir.path(), "Red", "red_army", "allied", 10)];
    let p = pick_active_for(only_allied, Some(&plan));
    assert!(p.notes.is_empty() && p.skipped.is_empty());
    assert_eq!(ids(&p), ["red_army"]);
}

#[test]
fn runtime_cleans_up_a_save_after_the_faction_is_removed() {
    run_harness("cleanup");
}

/// Every faction patch lands exactly once in the real game files, and the patched `main.lua`
/// still compiles.
#[test]
fn patches_match_the_vanilla_sources_once() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../decompile");
    if !base.join("main.lua").is_file() {
        eprintln!("skipping: decompiled game sources not found");
        return;
    }
    let dir = TempDir::new().unwrap();
    let lf = loaded(&red_army_mod(dir.path()));
    let mut patched_main = None;
    for patch in build_faction_patches(&[lf], None) {
        let (target, entry, text) = match &patch {
            Patch::Pattern(p) => {
                let target = p.target.primary().to_string();
                let src = fs::read_to_string(base.join(&target)).unwrap();
                let mut rope = crop::Rope::from(src.as_str());
                let entry = p.apply(&target, &mut rope, Path::new("test")).unwrap();
                (target, entry, rope.to_string())
            }
            Patch::Regex(p) => {
                let target = p.target.primary().to_string();
                let src = fs::read_to_string(base.join(&target)).unwrap();
                let mut rope = crop::Rope::from(src.as_str());
                let entry = p.apply(&target, &mut rope, Path::new("test")).unwrap();
                (target, entry, rope.to_string())
            }
            _ => unreachable!(),
        };
        assert_eq!(
            entry.matched_count,
            1,
            "{:?} on {target}: {:?}",
            patch.id(),
            entry.warnings
        );
        if target == "main.lua" {
            // Apply both main.lua patches to the same text for the compile check.
            let current: String = patched_main
                .take()
                .unwrap_or_else(|| fs::read_to_string(base.join("main.lua")).unwrap());
            let mut rope = crop::Rope::from(current.as_str());
            if let Patch::Pattern(p) = &patch {
                p.apply("main.lua", &mut rope, Path::new("test"));
            }
            patched_main = Some(rope.to_string());
        } else if patch.id() == Some("qm_factions_vehicle_label") {
            assert!(text.contains("__qm_vehicle_label(player.deployedVehicles[id])"));
        } else if patch.id() == Some("qm_factions_magazines") {
            // The patched addParticle drops a faction's magazine and still drops the game's.
            let Some(lua) = lua_exe() else { continue };
            let file = dir.path().join("particlesSystem_patched.lua");
            fs::write(&file, &text).unwrap();
            let script = format!(
                r#"local img = {{ getWidth = function() return 40 end, getHeight = function() return 20 end }}
genericMag, ppshMag, ownMag = img, img, img
randNbr = function(n) return 1 end
player = {{ option_maxParticles = 1 }}
assert((loadstring or load)(io.open([[{}]], 'rb'):read('*a'), '@particlesSystem.lua'))()
local function drop(typez) local p = {{}} addParticle(p, typez, 1, 2, 0) return p end
assert(drop('ppshMag').sprite == 'ppshMag')
assert(drop('ownMag').lifeSpan == nil, 'an unknown magazine is left empty by the game')
__qm_mag_sprites = {{ ownMag = true, goneMag = true }}
local p = drop('ownMag')
assert(p.sprite == 'ownMag' and p.typez == 'ownMag' and p.lifeSpan and p.velocity)
assert(p.xoffset == 20 and p.yoffset == 10)
p = drop('goneMag')
assert(p.sprite == 'genericMag' and p.lifeSpan, 'a missing image falls back to the generic magazine')
assert(drop('ppshMag').sprite == 'ppshMag')
print('MAGAZINES OK')"#,
                file.display()
            );
            let out = Command::new(lua).arg("-e").arg(script).output().unwrap();
            let seen = String::from_utf8_lossy(&out.stdout).to_string()
                + &String::from_utf8_lossy(&out.stderr);
            assert!(seen.contains("MAGAZINES OK"), "{seen}");
        } else {
            // The tag-sound check goes in front of vanilla's Garand check, same indent.
            assert!(text.contains("__qm_sound_by_tag[player.deployedTroops[idz].weapon.tag]) then"));
            assert!(text.contains("elseif(player.deployedTroops[idz].weapon.tag == \"m1\""));
            let compile = std::process::Command::new(lua_exe().unwrap_or_else(|| "lua".into()))
                .arg("-e")
                .arg(format!("assert((loadstring or load)(io.open([[{}]], 'rb'):read('*a')))", {
                    let p = dir.path().join(target.replace('/', "_"));
                    fs::write(&p, &text).unwrap();
                    p.display().to_string()
                }))
                .output();
            if let Ok(out) = compile {
                assert!(out.status.success(), "{target} no longer compiles: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
    }
    let Some(lua) = lua_exe() else { return };
    let main = patched_main.unwrap();
    let out_path = dir.path().join("main_patched.lua");
    fs::write(&out_path, &main).unwrap();
    let script = format!(
        "local f = assert(io.open([[{}]], 'rb')); local src = f:read('*a'); f:close(); assert((loadstring or load)(src, '@main.lua')); print('COMPILES')",
        out_path.display()
    );
    let out = Command::new(lua).arg("-e").arg(script).output().unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("COMPILES"), "{text}");
}
