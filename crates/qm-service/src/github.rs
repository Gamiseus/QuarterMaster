//! The latest release of a GitHub repository that is not in the mod index (spec 8.6), read from
//! the github.com website's `releases/latest` redirect: its `Location` names the tag. This is
//! not the REST API and needs no token.
use crate::net::NetPolicy;
use qm_manager::community::fetch_head_location;

const USER_AGENT: &str = "QuarterMaster-Launcher/1.0 (Mud and Blood Mod Manager)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LatestRelease {
    pub tag: String,
    pub download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckError {
    /// github.com answered 429: stop checking for now.
    Busy,
    /// No such repository, or it has no releases.
    NotFound,
    Network(String),
}

/// A GitHub owner or repository name that is safe to put in a path.
pub(crate) fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// One request to `{github_base}/{owner}/{repo}/releases/latest`, redirects not followed.
pub(crate) fn latest_release(
    net: &NetPolicy,
    owner: &str,
    repo: &str,
    asset: Option<&str>,
) -> Result<LatestRelease, CheckError> {
    if !valid_name(owner) || !valid_name(repo) {
        return Err(CheckError::Network(
            "Invalid GitHub owner or repository name.".into(),
        ));
    }
    let base = net.github_base.trim_end_matches('/');
    let url = format!("{base}/{owner}/{repo}/releases/latest");
    let (status, location) = fetch_head_location(&url, USER_AGENT, net.allow_download.as_ref())
        .map_err(CheckError::Network)?;
    match status {
        429 => return Err(CheckError::Busy),
        404 => return Err(CheckError::NotFound),
        300..=399 => {}
        s => {
            return Err(CheckError::Network(format!(
                "github.com answered HTTP {s}."
            )))
        }
    }
    // `.../releases/tag/<tag>`; a repository without releases redirects to `.../releases`.
    let tag = location
        .as_deref()
        .and_then(|l| l.split(['?', '#']).next())
        .and_then(|l| l.rsplit_once("/releases/tag/").map(|(_, t)| t))
        .map(percent_decode)
        .filter(|t| !t.is_empty() && !t.contains('/'))
        .ok_or(CheckError::NotFound)?;
    let seg = encode_segment(&tag);
    let download_url = match asset {
        Some(a) => format!(
            "{base}/{owner}/{repo}/releases/download/{seg}/{}",
            encode_segment(a)
        ),
        None => format!("{}/{owner}/{repo}/zip/refs/tags/{seg}", codeload_base(base)),
    };
    Ok(LatestRelease { tag, download_url })
}

/// Source archives come from codeload; tests keep their local server.
fn codeload_base(github_base: &str) -> String {
    if github_base.eq_ignore_ascii_case("https://github.com") {
        "https://codeload.github.com".to_string()
    } else {
        format!("{github_base}/codeload")
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
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

/// Percent-encode everything but RFC 3986 unreserved characters (one path segment).
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_segments() {
        assert!(valid_name("Gamiseus") && valid_name("MnB-Mods") && valid_name("a.b_c"));
        for bad in ["", ".", "..", "a/b", "a?b", "a b", "a%2e"] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert_eq!(encode_segment("v1.0"), "v1.0");
        assert_eq!(encode_segment("rel/1 x"), "rel%2F1%20x");
        assert_eq!(percent_decode("v1.0%2Bbuild"), "v1.0+build");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
