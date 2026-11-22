//! The Mod Engine's sprite line-up (2.2.0): the images an artist can pick (the game's, the
//! mod's, other mods'), their pixels for the preview, where the game shows a file of the mod,
//! and saving the place the artist lined it up at into `qm.toml` (`[sprites.<global>]`) or
//! `faction.toml` (its `[sprites]` entry), keeping the rest of the file as it is.
//!
//! A place keeps the box the game's code sees (the original's size, or a size rule's) and
//! moves, sizes, turns and mirrors the picture inside it: `offset`, `draw_size`, `rotate` and
//! `flip`, as the loader applies them.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::ops::engine::{decode_text, encode_text, write_atomic};
use crate::ops::sprites::base64;
use crate::ops::studio::{add_feature, with_manifest};
use crate::service::Service;
use qm_core::patch::loader::{classify_asset_relative_path, DiscoveredAsset};
use qm_core::patch::SpriteRule;
use qm_manager::profiles::is_reparse_point;
use qm_manager::vanilla_assets::{cat_stem_of, png_size, png_size_of_file, VanillaAssetListing};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use toml_edit::{Array, Document as DocumentMut, InlineTable, Item, Table, Value};

/// Largest image the preview reads.
pub const MAX_SPRITE_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Most images one list carries.
pub const MAX_SPRITE_ITEMS: usize = 5000;
/// The image types the game loads.
const IMAGE_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "bmp"];
/// The keys of a place, in the order they are written.
const PLACE_KEYS: [&str; 4] = ["offset", "draw_size", "rotate", "flip"];
/// Most numbered images one game loop is read as.
const MAX_LOOP: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SpriteSource {
    /// The game's own images (`owner` is empty).
    Game,
    /// A mod folder of the workspace (`owner` is the folder).
    Workspace,
    /// A library mod (`owner` is its id; its newest version is read).
    Library,
}

/// One image: where it comes from and its path there, with `/`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
pub struct SpriteRef {
    pub source: SpriteSource,
    pub owner: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SpriteItem {
    pub sprite: SpriteRef,
    /// The file name without its extension.
    pub name: String,
    /// The pixel size, when it could be read (PNG); 0 otherwise.
    pub width: u32,
    pub height: u32,
}

/// A place to pick images from: the game, a workspace mod, a library mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SpriteShelf {
    pub source: SpriteSource,
    pub owner: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SpriteCatalog {
    pub items: Vec<SpriteItem>,
    /// More images than `MAX_SPRITE_ITEMS` were found and left out.
    pub truncated: bool,
}

/// Where Save writes a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SpriteTarget {
    /// `[sprites.<global>]` in `qm.toml`.
    Qm,
    /// The `[sprites]` entry of `faction.toml`.
    Faction,
}

/// Where the picture sits in its box, in the game's pixels: its top-left corner's move from
/// the box's, the size it is drawn at, its turn in degrees clockwise about its middle, and
/// its mirroring. The whole box, unturned, is no place.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SpritePlace {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotate: f64,
    pub flip_x: bool,
    pub flip_y: bool,
}

/// A game image a file of the mod is shown as.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SpriteUse {
    /// The game global (`allied_fire`).
    pub global: String,
    /// Where Save writes.
    pub target: SpriteTarget,
    /// The game's own image at that global, to line up with; None when the game has none
    /// (the mod adds the image).
    pub original: Option<SpriteItem>,
    /// The box: the size the game's code sees, in the game's pixels.
    pub box_width: f64,
    pub box_height: f64,
    /// The place saved now, or the whole box.
    pub place: SpritePlace,
    /// A place is saved.
    pub placed: bool,
    /// The global's size rule as written (`size = [40, 12]`), when it has one.
    pub size_rule: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SpriteUses {
    pub file: SpriteItem,
    pub uses: Vec<SpriteUse>,
    /// What to know, in plain words: why there are none, or how the game was read.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SpriteSaveRequest {
    pub global: String,
    pub target: SpriteTarget,
    pub place: SpritePlace,
    /// The box the place was made in (the use's), to tell a draw size from the whole box.
    pub box_width: f64,
    pub box_height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SpriteSaved {
    /// The file written: `qm.toml` or `faction.toml`.
    pub file: String,
    /// The entry as it is now in that file; empty when it was taken out.
    pub written: String,
    /// What it does in game, in plain words.
    pub message: String,
    /// The feature added to `mod.json`, when one was.
    pub feature_added: Option<String>,
}

/// The game's images by global, read from its Lua: `name = love.graphics.newImage("...")`
/// and the numbered loops (`_G["rank"..z] = love.graphics.newImage("sprites/rank"..z..".png")`).
#[derive(Debug, Default)]
pub(crate) struct GameImages {
    /// Global -> the game file it is loaded from.
    by_global: BTreeMap<String, String>,
    /// Lowercase `folder/stem` of a game file -> the globals loaded from it.
    by_cat_stem: HashMap<String, Vec<String>>,
    /// Lowercase file stem -> the `folder/stem` keys that have it.
    by_stem: HashMap<String, Vec<String>>,
}

impl GameImages {
    pub(crate) fn read(
        files: &HashMap<String, String>,
        listing: Option<&VanillaAssetListing>,
    ) -> Self {
        let mut out = GameImages::default();
        let (Ok(direct), Ok(looped), Ok(for_re)) = (
            regex_lite::Regex::new(
                r#"(?m)^[ \t]*([A-Za-z_][A-Za-z0-9_]*)[ \t]*=[ \t]*love\.graphics\.newImage[ \t]*\([ \t]*"([^"]+)"[ \t]*\)"#,
            ),
            regex_lite::Regex::new(
                r#"(?m)^[ \t]*_G\[[ \t]*"([A-Za-z_][A-Za-z0-9_]*)"[ \t]*\.\.[ \t]*([A-Za-z_][A-Za-z0-9_]*)[ \t]*(?:\.\.[ \t]*"([A-Za-z0-9_]*)"[ \t]*)?\][ \t]*=[ \t]*love\.graphics\.newImage[ \t]*\([ \t]*"([^"]+)"[ \t]*(?:\.\.[ \t]*([A-Za-z_][A-Za-z0-9_]*)[ \t]*\.\.[ \t]*"([^"]*)"[ \t]*)?\)"#,
            ),
            // `for z = 1, 3 do`; the bounds are kept only when they are numbers.
            regex_lite::Regex::new(
                r"for[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*=[ \t]*([^,]*),[ \t]*([^ \t,]*)",
            ),
        ) else {
            return out;
        };
        // main.lua first, as the game runs it, then the rest by name.
        let mut names: Vec<&String> = files.keys().collect();
        names.sort_by_key(|n| (n.as_str() != "main.lua", n.to_ascii_lowercase()));
        let listed: Vec<String> = listing
            .map(|l| l.paths().map(str::to_string).collect())
            .unwrap_or_default();
        for name in names {
            let Some(text) = files.get(name) else {
                continue;
            };
            for c in direct.captures_iter(text) {
                if let (Some(g), Some(p)) = (c.get(1), c.get(2)) {
                    out.add(g.as_str(), p.as_str());
                }
            }
            for c in looped.captures_iter(text) {
                let (Some(whole), Some(prefix), Some(var), Some(head)) =
                    (c.get(0), c.get(1), c.get(2), c.get(4))
                else {
                    continue;
                };
                let suffix = c.get(3).map_or("", |m| m.as_str());
                let path_var = c.get(5).map(|m| m.as_str());
                let tail = c.get(6).map_or("", |m| m.as_str());
                // The loop's bounds, from the nearest `for <var> = ...` a few lines up, when
                // they are numbers.
                let before = &text[..whole.start()];
                let bounds = before
                    .lines()
                    .rev()
                    .take(8)
                    .filter_map(|l| for_re.captures(l))
                    .find(|f| f.get(1).is_some_and(|v| v.as_str() == var.as_str()))
                    .and_then(|f| {
                        let a: u32 = f.get(2)?.as_str().trim().parse().ok()?;
                        let b: u32 = f.get(3)?.as_str().trim().parse().ok()?;
                        Some((a, b.min(a.saturating_add(MAX_LOOP))))
                    });
                let numbers: Vec<u32> = match (bounds, path_var) {
                    (Some((a, b)), _) => (a..=b).collect(),
                    // No bounds: the numbers the game has files for.
                    (None, Some(_)) => {
                        let (head_l, tail_l) = (
                            head.as_str().to_ascii_lowercase(),
                            tail.to_ascii_lowercase(),
                        );
                        let mut n: Vec<u32> = listed
                            .iter()
                            .filter_map(|p| {
                                let p = p.to_ascii_lowercase();
                                let mid = p.strip_prefix(&head_l)?.strip_suffix(&tail_l)?;
                                (!mid.is_empty() && mid.bytes().all(|b| b.is_ascii_digit()))
                                    .then(|| mid.parse().ok())
                                    .flatten()
                            })
                            .collect();
                        n.sort_unstable();
                        n
                    }
                    (None, None) => Vec::new(),
                };
                for n in numbers {
                    let global = format!("{}{n}{suffix}", prefix.as_str());
                    let path = match path_var {
                        Some(_) => format!("{}{n}{tail}", head.as_str()),
                        None => head.as_str().to_string(),
                    };
                    out.add(&global, &path);
                }
            }
        }
        out
    }

    fn add(&mut self, global: &str, path: &str) {
        let path = path.replace('\\', "/").trim_start_matches('/').to_string();
        if self.by_global.contains_key(global) {
            return;
        }
        let cat_stem = cat_stem_of(&path);
        self.by_global.insert(global.to_string(), path);
        let globals = self.by_cat_stem.entry(cat_stem.clone()).or_default();
        if !globals.iter().any(|g| g == global) {
            globals.push(global.to_string());
        }
        let stem = cat_stem.rsplit('/').next().unwrap_or(&cat_stem).to_string();
        let keys = self.by_stem.entry(stem).or_default();
        if !keys.contains(&cat_stem) {
            keys.push(cat_stem);
        }
    }

    /// The globals the asset autoloader gives a mod's image file, as `__qm_auto_asset` finds
    /// them: the game's file of the same folder and name, else of the same name in another
    /// folder (not across `sprites/`, `ui/` and `cheevos/`). Empty: a new image.
    fn autoload_targets(&self, asset: &DiscoveredAsset) -> Vec<String> {
        let cat = asset.category.to_ascii_lowercase();
        let stem = asset.stem.to_ascii_lowercase();
        if let Some(g) = self.by_cat_stem.get(&format!("{cat}/{stem}")) {
            return g.clone();
        }
        let guarded = ["sprites", "ui", "cheevos"];
        let main = |c: &str| c.split('/').next().unwrap_or("").to_string();
        let mine = main(&cat);
        let mut out: Vec<String> = Vec::new();
        for key in self.by_stem.get(&stem).into_iter().flatten() {
            let theirs = main(key);
            if guarded.contains(&theirs.as_str())
                && guarded.contains(&mine.as_str())
                && theirs != mine
            {
                continue;
            }
            for g in self.by_cat_stem.get(key).into_iter().flatten() {
                if !out.contains(g) {
                    out.push(g.clone());
                }
            }
        }
        out
    }
}

impl Service {
    /// The places to pick images from: this mod, the game, the workspace's other mods and
    /// the library's mods.
    pub fn engine_sprite_shelves(&self, folder: &str) -> QmResult<Vec<SpriteShelf>> {
        self.engine_project_dir(folder)?;
        let projects = self.engine_projects()?.projects;
        let label = projects
            .iter()
            .find(|p| p.folder == folder)
            .map_or_else(|| folder.to_string(), |p| p.name.clone());
        let mut out = vec![SpriteShelf {
            source: SpriteSource::Workspace,
            owner: folder.to_string(),
            label,
        }];
        if self.paths().load_settings().game_exe_path.is_some() {
            out.push(SpriteShelf {
                source: SpriteSource::Game,
                owner: String::new(),
                label: "The game".to_string(),
            });
        }
        for p in projects.into_iter().filter(|p| p.folder != folder) {
            out.push(SpriteShelf {
                source: SpriteSource::Workspace,
                owner: p.folder,
                label: p.name,
            });
        }
        for m in self.engine_library_mods() {
            out.push(SpriteShelf {
                source: SpriteSource::Library,
                label: format!("{} {}", m.name, m.version),
                owner: m.id,
            });
        }
        Ok(out)
    }

    /// The images of a shelf, by path, with their pixel sizes.
    pub fn engine_sprite_catalog(
        &self,
        source: SpriteSource,
        owner: &str,
    ) -> QmResult<SpriteCatalog> {
        let mut items = Vec::new();
        let mut truncated = false;
        match source {
            SpriteSource::Game => {
                let listing = self.game_listing()?;
                for a in listing.assets() {
                    if !is_image(&a.path) {
                        continue;
                    }
                    if items.len() >= MAX_SPRITE_ITEMS {
                        truncated = true;
                        break;
                    }
                    items.push(SpriteItem {
                        name: stem_of(&a.path),
                        width: a.width.unwrap_or(0),
                        height: a.height.unwrap_or(0),
                        sprite: SpriteRef {
                            source,
                            owner: String::new(),
                            path: a.path.clone(),
                        },
                    });
                }
            }
            SpriteSource::Workspace | SpriteSource::Library => {
                let root = self.sprite_owner_dir(source, owner)?;
                let mut stack = vec![root.clone()];
                'walk: while let Some(dir) = stack.pop() {
                    let Ok(rd) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for e in rd.flatten() {
                        let path = e.path();
                        if e.file_name().to_string_lossy().starts_with('.')
                            || is_reparse_point(&path)
                        {
                            continue;
                        }
                        let Ok(meta) = std::fs::symlink_metadata(&path) else {
                            continue;
                        };
                        if meta.is_dir() {
                            stack.push(path);
                            continue;
                        }
                        let Ok(rel) = path.strip_prefix(&root) else {
                            continue;
                        };
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if !meta.is_file() || !is_image(&rel) {
                            continue;
                        }
                        if items.len() >= MAX_SPRITE_ITEMS {
                            truncated = true;
                            break 'walk;
                        }
                        let (width, height) = png_size_of_file(&path).unwrap_or((0, 0));
                        items.push(SpriteItem {
                            name: stem_of(&rel),
                            width,
                            height,
                            sprite: SpriteRef {
                                source,
                                owner: owner.to_string(),
                                path: rel,
                            },
                        });
                    }
                }
            }
        }
        items.sort_by_cached_key(|i| i.sprite.path.to_ascii_lowercase());
        Ok(SpriteCatalog { items, truncated })
    }

    /// `data:image/...;base64,...` for an image of a shelf. The game's are kept in memory; a
    /// mod's are read again each time, so an artist's edit shows.
    pub fn engine_sprite_image(&self, sprite: &SpriteRef) -> QmResult<String> {
        let ext = ext_of(&sprite.path);
        if !IMAGE_EXTS.contains(&ext.as_str()) {
            return Err(invalid(format!(
                "'{}' is not an image the game loads.",
                sprite.path
            )));
        }
        let mime = match ext.as_str() {
            "png" => "image/png",
            "bmp" => "image/bmp",
            _ => "image/jpeg",
        };
        let bytes = match sprite.source {
            SpriteSource::Game => {
                let path = qm_manager::library::validate_safe_relative_path(&sprite.path)
                    .map_err(invalid)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Some(hit) = self
                    .inner
                    .sprite_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&path)
                {
                    return Ok(hit.clone());
                }
                let bytes = self
                    .read_payload_file(&path, MAX_SPRITE_IMAGE_BYTES)?
                    .ok_or_else(|| not_found(format!("The game has no image {path}.")))?;
                let url = format!("data:{mime};base64,{}", base64(&bytes));
                self.inner
                    .sprite_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(path, url.clone());
                return Ok(url);
            }
            SpriteSource::Workspace | SpriteSource::Library => {
                let dir = self.sprite_owner_dir(sprite.source, &sprite.owner)?;
                let (file, shown) = Service::engine_file_path(&dir, &sprite.path)?;
                let meta = std::fs::metadata(&file)
                    .map_err(|_| not_found(format!("No image '{shown}'.")))?;
                if !meta.is_file() {
                    return Err(not_found(format!("No image '{shown}'.")));
                }
                if meta.len() > MAX_SPRITE_IMAGE_BYTES {
                    return Err(invalid(format!(
                        "'{shown}' is over {} MB, too large to preview.",
                        MAX_SPRITE_IMAGE_BYTES / 1024 / 1024
                    )));
                }
                std::fs::read(&file).or_kind(ErrorKind::Io)?
            }
        };
        Ok(format!("data:{mime};base64,{}", base64(&bytes)))
    }

    /// Where the game shows image `path` of workspace mod `folder`: the globals a faction sets
    /// it on, and the ones the asset autoloader gives it, each with the game's own image, the
    /// box and the place saved now.
    pub fn engine_sprite_uses(&self, folder: &str, path: &str) -> QmResult<SpriteUses> {
        let dir = self.engine_project_dir(folder)?;
        let (file, shown) = Service::engine_file_path(&dir, path)?;
        if !file.is_file() {
            return Err(not_found(format!("No file '{shown}' in {folder}.")));
        }
        if !is_image(&shown) {
            return Err(invalid(format!(
                "'{shown}' is not an image the game loads."
            )));
        }
        let (fw, fh) = png_size_of_file(&file).unwrap_or((0, 0));
        let item = SpriteItem {
            name: stem_of(&shown),
            width: fw,
            height: fh,
            sprite: SpriteRef {
                source: SpriteSource::Workspace,
                owner: folder.to_string(),
                path: shown.clone(),
            },
        };
        let images = self.game_images();
        let mut notes: Vec<String> = Vec::new();
        if images.is_none() {
            notes.push("The game's files are not read yet (set the game in Settings), so the game's images cannot be shown.".to_string());
        }
        let empty = GameImages::default();
        let images_ref = images.as_deref().unwrap_or(&empty);
        let mut uses: Vec<SpriteUse> = Vec::new();

        // A faction's images.
        if let Ok(text) = std::fs::read_to_string(dir.join("faction.toml")) {
            let report = qm_schema::faction::check_faction(&text, &dir);
            match &report.faction {
                Some(f) => {
                    for global in faction_globals(f, &shown) {
                        let rule = f.sprite_sizes.get(&global).map(|s| SpriteRule {
                            size: s.size,
                            scale: s.scale,
                            like: s.like.clone(),
                            offset: s.offset,
                            rotate: s.rotate,
                            flip: s.flip.clone(),
                            draw_size: s.draw_size,
                        });
                        uses.push(self.sprite_use(images_ref, &global, SpriteTarget::Faction, rule.as_ref(), (fw, fh)));
                    }
                    if f.vehicles.iter().any(|v| v.debris.iter().any(|d| d.eq_ignore_ascii_case(&shown))) {
                        notes.push("This image is vehicle debris: the faction spreads its debris over several game images, so it is not lined up here.".to_string());
                    }
                }
                None => notes.push(
                    "faction.toml has a mistake, so the images it sets cannot be found. Check it in Files.".to_string(),
                ),
            }
        }

        // The asset autoloader's.
        if let Some(asset) = classify_asset_relative_path(&shown) {
            let rules = read_qm_sprite_rules(&dir);
            let mut targets = images_ref.autoload_targets(&asset);
            if targets.is_empty() {
                targets.push(asset.stem.clone());
                if images.is_some() {
                    notes.push(format!(
                        "The game has no image named {}, so this one is added as a new image of that name. Pick an image to compare it with.",
                        asset.stem
                    ));
                }
            }
            for global in targets {
                if uses.iter().any(|u| u.global == global) {
                    continue;
                }
                let rule = rules.get(&global);
                uses.push(self.sprite_use(images_ref, &global, SpriteTarget::Qm, rule, (fw, fh)));
            }
        } else if uses.is_empty() {
            notes.push(
                "The game does not load this file: only images under assets/ and the ones faction.toml names are. Pick an image to compare it with; the place cannot be saved."
                    .to_string(),
            );
        }
        Ok(SpriteUses {
            file: item,
            uses,
            note: (!notes.is_empty()).then(|| notes.join(" ")),
        })
    }

    /// Saves a place into `qm.toml` or `faction.toml`, writing only what differs from the
    /// whole box, and lists the feature that reads it in `mod.json`.
    pub fn engine_sprite_save(
        &self,
        folder: &str,
        req: SpriteSaveRequest,
    ) -> QmResult<SpriteSaved> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        if !is_lua_name(&req.global) {
            return Err(invalid(format!(
                "'{}' is not the name of a game image.",
                req.global
            )));
        }
        let p = req.place;
        let finite = [
            p.x,
            p.y,
            p.width,
            p.height,
            p.rotate,
            req.box_width,
            req.box_height,
        ]
        .iter()
        .all(|v| v.is_finite());
        if !finite
            || p.width <= 0.0
            || p.height <= 0.0
            || req.box_width <= 0.0
            || req.box_height <= 0.0
        {
            return Err(invalid(
                "The place has a size of 0 or a value that is not a number.",
            ));
        }
        let keys = place_keys(&p, req.box_width, req.box_height);
        let (file, entry, feature) = match req.target {
            SpriteTarget::Qm => (
                "qm.toml",
                save_qm(&dir, &req.global, &keys)?,
                "sprite_transforms",
            ),
            SpriteTarget::Faction => (
                "faction.toml",
                save_faction(&dir, &req.global, &keys)?,
                "faction_sprite_sizes",
            ),
        };
        let feature_added = if keys.is_empty() {
            None
        } else {
            add_feature(&dir, feature)?.then(|| feature.to_string())
        };
        let mut message = describe(&req.global, &keys, file, req.target);
        if let Some(f) = &feature_added {
            message.push_str(&format!(
                " Added \"{f}\" to mod.json, so a QuarterMaster too old for it skips the mod instead of drawing the image out of place."
            ));
        }
        Ok(SpriteSaved {
            file: file.to_string(),
            written: entry,
            message,
            feature_added,
        })
    }

    /// One use of a file as `global`: the game's image there, the box, the place.
    fn sprite_use(
        &self,
        images: &GameImages,
        global: &str,
        target: SpriteTarget,
        rule: Option<&SpriteRule>,
        file_size: (u32, u32),
    ) -> SpriteUse {
        let original = images
            .by_global
            .get(global)
            .map(|path| self.game_item(path));
        let orig_size = |item: &SpriteItem| (f64::from(item.width), f64::from(item.height));
        let natural = original
            .as_ref()
            .map(orig_size)
            .filter(|(w, h)| *w > 0.0 && *h > 0.0)
            .unwrap_or((f64::from(file_size.0), f64::from(file_size.1)));
        let (mut bw, mut bh) = natural;
        let mut size_rule = None;
        if let Some(r) = rule {
            if let Some([w, h]) = r.size {
                (bw, bh) = (w, h);
                size_rule = Some(format!("size = [{}, {}]", num(w), num(h)));
            } else if let Some(k) = r.scale {
                (bw, bh) = (f64::from(file_size.0) * k, f64::from(file_size.1) * k);
                size_rule = Some(format!("scale = {}", num(k)));
            } else if let Some(other) = &r.like {
                if let Some(item) = images.by_global.get(other).map(|p| self.game_item(p)) {
                    (bw, bh) = orig_size(&item);
                }
                size_rule = Some(format!("like = \"{other}\""));
            }
        }
        let placed = rule.is_some_and(|r| {
            r.offset.is_some() || r.rotate.is_some() || r.flip.is_some() || r.draw_size.is_some()
        });
        let flip = rule.and_then(|r| r.flip.as_deref()).unwrap_or("");
        let [x, y] = rule.and_then(|r| r.offset).unwrap_or([0.0, 0.0]);
        let [width, height] = rule.and_then(|r| r.draw_size).unwrap_or([bw, bh]);
        SpriteUse {
            global: global.to_string(),
            target,
            original,
            box_width: bw,
            box_height: bh,
            place: SpritePlace {
                x,
                y,
                width,
                height,
                rotate: rule.and_then(|r| r.rotate).unwrap_or(0.0),
                flip_x: flip.contains('x'),
                flip_y: flip.contains('y'),
            },
            placed,
            size_rule,
        }
    }

    /// A game image as an item, its size from the listing or the file's header.
    fn game_item(&self, path: &str) -> SpriteItem {
        let listed = self
            .vanilla_files()
            .and_then(|v| v.listing.clone())
            .and_then(|l| {
                let lower = cat_stem_of(path);
                l.get(&lower)
                    .filter(|a| a.path.eq_ignore_ascii_case(path))
                    .map(|a| (a.path.clone(), a.width, a.height))
            });
        let (path, w, h) = match listed {
            Some((p, Some(w), Some(h))) => (p, w, h),
            other => {
                let p = other.map_or_else(|| path.to_string(), |(p, _, _)| p);
                let size = self
                    .read_payload_file(&p, MAX_SPRITE_IMAGE_BYTES)
                    .ok()
                    .flatten()
                    .and_then(|b| png_size(&b))
                    .unwrap_or((0, 0));
                (p, size.0, size.1)
            }
        };
        SpriteItem {
            name: stem_of(&path),
            width: w,
            height: h,
            sprite: SpriteRef {
                source: SpriteSource::Game,
                owner: String::new(),
                path,
            },
        }
    }

    /// The game's images by global, read once per game version.
    fn game_images(&self) -> Option<Arc<GameImages>> {
        let vanilla = self.vanilla_files()?;
        if vanilla.files.is_empty() {
            return None;
        }
        let key = vanilla
            .info
            .as_ref()
            .map(|i| i.exe_sha256.clone())
            .unwrap_or_else(|| vanilla.game_version.clone());
        if let Some((k, imgs)) = &self.engine_state().images {
            if *k == key {
                return Some(imgs.clone());
            }
        }
        let imgs = Arc::new(GameImages::read(&vanilla.files, vanilla.listing.as_deref()));
        self.engine_state_mut().images = Some((key, imgs.clone()));
        Some(imgs)
    }

    fn game_listing(&self) -> QmResult<Arc<VanillaAssetListing>> {
        self.vanilla_files()
            .and_then(|v| v.listing.clone())
            .ok_or_else(|| {
                not_found("The game's files are not read yet. Set the game in Settings, Paths.")
            })
    }

    fn sprite_owner_dir(&self, source: SpriteSource, owner: &str) -> QmResult<PathBuf> {
        match source {
            SpriteSource::Workspace => self.engine_project_dir(owner),
            SpriteSource::Library => {
                let m = self
                    .paths()
                    .list_installed_mods()
                    .into_iter()
                    .find(|m| m.manifest.id == owner && !m.is_workspace_linked)
                    .ok_or_else(|| not_found(format!("No library mod '{owner}'.")))?;
                if is_reparse_point(&m.install_path) {
                    return Err(invalid(format!("'{owner}' is a link.")));
                }
                Ok(m.install_path)
            }
            SpriteSource::Game => Err(invalid("The game is not a folder.")),
        }
    }
}

/// The globals a faction sets image `rel` on: frames (`<tag>_<frame>`), weapons
/// (`top_<tag>`, `side_<tag>`), `[sprites]`, vehicle parts and class badges (`spec_<class>`).
fn faction_globals(f: &qm_schema::faction::ResolvedFaction, rel: &str) -> Vec<String> {
    let same = |p: &str| p.eq_ignore_ascii_case(rel);
    let mut out: Vec<String> = Vec::new();
    for (tag, frames) in &f.frames {
        for (frame, p) in frames {
            if same(p) {
                out.push(format!("{tag}_{frame}"));
            }
        }
    }
    for w in &f.weapons {
        if w.top.as_deref().is_some_and(same) {
            out.push(format!("top_{}", w.tag));
        }
        if w.side.as_deref().is_some_and(same) {
            out.push(format!("side_{}", w.tag));
        }
    }
    for (global, p) in &f.sprites {
        if same(p) {
            out.push(global.clone());
        }
    }
    for v in &f.vehicles {
        for (global, p) in &v.parts {
            if same(p) {
                out.push(global.clone());
            }
        }
    }
    // A class's badge goes on `spec_<class>`.
    for (class, c) in &f.classes {
        if c.icon.as_deref().is_some_and(same) {
            out.push(format!("spec_{class}"));
        }
    }
    out.dedup();
    out
}

/// `qm.toml`'s `[sprites]` rules; empty when there are none or the file does not read.
fn read_qm_sprite_rules(dir: &Path) -> BTreeMap<String, SpriteRule> {
    std::fs::read_to_string(dir.join("qm.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Value>(&t).ok())
        .and_then(|v| v.get("sprites").cloned())
        .and_then(|s| s.try_into::<BTreeMap<String, SpriteRule>>().ok())
        .unwrap_or_default()
}

/// The keys a place writes: only what differs from the whole box, unturned.
fn place_keys(p: &SpritePlace, box_w: f64, box_h: f64) -> Vec<(&'static str, Value)> {
    let r2 = |v: f64| (v * 100.0).round() / 100.0;
    let mut keys = Vec::new();
    let (x, y) = (r2(p.x), r2(p.y));
    if x != 0.0 || y != 0.0 {
        keys.push(("offset", pair(x, y)));
    }
    let (w, h) = (r2(p.width), r2(p.height));
    if (w - r2(box_w)).abs() > 0.004 || (h - r2(box_h)).abs() > 0.004 {
        keys.push(("draw_size", pair(w, h)));
    }
    // Turned into -180 to 180.
    let mut turn = r2(p.rotate % 360.0);
    if turn > 180.0 {
        turn -= 360.0;
    } else if turn <= -180.0 {
        turn += 360.0;
    }
    if turn != 0.0 {
        keys.push(("rotate", number(turn)));
    }
    let flip = format!(
        "{}{}",
        if p.flip_x { "x" } else { "" },
        if p.flip_y { "y" } else { "" }
    );
    if !flip.is_empty() {
        keys.push(("flip", Value::from(flip)));
    }
    keys
}

fn pair(a: f64, b: f64) -> Value {
    let mut arr = Array::new();
    arr.push(number(a));
    arr.push(number(b));
    Value::Array(arr)
}

/// A whole number without its `.0`.
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 1e12 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

fn num(v: f64) -> String {
    number(v).to_string().trim().to_string()
}

/// Reads a mod's TOML file for editing: text, line endings, byte order mark.
fn read_toml(path: &Path, name: &str) -> QmResult<Option<(String, bool, bool)>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let f = decode_text(name, &bytes)?;
            Ok(Some((f.text, f.crlf, f.bom)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(QmError::new(
            ErrorKind::Io,
            format!("{name} could not be read: {e}"),
        )),
    }
}

fn parse(name: &str, text: &str) -> QmResult<DocumentMut> {
    text.parse::<DocumentMut>().map_err(|e| {
        invalid(format!(
            "{name} has a mistake, so the place cannot be saved into it: {}",
            e.message()
        ))
    })
}

/// Sets the place keys of `[sprites.<global>]` in `qm.toml`, which is made when the mod has
/// none. Returns the entry as written.
fn save_qm(dir: &Path, global: &str, keys: &[(&'static str, Value)]) -> QmResult<String> {
    let path = dir.join("qm.toml");
    let (text, crlf, bom) = match read_toml(&path, "qm.toml")? {
        Some(t) => t,
        None if keys.is_empty() => return Ok(String::new()),
        None => (with_manifest(dir, ""), false, false),
    };
    let mut doc = parse("qm.toml", &text)?;
    if doc.get("sprites").is_none() {
        if keys.is_empty() {
            return Ok(String::new());
        }
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert("sprites", Item::Table(t));
    }
    let sprites = doc
        .get_mut("sprites")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| invalid("qm.toml has `sprites` as something other than a table."))?;
    if sprites.get(global).is_none() {
        if keys.is_empty() {
            return Ok(String::new());
        }
        sprites.insert(global, Item::Table(Table::new()));
    }
    let entry = sprites
        .get_mut(global)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            invalid(format!(
                "qm.toml has `sprites.{global}` as something other than a table."
            ))
        })?;
    for k in PLACE_KEYS {
        entry.remove(k);
    }
    for (k, v) in keys {
        entry.insert(k, Item::Value(v.clone()));
    }
    let written = if entry.is_empty() {
        sprites.remove(global);
        String::new()
    } else {
        let lines: Vec<String> = entry
            .iter()
            .map(|(k, v)| format!("{k} = {}", v.to_string().trim()))
            .collect();
        format!("[sprites.{global}]\n{}", lines.join("\n"))
    };
    if sprites.is_empty() {
        doc.remove("sprites");
    }
    write_atomic(&path, &encode_text(&doc.to_string(), crlf, bom))?;
    Ok(written)
}

/// Sets the place keys of the `[sprites]` entry of `faction.toml`: a file name becomes a
/// table with the file; a table that is left with only its file becomes the name again.
fn save_faction(dir: &Path, global: &str, keys: &[(&'static str, Value)]) -> QmResult<String> {
    let path = dir.join("faction.toml");
    let (text, crlf, bom) = read_toml(&path, "faction.toml")?
        .ok_or_else(|| not_found("The mod has no faction.toml."))?;
    let mut doc = parse("faction.toml", &text)?;
    if doc.get("sprites").is_none() {
        if keys.is_empty() {
            return Ok(String::new());
        }
        doc.insert("sprites", Item::Table(Table::new()));
    }
    let sprites = doc
        .get_mut("sprites")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| invalid("faction.toml has `sprites` as something other than a table."))?;
    let mut table = InlineTable::new();
    match sprites.get(global) {
        None => {}
        Some(item) => {
            if let Some(file) = item.as_str() {
                table.insert("file", Value::from(file));
            } else if let Some(t) = item.as_table_like() {
                for (k, v) in t.iter() {
                    if PLACE_KEYS.contains(&k) {
                        continue;
                    }
                    if let Some(v) = v.as_value() {
                        table.insert(k, v.clone());
                    }
                }
            } else {
                return Err(invalid(format!(
                    "faction.toml has `sprites.{global}` as something other than a file or a table."
                )));
            }
        }
    }
    for (k, v) in keys {
        table.insert(*k, v.clone());
    }
    let only_file = table.len() == 1 && table.get("file").is_some_and(|v| v.as_str().is_some());
    let written = if table.is_empty() {
        sprites.remove(global);
        String::new()
    } else if only_file {
        let file = table
            .get("file")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let v = Value::from(file);
        let shown = format!("{global} = {}", v.to_string().trim());
        sprites.insert(global, Item::Value(v));
        shown
    } else {
        table.fmt();
        let v = Value::InlineTable(table);
        let shown = format!("{global} = {}", v.to_string().trim());
        sprites.insert(global, Item::Value(v));
        shown
    };
    if sprites.is_empty() {
        doc.remove("sprites");
    }
    write_atomic(&path, &encode_text(&doc.to_string(), crlf, bom))?;
    Ok(if written.is_empty() {
        written
    } else {
        format!("[sprites]\n{written}")
    })
}

/// What a saved place does, in plain words.
fn describe(
    global: &str,
    keys: &[(&'static str, Value)],
    file: &str,
    target: SpriteTarget,
) -> String {
    let at = match target {
        SpriteTarget::Qm => format!("{file}, [sprites.{global}]"),
        SpriteTarget::Faction => format!("{file}, [sprites] {global}"),
    };
    if keys.is_empty() {
        return format!(
            "Saved into {at}: no place, so the game draws your image filling the original's box."
        );
    }
    let mut parts: Vec<String> = Vec::new();
    let nums = |v: &Value| -> Vec<f64> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let px = |v: f64| {
        let n = num(v.abs());
        if n == "1" {
            "1 pixel".to_string()
        } else {
            format!("{n} pixels")
        }
    };
    for (k, v) in keys {
        match *k {
            "offset" => {
                let n = nums(v);
                let (x, y) = (
                    n.first().copied().unwrap_or(0.0),
                    n.get(1).copied().unwrap_or(0.0),
                );
                let mut m = Vec::new();
                if x != 0.0 {
                    m.push(format!(
                        "{} {}",
                        px(x),
                        if x > 0.0 { "right" } else { "left" }
                    ));
                }
                if y != 0.0 {
                    m.push(format!("{} {}", px(y), if y > 0.0 { "down" } else { "up" }));
                }
                parts.push(format!("moved {}", m.join(" and ")));
            }
            "draw_size" => {
                let n = nums(v);
                parts.push(format!(
                    "drawn at {} x {}",
                    num(n.first().copied().unwrap_or(0.0)),
                    num(n.get(1).copied().unwrap_or(0.0))
                ));
            }
            "rotate" => {
                let t = v
                    .as_float()
                    .or_else(|| v.as_integer().map(|i| i as f64))
                    .unwrap_or(0.0);
                parts.push(format!(
                    "turned {} degrees {}",
                    num(t.abs()),
                    if t > 0.0 {
                        "clockwise"
                    } else {
                        "anticlockwise"
                    }
                ));
            }
            "flip" => {
                let f = v.as_str().unwrap_or("");
                parts.push(
                    match f {
                        "x" => "mirrored left to right",
                        "y" => "mirrored top to bottom",
                        _ => "mirrored both ways",
                    }
                    .to_string(),
                );
            }
            _ => {}
        }
    }
    let list = match parts.len() {
        1 => parts.remove(0),
        _ => {
            let last = parts.pop().unwrap_or_default();
            format!("{} and {last}", parts.join(", "))
        }
    };
    format!("Saved into {at}. In game your image is {list}, inside the original's box.")
}

fn is_image(path: &str) -> bool {
    IMAGE_EXTS.contains(&ext_of(path).as_str())
}

fn ext_of(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn stem_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}

fn is_lua_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn not_found(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::NotFound, msg.into())
}

fn invalid(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg.into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn lua(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn game_images_are_read_from_the_lua() {
        let files = lua(&[(
            "main.lua",
            "allied_fire = love.graphics.newImage(\"sprites/allied_fire.png\")\n\
             --allied_fire = love.graphics.newImage(\"sprites/soldier1_fire.png\")\n\
             top_m1 = love.graphics.newImage (\"sprites/top_m1.png\")\n\
             for z = 1, 3 do\n  _G[\"germRank\"..z] = love.graphics.newImage(\"sprites/germRank\"..z..\".png\")\nend\n\
             for z = 1, 2 do\n    _G[\"zombie_reload\"..z] = love.graphics.newImage(\"sprites/zombie_idle.png\")\nend\n\
             for z = 1, maxFence do\n  _G[\"fence\"..z..\"_post\"] = love.graphics.newImage(\"sprites/fence\"..z..\"_post.png\")\nend\n",
        )]);
        let listing = VanillaAssetListing::new(
            [
                "sprites/fence1_post.png",
                "sprites/fence2_post.png",
                "sprites/fence10_post.png",
                "sprites/fenceX_post.png",
            ]
            .iter()
            .map(|p| qm_manager::vanilla_assets::VanillaAsset {
                path: p.to_string(),
                size: 1,
                width: Some(4),
                height: Some(4),
            })
            .collect(),
        );
        let g = GameImages::read(&files, Some(&listing));
        assert_eq!(
            g.by_global.get("allied_fire").map(String::as_str),
            Some("sprites/allied_fire.png"),
            "the comment is left out"
        );
        assert_eq!(
            g.by_global.get("top_m1").map(String::as_str),
            Some("sprites/top_m1.png")
        );
        assert_eq!(
            g.by_global.get("germRank3").map(String::as_str),
            Some("sprites/germRank3.png")
        );
        assert!(!g.by_global.contains_key("germRank4"));
        assert_eq!(
            g.by_global.get("zombie_reload2").map(String::as_str),
            Some("sprites/zombie_idle.png")
        );
        assert_eq!(
            g.by_global.get("fence10_post").map(String::as_str),
            Some("sprites/fence10_post.png"),
            "numbers from the files"
        );
        assert!(!g.by_global.keys().any(|k| k.contains('X')));
        assert_eq!(
            g.by_cat_stem.get("sprites/zombie_idle").map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn the_autoloader_targets_are_mirrored() {
        let files = lua(&[(
            "main.lua",
            "allied_fire = love.graphics.newImage(\"sprites/allied_fire.png\")\n\
             perkIcon = love.graphics.newImage(\"ui/perk_1.png\")\n\
             cheevo = love.graphics.newImage(\"cheevos/medal.png\")\n",
        )]);
        let g = GameImages::read(&files, None);
        let t = |rel: &str| g.autoload_targets(&classify_asset_relative_path(rel).unwrap());
        assert_eq!(t("assets/sprites/allied_fire.png"), ["allied_fire"]);
        assert_eq!(
            t("assets/images/allied_fire.png"),
            ["allied_fire"],
            "images/ is sprites/"
        );
        assert_eq!(
            t("assets/splash/perk_1.png"),
            ["perkIcon"],
            "by name from another folder"
        );
        assert!(
            t("assets/sprites/perk_1.png").is_empty(),
            "not across sprites/ and ui/"
        );
        assert!(
            t("assets/other/perk_1.png").is_empty(),
            "an image in another folder is a sprite"
        );
        assert!(t("assets/sprites/brand_new.png").is_empty());
    }

    #[test]
    fn a_place_writes_only_what_differs() {
        let whole = SpritePlace {
            x: 0.0,
            y: 0.0,
            width: 60.0,
            height: 30.0,
            rotate: 360.0,
            flip_x: false,
            flip_y: false,
        };
        assert!(place_keys(&whole, 60.0, 30.0).is_empty());
        let p = SpritePlace {
            x: 3.0,
            y: -2.004,
            width: 64.0,
            height: 30.0,
            rotate: 190.0,
            flip_x: true,
            flip_y: false,
        };
        let keys: Vec<String> = place_keys(&p, 60.0, 30.0)
            .iter()
            .map(|(k, v)| format!("{k} = {}", v.to_string().trim()))
            .collect();
        assert_eq!(
            keys,
            [
                "offset = [3, -2]",
                "draw_size = [64, 30]",
                "rotate = -170",
                "flip = \"x\""
            ]
        );
    }

    #[test]
    fn the_message_says_it_in_plain_words() {
        let p = SpritePlace {
            x: 3.0,
            y: -1.0,
            width: 64.0,
            height: 40.0,
            rotate: -5.0,
            flip_x: true,
            flip_y: true,
        };
        let m = describe(
            "allied_fire",
            &place_keys(&p, 60.0, 30.0),
            "qm.toml",
            SpriteTarget::Qm,
        );
        assert_eq!(
            m,
            "Saved into qm.toml, [sprites.allied_fire]. In game your image is moved 3 pixels right and 1 pixel up, drawn at 64 x 40, turned 5 degrees anticlockwise and mirrored both ways, inside the original's box."
        );
    }
}
