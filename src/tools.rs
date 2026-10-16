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
// otherwise the first known location that exists
pub fn locate(tool: Tool, cfg: &ToolsConfig) -> Option<PathBuf> {
    locate_with(tool, cfg, &real_env, &|p: &Path| p.is_file())
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
