use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityCapability {
    /// `os.execute`, `io.popen`, `os.remove`, `os.rename`
    ProcessExecution,
    /// `require("ffi")`, `ffi.cdef`, `ffi.load`, `package.loadlib`
    NativeFfi,
    /// `socket`, `http`, `https`, `enet`
    NetworkAccess,
    /// `io.open`, `io.output`, `io.input` (outside LÖVE `love.filesystem` sandbox)
    RawFilesystemIo,
    /// `loadstring`, `loadfile`, `dofile`
    DynamicCodeExecution,
}

impl SecurityCapability {
    pub fn label(&self) -> &'static str {
        match self {
            SecurityCapability::ProcessExecution => "Shell / OS Command Execution",
            SecurityCapability::NativeFfi => "Native DLL / LuaJIT FFI",
            SecurityCapability::NetworkAccess => "Network / Socket Access",
            SecurityCapability::RawFilesystemIo => "Raw Disk I/O (Outside LÖVE Sandbox)",
            SecurityCapability::DynamicCodeExecution => "Dynamic Code Loading",
        }
    }

    pub fn severity(&self) -> &'static str {
        match self {
            SecurityCapability::ProcessExecution | SecurityCapability::NativeFfi => "high",
            SecurityCapability::NetworkAccess => "medium",
            SecurityCapability::RawFilesystemIo | SecurityCapability::DynamicCodeExecution => "low",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub capability: SecurityCapability,
    pub severity: String,
    pub file: String,
    pub line: usize,
    pub pattern: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityScanReport {
    pub capabilities: Vec<SecurityCapability>,
    pub findings: Vec<SecurityFinding>,
    pub has_high_risk: bool,
}

impl SecurityScanReport {
    /// Scans all `.lua` and `.toml` files in `mod_dir` for elevated capabilities.
    pub fn scan_directory(mod_dir: &Path) -> Self {
        let mut findings = Vec::new();

        for entry in WalkDir::new(mod_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext != "lua" && ext != "toml" {
                continue;
            }
            let rel = path
                .strip_prefix(mod_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");

            if let Ok(content) = std::fs::read_to_string(path) {
                findings.extend(Self::scan_source(&rel, &content));
            }
        }

        Self::from_findings(findings)
    }

    pub fn scan_source(file_name: &str, content: &str) -> Vec<SecurityFinding> {
        let rules: &[(SecurityCapability, &str, &str)] = &[
            (
                SecurityCapability::ProcessExecution,
                "os.execute",
                r"\bos\.execute\s*\(",
            ),
            (
                SecurityCapability::ProcessExecution,
                "io.popen",
                r"\bio\.popen\s*\(",
            ),
            (
                SecurityCapability::ProcessExecution,
                "os.remove",
                r"\bos\.remove\s*\(",
            ),
            (
                SecurityCapability::NativeFfi,
                "require('ffi')",
                r#"require\s*\(?\s*["']ffi["']"#,
            ),
            (
                SecurityCapability::NativeFfi,
                "ffi.load / ffi.cdef",
                r"\bffi\.(load|cdef|C)\b",
            ),
            (
                SecurityCapability::NativeFfi,
                "package.loadlib",
                r"\bpackage\.loadlib\s*\(",
            ),
            (
                SecurityCapability::NetworkAccess,
                "socket / http",
                r#"require\s*\(?\s*["'](socket|socket\.http|ssl\.https|enet)["']"#,
            ),
            (
                SecurityCapability::RawFilesystemIo,
                "io.open",
                r"\bio\.open\s*\(",
            ),
            (
                SecurityCapability::DynamicCodeExecution,
                "loadstring / loadfile",
                r"\b(loadstring|loadfile|dofile)\s*\(",
            ),
        ];

        let compiled: Vec<(SecurityCapability, &str, regex_lite::Regex)> = rules
            .iter()
            .filter_map(|(cap, name, pat)| {
                regex_lite::Regex::new(pat).ok().map(|re| (*cap, *name, re))
            })
            .collect();

        let mut findings = Vec::new();
        for (idx, raw_line) in content.lines().enumerate() {
            let trimmed = raw_line.trim();
            if trimmed.starts_with("--") || trimmed.starts_with('#') {
                continue;
            }
            for (cap, pat_name, re) in &compiled {
                if re.is_match(trimmed) {
                    findings.push(SecurityFinding {
                        capability: *cap,
                        severity: cap.severity().to_string(),
                        file: file_name.to_string(),
                        line: idx + 1,
                        pattern: (*pat_name).to_string(),
                        snippet: trimmed.chars().take(120).collect(),
                    });
                }
            }
        }
        findings
    }

    pub fn from_findings(findings: Vec<SecurityFinding>) -> Self {
        let mut caps: Vec<SecurityCapability> = findings.iter().map(|f| f.capability).collect();
        caps.sort();
        caps.dedup();
        let has_high_risk = caps.iter().any(|c| {
            matches!(
                c,
                SecurityCapability::ProcessExecution | SecurityCapability::NativeFfi
            )
        });
        Self {
            capabilities: caps,
            findings,
            has_high_risk,
        }
    }
}

/// Per-mod `.qm-integrity.json` manifest stored inside `library/<mod_id>/<version>/`
/// for Wabbajack-style pristine library verification and trust tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityManifest {
    pub mod_id: String,
    pub version: String,
    pub archive_sha256: Option<String>,
    pub source_url: Option<String>,
    pub imported_at: String,
    /// Relative path -> SHA-256 hex digest
    pub files: BTreeMap<String, String>,
    pub security_report: SecurityScanReport,
}

impl IntegrityManifest {
    pub fn compute_for_directory(
        mod_dir: &Path,
        mod_id: &str,
        version: &str,
        archive_sha256: Option<String>,
        source_url: Option<String>,
        imported_at: String,
    ) -> Result<Self, String> {
        let mut files = BTreeMap::new();
        for entry in WalkDir::new(mod_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name == ".qm-integrity.json" {
                continue;
            }
            let rel = path
                .strip_prefix(mod_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(path)
                .map_err(|e| format!("Failed to read {} for hashing: {e}", path.display()))?;
            let hash = sha256_hex(&bytes);
            files.insert(rel, hash);
        }

        let security_report = SecurityScanReport::scan_directory(mod_dir);
        Ok(Self {
            mod_id: mod_id.to_string(),
            version: version.to_string(),
            archive_sha256,
            source_url,
            imported_at,
            files,
            security_report,
        })
    }

    /// Verifies that no files in `mod_dir` have been modified, deleted, or injected since import.
    pub fn verify_directory(&self, mod_dir: &Path) -> IntegrityVerifyResult {
        let mut modified = Vec::new();
        let mut missing = Vec::new();
        let mut untracked = Vec::new();

        for (rel, expected_hash) in &self.files {
            let full = mod_dir.join(rel);
            match std::fs::read(&full) {
                Ok(bytes) => {
                    let actual = sha256_hex(&bytes);
                    if &actual != expected_hash {
                        modified.push(rel.clone());
                    }
                }
                Err(_) => missing.push(rel.clone()),
            }
        }

        for entry in WalkDir::new(mod_dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if file_name == ".qm-integrity.json" {
                continue;
            }
            let rel = path
                .strip_prefix(mod_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            if !self.files.contains_key(&rel) {
                untracked.push(rel);
            }
        }

        let is_pristine = modified.is_empty() && missing.is_empty() && untracked.is_empty();
        IntegrityVerifyResult {
            is_pristine,
            modified,
            missing,
            untracked,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityVerifyResult {
    pub is_pristine: bool,
    pub modified: Vec<String>,
    pub missing: Vec<String>,
    pub untracked: Vec<String>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_lua_capabilities_and_ignores_comments() {
        let code = r#"
-- os.execute("ignored in comment")
local f = io.open("Mods/GMenu/config.lua", "r")
local p = io.popen("dir Mods /b")
"#;
        let findings = SecurityScanReport::scan_source("gmenu_core.lua", code);
        let report = SecurityScanReport::from_findings(findings);
        assert!(report.has_high_risk);
        assert!(report
            .capabilities
            .contains(&SecurityCapability::ProcessExecution));
        assert!(report
            .capabilities
            .contains(&SecurityCapability::RawFilesystemIo));
        assert_eq!(report.findings.len(), 2);
    }
}
