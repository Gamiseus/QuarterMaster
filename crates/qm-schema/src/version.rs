#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;

/// Parsed version supporting `v#.#`, `v#.#.#`, game letter suffixes (e.g. `0.5.3b`),
/// and SemVer prereleases (e.g. `1.2.0-alpha.1`, `1.2.0-rc.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Optional single-letter game build suffix like `b` in `0.5.3b` (orders after `0.5.3`).
    pub letter_suffix: Option<char>,
    /// Optional SemVer prerelease identifiers (orders *before* the release version).
    pub prerelease: Vec<PreReleaseIdent>,
    /// Optional build metadata (`+build123`), ignored for precedence.
    pub build_metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreReleaseIdent {
    Numeric(u64),
    AlphaNumeric(String),
}

impl Ord for PreReleaseIdent {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (PreReleaseIdent::Numeric(a), PreReleaseIdent::Numeric(b)) => a.cmp(b),
            // Per SemVer 2.0.0 §11.4.3: numeric identifiers always have lower precedence than non-numeric
            (PreReleaseIdent::Numeric(_), PreReleaseIdent::AlphaNumeric(_)) => Ordering::Less,
            (PreReleaseIdent::AlphaNumeric(_), PreReleaseIdent::Numeric(_)) => Ordering::Greater,
            (PreReleaseIdent::AlphaNumeric(a), PreReleaseIdent::AlphaNumeric(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreReleaseIdent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParsedVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.minor.cmp(&other.minor) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match self.patch.cmp(&other.patch) {
            Ordering::Equal => {}
            ord => return ord,
        }
        // Letter suffix like `0.5.3b` > `0.5.3a` > `0.5.3`
        match (self.letter_suffix, other.letter_suffix) {
            (None, None) => {}
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (Some(a), Some(b)) => match a.cmp(&b) {
                Ordering::Equal => {}
                ord => return ord,
            },
        }
        // Prerelease: normal version (empty prerelease) has higher precedence than prerelease
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.prerelease.cmp(&other.prerelease),
        }
    }
}

impl PartialOrd for ParsedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for ParsedVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(ch) = self.letter_suffix {
            write!(f, "{ch}")?;
        }
        if !self.prerelease.is_empty() {
            write!(f, "-")?;
            for (i, ident) in self.prerelease.iter().enumerate() {
                if i > 0 {
                    write!(f, ".")?;
                }
                match ident {
                    PreReleaseIdent::Numeric(n) => write!(f, "{n}")?,
                    PreReleaseIdent::AlphaNumeric(s) => write!(f, "{s}")?,
                }
            }
        }
        Ok(())
    }
}

impl ParsedVersion {
    pub const ZERO: Self = Self {
        major: 0,
        minor: 0,
        patch: 0,
        letter_suffix: None,
        prerelease: Vec::new(),
        build_metadata: None,
    };

    pub fn is_prerelease(&self) -> bool {
        !self.prerelease.is_empty()
    }

    /// Strict-but-practical parser supporting `v#.#`, `v#.#.#`, `0.5.3b`, and `-prerelease+build`.
    /// Rejects stray prefixes, spaces, or ambiguous junk.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("Empty version string".to_string());
        }

        // Allow optional leading 'v' or 'V' only if followed immediately by an ASCII digit
        let without_v = if let Some(rest) = trimmed
            .strip_prefix('v')
            .or_else(|| trimmed.strip_prefix('V'))
        {
            if !rest.starts_with(|c: char| c.is_ascii_digit()) {
                return Err(format!("Malformed version prefix in '{raw}'"));
            }
            rest
        } else {
            trimmed
        };

        // Split off +build metadata first
        let (core_and_pre, build_metadata) = match without_v.split_once('+') {
            Some((left, build)) => {
                if build.is_empty()
                    || !build
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
                {
                    return Err(format!("Invalid build metadata in '{raw}'"));
                }
                (left, Some(build.to_string()))
            }
            None => (without_v, None),
        };

        // Split off -prerelease
        let (core_part, pre_part) = match core_and_pre.split_once('-') {
            Some((left, pre)) => {
                if pre.is_empty() {
                    return Err(format!("Empty prerelease tag in '{raw}'"));
                }
                (left, Some(pre))
            }
            None => (core_and_pre, None),
        };

        let segments: Vec<&str> = core_part.split('.').collect();
        let (major_seg, minor_seg, patch_seg) = match segments.as_slice() {
            [major] => (*major, None, None),
            [major, minor] => (*major, Some(*minor), None),
            [major, minor, patch] => (*major, Some(*minor), Some(*patch)),
            _ => {
                return Err(format!(
                    "Expected 1 to 3 numeric version segments (e.g. 2, 1.0, or 1.0.0), got '{raw}'"
                ));
            }
        };

        let major: u64 = major_seg
            .parse()
            .map_err(|_| format!("Invalid major version in '{raw}'"))?;

        let (minor, patch, letter_suffix) = match (minor_seg, patch_seg) {
            (None, _) => (0, 0, None),
            (Some(minor_seg), None) => {
                let (min_num, suffix) = parse_num_with_optional_letter(minor_seg, raw)?;
                (min_num, 0, suffix)
            }
            (Some(minor_seg), Some(patch_seg)) => {
                let minor: u64 = minor_seg
                    .parse()
                    .map_err(|_| format!("Invalid minor version in '{raw}'"))?;
                let (patch_num, suffix) = parse_num_with_optional_letter(patch_seg, raw)?;
                (minor, patch_num, suffix)
            }
        };

        let mut prerelease = Vec::new();
        if let Some(pre) = pre_part {
            for part in pre.split('.') {
                if part.is_empty() || !part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return Err(format!("Invalid prerelease identifier '{part}' in '{raw}'"));
                }
                if part.chars().all(|c| c.is_ascii_digit()) {
                    let n: u64 = part
                        .parse()
                        .map_err(|_| format!("Invalid numeric prerelease '{part}' in '{raw}'"))?;
                    prerelease.push(PreReleaseIdent::Numeric(n));
                } else {
                    prerelease.push(PreReleaseIdent::AlphaNumeric(part.to_lowercase()));
                }
            }
        }

        Ok(ParsedVersion {
            major,
            minor,
            patch,
            letter_suffix,
            prerelease,
            build_metadata,
        })
    }
}

fn parse_num_with_optional_letter(seg: &str, raw: &str) -> Result<(u64, Option<char>), String> {
    let Some(last_char) = seg.chars().last() else {
        return Err(format!("Empty version segment in '{raw}'"));
    };
    if last_char.is_ascii_alphabetic() {
        let num_part = seg.strip_suffix(last_char).unwrap_or_default();
        if num_part.is_empty() || !num_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("Ambiguous version segment '{seg}' in '{raw}'"));
        }
        let n: u64 = num_part
            .parse()
            .map_err(|_| format!("Invalid number in '{raw}'"))?;
        Ok((n, Some(last_char.to_ascii_lowercase())))
    } else {
        let n: u64 = seg
            .parse()
            .map_err(|_| format!("Invalid numeric segment '{seg}' in '{raw}'"))?;
        Ok((n, None))
    }
}

/// Update evaluation outcome implementing Section 3 Version Parsing Policy:
/// - Unparseable remote tag -> Skip and flag in UI; never offer as update
/// - Unparseable local version -> Treat as 0.0.0, warn, never auto-update
/// - Ambiguity of any kind -> Never auto-update across it
/// - Prerelease tags -> Ordered per semver precedence; shown only if opted into prereleases
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateEvaluation {
    pub is_update_available: bool,
    pub allow_auto_update: bool,
    pub local_parsed: ParsedVersion,
    pub remote_parsed: Option<ParsedVersion>,
    pub warning: Option<String>,
    pub skipped_reason: Option<String>,
}

pub fn evaluate_update(
    local_raw: &str,
    remote_raw: &str,
    opt_in_prereleases: bool,
) -> UpdateEvaluation {
    let mut warning = None;
    let mut had_local_ambiguity = false;

    let local_parsed = match ParsedVersion::parse(local_raw) {
        Ok(v) => v,
        Err(e) => {
            had_local_ambiguity = true;
            warning = Some(format!(
                "Local version '{}' could not be parsed ({}); treating as 0.0.0",
                local_raw, e
            ));
            ParsedVersion::ZERO
        }
    };

    let remote_parsed = match ParsedVersion::parse(remote_raw) {
        Ok(v) => v,
        Err(e) => {
            return UpdateEvaluation {
                is_update_available: false,
                allow_auto_update: false,
                local_parsed,
                remote_parsed: None,
                warning: Some(format!(
                    "Remote tag '{}' is malformed ({}); skipping update",
                    remote_raw, e
                )),
                skipped_reason: Some("Unparseable remote tag".to_string()),
            };
        }
    };

    if remote_parsed.is_prerelease() && !opt_in_prereleases {
        return UpdateEvaluation {
            is_update_available: false,
            allow_auto_update: false,
            local_parsed,
            remote_parsed: Some(remote_parsed),
            warning,
            skipped_reason: Some(
                "Remote version is a prerelease and prerelease channel is disabled".to_string(),
            ),
        };
    }

    let is_newer = remote_parsed > local_parsed;
    let allow_auto = is_newer && !had_local_ambiguity && !remote_parsed.is_prerelease();

    UpdateEvaluation {
        is_update_available: is_newer,
        allow_auto_update: allow_auto,
        local_parsed,
        remote_parsed: Some(remote_parsed),
        warning,
        skipped_reason: None,
    }
}

/// SemVer requirement comparator supporting ranges like `>=1.2 <2`, `>=0.5.3`, `^1.0`, `~1.2`, `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReq {
    pub comparators: Vec<Comparator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompOp {
    Exact,
    Greater,
    GreaterEq,
    Less,
    LessEq,
    Caret,
    Tilde,
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparator {
    pub op: CompOp,
    pub version: ParsedVersion,
}

impl VersionReq {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "*" || trimmed.eq_ignore_ascii_case("any") {
            return Ok(Self {
                comparators: vec![Comparator {
                    op: CompOp::Wildcard,
                    version: ParsedVersion::ZERO,
                }],
            });
        }

        // Split on whitespace or commas, e.g. ">=1.2 <2.0" or ">=1.0.0, <2.0.0"
        let tokens: Vec<&str> = trimmed
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .collect();

        let mut comparators = Vec::new();
        for tok in tokens {
            if tok == "*" {
                comparators.push(Comparator {
                    op: CompOp::Wildcard,
                    version: ParsedVersion::ZERO,
                });
                continue;
            }
            let (op, ver_str) = if let Some(r) = tok.strip_prefix(">=") {
                (CompOp::GreaterEq, r)
            } else if let Some(r) = tok.strip_prefix("<=") {
                (CompOp::LessEq, r)
            } else if let Some(r) = tok.strip_prefix('>') {
                (CompOp::Greater, r)
            } else if let Some(r) = tok.strip_prefix('<') {
                (CompOp::Less, r)
            } else if let Some(r) = tok.strip_prefix('^') {
                (CompOp::Caret, r)
            } else if let Some(r) = tok.strip_prefix('~') {
                (CompOp::Tilde, r)
            } else if let Some(r) = tok.strip_prefix('=') {
                (CompOp::Exact, r)
            } else {
                (CompOp::Exact, tok)
            };

            let version = ParsedVersion::parse(ver_str)?;
            comparators.push(Comparator { op, version });
        }

        if comparators.is_empty() {
            return Err(format!("No valid version constraints in '{raw}'"));
        }

        Ok(Self { comparators })
    }

    pub fn matches(&self, v: &ParsedVersion) -> bool {
        self.comparators.iter().all(|c| c.matches(v))
    }
}

impl Comparator {
    pub fn matches(&self, v: &ParsedVersion) -> bool {
        match self.op {
            CompOp::Wildcard => true,
            CompOp::Exact => v == &self.version,
            CompOp::Greater => v > &self.version,
            CompOp::GreaterEq => v >= &self.version,
            CompOp::Less => v < &self.version,
            CompOp::LessEq => v <= &self.version,
            CompOp::Caret => {
                if v < &self.version {
                    return false;
                }
                if self.version.major > 0 {
                    v.major == self.version.major
                } else if self.version.minor > 0 {
                    v.major == 0 && v.minor == self.version.minor
                } else {
                    v.major == 0 && v.minor == 0 && v.patch == self.version.patch
                }
            }
            CompOp::Tilde => {
                v >= &self.version && v.major == self.version.major && v.minor == self.version.minor
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_and_three_segment_versions() {
        let v1 = ParsedVersion::parse("v1.2").unwrap();
        assert_eq!((v1.major, v1.minor, v1.patch), (1, 2, 0));

        let v2 = ParsedVersion::parse("1.2.3").unwrap();
        assert_eq!((v2.major, v2.minor, v2.patch), (1, 2, 3));

        let v_game = ParsedVersion::parse("0.5.3b").unwrap();
        assert_eq!(v_game.major, 0);
        assert_eq!(v_game.minor, 5);
        assert_eq!(v_game.patch, 3);
        assert_eq!(v_game.letter_suffix, Some('b'));
        assert!(v_game > ParsedVersion::parse("0.5.3").unwrap());
    }

    #[test]
    fn prerelease_ordering_and_update_policy() {
        let alpha = ParsedVersion::parse("1.0.0-alpha.1").unwrap();
        let beta = ParsedVersion::parse("1.0.0-beta.1").unwrap();
        let rel = ParsedVersion::parse("1.0.0").unwrap();
        assert!(alpha < beta);
        assert!(beta < rel);

        // Unparseable remote is skipped and never offered
        let eval_bad_remote = evaluate_update("1.0.0", "release-final-final", false);
        assert!(!eval_bad_remote.is_update_available);
        assert!(!eval_bad_remote.allow_auto_update);
        assert!(eval_bad_remote.warning.is_some());

        // Unparseable local treats as 0.0.0 with warning, disables auto-update
        let eval_bad_local = evaluate_update("invalid_ver", "1.0.0", false);
        assert!(eval_bad_local.is_update_available);
        assert!(!eval_bad_local.allow_auto_update);
        assert_eq!(eval_bad_local.local_parsed, ParsedVersion::ZERO);
        assert!(eval_bad_local.warning.is_some());

        // Prerelease hidden unless opted in
        let eval_pre_off = evaluate_update("1.0.0", "1.1.0-beta.1", false);
        assert!(!eval_pre_off.is_update_available);
        let eval_pre_on = evaluate_update("1.0.0", "1.1.0-beta.1", true);
        assert!(eval_pre_on.is_update_available);
        assert!(!eval_pre_on.allow_auto_update);
    }

    #[test]
    fn version_range_matching() {
        let req = VersionReq::parse(">=1.2 <2").unwrap();
        assert!(req.matches(&ParsedVersion::parse("1.2.0").unwrap()));
        assert!(req.matches(&ParsedVersion::parse("1.9.5").unwrap()));
        assert!(!req.matches(&ParsedVersion::parse("1.1.9").unwrap()));
        assert!(!req.matches(&ParsedVersion::parse("2.0.0").unwrap()));

        let game_req = VersionReq::parse(">=0.5.3").unwrap();
        assert!(game_req.matches(&ParsedVersion::parse("0.5.3b").unwrap()));
    }
}
