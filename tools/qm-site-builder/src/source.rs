//! Where repository data comes from: the GitHub API inside the site's workflow (with the
//! workflow's own token, never the launcher's), or recorded fixtures for tests and local
//! previews. The launcher itself never uses this code.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RepoInfo {
    pub full_name: String,
    pub description: String,
    pub default_branch: String,
    pub topics: Vec<String>,
    /// SPDX id, when GitHub detected one.
    pub license: Option<String>,
    pub fork: bool,
    pub archived: bool,
    /// When the repository was made (RFC 3339); empty when unknown.
    pub created_at: String,
}

/// A GitHub account, as far as a registration looks at one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// When the account was made (RFC 3339).
    pub created_at: String,
    /// Fixtures only: the organizations the account is a public member of.
    pub orgs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    pub tag: String,
    pub published_at: String,
    /// The release description (Markdown), shown as the mod page's changelog.
    pub notes: String,
    pub assets: Vec<Asset>,
}

pub trait Source {
    /// Repositories with the `quartermaster-mod` topic.
    fn topic_repos(&self) -> Result<Vec<String>>;
    fn repo(&self, full_name: &str) -> Result<Option<RepoInfo>>;
    /// Every file path in the default branch.
    fn tree(&self, repo: &RepoInfo) -> Result<Vec<String>>;
    fn file(&self, repo: &RepoInfo, path: &str) -> Result<Option<String>>;
    /// A file that is not text, such as a card picture. An error when it is larger than `max`.
    fn bytes(&self, repo: &RepoInfo, path: &str, max: usize) -> Result<Option<Vec<u8>>>;
    fn latest_release(&self, full_name: &str) -> Result<Option<Release>>;
    fn download(&self, url: &str) -> Result<Vec<u8>>;
    fn account(&self, login: &str) -> Result<Option<Account>>;
    /// `login` is a public member of the organization `org`.
    fn is_public_member(&self, org: &str, login: &str) -> Result<bool>;
}

// ---- Fixtures ----

/// A recorded copy of GitHub, as text files:
///
/// - `topic.json`: `["owner/name", ...]`;
/// - `repos/<owner>/<name>/repo.json`: a `RepoInfo`;
/// - `repos/<owner>/<name>/files/...`: the default branch's files;
/// - `repos/<owner>/<name>/release.json`: the latest release, whose assets list the mods they
///   hold as `"mods": [{ "id", "name", "version" }]`, and may add text files as
///   `"files": { "path": "text" }`. The asset archives are made from those, byte for byte the
///   same every time;
/// - `users/<login>.json`: an `Account`.
pub struct FixtureSource {
    root: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct FixtureMod {
    id: String,
    name: String,
    version: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct FixtureAsset {
    name: String,
    mods: Vec<FixtureMod>,
    /// More text files for the archive, by path.
    files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct FixtureRelease {
    tag: String,
    published_at: String,
    notes: String,
    assets: Vec<FixtureAsset>,
}

impl FixtureSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn repo_dir(&self, full_name: &str) -> PathBuf {
        let (owner, name) = full_name.split_once('/').unwrap_or((full_name, ""));
        self.root.join("repos").join(owner).join(name)
    }

    fn release(&self, full_name: &str) -> Result<Option<FixtureRelease>> {
        read_json(&self.repo_dir(full_name).join("release.json"))
    }

    fn asset_url(full_name: &str, tag: &str, name: &str) -> String {
        format!("https://github.com/{full_name}/releases/download/{tag}/{name}")
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// A zip holding `<id>/mod.json` and `<id>/qm.toml` per mod and the asset's other files, with
/// fixed timestamps.
fn fixture_zip(asset: &FixtureAsset) -> Vec<u8> {
    let mods = &asset.mods;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts =
            zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
        for m in mods {
            let manifest = serde_json::json!({ "id": m.id, "name": m.name, "version": m.version });
            zip.start_file(format!("{}/mod.json", m.id), opts)
                .expect("zip entry");
            zip.write_all(manifest.to_string().as_bytes())
                .expect("zip write");
            zip.start_file(format!("{}/qm.toml", m.id), opts)
                .expect("zip entry");
            zip.write_all(format!("[manifest]\nversion = \"{}\"\n", m.version).as_bytes())
                .expect("zip write");
        }
        for (path, text) in &asset.files {
            zip.start_file(path.as_str(), opts).expect("zip entry");
            zip.write_all(text.as_bytes()).expect("zip write");
        }
        zip.finish().expect("zip finish");
    }
    buf.into_inner()
}

impl Source for FixtureSource {
    fn topic_repos(&self) -> Result<Vec<String>> {
        Ok(read_json(&self.root.join("topic.json"))?.unwrap_or_default())
    }

    fn repo(&self, full_name: &str) -> Result<Option<RepoInfo>> {
        read_json(&self.repo_dir(full_name).join("repo.json"))
    }

    fn tree(&self, repo: &RepoInfo) -> Result<Vec<String>> {
        let base = self.repo_dir(&repo.full_name).join("files");
        let mut out = Vec::new();
        let mut stack = vec![base.clone()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(&base) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn file(&self, repo: &RepoInfo, path: &str) -> Result<Option<String>> {
        let p = self.repo_dir(&repo.full_name).join("files").join(path);
        Ok(std::fs::read_to_string(p).ok())
    }

    fn bytes(&self, repo: &RepoInfo, path: &str, max: usize) -> Result<Option<Vec<u8>>> {
        let p = self.repo_dir(&repo.full_name).join("files").join(path);
        match std::fs::read(p) {
            Ok(b) if b.len() > max => Err(format!("{path}: larger than {} KB", max / 1024)),
            Ok(b) => Ok(Some(b)),
            Err(_) => Ok(None),
        }
    }

    fn latest_release(&self, full_name: &str) -> Result<Option<Release>> {
        Ok(self.release(full_name)?.map(|r| {
            let assets = r
                .assets
                .iter()
                .map(|a| Asset {
                    name: a.name.clone(),
                    url: Self::asset_url(full_name, &r.tag, &a.name),
                    size: fixture_zip(a).len() as u64,
                })
                .collect();
            Release {
                tag: r.tag,
                published_at: r.published_at,
                notes: r.notes,
                assets,
            }
        }))
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        // https://github.com/<owner>/<name>/releases/download/<tag>/<asset>
        let rest = url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| format!("not a fixture URL: {url}"))?;
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 6 {
            return Err(format!("not a fixture asset URL: {url}"));
        }
        let full = format!("{}/{}", parts[0], parts[1]);
        let release = self
            .release(&full)?
            .ok_or_else(|| format!("no release for {full}"))?;
        let asset = release
            .assets
            .iter()
            .find(|a| a.name == parts[5])
            .ok_or_else(|| format!("no asset {url}"))?;
        Ok(fixture_zip(asset))
    }

    fn account(&self, login: &str) -> Result<Option<Account>> {
        read_json(&self.root.join("users").join(format!("{login}.json")))
    }

    fn is_public_member(&self, org: &str, login: &str) -> Result<bool> {
        Ok(self
            .account(login)?
            .is_some_and(|a| a.orgs.iter().any(|o| o.eq_ignore_ascii_case(org))))
    }
}

// ---- GitHub, inside the site's workflow ----

/// The GitHub API with the workflow's `GITHUB_TOKEN`. Only the site's build runs this.
pub struct GitHubSource {
    token: Option<String>,
    agent: ureq::Agent,
}

const API: &str = "https://api.github.com";
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;

impl GitHubSource {
    pub fn from_env() -> Self {
        Self {
            token: std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(60))
                .build(),
        }
    }

    fn get(&self, url: &str, accept: &str) -> Result<Option<ureq::Response>> {
        let mut req = self
            .agent
            .get(url)
            .set("User-Agent", "qm-site-builder")
            .set("Accept", accept);
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.call() {
            Ok(r) => Ok(Some(r)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(format!("{url}: {e}")),
        }
    }

    fn json(&self, url: &str) -> Result<Option<serde_json::Value>> {
        match self.get(url, "application/vnd.github+json")? {
            Some(r) => r.into_json().map(Some).map_err(|e| format!("{url}: {e}")),
            None => Ok(None),
        }
    }
}

impl Source for GitHubSource {
    fn topic_repos(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for page in 1..=10 {
            let url = format!(
                "{API}/search/repositories?q=topic:quartermaster-mod&per_page=100&page={page}"
            );
            let Some(v) = self.json(&url)? else { break };
            let items = v["items"].as_array().cloned().unwrap_or_default();
            out.extend(
                items
                    .iter()
                    .filter_map(|i| i["full_name"].as_str().map(str::to_string)),
            );
            if items.len() < 100 {
                break;
            }
        }
        Ok(out)
    }

    fn repo(&self, full_name: &str) -> Result<Option<RepoInfo>> {
        Ok(self
            .json(&format!("{API}/repos/{full_name}"))?
            .map(|v| RepoInfo {
                full_name: v["full_name"].as_str().unwrap_or(full_name).to_string(),
                description: v["description"].as_str().unwrap_or_default().to_string(),
                default_branch: v["default_branch"].as_str().unwrap_or("main").to_string(),
                topics: v["topics"]
                    .as_array()
                    .map(|t| {
                        t.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                license: v["license"]["spdx_id"]
                    .as_str()
                    .filter(|s| *s != "NOASSERTION")
                    .map(str::to_string),
                fork: v["fork"].as_bool().unwrap_or(false),
                archived: v["archived"].as_bool().unwrap_or(false),
                created_at: v["created_at"].as_str().unwrap_or_default().to_string(),
            }))
    }

    fn tree(&self, repo: &RepoInfo) -> Result<Vec<String>> {
        let url = format!(
            "{API}/repos/{}/git/trees/{}?recursive=1",
            repo.full_name, repo.default_branch
        );
        Ok(self
            .json(&url)?
            .and_then(|v| v["tree"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter(|e| e["type"] == "blob")
            .filter_map(|e| e["path"].as_str().map(str::to_string))
            .collect())
    }

    fn file(&self, repo: &RepoInfo, path: &str) -> Result<Option<String>> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{path}",
            repo.full_name, repo.default_branch
        );
        match self.get(&url, "text/plain")? {
            Some(r) => r.into_string().map(Some).map_err(|e| format!("{url}: {e}")),
            None => Ok(None),
        }
    }

    fn bytes(&self, repo: &RepoInfo, path: &str, max: usize) -> Result<Option<Vec<u8>>> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{path}",
            repo.full_name, repo.default_branch
        );
        let Some(resp) = self.get(&url, "application/octet-stream")? else {
            return Ok(None);
        };
        let mut out = Vec::new();
        resp.into_reader()
            .take(max as u64 + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("{url}: {e}"))?;
        if out.len() > max {
            return Err(format!("{path}: larger than {} KB", max / 1024));
        }
        Ok(Some(out))
    }

    fn latest_release(&self, full_name: &str) -> Result<Option<Release>> {
        Ok(self
            .json(&format!("{API}/repos/{full_name}/releases/latest"))?
            .map(|v| Release {
                tag: v["tag_name"].as_str().unwrap_or_default().to_string(),
                published_at: v["published_at"].as_str().unwrap_or_default().to_string(),
                notes: v["body"].as_str().unwrap_or_default().to_string(),
                assets: v["assets"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|a| Asset {
                        name: a["name"].as_str().unwrap_or_default().to_string(),
                        url: a["browser_download_url"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        size: a["size"].as_u64().unwrap_or(0),
                    })
                    .collect(),
            }))
    }

    fn download(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self
            .get(url, "application/octet-stream")?
            .ok_or_else(|| format!("{url}: not found"))?;
        let mut out = Vec::new();
        resp.into_reader()
            .take(MAX_ASSET_BYTES + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("{url}: {e}"))?;
        if out.len() as u64 > MAX_ASSET_BYTES {
            return Err(format!(
                "{url}: larger than {} MiB",
                MAX_ASSET_BYTES / 1024 / 1024
            ));
        }
        Ok(out)
    }

    fn account(&self, login: &str) -> Result<Option<Account>> {
        Ok(self
            .json(&format!("{API}/users/{login}"))?
            .map(|v| Account {
                created_at: v["created_at"].as_str().unwrap_or_default().to_string(),
                orgs: Vec::new(),
            }))
    }

    fn is_public_member(&self, org: &str, login: &str) -> Result<bool> {
        // 204 when a public member; 404 otherwise (also when `org` is a user, not an organization).
        Ok(self
            .get(
                &format!("{API}/orgs/{org}/public_members/{login}"),
                "application/vnd.github+json",
            )?
            .is_some_and(|r| r.status() == 204))
    }
}

/// `owner/name` keys (lower case) to their registry entries, sorted, so output order never
/// depends on the disk.
pub type Registry = BTreeMap<String, RegistryEntry>;

/// One registered repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryEntry {
    pub repository: String,
    pub contact: Option<String>,
    /// Scan findings the site's owner allows for this repository.
    pub allow: Vec<String>,
}

/// The most repositories one registry file may list.
pub const MAX_REPOSITORIES: usize = 10;

/// The `allow` key: a list for every repository in the file, or a table with a list per
/// repository.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Allow {
    All(Vec<String>),
    Each(BTreeMap<String, Vec<String>>),
}

/// A file in `registry/`: one author's repositories.
///
/// ```toml
/// repository = "owner/name"              # one repository, or
/// repositories = ["owner/a", "owner/b"]  # several
/// contact = "optional: how to reach you"
/// allow = ["native_ffi"]                 # set by the site's owner only
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RegistryFile {
    pub repository: Option<String>,
    pub repositories: Vec<String>,
    pub contact: Option<String>,
    pub allow: Option<Allow>,
}

impl RegistryFile {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| e.message().to_string())
    }

    /// The repositories in file order, each once (names compare without letter case).
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in self.repository.iter().chain(&self.repositories) {
            let r = r.trim();
            if !out.iter().any(|o| o.eq_ignore_ascii_case(r)) {
                out.push(r.to_string());
            }
        }
        out
    }

    /// What is wrong with the file, in words an author can act on; empty when it is valid.
    pub fn problems(&self) -> Vec<String> {
        let names = self.names();
        let mut out = Vec::new();
        if names.is_empty() {
            out.push(
                "It lists no repository. Add `repository = \"owner/name\"`, or `repositories = [\"owner/a\", \"owner/b\"]` for several."
                    .to_string(),
            );
        }
        if names.len() > MAX_REPOSITORIES {
            out.push(format!(
                "It lists {} repositories; one file holds at most {MAX_REPOSITORIES}.",
                names.len()
            ));
        }
        for n in &names {
            if !qm_schema::is_repo_name(n) {
                out.push(format!("\"{n}\" is not an \"owner/name\" repository."));
            }
        }
        if let Some(Allow::Each(map)) = &self.allow {
            for key in map.keys() {
                if !names.iter().any(|n| n.eq_ignore_ascii_case(key)) {
                    out.push(format!(
                        "`allow` names \"{key}\", which is not a repository in this file."
                    ));
                }
            }
        }
        out
    }

    /// The findings allowed for `repository`.
    pub fn allow_for(&self, repository: &str) -> Vec<String> {
        match &self.allow {
            None => Vec::new(),
            Some(Allow::All(list)) => list.clone(),
            Some(Allow::Each(map)) => map
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(repository))
                .map(|(_, v)| v.clone())
                .unwrap_or_default(),
        }
    }

    pub fn entries(&self) -> Vec<RegistryEntry> {
        self.names()
            .into_iter()
            .map(|repository| RegistryEntry {
                allow: self.allow_for(&repository),
                contact: self.contact.clone(),
                repository,
            })
            .collect()
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn one_repository_or_several() {
        let one = RegistryFile::parse("repository = \"alice/guns\"\ncontact = \"a@example.com\"\n")
            .unwrap();
        assert!(one.problems().is_empty());
        assert_eq!(one.names(), ["alice/guns"]);
        assert_eq!(one.entries()[0].contact.as_deref(), Some("a@example.com"));

        let many = RegistryFile::parse(
            "repositories = [\"alice/guns\", \"alice/maps\", \"Alice/Guns\"]\n\n[allow]\n\"alice/maps\" = [\"native_ffi\"]\n",
        )
        .unwrap();
        assert!(many.problems().is_empty(), "{:?}", many.problems());
        assert_eq!(many.names(), ["alice/guns", "alice/maps"]);
        let entries = many.entries();
        assert!(entries[0].allow.is_empty());
        assert_eq!(entries[1].allow, ["native_ffi"]);
        // A plain list allows the kinds for every repository in the file.
        let all =
            RegistryFile::parse("repositories = [\"a/x\", \"a/y\"]\nallow = [\"script_file\"]\n")
                .unwrap();
        assert_eq!(all.allow_for("A/Y"), ["script_file"]);
    }

    #[test]
    fn bad_files_say_why() {
        assert!(RegistryFile::parse("repository = \"a/b\"\nsurprise = 1\n")
            .unwrap_err()
            .contains("unknown field"));
        assert_eq!(
            RegistryFile::parse("contact = \"x\"\n")
                .unwrap()
                .problems()
                .len(),
            1
        );
        let bad =
            RegistryFile::parse("repositories = [\"not a repo\"]\n\n[allow]\n\"a/other\" = []\n")
                .unwrap();
        assert_eq!(bad.problems().len(), 2, "{:?}", bad.problems());
        let list: Vec<String> = (0..11).map(|i| format!("\"a/r{i}\"")).collect();
        let many = RegistryFile::parse(&format!("repositories = [{}]\n", list.join(", "))).unwrap();
        assert!(many.problems()[0].contains("at most 10"));
    }
}
