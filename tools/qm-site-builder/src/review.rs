//! The review of a registration pull request: whether it can be merged on its own, must wait
//! for the site's owner, or is refused, and the comment that says why.
//!
//! Everything about the pull request arrives as data: the list of files it changes and the
//! text of the one registry file. Nothing from it is run.
use crate::scan::clean;
use crate::source::{RegistryFile, Source};
use crate::verify::{self, Candidate};
use crate::{known_owners, read_registry_files, scan, State, IDS_FILE};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// A registry file is a few lines; anything larger is not one.
pub const MAX_FILE_BYTES: usize = 2048;
/// More open registrations than this from one author wait for the site's owner.
pub const MAX_OPEN_PRS: u32 = 3;
/// More automatic merges than this in one day wait for the site's owner.
pub const MAX_MERGES_PER_DAY: u32 = 20;
/// A GitHub account younger than this is confirmed by the site's owner.
pub const MIN_ACCOUNT_DAYS: i64 = 7;

/// One file of a pull request, as GitHub lists it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrFile {
    pub filename: String,
    /// `added`, `modified`, `removed`, `renamed`, ...
    pub status: String,
}

pub struct ReviewInput<'a> {
    pub files: &'a [PrFile],
    /// The registry file as the pull request has it; `None` when it could not be fetched.
    pub content: Option<&'a str>,
    /// `registry/` as it is on the main branch.
    pub registry_dir: &'a Path,
    pub denylist: &'a BTreeSet<String>,
    /// Who opened the pull request.
    pub author: &'a str,
    /// Who owns the site's repository.
    pub site_owner: &'a str,
    pub state: State<'a>,
    /// The author's open registration pull requests, this one included.
    pub open_prs: u32,
    /// Registrations merged automatically today.
    pub merged_today: u32,
    /// The time of the review (RFC 3339).
    pub now: &'a str,
    pub site_url: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Every check passed: merge it.
    Pass,
    /// Nothing is wrong that the author can fix; the site's owner decides.
    Hold,
    /// It cannot be merged as it is.
    Reject,
}

#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub verdict: Verdict,
    /// The repositories this pull request adds.
    pub repositories: Vec<String>,
    /// What the author must change.
    pub rejects: Vec<String>,
    /// What waits for the site's owner.
    pub holds: Vec<String>,
    /// The mods that would be listed.
    pub listed: Vec<String>,
    /// Things worth knowing that block nothing.
    pub notes: Vec<String>,
    /// The comment for the pull request.
    pub markdown: String,
}

/// Days since 1970-01-01 of an RFC 3339 time's date.
fn day_number(ts: &str) -> Option<i64> {
    let mut parts = ts.get(..10)?.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `registry/<stem>.toml` gives `<stem>`; any other path gives nothing.
fn registry_stem(path: &str) -> Option<&str> {
    let stem = path.strip_prefix("registry/")?.strip_suffix(".toml")?;
    let plain = !stem.is_empty()
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    plain.then_some(stem)
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

struct Lists {
    repositories: Vec<String>,
    rejects: Vec<String>,
    holds: Vec<String>,
    listed: Vec<String>,
    notes: Vec<String>,
}

fn run(src: &dyn Source, input: &ReviewInput, out: &mut Lists) {
    // 1. The shape of the pull request.
    let [file] = input.files else {
        let names: Vec<String> = input
            .files
            .iter()
            .take(10)
            .map(|f| format!("`{}`", f.filename))
            .collect();
        out.rejects.push(format!(
            "A registration changes exactly one file, `registry/<owner>.toml`. This pull request changes {}: {}.",
            input.files.len(),
            names.join(", ")
        ));
        return;
    };
    let Some(stem) = registry_stem(&file.filename).filter(|s| format!("{s}.toml") != IDS_FILE)
    else {
        out.rejects.push(format!(
            "`{}` is not a registration file. Add `registry/<owner>.toml` (or `registry/<owner>-<name>.toml`).",
            file.filename
        ));
        return;
    };
    if !matches!(file.status.as_str(), "added" | "modified") {
        out.rejects.push(format!(
            "A registration adds a file or adds repositories to one. This pull request has `{}` as {}.",
            file.filename, file.status
        ));
        return;
    }
    let Some(content) = input.content else {
        out.rejects
            .push(format!("`{}` could not be read.", file.filename));
        return;
    };
    if content.len() > MAX_FILE_BYTES {
        out.rejects.push(format!(
            "`{}` is {} bytes; a registration file is at most {MAX_FILE_BYTES}.",
            file.filename,
            content.len()
        ));
        return;
    }
    let registry = match RegistryFile::parse(content) {
        Ok(r) => r,
        Err(e) => {
            out.rejects.push(format!(
                "`{}` is not a valid registration file: {e}. It holds `repository = \"owner/name\"`, or `repositories = [\"owner/a\", \"owner/b\"]`, and optionally `contact`.",
                file.filename
            ));
            return;
        }
    };
    let problems = registry.problems();
    if !problems.is_empty() {
        out.rejects.extend(problems);
        return;
    }
    let names = registry.names();
    let owner = names[0].split('/').next().unwrap_or_default().to_string();
    if let Some(other) = names
        .iter()
        .find(|n| !eq(n.split('/').next().unwrap_or_default(), &owner))
    {
        out.rejects.push(format!(
            "One file lists one owner's repositories. `{other}` is not `{owner}`'s; give it its own file."
        ));
        return;
    }
    let stem_fits = eq(stem, &owner)
        || stem
            .to_ascii_lowercase()
            .starts_with(&format!("{}-", owner.to_ascii_lowercase()));
    if !stem_fits {
        out.rejects.push(format!(
            "Name the file after the repositories' owner: `registry/{owner}.toml` (or `registry/{owner}-<name>.toml`), not `{}`.",
            file.filename
        ));
        return;
    }

    // What the file was before, and what the other files hold.
    let files = match read_registry_files(input.registry_dir) {
        Ok(f) => f,
        Err(e) => {
            out.holds.push(format!(
                "The registry on the main branch could not be read: {e}"
            ));
            return;
        }
    };
    let this_name = format!("{stem}.toml");
    let before = files
        .iter()
        .find(|(name, _)| eq(name, &this_name))
        .map(|(_, f)| f.clone())
        .unwrap_or_default();
    let before_names = before.names();
    let added: Vec<String> = names
        .iter()
        .filter(|n| !before_names.iter().any(|b| eq(b, n)))
        .cloned()
        .collect();
    for gone in before_names
        .iter()
        .filter(|b| !names.iter().any(|n| eq(n, b)))
    {
        out.holds.push(format!(
            "It takes `{gone}` out of the registry. The site's owner confirms removals."
        ));
    }
    let by_site_owner = eq(input.author, input.site_owner);
    if registry.allow != before.allow && !by_site_owner {
        out.holds.push(
            "`allow` is set by the site's owner. Take it out of the file, and say in the pull request what the mod needs and why."
                .to_string(),
        );
    }
    for repo in &added {
        if input.denylist.contains(&repo.to_ascii_lowercase()) {
            out.rejects.push(format!("`{repo}` cannot be listed."));
        }
        if let Some((name, _)) = files
            .iter()
            .filter(|(name, _)| !eq(name, &this_name))
            .find(|(_, f)| f.names().iter().any(|n| eq(n, repo)))
        {
            out.rejects.push(format!(
                "`{repo}` is already registered, in `registry/{name}`."
            ));
        }
    }
    if !out.rejects.is_empty() {
        return;
    }
    out.repositories = added.clone();
    if added.is_empty() && out.holds.is_empty() {
        out.notes
            .push("No repository is added; only the file's other details change.".to_string());
    }

    // 2. Who is asking.
    let theirs = eq(input.author, &owner)
        || by_site_owner
        || src.is_public_member(&owner, input.author).unwrap_or(false);
    if !theirs {
        out.holds.push(format!(
            "`{}` opened this for `{owner}`'s repositories. If `{owner}` is an organization, make your membership of it public and push again; otherwise the site's owner confirms it.",
            input.author
        ));
    }
    if !by_site_owner {
        let age = src
            .account(input.author)
            .ok()
            .flatten()
            .and_then(|a| Some(day_number(input.now)? - day_number(&a.created_at)?));
        match age {
            Some(days) if days >= MIN_ACCOUNT_DAYS => {}
            Some(days) => out.holds.push(format!(
                "The GitHub account `{}` is {days} days old. Accounts under {MIN_ACCOUNT_DAYS} days are confirmed by the site's owner.",
                input.author
            )),
            None => out.holds.push(format!(
                "The age of the GitHub account `{}` could not be read.",
                input.author
            )),
        }
        if input.open_prs > MAX_OPEN_PRS {
            out.holds.push(format!(
                "`{}` has {} registrations open; over {MAX_OPEN_PRS} wait for the site's owner. One file can list up to {} repositories.",
                input.author,
                input.open_prs,
                crate::source::MAX_REPOSITORIES
            ));
        }
        if input.merged_today >= MAX_MERGES_PER_DAY {
            out.holds.push(format!(
                "{MAX_MERGES_PER_DAY} registrations were merged automatically today, which is the daily limit. The site's owner merges this one."
            ));
        }
    }

    // 3 to 5. Each added repository: it exists, it meets the publishing rule, and its archive
    // passes the safety scan.
    let owners = known_owners(&input.state);
    for repo in &added {
        let candidate = Candidate {
            full_name: repo.clone(),
            registered: true,
            // Only what the main branch already allows counts, unless the site's owner asks.
            allow: if by_site_owner {
                registry.allow_for(repo)
            } else {
                before.allow_for(repo)
            },
        };
        let report = verify::verify(src, &candidate, input.site_url);
        let Some(info) = &report.repo else {
            out.rejects
                .push(format!("`{repo}`: {}", report.problems.join(" ")));
            continue;
        };
        if info.archived {
            out.rejects.push(format!(
                "`{repo}` is archived. Unarchive it to register it."
            ));
            continue;
        }
        if report.mods.is_empty() && report.held.is_empty() {
            out.rejects
                .extend(report.problems.iter().map(|p| format!("`{repo}`: {p}")));
            continue;
        }
        for h in &report.held {
            let reasons: String = h.reasons.iter().map(|r| format!("\n  - {r}")).collect();
            if h.reject {
                out.rejects.push(format!(
                    "`{repo}`: **{}** (`{}`) {} fails the safety check:{reasons}",
                    clean(&h.name),
                    clean(&h.id),
                    clean(&h.version)
                ));
            } else {
                out.holds.push(format!(
                    "`{repo}`: **{}** (`{}`) {} needs the site's owner to allow `{}`:{reasons}",
                    clean(&h.name),
                    clean(&h.id),
                    clean(&h.version),
                    h.kinds.join("`, `")
                ));
            }
        }
        out.notes.extend(
            report
                .problems
                .iter()
                .filter(|p| !p.contains("the safety check"))
                .map(|p| format!("`{repo}`: {p}")),
        );
        for m in &report.mods {
            let e = &m.entry;
            match owners.get(&e.id).filter(|o| !eq(o, repo)) {
                Some(other) => out.holds.push(format!(
                    "`{repo}`: the mod id `{}` is already listed from `{other}`. Give the mod its own id, or the site's owner moves the id.",
                    clean(&e.id)
                )),
                None => {
                    let can: Vec<&str> = m
                        .capabilities
                        .iter()
                        .map(|c| scan::capability_name(*c))
                        .collect();
                    out.listed.push(format!(
                        "**{}** (`{}`) {} from `{}` in `{repo}`. {}",
                        clean(&e.name),
                        clean(&e.id),
                        clean(&e.latest_version),
                        clean(&e.asset_name),
                        if can.is_empty() {
                            "Plain sandboxed Lua.".to_string()
                        } else {
                            format!("Can do: {}.", can.join(", ").to_lowercase())
                        }
                    ));
                }
            }
        }
    }
}

/// Reviews a registration pull request.
pub fn review(src: &dyn Source, input: &ReviewInput) -> Review {
    let mut lists = Lists {
        repositories: Vec::new(),
        rejects: Vec::new(),
        holds: Vec::new(),
        listed: Vec::new(),
        notes: Vec::new(),
    };
    run(src, input, &mut lists);
    // Whatever a mod's own text says, the comment mentions nobody.
    for list in [
        &mut lists.rejects,
        &mut lists.holds,
        &mut lists.listed,
        &mut lists.notes,
    ] {
        for line in list.iter_mut() {
            *line = line.replace('@', "");
        }
    }
    let verdict = if !lists.rejects.is_empty() {
        Verdict::Reject
    } else if !lists.holds.is_empty() {
        Verdict::Hold
    } else {
        Verdict::Pass
    };

    let section = |title: &str, items: &[String]| -> String {
        if items.is_empty() {
            return String::new();
        }
        let list: String = items.iter().map(|i| format!("- {i}\n")).collect();
        format!("\n**{title}**\n\n{list}")
    };
    let mut md = match verdict {
        Verdict::Pass => "### Registration accepted\n\nEvery check passed. This pull request is merged automatically, and the mods appear on the site and in QuarterMaster's Browse tab within the hour.\n".to_string(),
        Verdict::Hold => "### Registration is waiting for the site's owner\n\nNothing below needs a change from you unless it says so. The site's owner has been asked to look.\n".to_string(),
        Verdict::Reject => "### Registration cannot be accepted yet\n\nFix what is listed under \"To fix\" and push to this pull request; the check runs again on its own.\n".to_string(),
    };
    md.push_str(&section("To fix", &lists.rejects));
    md.push_str(&section("Waiting for the site's owner", &lists.holds));
    md.push_str(&section(
        if verdict == Verdict::Pass {
            "Listed"
        } else {
            "Passed"
        },
        &lists.listed,
    ));
    md.push_str(&section("Notes", &lists.notes));
    md.push_str(&format!(
        "\nThe rules are in the author guide: {}/publish/\n",
        input.site_url.trim_end_matches('/')
    ));

    Review {
        verdict,
        repositories: lists.repositories,
        rejects: lists.rejects,
        holds: lists.holds,
        listed: lists.listed,
        notes: lists.notes,
        markdown: md,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_numbers() {
        assert_eq!(day_number("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(day_number("2026-11-12T05:00:00Z"), Some(20_769));
        assert_eq!(
            day_number("2026-03-01T00:00:00Z").unwrap()
                - day_number("2026-02-28T23:59:59Z").unwrap(),
            1
        );
        assert_eq!(day_number("yesterday"), None);
    }

    #[test]
    fn registry_paths() {
        assert_eq!(registry_stem("registry/alice.toml"), Some("alice"));
        assert_eq!(
            registry_stem("registry/alice-guns.toml"),
            Some("alice-guns")
        );
        for bad in [
            "registry/.toml",
            "registry/sub/alice.toml",
            "registry/alice.txt",
            "registry/denylist.txt",
            "Registry/alice.toml",
            "registry/a b.toml",
            ".github/workflows/build.yml",
        ] {
            assert_eq!(registry_stem(bad), None, "{bad}");
        }
    }
}
