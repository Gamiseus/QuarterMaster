//! Network policy: https only; the mod index on `gamiseus.github.io` (its `/api/` path only);
//! github.com and its release asset hosts for downloads and the latest-release redirect; a
//! 512 MiB download cap. The GitHub REST API is never called. `NetPolicy` lets tests swap in a
//! local server.
use crate::error::{ErrorKind, QmError, QmResult};
use std::sync::Arc;

pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// The mod index. Compiled in and not configurable, so a settings edit cannot point the
/// launcher at another index.
pub const INDEX_URL: &str = "https://gamiseus.github.io/api/v1/index.json";
const INDEX_HOST: &str = "gamiseus.github.io";
const INDEX_PATH_PREFIX: &str = "/api/";

const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "codeload.github.com",
];

pub(crate) const MSG_NOT_ALLOWED: &str =
    "Downloads are only allowed over https from GitHub release hosts.";
const MSG_INDEX_NOT_ALLOWED: &str =
    "The mod index is only read over https from gamiseus.github.io/api/.";

/// Checks one URL (a start URL or a redirect hop) before it is requested.
pub type AllowFn = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Where the index and github.com live, and which URLs every hop may reach. Not a specta type.
#[derive(Clone)]
pub struct NetPolicy {
    /// The mod index URL (`INDEX_URL` in production).
    pub index_url: String,
    pub allow_index: AllowFn,
    /// Base of `/<owner>/<repo>/releases/latest`, without a trailing slash.
    pub github_base: String,
    /// Downloads and the github.com release redirect.
    pub allow_download: AllowFn,
}

impl NetPolicy {
    pub fn production() -> Self {
        Self {
            index_url: INDEX_URL.to_string(),
            allow_index: Arc::new(|u: &str| as_hop_result(check_index_url(u), u)),
            github_base: "https://github.com".to_string(),
            allow_download: Arc::new(|u: &str| as_hop_result(check_download_url(u), u)),
        }
    }

    /// Refuses every URL (tests that must never reach the network).
    pub fn deny_all() -> Self {
        let deny: AllowFn = Arc::new(|u: &str| Err(format!("Network access is disabled ({u})")));
        Self {
            index_url: INDEX_URL.to_string(),
            allow_index: deny.clone(),
            github_base: "https://github.com".to_string(),
            allow_download: deny,
        }
    }
}

fn as_hop_result(r: QmResult<()>, url: &str) -> Result<(), String> {
    r.map_err(|e| format!("{} ({url})", e.message))
}

/// `InvalidInput` unless `url` is `https://gamiseus.github.io/api/...`.
pub fn check_index_url(url: &str) -> QmResult<()> {
    let reject = || {
        QmError::new(ErrorKind::InvalidInput, MSG_INDEX_NOT_ALLOWED).with_details(url.to_string())
    };
    let host = https_host(url).ok_or_else(reject)?;
    if host != INDEX_HOST {
        return Err(reject());
    }
    let path = url.trim()["https://".len() + host.len()..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    // No dot segments: `/api/../` must not reach the rest of the site.
    if path.starts_with(INDEX_PATH_PREFIX)
        && !path
            .split('/')
            .any(|seg| seg == ".." || seg == "." || seg.contains('%'))
    {
        Ok(())
    } else {
        Err(reject())
    }
}

/// `InvalidInput` unless `url` is `https://<allowed host>/...` with no userinfo or port.
pub fn check_download_url(url: &str) -> QmResult<()> {
    let reject =
        || QmError::new(ErrorKind::InvalidInput, MSG_NOT_ALLOWED).with_details(url.to_string());
    match https_host(url) {
        Some(h) if ALLOWED_HOSTS.contains(&h.as_str()) => Ok(()),
        _ => Err(reject()),
    }
}

/// The lowercase host of an `https://` URL without userinfo or port; `None` otherwise.
fn https_host(url: &str) -> Option<String> {
    let url = url.trim();
    let scheme_len = "https://".len();
    if !url
        .get(..scheme_len)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        return None;
    }
    let authority = url[scheme_len..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    // Userinfo (`user@host`) and explicit ports are never needed.
    if authority.is_empty() || authority.contains(['@', ':', '\\']) {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_host_allowed_only_for_api_path() {
        assert!(check_index_url(INDEX_URL).is_ok());
        assert!(check_index_url("https://gamiseus.github.io/api/v1/launcher.json").is_ok());
        for bad in [
            "https://gamiseus.github.io/",
            "https://gamiseus.github.io/mods/x/",
            "https://gamiseus.github.io/api/../mods/",
            "https://gamiseus.github.io/api/%2e%2e/x",
            "http://gamiseus.github.io/api/v1/index.json",
            "https://gamiseus.github.io:443/api/v1/index.json",
            "https://evil.github.io/api/v1/index.json",
            "https://api.github.com/repos/a/b/releases/latest",
        ] {
            assert!(check_index_url(bad).is_err(), "{bad}");
        }
        // The index host is not a download host, and the API host is neither.
        assert!(check_download_url("https://gamiseus.github.io/api/v1/index.json").is_err());
        assert!(check_download_url("https://api.github.com/repos/a/b/releases/latest").is_err());
        assert!(check_download_url("https://github.com/a/b/releases/latest").is_ok());
    }
}
