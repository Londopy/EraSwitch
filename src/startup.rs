// startup.rs
// run at login (the same HKCU Run value the installer's opt-in task writes)
// and the AppUserModelID that gives toasts EraSwitch's name

use anyhow::Result;
use std::path::Path;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

// same string the installers use
pub const AUMID: &str = "Londopy.EraSwitch";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "EraSwitch";

pub fn setup_aumid() {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey(format!(r"Software\Classes\AppUserModelId\{AUMID}")) {
        let _ = key.set_value("DisplayName", &"EraSwitch".to_string());
    }
    let id: Vec<u16> = AUMID.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            windows::core::PCWSTR(id.as_ptr()),
        );
    }
}

pub fn is_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

pub fn set_enabled(enabled: bool, exe: &Path) -> Result<()> {
    let key =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)?;
    if enabled {
        key.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()))?;
    } else {
        let _ = key.delete_value(RUN_VALUE);
    }
    Ok(())
}

// keeps the Run value in step with config, pointing at this exe
pub fn sync(enabled: bool) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if let Err(e) = set_enabled(enabled, &exe) {
        crate::dlog!("startup: updating the Run value failed: {e}");
    }
}
