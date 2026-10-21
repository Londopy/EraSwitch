// tools.rs
// finds the helper programs EraSwitch drives (RetroBar for the taskbar,
// Open-Shell for the Start menu) and installs them with winget on request.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::config::ToolsConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    RetroBar,
    OpenShell,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::RetroBar => "RetroBar",
            Tool::OpenShell => "Open-Shell",
        }
    }

    pub fn winget_id(self) -> &'static str {
        match self {
            Tool::RetroBar => "dremin.RetroBar",
            Tool::OpenShell => "Open-Shell.Open-Shell-Menu",
        }
    }

    pub fn homepage(self) -> &'static str {
        match self {
            Tool::RetroBar => "https://github.com/dremin/RetroBar",
            Tool::OpenShell => "https://github.com/Open-Shell/Open-Shell-Menu",
        }
    }

    fn configured(self, cfg: &ToolsConfig) -> &str {
        match self {
            Tool::RetroBar => &cfg.retrobar,
            Tool::OpenShell => &cfg.openshell,
        }
    }

    // where the installers put them (per-user or per-machine, depending on
    // how they were installed), so both kinds of folder are checked.
    fn known_locations(self, env: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<PathBuf> {
        let mut out = Vec::new();
        match self {
            Tool::RetroBar => {
                if let Some(d) = env("LOCALAPPDATA") {
                    out.push(d.join("Programs").join("RetroBar").join("RetroBar.exe"));
                    // winget's portable install (ARM64): a link to the real exe
                    out.push(
                        d.join("Microsoft")
                            .join("WinGet")
                            .join("Links")
                            .join("RetroBar.exe"),
                    );
                }
                for var in ["ProgramFiles", "ProgramFiles(x86)"] {
                    if let Some(d) = env(var) {
                        out.push(d.join("RetroBar").join("RetroBar.exe"));
                    }
                }
            }
            Tool::OpenShell => {
                for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                    if let Some(d) = env(var) {
                        out.push(d.join("Open-Shell").join("StartMenu.exe"));
                    }
                }
            }
        }
        out
    }
}

fn real_env(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

// a configured path wins (even a wrong one, so the UI can say it's wrong);
// otherwise the first known location that exists, then the folder its
// uninstall entry names (a custom install folder)
pub fn locate(tool: Tool, cfg: &ToolsConfig) -> Option<PathBuf> {
    let configured = !tool.configured(cfg).trim().is_empty();
    let found = locate_with(tool, cfg, &real_env, &|p: &Path| p.is_file()).or_else(|| {
        // only when nothing is configured: a wrong path in Settings must show
        // up as wrong, not be papered over by another copy
        if configured {
            None
        } else {
            from_uninstall_entry(tool)
        }
    });
    // a link (winget's portable installs) is followed to the real exe, which
    // is the path the running process reports
    found.map(|p| match std::fs::read_link(&p) {
        Ok(target) if target.is_absolute() => target,
        _ => p,
    })
}

// RetroBar's installer has a fixed product code; its uninstall entry records
// where it went. (Open-Shell's MSI code changes per version, and it always
// lands in Program Files anyway.)
fn from_uninstall_entry(tool: Tool) -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    if tool != Tool::RetroBar {
        return None;
    }
    let id = "{574527FE-00A4-4F85-92AD-B4B8B4077D73}_is1";
    let places = [
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_CURRENT_USER,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ];
    places.iter().find_map(|(root, key)| {
        let dir: String = RegKey::predef(*root)
            .open_subkey(format!(r"{key}\{id}"))
            .and_then(|k| k.get_value("InstallLocation"))
            .ok()?;
        let exe = PathBuf::from(dir.trim_matches('"')).join("RetroBar.exe");
        exe.is_file().then_some(exe)
    })
}

// the helpers the era's enabled pieces need that can't be found
pub fn missing(cfg: &crate::config::AppConfig) -> Vec<Tool> {
    let mut out = Vec::new();
    if cfg.era.taskbar.enabled && locate(Tool::RetroBar, &cfg.tools).is_none() {
        out.push(Tool::RetroBar);
    }
    if cfg.era.start_menu.enabled && locate(Tool::OpenShell, &cfg.tools).is_none() {
        out.push(Tool::OpenShell);
    }
    out
}

pub fn parse_list(s: &str) -> Vec<Tool> {
    s.split(',')
        .filter_map(|t| match t.trim().to_ascii_lowercase().as_str() {
            "retrobar" => Some(Tool::RetroBar),
            "openshell" | "open-shell" => Some(Tool::OpenShell),
            _ => None,
        })
        .collect()
}

pub fn winget_available() -> bool {
    std::process::Command::new("winget")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// installs each tool with winget, then undoes what their installers do on
// their own: Open-Shell's sets itself to start at login and takes over the
// Windows 11 Start menu straight away, and a RetroBar that gets started would
// take over the taskbar (and switch it to auto-hide). EraSwitch starts both
// only while the era is on. returns one line per tool for a summary.
pub fn install(tools: &[Tool], cfg: &ToolsConfig, data_dir: &Path) -> (bool, Vec<String>) {
    let era_on = crate::switch::is_active(data_dir);
    let appbar_before = crate::win_ops::taskbar_appbar_state();
    let mut ok = true;
    let mut lines = Vec::new();
    for &tool in tools {
        if locate(tool, cfg).is_some() {
            lines.push(format!("{}: already installed", tool.label()));
            continue;
        }
        crate::dlog!("tools: installing {} with winget", tool.label());
        let result = winget_install(tool).and_then(|()| {
            locate(tool, cfg).with_context(|| {
                format!(
                    "winget finished, but {} isn't where EraSwitch looks for it",
                    tool.label()
                )
            })
        });
        match result {
            Ok(exe) => {
                if !era_on {
                    let tidy = match tool {
                        Tool::OpenShell => crate::openshell::tame_after_install(&exe),
                        Tool::RetroBar => crate::retrobar::tame_after_install(&exe, appbar_before),
                    };
                    if let Err(e) = tidy {
                        crate::dlog!("tools: tidying after {} failed: {e:#}", tool.label());
                    }
                }
                lines.push(format!("{}: installed", tool.label()));
            }
            Err(e) => {
                ok = false;
                lines.push(format!("{}: not installed ({e:#})", tool.label()));
            }
        }
    }
    (ok, lines)
}

fn locate_with(
    tool: Tool,
    cfg: &ToolsConfig,
    env: &dyn Fn(&str) -> Option<PathBuf>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let configured = tool.configured(cfg).trim();
    if !configured.is_empty() {
        let p = PathBuf::from(configured);
        return exists(&p).then_some(p);
    }
    tool.known_locations(env).into_iter().find(|p| exists(p))
}

pub fn require(tool: Tool, cfg: &ToolsConfig) -> Result<PathBuf> {
    match locate(tool, cfg) {
        Some(p) => Ok(p),
        None => bail!(
            "{} isn't installed (or its path in Settings is wrong). Install it from Settings > General, or from {}",
            tool.label(),
            tool.homepage()
        ),
    }
}

// runs `winget install` in a visible console so the user sees progress and
// any UAC prompt (Open-Shell installs per-machine). blocks until it finishes.
pub fn winget_install(tool: Tool) -> Result<()> {
    let status = std::process::Command::new("winget")
        .args([
            "install",
            "--id",
            tool.winget_id(),
            "--exact",
            "--source",
            "winget",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ])
        .status()
        .context("Couldn't run winget (is App Installer installed?)")?;
    if !status.success() {
        bail!("winget exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_at(root: &'static str) -> impl Fn(&str) -> Option<PathBuf> {
        move |var| Some(PathBuf::from(root).join(var))
    }

    #[test]
    fn finds_retrobar_in_local_programs() {
        let want = PathBuf::from("R/LOCALAPPDATA/Programs/RetroBar/RetroBar.exe");
        let w = want.clone();
        let got = locate_with(
            Tool::RetroBar,
            &ToolsConfig::default(),
            &env_at("R"),
            &move |p: &Path| p == w,
        );
        assert_eq!(got, Some(want));
    }

    #[test]
    fn finds_openshell_in_program_files() {
        let want = PathBuf::from("R/ProgramFiles/Open-Shell/StartMenu.exe");
        let w = want.clone();
        let got = locate_with(
            Tool::OpenShell,
            &ToolsConfig::default(),
            &env_at("R"),
            &move |p: &Path| p == w,
        );
        assert_eq!(got, Some(want));
    }

    #[test]
    fn configured_path_wins_and_must_exist() {
        let cfg = ToolsConfig {
            retrobar: "D:/tools/RetroBar.exe".into(),
            ..Default::default()
        };
        let found = locate_with(Tool::RetroBar, &cfg, &env_at("R"), &|_: &Path| true);
        assert_eq!(found, Some(PathBuf::from("D:/tools/RetroBar.exe")));
        let missing = locate_with(Tool::RetroBar, &cfg, &env_at("R"), &|_: &Path| false);
        assert_eq!(missing, None);
    }

    #[test]
    fn finds_winget_portable_retrobar_link() {
        let want = PathBuf::from("R/LOCALAPPDATA/Microsoft/WinGet/Links/RetroBar.exe");
        let w = want.clone();
        let got = locate_with(
            Tool::RetroBar,
            &ToolsConfig::default(),
            &env_at("R"),
            &move |p: &Path| p == w,
        );
        assert_eq!(got, Some(want));
    }

    #[test]
    fn helper_lists_parse() {
        assert_eq!(
            parse_list("retrobar,openshell"),
            vec![Tool::RetroBar, Tool::OpenShell]
        );
        assert_eq!(parse_list(" Open-Shell "), vec![Tool::OpenShell]);
        assert_eq!(parse_list("nonsense,,"), vec![]);
    }

    #[test]
    fn only_enabled_pieces_need_their_tool() {
        let mut cfg = crate::config::AppConfig::default();
        cfg.tools.retrobar = r"Z:\definitely\not\here.exe".into();
        cfg.tools.openshell = r"Z:\definitely\not\here.exe".into();
        assert_eq!(missing(&cfg), vec![Tool::RetroBar, Tool::OpenShell]);
        cfg.era.start_menu.enabled = false;
        assert_eq!(missing(&cfg), vec![Tool::RetroBar]);
        cfg.era.taskbar.enabled = false;
        assert!(missing(&cfg).is_empty());
    }

    #[test]
    fn nothing_installed_is_none() {
        let got = locate_with(
            Tool::OpenShell,
            &ToolsConfig::default(),
            &env_at("R"),
            &|_: &Path| false,
        );
        assert_eq!(got, None);
    }
}
