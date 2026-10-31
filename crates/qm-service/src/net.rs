//! Download URL policy: https only, GitHub release asset hosts only, 512 MiB cap.
use crate::error::{ErrorKind, QmError, QmResult};

pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "codeload.github.com",
];

const MSG_NOT_ALLOWED: &str = "Downloads are only allowed over https from GitHub release hosts.";

/// `InvalidInput` unless `url` is `https://<allowed host>/...` with no userinfo or port.
pub fn check_download_url(url: &str) -> QmResult<()> {
    let reject =
        || QmError::new(ErrorKind::InvalidInput, MSG_NOT_ALLOWED).with_details(url.to_string());
    let url = url.trim();
    let scheme_len = "https://".len();
    let has_scheme = url
        .get(..scheme_len)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"));
    if !has_scheme {
        return Err(reject());
    }
    let rest = &url[scheme_len..];
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Userinfo (`user@host`) and explicit ports are never needed for release assets.
    if authority.contains(['@', ':', '\\']) {
        return Err(reject());
    }
    let host = authority.to_ascii_lowercase();
    if ALLOWED_HOSTS.contains(&host.as_str()) {
        Ok(())
    } else {
        Err(reject())
    }
}
