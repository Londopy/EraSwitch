// retrobar.rs
// the taskbar piece. RetroBar draws an XP taskbar and hides explorer's while it
// runs. two things it does that we have to undo ourselves:
//  - it switches explorer's taskbar to auto-hide and only switches it back on a
//    clean exit (ManagedShell's ExplorerHelper). we stop it hard, so we save
//    the appbar state first and put it back after.
//  - its theme lives in %LOCALAPPDATA%\RetroBar\settings.json, which we edit,
//    so the original file is kept to put back too.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::win_ops;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub exe: String,
    pub was_running: bool,
    // explorer taskbar's ABM_GETSTATE bits before we started
    pub appbar_state: u32,
    // settings.json as it was; None when there wasn't one
    pub settings_json: Option<String>,
}

pub fn settings_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("RetroBar")
        .join("settings.json")
}

// sets "Theme" and leaves every other RetroBar setting alone.
// a file that isn't a JSON object is replaced (RetroBar would reset it anyway).
pub fn with_theme(existing: Option<&str>, theme: &str) -> String {
    let mut obj = existing
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| match v {
            serde_json::Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    obj.insert("Theme".into(), serde_json::Value::String(theme.into()));
    serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap_or_default()
}

pub fn snapshot(exe: &Path) -> Saved {
    Saved {
        exe: exe.to_string_lossy().into_owned(),
        was_running: win_ops::is_running(exe),
        appbar_state: win_ops::taskbar_appbar_state(),
        settings_json: std::fs::read_to_string(settings_path()).ok(),
    }
}

pub fn apply(saved: &Saved, theme: &str) -> Result<()> {
    // switch.rs already checked the era; the name goes into RetroBar's own
    // file lookup, so check again right where it's written
    if !crate::era::safe_name(theme) {
        bail!("'{theme}' isn't a valid RetroBar theme name");
    }
    let exe = PathBuf::from(&saved.exe);
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, with_theme(saved.settings_json.as_deref(), theme))
        .with_context(|| format!("Writing {}", path.display()))?;

    // a copy that's already running wouldn't pick up the new theme
    if win_ops::is_running(&exe) {
        stop(&exe)?;
    }
    launch(&exe)?;

    // RetroBar hides explorer's taskbar once it's up; give it a few seconds
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while win_ops::explorer_taskbar_visible() {
        if std::time::Instant::now() >= deadline {
            // it didn't start (a missing .NET runtime, say): that's a failed
            // switch, and switch.rs rolls everything back
            bail!("RetroBar didn't take over the taskbar within 8 seconds");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

// after a reboot in XP mode: start it again if it isn't running
pub fn resume(saved: &Saved) -> Result<()> {
    let exe = PathBuf::from(&saved.exe);
    if !win_ops::is_running(&exe) {
        launch(&exe)?;
    }
    Ok(())
}

// stops every copy, and fails if one is still there afterwards (an elevated
// RetroBar can refuse to be stopped)
fn stop(exe: &Path) -> Result<()> {
    win_ops::kill_all(exe)?;
    if !win_ops::wait_gone(exe, Duration::from_secs(3)) {
        bail!(
            "RetroBar is still running and couldn't be stopped. Close it from its own menu, then try again."
        );
    }
    Ok(())
}

fn launch(exe: &Path) -> Result<()> {
    std::process::Command::new(exe)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .spawn()
        .with_context(|| format!("Starting {}", exe.display()))?;
    Ok(())
}

pub fn restore(saved: &Saved) -> Result<()> {
    let exe = PathBuf::from(&saved.exe);
    // the auto-hide fix below only sticks once RetroBar is really gone
    stop(&exe)?;

    // undo RetroBar's auto-hide (it only does that itself on a clean exit)
    win_ops::set_taskbar_appbar_state(saved.appbar_state);
    win_ops::show_explorer_taskbar();

    let path = settings_path();
    match &saved.settings_json {
        Some(text) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, text).with_context(|| format!("Restoring {}", path.display()))?
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }

    // you were using RetroBar before EraSwitch: keep it that way
    if saved.was_running {
        launch(&exe)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_is_set_on_a_fresh_file() {
        let out = with_theme(None, "Windows XP Blue");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["Theme"], "Windows XP Blue");
    }

    #[test]
    fn other_settings_survive() {
        let before = r#"{ "Theme": "Windows 95-98", "ShowClock": false, "Language": "English" }"#;
        let out = with_theme(Some(before), "Windows XP Silver");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["Theme"], "Windows XP Silver");
        assert_eq!(v["ShowClock"], false);
        assert_eq!(v["Language"], "English");
    }

    #[test]
    fn garbage_is_replaced() {
        let out = with_theme(Some("not json ["), "Windows XP Blue");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["Theme"], "Windows XP Blue");
        let out = with_theme(Some("[1, 2]"), "Windows XP Blue");
        assert!(serde_json::from_str::<serde_json::Value>(&out)
            .unwrap()
            .is_object());
    }
}
