// config.rs
// AppConfig and its sections, loaded from %APPDATA%\EraSwitch\config.toml
// (or next to the exe when a `portable` file sits there)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::era::Era;
use crate::updater::UpdaterConfig;

pub const APP_NAME: &str = "EraSwitch";
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+e";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub hotkeys: HotkeysConfig,
    pub era: Era,
    pub tools: ToolsConfig,
    pub startup: StartupConfig,
    pub notifications: NotificationsConfig,
    pub updater: UpdaterConfig,
}

// UpdaterConfig lives in updater.rs (template); compare it field by field
impl PartialEq for UpdaterConfig {
    fn eq(&self, o: &Self) -> bool {
        self.enabled == o.enabled
            && self.channel == o.channel
            && self.check_interval_h == o.check_interval_h
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HotkeysConfig {
    // flips between Windows 11 and the era
    pub toggle: String,
}

impl Default for HotkeysConfig {
    fn default() -> Self {
        Self {
            toggle: DEFAULT_HOTKEY.into(),
        }
    }
}

// where the helper programs live; empty means "find it yourself"
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToolsConfig {
    pub retrobar: String,
    pub openshell: String,
    // the first-run "install the helpers?" offer was shown; switching with a
    // helper missing still offers again, since you asked for it then
    pub offered: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StartupConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NotificationsConfig {
    pub enabled: bool,
    pub on_update: bool,
    pub on_hotkey_fail: bool,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_update: true,
            on_hotkey_fail: true,
        }
    }
}

// a `portable` file next to the exe keeps everything in the exe's folder
pub fn is_portable() -> bool {
    exe_dir().is_some_and(|d| d.join("portable").exists())
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

// %APPDATA%\EraSwitch, or the exe's folder in portable mode
pub fn data_dir() -> PathBuf {
    if is_portable() {
        if let Some(d) = exe_dir() {
            return d;
        }
    }
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path();
    if !path.exists() {
        let cfg = AppConfig::default();
        save_config(&cfg)?;
        return Ok(cfg);
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("Reading {}", path.display()))?;
    match toml::from_str(&text) {
        Ok(cfg) => Ok(cfg),
        Err(e) => {
            // a broken file shouldn't brick the app: keep a copy and start fresh
            crate::dlog!("config.toml didn't parse ({e}); starting from defaults");
            let _ = std::fs::copy(&path, path.with_extension("toml.bad"));
            Ok(AppConfig::default())
        }
    }
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    let path = config_path();
    std::fs::create_dir_all(data_dir())?;
    let text = toml::to_string_pretty(cfg)?;
    // write then rename so a crash never leaves half a config
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_roundtrips() {
        let cfg = AppConfig::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: AppConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn missing_sections_fall_back_to_defaults() {
        let back: AppConfig = toml::from_str("[hotkeys]\ntoggle = \"ctrl+alt+x\"\n").unwrap();
        assert_eq!(back.hotkeys.toggle, "ctrl+alt+x");
        assert_eq!(back.era, Era::default());
        assert!(back.updater.enabled);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let back: AppConfig = toml::from_str("[general]\nfuture_thing = 3\n").unwrap();
        assert_eq!(back, AppConfig::default());
    }
}
