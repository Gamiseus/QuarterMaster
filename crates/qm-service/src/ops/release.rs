//! The Release helper (Mod Engine, Stage E): raise a workspace mod's version, write its
//! changelog lines, run the full check and pack the zip. Nothing is uploaded: the author
//! attaches the zip on GitHub's release page, which the UI opens.
//!
//! The zip goes to `<workspace>/_releases/<folder>/`, a folder the loader and the Mod Engine
//! skip (the `_` rule), never next to the mod's own folder.
use crate::error::{ErrorKind, OrKind, QmError, QmResult};
use crate::ops::engine::{decode_text, encode_text, write_atomic};
use crate::service::Service;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use toml_edit::{Document as DocumentMut, Item, Value};

/// Where the zips of a mod's releases go, inside the workspace folder.
const RELEASES_DIR: &str = "_releases";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ReleaseZip {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ReleaseInfo {
    pub mod_id: String,
    pub name: String,
    /// `version` in `mod.json` (None without the file or the key).
    pub json_version: Option<String>,
    /// `[manifest] version` in `qm.toml`.
    pub toml_version: Option<String>,
    /// The next patch, minor and major versions after the newer of the two.
    pub next: Vec<String>,
    /// `owner/repo`, from `[publish] repository`, else `github` in `mod.json`.
    pub repository: Option<String>,
    /// The mod has a `[publish]` table (it asks to be listed on the site).
    pub has_publish: bool,
    pub has_readme: bool,
    /// The newest section of `CHANGELOG.md`: its heading and lines.
    pub changelog_version: Option<String>,
    pub changelog_lines: Vec<String>,
    /// The zip in `_releases/<folder>`, when one was packed.
    pub last_zip: Option<ReleaseZip>,
    /// What the zip is named (`<folder>.zip`).
    pub asset_name: String,
}

impl Service {
    fn release_dir(&self, folder: &str) -> QmResult<PathBuf> {
        let dir = self.engine_project_dir(folder)?;
        let ws = dir
            .parent()
            .ok_or_else(|| QmError::new(ErrorKind::Internal, "The mod has no workspace folder."))?;
        Ok(ws.join(RELEASES_DIR).join(folder))
    }

    /// What the Release view shows for the workspace mod in `folder`.
    pub fn release_info(&self, folder: &str) -> QmResult<ReleaseInfo> {
        let dir = self.engine_project_dir(folder)?;
        let json = read_json(&dir);
        let toml_doc = std::fs::read_to_string(dir.join("qm.toml"))
            .ok()
            .and_then(|t| t.parse::<DocumentMut>().ok());
        let json_str = |k: &str| {
            json.as_ref()
                .and_then(|j| j.get(k))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
        };
        let toml_version = toml_doc.as_ref().and_then(|d| {
            d.get("manifest")?
                .get("version")?
                .as_str()
                .map(str::to_string)
        });
        let publish = toml_doc.as_ref().and_then(|d| d.get("publish"));
        let repository = publish
            .and_then(|p| p.get("repository"))
            .and_then(Item::as_str)
            .map(str::to_string)
            .or_else(|| json_str("github"))
            .map(|r| clean_repository(&r))
            .filter(|r| is_repository(r));
        let json_version = json_str("version");
        let newest = [json_version.as_deref(), toml_version.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(parse_version)
            .max()
            .unwrap_or((0, 0, 0));
        let (changelog_version, changelog_lines) = top_changelog(&dir.join("CHANGELOG.md"));
        let last_zip = self
            .release_dir(folder)
            .ok()
            .and_then(|d| last_zip(&d, folder));
        Ok(ReleaseInfo {
            mod_id: json_str("id").unwrap_or_else(|| folder.to_string()),
            name: json_str("name").unwrap_or_else(|| folder.to_string()),
            json_version,
            toml_version,
            next: next_versions(newest),
            repository,
            has_publish: publish.is_some(),
            has_readme: dir.join("README.md").is_file(),
            changelog_version,
            changelog_lines,
            last_zip,
            asset_name: format!("{folder}.zip"),
        })
    }

    /// Writes `version` to `mod.json` and `qm.toml` (where each has one) and `lines` under a
    /// new `## <version>` at the top of `CHANGELOG.md`.
    pub fn release_prepare(&self, folder: &str, version: &str, lines: &[String]) -> QmResult<()> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let version = version.trim();
        let wanted = parse_version(version).ok_or_else(|| {
            invalid(format!(
                "'{version}' is not a version: write Major.Minor.Patch, such as 1.2.0."
            ))
        })?;
        let info = self.release_info(folder)?;
        let current = [info.json_version.as_deref(), info.toml_version.as_deref()]
            .into_iter()
            .flatten()
            .filter_map(parse_version)
            .max();
        if current.is_some_and(|c| wanted <= c) {
            return Err(invalid(format!(
                "{version} is not newer than the mod's version now."
            )));
        }
        let json_path = dir.join("mod.json");
        if let Ok(bytes) = std::fs::read(&json_path) {
            let f = decode_text("mod.json", &bytes)?;
            if let Some(text) = set_json_version(&f.text, version) {
                write_atomic(&json_path, &encode_text(&text, f.crlf, f.bom))?;
            }
        }
        let toml_path = dir.join("qm.toml");
        if let Ok(bytes) = std::fs::read(&toml_path) {
            let f = decode_text("qm.toml", &bytes)?;
            if let Ok(mut doc) = f.text.parse::<DocumentMut>() {
                if let Some(m) = doc.get_mut("manifest").and_then(Item::as_table_mut) {
                    let decor = m
                        .get("version")
                        .and_then(Item::as_value)
                        .map(|v| v.decor().clone());
                    let mut v = Value::from(version);
                    if let Some(d) = decor {
                        *v.decor_mut() = d;
                    }
                    m.insert("version", Item::Value(v));
                    write_atomic(&toml_path, &encode_text(&doc.to_string(), f.crlf, f.bom))?;
                }
            }
        }
        let clean: Vec<&str> = lines
            .iter()
            .map(|l| l.trim().trim_start_matches(['-', '*']).trim())
            .filter(|l| !l.is_empty())
            .collect();
        if !clean.is_empty() {
            let path = dir.join("CHANGELOG.md");
            let (old, crlf, bom) = match std::fs::read(&path) {
                Ok(bytes) => {
                    let f = decode_text("CHANGELOG.md", &bytes)?;
                    (f.text, f.crlf, f.bom)
                }
                Err(_) => (String::new(), false, false),
            };
            let text = add_changelog_section(&old, version, &clean);
            write_atomic(&path, &encode_text(&text, crlf, bom))?;
        }
        Ok(())
    }

    /// Runs the full check and, when it passes, packs the mod into `_releases/<folder>/`.
    pub fn release_pack(&self, folder: &str) -> QmResult<ReleaseZip> {
        self.ensure_no_install()?;
        let dir = self.engine_project_dir(folder)?;
        let lint = self.engine_lint(folder)?;
        if !lint.passed {
            let errors: Vec<String> = lint
                .diagnostics
                .iter()
                .filter(|d| d.severity == crate::ops::engine::EngineSeverity::Error)
                .take(3)
                .map(|d| d.message.clone())
                .collect();
            return Err(invalid(format!(
                "The check found problems, so nothing was packed: {}",
                errors.join(" ")
            )));
        }
        let out = self.release_dir(folder)?;
        std::fs::create_dir_all(&out).or_kind(ErrorKind::Io)?;
        let (zip, entry) = qm_manager::devtools::package_mod_release(&dir, &out).map_err(|e| {
            QmError::new(ErrorKind::Io, "The mod could not be packed.").with_details(e)
        })?;
        Ok(ReleaseZip {
            name: zip
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(entry.asset_name),
            size_bytes: entry.size_bytes,
            sha256: entry.sha256,
            version: entry.version,
        })
    }

    /// The folder the zips go to, made when missing, for opening in Explorer.
    pub fn release_folder(&self, folder: &str) -> QmResult<PathBuf> {
        let out = self.release_dir(folder)?;
        std::fs::create_dir_all(&out).or_kind(ErrorKind::Io)?;
        Ok(out)
    }
}

fn invalid(msg: impl Into<String>) -> QmError {
    QmError::new(ErrorKind::InvalidInput, msg)
}

fn read_json(dir: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(dir.join("mod.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

/// `Major.Minor.Patch` as numbers; a `-suffix` is not a release version here.
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

fn next_versions((major, minor, patch): (u32, u32, u32)) -> Vec<String> {
    vec![
        format!("{major}.{minor}.{}", patch + 1),
        format!("{major}.{}.0", minor + 1),
        format!("{}.0.0", major + 1),
    ]
}

/// `owner/repo` from a repository written as a URL or with `.git`.
fn clean_repository(r: &str) -> String {
    r.trim()
        .trim_start_matches("https://")
        .trim_start_matches("github.com/")
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string()
}

fn is_repository(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// `text` with the top-level `"version"` value set, leaving the rest as it is. None when
/// there is no such key.
fn set_json_version(text: &str, version: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let current = value.get("version")?.as_str()?.to_string();
    let quoted = serde_json::to_string(&current).ok()?;
    let mut at = 0;
    while let Some(i) = text[at..].find("\"version\"") {
        let key = at + i;
        let after = &text[key + "\"version\"".len()..];
        let colon = after.find(':')?;
        if !after[..colon].trim().is_empty() {
            at = key + 1;
            continue;
        }
        let rest = &after[colon + 1..];
        let lead = rest.len() - rest.trim_start().len();
        if rest.trim_start().starts_with(&quoted) {
            let start = key + "\"version\"".len() + colon + 1 + lead;
            let replaced = format!(
                "{}{}{}",
                &text[..start],
                serde_json::to_string(version).ok()?,
                &text[start + quoted.len()..]
            );
            // Only a result that reads back with the new version is used.
            let check: serde_json::Value = serde_json::from_str(&replaced).ok()?;
            return (check.get("version")?.as_str()? == version).then_some(replaced);
        }
        at = key + 1;
    }
    None
}

/// The newest `## ` section of a changelog: its heading text and its `- ` lines.
fn top_changelog(path: &Path) -> (Option<String>, Vec<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, Vec::new());
    };
    let mut heading = None;
    let mut lines = Vec::new();
    for l in text.lines() {
        if let Some(h) = l.strip_prefix("## ") {
            if heading.is_some() {
                break;
            }
            heading = Some(h.trim().to_string());
        } else if heading.is_some() {
            if let Some(item) = l.trim_start().strip_prefix("- ") {
                lines.push(item.trim().to_string());
            }
        }
    }
    (heading, lines)
}

/// `old` with a `## <version>` section and its lines above the newest section (under the
/// title, when the file has one); a new file gets a title first.
fn add_changelog_section(old: &str, version: &str, lines: &[&str]) -> String {
    let mut section = format!("## {version}\n\n");
    for l in lines {
        section.push_str(&format!("- {l}\n"));
    }
    section.push('\n');
    let old = old.replace("\r\n", "\n");
    if old.trim().is_empty() {
        return format!("# Changelog\n\n{section}");
    }
    match old
        .find("\n## ")
        .map(|i| i + 1)
        .or_else(|| old.starts_with("## ").then_some(0))
    {
        Some(at) => format!("{}{section}{}", &old[..at], &old[at..]),
        None => {
            let mut out = old.trim_end().to_string();
            out.push_str("\n\n");
            out.push_str(&section);
            out
        }
    }
}

fn last_zip(dir: &Path, folder: &str) -> Option<ReleaseZip> {
    let zip = dir.join(format!("{folder}.zip"));
    let bytes = std::fs::read(&zip).ok()?;
    Some(ReleaseZip {
        name: format!("{folder}.zip"),
        size_bytes: bytes.len() as u64,
        sha256: qm_schema::sha256_hex(&bytes),
        version: zip_version(&bytes, folder),
    })
}

/// The version in the zip's `<folder>/mod.json`; empty when it has none.
fn zip_version(bytes: &[u8], folder: &str) -> String {
    use std::io::Read;
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return String::new();
    };
    let Ok(mut file) = zip.by_name(&format!("{folder}/mod.json")) else {
        return String::new();
    };
    let mut text = String::new();
    if file.read_to_string(&mut text).is_err() {
        return String::new();
    }
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("version")?.as_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_step() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3-beta"), None);
        assert_eq!(next_versions((1, 2, 3)), ["1.2.4", "1.3.0", "2.0.0"]);
    }

    #[test]
    fn repositories_are_cleaned_and_checked() {
        assert_eq!(
            clean_repository("https://github.com/you/mod.git/"),
            "you/mod"
        );
        assert!(is_repository("you/mod"));
        assert!(!is_repository("you"));
        assert!(!is_repository("you/mod/extra"));
        assert!(!is_repository("you/m od"));
    }

    #[test]
    fn the_json_version_changes_in_place() {
        let text = "{\n  \"id\": \"m\",\n  \"version\" : \"1.0.0\",\n  \"relationships\": { \"requires\": { \"x\": \"1.0.0\" } }\n}\n";
        assert_eq!(
            set_json_version(text, "1.1.0").unwrap(),
            "{\n  \"id\": \"m\",\n  \"version\" : \"1.1.0\",\n  \"relationships\": { \"requires\": { \"x\": \"1.0.0\" } }\n}\n"
        );
        assert!(set_json_version("{\"id\": \"m\"}", "1.0.0").is_none());
    }

    #[test]
    fn a_changelog_section_goes_above_the_newest_one() {
        let old = "# Changelog\n\n## 1.0.0\n\n- first\n";
        assert_eq!(
            add_changelog_section(old, "1.1.0", &["second", "third"]),
            "# Changelog\n\n## 1.1.0\n\n- second\n- third\n\n## 1.0.0\n\n- first\n"
        );
        assert_eq!(
            add_changelog_section("", "1.0.0", &["first"]),
            "# Changelog\n\n## 1.0.0\n\n- first\n\n"
        );
        assert_eq!(
            add_changelog_section("# Notes\n\nSome text.\n", "1.0.0", &["a"]),
            "# Notes\n\nSome text.\n\n## 1.0.0\n\n- a\n\n"
        );
    }
}
