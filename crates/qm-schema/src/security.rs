use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use walkdir::WalkDir;

#[cfg_attr(feature = "specta", derive(specta::Type))]
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

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityFinding {
    pub capability: SecurityCapability,
    pub severity: String,
    pub file: String,
    pub line: usize,
    pub pattern: String,
    pub snippet: String,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityScanReport {
    pub capabilities: Vec<SecurityCapability>,
    pub findings: Vec<SecurityFinding>,
    pub has_high_risk: bool,
}

impl SecurityScanReport {
    /// Scans all `.lua` and `.toml` files in `mod_dir` for elevated capabilities, and every
    /// other file a patch file names as a module or copy `source`: the game runs those as
    /// Lua whatever they are called.
    pub fn scan_directory(mod_dir: &Path) -> Self {
        let mut findings = Vec::new();
        // Sources with another extension, as paths relative to `mod_dir` with `/`.
        let mut named: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

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
                let sources = if ext == "toml" {
                    patch_sources(&content)
                } else {
                    lua_named_files(&content)
                };
                {
                    // A source is looked up from the mod's folder, then from the file's own.
                    let here = rel.rsplit_once('/').map_or("", |(dir, _)| dir);
                    for source in sources {
                        let source = source.replace('\\', "/");
                        named.insert(source.clone());
                        if !here.is_empty() {
                            named.insert(format!("{here}/{source}"));
                        }
                    }
                }
            }
        }
        for rel in named {
            let is_scanned = [".lua", ".toml"]
                .iter()
                .any(|e| rel.to_ascii_lowercase().ends_with(e));
            if is_scanned || rel.split('/').any(|seg| seg == "..") {
                continue;
            }
            let path = mod_dir.join(&rel);
            if !path.is_file() {
                continue;
            }
            // Text of a sensible size only: a large or binary file is no script.
            if path.metadata().is_ok_and(|m| m.len() > 2 * 1024 * 1024) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
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
                SecurityCapability::ProcessExecution,
                "os.rename",
                r"\bos\.rename\s*\(",
            ),
            (
                SecurityCapability::NativeFfi,
                "require('ffi')",
                r#"require\s*[(,]?\s*["']ffi["']"#,
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
                r#"require\s*[(,]?\s*["'](socket|socket\.http|ssl\.https|enet)["']"#,
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

#[cfg_attr(feature = "specta", derive(specta::Type))]
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

#[cfg_attr(feature = "specta", derive(specta::Type))]
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

/// The module and copy sources a patch file names, in the long form (`[patches.module] source`,
/// `[patches.copy] sources`) and the short one (`[[module]]`, `[[copy]]`). Empty when the text
/// is no patch file.
fn patch_sources(toml_text: &str) -> Vec<String> {
    let Ok(doc) = toml_text.parse::<toml::Value>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let module = |m: Option<&toml::Value>, out: &mut Vec<String>| {
        if let Some(s) = m.and_then(|m| m.get("source")).and_then(|s| s.as_str()) {
            out.push(s.to_string());
        }
    };
    let copy = |c: Option<&toml::Value>, out: &mut Vec<String>| {
        let sources = c.and_then(|c| c.get("sources")).and_then(|s| s.as_array());
        for s in sources.into_iter().flatten().filter_map(|s| s.as_str()) {
            out.push(s.to_string());
        }
    };
    let list = |key: &str| doc.get(key).and_then(|p| p.as_array()).into_iter().flatten();
    for patch in list("patches") {
        module(patch.get("module"), &mut out);
        copy(patch.get("copy"), &mut out);
    }
    for patch in list("module") {
        module(Some(patch), &mut out);
    }
    for patch in list("copy") {
        copy(Some(patch), &mut out);
    }
    out
}

/// Extensions that are never code: a file named in Lua with one of these is not read as Lua.
pub const NOT_CODE: [&str; 22] = [
    "png", "jpg", "jpeg", "bmp", "gif", "tga", "webp", "dds", "ogg", "wav", "mp3", "flac", "ttf",
    "otf", "fnt", "glsl", "json", "md", "csv", "zip", "ico", "svg",
];

/// File names written as string literals in Lua code (`dofile("extra.dat")`,
/// `love.filesystem.load('parts/more.txt')`): a mod can load any file as code at run time, so a
/// file of the mod that its Lua names is scanned too. Comments are read as well; a name that
/// matches no file of the mod costs nothing.
fn lua_named_files(lua_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = lua_text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end] != quote && bytes[end] != b'\n' {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == quote {
            let text = &lua_text[start..end];
            let ext = text.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            let plain = !text.is_empty()
                && text.len() <= 200
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '\\' | '.' | '_' | '-' | ' '));
            if let Some(ext) = ext {
                if plain && !ext.is_empty() && ext.len() <= 8 && !NOT_CODE.contains(&ext.as_str()) {
                    out.push(text.replace('\\', "/").trim_start_matches("./").to_string());
                }
            }
        }
        i = end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file the mod's Lua names is scanned whatever its extension; images, sounds and files
    /// nothing names are not.
    #[test]
    fn a_file_named_in_lua_is_scanned_as_code() {
        let temp = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let p = temp.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("main_mod.lua", "local f = love.filesystem.load('parts/more.dat')\ndofile(\"side.txt\")\nlocal img = 'art/logo.png'\n");
        write("parts/more.dat", "os.execute('calc')\n");
        write("side.txt", "io.popen('dir')\n");
        write("art/logo.png", "os.execute('not code')\n");
        write("unnamed.dat", "os.execute('nobody loads this')\n");
        // The short form names its sources too.
        write("qm.toml", "[manifest]\nversion = \"1.0.0\"\n\n[[module]]\nsource = \"short.bin\"\nname = \"s\"\n\n[[copy]]\ntarget = \"main.lua\"\nposition = \"append\"\nsources = [\"tail.bin\"]\n");
        write("short.bin", "os.execute('a')\n");
        write("tail.bin", "os.execute('b')\n");
        let report = SecurityScanReport::scan_directory(temp.path());
        let mut files: Vec<&str> = report.findings.iter().map(|f| f.file.as_str()).collect();
        files.sort();
        files.dedup();
        assert_eq!(files, ["main_mod.lua", "parts/more.dat", "short.bin", "side.txt", "tail.bin"], "{:?}", report.findings);
    }

    /// Lua under another extension is scanned when a patch file names it as a source.
    #[test]
    fn scans_the_files_a_patch_file_names_as_sources() {
        let dir = tempfile::tempdir().unwrap();
        let write = |rel: &str, text: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(
            "qm.toml",
            "[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.module]\nsource = \"data/notes.txt\"\nbefore = \"main.lua\"\nname = \"notes\"\n\n[[patches]]\n[patches.copy]\ntarget = \"main.lua\"\nposition = \"append\"\nsources = [\"extra.dat\"]\n",
        );
        write("data/notes.txt", "os.execute('calc')\n");
        write("extra.dat", "local f = io.open('x', 'w')\n");
        // Not named by any patch file: text that only mentions a call is left alone.
        write("README.txt", "This mod never calls os.execute.\n");
        let report = SecurityScanReport::scan_directory(dir.path());
        let files: Vec<&str> = report.findings.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(files, ["data/notes.txt", "extra.dat"], "{:?}", report.findings);
        assert!(report.has_high_risk);
    }

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

    /// `pcall(require, "ffi")` asks for a module as much as `require("ffi")` does.
    #[test]
    fn a_module_asked_for_through_pcall_is_found() {
        for (code, capability) in [
            ("local ok, ffi = pcall(require, \"ffi\")", SecurityCapability::NativeFfi),
            ("local ok, s = pcall(require, 'socket')", SecurityCapability::NetworkAccess),
            ("local ffi = require 'ffi'", SecurityCapability::NativeFfi),
        ] {
            let findings = SecurityScanReport::scan_source("a.lua", code);
            assert_eq!(findings.len(), 1, "{code}");
            assert_eq!(findings[0].capability, capability, "{code}");
        }
    }

    /// Every pattern the wiki's security table names for "Running programs" is found.
    #[test]
    fn each_program_running_call_is_found() {
        for call in ["os.execute", "io.popen", "os.remove", "os.rename"] {
            let findings = SecurityScanReport::scan_source("a.lua", &format!("{call}(x)\n"));
            assert_eq!(findings.len(), 1, "{call}");
            assert_eq!(findings[0].capability, SecurityCapability::ProcessExecution);
            assert_eq!(findings[0].severity, "high");
        }
    }
}
