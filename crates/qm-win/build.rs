use std::env;

fn main() {
    let system_drive = env::var("SYSTEMDRIVE").unwrap_or_else(|_| "C:".to_string());
    let windows_path = format!("{}\\Windows\\System32\\winmm.dll", system_drive);

    let dev_path = env::var("QM_VERSION_PATH");
    if let Ok(dev_path) = dev_path {
        forward_dll::forward_dll_with_dev_path(&windows_path, &dev_path).unwrap();
    } else {
        #[cfg(target_os = "windows")]
        forward_dll::forward_dll(&windows_path).unwrap();
        #[cfg(not(target_os = "windows"))]
        forward_dll::forward_dll_with_dev_path(&windows_path, "../../winmm.dll").unwrap();
    }
}
