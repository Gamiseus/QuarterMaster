//! The mod index served at `https://gamiseus.github.io/api/v1/index.json` (spec 8.3). The site
//! builder writes it and the launcher reads it, both through these types.
use serde::{Deserialize, Serialize};

pub const INDEX_SCHEMA_VERSION: u32 = 1;

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexFile {
    pub schema_version: u32,
    pub generated_at: String,
    pub mods: Vec<IndexEntry>,
}

/// How a mod got into the index: a pull request to the registry, or the GitHub topic.
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Listing {
    Registered,
    #[default]
    Discovered,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexEntry {
    /// The mod id from its `qm.toml`.
    pub id: String,
    pub name: String,
    /// `owner/name` on GitHub, as the `[publish]` table states it.
    pub repository: String,
    pub authors: Vec<String>,
    /// One line for lists.
    pub summary: String,
    pub topics: Vec<String>,
    pub license: Option<String>,
    pub latest_version: String,
    /// The release tag the version came from.
    pub release_tag: String,
    pub released_at: String,
    pub asset_name: String,
    /// The release asset, on a GitHub release host.
    pub download_url: String,
    /// Lowercase hex SHA-256 of the asset; installs are refused when the download differs.
    pub sha256: String,
    pub size_bytes: u64,
    /// The mod's page on the site.
    pub page_url: String,
    pub listing: Listing,
}

impl IndexFile {
    /// Parses and checks an index: a known schema, and every entry with an id, a download URL
    /// and a 64-character hex SHA-256.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let f: IndexFile = serde_json::from_slice(bytes)
            .map_err(|e| format!("The mod index is not valid JSON: {e}"))?;
        if f.schema_version != INDEX_SCHEMA_VERSION {
            return Err(format!("Unknown mod index schema {}.", f.schema_version));
        }
        for m in &f.mods {
            let hex = m.sha256.len() == 64 && m.sha256.bytes().all(|b| b.is_ascii_hexdigit());
            if m.id.is_empty() || m.download_url.is_empty() || !hex {
                return Err(format!("The mod index entry '{}' is incomplete.", m.id));
            }
        }
        Ok(f)
    }

    pub fn find(&self, id: &str) -> Option<&IndexEntry> {
        self.mods.iter().find(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> serde_json::Value {
        serde_json::json!({
            "id": "night-ops", "name": "Night Ops", "repository": "a/b", "latest_version": "1.0.0",
            "download_url": "https://github.com/a/b/releases/download/v1.0.0/night.zip",
            "sha256": "a".repeat(64), "listing": "registered"
        })
    }

    #[test]
    fn parses_and_checks_entries() {
        let ok = serde_json::json!({ "schema_version": 1, "generated_at": "t", "mods": [entry()] });
        let f = IndexFile::parse(ok.to_string().as_bytes()).unwrap();
        assert_eq!(f.find("night-ops").unwrap().listing, Listing::Registered);

        let mut bad = entry();
        bad["sha256"] = "xyz".into();
        let doc = serde_json::json!({ "schema_version": 1, "generated_at": "t", "mods": [bad] });
        assert!(IndexFile::parse(doc.to_string().as_bytes()).is_err());
        let future = serde_json::json!({ "schema_version": 2, "generated_at": "t", "mods": [] });
        assert!(IndexFile::parse(future.to_string().as_bytes()).is_err());
    }
}
