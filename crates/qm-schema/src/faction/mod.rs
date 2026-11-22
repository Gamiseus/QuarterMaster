//! Faction mods: a `faction.toml` that lets an army take over the Allied or German side.
//!
//! [`load_faction`] reads and checks the file against the mod folder, and returns a
//! [`ResolvedFaction`]: everything the game-side runtime needs, with every file found and every
//! text string worked out. QuarterMaster's loader turns that into Lua (see `qm-core`).
//!
//! Design: `docs/specs/2026-11-07-faction-framework-design.md`.

pub mod vanilla;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::manifest::{DiagnosticSeverity, ManifestDiagnostic};

/// File name QuarterMaster looks for at a mod's root.
pub const FACTION_FILE: &str = "faction.toml";
/// The schema version this build reads.
pub const FACTION_SCHEMA_VERSION: u32 = 1;
/// The editor schema (`#:schema` target) for `faction.toml`.
pub const FACTION_JSON_SCHEMA: &str = include_str!("../../../../schemas/faction.schema.json");

// ---------------------------------------------------------------------------------------------
// File format
// ---------------------------------------------------------------------------------------------

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Allied,
    Axis,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Allied => "allied",
            Side::Axis => "axis",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::Allied => "Allies",
            Side::Axis => "Germans",
        }
    }

    pub fn other(self) -> Side {
        match self {
            Side::Allied => Side::Axis,
            Side::Axis => Side::Allied,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactionFile {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub replaces: Side,
    pub side_name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub units: BTreeMap<String, UnitDef>,
    #[serde(default)]
    pub weapons: Vec<WeaponDef>,
    #[serde(default)]
    pub spawn_swap: BTreeMap<String, String>,
    #[serde(default)]
    pub pools: BTreeMap<String, PoolEdit>,
    #[serde(default)]
    pub classes: BTreeMap<String, ClassDef>,
    pub names: Option<NamesDef>,
    pub voices: Option<VoicesDef>,
    #[serde(default)]
    pub text: BTreeMap<String, String>,
    #[serde(default)]
    pub sprites: BTreeMap<String, SpriteEntry>,
    #[serde(default)]
    pub sounds: BTreeMap<String, String>,
    #[serde(default)]
    pub vehicles: Vec<VehicleDef>,
    #[serde(default)]
    pub explosions: Vec<ExplosionDef>,
    #[serde(default)]
    pub ordnance: Vec<OrdnanceDef>,
    #[serde(default)]
    pub skills: Vec<SkillDef>,
    pub ranks: Option<RanksDef>,
    pub hooks: Option<HooksDef>,
}

/// A `[sprites]` entry: an image file, or a table with the file and the size it is shown at.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SpriteEntry {
    File(String),
    Table(SpriteTable),
}

impl SpriteEntry {
    /// The image file, when the entry replaces the image.
    pub fn file(&self) -> Option<&str> {
        match self {
            SpriteEntry::File(f) => Some(f),
            SpriteEntry::Table(t) => t.file.as_deref(),
        }
    }
}

/// `rank_1 = { file = "misc/rank_1.png", size = [18, 22] }`: the file (or none, to size the
/// game's own image), one of `size`, `scale` and `like`, and a place inside the box
/// (`offset`, `rotate`, `flip`, `draw_size`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteTable {
    pub file: Option<String>,
    pub size: Option<[f64; 2]>,
    pub scale: Option<f64>,
    pub like: Option<String>,
    /// Moves the image, in the game's pixels, right and down.
    pub offset: Option<[f64; 2]>,
    /// Turns it about its middle, in degrees clockwise.
    pub rotate: Option<f64>,
    /// Mirrors it about its middle: "x", "y" or "xy".
    pub flip: Option<String>,
    /// Draws the picture at this size inside its box, in the game's pixels.
    pub draw_size: Option<[f64; 2]>,
}

/// `[ranks]`: the rank names the game shows on soldiers of the faction's side.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RanksDef {
    /// Ranks 0 to 3 of a soldier (the game's Private to Sergeant).
    pub soldier: Option<Vec<String>>,
    /// Ranks 0 to 3 of an officer (the game's Second Lieutenant to Major).
    pub officer: Option<Vec<String>>,
    /// A soldier with no rank yet (the game's Recruit); the first soldier rank unless set.
    pub recruit: Option<String>,
    /// An officer with no rank yet (the game's Cadet); the first officer rank unless set.
    pub cadet: Option<String>,
    /// The sergeant hero's title (the game's Sarge): his name, and the word in the game's lines
    /// about him. The Allies' only.
    pub sarge: Option<String>,
}

/// `[[skills]]`: a class skill changed, replaced, moved, removed or added.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDef {
    pub class: String,
    /// 1 to 3: the class's own skills; 4 and up: new ones.
    pub slot: u32,
    pub name: Option<String>,
    pub description: Option<String>,
    /// The skill's icon: an image file.
    pub icon: Option<String>,
    pub cooldown: Option<f64>,
    /// The skill level it comes with.
    pub unlock: Option<u32>,
    /// A game skill: `2` (the class's own) or `"light:3"` (any class's).
    pub game: Option<SkillGame>,
    /// An `[[ordnance]]` id of this faction.
    pub ordnance: Option<String>,
    #[serde(default)]
    pub remove: bool,
}

/// `game = 2` or `game = "light:3"`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SkillGame {
    Slot(u32),
    Named(String),
}

/// `[[explosions]]`: an explosion on one of the game's own, with numbers changed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplosionDef {
    pub id: String,
    pub based_on: Option<String>,
    pub size: Option<f64>,
    pub at_radius: Option<f64>,
    pub at_damage: Option<f64>,
    pub blast_radius: Option<f64>,
    pub blast_damage: Option<f64>,
    pub demo_radius: Option<f64>,
    pub frag_radius: Option<f64>,
    pub frag_damage: Option<f64>,
    pub morale_hit: Option<f64>,
    pub stun_range: Option<f64>,
    pub stun_time: Option<f64>,
    pub crater: Option<f64>,
}

/// `[[ordnance]]`: a grenade, rifle grenade, mortar, barrage, air strike or supply drop of the faction's own.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdnanceDef {
    pub id: String,
    /// `"thrown"` (a hand grenade, the default), `"rifle"` (a rifle grenade), `"mortar"` (a
    /// soldier's mortar rounds), `"artillery"` (a barrage), `"airstrike"` (a plane's run) or
    /// `"drop"` (a supply crate by parachute).
    pub kind: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Required for a thrown or rifle grenade; a mortar round or a shell shows only its blast.
    pub sprite: Option<String>,
    pub explosion: Option<String>,
    pub range: Option<f64>,
    pub cooldown: Option<f64>,
    pub tension: Option<f64>,
    /// A thrown grenade: how fast it turns in the air (the frag's is 0.1 to 1 at random); 0
    /// flies pointing the way it was thrown.
    pub spin: Option<f64>,
    pub skill: Option<OrdnanceSkill>,
    #[serde(default)]
    pub replaces_frag: bool,
    #[serde(default)]
    pub replaces_rifle_grenade: bool,
    /// A mortar's rounds per use, or an artillery barrage's shells.
    pub shots: Option<u32>,
    /// How far each round or shell lands from the last spot.
    pub spread: Option<f64>,
    #[serde(default)]
    pub replaces_mortar: bool,
    #[serde(default)]
    pub replaces_artillery: bool,
    /// An air strike's or a drop's plane: an image file.
    pub plane: Option<String>,
    /// A drop: the game's loot names its crate holds.
    pub contents: Option<Vec<String>>,
    #[serde(default)]
    pub replaces_bombing: bool,
    #[serde(default)]
    pub replaces_strafing: bool,
    #[serde(default)]
    pub replaces_drop: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrdnanceSkill {
    pub class: String,
    pub slot: u32,
    /// Slot 4 only: the skill level it comes with (3, after the third, unless set).
    pub unlock: Option<u32>,
    /// Slot 4 only: the button's image (the grenade's sprite unless set).
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDef {
    pub folder: Option<String>,
    #[serde(default)]
    pub frames: BTreeMap<String, String>,
    pub copy_from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LootSide {
    Allied,
    Axis,
    None,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponDef {
    pub reskin: Option<String>,
    pub tag: Option<String>,
    pub based_on: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub top_sprite: Option<String>,
    pub side_sprite: Option<String>,
    pub sound: Option<String>,
    pub mag_sprite: Option<String>,
    pub stats: Option<WeaponStats>,
    pub loot_side: Option<LootSide>,
    #[serde(default)]
    pub pools: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponStats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ammo: Option<i64>,
    #[serde(rename = "maxAmmo", skip_serializing_if = "Option::is_none")]
    pub max_ammo: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recoil: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulk: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rof: Option<f64>,
    #[serde(rename = "rofTime", skip_serializing_if = "Option::is_none")]
    pub rof_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jam: Option<f64>,
    #[serde(rename = "rgCapable", skip_serializing_if = "Option::is_none")]
    pub rg_capable: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolEdit {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassBonus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_health: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hth_damage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detect_range: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine_detect_range: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassDef {
    pub name: Option<String>,
    pub description: Option<String>,
    pub start_weapon: Option<String>,
    pub unlocks: Option<Vec<String>>,
    pub bonus: Option<ClassBonus>,
    pub skills: Option<Vec<String>>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NameFormat {
    /// `"A. Ivanov"`, like vanilla.
    #[default]
    Initial,
    /// `"Pyotr Ivanov"`, needs first names.
    Full,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamePool {
    pub first: Option<Vec<String>>,
    pub last: Option<Vec<String>>,
    pub first_file: Option<String>,
    pub last_file: Option<String>,
    pub format: Option<NameFormat>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamesDef {
    pub first: Option<Vec<String>>,
    pub last: Option<Vec<String>>,
    pub first_file: Option<String>,
    pub last_file: Option<String>,
    pub format: Option<NameFormat>,
    pub vehicles: Option<Vec<String>>,
    pub vehicles_file: Option<String>,
    pub french: Option<NamePool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicesDef {
    pub folder: Option<String>,
    pub fill_missing: Option<bool>,
    #[serde(default)]
    pub map: BTreeMap<String, String>,
    #[serde(default)]
    pub dirs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VehicleDef {
    pub reskin: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub turret: Option<String>,
    pub driver: Option<String>,
    pub gunner: Option<String>,
    pub icon: Option<String>,
    pub husk: Option<String>,
    pub debris: Option<String>,
    #[serde(default)]
    pub parts: BTreeMap<String, String>,
    pub call_signs: Option<Vec<String>>,
    pub call_signs_file: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HooksDef {
    pub lua: String,
}

// ---------------------------------------------------------------------------------------------
// Resolved form handed to the runtime
// ---------------------------------------------------------------------------------------------

/// A checked faction with every path relative to the mod root (forward slashes) and every
/// string the game shows already worked out.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedFaction {
    pub id: String,
    pub name: String,
    pub replaces: Side,
    pub side_name: Option<String>,
    pub description: Option<String>,
    /// Vanilla unit tag -> frame -> image.
    pub frames: BTreeMap<String, BTreeMap<String, String>>,
    pub weapons: Vec<ResolvedWeapon>,
    pub spawn_swap: BTreeMap<String, String>,
    /// Pool name -> weapons to add and remove.
    pub pools: BTreeMap<String, ResolvedPool>,
    pub classes: BTreeMap<String, ResolvedClass>,
    pub names: Option<ResolvedNames>,
    pub voices: Vec<ResolvedVoiceDir>,
    /// Localisation global -> text. Weapon, class and skill text is folded in here.
    pub text: BTreeMap<String, String>,
    /// Game global -> image.
    pub sprites: BTreeMap<String, String>,
    /// Game global -> the size it is shown at, handed to the `[sprites]` size rules.
    pub sprite_sizes: BTreeMap<String, ResolvedSpriteSize>,
    /// Game global -> sound.
    pub sounds: BTreeMap<String, String>,
    pub vehicles: Vec<ResolvedVehicle>,
    pub explosions: Vec<ResolvedExplosion>,
    pub ordnance: Vec<ResolvedOrdnance>,
    pub skills: Vec<ResolvedSkill>,
    pub ranks: Option<ResolvedRanks>,
    pub hooks: Option<String>,
}

/// One of `size`, `scale` and `like`, and a place, as the size rules of `qm.toml` take them.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ResolvedSpriteSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draw_size: Option<[f64; 2]>,
}

/// The `flip` values a sprite rule takes.
pub const SPRITE_FLIPS: [&str; 3] = ["x", "y", "xy"];

/// Rank names. A list holds ranks 0 to 3; a missing one keeps the game's.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ResolvedRanks {
    pub soldier: Vec<String>,
    pub officer: Vec<String>,
    pub recruit: Option<String>,
    pub cadet: Option<String>,
    pub sarge: Option<String>,
}

/// A class skill for `QM.Skills.set`.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSkill {
    pub class: String,
    pub slot: u32,
    pub name: Option<String>,
    pub description: Option<String>,
    /// The icon, relative to the mod folder.
    pub icon: Option<String>,
    pub cooldown: Option<f64>,
    pub unlock: Option<u32>,
    pub game_class: Option<String>,
    pub game_slot: Option<u32>,
    /// `<faction id>:<id>`.
    pub ordnance: Option<String>,
    pub remove: bool,
}

/// An explosion for `QM.Explosions.register`: its id is `<faction id>:<id>`; the numbers use
/// the game's names (`fragRadius`).
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedExplosion {
    pub id: String,
    pub based_on: String,
    pub numbers: BTreeMap<String, f64>,
}

/// The kinds of `[[ordnance]]`: (kind, the game's explosion when left out; none for a drop).
pub const ORDNANCE_KINDS: &[(&str, &str)] = &[
    ("thrown", "frag"),
    ("rifle", "rifle_grenade"),
    ("mortar", "mortar"),
    ("artillery", "artillery"),
    ("airstrike", "bomb"),
    ("drop", ""),
];

/// The keys that make ordnance stand in for the game's own: (key, its kind, what it replaces).
pub const ORDNANCE_SWAPS: &[(&str, &str, &str)] = &[
    ("replaces_frag", "thrown", "the frag"),
    ("replaces_rifle_grenade", "rifle", "the rifle grenade"),
    ("replaces_mortar", "mortar", "the mortar"),
    ("replaces_artillery", "artillery", "the artillery barrages"),
    ("replaces_bombing", "airstrike", "the bombing runs"),
    ("replaces_strafing", "airstrike", "the strafing runs"),
    ("replaces_drop", "drop", "the supply crates"),
];

/// Ordnance for `QM.Ordnance.register`.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedOrdnance {
    /// `<faction id>:<id>`.
    pub id: String,
    /// `"thrown"`, `"rifle"`, `"mortar"`, `"artillery"`, `"airstrike"` or `"drop"`.
    pub kind: String,
    /// Shown on a fourth skill's button.
    pub name: Option<String>,
    pub description: Option<String>,
    /// The image, relative to the mod folder (none for a mortar or artillery without one).
    pub sprite: Option<String>,
    /// A faction explosion's full id, or a game explosion tag.
    pub explosion: String,
    pub range: Option<f64>,
    pub cooldown: Option<f64>,
    pub tension: Option<f64>,
    /// A thrown grenade's turn rate in the air; 0 points it the way it was thrown.
    pub spin: Option<f64>,
    pub skill_class: Option<String>,
    pub skill_slot: Option<u32>,
    pub skill_unlock: Option<u32>,
    pub skill_icon: Option<String>,
    pub replaces_frag: bool,
    pub replaces_rifle_grenade: bool,
    pub shots: Option<u32>,
    pub spread: Option<f64>,
    pub replaces_mortar: bool,
    pub replaces_artillery: bool,
    /// The plane's image, relative to the mod folder.
    pub plane: Option<String>,
    pub contents: Vec<String>,
    pub replaces_bombing: bool,
    pub replaces_strafing: bool,
    pub replaces_drop: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedWeapon {
    pub tag: String,
    /// `"reskin"` or `"new"`.
    pub mode: String,
    pub based_on: Option<String>,
    pub top: Option<String>,
    pub side: Option<String>,
    pub sounds: Vec<String>,
    /// A game sound global used instead of the faction's own files.
    pub sound_name: Option<String>,
    pub mag_name: Option<String>,
    pub mag_file: Option<String>,
    pub stats: WeaponStats,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolvedPool {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedClass {
    pub start_weapon: Option<String>,
    pub unlocks: Option<Vec<String>>,
    pub bonus: Option<ClassBonus>,
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedNamePool {
    pub first: Vec<String>,
    pub last: Vec<String>,
    pub format: NameFormat,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedNames {
    pub soldiers: Option<ResolvedNamePool>,
    pub french: Option<ResolvedNamePool>,
    pub vehicles: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedVoiceDir {
    /// Vanilla folder, e.g. `sounds/vo/allied`.
    pub vanilla_dir: String,
    pub fill_missing: bool,
    /// Vanilla file stem (`contact3`) -> faction file.
    pub exact: BTreeMap<String, String>,
    /// Vanilla stem prefix (`contact`) -> the faction's numbered files in order.
    pub pools: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedVehicle {
    pub tag: String,
    pub name: Option<String>,
    /// Game global (`shermanBody`) -> image.
    pub parts: BTreeMap<String, String>,
    /// Debris images in order; the runtime spreads them over the vanilla debris frames.
    pub debris: Vec<String>,
    /// Call signs given to this vehicle when it spawns.
    pub call_signs: Vec<String>,
    /// The vanilla function that spawns it (`spawnSherman`), when call signs are set.
    pub spawner: Option<String>,
}

/// Result of reading one mod's `faction.toml`.
#[derive(Debug, Clone)]
pub struct FactionReport {
    /// `None` when the file has errors; the faction is then not loaded.
    pub faction: Option<ResolvedFaction>,
    pub diagnostics: Vec<ManifestDiagnostic>,
}

impl FactionReport {
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
    }
}

/// True when the mod folder has a `faction.toml`.
pub fn has_faction_file(mod_dir: &Path) -> bool {
    mod_dir.join(FACTION_FILE).is_file()
}

/// Who a faction mod is, without checking its files: enough for a mod list.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactionHeader {
    pub id: String,
    pub name: String,
    pub replaces: Side,
}

/// Reads the header of `<mod_dir>/faction.toml`, or `None` if there is none or it doesn't parse.
pub fn read_faction_header(mod_dir: &Path) -> Option<FactionHeader> {
    #[derive(Deserialize)]
    struct Head {
        id: String,
        name: String,
        replaces: Side,
    }
    let text = fs::read_to_string(mod_dir.join(FACTION_FILE)).ok()?;
    let table: toml::Table = toml::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let head: Head = toml::Value::Table(table).try_into().ok()?;
    Some(FactionHeader {
        id: head.id,
        name: head.name,
        replaces: head.replaces,
    })
}

/// Reads `<mod_dir>/faction.toml` and checks it against the files in `mod_dir`.
pub fn load_faction(mod_dir: &Path) -> FactionReport {
    let path = mod_dir.join(FACTION_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => check_faction(&text, mod_dir),
        Err(e) => FactionReport {
            faction: None,
            diagnostics: vec![ManifestDiagnostic::error(format!(
                "{FACTION_FILE}: can't read the file ({e})."
            ))],
        },
    }
}

/// Checks `faction.toml` text against the files under `mod_dir`.
pub fn check_faction(text: &str, mod_dir: &Path) -> FactionReport {
    let file: FactionFile = match toml::from_str(text.trim_start_matches('\u{feff}')) {
        Ok(f) => f,
        Err(e) => {
            return FactionReport {
                faction: None,
                diagnostics: vec![ManifestDiagnostic::error(format!(
                    "{FACTION_FILE}: {}",
                    e.to_string().trim_end()
                ))],
            }
        }
    };
    let mut cx = Checker {
        root: mod_dir,
        diags: Vec::new(),
    };
    let faction = cx.resolve(&file);
    let report = FactionReport {
        faction,
        diagnostics: cx.diags,
    };
    if report.has_errors() {
        FactionReport {
            faction: None,
            ..report
        }
    } else {
        report
    }
}

// ---------------------------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------------------------

struct Checker<'a> {
    root: &'a Path,
    diags: Vec<ManifestDiagnostic>,
}

/// An id inside the faction: lowercase letters, digits and underscores.
fn is_local_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `fragRadius` as written in faction.toml: `frag_radius`.
fn snake(key: &str) -> String {
    let mut out = String::new();
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_tag(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Splits `contact12` into `("contact", 12)`.
fn split_number(stem: &str) -> Option<(&str, u32)> {
    let digits = stem.len() - stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || digits == stem.len() {
        return None;
    }
    let (prefix, num) = stem.split_at(stem.len() - digits);
    num.parse().ok().map(|n| (prefix, n))
}

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

impl<'a> Checker<'a> {
    fn error(&mut self, msg: impl Into<String>) {
        self.diags.push(ManifestDiagnostic::error(format!(
            "{FACTION_FILE}: {}",
            msg.into()
        )));
    }

    fn warn(&mut self, msg: impl Into<String>) {
        self.diags.push(ManifestDiagnostic::warning(format!(
            "{FACTION_FILE}: {}",
            msg.into()
        )));
    }

    fn info(&mut self, msg: impl Into<String>) {
        self.diags.push(ManifestDiagnostic::info(format!(
            "{FACTION_FILE}: {}",
            msg.into()
        )));
    }

    /// Normalises a mod-relative path, or reports why it can't be used.
    fn rel(&mut self, what: &str, raw: &str) -> Option<String> {
        let norm = raw.trim().replace('\\', "/");
        let bad = norm.is_empty()
            || norm.starts_with('/')
            || norm.contains(':')
            || Path::new(&norm).components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            });
        if bad {
            self.error(format!(
                "{what}: \"{raw}\" must be a path inside the mod folder (no drive letters, no leading slash, no \"..\")."
            ));
            return None;
        }
        Some(
            norm.trim_start_matches("./")
                .trim_end_matches('/')
                .to_string(),
        )
    }

    /// A file that must exist and have one of `exts`.
    fn file(&mut self, what: &str, raw: &str, exts: &[&str]) -> Option<String> {
        let rel = self.rel(what, raw)?;
        let ext = ext_of(&rel);
        if !exts.contains(&ext.as_str()) {
            self.error(format!(
                "{what}: \"{rel}\" must be one of: {}.",
                exts.join(", ")
            ));
            return None;
        }
        if !self.root.join(&rel).is_file() {
            self.error(format!(
                "{what}: file \"{rel}\" not found in the mod folder."
            ));
            return None;
        }
        Some(rel)
    }

    fn folder(&mut self, what: &str, raw: &str) -> Option<String> {
        let rel = self.rel(what, raw)?;
        if !self.root.join(&rel).is_dir() {
            self.error(format!(
                "{what}: folder \"{rel}\" not found in the mod folder."
            ));
            return None;
        }
        Some(rel)
    }

    /// Files in a mod folder (not recursive), sorted, as `(file name, mod-relative path)`.
    fn list_dir(&self, rel_dir: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = fs::read_dir(self.root.join(rel_dir))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().is_file())
                    .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                    .filter(|n| !n.starts_with('.'))
                    .map(|n| {
                        let rel = if rel_dir.is_empty() {
                            n.clone()
                        } else {
                            format!("{rel_dir}/{n}")
                        };
                        (n, rel)
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    /// A numbered file set named by a prefix: `weapons/mosin` finds `mosin.wav`, `mosin1.wav`,
    /// `mosin2.wav`, ... A path with an extension names one file.
    fn numbered(&mut self, what: &str, raw: &str, exts: &[&str]) -> Vec<String> {
        let Some(rel) = self.rel(what, raw) else {
            return Vec::new();
        };
        if exts.contains(&ext_of(&rel).as_str()) {
            return self.file(what, &rel, exts).into_iter().collect();
        }
        let (dir, base) = match rel.rfind('/') {
            Some(i) => (rel[..i].to_string(), rel[i + 1..].to_string()),
            None => (String::new(), rel.clone()),
        };
        let mut exact: Vec<String> = Vec::new();
        let mut numbered: Vec<(u32, String)> = Vec::new();
        for (name, path) in self.list_dir(&dir) {
            if !exts.contains(&ext_of(&name).as_str()) {
                continue;
            }
            let stem = stem_of(&name);
            if stem.eq_ignore_ascii_case(&base) {
                exact.push(path);
            } else if let Some((pfx, n)) = split_number(stem) {
                if pfx.eq_ignore_ascii_case(&base) {
                    numbered.push((n, path));
                }
            }
        }
        numbered.sort();
        let mut out = exact;
        out.extend(numbered.into_iter().map(|(_, p)| p));
        if out.is_empty() {
            self.error(format!(
                "{what}: no {} files named \"{base}\" or \"{base}1\", \"{base}2\", ... in \"{}\".",
                exts.join("/"),
                if dir.is_empty() {
                    "the mod folder"
                } else {
                    &dir
                }
            ));
        }
        out
    }

    fn lines_file(&mut self, what: &str, raw: &str) -> Vec<String> {
        let Some(rel) = self.file(what, raw, &["txt"]) else {
            return Vec::new();
        };
        match fs::read_to_string(self.root.join(&rel)) {
            Ok(text) => text
                .trim_start_matches('\u{feff}')
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.to_string())
                .collect(),
            Err(e) => {
                self.error(format!("{what}: can't read \"{rel}\" ({e})."));
                Vec::new()
            }
        }
    }

    fn resolve(&mut self, f: &FactionFile) -> Option<ResolvedFaction> {
        if f.schema != FACTION_SCHEMA_VERSION {
            self.error(format!(
                "schema = {} is not supported; this QuarterMaster reads schema {FACTION_SCHEMA_VERSION}.",
                f.schema
            ));
            return None;
        }
        if f.id.is_empty()
            || !f
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            self.error(format!(
                "id \"{}\" must use only lowercase letters, digits and underscores.",
                f.id
            ));
        }
        if f.name.trim().is_empty() {
            self.error("name can't be empty.");
        }
        let side = f.replaces;

        // Weapons first: classes, pools and swaps can name the faction's new weapons.
        let weapons = self.weapons(f, side);
        let mut known: BTreeSet<String> = vanilla::WEAPONS
            .iter()
            .map(|(t, _)| t.to_string())
            .collect();
        for (w, _, _) in &weapons {
            known.insert(w.tag.clone());
        }

        let frames = self.units(f, side);
        let spawn_swap = self.spawn_swap(f, &known);
        let mut pools = self.pools(f, &known);
        for (w, _, pool_names) in &weapons {
            for p in pool_names {
                let entry = pools.entry(p.clone()).or_default();
                if !entry.add.contains(&w.tag) {
                    entry.add.push(w.tag.clone());
                }
            }
        }
        let mut text: BTreeMap<String, String> = BTreeMap::new();
        for (w, t, _) in &weapons {
            if let Some(t) = t {
                text.insert(format!("weaponText_{}", w.tag), t.clone());
            }
        }
        let weapons: Vec<ResolvedWeapon> = weapons.into_iter().map(|(w, _, _)| w).collect();
        let classes = self.classes(f, side, &known, &mut text);
        let names = self.names(f, side);
        let voices = self.voices(f, side);
        let (sprites, sprite_sizes) = self.sprites(&f.sprites);
        let ranks = self.ranks(f.ranks.as_ref(), side);
        let sounds = self.globals_map("sounds", &f.sounds, vanilla::AUDIO_EXTS);
        let vehicles = self.vehicles(f, side);
        let explosions = self.explosions(f);
        let ordnance = self.ordnance(f, side, &explosions);
        let skills = self.skills(f, side, &ordnance);
        for (k, v) in &f.text {
            if !is_identifier(k) {
                self.error(format!("text: \"{k}\" is not a valid game string name."));
                continue;
            }
            text.insert(k.clone(), v.clone());
        }
        let hooks = f
            .hooks
            .as_ref()
            .and_then(|h| self.file("hooks.lua", &h.lua, &["lua"]));

        Some(ResolvedFaction {
            id: f.id.clone(),
            name: f.name.trim().to_string(),
            replaces: side,
            side_name: f.side_name.clone(),
            description: f.description.clone(),
            frames,
            weapons,
            spawn_swap,
            pools,
            classes,
            names,
            voices,
            text,
            sprites,
            sprite_sizes,
            sounds,
            vehicles,
            explosions,
            ordnance,
            skills,
            ranks,
            hooks,
        })
    }

    fn explosions(&mut self, f: &FactionFile) -> Vec<ResolvedExplosion> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for (i, e) in f.explosions.iter().enumerate() {
            let label = format!("explosions[{i}] ({})", e.id);
            if !is_local_id(&e.id) {
                self.error(format!("{label}: id must use only lowercase letters, digits and underscores."));
                continue;
            }
            if !seen.insert(e.id.clone()) {
                self.error(format!("{label}: another explosion has this id."));
                continue;
            }
            let based_on = e.based_on.clone().unwrap_or_else(|| "frag".to_string());
            if vanilla::explosion_preset(&based_on).is_none() {
                self.error(format!(
                    "{label}: based_on \"{based_on}\" is not one of the game's explosions; use one of: {}.",
                    vanilla::EXPLOSION_PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                ));
                continue;
            }
            let mut numbers = BTreeMap::new();
            for (key, value) in [
                ("size", e.size),
                ("atRadius", e.at_radius),
                ("atDamage", e.at_damage),
                ("blastRadius", e.blast_radius),
                ("blastDamage", e.blast_damage),
                ("demoRadius", e.demo_radius),
                ("fragRadius", e.frag_radius),
                ("fragDamage", e.frag_damage),
                ("moraleHit", e.morale_hit),
                ("stunRange", e.stun_range),
                ("stunTime", e.stun_time),
                ("crater", e.crater),
            ] {
                let Some(v) = value else { continue };
                if !v.is_finite() || v < 0.0 {
                    self.error(format!("{label}: {} can't be negative.", snake(key)));
                    continue;
                }
                numbers.insert(key.to_string(), v);
            }
            out.push(ResolvedExplosion {
                id: format!("{}:{}", f.id, e.id),
                based_on,
                numbers,
            });
        }
        out
    }

    fn ordnance(&mut self, f: &FactionFile, side: Side, explosions: &[ResolvedExplosion]) -> Vec<ResolvedOrdnance> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let mut skills: BTreeSet<(String, u32)> = BTreeSet::new();
        // Each kind's swap of the game's own: the one grenade of the faction that has it.
        let mut swaps: BTreeMap<&str, String> = BTreeMap::new();
        for (i, g) in f.ordnance.iter().enumerate() {
            let label = format!("ordnance[{i}] ({})", g.id);
            if !is_local_id(&g.id) {
                self.error(format!("{label}: id must use only lowercase letters, digits and underscores."));
                continue;
            }
            if !seen.insert(g.id.clone()) {
                self.error(format!("{label}: another [[ordnance]] entry has this id."));
                continue;
            }
            let kind = g.kind.as_deref().unwrap_or("thrown");
            // (kind, the game's explosion when left out, the swap key, what it stands in for)
            let Some(&(_, default_explosion)) = ORDNANCE_KINDS.iter().find(|k| k.0 == kind) else {
                self.error(format!(
                    "{label}: kind must be \"thrown\" (a hand grenade), \"rifle\" (a rifle grenade), \"mortar\", \"artillery\", \"airstrike\" or \"drop\", not \"{kind}\"."
                ));
                continue;
            };
            let is_drop = kind == "drop";
            let has_plane = kind == "airstrike" || is_drop;
            let plane = match &g.plane {
                Some(p) if has_plane => match self.file(&format!("{label}.plane"), p, vanilla::IMAGE_EXTS) {
                    Some(p) => Some(p),
                    None => continue,
                },
                Some(_) => {
                    self.warn(format!("{label}: plane is for an air strike or a drop; it is left out."));
                    None
                }
                None => None,
            };
            let mut contents: Vec<String> = Vec::new();
            if let Some(list) = &g.contents {
                if !is_drop {
                    self.warn(format!("{label}: contents are for a drop; they are left out."));
                } else {
                    for item in list {
                        if !(item.starts_with("loot_") || item.starts_with("side_")) {
                            self.warn(format!(
                                "{label}.contents: \"{item}\" is not one of the game's loot names (loot_medPack, loot_ration, side_<gun>, ...)."
                            ));
                        }
                    }
                    contents = list.clone();
                }
            }
            if is_drop && g.explosion.is_some() {
                self.warn(format!("{label}: a drop has no explosion; explosion is left out."));
            }
            let thrown_or_rifle = kind == "thrown" || kind == "rifle";
            let sprite = match &g.sprite {
                Some(s) => match self.file(&format!("{label}.sprite"), s, vanilla::IMAGE_EXTS) {
                    Some(p) => Some(p),
                    None => continue,
                },
                None if thrown_or_rifle => {
                    self.error(format!("{label}: a {kind} grenade needs a sprite, the image it flies as."));
                    continue;
                }
                None => None,
            };
            if !thrown_or_rifle {
                if g.shots == Some(0) {
                    self.error(format!("{label}: shots must be 1 or more."));
                }
                if is_drop && g.spread.is_some() {
                    self.warn(format!("{label}: a drop lands on the spot; spread is left out."));
                }
            } else if g.shots.is_some() || g.spread.is_some() {
                self.warn(format!(
                    "{label}: shots and spread are for a mortar, artillery or an air strike; a {kind} grenade is one at a time."
                ));
            }
            if g.spread.is_some_and(|v| !v.is_finite() || v < 0.0) {
                self.error(format!("{label}: spread can't be negative."));
            }
            // The faction's own explosion, a preset by name, or a game explosion tag; the
            // game's own for the kind when left out.
            let wanted = g.explosion.clone().unwrap_or_else(|| default_explosion.to_string());
            let explosion = if is_drop {
                String::new()
            } else if let Some(e) = explosions.iter().find(|e| e.id == format!("{}:{wanted}", f.id)) {
                e.id.clone()
            } else if let Some((_, tag)) = vanilla::explosion_preset(&wanted) {
                tag.to_string()
            } else {
                self.error(format!(
                    "{label}: explosion \"{wanted}\" is not an explosion of this faction or one of the game's ({}).",
                    vanilla::EXPLOSION_PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                ));
                continue;
            };
            for (what, v) in [("range", g.range), ("cooldown", g.cooldown), ("tension", g.tension)] {
                if v.is_some_and(|v| !v.is_finite() || v < 0.0) {
                    self.error(format!("{label}: {what} can't be negative."));
                }
            }
            if g.range == Some(0.0) {
                self.error(format!("{label}: range must be more than 0."));
            }
            if g.spin.is_some_and(|v| !v.is_finite() || v < 0.0) {
                self.error(format!("{label}: spin can't be negative."));
            }
            if g.spin.is_some() && kind != "thrown" {
                self.warn(format!(
                    "{label}: spin is for a thrown grenade; a {kind} flies as the game's own, and spin is left out."
                ));
            }
            let (mut skill_class, mut skill_slot, mut skill_unlock, mut skill_icon) = (None, None, None, None);
            if let Some(s) = &g.skill {
                if !vanilla::is_spec(&s.class) {
                    self.error(format!(
                        "{label}.skill: \"{}\" is not a class; use one of: {}.",
                        s.class,
                        vanilla::SPECS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
                    ));
                } else if s.slot == 0 {
                    self.error(format!("{label}.skill: slot must be 1 or more: 1 to 3 are the class's own skills, 4 and up new ones."));
                } else if !skills.insert((s.class.clone(), s.slot)) {
                    self.error(format!("{label}.skill: another [[ordnance]] entry already takes {} skill {}.", s.class, s.slot));
                } else {
                    if side == Side::Axis {
                        self.warn(format!(
                            "{label}.skill: class skills are the player's, and the player commands the Allies; on the Axis side it is never used."
                        ));
                    }
                    skill_class = Some(s.class.clone());
                    skill_slot = Some(s.slot);
                    if let Some(u) = s.unlock {
                        if u >= 1 {
                            skill_unlock = Some(u);
                        } else {
                            self.error(format!("{label}.skill: unlock must be 1 or more (the skill level it comes with)."));
                        }
                    }
                    if let Some(icon) = &s.icon {
                        skill_icon = self.file(&format!("{label}.skill.icon"), icon, vanilla::IMAGE_EXTS);
                    }
                }
            }
            let set = [
                ("replaces_frag", g.replaces_frag),
                ("replaces_rifle_grenade", g.replaces_rifle_grenade),
                ("replaces_mortar", g.replaces_mortar),
                ("replaces_artillery", g.replaces_artillery),
                ("replaces_bombing", g.replaces_bombing),
                ("replaces_strafing", g.replaces_strafing),
                ("replaces_drop", g.replaces_drop),
            ];
            let mut bad = false;
            for (key, on) in set {
                if !on {
                    continue;
                }
                let Some(&(_, wants, what)) = ORDNANCE_SWAPS.iter().find(|s| s.0 == key) else {
                    continue;
                };
                if wants != kind {
                    let own: Vec<&str> = ORDNANCE_SWAPS.iter().filter(|s| s.1 == kind).map(|s| s.0).collect();
                    self.error(format!(
                        "{label}: {key} is for kind = \"{wants}\"; kind = \"{kind}\" has {}.",
                        if own.is_empty() { "none".to_string() } else { own.join(" or ") }
                    ));
                    bad = true;
                } else if let Some(other) = swaps.get(key) {
                    self.error(format!("{label}: {other} already replaces {what}; only one entry can."));
                    bad = true;
                } else {
                    swaps.insert(key, g.id.clone());
                    if key == "replaces_drop" && side == Side::Axis {
                        self.warn(format!(
                            "{label}: supply crates are the Allies'; on the Axis side replaces_drop is never used."
                        ));
                    }
                }
            }
            if bad {
                continue;
            }
            out.push(ResolvedOrdnance {
                id: format!("{}:{}", f.id, g.id),
                kind: kind.to_string(),
                name: g.name.clone(),
                description: g.description.clone(),
                sprite,
                explosion,
                range: g.range,
                cooldown: g.cooldown,
                tension: g.tension,
                spin: if kind == "thrown" { g.spin } else { None },
                skill_class,
                skill_slot,
                skill_unlock,
                skill_icon,
                replaces_frag: g.replaces_frag,
                replaces_rifle_grenade: g.replaces_rifle_grenade,
                shots: if thrown_or_rifle { None } else { g.shots },
                spread: if thrown_or_rifle || is_drop { None } else { g.spread },
                replaces_mortar: g.replaces_mortar,
                replaces_artillery: g.replaces_artillery,
                plane,
                contents,
                replaces_bombing: g.replaces_bombing,
                replaces_strafing: g.replaces_strafing,
                replaces_drop: g.replaces_drop,
            });
        }
        out
    }

    fn skills(&mut self, f: &FactionFile, side: Side, ordnance: &[ResolvedOrdnance]) -> Vec<ResolvedSkill> {
        let mut out = Vec::new();
        // The class skills the faction's ordnance already takes.
        let mut taken: BTreeSet<(String, u32)> = ordnance
            .iter()
            .filter_map(|o| Some((o.skill_class.clone()?, o.skill_slot?)))
            .collect();
        let specs = || vanilla::SPECS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ");
        for (i, s) in f.skills.iter().enumerate() {
            let label = format!("skills[{i}] ({} skill {})", s.class, s.slot);
            if !vanilla::is_spec(&s.class) {
                self.error(format!("{label}: \"{}\" is not a class; use one of: {}.", s.class, specs()));
                continue;
            }
            if s.slot == 0 {
                self.error(format!("{label}: slot must be 1 or more: 1 to 3 are the class's own skills, 4 and up new ones."));
                continue;
            }
            if !taken.insert((s.class.clone(), s.slot)) {
                self.error(format!(
                    "{label}: another [[skills]] or [[ordnance]] entry already takes {} skill {}.",
                    s.class, s.slot
                ));
                continue;
            }
            let actions = [s.game.is_some(), s.ordnance.is_some(), s.remove].iter().filter(|a| **a).count();
            if actions > 1 {
                self.error(format!("{label}: game, ordnance and remove are one or the other."));
                continue;
            }
            if s.slot > 3 && actions == 0 {
                self.error(format!("{label}: skill {} is a new skill; it needs game or ordnance.", s.slot));
                continue;
            }
            let (mut game_class, mut game_slot) = (None, None);
            if let Some(g) = &s.game {
                let parsed = match g {
                    SkillGame::Slot(n) => Some((s.class.clone(), *n)),
                    SkillGame::Named(t) => match t.split_once(':') {
                        Some((c, n)) => n.parse::<u32>().ok().map(|n| (c.to_string(), n)),
                        None => t.parse::<u32>().ok().map(|n| (s.class.clone(), n)),
                    },
                };
                match parsed {
                    Some((c, n)) if vanilla::is_spec(&c) && (1..=3).contains(&n) => {
                        if c == "comms" && s.class != "comms" {
                            self.warn(format!(
                                "{label}.game: the radio operator's calls go through only for radio operators."
                            ));
                        }
                        game_class = Some(c);
                        game_slot = Some(n);
                    }
                    _ => {
                        self.error(format!(
                            "{label}.game: a game skill is 1, 2 or 3 (the class's own), or \"<class>:<slot>\" such as \"light:3\"."
                        ));
                        continue;
                    }
                }
            }
            let mut ord = None;
            if let Some(id) = &s.ordnance {
                let full = format!("{}:{id}", f.id);
                if ordnance.iter().any(|o| o.id == full) {
                    ord = Some(full);
                } else {
                    self.error(format!("{label}.ordnance: \"{id}\" is not an [[ordnance]] entry of this faction."));
                    continue;
                }
            }
            if s.unlock == Some(0) {
                self.error(format!("{label}: unlock must be 1 or more (the skill level it comes with)."));
                continue;
            }
            if s.cooldown.is_some_and(|v| !v.is_finite() || v < 0.0) {
                self.error(format!("{label}: cooldown can't be negative."));
                continue;
            }
            let icon = match &s.icon {
                Some(p) => match self.file(&format!("{label}.icon"), p, vanilla::IMAGE_EXTS) {
                    Some(x) => Some(x),
                    None => continue,
                },
                None => None,
            };
            if s.remove
                && (s.name.is_some() || s.description.is_some() || icon.is_some() || s.cooldown.is_some() || s.unlock.is_some())
            {
                self.warn(format!("{label}: a removed skill shows nothing; its other keys are left out."));
            }
            if side == Side::Axis {
                self.warn(format!(
                    "{label}: class skills are the player's, and the player commands the Allies; on the Axis side it is never used."
                ));
            }
            out.push(ResolvedSkill {
                class: s.class.clone(),
                slot: s.slot,
                name: s.name.clone(),
                description: s.description.clone(),
                icon,
                cooldown: s.cooldown,
                unlock: s.unlock,
                game_class,
                game_slot,
                ordnance: ord,
                remove: s.remove,
            });
        }
        out
    }

    fn units(&mut self, f: &FactionFile, side: Side) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut by_variant: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (variant, def) in &f.units {
            if vanilla::variant_tag(side, variant).is_none() {
                let valid: Vec<&str> = vanilla::variants(side).iter().map(|(v, _)| *v).collect();
                self.error(format!(
                    "units.{variant}: the {} have no \"{variant}\" unit; use one of: {}.",
                    side.label(),
                    valid.join(", ")
                ));
                continue;
            }
            let known = vanilla::frames(side, variant);
            let mut frames: BTreeMap<String, String> = BTreeMap::new();
            if let Some(folder) = &def.folder {
                if let Some(dir) = self.folder(&format!("units.{variant}.folder"), folder) {
                    let mut ignored: Vec<String> = Vec::new();
                    for (name, path) in self.list_dir(&dir) {
                        if !vanilla::IMAGE_EXTS.contains(&ext_of(&name).as_str()) {
                            continue;
                        }
                        let stem = stem_of(&name);
                        match known.iter().find(|k| k.eq_ignore_ascii_case(stem)) {
                            Some(frame) => {
                                frames.insert(frame.to_string(), path);
                            }
                            None => ignored.push(name),
                        }
                    }
                    if !ignored.is_empty() {
                        self.warn(format!(
                            "units.{variant}: these files don't match a frame and are ignored: {}.",
                            ignored.join(", ")
                        ));
                    }
                }
            }
            for (frame, raw) in &def.frames {
                if !known.contains(&frame.as_str()) {
                    self.error(format!(
                        "units.{variant}.frames: \"{frame}\" is not a frame this unit has."
                    ));
                    continue;
                }
                if let Some(path) = self.file(
                    &format!("units.{variant}.frames.{frame}"),
                    raw,
                    vanilla::IMAGE_EXTS,
                ) {
                    frames.insert(frame.clone(), path);
                }
            }
            by_variant.insert(variant.clone(), frames);
        }

        // copy_from fills gaps from another variant's faction frames (one level, no chains).
        let snapshot = by_variant.clone();
        for (variant, def) in &f.units {
            let Some(src) = &def.copy_from else { continue };
            if src == variant {
                self.error(format!(
                    "units.{variant}.copy_from can't name the unit itself."
                ));
                continue;
            }
            if vanilla::variant_tag(side, src).is_none() {
                self.error(format!(
                    "units.{variant}.copy_from: the {} have no \"{src}\" unit.",
                    side.label()
                ));
                continue;
            }
            let Some(source) = snapshot.get(src) else {
                self.error(format!(
                    "units.{variant}.copy_from: \"{src}\" has no [units.{src}] section to copy from."
                ));
                continue;
            };
            let known = vanilla::frames(side, variant);
            if let Some(target) = by_variant.get_mut(variant) {
                for (frame, path) in source {
                    if known.contains(&frame.as_str()) && !target.contains_key(frame) {
                        target.insert(frame.clone(), path.clone());
                    }
                }
            }
        }

        let mut out = BTreeMap::new();
        for (variant, frames) in by_variant {
            let total = vanilla::frames(side, &variant).len();
            let missing = total.saturating_sub(frames.len());
            if frames.is_empty() {
                self.warn(format!(
                    "units.{variant}: no frames found, so this unit stays vanilla."
                ));
            } else if missing > 0 {
                let have: BTreeSet<&str> = frames.keys().map(|s| s.as_str()).collect();
                let list: Vec<&str> = vanilla::frames(side, &variant)
                    .into_iter()
                    .filter(|f| !have.contains(f))
                    .collect();
                self.info(format!(
                    "units.{variant}: {missing} of {total} frames keep the vanilla image: {}.",
                    list.join(", ")
                ));
            }
            if let Some(tag) = vanilla::variant_tag(side, &variant) {
                out.insert(tag.to_string(), frames);
            }
        }
        out
    }

    /// Each weapon with its squad UI text and the loot pools it joins.
    fn weapons(
        &mut self,
        f: &FactionFile,
        side: Side,
    ) -> Vec<(ResolvedWeapon, Option<String>, Vec<String>)> {
        let mut out = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (i, w) in f.weapons.iter().enumerate() {
            let label = match (&w.reskin, &w.tag) {
                (Some(t), _) | (None, Some(t)) => format!("weapons[{i}] ({t})"),
                _ => format!("weapons[{i}]"),
            };
            let (tag, mode, based_on) = match (&w.reskin, &w.tag) {
                (Some(_), Some(_)) => {
                    self.error(format!("{label}: use either reskin or tag, not both."));
                    continue;
                }
                (None, None) => {
                    self.error(format!(
                        "{label}: needs reskin = \"<vanilla tag>\" or tag = \"<new tag>\" with based_on."
                    ));
                    continue;
                }
                (Some(r), None) => {
                    if !vanilla::is_weapon(r) {
                        self.error(format!("{label}: \"{r}\" is not a vanilla weapon tag."));
                        continue;
                    }
                    if w.based_on.is_some() {
                        self.error(format!("{label}: based_on only applies to new weapons."));
                    }
                    if w.loot_side.is_some() || !w.pools.is_empty() {
                        self.error(format!(
                            "{label}: loot_side and pools only apply to new weapons; a reskin keeps its vanilla loot pools."
                        ));
                    }
                    (r.clone(), "reskin", None)
                }
                (None, Some(t)) => {
                    if !is_tag(t) {
                        self.error(format!(
                            "{label}: tag must use only letters, digits and underscores."
                        ));
                        continue;
                    }
                    if vanilla::is_weapon(t) {
                        self.error(format!(
                            "{label}: \"{t}\" is already a vanilla weapon; use reskin = \"{t}\" to change it."
                        ));
                        continue;
                    }
                    let Some(b) = &w.based_on else {
                        self.error(format!(
                            "{label}: a new weapon needs based_on = \"<vanilla tag>\"."
                        ));
                        continue;
                    };
                    if !vanilla::is_weapon(b) {
                        self.error(format!(
                            "{label}: based_on \"{b}\" is not a vanilla weapon tag."
                        ));
                        continue;
                    }
                    (t.clone(), "new", Some(b.clone()))
                }
            };
            if !seen.insert(tag.clone()) {
                self.error(format!("{label}: \"{tag}\" is defined more than once."));
                continue;
            }
            let top = w
                .top_sprite
                .as_ref()
                .and_then(|p| self.file(&format!("{label}.top_sprite"), p, vanilla::IMAGE_EXTS));
            let side_img = w
                .side_sprite
                .as_ref()
                .and_then(|p| self.file(&format!("{label}.side_sprite"), p, vanilla::IMAGE_EXTS));
            if mode == "new" && (w.top_sprite.is_none() || w.side_sprite.is_none()) {
                self.warn(format!(
                    "{label}: without top_sprite and side_sprite it uses the sprites of {}.",
                    based_on.as_deref().unwrap_or("its base weapon")
                ));
            }
            // A bare name (`springfieldSound`) reuses a game sound; anything with a slash or
            // a dot is the faction's own file or numbered set.
            let (sounds, sound_name) = match &w.sound {
                Some(s) if is_identifier(s) && !self.root.join(s).is_dir() => {
                    (Vec::new(), Some(s.clone()))
                }
                Some(s) => (
                    self.numbered(&format!("{label}.sound"), s, vanilla::AUDIO_EXTS),
                    None,
                ),
                None => (Vec::new(), None),
            };
            let (mag_name, mag_file) = match &w.mag_sprite {
                Some(m) if m.contains('/') || m.contains('\\') || m.contains('.') => (
                    None,
                    self.file(&format!("{label}.mag_sprite"), m, vanilla::IMAGE_EXTS),
                ),
                Some(m) if is_identifier(m) => {
                    if !vanilla::is_magazine(m) && !f.sprites.contains_key(m) {
                        self.warn(format!(
                            "{label}.mag_sprite: \"{m}\" is not one of the game's magazines and not in [sprites]. If the game has no sprite of that name, the weapon keeps its base magazine."
                        ));
                    }
                    (Some(m.clone()), None)
                }
                Some(m) => {
                    self.error(format!(
                        "{label}.mag_sprite: \"{m}\" is neither a game sprite name nor an image file."
                    ));
                    (None, None)
                }
                None => (None, None),
            };
            let stats = w.stats.clone().unwrap_or_default();
            self.check_stats(&label, &stats);

            let mut pools: Vec<String> = Vec::new();
            match w.loot_side {
                Some(LootSide::Allied) => pools.push("allied".into()),
                Some(LootSide::Axis) => pools.push("axis".into()),
                _ => {}
            }
            for p in &w.pools {
                if !vanilla::is_pool(p) {
                    self.error(format!("{label}.pools: \"{p}\" is not a weapon pool."));
                } else if !pools.contains(p) {
                    pools.push(p.clone());
                }
            }
            if mode == "new"
                && w.loot_side
                    == Some(match side {
                        Side::Allied => LootSide::Axis,
                        Side::Axis => LootSide::Allied,
                    })
            {
                self.info(format!(
                    "{label}: loot_side puts this weapon in the other side's loot."
                ));
            }

            let text = w.description.clone().or_else(|| w.name.clone());
            out.push((
                ResolvedWeapon {
                    tag,
                    mode: mode.to_string(),
                    based_on,
                    top,
                    side: side_img,
                    sounds,
                    sound_name,
                    mag_name,
                    mag_file,
                    stats,
                },
                text,
                pools,
            ));
        }
        out
    }

    fn check_stats(&mut self, label: &str, s: &WeaponStats) {
        if let Some(a) = &s.action {
            if !["semi", "bolt", "auto"].contains(&a.as_str()) {
                self.error(format!(
                    "{label}.stats.action must be \"semi\", \"bolt\" or \"auto\"."
                ));
            }
        }
        if let Some(c) = &s.class {
            if !["", "smg", "lmg", "atRifle"].contains(&c.as_str()) {
                self.error(format!(
                    "{label}.stats.class must be \"\", \"smg\", \"lmg\" or \"atRifle\"."
                ));
            }
        }
        for (k, v) in [("ammo", s.ammo), ("maxAmmo", s.max_ammo)] {
            if matches!(v, Some(n) if n < 1) {
                self.error(format!("{label}.stats.{k} must be at least 1."));
            }
        }
        if let (Some(a), Some(m)) = (s.ammo, s.max_ammo) {
            if a > m {
                self.warn(format!("{label}.stats: ammo is larger than maxAmmo."));
            }
        }
        if matches!(s.rg_capable, Some(n) if n != 0 && n != 1) {
            self.error(format!("{label}.stats.rgCapable must be 0 or 1."));
        }
        for (k, v) in [
            ("range", s.range),
            ("damage", s.damage),
            ("recoil", s.recoil),
            ("bulk", s.bulk),
            ("rof", s.rof),
            ("rofTime", s.rof_time),
            ("jam", s.jam),
        ] {
            if matches!(v, Some(n) if n < 0.0 || !n.is_finite()) {
                self.error(format!("{label}.stats.{k} can't be negative."));
            }
        }
    }

    fn spawn_swap(
        &mut self,
        f: &FactionFile,
        known: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (from, to) in &f.spawn_swap {
            if !vanilla::is_weapon(from) {
                self.error(format!(
                    "spawn_swap: \"{from}\" is not a vanilla weapon tag."
                ));
                continue;
            }
            if !known.contains(to) {
                self.error(format!(
                    "spawn_swap.{from}: \"{to}\" is neither a vanilla weapon nor one this faction adds."
                ));
                continue;
            }
            if from == to {
                self.warn(format!("spawn_swap.{from} swaps a weapon for itself."));
                continue;
            }
            let checks = vanilla::weapon_checks(from);
            if checks > 0 {
                self.warn(format!(
                    "spawn_swap.{from}: the game checks for \"{from}\" by name in {checks} place(s); those stop matching for swapped units. A reskin keeps them working."
                ));
            }
            out.insert(from.clone(), to.clone());
        }
        out
    }

    fn pools(
        &mut self,
        f: &FactionFile,
        known: &BTreeSet<String>,
    ) -> BTreeMap<String, ResolvedPool> {
        let mut out = BTreeMap::new();
        for (name, edit) in &f.pools {
            if !vanilla::is_pool(name) {
                let all: Vec<&str> = vanilla::POOLS.iter().map(|(p, _)| *p).collect();
                self.error(format!(
                    "pools.{name}: not a weapon pool; use one of: {}.",
                    all.join(", ")
                ));
                continue;
            }
            let mut pool = ResolvedPool::default();
            for t in &edit.add {
                if known.contains(t) {
                    if !pool.add.contains(t) {
                        pool.add.push(t.clone());
                    }
                } else {
                    self.error(format!("pools.{name}.add: \"{t}\" is not a known weapon."));
                }
            }
            for t in &edit.remove {
                if vanilla::is_weapon(t) {
                    if !pool.remove.contains(t) {
                        pool.remove.push(t.clone());
                    }
                } else {
                    self.error(format!(
                        "pools.{name}.remove: \"{t}\" is not a vanilla weapon."
                    ));
                }
            }
            out.insert(name.clone(), pool);
        }
        out
    }

    fn classes(
        &mut self,
        f: &FactionFile,
        side: Side,
        known: &BTreeSet<String>,
        text: &mut BTreeMap<String, String>,
    ) -> BTreeMap<String, ResolvedClass> {
        let mut out = BTreeMap::new();
        if !f.classes.is_empty() && side == Side::Axis {
            self.error("classes: only the Allied side has player classes; remove [classes] or set replaces = \"allied\".");
            return out;
        }
        for (spec, def) in &f.classes {
            let Some((_, mouse_over)) = vanilla::SPECS.iter().find(|(s, _)| s == spec) else {
                let all: Vec<&str> = vanilla::SPECS.iter().map(|(s, _)| *s).collect();
                self.error(format!(
                    "classes.{spec}: not a class slot; use one of: {}.",
                    all.join(", ")
                ));
                continue;
            };
            let label = format!("classes.{spec}");
            let weapon_ok = |cx: &mut Self, what: &str, t: &str| -> bool {
                if known.contains(t) {
                    true
                } else {
                    cx.error(format!("{label}.{what}: \"{t}\" is not a known weapon."));
                    false
                }
            };
            let start_weapon = def
                .start_weapon
                .as_ref()
                .filter(|t| weapon_ok(self, "start_weapon", t))
                .cloned();
            let unlocks = match &def.unlocks {
                Some(u) if u.len() != 3 => {
                    self.error(format!(
                        "{label}.unlocks needs exactly 3 weapons (tiers 1 to 3)."
                    ));
                    None
                }
                Some(u) => {
                    // Check every tier so each unknown weapon is reported, not just the first.
                    let bad = u.iter().filter(|t| !weapon_ok(self, "unlocks", t)).count();
                    (bad == 0).then(|| u.clone())
                }
                None => None,
            };
            if let Some(b) = &def.bonus {
                for (k, v) in [
                    ("detect_range", b.detect_range),
                    ("mine_detect_range", b.mine_detect_range),
                ] {
                    if matches!(v, Some(n) if n <= 0.0 || !n.is_finite()) {
                        self.error(format!(
                            "{label}.bonus.{k} is a multiplier and must be above 0."
                        ));
                    }
                }
            }
            match &def.skills {
                Some(s) if s.len() != 3 => {
                    self.error(format!("{label}.skills needs exactly 3 entries."));
                }
                Some(s) => {
                    for (i, t) in s.iter().enumerate() {
                        text.insert(format!("{spec}_skillText_{}", i + 1), t.clone());
                    }
                }
                None => {}
            }
            match (&def.name, &def.description) {
                (Some(n), Some(d)) => {
                    text.insert(mouse_over.to_string(), format!("{n}, {d}"));
                }
                (Some(n), None) => {
                    // Keep vanilla's wording after the name; the runtime fills it in.
                    text.insert(format!("{mouse_over}::name"), n.clone());
                }
                (None, Some(d)) => {
                    text.insert(format!("{mouse_over}::description"), d.clone());
                }
                (None, None) => {}
            }
            if spec == "commando" {
                // Vanilla has a second, older string for the commando button.
                for (k, v) in text.clone() {
                    if let Some(rest) = k.strip_prefix("mouseOverCommandoSpec") {
                        text.insert(format!("mouseOverSOFSpec{rest}"), v);
                    }
                }
            }
            let icon = def
                .icon
                .as_ref()
                .and_then(|p| self.file(&format!("{label}.icon"), p, vanilla::IMAGE_EXTS));
            out.insert(
                spec.clone(),
                ResolvedClass {
                    start_weapon,
                    unlocks,
                    bonus: def.bonus.clone(),
                    icon,
                },
            );
        }
        out
    }

    fn name_list(
        &mut self,
        label: &str,
        inline: &Option<Vec<String>>,
        file: &Option<String>,
    ) -> Vec<String> {
        match (inline, file) {
            (Some(_), Some(_)) => {
                self.error(format!("{label}: give the list or the file, not both."));
                Vec::new()
            }
            (Some(v), None) => v
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            (None, Some(p)) => self.lines_file(&format!("{label}_file"), p),
            (None, None) => Vec::new(),
        }
    }

    fn name_pool(&mut self, label: &str, p: &NamePool) -> Option<ResolvedNamePool> {
        let first = self.name_list(&format!("{label}.first"), &p.first, &p.first_file);
        let last = self.name_list(&format!("{label}.last"), &p.last, &p.last_file);
        let format = p.format.unwrap_or_default();
        // A missing or unreadable file is already reported; don't pile on.
        let last_given = p.last.is_some() || p.last_file.is_some();
        let first_given = p.first.is_some() || p.first_file.is_some();
        if last.is_empty() {
            if !last_given && (!first.is_empty() || p.format.is_some()) {
                self.error(format!("{label}: needs last names (last or last_file)."));
            }
            return None;
        }
        if format == NameFormat::Full && first.is_empty() {
            if !first_given {
                self.error(format!(
                    "{label}: format = \"full\" needs first names (first or first_file)."
                ));
            }
            return None;
        }
        if format == NameFormat::Initial && !first.is_empty() {
            self.info(format!(
                "{label}: first names are only used with format = \"full\"; vanilla style is an initial."
            ));
        }
        Some(ResolvedNamePool {
            first,
            last,
            format,
        })
    }

    fn names(&mut self, f: &FactionFile, side: Side) -> Option<ResolvedNames> {
        let n = f.names.as_ref()?;
        if side == Side::Axis
            && (n.french.is_some() || n.vehicles.is_some() || n.vehicles_file.is_some())
        {
            self.warn("names: on the axis side only the soldier names are used (for the German commander); french and vehicles are ignored.");
        }
        let soldiers = self.name_pool(
            "names",
            &NamePool {
                first: n.first.clone(),
                last: n.last.clone(),
                first_file: n.first_file.clone(),
                last_file: n.last_file.clone(),
                format: n.format,
            },
        );
        let french = n
            .french
            .as_ref()
            .and_then(|p| self.name_pool("names.french", p));
        let vehicles = self.name_list("names.vehicles", &n.vehicles, &n.vehicles_file);
        Some(ResolvedNames {
            soldiers,
            french,
            vehicles,
        })
    }

    fn voices(&mut self, f: &FactionFile, side: Side) -> Vec<ResolvedVoiceDir> {
        let Some(v) = &f.voices else {
            return Vec::new();
        };
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for (from, to) in &v.map {
            if is_identifier(from) && is_identifier(to) {
                map.insert(from.to_ascii_lowercase(), to.clone());
            } else {
                self.error(format!("voices.map: \"{from}\" = \"{to}\" must both be plain file names without extensions."));
            }
        }
        let mut dirs: Vec<(String, String)> = Vec::new();
        if let Some(folder) = &v.folder {
            dirs.push((side.as_str().to_string(), folder.clone()));
        }
        for (vanilla_dir, folder) in &v.dirs {
            if !vanilla::voice_dirs(side).contains(&vanilla_dir.as_str()) {
                self.error(format!(
                    "voices.dirs: the {} have no \"{vanilla_dir}\" voice folder; use one of: {}.",
                    side.label(),
                    vanilla::voice_dirs(side).join(", ")
                ));
                continue;
            }
            if dirs.iter().any(|(d, _)| d == vanilla_dir) {
                self.error(format!("voices: \"{vanilla_dir}\" is given twice."));
                continue;
            }
            dirs.push((vanilla_dir.clone(), folder.clone()));
        }
        if dirs.is_empty() {
            self.error("voices: needs folder (and optionally dirs).");
        }
        let fill = v.fill_missing.unwrap_or(true);
        let mut out = Vec::new();
        for (vanilla_dir, folder) in dirs {
            let label = if vanilla_dir == side.as_str() {
                "voices.folder".to_string()
            } else {
                format!("voices.dirs.{vanilla_dir}")
            };
            let Some(dir) = self.folder(&label, &folder) else {
                continue;
            };
            let mut exact = BTreeMap::new();
            let mut numbered: BTreeMap<String, Vec<(u32, String)>> = BTreeMap::new();
            for (name, path) in self.list_dir(&dir) {
                if !vanilla::AUDIO_EXTS.contains(&ext_of(&name).as_str()) {
                    continue;
                }
                let stem = stem_of(&name);
                let renamed = match map.get(&stem.to_ascii_lowercase()) {
                    Some(to) => to.clone(),
                    None => match split_number(stem) {
                        Some((pfx, n)) => match map.get(&pfx.to_ascii_lowercase()) {
                            Some(to) => format!("{to}{n}"),
                            None => stem.to_string(),
                        },
                        None => stem.to_string(),
                    },
                };
                if let Some((pfx, n)) = split_number(&renamed) {
                    numbered
                        .entry(pfx.to_string())
                        .or_default()
                        .push((n, path.clone()));
                }
                exact.insert(renamed, path);
            }
            if exact.is_empty() {
                self.warn(format!("{label}: no sound files in \"{dir}\"."));
            }
            let pools = numbered
                .into_iter()
                .map(|(k, mut v)| {
                    v.sort();
                    (k, v.into_iter().map(|(_, p)| p).collect())
                })
                .collect();
            out.push(ResolvedVoiceDir {
                vanilla_dir: format!("sounds/vo/{vanilla_dir}"),
                fill_missing: fill,
                exact,
                pools,
            });
        }
        out
    }

    fn globals_map(
        &mut self,
        section: &str,
        map: &BTreeMap<String, String>,
        exts: &[&str],
    ) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (global, raw) in map {
            if !is_identifier(global) {
                self.error(format!(
                    "{section}: \"{global}\" is not a valid game global name."
                ));
                continue;
            }
            if let Some(p) = self.file(&format!("{section}.{global}"), raw, exts) {
                out.insert(global.clone(), p);
            }
        }
        out
    }

    /// `[sprites]`: the image files, and the sizes and places of the entries that give one.
    fn sprites(
        &mut self,
        map: &BTreeMap<String, SpriteEntry>,
    ) -> (BTreeMap<String, String>, BTreeMap<String, ResolvedSpriteSize>) {
        let mut files = BTreeMap::new();
        let mut sizes = BTreeMap::new();
        for (global, entry) in map {
            if !is_identifier(global) {
                self.error(format!("sprites: \"{global}\" is not a valid game global name."));
                continue;
            }
            let label = format!("sprites.{global}");
            let file = match entry.file() {
                Some(raw) => match self.file(&label, raw, vanilla::IMAGE_EXTS) {
                    Some(p) => Some(p),
                    None => continue,
                },
                None => None,
            };
            if let SpriteEntry::Table(t) = entry {
                let set = usize::from(t.size.is_some()) + usize::from(t.scale.is_some()) + usize::from(t.like.is_some());
                if set > 1 {
                    self.error(format!("{label}: give one of size, scale and like."));
                    continue;
                }
                let placed = t.offset.is_some() || t.rotate.is_some() || t.flip.is_some() || t.draw_size.is_some();
                if set == 0 && !placed && file.is_none() {
                    self.error(format!("{label}: needs a file, a size or a place (offset, rotate, flip, draw_size)."));
                    continue;
                }
                let good = |n: f64| n.is_finite() && n > 0.0;
                if t.size.is_some_and(|[w, h]| !good(w) || !good(h))
                    || t.scale.is_some_and(|k| !good(k))
                    || t.draw_size.is_some_and(|[w, h]| !good(w) || !good(h))
                {
                    self.error(format!("{label}: sizes and scales must be more than 0."));
                    continue;
                }
                if let Some(like) = &t.like {
                    if !is_identifier(like) || like == global {
                        self.error(format!("{label}: like must name another image."));
                        continue;
                    }
                }
                if t.offset.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite())
                    || t.rotate.is_some_and(|r| !r.is_finite())
                {
                    self.error(format!("{label}: offset and rotate must be numbers."));
                    continue;
                }
                if let Some(flip) = &t.flip {
                    if !SPRITE_FLIPS.contains(&flip.as_str()) {
                        self.error(format!("{label}: flip is \"x\", \"y\" or \"xy\", not \"{flip}\"."));
                        continue;
                    }
                }
                if set == 1 || placed {
                    sizes.insert(
                        global.clone(),
                        ResolvedSpriteSize {
                            size: t.size,
                            scale: t.scale,
                            like: t.like.clone(),
                            offset: t.offset,
                            rotate: t.rotate,
                            flip: t.flip.clone(),
                            draw_size: t.draw_size,
                        },
                    );
                }
            }
            if let Some(p) = file {
                files.insert(global.clone(), p);
            }
        }
        (files, sizes)
    }

    /// `[ranks]`: up to four names per list, none empty.
    fn ranks(&mut self, r: Option<&RanksDef>, side: Side) -> Option<ResolvedRanks> {
        let r = r?;
        if r.sarge.is_some() && side == Side::Axis {
            self.warn("ranks.sarge: the Sarge is the Allies'; on the Axis side it is never used.".to_string());
        }
        let mut bad = false;
        for (key, list) in [("soldier", &r.soldier), ("officer", &r.officer)] {
            if let Some(list) = list {
                if list.len() > 4 {
                    self.error(format!(
                        "ranks.{key}: the game has four ranks (0 to 3); this list has {}.",
                        list.len()
                    ));
                    bad = true;
                }
                if list.iter().any(|n| n.trim().is_empty()) {
                    self.error(format!("ranks.{key}: a rank name can't be empty."));
                    bad = true;
                }
            }
        }
        for (key, name) in [("recruit", &r.recruit), ("cadet", &r.cadet), ("sarge", &r.sarge)] {
            if name.as_deref().is_some_and(|n| n.trim().is_empty()) {
                self.error(format!("ranks.{key}: a rank name can't be empty."));
                bad = true;
            }
        }
        if bad {
            return None;
        }
        Some(ResolvedRanks {
            soldier: r.soldier.clone().unwrap_or_default(),
            officer: r.officer.clone().unwrap_or_default(),
            recruit: r.recruit.clone(),
            cadet: r.cadet.clone(),
            sarge: if side == Side::Allied { r.sarge.clone() } else { None },
        })
    }

    fn vehicles(&mut self, f: &FactionFile, side: Side) -> Vec<ResolvedVehicle> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for (i, v) in f.vehicles.iter().enumerate() {
            let tag = &v.reskin;
            let label = format!("vehicles[{i}] ({tag})");
            if !vanilla::vehicles(side).contains(&tag.as_str()) {
                if vanilla::vehicles(side.other()).contains(&tag.as_str()) {
                    self.error(format!(
                        "{label}: \"{tag}\" belongs to the {}; a faction can only reskin its own side's vehicles.",
                        side.other().label()
                    ));
                } else {
                    self.error(format!("{label}: \"{tag}\" is not a vanilla vehicle."));
                }
                continue;
            }
            if !seen.insert(tag.clone()) {
                self.error(format!("{label}: \"{tag}\" is reskinned more than once."));
                continue;
            }
            let mut parts = BTreeMap::new();
            let named = [
                ("body", &v.body),
                ("turret", &v.turret),
                ("driver", &v.driver),
                ("gunner", &v.gunner),
                ("icon", &v.icon),
                ("husk", &v.husk),
            ];
            for (key, val) in named {
                let Some(raw) = val else { continue };
                let suffix = vanilla::VEHICLE_PARTS
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, s)| *s)
                    .unwrap_or("");
                if let Some(p) = self.file(&format!("{label}.{key}"), raw, vanilla::IMAGE_EXTS) {
                    parts.insert(format!("{tag}{suffix}"), p);
                }
            }
            for (suffix, raw) in &v.parts {
                if !suffix
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    || suffix.is_empty()
                {
                    self.error(format!(
                        "{label}.parts: \"{suffix}\" is not a valid part name."
                    ));
                    continue;
                }
                if let Some(p) =
                    self.file(&format!("{label}.parts.{suffix}"), raw, vanilla::IMAGE_EXTS)
                {
                    parts.insert(format!("{tag}{suffix}"), p);
                }
            }
            let debris = match &v.debris {
                Some(d) => self.numbered(&format!("{label}.debris"), d, vanilla::IMAGE_EXTS),
                None => Vec::new(),
            };
            let call_signs = self.name_list(
                &format!("{label}.call_signs"),
                &v.call_signs,
                &v.call_signs_file,
            );
            let spawner = if call_signs.is_empty() {
                None
            } else {
                match vanilla::vehicle_spawner(tag) {
                    Some(f) => Some(f.to_string()),
                    None => {
                        self.warn(format!("{label}: call signs only work for Allied vehicles that have them; \"{tag}\" has none."));
                        None
                    }
                }
            };
            if parts.is_empty() && debris.is_empty() && v.name.is_none() && call_signs.is_empty() {
                self.warn(format!(
                    "{label}: sets nothing, so this vehicle stays vanilla."
                ));
            }
            out.push(ResolvedVehicle {
                tag: tag.clone(),
                name: v.name.clone(),
                parts,
                debris,
                call_signs,
                spawner,
            });
        }
        out
    }
}

/// Picks one faction per side from `(priority, mod id, faction)` candidates: the highest
/// priority wins, ties go to the mod id that sorts last (it would also load last).
/// Returns the winners and a message for every faction left out.
pub fn pick_active<T: Clone>(
    candidates: &[(i32, String, Side, T)],
) -> (Vec<(String, T)>, Vec<String>) {
    let mut winners: BTreeMap<Side, (i32, String, T)> = BTreeMap::new();
    for (prio, mod_id, side, item) in candidates {
        let better = match winners.get(side) {
            Some((p, m, _)) => (*prio, mod_id) >= (*p, m),
            None => true,
        };
        if better {
            winners.insert(*side, (*prio, mod_id.clone(), item.clone()));
        }
    }
    let mut skipped = Vec::new();
    for (_, mod_id, side, _) in candidates {
        if let Some((_, w, _)) = winners.get(side) {
            if w != mod_id {
                skipped.push(format!(
                    "Faction mod '{mod_id}' also replaces the {}; only '{w}' is used. Enable one faction per side.",
                    side.label()
                ));
            }
        }
    }
    (
        winners.into_values().map(|(_, m, t)| (m, t)).collect(),
        skipped,
    )
}

#[cfg(test)]
mod tests;
