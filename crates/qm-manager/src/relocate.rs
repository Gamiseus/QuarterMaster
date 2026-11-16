//! Moving a QuarterMaster data folder (from `%APPDATA%\QuarterMaster` to beside the program).
use crate::profiles::is_reparse_point;
use std::fs;
use std::path::{Path, PathBuf};

/// Left in a data folder that was moved away, so it is not offered again.
pub const MOVED_MARKER: &str = "moved.txt";
/// Left in the cache of a moved data folder: the next start stages the active profile again,
/// because links are not copied.
pub const RESTAGE_MARKER: &str = "restage-needed";

/// `%APPDATA%\QuarterMaster`, where QuarterMaster keeps its data when it is not portable.
/// `QM_LEGACY_DATA_DIR` overrides it.
pub fn appdata_root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("QM_LEGACY_DATA_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(p));
    }
    dirs::data_dir().map(|d| d.join("QuarterMaster"))
}

/// A short Windows version, such as `Windows 24H2 (build 26200)`.
pub fn windows_version() -> Option<String> {
    #[cfg(windows)]
    {
        const KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let build = crate::storage::read_registry_string(KEY, "CurrentBuild")?;
        let release = crate::storage::read_registry_string(KEY, "DisplayVersion");
        Some(match release {
            Some(r) => format!("Windows {r} (build {build})"),
            None => format!("Windows (build {build})"),
        })
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Folders that are rebuilt and never worth moving.
fn skipped(rel: &Path) -> bool {
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    match parts.as_slice() {
        [a, b] if a == "cache" => b.starts_with("import-staging-") || b == "updates",
        [.., last] => last == ".qm-staging",
        [] => false,
    }
}

/// Copies everything under `old_root` into `new_root`, over what is there. Links (the mods of a
/// staged profile, workspace mods) are left out: they point into the old folder and are made
/// again. Returns the number of files copied.
pub fn copy_data_root(old_root: &Path, new_root: &Path) -> Result<u64, String> {
    fn walk(src: &Path, dst: &Path, rel: &Path, count: &mut u64) -> Result<(), String> {
        fs::create_dir_all(dst).map_err(|e| format!("Failed to create {}: {e}", dst.display()))?;
        let entries =
            fs::read_dir(src).map_err(|e| format!("Failed to read {}: {e}", src.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read {}: {e}", src.display()))?;
            let path = entry.path();
            let name = entry.file_name();
            let rel = rel.join(&name);
            if is_reparse_point(&path) || skipped(&rel) {
                continue;
            }
            let target = dst.join(&name);
            let meta = fs::symlink_metadata(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            if meta.is_dir() {
                walk(&path, &target, &rel, count)?;
            } else if meta.is_file() {
                fs::copy(&path, &target).map_err(|e| {
                    format!("Failed to copy {} to {}: {e}", path.display(), target.display())
                })?;
                *count += 1;
            }
        }
        Ok(())
    }
    if !old_root.is_dir() {
        return Err(format!("Folder not found: {}", old_root.display()));
    }
    let mut count = 0;
    walk(old_root, new_root, Path::new(""), &mut count)?;
    let _ = fs::remove_file(new_root.join(MOVED_MARKER));
    let cache = new_root.join("cache");
    let _ = fs::create_dir_all(&cache);
    fs::write(cache.join(RESTAGE_MARKER), b"")
        .map_err(|e| format!("Failed to write in {}: {e}", cache.display()))?;
    Ok(count)
}

/// Deletes a folder without ever following a link: a link is removed, its target is not touched.
pub fn remove_tree_keeping_link_targets(dir: &Path) -> Result<(), String> {
    remove_tree_keeping(dir, None).map(|_| ())
}

/// `remove_tree_keeping_link_targets`, leaving the folder `keep` (and so the folders above it)
/// in place. Ok(true) when `dir` itself is gone.
pub fn remove_tree_keeping(dir: &Path, keep: Option<&Path>) -> Result<bool, String> {
    let mut kept = false;
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {e}", dir.display()))?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if keep.is_some_and(|k| k == path) {
            kept = true;
            continue;
        }
        let fail = |e: std::io::Error| format!("Failed to remove {}: {e}", path.display());
        if is_reparse_point(&path) {
            // A junction or folder link goes with remove_dir, a file link with remove_file.
            fs::remove_dir(&path).or_else(|_| fs::remove_file(&path)).map_err(fail)?;
        } else if path.is_dir() {
            if !remove_tree_keeping(&path, keep)? {
                kept = true;
            }
        } else {
            fs::remove_file(&path)
                .or_else(|_| {
                    // Read-only files (a copied archive, say) need the flag cleared first.
                    let mut perms = fs::metadata(&path)?.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    perms.set_readonly(false);
                    fs::set_permissions(&path, perms)?;
                    fs::remove_file(&path)
                })
                .map_err(fail)?;
        }
    }
    if kept {
        return Ok(false);
    }
    fs::remove_dir(dir)
        .map(|()| true)
        .map_err(|e| format!("Failed to remove {}: {e}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::create_dir_junction_or_copy;

    #[test]
    fn a_data_folder_is_copied_without_its_links_and_removed_without_touching_their_targets() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(old.join("library/gcore/1.0.0")).unwrap();
        fs::write(old.join("library/gcore/1.0.0/qm.toml"), "x").unwrap();
        fs::create_dir_all(old.join("profiles/default/Mods/qm")).unwrap();
        fs::write(old.join("profiles/default/Mods/qm/state.json"), "{}").unwrap();
        fs::write(old.join("settings.json"), "{}").unwrap();
        fs::create_dir_all(old.join("cache/import-staging-abc")).unwrap();
        fs::write(old.join("cache/import-staging-abc/junk"), "x").unwrap();
        fs::write(old.join("cache/qm-dll-hashes.json"), "[]").unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let staged = old.join("profiles").join("default").join("Mods");
        let lib = old.join("library").join("gcore").join("1.0.0");
        create_dir_junction_or_copy(&lib, &staged.join("gcore")).unwrap();
        create_dir_junction_or_copy(&outside, &staged.join("linked")).unwrap();
        assert!(is_reparse_point(&staged.join("gcore")), "the test needs real links");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("settings.json"), "fresh").unwrap();

        let copied = copy_data_root(&old, &new).unwrap();
        assert_eq!(copied, 4, "the mod file, the state file, settings and the hashes");
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), "{}");
        assert!(new.join("library/gcore/1.0.0/qm.toml").is_file());
        assert!(new.join("profiles/default/Mods/qm/state.json").is_file());
        assert!(!new.join("profiles/default/Mods/gcore").exists(), "links are not copied");
        assert!(!new.join("cache/import-staging-abc").exists());
        assert!(new.join("cache").join(RESTAGE_MARKER).is_file());

        // A folder to keep stays, with the folders above it; everything else goes.
        let keep = new.join("library").join("gcore");
        assert!(!remove_tree_keeping(&new, Some(&keep)).unwrap());
        assert!(keep.join("1.0.0").join("qm.toml").is_file());
        assert!(!new.join("settings.json").exists() && !new.join("profiles").exists());

        remove_tree_keeping_link_targets(&old).unwrap();
        assert!(!old.exists());
        assert_eq!(fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");
    }
}
