//! Game images and ribbon names, read from the game payload (the `.love` archive fused into
//! `mnb.exe`, or an unpacked game folder) and cached in memory.
use crate::error::{ErrorKind, QmError, QmResult};
use crate::service::Service;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest sprite read; the game's ribbons and awards are a few KB.
const MAX_SPRITE_BYTES: u64 = 2 * 1024 * 1024;
const LOCALISATION: &str = "includes/localisation.lua";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SpriteKind {
    Ribbon,
    Award,
}

impl SpriteKind {
    fn path(self, id: u32) -> String {
        match self {
            SpriteKind::Ribbon => format!("sprites/rib{id}.png"),
            SpriteKind::Award => format!("sprites/award{id}.png"),
        }
    }
}

/// A ribbon's name and description, from the game's `localisation.lua` (`ribName_<N>`, `ribDesc_<N>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RibbonInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
}

impl Service {
    /// `data:image/png;base64,...` for a ribbon or award image; `not_found` when the game has none.
    pub fn game_sprite_get(&self, kind: SpriteKind, id: u32) -> QmResult<String> {
        let path = kind.path(id);
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
            .read_payload_file(&path, MAX_SPRITE_BYTES)?
            .ok_or_else(|| {
                QmError::new(
                    ErrorKind::NotFound,
                    format!("The game has no image {path}."),
                )
            })?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(QmError::new(
                ErrorKind::Integrity,
                format!("{path} is not a PNG image."),
            ));
        }
        let url = format!("data:image/png;base64,{}", base64(&bytes));
        self.inner
            .sprite_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path, url.clone());
        Ok(url)
    }

    /// Every ribbon the game names, by id. Empty when the game files cannot be read.
    pub fn game_ribbons_get(&self) -> QmResult<Vec<RibbonInfo>> {
        if let Some(hit) = self
            .inner
            .ribbon_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(hit);
        }
        let text = self
            .read_payload_file(LOCALISATION, 4 * 1024 * 1024)?
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let ribbons = parse_ribbons(&text);
        *self
            .inner
            .ribbon_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(ribbons.clone());
        Ok(ribbons)
    }

    /// A file from the game payload: the archive inside `mnb.exe` first, then an unpacked folder.
    fn read_payload_file(&self, inner: &str, max: u64) -> QmResult<Option<Vec<u8>>> {
        let exe =
            self.paths().load_settings().game_exe_path.ok_or_else(|| {
                QmError::new(ErrorKind::NotFound, "The game executable is not set.")
            })?;
        if let Some(bytes) = read_from_archive(&exe, inner, max) {
            return Ok(Some(bytes));
        }
        let loose: Option<PathBuf> = exe.parent().map(|d| d.join(inner));
        Ok(loose.and_then(|p| {
            let meta = std::fs::metadata(&p).ok()?;
            (meta.is_file() && meta.len() <= max)
                .then(|| std::fs::read(&p).ok())
                .flatten()
        }))
    }
}

fn read_from_archive(archive: &Path, inner: &str, max: u64) -> Option<Vec<u8>> {
    let file = std::fs::File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut entry = zip.by_name(inner).ok()?;
    if entry.size() > max {
        return None;
    }
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry.by_ref().take(max).read_to_end(&mut out).ok()?;
    Some(out)
}

/// `ribName_<N> = "..."` and `ribDesc_<N> = "..."` lines, joined by id.
pub(crate) fn parse_ribbons(text: &str) -> Vec<RibbonInfo> {
    let mut names = std::collections::BTreeMap::<u32, (String, String)>::new();
    for line in text.lines() {
        let line = line.trim();
        for (prefix, is_name) in [("ribName_", true), ("ribDesc_", false)] {
            let Some(rest) = line.strip_prefix(prefix) else {
                continue;
            };
            let Some((num, value)) = rest.split_once('=') else {
                continue;
            };
            let Ok(id) = num.trim().parse::<u32>() else {
                continue;
            };
            let value = value.trim();
            let Some(inner) = value
                .strip_prefix('"')
                .and_then(|v| v.rfind('"').map(|end| &v[..end]))
            else {
                continue;
            };
            let clean = inner.replace("\\n", " ").replace("\\\"", "\"");
            let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
            let entry = names.entry(id).or_default();
            if is_name {
                entry.0 = clean;
            } else {
                entry.1 = clean;
            }
        }
    }
    names
        .into_iter()
        .filter(|(_, (name, _))| !name.is_empty())
        .map(|(id, (name, description))| RibbonInfo {
            id,
            name,
            description,
        })
        .collect()
}

/// Standard base64 with padding (no extra dependency for one small use).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn ribbons_are_parsed_from_localisation() {
        let text = "x = 1\nribName_1 = \"Combat\"\nribDesc_1 = \"Kill 100 enemies, +1 damage \\n to all attacks\"\nribName_61 = \"Mud\"\nribDesc_2 = \"no name\"\n";
        assert_eq!(
            parse_ribbons(text),
            vec![
                RibbonInfo {
                    id: 1,
                    name: "Combat".into(),
                    description: "Kill 100 enemies, +1 damage to all attacks".into()
                },
                RibbonInfo {
                    id: 61,
                    name: "Mud".into(),
                    description: String::new()
                },
            ]
        );
    }
}
