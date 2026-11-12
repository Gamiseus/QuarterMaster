use crate::storage::QuarterMasterPaths;
use qm_schema::security::SecurityScanReport;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const USER_AGENT: &str = "QuarterMaster-Launcher/1.0 (Mud and Blood Mod Manager)";
pub const CACHE_TTL_SECONDS: u64 = 900; // 15 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityModItem {
    pub id: String,
    pub name: String,
    pub repo_full_name: String,
    pub owner: String,
    pub description: String,
    pub stargazers_count: u64,
    pub forks_count: u64,
    pub updated_at: String,
    pub html_url: String,
    pub topics: Vec<String>,
    pub license: Option<String>,
    pub source_type: String, // "GitHub" | "NexusMods"
    pub latest_release_tag: Option<String>,
    pub download_url: Option<String>,
    pub download_asset_name: Option<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub update_key: Option<String>,
}

/// Fetches real community mods by searching GitHub topics and keywords.
/// Respects unauthenticated rate limits (60 req/hr) via 15-minute disk caching.
#[allow(clippy::too_many_arguments)]
pub fn get_community_catalog(
    paths: &QuarterMasterPaths,
    query_opt: Option<&str>,
    topic_filter: Option<&str>,
    source_filter: Option<&str>,
    force_refresh: bool,
    api_base: &str,
    allow_api: UrlCheck<'_>,
    token: Option<&str>,
) -> Value {
    let token = token.map(str::trim).filter(|t| !t.is_empty());
    let cache_file = paths.cache_dir.join("community-catalog-cache.json");
    let trimmed_query = query_opt.unwrap_or("").trim();
    let is_default_search = trimmed_query.is_empty();

    // Check disk cache if not forcing refresh and using default search
    if !force_refresh && is_default_search && cache_file.is_file() {
        if let Ok(meta) = fs::metadata(&cache_file) {
            if let Ok(mtime) = meta.modified() {
                if let Ok(elapsed) = SystemTime::now().duration_since(mtime) {
                    if elapsed.as_secs() < CACHE_TTL_SECONDS {
                        if let Ok(cached_str) = fs::read_to_string(&cache_file) {
                            if let Ok(mut cached_val) = serde_json::from_str::<Value>(&cached_str) {
                                // Mark already installed state against current library
                                normalize_download_urls(&mut cached_val);
                                annotate_installed_status(paths, &mut cached_val);
                                return cached_val;
                            }
                        }
                    }
                }
            }
        }
    }

    // Build GitHub query
    // Search strictly for mods tagged with `quartermaster-mod` or direct repository slugs
    let gh_query = if !trimmed_query.is_empty() {
        if trimmed_query.contains('/') && !trimmed_query.contains(' ') {
            // Direct repository slug: e.g. "owner/repo"
            format!("repo:{}", trimmed_query)
        } else {
            format!("{trimmed_query} topic:quartermaster-mod")
        }
    } else if let Some(topic) = topic_filter.filter(|t| !t.is_empty() && *t != "all") {
        format!("topic:{topic}")
    } else {
        // Default discovery query: strictly search for repositories tagged for QuarterMaster
        "topic:quartermaster-mod".to_string()
    };

    let mut items = Vec::new();
    let mut rate_limited = false;
    let mut error_note: Option<String> = None;

    // 1. Fetch from GitHub Search API
    if source_filter.unwrap_or("all") != "NexusMods" {
        match fetch_github_repositories(api_base, &gh_query, token, allow_api) {
            Ok(repos) => {
                let mut releases_limited = false;
                for repo in repos {
                    let full_name = repo["full_name"].as_str().unwrap_or("").to_string();
                    let owner = repo["owner"]["login"].as_str().unwrap_or("").to_string();
                    let name = repo["name"].as_str().unwrap_or(&full_name).to_string();
                    let desc = repo["description"].as_str().unwrap_or("No description provided.").to_string();
                    let stars = repo["stargazers_count"].as_u64().unwrap_or(0);
                    let forks = repo["forks_count"].as_u64().unwrap_or(0);
                    let updated_at = repo["updated_at"].as_str().unwrap_or("").to_string();
                    let html_url = repo["html_url"].as_str().unwrap_or("").to_string();
                    let license = repo["license"]["spdx_id"].as_str().map(|s| s.to_string());
                    let topics: Vec<String> = repo["topics"]
                        .as_array()
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();

                    // Check for releases / asset zip
                    let (rel_tag, dl_url, asset_name) = fetch_latest_release_asset(
                        api_base,
                        &owner,
                        &name,
                        token,
                        allow_api,
                        &mut releases_limited,
                    );

                    let item_id = format!("{}_{}", owner, name).to_ascii_lowercase().replace('-', "_");
                    let update_key = format!("GitHub:{full_name}");

                    items.push(CommunityModItem {
                        id: item_id,
                        name,
                        repo_full_name: full_name,
                        owner,
                        description: desc,
                        stargazers_count: stars,
                        forks_count: forks,
                        updated_at,
                        html_url,
                        topics,
                        license,
                        source_type: "GitHub".to_string(),
                        latest_release_tag: rel_tag,
                        download_url: dl_url,
                        download_asset_name: asset_name,
                        installed: false,
                        installed_version: None,
                        update_key: Some(update_key),
                    });
                }
                if releases_limited {
                    error_note = Some(
                        "GitHub API rate limit reached. Some mods offer their source archive instead of a release."
                            .to_string(),
                    );
                }
            }
            Err(e) => {
                if e == RATE_LIMITED {
                    rate_limited = true;
                    error_note = Some(
                        "GitHub API rate limit reached (60 req/hr unauthenticated). Showing cached results."
                            .to_string(),
                    );
                } else {
                    error_note = Some(format!("Network error contacting GitHub: {e}"));
                }
            }
        }
    }

    // 2. Nexus Mods Directory entry
    if source_filter.unwrap_or("all") != "GitHub" {
        items.push(CommunityModItem {
            id: "nexus_mudandblood".to_string(),
            name: "Mud and Blood on Nexus Mods".to_string(),
            repo_full_name: "NexusMods:mudandblood".to_string(),
            owner: "NexusMods".to_string(),
            description: "Official Mud and Blood modding community hub on Nexus Mods. Download mods directly, manage via nxm:// links, or drag and drop archives.".to_string(),
            stargazers_count: 0,
            forks_count: 0,
            updated_at: "Live".to_string(),
            html_url: "https://www.nexusmods.com/mudandblood/mods".to_string(),
            topics: vec!["nexusmods".to_string(), "mud-and-blood".to_string()],
            license: None,
            source_type: "NexusMods".to_string(),
            latest_release_tag: None,
            download_url: None,
            download_asset_name: None,
            installed: false,
            installed_version: None,
            update_key: Some("Nexus:mudandblood".to_string()),
        });
    }

    // If rate limited or error, try returning stale cache
    if (rate_limited || items.is_empty()) && cache_file.is_file() {
        if let Ok(cached_str) = fs::read_to_string(&cache_file) {
            if let Ok(mut cached_val) = serde_json::from_str::<Value>(&cached_str) {
                if let Some(obj) = cached_val.as_object_mut() {
                    obj.insert("is_cached".to_string(), json!(true));
                    if let Some(err) = error_note {
                        obj.insert("error_message".to_string(), json!(err));
                    }
                }
                normalize_download_urls(&mut cached_val);
                annotate_installed_status(paths, &mut cached_val);
                return cached_val;
            }
        }
    }

    let mut response = json!({
        "catalog": items,
        "total_count": items.len(),
        "is_cached": false,
        "query": trimmed_query,
        "rate_limited": rate_limited,
        "error_message": error_note,
    });

    // Annotate installed status
    annotate_installed_status(paths, &mut response);

    // Save default query results to cache
    if is_default_search && !items.is_empty() {
        let _ = fs::create_dir_all(&paths.cache_dir);
        let _ = fs::write(&cache_file, serde_json::to_string_pretty(&response).unwrap_or_default());
    }

    response
}

/// Annotates catalog entries with installed status against user's actual library
fn annotate_installed_status(paths: &QuarterMasterPaths, val: &mut Value) {
    let installed = paths.list_installed_mods();
    let Some(arr) = val.get_mut("catalog").and_then(|c| c.as_array_mut()) else {
        return;
    };

    for item in arr.iter_mut() {
        let update_key = item.get("update_key").and_then(|k| k.as_str()).unwrap_or("");
        let repo_full_name = item.get("repo_full_name").and_then(|r| r.as_str()).unwrap_or("");
        let item_name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");

        let mut is_installed = false;
        let mut inst_version = None;

        for m in &installed {
            let m_update_keys = &m.manifest.update_keys;
            let m_name = &m.manifest.name;

            let matches_update_key = m_update_keys.iter().any(|k| {
                k.eq_ignore_ascii_case(update_key)
                    || (!repo_full_name.is_empty() && k.to_ascii_lowercase().contains(&repo_full_name.to_ascii_lowercase()))
            });

            let matches_name = m_name.eq_ignore_ascii_case(item_name);

            if matches_update_key || matches_name {
                is_installed = true;
                inst_version = Some(m.manifest.version.clone());
                break;
            }
        }

        if let Some(obj) = item.as_object_mut() {
            obj.insert("installed".to_string(), json!(is_installed));
            obj.insert("installed_version".to_string(), json!(inst_version));
        }
    }
}

/// The `Err` of an API call that hit GitHub's rate limit (403 or 429).
const RATE_LIMITED: &str = "GitHub API rate limit reached";
/// A search or release JSON is small; anything near this is not one.
const MAX_API_BYTES: u64 = 8 * 1024 * 1024;
const API_HOST: &str = "api.github.com";
/// The real GitHub API, for callers without a service network policy.
pub const GITHUB_API_BASE: &str = "https://api.github.com";

/// Accepts only `https://api.github.com/...` (no userinfo or port): the `allow_api` callback for
/// callers without a service network policy.
pub fn allow_github_api(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url.trim()).map_err(|e| format!("Invalid URL '{url}': {e}"))?;
    let ok = parsed.scheme() == "https"
        && parsed.host_str() == Some(API_HOST)
        && parsed.port().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none();
    if ok {
        Ok(())
    } else {
        Err(format!("Only https://{API_HOST} may be contacted ({url})"))
    }
}

/// GET a GitHub API JSON document; every hop is checked with `allow`. A rate limit is
/// `Err(RATE_LIMITED)`.
fn github_api_get(url: &str, token: Option<&str>, allow: UrlCheck<'_>) -> Result<Value, String> {
    let mut headers = vec![
        ("User-Agent", USER_AGENT.to_string()),
        ("Accept", "application/vnd.github+json".to_string()),
    ];
    if let Some(t) = token {
        headers.push(("Authorization", format!("Bearer {t}")));
    }
    let resp = fetch_conditional(url, &headers, allow, MAX_API_BYTES)?;
    match resp.status {
        200 => {
            serde_json::from_slice(&resp.body).map_err(|e| format!("Invalid GitHub response: {e}"))
        }
        403 | 429 => Err(RATE_LIMITED.to_string()),
        status => Err(format!("GitHub returned HTTP {status}")),
    }
}

/// Calls the GitHub Search API at `api_base`.
fn fetch_github_repositories(
    api_base: &str,
    query: &str,
    token: Option<&str>,
    allow: UrlCheck<'_>,
) -> Result<Vec<Value>, String> {
    let base = api_base.trim_end_matches('/');
    let mut url = url::Url::parse(&format!("{base}/search/repositories"))
        .map_err(|e| format!("Invalid API base '{api_base}': {e}"))?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("sort", "stars")
        .append_pair("order", "desc");
    let json_val = github_api_get(url.as_str(), token, allow)?;
    Ok(json_val
        .get("items")
        .and_then(|it| it.as_array())
        .cloned()
        .unwrap_or_default())
}

/// GitHub owner and repository names: ASCII letters, digits, `.`, `_`, `-`.
fn valid_github_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `https://codeload.github.com/{owner}/{repo}/zip/{reference...}`, each segment encoded.
fn codeload_url(owner: &str, repo: &str, reference: &[&str]) -> Option<String> {
    let mut url = url::Url::parse("https://codeload.github.com/").ok()?;
    url.path_segments_mut()
        .ok()?
        .clear()
        .extend([owner, repo, "zip"])
        .extend(reference);
    Some(url.to_string())
}

/// The default-branch source archive (offered when no release asset is known).
fn default_branch_archive(owner: &str, repo: &str) -> Option<String> {
    codeload_url(owner, repo, &["HEAD"])
}

/// `https://api.github.com/repos/{o}/{r}/zipball/{ref}` (refused by the download allowlist) as
/// the equivalent codeload URL; `None` for any other URL.
fn codeload_from_zipball(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    if parsed.scheme() != "https" || parsed.host_str() != Some(API_HOST) {
        return None;
    }
    let segs: Vec<String> = parsed.path_segments()?.map(percent_decode).collect();
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    match segs.as_slice() {
        ["repos", owner, repo, "zipball", reference @ ..]
            if valid_github_name(owner) && valid_github_name(repo) =>
        {
            if reference.is_empty() || reference.iter().any(|r| r.is_empty()) {
                default_branch_archive(owner, repo)
            } else {
                codeload_url(owner, repo, reference)
            }
        }
        _ => None,
    }
}

/// Decode `%XX` escapes in one path segment (invalid escapes stay as they are).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Rewrite API zipball URLs in a catalog value (old disk caches hold them) to codeload.
fn normalize_download_urls(val: &mut Value) {
    let Some(arr) = val.get_mut("catalog").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for item in arr {
        let fixed = item
            .get("download_url")
            .and_then(Value::as_str)
            .and_then(codeload_from_zipball);
        if let (Some(fixed), Some(obj)) = (fixed, item.as_object_mut()) {
            obj.insert("download_url".to_string(), json!(fixed));
        }
    }
}

/// Checks the latest release of a repository for a downloadable zip asset. Without one (or
/// without an answer) the source archive on codeload is offered. Once `limited` is set (the
/// first rate limit) no more requests are made.
fn fetch_latest_release_asset(
    api_base: &str,
    owner: &str,
    repo: &str,
    token: Option<&str>,
    allow: UrlCheck<'_>,
    limited: &mut bool,
) -> (Option<String>, Option<String>, Option<String>) {
    if !valid_github_name(owner) || !valid_github_name(repo) {
        return (None, None, None);
    }
    let fallback = || {
        (
            None,
            default_branch_archive(owner, repo),
            Some("source.zip".to_string()),
        )
    };
    if *limited {
        return fallback();
    }
    let base = api_base.trim_end_matches('/');
    let url = format!("{base}/repos/{owner}/{repo}/releases/latest");
    let rel_json = match github_api_get(&url, token, allow) {
        Ok(v) => v,
        Err(e) => {
            if e == RATE_LIMITED {
                *limited = true;
            }
            return fallback();
        }
    };

    let tag = rel_json
        .get("tag_name")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());

    if let Some(assets) = rel_json.get("assets").and_then(|a| a.as_array()) {
        for asset in assets {
            let name = asset.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name.ends_with(".zip") || name.ends_with(".qmpack") {
                let dl_url = asset
                    .get("browser_download_url")
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string());
                return (tag, dl_url, Some(name.to_string()));
            }
        }
    }

    // No zip asset: the release's source archive on codeload (the API zipball URL is not on a
    // download host).
    let source = match tag.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => codeload_url(owner, repo, &["refs", "tags", t]),
        None => rel_json
            .get("zipball_url")
            .and_then(Value::as_str)
            .and_then(codeload_from_zipball)
            .or_else(|| default_branch_archive(owner, repo)),
    };
    (tag, source, Some("release.zip".to_string()))
}

/// Downloads a remote mod archive with safety verifications:
/// 1. Maximum size enforcement (`max_bytes`)
/// 2. Zip-slip protection (no directory traversal or drive letters)
/// 3. Schema verification (checks for qm.toml or mod.json)
/// 4. Lua static security audit (flags ffi, os.execute, etc.)
/// 5. Clean import into library/<mod_id>/<version>/ with the `upgrade` policy
pub fn download_and_install_community_mod(
    paths: &QuarterMasterPaths,
    download_url: &str,
    expected_mod_id: Option<&str>,
    max_bytes: u64,
    allow: UrlCheck<'_>,
) -> Result<Value, String> {
    let zip = download_to_temp(paths, download_url, max_bytes, allow)?;
    install_downloaded(paths, &zip, download_url.trim(), expected_mod_id, "upgrade")
}

/// Download `download_url` (size-capped, every hop checked by `allow`) into a temp zip under
/// `cache/downloads`; returns its path. Nothing in the library is touched.
pub fn download_to_temp(
    paths: &QuarterMasterPaths,
    download_url: &str,
    max_bytes: u64,
    allow: UrlCheck<'_>,
) -> Result<PathBuf, String> {
    let trimmed_url = download_url.trim();
    if trimmed_url.is_empty() {
        return Err("Download URL is required".to_string());
    }

    // Safety: Allow HTTPS downloads from GitHub, NexusMods, or user-approved hosts
    if !trimmed_url.starts_with("https://") && !trimmed_url.starts_with("http://127.0.0.1") {
        return Err("Only secure HTTPS download URLs are permitted".to_string());
    }

    let dl_dir = paths.cache_dir.join("downloads");
    fs::create_dir_all(&dl_dir).map_err(|e| format!("Failed to create download cache dir: {e}"))?;

    let filename = trimmed_url
        .split('?')
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty() && s.ends_with(".zip"))
        .unwrap_or("mod_download.zip");

    let dest_file = dl_dir.join(format!(
        "qm_{}_{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        filename
    ));

    let buffer = fetch_capped(trimmed_url, USER_AGENT, allow, max_bytes)?;

    if let Err(e) = fs::write(&dest_file, &buffer) {
        let _ = fs::remove_file(&dest_file);
        return Err(format!("Failed to write downloaded archive: {e}"));
    }
    Ok(dest_file)
}

/// Audit a downloaded `zip`, then import it with `policy` (`upgrade` | `replace` | `alongside`)
/// recording `source_url`. The zip is removed on every exit path.
pub fn install_downloaded(
    paths: &QuarterMasterPaths,
    zip: &Path,
    source_url: &str,
    expected_mod_id: Option<&str>,
    policy: &str,
) -> Result<Value, String> {
    let result = (|| {
        // Perform pre-flight safety audit on the zip archive
        let security_report = audit_remote_archive_safety(zip)?;
        let import_res = paths.import_mod_from_path_with_options(
            zip,
            Some(source_url.to_string()),
            false,
            expected_mod_id.map(|id| vec![id.to_string()]),
            Some(policy.to_ascii_lowercase()),
        )?;
        Ok(json!({
            "ok": true,
            "import_summary": import_res,
            "security_audit": security_report,
        }))
    })();
    let _ = fs::remove_file(zip);
    result
}

fn size_limit_error(max_bytes: u64) -> String {
    format!(
        "Download exceeds the size limit of {} MiB",
        max_bytes / (1024 * 1024)
    )
}

/// Checks one URL (the start URL and every redirect target) before it is requested.
pub type UrlCheck<'a> = &'a dyn Fn(&str) -> Result<(), String>;

/// Redirect hops followed after the first request.
pub const MAX_REDIRECTS: u32 = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Reads a body, refusing a declared length above `max_bytes` and stopping with an error once
/// more than `max_bytes` bytes arrive.
pub fn cap_reader(
    declared: Option<u64>,
    reader: impl Read,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    if declared.is_some_and(|len| len > max_bytes) {
        return Err(size_limit_error(max_bytes));
    }
    let mut buffer = Vec::new();
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut buffer)
        .map_err(|e| format!("Read error: {e}"))?;
    if buffer.len() as u64 > max_bytes {
        return Err(size_limit_error(max_bytes));
    }
    Ok(buffer)
}

/// What to do with one response: `Some(next_url)` to follow a redirect (already checked with
/// `allow`), `None` to read this response's body.
pub fn next_hop(
    current: &str,
    status: u16,
    location: Option<&str>,
    allow: UrlCheck<'_>,
) -> Result<Option<String>, String> {
    if !(300..400).contains(&status) {
        return Ok(None);
    }
    let location = location
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .ok_or_else(|| format!("Redirect {status} without a Location header"))?;
    let base = url::Url::parse(current).map_err(|e| format!("Invalid URL '{current}': {e}"))?;
    let next = base
        .join(location)
        .map_err(|e| format!("Invalid redirect location '{location}': {e}"))?
        .to_string();
    allow(&next)?;
    Ok(Some(next))
}

/// GET `start` and return its body, following up to `MAX_REDIRECTS` redirects by hand so `allow`
/// sees every hop before it is requested. Connect and per-read timeouts only (no total deadline).
pub fn fetch_capped(
    start: &str,
    user_agent: &str,
    allow: UrlCheck<'_>,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    allow(start)?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();
    let mut url = start.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let resp = agent
            .get(&url)
            .set("User-Agent", user_agent)
            .call()
            .map_err(|e| format!("Download request failed: {e}"))?;
        match next_hop(&url, resp.status(), resp.header("Location"), allow)? {
            Some(next) => url = next,
            None => {
                let declared = resp
                    .header("Content-Length")
                    .and_then(|v| v.trim().parse::<u64>().ok());
                return cap_reader(declared, resp.into_reader(), max_bytes);
            }
        }
    }
    Err(format!("Download request failed: more than {MAX_REDIRECTS} redirects"))
}

/// GET `url` once without following redirects (`allow` checks it first). Returns the status
/// and the `Location` header; the body is not read. For github.com's `releases/latest`
/// redirect, which names the latest tag without the REST API.
pub fn fetch_head_location(url: &str, user_agent: &str, allow: UrlCheck<'_>) -> Result<(u16, Option<String>), String> {
    allow(url)?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();
    match agent.get(url).set("User-Agent", user_agent).call() {
        Ok(resp) => Ok((resp.status(), resp.header("Location").map(str::to_string))),
        Err(ureq::Error::Status(status, resp)) => Ok((status, resp.header("Location").map(str::to_string))),
        Err(ureq::Error::Transport(t)) => Err(format!("Request failed: {t}")),
    }
}

/// One final response from `fetch_conditional`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    pub status: u16,
    pub etag: Option<String>,
    pub body: Vec<u8>,
    /// GitHub said the rate limit is spent: `X-RateLimit-Remaining: 0` or a `Retry-After`
    /// (secondary limit). Lets a 403 for a rate limit be told apart from a refused token.
    pub rate_limited: bool,
}

fn rate_limit_spent(resp: &ureq::Response) -> bool {
    resp.header("X-RateLimit-Remaining").is_some_and(|v| v.trim() == "0") || resp.header("Retry-After").is_some()
}

/// GET `start` with `headers`, following redirects by hand like `fetch_capped` (`allow` sees
/// every hop first). Returns the final status, its `ETag` and body: 304 comes back with an empty
/// body, 4xx/5xx with their (capped) body. Only transport failures and refused hops are `Err`.
/// The `Authorization` header is never sent once a hop leaves the first origin.
pub fn fetch_conditional(
    start: &str,
    headers: &[(&str, String)],
    allow: UrlCheck<'_>,
    max_bytes: u64,
) -> Result<FetchResponse, String> {
    allow(start)?;
    let origin = |u: &str| url::Url::parse(u).ok().map(|p| p.origin());
    let first_origin = origin(start).ok_or_else(|| format!("Invalid URL '{start}'"))?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();
    let mut url = start.to_string();
    let mut same_origin = true;
    for _ in 0..=MAX_REDIRECTS {
        same_origin = same_origin && origin(&url).as_ref() == Some(&first_origin);
        let mut req = agent.get(&url);
        for (name, value) in headers {
            if same_origin || !name.eq_ignore_ascii_case("authorization") {
                req = req.set(name, value);
            }
        }
        let resp = match req.call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(status, resp)) => {
                return read_response(status, resp, max_bytes)
            }
            Err(ureq::Error::Transport(t)) => return Err(format!("Request failed: {t}")),
        };
        let status = resp.status();
        if status == 304 {
            return Ok(FetchResponse {
                status,
                etag: resp.header("ETag").map(str::to_string),
                body: Vec::new(),
                rate_limited: false,
            });
        }
        match next_hop(&url, status, resp.header("Location"), allow)? {
            Some(next) => url = next,
            None => return read_response(status, resp, max_bytes),
        }
    }
    Err(format!(
        "Request failed: more than {MAX_REDIRECTS} redirects"
    ))
}

fn read_response(
    status: u16,
    resp: ureq::Response,
    max_bytes: u64,
) -> Result<FetchResponse, String> {
    let etag = resp.header("ETag").map(str::to_string);
    let rate_limited = rate_limit_spent(&resp);
    let declared = resp
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok());
    let body = cap_reader(declared, resp.into_reader(), max_bytes)?;
    Ok(FetchResponse {
        status,
        etag,
        body,
        rate_limited,
    })
}

/// Audits a downloaded zip archive for path safety, schemas, and dangerous Lua functions
fn audit_remote_archive_safety(zip_path: &Path) -> Result<Value, String> {
    let file = fs::File::open(zip_path).map_err(|e| format!("Failed to open downloaded zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Invalid zip archive: {e}"))?;

    let mut has_qm_toml = false;
    let mut has_mod_json = false;
    let suspicious_paths: Vec<String> = Vec::new();
    let mut detected_capabilities = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("Zip entry read error: {e}"))?;
        let name = entry.name().to_string();
        let normalized = name.replace('\\', "/");

        // 1. Zip-slip & reserved names check
        if normalized.contains("..") || normalized.starts_with('/') || normalized.contains(':') {
            return Err(format!("Rejected archive containing malicious path traversal: '{name}'"));
        }
        for segment in normalized.split('/') {
            let seg_upper = segment.to_ascii_uppercase();
            if matches!(seg_upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "LPT1") {
                return Err(format!("Rejected archive containing reserved Windows filename: '{name}'"));
            }
        }

        if normalized.ends_with("qm.toml") {
            has_qm_toml = true;
        }
        if normalized.ends_with("mod.json") {
            has_mod_json = true;
        }

        // 2. Scan Lua files for dangerous capabilities
        if normalized.ends_with(".lua") && entry.size() < 2 * 1024 * 1024 {
            let mut lua_content = String::new();
            if entry.read_to_string(&mut lua_content).is_ok() {
                let findings = SecurityScanReport::scan_source(&name, &lua_content);
                for f in findings {
                    let cap_str = format!("{}: {}", f.capability.label(), f.snippet);
                    if !detected_capabilities.contains(&cap_str) {
                        detected_capabilities.push(cap_str);
                    }
                }
            }
        }
    }

    if !has_qm_toml && !has_mod_json {
        return Err("Archive does not contain required 'qm.toml' or 'mod.json' mod manifest".to_string());
    }

    let is_clean = detected_capabilities.is_empty();

    Ok(json!({
        "safe_paths": suspicious_paths.is_empty(),
        "has_qm_toml": has_qm_toml,
        "has_mod_json": has_mod_json,
        "is_safe": is_clean,
        "capabilities_detected": detected_capabilities,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_community_item_serialization() {
        let item = CommunityModItem {
            id: "test_author_mod".to_string(),
            name: "Test Mod".to_string(),
            repo_full_name: "test/mod".to_string(),
            owner: "test".to_string(),
            description: "Test description".to_string(),
            stargazers_count: 42,
            forks_count: 5,
            updated_at: "2026-10-25T12:00:00Z".to_string(),
            html_url: "https://github.com/test/mod".to_string(),
            topics: vec!["mud-and-blood".to_string()],
            license: Some("MIT".to_string()),
            source_type: "GitHub".to_string(),
            latest_release_tag: Some("v1.0.0".to_string()),
            download_url: Some("https://github.com/test/mod/releases/download/v1.0.0/test_mod.zip".to_string()),
            download_asset_name: Some("test_mod.zip".to_string()),
            installed: false,
            installed_version: None,
            update_key: Some("GitHub:test/mod".to_string()),
        };

        let val = serde_json::to_value(&item).unwrap();
        assert_eq!(val["stargazers_count"], 42);
        assert_eq!(val["topics"][0], "mud-and-blood");
        assert_eq!(val["source_type"], "GitHub");
    }

    fn https_github_only(u: &str) -> Result<(), String> {
        let ok = url::Url::parse(u)
            .is_ok_and(|p| p.scheme() == "https" && p.host_str() == Some("github.com"));
        if ok {
            Ok(())
        } else {
            Err(format!("not allowed: {u}"))
        }
    }

    #[test]
    fn cap_reader_rejects_declared_length_over_limit() {
        let err = cap_reader(Some(11), &b"tiny"[..], 10).unwrap_err();
        assert!(err.contains("size limit"), "{err}");
    }

    #[test]
    fn cap_reader_rejects_body_of_max_plus_one() {
        assert_eq!(cap_reader(None, &[7u8; 10][..], 10).unwrap().len(), 10);
        let err = cap_reader(None, &[7u8; 11][..], 10).unwrap_err();
        assert!(err.contains("size limit"), "{err}");
        // A lying Content-Length does not lift the cap.
        assert!(cap_reader(Some(1), &[7u8; 11][..], 10).is_err());
    }

    #[test]
    fn next_hop_checks_every_redirect_target() {
        let a = &https_github_only;
        assert_eq!(next_hop("https://github.com/a", 200, None, a).unwrap(), None);
        assert_eq!(
            next_hop("https://github.com/o/r/x", 302, Some("/o/r/y"), a).unwrap(),
            Some("https://github.com/o/r/y".to_string())
        );
        assert!(next_hop("https://github.com/a", 302, Some("http://github.com/a"), a).is_err());
        assert!(next_hop("https://github.com/a", 301, Some("https://evil.example/a"), a).is_err());
        assert!(next_hop("https://github.com/a", 302, None, a).is_err());
    }

    #[test]
    fn fetch_capped_refuses_redirect_to_disallowed_host_before_requesting_it() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
                line.clear();
            }
            let mut s = stream;
            s.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/payload.zip\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });
        let start = format!("http://127.0.0.1:{port}/mod.zip");
        let local = start.clone();
        let seen = std::sync::Mutex::new(Vec::new());
        let allow = |u: &str| {
            seen.lock().unwrap().push(u.to_string());
            if u == local {
                Ok(())
            } else {
                Err(format!("not allowed: {u}"))
            }
        };
        let err = fetch_capped(&start, USER_AGENT, &allow, 1024).unwrap_err();
        server.join().unwrap();
        assert_eq!(err, "not allowed: https://evil.example/payload.zip");
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    /// Serves `responses` in order, one per connection, and returns each request head.
    fn serve_in_order(responses: Vec<String>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut heads = Vec::new();
            for body in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
                    head.push_str(&line);
                    line.clear();
                }
                heads.push(head);
                let mut s = stream;
                s.write_all(body.as_bytes()).unwrap();
            }
            heads
        });
        (port, server)
    }

    #[test]
    fn fetch_conditional_returns_304_and_status_codes() {
        let resp = |status: &str, extra: &str, body: &str| {
            format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        let (port, server) = serve_in_order(vec![
            resp("304 Not Modified", "ETag: \"e1\"\r\n", ""),
            resp("429 Too Many Requests", "", "slow down"),
            resp("404 Not Found", "", "{}"),
            resp("200 OK", "ETag: \"e2\"\r\n", "{\"tag_name\":\"v1\"}"),
        ]);
        let base = format!("http://127.0.0.1:{port}");
        let allow = |u: &str| {
            if u.starts_with(&base) {
                Ok(())
            } else {
                Err(format!("not allowed: {u}"))
            }
        };
        let headers = [("If-None-Match", "\"e1\"".to_string())];
        let r = fetch_conditional(&format!("{base}/a"), &headers, &allow, 1024).unwrap();
        assert_eq!((r.status, r.etag.as_deref()), (304, Some("\"e1\"")));
        assert!(r.body.is_empty());
        let r = fetch_conditional(&format!("{base}/b"), &[], &allow, 1024).unwrap();
        assert_eq!((r.status, r.body.as_slice()), (429, &b"slow down"[..]));
        let r = fetch_conditional(&format!("{base}/c"), &[], &allow, 1024).unwrap();
        assert_eq!(r.status, 404);
        let r = fetch_conditional(&format!("{base}/d"), &[], &allow, 1024).unwrap();
        assert_eq!((r.status, r.etag.as_deref()), (200, Some("\"e2\"")));
        assert_eq!(r.body, b"{\"tag_name\":\"v1\"}");
        let heads = server.join().unwrap();
        assert!(
            heads[0]
                .to_ascii_lowercase()
                .contains("if-none-match: \"e1\""),
            "{}",
            heads[0]
        );
        // A refused start URL is an error before any request.
        assert!(fetch_conditional("https://evil.example/", &[], &allow, 1024).is_err());
        // A transport failure is an error.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://127.0.0.1:{}/x", closed.local_addr().unwrap().port());
        drop(closed);
        let any = |_: &str| Ok(());
        assert!(fetch_conditional(&dead, &[], &any, 1024).is_err());
    }

    #[test]
    fn test_audit_rejects_zip_slip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let zip_path = temp_dir.path().join("evil.zip");

        {
            let file = fs::File::create(&zip_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options: zip::write::FileOptions<()> = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zip.start_file("../evil.exe", options).unwrap();
            use std::io::Write;
            zip.write_all(b"malicious").unwrap();
            zip.finish().unwrap();
        }

        let res = audit_remote_archive_safety(&zip_path);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("path traversal"));
    }

    #[test]
    fn zipball_urls_map_to_codeload() {
        assert_eq!(
            codeload_from_zipball("https://api.github.com/repos/o/r/zipball/v1.0").as_deref(),
            Some("https://codeload.github.com/o/r/zip/v1.0")
        );
        assert_eq!(
            codeload_from_zipball("https://api.github.com/repos/o/r/zipball").as_deref(),
            Some("https://codeload.github.com/o/r/zip/HEAD")
        );
        assert_eq!(codeload_from_zipball("https://github.com/o/r/archive/x.zip"), None);
        assert_eq!(codeload_from_zipball("https://api.github.com/repos/o/../zipball/x"), None);
        assert_eq!(
            codeload_url("o", "r", &["refs", "tags", "v 1#x"]).as_deref(),
            Some("https://codeload.github.com/o/r/zip/refs/tags/v%201%23x")
        );
        let mut cached = json!({ "catalog": [
            { "download_url": "https://api.github.com/repos/o/r/zipball/v2" },
            { "download_url": "https://github.com/o/r/releases/download/v2/m.zip" }
        ]});
        normalize_download_urls(&mut cached);
        assert_eq!(
            cached["catalog"][0]["download_url"],
            "https://codeload.github.com/o/r/zip/v2"
        );
        assert_eq!(
            cached["catalog"][1]["download_url"],
            "https://github.com/o/r/releases/download/v2/m.zip"
        );
    }

    #[test]
    fn allow_github_api_accepts_only_the_api_host() {
        assert!(allow_github_api("https://api.github.com/search/repositories?q=x").is_ok());
        for url in [
            "http://api.github.com/x",
            "https://github.com/x",
            "https://api.github.com:8443/x",
            "https://user@api.github.com/x",
            "https://api.github.com.evil.example/x",
            "http://127.0.0.1:1/x",
        ] {
            assert!(allow_github_api(url).is_err(), "{url}");
        }
    }
}
