use crate::storage::QuarterMasterPaths;
use qm_schema::security::SecurityScanReport;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, SystemTime};

const GITHUB_SEARCH_API: &str = "https://api.github.com/search/repositories";
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
pub fn get_community_catalog(
    paths: &QuarterMasterPaths,
    query_opt: Option<&str>,
    topic_filter: Option<&str>,
    source_filter: Option<&str>,
    force_refresh: bool,
) -> Value {
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

    let settings = paths.load_settings();
    let mut items = Vec::new();
    let mut rate_limited = false;
    let mut error_note: Option<String> = None;

    // 1. Fetch from GitHub Search API
    if source_filter.unwrap_or("all") != "NexusMods" {
        match fetch_github_repositories(&gh_query, settings.github_token.as_deref()) {
            Ok(repos) => {
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
                        &owner,
                        &name,
                        settings.github_token.as_deref(),
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
            }
            Err(e) => {
                if e.contains("403") || e.contains("rate limit") {
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

/// Calls GitHub Search API
fn fetch_github_repositories(
    query: &str,
    token_opt: Option<&str>,
) -> Result<Vec<Value>, String> {
    let mut req = ureq::get(GITHUB_SEARCH_API)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github.v3+json")
        .timeout(Duration::from_secs(8))
        .query("q", query)
        .query("sort", "stars")
        .query("order", "desc");

    if let Some(token) = token_opt.filter(|t| !t.trim().is_empty()) {
        req = req.set("Authorization", &format!("Bearer {}", token.trim()));
    }

    let resp = req.call().map_err(|e| format!("{e}"))?;
    let body_str = resp.into_string().map_err(|e| format!("{e}"))?;
    let json_val: Value = serde_json::from_str(&body_str).map_err(|e| format!("{e}"))?;

    let items = json_val
        .get("items")
        .and_then(|it| it.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(items)
}

/// Checks latest release for a given repository to find downloadable zip assets
fn fetch_latest_release_asset(
    owner: &str,
    repo: &str,
    token_opt: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let mut req = ureq::get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github.v3+json")
        .timeout(Duration::from_secs(4));

    if let Some(token) = token_opt.filter(|t| !t.trim().is_empty()) {
        req = req.set("Authorization", &format!("Bearer {}", token.trim()));
    }

    let Ok(resp) = req.call() else {
        // Fallback: Default branch archive URL
        let fallback_dl = format!("https://github.com/{owner}/{repo}/archive/refs/heads/main.zip");
        return (None, Some(fallback_dl), Some("main.zip".to_string()));
    };

    let Ok(body) = resp.into_string() else {
        return (None, None, None);
    };

    let Ok(rel_json) = serde_json::from_str::<Value>(&body) else {
        return (None, None, None);
    };

    let tag = rel_json.get("tag_name").and_then(|t| t.as_str()).map(|s| s.to_string());

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

    // If release has no zip asset, use zipball_url
    let zipball = rel_json
        .get("zipball_url")
        .and_then(|z| z.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some(format!("https://github.com/{owner}/{repo}/archive/refs/heads/main.zip")));

    (tag, zipball, Some("release.zip".to_string()))
}

/// Downloads a remote mod archive with safety verifications:
/// 1. Maximum size enforcement (`max_bytes`)
/// 2. Zip-slip protection (no directory traversal or drive letters)
/// 3. Schema verification (checks for qm.toml or mod.json)
/// 4. Lua static security audit (flags ffi, os.execute, etc.)
/// 5. Clean import into library/<mod_id>/<version>/
pub fn download_and_install_community_mod(
    paths: &QuarterMasterPaths,
    download_url: &str,
    expected_mod_id: Option<&str>,
    max_bytes: u64,
    allow: UrlCheck<'_>,
) -> Result<Value, String> {
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
            .map(|d| d.as_millis())
            .unwrap_or(0),
        filename
    ));

    let buffer = fetch_capped(trimmed_url, USER_AGENT, allow, max_bytes)?;

    fs::write(&dest_file, &buffer)
        .map_err(|e| format!("Failed to write downloaded archive: {e}"))?;

    // Perform pre-flight safety audit on the zip archive
    let security_report = audit_remote_archive_safety(&dest_file)?;

    // Import cleanly into library
    let import_res = paths.import_mod_from_path_with_options(
        &dest_file,
        None,
        false,
        expected_mod_id.map(|id| vec![id.to_string()]),
        Some("Upgrade".to_string()),
    )?;

    // Clean up temporary downloaded archive
    let _ = fs::remove_file(&dest_file);

    Ok(json!({
        "ok": true,
        "import_summary": import_res,
        "security_audit": security_report,
    }))
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
}
