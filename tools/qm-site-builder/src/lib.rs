//! Builds the QuarterMaster site and its mod index (plan 2c Task 12, spec 8). Runs in the
//! `gamiseus.github.io` repository's GitHub Actions workflow; the launcher only reads the result.
pub mod highlight;
pub mod render;
pub mod review;
pub mod scan;
pub mod site;
pub mod source;
pub mod verify;
pub mod wiki;

use qm_schema::{IndexFile, INDEX_SCHEMA_VERSION};
use site::ModPage;
use source::{Registry, RegistryFile, RepoInfo, Source};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use verify::{Candidate, Held, Published, RepoReport};

/// Which repository each mod id belongs to (`id` to `owner/name`). An id is given to the first
/// repository that lists it and stays there: the launcher matches updates by id, so an id that
/// could change hands would let one author replace another's mod.
pub type Owners = BTreeMap<String, String>;

/// In `registry/`: ids the site's owner assigns by hand (`id = "owner/name"`). Not a
/// registration file.
pub const IDS_FILE: &str = "ids.toml";

/// A mod the safety scan keeps off the list, for the build's report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldMod {
    pub repository: String,
    pub held: Held,
    /// The version still listed from the previous build, when there was a different one.
    pub kept_version: Option<String>,
}

/// Everything a build found.
pub struct Build {
    pub index: IndexFile,
    pub pages: Vec<ModPage>,
    /// Repositories looked at and why mods were not listed.
    pub reports: Vec<RepoReport>,
    /// Every id's repository after this build: the ones given in, and the ids first listed now.
    pub owners: Owners,
    pub held: Vec<HeldMod>,
}

/// What the previous build left: who owns each id, and what was listed.
#[derive(Default, Clone, Copy)]
pub struct State<'a> {
    pub owners: Option<&'a Owners>,
    pub previous: Option<&'a IndexFile>,
}

fn registry_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("toml"))
                .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some(IDS_FILE))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// `registry/*.toml`: each file lists one author's repositories.
pub fn read_registry(dir: &Path) -> Result<Registry, String> {
    Ok(read_registry_files(dir)?
        .into_iter()
        .flat_map(|(_, file)| file.entries())
        .map(|e| (e.repository.to_ascii_lowercase(), e))
        .collect())
}

/// Each registry file by its file name.
pub fn read_registry_files(dir: &Path) -> Result<Vec<(String, RegistryFile)>, String> {
    let mut out = Vec::new();
    for p in registry_files(dir) {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let raw = std::fs::read_to_string(&p).map_err(|err| format!("{}: {err}", p.display()))?;
        let file = RegistryFile::parse(&raw).map_err(|err| format!("{}: {err}", p.display()))?;
        if let Some(problem) = file.problems().first() {
            return Err(format!("{}: {problem}", p.display()));
        }
        out.push((name, file));
    }
    Ok(out)
}

/// `registry/ids.toml`: ids the site's owner assigns by hand. No file means none.
pub fn read_id_overrides(dir: &Path) -> Result<Owners, String> {
    let path = dir.join(IDS_FILE);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Ok(Owners::new());
    };
    let ids: Owners = toml::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    match ids.iter().find(|(_, repo)| !qm_schema::is_repo_name(repo)) {
        Some((id, repo)) => Err(format!(
            "{}: `{id}` must be an \"owner/name\" repository (got \"{repo}\").",
            path.display()
        )),
        None => Ok(ids),
    }
}

/// `registry/denylist.txt`: one `owner/name` per line; `#` starts a comment.
pub fn read_denylist(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| {
            l.split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// The ids' repositories a build starts from: the stored ones, then every id the previous index
/// listed that has none yet.
pub fn known_owners(state: &State) -> Owners {
    let mut owners = state.owners.cloned().unwrap_or_default();
    for e in state.previous.iter().flat_map(|p| &p.mods) {
        owners
            .entry(e.id.clone())
            .or_insert_with(|| e.repository.clone());
    }
    owners
}

fn readme_of(src: &dyn Source, repo: &RepoInfo, dir: &str) -> String {
    let readme = ["README.md", "readme.md", "Readme.md"]
        .iter()
        .map(|n| {
            if dir.is_empty() {
                n.to_string()
            } else {
                format!("{dir}/{n}")
            }
        })
        .find_map(|p| src.file(repo, &p).ok().flatten())
        .or_else(|| src.file(repo, "README.md").ok().flatten())
        .unwrap_or_default();
    let base = format!(
        "https://raw.githubusercontent.com/{}/{}/{}",
        repo.full_name,
        repo.default_branch,
        if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        }
    );
    render::render_markdown(&readme, Some(&base))
}

fn mod_page(src: &dyn Source, repo: &RepoInfo, m: &Published, notes: &str) -> ModPage {
    ModPage {
        entry: m.entry.clone(),
        readme_html: readme_of(src, repo, &m.dir),
        requires: m.requires.clone(),
        conflicts: m.conflicts.clone(),
        notes_html: render::render_markdown(notes, None),
        image: m.image.clone(),
        capabilities: m
            .capabilities
            .iter()
            .map(|c| scan::capability_name(*c).to_string())
            .collect(),
        scanned: true,
        notice: None,
    }
}

/// Discover, verify, scan and hash every candidate, with no earlier build to go by.
pub fn build(
    src: &dyn Source,
    registry: &Registry,
    denylist: &BTreeSet<String>,
    site_url: &str,
    generated_at: &str,
) -> Result<Build, String> {
    build_with(
        src,
        registry,
        denylist,
        site_url,
        generated_at,
        &State::default(),
    )
}

/// Discover, verify, scan and hash every candidate (spec 8.3 steps 2 to 5).
pub fn build_with(
    src: &dyn Source,
    registry: &Registry,
    denylist: &BTreeSet<String>,
    site_url: &str,
    generated_at: &str,
    state: &State,
) -> Result<Build, String> {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen = BTreeSet::new();
    for e in registry.values() {
        if seen.insert(e.repository.to_ascii_lowercase()) {
            candidates.push(Candidate {
                full_name: e.repository.clone(),
                registered: true,
                allow: e.allow.clone(),
            });
        }
    }
    for full in src.topic_repos()? {
        if seen.insert(full.to_ascii_lowercase()) {
            candidates.push(Candidate::new(full, false));
        }
    }
    candidates.sort_by_key(|c| c.full_name.to_ascii_lowercase());

    // One report per candidate, in the same order.
    let mut reports: Vec<RepoReport> = candidates
        .iter()
        .map(|c| {
            if denylist.contains(&c.full_name.to_ascii_lowercase()) {
                RepoReport {
                    full_name: c.full_name.clone(),
                    problems: vec!["It is on the denylist.".into()],
                    ..Default::default()
                }
            } else {
                verify::verify(src, c, site_url)
            }
        })
        .collect();

    // Who lists each id. A recorded owner keeps its id whoever else claims it. An id nobody
    // owns yet goes to a registered repository before a discovered one, then to the older one.
    let mut owners = known_owners(state);
    let mut claims: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (ri, report) in reports.iter().enumerate() {
        for (mi, m) in report.mods.iter().enumerate() {
            claims.entry(m.entry.id.clone()).or_default().push((ri, mi));
        }
    }
    let mut winners: Vec<(usize, usize)> = Vec::new();
    let mut refused: Vec<(usize, String)> = Vec::new();
    for (id, claimants) in &claims {
        let winner = match owners.get(id) {
            Some(owner) => claimants
                .iter()
                .find(|(ri, _)| reports[*ri].full_name.eq_ignore_ascii_case(owner))
                .copied(),
            None => claimants
                .iter()
                .min_by_key(|(ri, _)| {
                    let created = reports[*ri]
                        .repo
                        .as_ref()
                        .map(|r| r.created_at.clone())
                        .unwrap_or_default();
                    (
                        !candidates[*ri].registered,
                        created.is_empty(),
                        created,
                        reports[*ri].full_name.to_ascii_lowercase(),
                    )
                })
                .copied(),
        };
        let holder = match winner {
            Some((ri, mi)) => {
                winners.push((ri, mi));
                let name = reports[ri].full_name.clone();
                owners.entry(id.clone()).or_insert_with(|| name.clone());
                name
            }
            None => owners.get(id).cloned().unwrap_or_default(),
        };
        for (ri, _) in claimants.iter().filter(|c| Some(**c) != winner) {
            refused.push((
                *ri,
                format!(
                    "The mod id `{id}` belongs to `{holder}`, so `{id}` from this repository is not listed. Give the mod its own id."
                ),
            ));
        }
    }
    for (ri, problem) in refused {
        reports[ri].problems.push(problem);
    }

    let mut pages: Vec<ModPage> = Vec::new();
    for (ri, mi) in winners {
        let report = &reports[ri];
        if let Some(repo) = &report.repo {
            pages.push(mod_page(src, repo, &report.mods[mi], &report.release_notes));
        }
    }

    // A release the scan holds does not take the mod off the list when an earlier, different
    // archive was listed: that one stays until the new release is cleared.
    let mut held = Vec::new();
    for report in &reports {
        for h in &report.held {
            let earlier = state
                .previous
                .and_then(|p| p.find(&h.id))
                .filter(|e| e.repository.eq_ignore_ascii_case(&report.full_name))
                .filter(|e| !e.sha256.eq_ignore_ascii_case(&h.sha256))
                .filter(|_| {
                    owners
                        .get(&h.id)
                        .is_none_or(|o| o.eq_ignore_ascii_case(&report.full_name))
                })
                .filter(|_| !pages.iter().any(|p| p.entry.id == h.id));
            if let (Some(entry), Some(repo)) = (earlier, &report.repo) {
                pages.push(ModPage {
                    entry: entry.clone(),
                    readme_html: readme_of(src, repo, ""),
                    requires: Vec::new(),
                    conflicts: Vec::new(),
                    notes_html: String::new(),
                    image: None,
                    capabilities: Vec::new(),
                    scanned: false,
                    notice: Some(format!(
                        "Version {} has been released and is being checked before it is listed. This is the version before it.",
                        h.version
                    )),
                });
            }
            held.push(HeldMod {
                repository: report.full_name.clone(),
                held: h.clone(),
                kept_version: earlier.map(|e| e.latest_version.clone()),
            });
        }
    }

    pages.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
    let index = IndexFile {
        schema_version: INDEX_SCHEMA_VERSION,
        generated_at: generated_at.to_string(),
        mods: pages.iter().map(|p| p.entry.clone()).collect(),
    };
    Ok(Build {
        index,
        pages,
        reports,
        owners,
        held,
    })
}

/// The check for one registry file, without a pull request: Markdown, and whether every
/// repository in it can be listed.
pub fn check_registration(src: &dyn Source, file: &Path, site_url: &str) -> (String, bool) {
    let parsed = std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|raw| RegistryFile::parse(&raw));
    let registry = match parsed {
        Ok(f) => f,
        Err(e) => return (format!("### Registration check failed\n\n`{}` could not be read: {e}\n\nIt must contain `repository = \"owner/name\"`.\n", file.display()), false),
    };
    if let Some(problem) = registry.problems().first() {
        return (
            format!("### Registration check failed\n\n{problem}\n\nIt must contain `repository = \"owner/name\"`.\n"),
            false,
        );
    }
    let mut md = String::new();
    let mut all_ok = true;
    for entry in registry.entries() {
        let report = verify::verify(
            src,
            &Candidate {
                full_name: entry.repository.clone(),
                registered: true,
                allow: entry.allow.clone(),
            },
            site_url,
        );
        let ok = !report.mods.is_empty();
        all_ok &= ok;
        if ok {
            md.push_str(&format!(
                "### Registration check passed\n\n`{}` publishes:\n\n",
                entry.repository
            ));
            for e in report.mods.iter().map(|m| &m.entry) {
                md.push_str(&format!(
                    "- **{}** (`{}`) {} from `{}`\n",
                    e.name, e.id, e.latest_version, e.asset_name
                ));
            }
        } else {
            md.push_str(&format!(
                "### Registration check failed\n\n`{}` cannot be listed yet.\n",
                entry.repository
            ));
        }
        if !report.problems.is_empty() {
            md.push_str("\n**To fix:**\n\n");
            for p in &report.problems {
                md.push_str(&format!("- {p}\n"));
            }
            for h in &report.held {
                md.push_str(&format!("\n`{}` {}:\n\n", h.id, h.version));
                for reason in &h.reasons {
                    md.push_str(&format!("- {reason}\n"));
                }
            }
            md.push_str(
                "\nThe author guide explains each rule: https://gamiseus.github.io/publish/\n",
            );
        }
        md.push('\n');
    }
    (md, all_ok)
}
