//! The machine's `PATH`, for the installer's "add qm to PATH" option. Only the installer and the
//! uninstaller call this, through `qm-launcher.exe --path-add` and `--path-remove`, and they run
//! with administrator rights.

/// Two `PATH` entries name the same folder: case and a trailing `\` do not matter.
fn same_entry(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_matches('"')
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// `path` with `dir` added at the end; None when it is already listed.
pub fn with_entry(path: &str, dir: &str) -> Option<String> {
    if path.split(';').any(|e| same_entry(e, dir)) {
        return None;
    }
    let base = path.trim_end_matches(';');
    Some(if base.is_empty() {
        dir.to_string()
    } else {
        format!("{base};{dir}")
    })
}

/// `path` without `dir`; None when it is not listed. Every other entry is kept as written.
pub fn without_entry(path: &str, dir: &str) -> Option<String> {
    if !path.split(';').any(|e| same_entry(e, dir)) {
        return None;
    }
    let kept: Vec<&str> = path
        .split(';')
        .filter(|e| !e.is_empty() && !same_entry(e, dir))
        .collect();
    Some(kept.join(";"))
}

/// Adds `dir` to the machine `PATH`, or removes it. Ok(true) when the value changed. The value
/// keeps its type and its `%VAR%` references, and is never written when it could not be read.
#[cfg(windows)]
pub fn set_machine_path(dir: &std::path::Path, add: bool) -> Result<bool, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    const KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (key, name) = (wide(KEY), wide("Path"));
    let dir = dir.to_string_lossy();
    if dir.is_empty() || dir.contains(';') {
        return Err(format!("'{dir}' cannot be a PATH entry."));
    }

    let mut hkey = HKEY::default();
    // SAFETY: `key` is NUL-terminated and outlives the call; `hkey` receives the handle.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("The machine PATH could not be opened ({status:?})."));
    }
    let result = (|| {
        let mut kind = REG_VALUE_TYPE::default();
        let mut buf = vec![0u8; 8192];
        let mut read = None;
        for _ in 0..4 {
            let mut bytes = buf.len() as u32;
            // SAFETY: `name` is NUL-terminated; `bytes` is the buffer's size in bytes.
            let status = unsafe {
                RegQueryValueExW(
                    hkey,
                    PCWSTR(name.as_ptr()),
                    None,
                    Some(&mut kind),
                    Some(buf.as_mut_ptr()),
                    Some(&mut bytes),
                )
            };
            if status == ERROR_MORE_DATA {
                buf.resize(bytes as usize + 2, 0);
                continue;
            }
            if status != ERROR_SUCCESS {
                return Err(format!("The machine PATH could not be read ({status:?})."));
            }
            read = Some(bytes as usize);
            break;
        }
        let len = read.ok_or("The machine PATH could not be read.")?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err("The machine PATH is not text.".to_string());
        }
        let data = &buf[..len.min(buf.len())];
        let units: Vec<u16> = (0..data.len() / 2)
            .map(|i| u16::from_le_bytes([data[2 * i], data[2 * i + 1]]))
            .collect();
        let end = units.iter().position(|c| *c == 0).unwrap_or(units.len());
        let current = String::from_utf16(&units[..end])
            .map_err(|_| "The machine PATH is not valid text.".to_string())?;
        if current.trim().is_empty() {
            // An empty machine PATH means something is wrong; do not replace it.
            return Err("The machine PATH is empty.".to_string());
        }
        let next = if add {
            with_entry(&current, &dir)
        } else {
            without_entry(&current, &dir)
        };
        let Some(next) = next else {
            return Ok(false);
        };
        let data: Vec<u8> = wide(&next).iter().flat_map(|u| u.to_le_bytes()).collect();
        // SAFETY: `name` is NUL-terminated; `data` is the NUL-terminated UTF-16 value.
        let status =
            unsafe { RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, kind, Some(&data)) };
        if status != ERROR_SUCCESS {
            return Err(format!("The machine PATH could not be written ({status:?})."));
        }
        Ok(true)
    })();
    // SAFETY: `hkey` was opened above and is closed once.
    let _ = unsafe { RegCloseKey(hkey) };
    if matches!(result, Ok(true)) {
        // Tell running programs (Explorer, so new terminals) that the environment changed.
        let env = wide("Environment");
        // SAFETY: `env` is NUL-terminated and outlives the call; the result is not needed.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(env.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                3000,
                None,
            );
        }
    }
    result
}

#[cfg(not(windows))]
pub fn set_machine_path(_dir: &std::path::Path, _add: bool) -> Result<bool, String> {
    Err("Only on Windows.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const QM: &str = r"C:\Program Files\QuarterMaster";

    #[test]
    fn an_entry_is_added_once_and_the_rest_is_kept_as_written() {
        let path = r"%SystemRoot%\system32;C:\Tools\;";
        let added = with_entry(path, QM).unwrap();
        assert_eq!(added, r"%SystemRoot%\system32;C:\Tools\;C:\Program Files\QuarterMaster");
        assert_eq!(with_entry(&added, QM), None);
        assert_eq!(with_entry(&added, r"c:\program files\quartermaster\"), None);
        assert_eq!(with_entry("", QM).unwrap(), QM);
    }

    #[test]
    fn an_entry_is_removed_wherever_it_is_and_nothing_else_changes() {
        let path = r"%SystemRoot%\system32;c:\program files\QuarterMaster\;C:\Tools";
        assert_eq!(without_entry(path, QM).unwrap(), r"%SystemRoot%\system32;C:\Tools");
        assert_eq!(without_entry(r"C:\Tools", QM), None);
        // A folder that only starts the same is another entry.
        assert_eq!(without_entry(r"C:\Program Files\QuarterMaster2", QM), None);
    }
}
