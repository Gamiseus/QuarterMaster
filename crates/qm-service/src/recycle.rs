//! Moving a file or folder to the Windows Recycle Bin, so a delete in the Mod Engine can be
//! undone from Explorer.
use std::path::Path;

/// Moves `path` to the Recycle Bin without asking and without a progress window. On a drive
/// without a Recycle Bin, Windows deletes it.
#[cfg(windows)]
pub(crate) fn to_recycle_bin(path: &Path) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
        SHFILEOPSTRUCTW,
    };
    let full = std::path::absolute(path).map_err(|e| e.to_string())?;
    // A list of paths, each NUL-terminated, ending with an extra NUL.
    let mut from: Vec<u16> = full.as_os_str().to_string_lossy().encode_utf16().collect();
    from.push(0);
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    // SAFETY: `op` is filled as the API documents, and `from` outlives the call.
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(format!("SHFileOperation failed with code {rc:#x}"));
    }
    if op.fAnyOperationsAborted.as_bool() || path.exists() {
        return Err("the delete was cancelled".to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn to_recycle_bin(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Puts a small file into this user's Recycle Bin, so it does not run by default.
    #[test]
    #[ignore = "writes to the Recycle Bin"]
    fn a_file_goes_to_the_recycle_bin() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("qm-recycle-test.txt");
        std::fs::write(&f, "QuarterMaster Recycle Bin test; safe to delete.").unwrap();
        to_recycle_bin(&f).unwrap();
        assert!(!f.exists());
        let sub = dir.path().join("folder");
        std::fs::create_dir_all(sub.join("inner")).unwrap();
        std::fs::write(sub.join("inner").join("a.lua"), "x").unwrap();
        to_recycle_bin(&sub).unwrap();
        assert!(!sub.exists());
    }
}
