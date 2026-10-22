use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use zip::ZipArchive;

// Sidecar debug entry written to the dump dir
#[derive(Serialize, Debug)]
pub struct PatchDebugEntry {
    pub patch_source: PatchSource,
    pub regions: Vec<PatchRegion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

#[derive(Serialize, Debug, Clone)]
pub enum DebugPatchType {
    #[serde(rename = "pattern")]
    Pattern,
    #[serde(rename = "regex")]
    Regex,
    #[serde(rename = "copy")]
    Copy,
}

#[derive(Serialize, Debug, Clone)]
pub struct PatchSource {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    pub patch_type: DebugPatchType,
}

#[derive(Serialize, Debug)]
pub struct PatchRegion {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone)]
pub struct ByteRegion {
    pub start: usize,
    pub end: usize,
    pub delta: isize,
}

impl ByteRegion {
    /// Adjust this region based on an edit that occurred elsewhere.
    pub fn adjust(&mut self, edit_pos: usize, delta: isize) {
        if edit_pos <= self.start {
            self.start = self.start.saturating_add_signed(delta);
            self.end = self.end.saturating_add_signed(delta);
        } else if edit_pos < self.end {
            self.end = self.end.saturating_add_signed(delta);
        }
    }
}

/// Dirty byte-based debug entry.
#[derive(Debug)]
pub struct ByteDebugEntry {
    pub patch_source: PatchSource,
    pub regions: Vec<ByteRegion>,
    pub warnings: Option<Vec<String>>,
}

impl ByteDebugEntry {
    /// Adjust all regions in this entry based on the edit that occurred.
    pub fn adjust(&mut self, edit_pos: usize, delta: isize) {
        for region in &mut self.regions {
            region.adjust(edit_pos, delta);
        }
    }
}

#[derive(Serialize, Debug)]
pub struct PatchDebug {
    pub buffer_name: String,
    pub entries: Vec<PatchDebugEntry>,
}

impl PatchDebug {
    pub fn new(buffer_name: &str) -> Self {
        Self {
            buffer_name: buffer_name.to_string(),
            entries: Vec::new(),
        }
    }

    /// Convert from byte-based to line-based.
    pub fn from_byte_entries(buffer_name: &str, byte_entries: Vec<ByteDebugEntry>, rope: &crop::Rope) -> Self {
        let entries = byte_entries
            .into_iter()
            .map(|entry| PatchDebugEntry {
                patch_source: entry.patch_source,
                regions: entry
                    .regions
                    .into_iter()
                    .map(|r| PatchRegion {
                        start_line: rope.line_of_byte(r.start) + 1,
                        end_line: rope.line_of_byte(r.end.saturating_sub(1)) + 1,
                    })
                    .collect(),
                warnings: entry.warnings,
            })
            .collect();

        Self {
            buffer_name: buffer_name.to_string(),
            entries,
        }
    }
}

/// Dump the buffer and its sidecar.
pub fn write_dump(
    mod_dir: &Path,
    dir_name: &str,
    name: &str,
    buffer: &str,
    debug: Option<&PatchDebug>,
) {
    if name.chars().count() > 100 {
        return;
    }

    // Write to qm/<dir_name>/<name>. Always overwrite so dumps reflect the
    // current mod set / game version instead of the first launch ever.
    let dump_path = mod_dir.join("qm").join(dir_name).join(name);

    if let Some(parent) = dump_path.parent() {
        if !parent.is_dir() {
            if let Err(e) = fs::create_dir_all(parent) {
                log::error!("Failed to create directory at {parent:?}: {e:?}");
                return;
            }
        }
    }

    if let Err(e) = fs::write(&dump_path, buffer) {
        log::error!("Failed to write dump to {dump_path:?}: {e:?}");
        return;
    }

    let mut json_path = dump_path;
    json_path.add_extension("json");

    match debug {
        Some(debug) => {
            match serde_json::to_string_pretty(debug) {
                Ok(json) => {
                    if let Err(e) = fs::write(&json_path, json) {
                        log::error!("Failed to write debug JSON to {json_path:?}: {e:?}");
                    }
                }
                Err(e) => {
                    log::error!("Failed to serialize debug info: {e:?}");
                }
            }
        }
        None => (),
    }
}

/// Locate the game's fused executable containing the vanilla game archive.
pub fn find_game_archive() -> Result<fs::File, String> {
    // 1. Try std::env::current_exe()
    if let Ok(exe_path) = std::env::current_exe() {
        if let Ok(f) = fs::File::open(&exe_path) {
            if ZipArchive::new(f).is_ok() {
                return fs::File::open(&exe_path).map_err(|e| e.to_string());
            }
        }
        // Check next to current_exe
        if let Some(parent) = exe_path.parent() {
            let mnb = parent.join("mnb.exe");
            if mnb.exists() {
                if let Ok(f) = fs::File::open(&mnb) {
                    if ZipArchive::new(f).is_ok() {
                        return fs::File::open(&mnb).map_err(|e| e.to_string());
                    }
                }
            }
        }
    }

    // 2. Try current working directory
    let mnb = Path::new("mnb.exe");
    if mnb.exists() {
        if let Ok(f) = fs::File::open(mnb) {
            if ZipArchive::new(f).is_ok() {
                return fs::File::open(mnb).map_err(|e| e.to_string());
            }
        }
    }

    Err("Could not locate fused game archive (mnb.exe)".to_string())
}

/// Extract and dump all vanilla game files (or matching a filter) directly from the game archive.
pub fn dump_vanilla_files(
    mod_dir: &Path,
    target_dir: Option<&Path>,
    filter: Option<&str>,
) -> Result<(usize, PathBuf), String> {
    let file = find_game_archive()?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("Failed to read game archive as zip: {e}"))?;

    let out_dir = match target_dir {
        Some(p) => {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                mod_dir.join(p)
            }
        }
        None => mod_dir.join("qm").join("game-dump"),
    };

    if !out_dir.exists() {
        fs::create_dir_all(&out_dir)
            .map_err(|e| format!("Failed to create dump directory at {out_dir:?}: {e}"))?;
    }

    let filter_ext = filter.unwrap_or("lua");
    let dump_all = filter_ext == "*" || filter_ext == "all" || filter_ext == "*.*";

    let mut count = 0usize;

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };

        if entry.is_dir() {
            continue;
        }

        let name = entry.name().to_string();

        if !dump_all {
            if let Some(ext) = filter_ext.strip_prefix("*.") {
                if !name.ends_with(ext) {
                    continue;
                }
            } else if !name.ends_with(filter_ext) {
                continue;
            }
        }

        // Reject absolute paths / `..` traversal so entries can never escape out_dir
        let rel = match entry.enclosed_name() {
            Some(p) => p,
            None => {
                log::warn!("Skipping unsafe archive entry path: {name}");
                continue;
            }
        };
        let dest_path = out_dir.join(rel);
        if let Some(parent) = dest_path.parent() {
            if !parent.exists() {
                let _ = fs::create_dir_all(parent);
            }
        }

        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_ok() {
            if fs::write(&dest_path, &buf).is_ok() {
                count += 1;
            }
        }
    }

    log::info!("Dumped {count} vanilla files to {}", out_dir.display());
    Ok((count, out_dir))
}
