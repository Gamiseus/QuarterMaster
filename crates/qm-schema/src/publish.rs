//! The `[publish]` table in a mod's `qm.toml` (spec 8.2): the author's claim that a repository
//! publishes the mod. The site builder lists a mod only when the claim matches the repository it
//! found the file in, so forks and mirrors never appear by accident.
use serde::{Deserialize, Serialize};

pub const MAX_SUMMARY_CHARS: usize = 140;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishInfo {
    /// `owner/name` of the repository that publishes the mod.
    pub repository: String,
    /// The release asset to install; otherwise the first `.zip` or `.qmpack`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    /// Release channel; only `stable` is used today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// One line for the site's mod list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// A picture for the mod's card on the site: a file in the mod's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// The largest picture the site copies for a mod's card.
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;
/// The file types a card picture may be.
pub const IMAGE_TYPES: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

impl PublishInfo {
    /// Problems with the table, in words an author can act on; empty when it is valid.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !is_repo_name(&self.repository) {
            out.push(format!(
                "`repository` must be \"owner/name\" (got \"{}\").",
                self.repository
            ));
        }
        if let Some(a) = &self.asset {
            let lower = a.to_ascii_lowercase();
            if a.contains(['/', '\\']) || !(lower.ends_with(".zip") || lower.ends_with(".qmpack")) {
                out.push(format!(
                    "`asset` must be a .zip or .qmpack file name (got \"{a}\")."
                ));
            }
        }
        if let Some(c) = &self.channel {
            if c != "stable" {
                out.push(format!("`channel` must be \"stable\" (got \"{c}\")."));
            }
        }
        if let Some(s) = &self.summary {
            if s.chars().count() > MAX_SUMMARY_CHARS {
                out.push(format!(
                    "`summary` must be at most {MAX_SUMMARY_CHARS} characters."
                ));
            }
            if s.contains(['\n', '\r']) {
                out.push("`summary` must be one line.".to_string());
            }
        }
        if let Some(i) = &self.image {
            let inside = !i.is_empty()
                && !i.starts_with('/')
                && !i.contains(['\\', ':'])
                && i.split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..");
            let known = i
                .rsplit_once('.')
                .is_some_and(|(_, ext)| IMAGE_TYPES.contains(&ext.to_ascii_lowercase().as_str()));
            if !inside || !known {
                out.push(format!(
                    "`image` must be a .png, .jpg, .webp or .gif file in the mod's folder, such as \"preview.png\" (got \"{i}\")."
                ));
            }
        }
        out
    }

    /// The claim names `full_name` (GitHub names are not case-sensitive).
    pub fn names(&self, full_name: &str) -> bool {
        self.repository
            .trim()
            .eq_ignore_ascii_case(full_name.trim())
    }
}

/// `owner/name` with GitHub's allowed characters.
pub fn is_repo_name(s: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    matches!(s.split_once('/'), Some((o, n)) if ok(o) && ok(n) && !n.contains('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct QmToml {
        publish: Option<PublishInfo>,
    }

    #[test]
    fn publish_table_parses_and_validates() {
        let raw = "[manifest]\nversion = \"1.0.0\"\n\n[publish]\nrepository = \"Gamiseus/MnB-Mods\"\nasset = \"GCore.zip\"\nsummary = \"Core framework.\"\n";
        let p = toml::from_str::<QmToml>(raw).unwrap().publish.unwrap();
        assert!(p.problems().is_empty(), "{:?}", p.problems());
        assert!(p.names("gamiseus/mnb-mods"));
        assert!(!p.names("someone/MnB-Mods"));

        let bad = PublishInfo {
            repository: "not a repo".into(),
            asset: Some("dir/x.exe".into()),
            channel: Some("beta".into()),
            summary: Some("x".repeat(141)),
            image: Some("../other/preview.png".into()),
        };
        assert_eq!(bad.problems().len(), 5);
        for (image, ok) in [
            ("preview.png", true),
            ("art/Card.JPG", true),
            ("preview.svg", false),
            ("/preview.png", false),
            ("art\\preview.png", false),
            ("https://example.com/a.png", false),
            ("preview", false),
        ] {
            let p = PublishInfo {
                repository: "a/b".into(),
                image: Some(image.into()),
                ..Default::default()
            };
            assert_eq!(p.problems().is_empty(), ok, "{image}");
        }
        assert!(toml::from_str::<QmToml>("[manifest]\nversion = \"1\"\n")
            .unwrap()
            .publish
            .is_none());
    }
}
