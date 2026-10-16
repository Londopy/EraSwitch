// openshell.rs
// the Start menu piece. Open-Shell reads its settings from
// HKCU\Software\OpenShell\StartMenu\Settings; `StartMenu.exe -reloadsettings`
// picks up changes and `-exit` stops it.
//
// its installer also adds an HKLM Run entry (`StartMenu.exe -autorun`) that
// would bring the XP menu back at every login. that run quits when the
// per-user AutoStart value is 0 (StartMenu.cpp), so we keep it 0 unless you
// were already running Open-Shell before EraSwitch touched it.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
use winreg::RegKey;

use crate::era::StartMenuPiece;
use crate::win_ops;

pub const SETTINGS_KEY: &str = r"Software\OpenShell\StartMenu\Settings";

// one registry value as it was: both None = it didn't exist
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SavedValue {
    pub name: String,
    pub dword: Option<u32>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub exe: String,
    pub was_running: bool,
    pub values: Vec<SavedValue>,
}

// Open-Shell keeps one skin setting per menu style
pub fn skin_value_name(menu_style: u32) -> &'static str {
    match menu_style {
        0 => "SkinC1",
        2 => "SkinW7",
        _ => "SkinC2",
    }
}

fn touched_values(piece: &StartMenuPiece) -> [&'static str; 3] {
    ["MenuStyle", skin_value_name(piece.menu_style), "AutoStart"]
}

fn read_value(key: &RegKey, name: &str) -> SavedValue {
    SavedValue {
        name: name.into(),
        dword: key.get_value::<u32, _>(name).ok(),
        text: key.get_value::<String, _>(name).ok(),
    }
}

pub fn snapshot_at(subkey: &str, exe: &Path, piece: &StartMenuPiece) -> Saved {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let values = match hkcu.open_subkey_with_flags(subkey, KEY_READ) {
        Ok(key) => touched_values(piece)
            .iter()
            .map(|n| read_value(&key, n))
            .collect(),
        Err(_) => touched_values(piece)
            .iter()
            .map(|n| SavedValue {
                name: (*n).into(),
                ..Default::default()
            })
            .collect(),
    };
    Saved {
        exe: exe.to_string_lossy().into_owned(),
        was_running: win_ops::is_running(exe),
        values,
    }
}

pub fn write_settings_at(subkey: &str, piece: &StartMenuPiece) -> Result<()> {
    // the skin name becomes a file Open-Shell looks up; plain names only
    if !crate::era::safe_name(&piece.skin) || piece.menu_style > 2 {
        bail!("'{}' isn't a valid Open-Shell skin", piece.skin);
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(subkey)
        .with_context(|| format!("Opening HKCU\\{subkey}"))?;
    key.set_value("MenuStyle", &piece.menu_style)?;
    key.set_value(skin_value_name(piece.menu_style), &piece.skin)?;
    // never at login on its own: EraSwitch starts it while you're in the era
    key.set_value("AutoStart", &0u32)?;
    Ok(())
}

pub fn restore_values_at(subkey: &str, saved: &Saved) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // created if it went missing during the era, so a restore can always finish
    let (key, _) = hkcu
        .create_subkey(subkey)
        .with_context(|| format!("Opening HKCU\\{subkey}"))?;
    for v in &saved.values {
        // AutoStart only goes back to what it was if you used Open-Shell
        // before; otherwise it stays 0 so it can't take over Windows 11's menu
        if v.name == "AutoStart" && !saved.was_running {
            key.set_value("AutoStart", &0u32)?;
            continue;
        }
        match (&v.dword, &v.text) {
            (Some(d), _) => key.set_value(&v.name, d)?,
            (None, Some(t)) => key.set_value(&v.name, t)?,
            (None, None) => {
                let _ = key.delete_value(&v.name);
            }
        }
    }
    Ok(())
}

fn run(exe: &Path, arg: Option<&str>) -> Result<()> {
    let mut cmd = std::process::Command::new(exe);
    if let Some(a) = arg {
        cmd.arg(a);
    }
    cmd.spawn()
        .with_context(|| format!("Starting {}", exe.display()))?;
    Ok(())
}

pub fn snapshot(exe: &Path, piece: &StartMenuPiece) -> Saved {
    snapshot_at(SETTINGS_KEY, exe, piece)
}

pub fn apply(saved: &Saved, piece: &StartMenuPiece) -> Result<()> {
    write_settings_at(SETTINGS_KEY, piece)?;
    let exe = PathBuf::from(&saved.exe);
    if win_ops::is_running(&exe) {
        run(&exe, Some("-reloadsettings"))
    } else {
        run(&exe, None)
    }
}

pub fn resume(saved: &Saved) -> Result<()> {
    let exe = PathBuf::from(&saved.exe);
    if !win_ops::is_running(&exe) {
        run(&exe, None)?;
    }
    Ok(())
}

pub fn restore(saved: &Saved) -> Result<()> {
    let exe = PathBuf::from(&saved.exe);
    if !saved.was_running && win_ops::is_running(&exe) {
        // ask nicely first, then make sure
        let _ = run(&exe, Some("-exit"));
        if !win_ops::wait_gone(&exe, Duration::from_secs(5)) {
            win_ops::kill_all(&exe)?;
            if !win_ops::wait_gone(&exe, Duration::from_secs(3)) {
                bail!("Open-Shell is still running and couldn't be stopped");
            }
        }
    }
    restore_values_at(SETTINGS_KEY, saved)?;
    if saved.was_running {
        let _ = run(&exe, Some("-reloadsettings"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // a throwaway key so tests never touch the real Open-Shell settings
    fn test_key(tag: &str) -> String {
        format!(
            r"Software\EraSwitchTests\openshell-{tag}-{}",
            std::process::id()
        )
    }

    fn cleanup(subkey: &str) {
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(subkey);
    }

    #[test]
    fn skin_value_follows_menu_style() {
        assert_eq!(skin_value_name(0), "SkinC1");
        assert_eq!(skin_value_name(1), "SkinC2");
        assert_eq!(skin_value_name(2), "SkinW7");
    }

    #[test]
    fn apply_then_restore_puts_values_back() {
        let k = test_key("roundtrip");
        cleanup(&k);
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(&k).unwrap();
        key.set_value("MenuStyle", &2u32).unwrap();
        key.set_value("SkinW7", &"Midnight".to_string()).unwrap();
        // no SkinC2, no AutoStart

        let piece = StartMenuPiece::default();
        let mut saved = snapshot_at(&k, Path::new("C:/nope/StartMenu.exe"), &piece);
        saved.was_running = true; // as if you used Open-Shell before
        write_settings_at(&k, &piece).unwrap();
        assert_eq!(key.get_value::<u32, _>("MenuStyle").unwrap(), 1);
        assert_eq!(
            key.get_value::<String, _>("SkinC2").unwrap(),
            "Windows XP Luna"
        );
        assert_eq!(key.get_value::<u32, _>("AutoStart").unwrap(), 0);

        restore_values_at(&k, &saved).unwrap();
        assert_eq!(key.get_value::<u32, _>("MenuStyle").unwrap(), 2);
        assert!(key.get_value::<String, _>("SkinC2").is_err());
        assert!(key.get_value::<u32, _>("AutoStart").is_err());
        assert_eq!(key.get_value::<String, _>("SkinW7").unwrap(), "Midnight");
        cleanup(&k);
    }

    #[test]
    fn autostart_stays_off_when_open_shell_was_not_in_use() {
        let k = test_key("autostart");
        cleanup(&k);
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(&k).unwrap();
        // what Open-Shell's installer leaves behind
        key.set_value("AutoStart", &1u32).unwrap();

        let piece = StartMenuPiece::default();
        let saved = snapshot_at(&k, Path::new("C:/nope/StartMenu.exe"), &piece);
        assert!(!saved.was_running);
        write_settings_at(&k, &piece).unwrap();
        restore_values_at(&k, &saved).unwrap();
        assert_eq!(key.get_value::<u32, _>("AutoStart").unwrap(), 0);
        cleanup(&k);
    }
}
