//! `qm-site-builder build ...` builds the site, `review-registration` reviews a registration
//! pull request, `check-registration <file>` checks one registry file by hand, and
//! `scan <archive>` runs the safety scan on a mod archive.
use qm_schema::IndexFile;
use qm_site_builder::review::{review, PrFile, ReviewInput, Verdict};
use qm_site_builder::source::{FixtureSource, GitHubSource, Source};
use qm_site_builder::{
    build_with, check_registration, read_denylist, read_id_overrides, read_registry, scan, site,
    wiki, Build, Owners, State,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  qm-site-builder build --out DIR [--registry DIR] [--static DIR] [--wiki FILE]
                         [--changelog FILE] [--schemas DIR] [--site-url URL]
                         [--owners FILE] [--previous-index FILE] [--report FILE]
                         [--generated-at TIME] [--fixtures DIR]
  qm-site-builder review-registration --files FILE --author LOGIN --site-owner LOGIN
                         [--content FILE] [--registry DIR] [--owners FILE]
                         [--previous-index FILE] [--open-prs N] [--merged-today N]
                         [--result FILE] [--comment FILE] [--now TIME] [--fixtures DIR]
  qm-site-builder check-registration FILE [--fixtures DIR] [--summary FILE]
  qm-site-builder scan ARCHIVE

build
  --wiki            the modding wiki (one Markdown file), published as wiki/
  --changelog       the QuarterMaster changelog, published as changelog/
  --schemas         a folder of editor schemas, published as schemas/
  --static          files to add to static/, replacing built-in ones of the same name
  --owners          the live site's api/v1/owners.json: which repository owns each mod id
  --previous-index  the live site's api/v1/index.json: what is listed now
  --report          where to write the mods the safety scan holds (JSON)

review-registration
  --files           the pull request's file list, as GitHub's API gives it (JSON)
  --content         the registry file as the pull request has it
  --result          where to write the verdict (JSON); --comment, the comment (Markdown)
  The exit code is 0 for pass, 3 for hold and 1 for reject.

scan prints what the safety scan finds in a .zip or .qmpack; the exit code is 1 when a
finding rejects the mod, 3 when one holds it.

Without --fixtures it reads GitHub with the GITHUB_TOKEN environment variable (the workflow's token).";

/// `api/v1/owners.json`: which repository owns each mod id.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct OwnersFile {
    schema_version: u32,
    owners: Owners,
}

/// The owners file a flag names; nothing when the flag was not given.
fn read_owners(args: &Args) -> Result<Option<Owners>, String> {
    read_flag(args, "owners")?
        .map(|raw| {
            serde_json::from_str::<OwnersFile>(&raw)
                .map(|f| f.owners)
                .map_err(|e| format!("--owners: {e}"))
        })
        .transpose()
}

/// The index a flag names; nothing when the flag was not given.
fn read_previous(args: &Args) -> Result<Option<IndexFile>, String> {
    read_flag(args, "previous-index")?
        .map(|raw| IndexFile::parse(raw.as_bytes()).map_err(|e| format!("--previous-index: {e}")))
        .transpose()
}

/// The mods the scan holds, for the workflow to raise with the site's owner.
fn report_json(b: &Build) -> serde_json::Value {
    let held: Vec<serde_json::Value> = b
        .held
        .iter()
        .map(|h| {
            serde_json::json!({
                "repository": h.repository,
                "id": h.held.id,
                "name": h.held.name,
                "version": h.held.version,
                "asset": h.held.asset,
                "sha256": h.held.sha256,
                "action": if h.held.reject { "reject" } else { "hold" },
                "kinds": h.held.kinds,
                "reasons": h.held.reasons,
                "kept_version": h.kept_version,
            })
        })
        .collect();
    serde_json::json!({ "held": held })
}

fn number_flag(args: &Args, name: &str) -> Result<u32, String> {
    match args.flags.get(name) {
        Some(v) => v.parse().map_err(|_| format!("--{name} must be a number")),
        None => Ok(0),
    }
}

fn required<'a>(args: &'a Args, name: &str) -> Result<&'a String, String> {
    args.flags
        .get(name)
        .ok_or_else(|| format!("--{name} is required\n\n{USAGE}"))
}

/// The file a flag names, or nothing when the flag was not given.
fn read_flag(args: &Args, name: &str) -> Result<Option<String>, String> {
    args.flags
        .get(name)
        .map(|p| std::fs::read_to_string(p).map_err(|e| format!("--{name} {p}: {e}")))
        .transpose()
}

/// Copies the `.json` files of `from` into `to`, in name order.
fn copy_schemas(from: &Path, to: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("--schemas {}: {e}", from.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(from)
        .map_err(err)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    files.sort();
    std::fs::create_dir_all(to).map_err(err)?;
    for f in files {
        if let Some(name) = f.file_name() {
            std::fs::copy(&f, to.join(name)).map_err(err)?;
        }
    }
    Ok(())
}

struct Args {
    cmd: String,
    positional: Vec<String>,
    flags: std::collections::HashMap<String, String>,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or_else(|| USAGE.to_string())?;
    let mut positional = Vec::new();
    let mut flags = std::collections::HashMap::new();
    while let Some(a) = it.next() {
        if let Some(name) = a.strip_prefix("--") {
            let v = it
                .next()
                .ok_or_else(|| format!("--{name} needs a value\n\n{USAGE}"))?;
            flags.insert(name.to_string(), v);
        } else {
            positional.push(a);
        }
    }
    Ok(Args {
        cmd,
        positional,
        flags,
    })
}

fn source(args: &Args) -> Box<dyn Source> {
    match args.flags.get("fixtures") {
        Some(dir) => Box::new(FixtureSource::new(dir)),
        None => Box::new(GitHubSource::from_env()),
    }
}

fn run() -> Result<u8, String> {
    let args = parse()?;
    let site_url = args
        .flags
        .get("site-url")
        .cloned()
        .unwrap_or_else(|| "https://gamiseus.github.io".to_string());
    match args.cmd.as_str() {
        "build" => {
            let out = PathBuf::from(
                args.flags
                    .get("out")
                    .ok_or_else(|| format!("--out is required\n\n{USAGE}"))?,
            );
            let registry_dir = PathBuf::from(
                args.flags
                    .get("registry")
                    .cloned()
                    .unwrap_or_else(|| "registry".into()),
            );
            let registry = read_registry(&registry_dir)?;
            let denylist = read_denylist(&registry_dir.join("denylist.txt"));
            // Ids the site's owner assigned by hand come before the stored ones.
            let mut owners = read_owners(&args)?.unwrap_or_default();
            owners.extend(read_id_overrides(&registry_dir)?);
            let previous = read_previous(&args)?;
            let generated_at = args
                .flags
                .get("generated-at")
                .cloned()
                .unwrap_or_else(utc_now);
            let src = source(&args);
            let b = build_with(
                src.as_ref(),
                &registry,
                &denylist,
                &site_url,
                &generated_at,
                &State {
                    owners: Some(&owners),
                    previous: previous.as_ref(),
                },
            )?;
            let changelog = read_flag(&args, "changelog")?;
            let changelog_html = changelog
                .as_deref()
                .map(|c| wiki::render_page(wiki::changelog_entries(c)));
            let version = changelog.as_deref().and_then(wiki::first_version);
            let wiki = read_flag(&args, "wiki")?.map(|md| wiki::parse(&md, changelog.is_some()));
            if let Some(w) = wiki.as_ref().filter(|w| !w.problems.is_empty()) {
                return Err(format!(
                    "The wiki has links that go nowhere:\n  - {}",
                    w.problems.join("\n  - ")
                ));
            }
            let static_dir = args.flags.get("static").map(PathBuf::from);
            site::write_site(
                &site::SiteInput {
                    index: &b.index,
                    pages: &b.pages,
                    wiki: wiki.as_ref(),
                    changelog_html: changelog_html.as_deref(),
                    version: version.as_deref(),
                    site_url: &site_url,
                },
                &out,
                static_dir.as_deref(),
            )
            .map_err(|e| format!("writing the site: {e}"))?;
            if let Some(dir) = args.flags.get("schemas") {
                copy_schemas(Path::new(dir), &out.join("schemas"))?;
            }
            let owners_file = OwnersFile {
                schema_version: 1,
                owners: b.owners.clone(),
            };
            std::fs::write(
                out.join("api/v1/owners.json"),
                serde_json::to_string_pretty(&owners_file).map_err(|e| e.to_string())? + "\n",
            )
            .map_err(|e| format!("writing owners.json: {e}"))?;
            if let Some(path) = args.flags.get("report") {
                std::fs::write(
                    path,
                    serde_json::to_string_pretty(&report_json(&b)).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("--report {path}: {e}"))?;
            }
            if let Some(w) = &wiki {
                println!("Wrote {} wiki pages.", w.pages.len());
            }
            println!(
                "Listed {} mods from {} repositories.",
                b.index.mods.len(),
                b.reports.iter().filter(|r| !r.mods.is_empty()).count()
            );
            for r in b.reports.iter().filter(|r| !r.problems.is_empty()) {
                println!("{}:", r.full_name);
                for p in &r.problems {
                    println!("  - {p}");
                }
            }
            for h in &b.held {
                println!(
                    "Held: {} `{}` {} ({}){}",
                    h.repository,
                    h.held.id,
                    h.held.version,
                    h.held.kinds.join(", "),
                    match &h.kept_version {
                        Some(v) => format!("; {v} stays listed"),
                        None => String::new(),
                    }
                );
            }
            Ok(0)
        }
        "review-registration" => {
            let files_path = required(&args, "files")?;
            let files: Vec<PrFile> = std::fs::read_to_string(files_path)
                .map_err(|e| format!("--files {files_path}: {e}"))
                .and_then(|raw| {
                    // PowerShell writes a byte order mark; JSON does not allow one.
                    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
                        .map_err(|e| format!("--files {files_path}: {e}"))
                })?;
            let content = read_flag(&args, "content")?;
            let registry_dir = PathBuf::from(
                args.flags
                    .get("registry")
                    .cloned()
                    .unwrap_or_else(|| "registry".into()),
            );
            let denylist = read_denylist(&registry_dir.join("denylist.txt"));
            let mut owners = read_owners(&args)?.unwrap_or_default();
            owners.extend(read_id_overrides(&registry_dir)?);
            let previous = read_previous(&args)?;
            let now = args.flags.get("now").cloned().unwrap_or_else(utc_now);
            let src = source(&args);
            let r = review(
                src.as_ref(),
                &ReviewInput {
                    files: &files,
                    content: content.as_deref(),
                    registry_dir: &registry_dir,
                    denylist: &denylist,
                    author: required(&args, "author")?,
                    site_owner: required(&args, "site-owner")?,
                    state: State {
                        owners: Some(&owners),
                        previous: previous.as_ref(),
                    },
                    open_prs: number_flag(&args, "open-prs")?,
                    merged_today: number_flag(&args, "merged-today")?,
                    now: &now,
                    site_url: &site_url,
                },
            );
            println!("{}", r.markdown);
            if let Some(path) = args.flags.get("result") {
                std::fs::write(
                    path,
                    serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?,
                )
                .map_err(|e| format!("--result {path}: {e}"))?;
            }
            if let Some(path) = args.flags.get("comment") {
                std::fs::write(path, &r.markdown).map_err(|e| format!("--comment {path}: {e}"))?;
            }
            Ok(match r.verdict {
                Verdict::Pass => 0,
                Verdict::Hold => 3,
                Verdict::Reject => 1,
            })
        }
        "scan" => {
            let file = args
                .positional
                .first()
                .ok_or_else(|| format!("which archive?\n\n{USAGE}"))?;
            let bytes = std::fs::read(file).map_err(|e| format!("{file}: {e}"))?;
            let found = scan::scan_archive(&bytes);
            let can: Vec<&str> = found
                .capabilities
                .iter()
                .map(|c| scan::capability_name(*c))
                .collect();
            println!(
                "Can do: {}",
                if can.is_empty() {
                    "nothing outside the game (plain sandboxed Lua)".to_string()
                } else {
                    can.join(", ")
                }
            );
            for f in &found.findings {
                println!(
                    "{} [{}] {}",
                    match f.action {
                        scan::Action::Reject => "REJECT",
                        scan::Action::Hold => "HOLD  ",
                    },
                    f.kind,
                    f.text()
                );
            }
            if found.findings.is_empty() {
                println!("No findings.");
            }
            Ok(
                if found
                    .findings
                    .iter()
                    .any(|f| f.action == scan::Action::Reject)
                {
                    1
                } else if found.findings.is_empty() {
                    0
                } else {
                    3
                },
            )
        }
        "check-registration" => {
            let file = args
                .positional
                .first()
                .ok_or_else(|| format!("which registry file?\n\n{USAGE}"))?;
            let src = source(&args);
            let (md, ok) = check_registration(src.as_ref(), &PathBuf::from(file), &site_url);
            println!("{md}");
            if let Some(summary) = args.flags.get("summary") {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(summary)
                    .map_err(|e| format!("{summary}: {e}"))?;
                f.write_all(md.as_bytes())
                    .map_err(|e| format!("{summary}: {e}"))?;
            }
            Ok(u8::from(!ok))
        }
        _ => Err(USAGE.to_string()),
    }
}

/// UTC now as RFC 3339, without a date library.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
