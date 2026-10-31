//! Windows process helpers: snapshot-based process lookup and console-free system commands.
use std::process::Command;

/// `CREATE_NO_WINDOW`: console programs spawned by the launcher get no visible window.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Pid of the first running process whose image name equals `exe_name` (case-insensitive).
#[cfg(windows)]
pub fn find_process(exe_name: &str) -> Option<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: plain Win32 calls; the entry buffer is sized via dwSize and the handle is closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = None;
        let mut more = Process32FirstW(snap, &mut entry).is_ok();
        while more {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(exe_name) {
                found = Some(entry.th32ProcessID);
                break;
            }
            more = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub fn find_process(_exe_name: &str) -> Option<u32> {
    None
}

/// A `Command` for a console program in `%SystemRoot%\System32` (e.g. `cmd`, `reg`), called by
/// absolute path and spawned without a console window.
pub fn system_command(program: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let exe = std::path::Path::new(&root)
            .join("System32")
            .join(format!("{program}.exe"));
        let mut cmd = Command::new(exe);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn finds_current_process_case_insensitively() {
        let exe = std::env::current_exe().expect("current exe");
        let name = exe
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .to_uppercase();
        assert!(find_process(&name).is_some(), "{name} not found");
        assert_eq!(find_process("definitely-not-running-qm.exe"), None);
    }

    #[cfg(windows)]
    #[test]
    fn system_command_uses_absolute_system32_path() {
        let cmd = system_command("reg");
        let program = std::path::PathBuf::from(cmd.get_program());
        assert!(program.is_absolute(), "{}", program.display());
        assert!(program.ends_with(r"System32\reg.exe"), "{}", program.display());
        let out = system_command("cmd")
            .args(["/C", "exit", "0"])
            .output()
            .expect("run cmd");
        assert!(out.status.success());
    }
}
