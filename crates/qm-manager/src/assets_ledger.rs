//! The asset ledger (Phase E D76, D77, D80): every sound, image or font that more than one
//! source claims, grouped, with today's winner.
//!
//! Sources are the game itself, each enabled mod's `assets/` files and each active faction.
//! A *slot* is one file the game loads, named by its lowercase `folder/stem`. A *group* is a
//! numbered set inside one folder (`sounds/vo/axis/axishurt#`) or, for everything else, the
//! folder. Without a choice the last source to load a file wins it: mods in the order their
//! asset blocks run, then factions. A choice names a group or a slot and its winner.
//!
//! The slot of a mod's file is worked out the way the autoloader does it in the game: the
//! same folder and name first, then the same name in any folder. That is a prediction; the
//! game reports what it really used, and the ledger shows both when they differ.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use qm_core::patch::faction::LoadedFaction;
use qm_core::patch::loader::{auto_asset_manifest, auto_asset_run_order, AutoAssetManifest};
use qm_core::patch::table::PatchTable;
use serde::{Deserialize, Serialize};

use crate::vanilla_assets::{cat_stem_of, numbered, png_size_of_file, VanillaAssetListing};

/// The winner name that means "the game's own file".
pub const VANILLA: &str = "vanilla";

/// How a faction is named as a source.
pub fn faction_source(id: &str) -> String {
    format!("faction:{id}")
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Sound,
    Image,
    Font,
    Text,
}

impl AssetKind {
    pub fn of_ext(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "wav" | "mp3" | "ogg" => Some(Self::Sound),
            "png" | "jpg" | "jpeg" | "bmp" => Some(Self::Image),
            "ttf" | "otf" => Some(Self::Font),
            "txt" => Some(Self::Text),
            _ => None,
        }
    }

    fn of_path(path: &str) -> Option<Self> {
        Self::of_ext(path.rsplit('.').next().unwrap_or(""))
    }
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// A numbered set in one folder.
    Set,
    /// Everything else in one folder.
    Folder,
}

/// A plain-text note on a group or a slot. `code` is stable; `text` is for the user.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerNote {
    pub code: String,
    pub text: String,
}

fn note(code: &str, text: String) -> LedgerNote {
    LedgerNote {
        code: code.to_string(),
        text,
    }
}

/// The game's own file in a slot.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub height: Option<u32>,
}

/// One source's file for a slot.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    /// A mod id, or `faction:<id>`.
    pub source: String,
    /// The file inside the mod: `assets/...` for a mod, the faction's own path for a faction.
    pub file: String,
    /// For a faction: the game global the file is set on (`allied_idle`), or the voice file
    /// it replaces (`sounds/vo/allied/contact1`). It is how the plan tells the faction to
    /// leave the file alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub height: Option<u32>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetSlot {
    /// Lowercase `folder/stem`.
    pub key: String,
    /// The file name without its extension, as the game or the first claim writes it.
    pub name: String,
    pub kind: AssetKind,
    /// The number of a member of a numbered set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub game: Option<GameFile>,
    /// In the order the game loads them; without a choice the last one wins.
    pub claims: Vec<Claim>,
    /// The choice made for this slot alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub choice: Option<String>,
    /// Who wins with the choices as they are: a source, or `vanilla`.
    pub winner: String,
    /// What the game used in the profile's last run, when it reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub last_run_winner: Option<String>,
    pub notes: Vec<LedgerNote>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupClaimant {
    pub source: String,
    /// How many slots of the group it has a file for.
    pub files: u32,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetGroup {
    /// The set key (`sounds/vo/axis/axishurt#`) or the folder (`sprites`).
    pub key: String,
    pub kind: GroupKind,
    /// The key with the game's spelling (`sounds/vo/axis/axisHurt#`).
    pub label: String,
    /// How many files the game has in the group (all members of a set; for a folder, the
    /// listed slots the game has).
    pub game_count: u32,
    /// In load order; without a choice the last one wins a slot both have.
    pub claimants: Vec<GroupClaimant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "specta", specta(optional))]
    pub choice: Option<String>,
    pub slots: Vec<AssetSlot>,
    pub notes: Vec<LedgerNote>,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetLedger {
    /// False when the game's file list could not be read: then only files two sources both
    /// ship are listed, and nothing is known about the game's own files.
    pub has_game_files: bool,
    pub groups: Vec<AssetGroup>,
}

/// What the ledger is built from.
pub struct LedgerInput<'a> {
    /// The enabled mods' asset manifests, in the order the game runs them.
    pub manifests: &'a [AutoAssetManifest],
    /// The active factions; they load after every mod.
    pub factions: &'a [LoadedFaction],
    pub listing: Option<&'a VanillaAssetListing>,
    /// Group or slot key -> winner.
    pub choices: &'a BTreeMap<String, String>,
    /// Slot key -> the source the game used in the last run.
    pub last_run: &'a BTreeMap<String, String>,
}

/// The asset manifests of `table`'s mods in the order the game runs them: the order of
/// their blocks in the patched `main.lua` when it is known, else the table's order.
pub fn manifests_in_run_order(table: &PatchTable, main_lua: Option<&str>) -> Vec<AutoAssetManifest> {
    let mut all: Vec<AutoAssetManifest> = table
        .patches
        .iter()
        .filter_map(|(p, _, _)| auto_asset_manifest(p))
        .collect();
    let Some(text) = main_lua else {
        return all;
    };
    let mut out = Vec::with_capacity(all.len());
    for id in auto_asset_run_order(text) {
        if let Some(i) = all.iter().position(|m| m.mod_id == id) {
            out.push(all.remove(i));
        }
    }
    // A block that did not land (its anchor is gone) loads nothing in the game.
    out
}

/// Every mod's asset manifest among `patches` (the raw load, before a plan left files out),
/// in the order the game runs them: the mods whose blocks are in the patched `main.lua` in
/// that order. A mod with no block there (a choice left out all its files, so its order is
/// not known) goes first, where it wins nothing by default.
pub fn all_manifests_in_run_order<'a>(
    patches: impl Iterator<Item = &'a qm_core::patch::Patch>,
    main_lua: Option<&str>,
) -> Vec<AutoAssetManifest> {
    let mut all: Vec<AutoAssetManifest> = patches.filter_map(auto_asset_manifest).collect();
    let Some(text) = main_lua else {
        return all;
    };
    let mut ordered = Vec::with_capacity(all.len());
    for id in auto_asset_run_order(text) {
        if let Some(i) = all.iter().position(|m| m.mod_id == id) {
            ordered.push(all.remove(i));
        }
    }
    all.extend(ordered);
    all
}

struct RawClaim {
    source: String,
    file: String,
    target: Option<String>,
    size: Option<(u32, u32)>,
    /// The name as the source writes it.
    name: String,
    /// How many game files the file was matched to by its name (0: its own folder and name).
    by_name: usize,
}

struct SlotAcc {
    kind: AssetKind,
    claims: Vec<RawClaim>,
}

/// The slots a file with this folder, stem and kind fills, the way the autoloader resolves
/// it: the same folder and name, else the same name in any folder, else a slot of its own.
fn slots_for(
    listing: Option<&VanillaAssetListing>,
    folder: &str,
    stem: &str,
    kind: AssetKind,
) -> (Vec<String>, bool) {
    let stem_l = stem.to_ascii_lowercase();
    let own = if folder.is_empty() {
        stem_l.clone()
    } else {
        format!("{}/{stem_l}", folder.to_ascii_lowercase())
    };
    let Some(listing) = listing else {
        return (vec![own], false);
    };
    if listing
        .get(&own)
        .is_some_and(|a| AssetKind::of_path(&a.path) == Some(kind))
    {
        return (vec![own], false);
    }
    let by_name: Vec<String> = listing
        .with_stem(&stem_l)
        .filter(|a| AssetKind::of_path(&a.path) == Some(kind))
        .map(|a| a.cat_stem())
        .collect();
    if by_name.is_empty() {
        (vec![own], false)
    } else {
        (by_name, true)
    }
}

fn stem_of(path: &str) -> &str {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

fn folder_of(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |(f, _)| f)
}

/// Builds the ledger. Only groups where two sources meet are listed (a mod replacing a game
/// file counts: the game is a source).
pub fn build(input: &LedgerInput<'_>) -> AssetLedger {
    let listing = input.listing;
    let mut slots: BTreeMap<String, SlotAcc> = BTreeMap::new();
    #[allow(clippy::too_many_arguments)]
    let mut add = |keys: Vec<String>,
                   by_name: bool,
                   kind: AssetKind,
                   source: &str,
                   file: &str,
                   name: &str,
                   size: Option<(u32, u32)>,
                   target: Option<&str>| {
        let by_name = if by_name { keys.len() } else { 0 };
        for key in keys {
            let acc = slots.entry(key).or_insert_with(|| SlotAcc {
                kind,
                claims: Vec::new(),
            });
            // One file per source and slot: a later one of the same source replaces it.
            acc.claims.retain(|c| c.source != source);
            acc.claims.push(RawClaim {
                source: source.to_string(),
                file: file.to_string(),
                target: target.map(str::to_string),
                size,
                name: name.to_string(),
                by_name,
            });
        }
    };

    for m in input.manifests {
        for a in &m.assets {
            let Some(kind) = AssetKind::of_ext(&a.ext) else {
                continue;
            };
            let (keys, by_name) = if kind == AssetKind::Text {
                (vec![format!("text/{}", a.stem.to_ascii_lowercase())], false)
            } else {
                slots_for(listing, &a.category, &a.stem, kind)
            };
            let size = (a.ext == "png")
                .then(|| png_size_of_file(&Path::new(&m.dir).join(&a.rel_path)))
                .flatten();
            add(keys, by_name, kind, &m.mod_id, &a.rel_path, &a.stem, size, None);
        }
    }

    for lf in input.factions {
        let source = faction_source(&lf.faction.id);
        // An image set on the game global `name`.
        let mut image = |name: &str, file: &str| {
            let (keys, by_name) = slots_for(listing, "sprites", name, AssetKind::Image);
            let size = png_size_of_file(&lf.root.join(file));
            add(keys, by_name, AssetKind::Image, &source, file, name, size, Some(name));
        };
        for (tag, frames) in &lf.faction.frames {
            for (frame, file) in frames {
                image(&format!("{tag}_{frame}"), file);
            }
        }
        for w in &lf.faction.weapons {
            if let Some(file) = &w.top {
                image(&format!("top_{}", w.tag), file);
            }
            if let Some(file) = &w.side {
                image(&format!("side_{}", w.tag), file);
            }
        }
        for (global, file) in &lf.faction.sprites {
            image(global, file);
        }
        for v in &lf.faction.vehicles {
            for (global, file) in &v.parts {
                image(global, file);
            }
            // The runtime spreads the debris images over the game's debris frames, in order.
            if v.debris.is_empty() {
                continue;
            }
            let game_frames = listing.map_or(0, |l| {
                (1usize..)
                    .take_while(|i| {
                        l.with_stem(&format!("{}_debris{i}", v.tag).to_ascii_lowercase())
                            .next()
                            .is_some()
                    })
                    .count()
            });
            let frames = if game_frames == 0 { v.debris.len() } else { game_frames };
            for i in 1..=frames {
                if let Some(file) = v.debris.get((i - 1) % v.debris.len()) {
                    image(&format!("{}_debris{i}", v.tag), file);
                }
            }
        }
        for (global, file) in &lf.faction.sounds {
            // The game's sound globals are the file name plus `Sound` or `Music`.
            let base = global
                .strip_suffix("Sound")
                .or_else(|| global.strip_suffix("Music"))
                .unwrap_or(global);
            let (keys, by_name) = slots_for(listing, "sounds", base, AssetKind::Sound);
            add(keys, by_name, AssetKind::Sound, &source, file, base, None, Some(global));
        }
        // Voices replace the files of a game voice folder: the exact names, and with
        // `fill_missing` every other numbered line from the faction's own lines in turn.
        for dir in &lf.faction.voices {
            let folder = dir.vanilla_dir.trim_matches('/');
            let mut voice = |stem: &str, file: &str| {
                let key = format!("{folder}/{stem}");
                add(
                    vec![key.to_ascii_lowercase()],
                    false,
                    AssetKind::Sound,
                    &source,
                    file,
                    stem,
                    None,
                    Some(&key),
                );
            };
            let Some(listing) = listing else {
                // Without the game's file list: the exact names only.
                for (stem, file) in &dir.exact {
                    voice(stem, file);
                }
                continue;
            };
            let prefix = format!("{folder}/");
            for a in listing.assets() {
                let Some(rest) = a.path.strip_prefix(&prefix) else {
                    continue;
                };
                if rest.contains('/') || AssetKind::of_path(rest) != Some(AssetKind::Sound) {
                    continue;
                }
                let stem = stem_of(rest);
                if let Some(file) = dir.exact.get(stem) {
                    voice(stem, file);
                } else if dir.fill_missing {
                    let digits = stem.bytes().rev().take_while(u8::is_ascii_digit).count();
                    let (name, num) = stem.split_at(stem.len() - digits);
                    let pool = dir.pools.get(name).filter(|p| !p.is_empty() && !name.is_empty());
                    if let (Some(pool), Ok(n)) = (pool, num.parse::<usize>()) {
                        if let Some(file) = pool.get(n.saturating_sub(1) % pool.len()) {
                            voice(stem, file);
                        }
                    }
                }
            }
        }
    }

    // Numbered sets with two or more members are their own group.
    let mut set_members: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for key in slots.keys() {
        if let Some((set, n)) = numbered(key) {
            set_members.entry(set).or_default().insert(n);
        }
    }
    let group_of = |key: &str| -> (String, GroupKind, Option<u32>) {
        if let Some((set, n)) = numbered(key) {
            let game = listing.map_or(0, |l| l.set_count(&set));
            let claimed = set_members.get(&set).map_or(0, BTreeSet::len);
            if game >= 2 || claimed >= 2 {
                return (set, GroupKind::Set, Some(n));
            }
        }
        (folder_of(key).to_string(), GroupKind::Folder, None)
    };

    let mut groups: BTreeMap<String, AssetGroup> = BTreeMap::new();
    for (key, acc) in slots {
        let (gkey, gkind, number) = group_of(&key);
        let game = listing.and_then(|l| l.get(&key)).map(|a| GameFile {
            path: a.path.clone(),
            width: a.width,
            height: a.height,
        });
        let name = game
            .as_ref()
            .map(|g| stem_of(&g.path).to_string())
            .or_else(|| acc.claims.first().map(|c| c.name.clone()))
            .unwrap_or_default();
        let mut notes = Vec::new();
        if let Some(g) = &game {
            for c in acc.claims.iter().filter(|c| c.by_name > 1) {
                notes.push(note(
                    "matches_several",
                    format!(
                        "{}'s {} has the same name as {} of the game's files and replaces all of them, {} included.",
                        c.source, c.file, c.by_name, g.path
                    ),
                ));
            }
            if acc.kind == AssetKind::Font {
                notes.push(note(
                    "font_all_sizes",
                    "The game makes every size of this font from one file, so a replacement changes all of them.".to_string(),
                ));
            }
            if let (Some(gw), Some(gh)) = (g.width, g.height) {
                for c in &acc.claims {
                    if let Some((w, h)) = c.size {
                        if (w, h) != (gw, gh) {
                            notes.push(note(
                                "asset_size_differs",
                                format!(
                                    "{}'s image is {w}x{h}; the game's is {gw}x{gh}. Code that draws it at fixed offsets may be off.",
                                    c.source
                                ),
                            ));
                        }
                    }
                }
            }
        }
        let claims = acc
            .claims
            .into_iter()
            .map(|c| Claim {
                source: c.source,
                file: c.file,
                target: c.target,
                width: c.size.map(|s| s.0),
                height: c.size.map(|s| s.1),
            })
            .collect();
        let group = groups.entry(gkey.clone()).or_insert_with(|| AssetGroup {
            key: gkey.clone(),
            kind: gkind,
            label: gkey.clone(),
            game_count: 0,
            claimants: Vec::new(),
            choice: input.choices.get(&gkey).cloned(),
            slots: Vec::new(),
            notes: Vec::new(),
        });
        group.slots.push(AssetSlot {
            choice: input.choices.get(&key).cloned(),
            last_run_winner: input.last_run.get(&key).cloned(),
            key,
            name,
            kind: acc.kind,
            number,
            game,
            claims,
            winner: String::new(),
            notes,
        });
    }

    let source_order = source_order(input);
    let mut out = Vec::new();
    for (_, mut g) in groups {
        // A folder group lists only the slots where two sources meet; a set lists them all.
        let overlaps = |s: &AssetSlot| s.game.is_some() || s.claims.len() >= 2;
        if !g.slots.iter().any(overlaps) {
            continue;
        }
        if g.kind == GroupKind::Folder {
            g.slots.retain(overlaps);
        }
        g.slots.sort_by(|a, b| (a.number, &a.key).cmp(&(b.number, &b.key)));
        finish_group(&mut g, listing, &source_order);
        out.push(g);
    }
    AssetLedger {
        has_game_files: listing.is_some(),
        groups: out,
    }
}

/// Every source in load order: the mods as their blocks run, then the factions.
fn source_order(input: &LedgerInput<'_>) -> Vec<String> {
    input
        .manifests
        .iter()
        .map(|m| m.mod_id.clone())
        .chain(input.factions.iter().map(|f| faction_source(&f.faction.id)))
        .collect()
}

fn finish_group(g: &mut AssetGroup, listing: Option<&VanillaAssetListing>, order: &[String]) {
    g.game_count = match g.kind {
        GroupKind::Set => listing.map_or(0, |l| l.set_count(&g.key)),
        GroupKind::Folder => g.slots.iter().filter(|s| s.game.is_some()).count() as u32,
    };
    // The label keeps the game's spelling of the set or folder.
    if let Some(game) = g.slots.iter().find_map(|s| s.game.as_ref()) {
        let folder = game.path.rsplit_once('/').map_or("", |(f, _)| f);
        g.label = match g.kind {
            GroupKind::Folder => folder.to_string(),
            GroupKind::Set => {
                let stem = stem_of(&game.path).trim_end_matches(|c: char| c.is_ascii_digit());
                if folder.is_empty() {
                    format!("{stem}#")
                } else {
                    format!("{folder}/{stem}#")
                }
            }
        };
    }

    let mut files: BTreeMap<&str, u32> = BTreeMap::new();
    for s in &g.slots {
        for c in &s.claims {
            *files.entry(c.source.as_str()).or_default() += 1;
        }
    }
    g.claimants = order
        .iter()
        .filter_map(|src| {
            files.get(src.as_str()).map(|n| GroupClaimant {
                source: src.clone(),
                files: *n,
            })
        })
        .collect();

    if g.kind == GroupKind::Set && g.game_count > 0 {
        for c in &g.claimants {
            let in_game = g
                .slots
                .iter()
                .filter(|s| s.game.is_some() && s.claims.iter().any(|x| x.source == c.source))
                .count() as u32;
            if in_game > 0 && in_game < g.game_count {
                g.notes.push(note(
                    "only_some_of_set",
                    format!(
                        "{} has {in_game} of the game's {}; it has no file for the other {}.",
                        c.source,
                        g.game_count,
                        g.game_count - in_game
                    ),
                ));
            }
        }
    }

    let group_choice = g.choice.clone();
    let (kind, game_count) = (g.kind, g.game_count);
    for s in &mut g.slots {
        if kind == GroupKind::Set && game_count > 0 && s.game.is_none() {
            if let Some(n) = s.number {
                s.notes.push(note(
                    "added_number",
                    format!(
                        "Number {n} is not one of the game's {game_count}. It is an addition; nothing is renumbered."
                    ),
                ));
            }
        }
        let default = s
            .claims
            .last()
            .map_or_else(|| VANILLA.to_string(), |c| c.source.clone());
        let choice = s.choice.clone().or_else(|| group_choice.clone());
        s.winner = match choice.as_deref() {
            None => default,
            Some(VANILLA) => VANILLA.to_string(),
            Some(src) if s.claims.iter().any(|c| c.source == src) => src.to_string(),
            Some(src) => {
                s.notes.push(note(
                    "filled_from",
                    format!("{src} has no file here, so {} is used.", display_source(&default)),
                ));
                default
            }
        };
        if let Some(last) = &s.last_run_winner {
            if *last != s.winner {
                s.notes.push(note(
                    "last_run_differs",
                    format!("In the last run the game used {}.", display_source(last)),
                ));
            }
        }
    }
}

fn display_source(source: &str) -> String {
    if source == VANILLA {
        "the game's own file".to_string()
    } else {
        source.to_string()
    }
}

/// What the ledger's choices leave out, for the plan's `asset_exclusions` (D78): a mod id ->
/// the `assets/...` files that are not loaded, and `faction:<id>` -> the game globals and
/// voice files (`Claim::target`) that faction leaves alone. A slot without a choice, or whose chosen source has no file for
/// it, leaves nothing out. Lists are sorted and without repeats.
pub fn exclusions(ledger: &AssetLedger) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for per_source in exclusions_by_key(ledger).into_values() {
        for (source, files) in per_source {
            out.entry(source).or_default().extend(files);
        }
    }
    out.into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

/// `exclusions`, split by the choice that causes them: choice key (a group's or a slot's)
/// -> source -> what it leaves out. A slot with its own choice counts under the slot's key,
/// not its group's. A key whose choice leaves nothing out is not listed.
pub fn exclusions_by_key(ledger: &AssetLedger) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
    let mut out: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for g in &ledger.groups {
        for s in &g.slots {
            let (key, choice) = match (&s.choice, &g.choice) {
                (Some(c), _) => (&s.key, c.as_str()),
                (None, Some(c)) => (&g.key, c.as_str()),
                (None, None) => continue,
            };
            let decided = choice == VANILLA || s.claims.iter().any(|c| c.source == choice);
            if !decided {
                continue;
            }
            for c in s.claims.iter().filter(|c| c.source != choice) {
                // A mod is told by file, a faction by the global or voice file it sets.
                let what = c.target.clone().unwrap_or_else(|| c.file.clone());
                out.entry(key.clone())
                    .or_default()
                    .entry(c.source.clone())
                    .or_default()
                    .insert(what);
            }
        }
    }
    out.into_iter()
        .map(|(k, v)| {
            (
                k,
                v.into_iter().map(|(s, f)| (s, f.into_iter().collect())).collect(),
            )
        })
        .collect()
}

/// What a choice of `winner` for `key` would leave out, with the ledger's other choices as
/// they are.
pub fn exclusions_of(ledger: &AssetLedger, key: &str) -> BTreeMap<String, Vec<String>> {
    exclusions_by_key(ledger).remove(key).unwrap_or_default()
}

impl AssetLedger {
    pub fn group(&self, key: &str) -> Option<&AssetGroup> {
        self.groups.iter().find(|g| g.key == key)
    }

    pub fn slot(&self, key: &str) -> Option<&AssetSlot> {
        self.groups
            .iter()
            .flat_map(|g| &g.slots)
            .find(|s| s.key == key)
    }

    /// Whether `key` names a group or a slot of this ledger (a choice for any other key
    /// does nothing).
    pub fn has_key(&self, key: &str) -> bool {
        self.group(key).is_some() || self.slot(key).is_some()
    }
}

/// Lowercase slot key of a path inside the game archive or a mod's `assets/` folder.
pub fn slot_key_of_path(path: &str) -> String {
    cat_stem_of(path)
}

#[cfg(test)]
#[path = "assets_ledger_tests.rs"]
mod tests;
