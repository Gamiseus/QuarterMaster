//! The publishing rule (spec 8.2): a repository is a mod source only when it is registered or has
//! the topic, a `qm.toml` `[publish]` table names it exactly, and its latest release holds an
//! archive with that mod's id.
use crate::scan::{scan_archive, Action, ArchiveScan};
use crate::site::{image_type, CardImage};
use crate::source::{RepoInfo, Source};
use qm_schema::{IndexEntry, Listing, ModManifest, PublishInfo, SecurityCapability};
use serde::Deserialize;
use std::io::Read;

pub const TOPIC: &str = "quartermaster-mod";
/// `qm.toml` files deeper than this are not looked at (`<Mod>/<version>/qm.toml` is the deepest
/// common layout).
const MAX_DEPTH: usize = 3;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub full_name: String,
    pub registered: bool,
    /// Scan findings the site's owner allows for this repository (its registry entry's `allow`).
    pub allow: Vec<String>,
}

impl Candidate {
    pub fn new(full_name: impl Into<String>, registered: bool) -> Self {
        Self {
            full_name: full_name.into(),
            registered,
            allow: Vec::new(),
        }
    }
}

/// One mod a repository publishes.
#[derive(Debug, Clone)]
pub struct Published {
    pub entry: IndexEntry,
    /// Where its `qm.toml` sits (for README lookup).
    pub dir: String,
    /// `(mod id, version range)` from `mod.json` `relationships`, sorted by id.
    pub requires: Vec<(String, String)>,
    pub conflicts: Vec<(String, String)>,
    /// The `[publish]` table's `image`, when it is a picture the site can show.
    pub image: Option<CardImage>,
    /// What the mod's code is able to do, by QuarterMaster's ratings.
    pub capabilities: Vec<SecurityCapability>,
}

/// A mod that met the publishing rule but that the safety scan keeps off the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub id: String,
    pub name: String,
    pub version: String,
    pub asset: String,
    /// The archive that was scanned.
    pub sha256: String,
    /// A finding rejects it outright; otherwise it waits for the site's owner.
    pub reject: bool,
    /// Each finding, in words (at most `MAX_REASONS`, then a count of the rest).
    pub reasons: Vec<String>,
    /// The finding kinds, for the `allow` list.
    pub kinds: Vec<String>,
}

/// The most findings spelled out for one mod.
pub const MAX_REASONS: usize = 12;

/// The picture `[publish] image` names, or why it cannot be used. The mod is listed either way.
fn card_image(
    src: &dyn Source,
    repo: &RepoInfo,
    tree: &[String],
    dir: &str,
    image: &str,
) -> Result<CardImage, String> {
    let path = join(dir, image);
    if !tree.contains(&path) {
        return Err(format!(
            "`{path}` (the `image`) is not in the repository, so the mod's card has no picture."
        ));
    }
    let bytes = src
        .bytes(repo, &path, qm_schema::publish::MAX_IMAGE_BYTES)
        .and_then(|b| b.ok_or_else(|| format!("{path}: not found")))
        .map_err(|e| format!("The `image` could not be used: {e}."))?;
    match image_type(&bytes) {
        Some(ext) => Ok(CardImage { ext, bytes }),
        None => Err(format!(
            "`{path}` (the `image`) is not a PNG, JPEG, WebP or GIF picture."
        )),
    }
}

fn sorted_pairs(m: &std::collections::HashMap<String, String>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = m.iter().map(|(k, r)| (k.clone(), r.clone())).collect();
    v.sort();
    v
}

/// What the builder found for one repository.
#[derive(Debug, Clone, Default)]
pub struct RepoReport {
    pub full_name: String,
    pub repo: Option<RepoInfo>,
    pub mods: Vec<Published>,
    /// The latest release's notes (Markdown).
    pub release_notes: String,
    /// Why mods (or the whole repository) were not listed, in words an author can act on.
    pub problems: Vec<String>,
    /// Mods the safety scan keeps off the list. Each also has a line in `problems`.
    pub held: Vec<Held>,
}

#[derive(Deserialize)]
struct QmToml {
    publish: Option<PublishInfo>,
}

fn depth(path: &str) -> usize {
    path.matches('/').count()
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// The mod ids an archive holds: every `mod.json` in it (a `.qmpack` keeps them under `mods/<id>/`).
fn archive_mod_ids(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("not a zip archive ({e})"))?;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        if !f
            .name()
            .rsplit('/')
            .next()
            .is_some_and(|n| n.eq_ignore_ascii_case("mod.json"))
            || f.size() > 1024 * 1024
        {
            continue;
        }
        let mut raw = String::new();
        if f.read_to_string(&mut raw).is_ok() {
            if let Ok(m) = ModManifest::from_json_str(&raw) {
                out.push((m.id, m.version));
            }
        }
    }
    Ok(out)
}

/// A downloaded asset: its hash and size, the `(id, version)` of each mod in it, and its scan.
struct Archive {
    sha256: String,
    size: u64,
    ids: Vec<(String, String)>,
    scan: ArchiveScan,
}

pub fn verify(src: &dyn Source, c: &Candidate, page_base: &str) -> RepoReport {
    let mut r = RepoReport {
        full_name: c.full_name.clone(),
        ..Default::default()
    };
    let repo = match src.repo(&c.full_name) {
        Ok(Some(repo)) => repo,
        Ok(None) => {
            r.problems
                .push("The repository was not found (it may be private or renamed).".into());
            return r;
        }
        Err(e) => {
            r.problems.push(format!("GitHub could not be read: {e}"));
            return r;
        }
    };
    if !c.registered && !repo.topics.iter().any(|t| t == TOPIC) {
        r.problems.push(format!(
            "It is not registered and does not have the `{TOPIC}` topic."
        ));
        r.repo = Some(repo);
        return r;
    }
    let tree = match src.tree(&repo) {
        Ok(t) => t,
        Err(e) => {
            r.problems
                .push(format!("The file list could not be read: {e}"));
            r.repo = Some(repo);
            return r;
        }
    };
    let mut claims: Vec<(String, PublishInfo, ModManifest)> = Vec::new();
    for path in tree
        .iter()
        .filter(|p| p.rsplit('/').next() == Some("qm.toml") && depth(p) <= MAX_DEPTH)
    {
        let Ok(Some(raw)) = src.file(&repo, path) else {
            continue;
        };
        let publish = match toml::from_str::<QmToml>(&raw) {
            Ok(QmToml { publish: Some(p) }) => p,
            Ok(_) => continue,
            Err(e) => {
                r.problems.push(format!("`{path}` is not valid TOML: {e}"));
                continue;
            }
        };
        if !publish.names(&repo.full_name) {
            r.problems.push(format!(
                "`{path}` publishes `{}`, not `{}`. A fork or mirror is listed only once its own `[publish]` table names it.",
                publish.repository, repo.full_name
            ));
            continue;
        }
        let problems = publish.problems();
        if !problems.is_empty() {
            r.problems
                .extend(problems.into_iter().map(|p| format!("`{path}`: {p}")));
            continue;
        }
        let mod_json = join(dir_of(path), "mod.json");
        let Some(manifest) = src
            .file(&repo, &mod_json)
            .ok()
            .flatten()
            .and_then(|raw| ModManifest::from_json_str(&raw).ok())
        else {
            r.problems.push(format!(
                "`{mod_json}` is missing or not valid, so the mod's id is unknown."
            ));
            continue;
        };
        claims.push((dir_of(path).to_string(), publish, manifest));
    }
    if claims.is_empty() {
        if r.problems.is_empty() {
            r.problems
                .push("No `qm.toml` has a `[publish]` table naming this repository.".into());
        }
        r.repo = Some(repo);
        return r;
    }
    let release = match src.latest_release(&repo.full_name) {
        Ok(Some(rel)) => rel,
        Ok(None) => {
            r.problems.push(
                "It has no release yet. Publish a release with the mod archive attached.".into(),
            );
            r.repo = Some(repo);
            return r;
        }
        Err(e) => {
            r.problems
                .push(format!("The latest release could not be read: {e}"));
            r.repo = Some(repo);
            return r;
        }
    };
    let mut downloaded: std::collections::HashMap<String, Result<Archive, String>> =
        Default::default();
    for (dir, publish, manifest) in claims {
        let asset = match &publish.asset {
            Some(name) => release.assets.iter().find(|a| &a.name == name),
            None => release.assets.iter().find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.ends_with(".zip") || n.ends_with(".qmpack")
            }),
        };
        let Some(asset) = asset else {
            r.problems.push(match &publish.asset {
                Some(n) => format!("The release `{}` has no asset named `{n}`.", release.tag),
                None => format!(
                    "The release `{}` has no .zip or .qmpack asset.",
                    release.tag
                ),
            });
            continue;
        };
        let got = downloaded.entry(asset.url.clone()).or_insert_with(|| {
            let bytes = src.download(&asset.url)?;
            Ok(Archive {
                sha256: qm_schema::sha256_hex(&bytes),
                size: bytes.len() as u64,
                ids: archive_mod_ids(&bytes)?,
                scan: scan_archive(&bytes),
            })
        });
        let Archive {
            sha256,
            size,
            ids,
            scan,
        } = match got {
            Ok(x) => x,
            Err(e) => {
                r.problems
                    .push(format!("`{}` could not be read: {e}", asset.name));
                continue;
            }
        };
        let Some((_, version)) = ids.iter().find(|(id, _)| *id == manifest.id) else {
            r.problems.push(format!(
                "`{}` does not hold the mod `{}` (it holds {}).",
                asset.name,
                manifest.id,
                if ids.is_empty() {
                    "no mod.json".to_string()
                } else {
                    ids.iter()
                        .map(|(i, _)| format!("`{i}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
            continue;
        };
        let blocking = scan.blocking(&c.allow);
        if !blocking.is_empty() {
            let reject = blocking.iter().any(|f| f.action == Action::Reject);
            let mut reasons: Vec<String> = blocking
                .iter()
                .take(MAX_REASONS)
                .map(|f| f.text())
                .collect();
            if blocking.len() > MAX_REASONS {
                reasons.push(format!("and {} more.", blocking.len() - MAX_REASONS));
            }
            let mut kinds: Vec<String> = blocking.iter().map(|f| f.kind.to_string()).collect();
            kinds.sort();
            kinds.dedup();
            r.problems.push(format!(
                "`{}` {} {} the safety check ({}), so it is not listed.",
                manifest.id,
                version,
                if reject { "fails" } else { "is held by" },
                kinds.join(", ")
            ));
            r.held.push(Held {
                id: manifest.id.clone(),
                name: manifest.name.clone(),
                version: version.clone(),
                asset: asset.name.clone(),
                sha256: sha256.clone(),
                reject,
                reasons,
                kinds,
            });
            continue;
        }
        let entry = IndexEntry {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            repository: repo.full_name.clone(),
            authors: manifest.authors.clone(),
            summary: publish.summary.clone().unwrap_or_else(|| {
                let d = if manifest.description.is_empty() {
                    repo.description.clone()
                } else {
                    manifest.description.clone()
                };
                d.lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(qm_schema::publish::MAX_SUMMARY_CHARS)
                    .collect()
            }),
            topics: repo
                .topics
                .iter()
                .filter(|t| *t != TOPIC)
                .cloned()
                .collect(),
            license: repo.license.clone(),
            latest_version: version.clone(),
            release_tag: release.tag.clone(),
            released_at: release.published_at.clone(),
            asset_name: asset.name.clone(),
            download_url: asset.url.clone(),
            sha256: sha256.clone(),
            size_bytes: *size,
            page_url: format!("{}/mods/{}/", page_base.trim_end_matches('/'), manifest.id),
            listing: if c.registered {
                Listing::Registered
            } else {
                Listing::Discovered
            },
        };
        let image = publish.image.as_deref().and_then(|i| {
            card_image(src, &repo, &tree, &dir, i)
                .map_err(|e| r.problems.push(e))
                .ok()
        });
        r.mods.push(Published {
            entry,
            dir,
            image,
            capabilities: scan.capabilities.clone(),
            requires: sorted_pairs(&manifest.relationships.requires),
            conflicts: sorted_pairs(&manifest.relationships.conflicts),
        });
    }
    r.release_notes = release.notes.clone();
    r.repo = Some(repo);
    r
}
