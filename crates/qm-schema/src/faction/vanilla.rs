//! What Mud and Blood v0.5.3b has that a faction can take over. Read from the decompiled game
//! sources; regenerate after a game update. `schemas/faction.schema.json` repeats these lists for
//! editors, and a test keeps the two in step.

use super::Side;

/// Frames every soldier variant has.
const COMMON_FRAMES: &[&str] = &[
    "idle",
    "aim",
    "fire",
    "parry",
    "pinned",
    "pinCover",
    "call",
    "observe",
    "dead",
    "proneAim",
    "proneFire",
    "unjam1",
    "unjam2",
    "attack1",
    "attack2",
    "attack3",
    "attack4",
    "fragThrow1",
    "fragThrow2",
    "fragThrow3",
    "faustToss",
    "reload1",
    "reload2",
    "reload3",
    "reload4",
    "reload5",
    "reload6",
    "reload7",
    "reload8",
    "reload9",
    "reload10",
    "buringMan1",
    "buringMan2",
    "buringMan3",
    "buringMan4",
];

/// Allied variants that fire bazookas and talk on the radio.
const ALLIED_EXTRA: &[&str] = &[
    "analyze",
    "brief",
    "bazooka",
    "bazooka1",
    "bazooka2",
    "bazooka3",
    "bazookaFired",
    "faustFired",
];

const FLARE_MORTAR: &[&str] = &[
    "flare1", "flare2", "flare3", "flare4", "flare5", "flare6", "flare7", "mortar1", "mortar2",
    "mortar3", "mortar4",
];

const AXIS_EXTRA: &[&str] = &["bazooka1", "bazooka2", "bazooka3"];

/// Unit variants of a side as `(schema name, vanilla unit tag)`.
pub fn variants(side: Side) -> &'static [(&'static str, &'static str)] {
    match side {
        Side::Allied => &[
            ("base", "allied"),
            ("sarge", "sarge"),
            ("para", "para"),
            ("french", "french"),
        ],
        Side::Axis => &[("base", "axis"), ("officer", "officer"), ("volks", "volks")],
    }
}

/// Every variant name of either side.
pub const ALL_VARIANTS: &[&str] = &["base", "sarge", "para", "french", "officer", "volks"];

/// The vanilla unit tag for a variant, or `None` if the side has no such variant.
pub fn variant_tag(side: Side, variant: &str) -> Option<&'static str> {
    variants(side)
        .iter()
        .find(|(v, _)| *v == variant)
        .map(|(_, t)| *t)
}

/// The frames a variant has in vanilla, in a stable order.
pub fn frames(side: Side, variant: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    match (side, variant) {
        // The sergeant has no burnt corpse, flare or mortar frames, but can smoke.
        (Side::Allied, "sarge") => {
            out.extend(COMMON_FRAMES);
            out.extend(ALLIED_EXTRA);
            out.extend(["smoking1", "smoking2"]);
        }
        (Side::Allied, _) => {
            out.extend(COMMON_FRAMES);
            out.push("deadBBQ");
            out.extend(ALLIED_EXTRA);
            out.extend(FLARE_MORTAR);
        }
        (Side::Axis, v) => {
            out.extend(COMMON_FRAMES);
            out.push("deadBBQ");
            out.extend(AXIS_EXTRA);
            out.extend(FLARE_MORTAR);
            if v != "base" {
                out.push("jagerMove");
            }
        }
    }
    out
}

/// Every frame name any variant has.
pub fn all_frames() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for side in [Side::Allied, Side::Axis] {
        for (v, _) in variants(side) {
            for f in frames(side, v) {
                if !out.contains(&f) {
                    out.push(f);
                }
            }
        }
    }
    out
}

/// The 12 class slots, with the localisation string that holds each one's mouse-over text.
pub const SPECS: &[(&str, &str)] = &[
    ("rifle", "mouseOverRifleSpec"),
    ("light", "mouseOverLightSpec"),
    ("gunner", "mouseOverGunnerSpec"),
    ("medic", "mouseOverMedicSpec"),
    ("comms", "mouseOverCommsSpec"),
    ("officer", "mouseOverOfficerSpec"),
    ("engy", "mouseOverEngySpec"),
    ("scout", "mouseOverScoutSpec"),
    ("sniper", "mouseOverSniperSpec"),
    ("commando", "mouseOverCommandoSpec"),
    ("sapper", "mouseOverSapperSpec"),
    ("intel", "mouseOverIntelSpec"),
];

pub fn is_spec(s: &str) -> bool {
    SPECS.iter().any(|(k, _)| *k == s)
}

/// Every weapon `weaponCreator` builds, with how many places in vanilla check for that exact tag
/// (`dude.weapon.tag == "k98"` and the like). A `spawn_swap` of a tag stops those checks matching.
pub const WEAPONS: &[(&str, usize)] = &[
    ("m1", 3),
    ("g98", 0),
    ("k98", 14),
    ("springfield", 3),
    ("boysRifle", 2),
    ("pz39", 1),
    ("g33", 0),
    ("mas36", 0),
    ("m1Carbine", 0),
    ("matchGarand", 1),
    ("m1941", 0),
    ("t20", 1),
    ("t22", 1),
    ("scopedK98", 0),
    ("1917", 0),
    ("scopedGarand", 0),
    ("scopedSpringfield", 2),
    ("scopedEnfield", 0),
    ("g43", 0),
    ("scopedG43", 0),
    ("svt40", 0),
    ("vg15", 0),
    ("m1CarbinePara", 0),
    ("m2Carbine", 0),
    ("m2CarbineA1", 0),
    ("bren", 0),
    ("chauchat", 0),
    ("zb26", 0),
    ("m1919", 0),
    ("vicker", 0),
    ("mg42", 0),
    ("mg15", 0),
    ("mg34", 0),
    ("tommy", 0),
    ("stenMKVI", 1),
    ("sten", 0),
    ("bar", 0),
    ("johnson", 0),
    ("m3", 0),
    ("m38", 0),
    ("mp40", 0),
    ("mp41", 0),
    ("ppsh41", 0),
    ("stg44", 0),
    ("irstg44", 6),
    ("fg42", 0),
    ("shotgun", 9),
    ("slamShotgun", 1),
];

pub fn is_weapon(tag: &str) -> bool {
    WEAPONS.iter().any(|(t, _)| *t == tag)
}

pub fn weapon_checks(tag: &str) -> usize {
    WEAPONS
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, n)| *n)
        .unwrap_or(0)
}

/// Weapons whose firing sound the game picks by tag (`garand1..4`, `springfieldSound1..3`)
/// instead of `weapon.sound`.
pub const TAG_PICKED_SOUNDS: &[&str] = &[
    "m1",
    "matchGarand",
    "t20",
    "t22",
    "springfield",
    "scopedSpringfield",
];

/// Magazine sprites the game can drop on a reload (`addParticle` in particlesSystem.lua). Any
/// other `weapon.magSprite` needs the faction magazine patch.
pub const MAGAZINES: &[&str] = &[
    "genericMag",
    "m3Mag",
    "g43Mag",
    "mg15Mag",
    "ammoBox",
    "ammoPan",
    "barMag",
    "brenMag",
    "tommyMag",
    "m1CarbineMag",
    "mp40Mag",
    "stgMag",
    "ppshMag",
];

pub fn is_magazine(name: &str) -> bool {
    MAGAZINES.contains(&name)
}

/// Random weapon pools the game draws loot and reinforcement guns from, keyed by schema name,
/// with the vanilla function that draws from each.
pub const POOLS: &[(&str, &str)] = &[
    ("allied", "getRandomAlliedGun"),
    ("allied_sniper", "getRandomAlliedSniper"),
    ("grunt", "getRandomGruntGun"),
    ("para", "getRandomParaGun"),
    ("resistance", "getRandomResistanceGun"),
    ("smg", "getRandomSMG"),
    ("axis", "getRandomGermGun"),
    ("axis_crew", "getRandomGermCrewGun"),
];

pub fn is_pool(name: &str) -> bool {
    POOLS.iter().any(|(p, _)| *p == name)
}

/// Vehicles and towed guns each side spawns (from the `spawn*` functions' `faction` field).
pub fn vehicles(side: Side) -> &'static [&'static str] {
    match side {
        Side::Allied => &[
            "sherman",
            "stuart",
            "churchill",
            "m10",
            "m20",
            "m5",
            "greyHound",
            "jeep",
            "deuce",
            "weasel",
            "weaselBoxed",
            "halfTrack",
            "ambulance",
            "howitzer",
            "landMattress",
            "bofor",
            "m2hb",
            "clownCar",
        ],
        Side::Axis => &[
            "panzer4",
            "panther",
            "tiger",
            "stug",
            "flakPanzer",
            "wespe",
            "puma",
            "sdkfz",
            "sdkfz2",
            "sdkfz2Boxed",
            "sdkfz222",
            "opel",
            "kubel",
            "benz",
            "bike",
            "horseCart",
            "goliath",
            "r35",
            "nebel",
            "flak30",
            "flak38",
            "germHowitzer",
            "pak36",
            "pak38",
            "pak88",
            "zb60",
            "armoredCannon",
            "brestCannon",
        ],
    }
}

/// The function that spawns an Allied vehicle, for giving it faction call signs.
pub const VEHICLE_SPAWNERS: &[(&str, &str)] = &[
    ("sherman", "spawnSherman"),
    ("stuart", "spawnStuart"),
    ("churchill", "spawnChurchill"),
    ("m10", "spawnM10"),
    ("m20", "spawnM20"),
    ("m5", "spawnM5"),
    ("greyHound", "spawnGreyHound"),
    ("jeep", "spawnJeep"),
    ("deuce", "spawnDeuce"),
    ("weasel", "spawnWeasel"),
    ("halfTrack", "spawnHalfTrack"),
    ("ambulance", "spawnAmbulance"),
    ("howitzer", "spawnHowitzer"),
    ("landMattress", "spawnLandMattress"),
    ("bofor", "createMNB2Bofor"),
    ("m2hb", "createMNB2Fifty"),
    ("clownCar", "spawnClownCar"),
];

pub fn vehicle_spawner(tag: &str) -> Option<&'static str> {
    VEHICLE_SPAWNERS
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, f)| *f)
}

/// Voice line folders under `sounds/vo/` that belong to a side.
pub fn voice_dirs(side: Side) -> &'static [&'static str] {
    match side {
        Side::Allied => &["allied", "frenchies"],
        Side::Axis => &["axis"],
    }
}

/// Named sprite parts a vehicle reskin can set, as `(schema key, global suffix)`.
/// The game global is the vehicle tag followed by the suffix (`shermanBody`, `sherman_husk`).
pub const VEHICLE_PARTS: &[(&str, &str)] = &[
    ("body", "Body"),
    ("turret", "Turret"),
    ("driver", "Driver"),
    ("gunner", "Gunner"),
    ("icon", "Icon"),
    ("husk", "_husk"),
];

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp"];
pub const AUDIO_EXTS: &[&str] = &["wav", "ogg", "mp3"];
