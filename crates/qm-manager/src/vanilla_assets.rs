//! The game's own asset files, listed from the game archive (Phase E D79).
//!
//! The game executable ends in a zip that holds every sound, image and font next to the Lua
//! files. The Lua sources only name some of them (many paths are built in loops), so the
//! archive listing is what tells an override from an addition, gives the size of each vanilla
//! image, and counts the members of a numbered set (`axisHurt1..30`).

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek};
use std::path::Path;

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::storage::VanillaGameInfo;

/// The file the listing is cached in, next to the cached `main.lua`.
pub const LISTING_FILE: &str = "vanilla_assets.json";

/// Bumped when the listing's content changes, so an older cache file is rebuilt.
const LISTING_VERSION: u32 = 1;

/// The extensions the asset autoloader handles (`classify_asset_relative_path`).
const ASSET_EXTS: [&str; 9] = ["wav", "mp3", "ogg", "png", "jpg", "jpeg", "bmp", "ttf", "otf"];

/// One asset file of the game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VanillaAsset {
    /// The path inside the archive, with `/` separators, as the game names it.
    pub path: String,
    /// The file size in bytes.
    pub size: u64,
    /// The image size, for a PNG whose header could be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl VanillaAsset {
    /// Lowercase `folder/stem`, the key the autoloader registers a file under.
    pub fn cat_stem(&self) -> String {
        cat_stem_of(&self.path)
    }
}

/// Lowercase path without its extension (`sounds/vo/axis/axisHurt1.wav` ->
/// `sounds/vo/axis/axishurt1`).
pub fn cat_stem_of(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.trim_start_matches('/');
    let cut = match (p.rfind('.'), p.rfind('/')) {
        (Some(dot), Some(slash)) if dot > slash => dot,
        (Some(dot), None) => dot,
        _ => p.len(),
    };
    p.get(..cut).unwrap_or(p).to_ascii_lowercase()
}

/// A numbered slot split into its set key and number: `sounds/garand12` ->
/// (`sounds/garand#`, 12). None when the stem does not end in digits or is digits only.
pub fn numbered(cat_stem: &str) -> Option<(String, u32)> {
    let stem_start = cat_stem.rfind('/').map_or(0, |i| i + 1);
    let stem = cat_stem.get(stem_start..)?;
    let digits = stem.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits == stem.len() || digits > 6 {
        return None;
    }
    let cut = cat_stem.len() - digits;
    let n: u32 = cat_stem.get(cut..)?.parse().ok()?;
    Some((format!("{}#", cat_stem.get(..cut)?), n))
}

#[derive(Serialize, Deserialize)]
struct ListingFile {
    version: u32,
    assets: Vec<VanillaAsset>,
}

/// Every asset file of one game version, sorted by path.
#[derive(Debug, Clone, Default)]
pub struct VanillaAssetListing {
    assets: Vec<VanillaAsset>,
    by_cat_stem: HashMap<String, usize>,
    /// Lowercase file stem -> the files with it, in path order.
    by_stem: HashMap<String, Vec<usize>>,
    /// Numbered set key -> how many members the game has.
    set_counts: HashMap<String, u32>,
}

impl VanillaAssetListing {
    pub fn new(mut assets: Vec<VanillaAsset>) -> Self {
        assets.sort_by(|a, b| a.path.cmp(&b.path));
        assets.dedup_by(|a, b| a.path == b.path);
        let mut by_cat_stem = HashMap::new();
        let mut by_stem: HashMap<String, Vec<usize>> = HashMap::new();
        let mut set_counts: HashMap<String, u32> = HashMap::new();
        for (i, a) in assets.iter().enumerate() {
            let key = a.cat_stem();
            if let Some((set, _)) = numbered(&key) {
                *set_counts.entry(set).or_default() += 1;
            }
            let stem = key.rsplit('/').next().unwrap_or(&key).to_string();
            by_stem.entry(stem).or_default().push(i);
            by_cat_stem.entry(key).or_insert(i);
        }
        Self {
            assets,
            by_cat_stem,
            by_stem,
            set_counts,
        }
    }

    pub fn assets(&self) -> &[VanillaAsset] {
        &self.assets
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// The archive paths, for `VanillaAssetIndex::from_archive_paths`.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.assets.iter().map(|a| a.path.as_str())
    }

    /// The game's file for a lowercase `folder/stem`.
    pub fn get(&self, cat_stem: &str) -> Option<&VanillaAsset> {
        self.by_cat_stem
            .get(cat_stem)
            .and_then(|i| self.assets.get(*i))
    }

    /// The game's files with a lowercase file stem, in any folder.
    pub fn with_stem(&self, stem: &str) -> impl Iterator<Item = &VanillaAsset> {
        self.by_stem
            .get(stem)
            .into_iter()
            .flatten()
            .filter_map(|i| self.assets.get(*i))
    }

    /// How many members the game has in a numbered set (`sounds/vo/axis/axishurt#`).
    pub fn set_count(&self, set_key: &str) -> u32 {
        self.set_counts.get(set_key).copied().unwrap_or(0)
    }

    /// Lists the asset entries of a zip. None when it is not a zip.
    pub fn from_archive<R: Read + Seek>(reader: R) -> Option<Self> {
        let mut archive = ZipArchive::new(reader).ok()?;
        let mut assets = Vec::new();
        for i in 0..archive.len() {
            let Ok(mut entry) = archive.by_index(i) else {
                continue;
            };
            if entry.is_dir() {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else {
                continue;
            };
            let path = rel.to_string_lossy().replace('\\', "/");
            let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if !ASSET_EXTS.contains(&ext.as_str()) {
                continue;
            }
            let size = entry.size();
            let (mut width, mut height) = (None, None);
            if ext == "png" {
                // The size is in the first 24 bytes; nothing else is decoded.
                let mut head = [0u8; 24];
                if entry.read_exact(&mut head).is_ok() {
                    if let Some((w, h)) = png_size(&head) {
                        width = Some(w);
                        height = Some(h);
                    }
                }
            }
            assets.push(VanillaAsset {
                path,
                size,
                width,
                height,
            });
        }
        Some(Self::new(assets))
    }

    /// The listing for a cached game: read from the cache folder, or made from the game
    /// executable and stored there. None when the executable holds no archive (a cache made
    /// from loose files), so callers fall back to what the Lua sources name.
    pub fn load_or_build(info: &VanillaGameInfo) -> Option<Self> {
        let file = info.cache_dir.join(LISTING_FILE);
        if let Some(listing) = Self::read(&file) {
            return Some(listing);
        }
        let exe = fs::File::open(&info.exe_path).ok()?;
        let listing = Self::from_archive(std::io::BufReader::new(exe))?;
        if listing.is_empty() {
            return None;
        }
        // A failed write only means the listing is made again next time.
        let _ = listing.write(&file);
        Some(listing)
    }

    fn read(file: &Path) -> Option<Self> {
        let raw = fs::read_to_string(file).ok()?;
        let parsed: ListingFile = serde_json::from_str(&raw).ok()?;
        (parsed.version == LISTING_VERSION).then(|| Self::new(parsed.assets))
    }

    fn write(&self, file: &Path) -> Result<(), String> {
        let json = serde_json::to_string(&ListingFile {
            version: LISTING_VERSION,
            assets: self.assets.clone(),
        })
        .map_err(|e| format!("Failed to serialize {LISTING_FILE}: {e}"))?;
        qm_core::handshake::write_file_atomic(file, json.as_bytes())
    }
}

/// Width and height from the start of a PNG file (signature, then the IHDR chunk).
pub fn png_size(head: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if head.get(..8)? != SIGNATURE || head.get(12..16)? != b"IHDR" {
        return None;
    }
    let be = |at: usize| -> Option<u32> {
        let b: [u8; 4] = head.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_be_bytes(b))
    };
    let (w, h) = (be(16)?, be(20)?);
    (w > 0 && h > 0).then_some((w, h))
}

/// The size of a PNG file on disk; None when it is not a PNG or cannot be read.
pub fn png_size_of_file(path: &Path) -> Option<(u32, u32)> {
    let mut head = [0u8; 24];
    fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    png_size(&head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    pub(crate) fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in entries {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn info(dir: &Path, exe: PathBuf) -> VanillaGameInfo {
        VanillaGameInfo {
            exe_path: exe,
            exe_sha256: "0".repeat(64),
            game_version: "0.5.3b".into(),
            steam_app_id: None,
            cache_dir: dir.to_path_buf(),
            lua_file_count: 1,
        }
    }

    #[test]
    fn cat_stems_and_numbered_sets() {
        assert_eq!(cat_stem_of("sounds/vo/axis/axisHurt1.wav"), "sounds/vo/axis/axishurt1");
        assert_eq!(cat_stem_of("\\Sprites\\Top_M1.PNG"), "sprites/top_m1");
        assert_eq!(cat_stem_of("fonts.v2/impact"), "fonts.v2/impact");
        assert_eq!(numbered("sounds/garand12"), Some(("sounds/garand#".into(), 12)));
        assert_eq!(numbered("sounds/vo/axis/axishurt1"), Some(("sounds/vo/axis/axishurt#".into(), 1)));
        assert_eq!(numbered("sounds/tension"), None);
        assert_eq!(numbered("sounds/1917"), None, "digits only is a name, not a set");
        assert_eq!(numbered("sprites2/idle"), None, "digits in a folder name do not count");
    }

    #[test]
    fn png_sizes_come_from_the_header() {
        assert_eq!(png_size(&png_bytes(64, 128)), Some((64, 128)));
        assert_eq!(png_size(b"not a png, but long enough to check"), None);
        assert_eq!(png_size(&png_bytes(64, 128)[..20]), None, "a cut header");
        assert_eq!(png_size(&png_bytes(0, 5)), None);
    }

    #[test]
    fn the_archive_listing_has_assets_only_with_png_sizes() {
        let png = png_bytes(32, 48);
        let zip = zip_with(&[
            ("main.lua", b"print(1)"),
            ("sounds/garand1.wav", b"RIFFxxxx"),
            ("sounds/garand2.wav", b"RIFF"),
            ("sprites/top_m1.png", &png),
            ("sprites/broken.png", b"short"),
            ("fonts/impact.ttf", b"font"),
            ("luasteam/readme.md", b"x"),
        ]);
        let listing = VanillaAssetListing::from_archive(std::io::Cursor::new(zip)).unwrap();
        let paths: Vec<_> = listing.paths().collect();
        assert_eq!(
            paths,
            [
                "fonts/impact.ttf",
                "sounds/garand1.wav",
                "sounds/garand2.wav",
                "sprites/broken.png",
                "sprites/top_m1.png"
            ]
        );
        let top = listing.get("sprites/top_m1").unwrap();
        assert_eq!((top.width, top.height), (Some(32), Some(48)));
        assert_eq!(top.size, png.len() as u64);
        assert_eq!(listing.get("sprites/broken").unwrap().width, None);
        assert_eq!(listing.get("sounds/garand1").unwrap().size, 8);
        assert_eq!(listing.set_count("sounds/garand#"), 2);
        assert_eq!(listing.set_count("sounds/mp40#"), 0);
        let stems: Vec<_> = listing.with_stem("garand1").map(|a| a.path.as_str()).collect();
        assert_eq!(stems, ["sounds/garand1.wav"]);
        assert_eq!(listing.with_stem("nope").count(), 0);
        assert!(VanillaAssetListing::from_archive(std::io::Cursor::new(b"plain".to_vec())).is_none());
    }

    #[test]
    fn the_listing_is_cached_beside_the_sources_and_rebuilt_when_missing_or_old() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mnb.exe");
        // A fused executable: some bytes, then the zip.
        let mut bytes = b"MZ-stub-".to_vec();
        bytes.extend(zip_with(&[("sounds/a1.wav", b"1"), ("sounds/a2.wav", b"22")]));
        fs::write(&exe, &bytes).unwrap();
        let cache = dir.path().join("mnb-00000000");
        fs::create_dir_all(&cache).unwrap();
        let info = info(&cache, exe.clone());

        // A cache from before Phase E has no listing: it is made and stored.
        let first = VanillaAssetListing::load_or_build(&info).unwrap();
        assert_eq!(first.assets().len(), 2);
        let file = cache.join(LISTING_FILE);
        assert!(file.is_file());

        // The stored file is used: the executable is not needed again.
        fs::remove_file(&exe).unwrap();
        let again = VanillaAssetListing::load_or_build(&info).unwrap();
        assert_eq!(again.assets(), first.assets());

        // A file of another version, or a broken one, is not used.
        fs::write(&file, r#"{"version": 0, "assets": []}"#).unwrap();
        assert!(VanillaAssetListing::load_or_build(&info).is_none(), "no exe to rebuild from");
        fs::write(&exe, &bytes).unwrap();
        fs::write(&file, "{").unwrap();
        assert_eq!(VanillaAssetListing::load_or_build(&info).unwrap().assets().len(), 2);
    }

    #[test]
    fn an_executable_without_an_archive_gives_no_listing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mnb.exe");
        fs::write(&exe, b"not a zip").unwrap();
        let info = info(dir.path(), exe);
        assert!(VanillaAssetListing::load_or_build(&info).is_none());
        assert!(!dir.path().join(LISTING_FILE).exists());
    }
}
