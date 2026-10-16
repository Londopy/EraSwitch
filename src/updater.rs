// updater.rs
// checks GitHub Releases for a newer version, downloads the portable zip,
// verifies its SHA-256 against the release checksums, swaps the running exe
// in place and restarts. installs only when the user clicks "Download & Install".
//
// all-users installs live in Program Files, which the app can't write to.
// those update through the matching installer instead (new .msi or setup.exe,
// one UAC prompt), then the app relaunches itself.
//
// Adapted from HideDesktopApps/src/updater.rs (itself grown from qbit-killswitch).
// Replace EraSwitch and EraSwitch.
//
// Cargo.toml:
//   reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "blocking"] }
//   serde = { version = "1", features = ["derive"] }
//   semver = { version = "1", features = ["serde"] }
//   sha2 = "0.10"
//   hex = "0.4"
//   zip = "2"
//   self-replace = "1"
//   anyhow = "1"
//   log = "0.4"

use anyhow::{bail, Context, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::Sender;

const GITHUB_API_LATEST: &str = "https://api.github.com/repos/Londopy/EraSwitch/releases/latest";
const GITHUB_API_RELEASES: &str = "https://api.github.com/repos/Londopy/EraSwitch/releases";
const APP_NAME: &str = "EraSwitch";

// [updater] section of config.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdaterConfig {
    pub enabled: bool,
    // "stable" or "beta" (beta also considers pre-releases)
    pub channel: String,
    // UI slider range is 1..=168
    pub check_interval_h: u32,
}

impl Default for UpdaterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            channel: "stable".into(),
            check_interval_h: 24,
        }
    }
}

// what the background thread reports back to the UI loop
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    Available(String),
    UpToDate,
    Failed(String),
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

// which build of the app is running
fn arch_suffix() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x64"
    }
    #[cfg(target_arch = "x86")]
    {
        "x86"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "x86", target_arch = "aarch64")))]
    {
        "x64"
    }
}

fn build_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("{}/{}", APP_NAME, env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Building HTTP client")
}

fn fetch_latest_release(channel: &str) -> Result<GithubRelease> {
    let client = build_client()?;
    if channel == "beta" {
        // beta: newest release of any kind, including pre-releases
        let releases: Vec<GithubRelease> = client
            .get(GITHUB_API_RELEASES)
            .send()
            .context("Fetching releases list")?
            .json()
            .context("Parsing releases list")?;
        releases
            .into_iter()
            .max_by(|a, b| {
                let va = Version::parse(a.tag_name.trim_start_matches('v')).ok();
                let vb = Version::parse(b.tag_name.trim_start_matches('v')).ok();
                va.cmp(&vb)
            })
            .ok_or_else(|| anyhow::anyhow!("No releases found"))
    } else {
        client
            .get(GITHUB_API_LATEST)
            .send()
            .context("Fetching latest release")?
            .json()
            .context("Parsing latest release")
    }
}

// Some(version) only when the remote release is strictly newer
pub fn check_for_update(channel: &str) -> Result<Option<String>> {
    let release = fetch_latest_release(channel)?;
    let remote_str = release.tag_name.trim_start_matches('v');
    let remote = Version::parse(remote_str)
        .with_context(|| format!("Parsing remote version '{remote_str}'"))?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).context("Parsing current version")?;
    Ok(if remote > current {
        Some(remote_str.to_string())
    } else {
        None
    })
}

// prefer "<App>-v1.2.3-x64-portable.zip", fall back to the combined
// "<App>-v1.2.3-portable.zip" that holds x64/, x86/ and arm64/ folders
fn select_update_zip<'a>(names: &'a [String], suffix: &str) -> Option<&'a String> {
    let arch_zip = format!("-{suffix}-portable");
    if let Some(n) = names
        .iter()
        .find(|n| n.ends_with(".zip") && n.contains(&arch_zip))
    {
        return Some(n);
    }
    names.iter().find(|n| is_combined_zip(n))
}

fn is_combined_zip(name: &str) -> bool {
    name.ends_with("-portable.zip")
        && !["x64", "x86", "arm64"]
            .iter()
            .any(|s| name.contains(&format!("-{s}-portable")))
}

// the exe under "<arch>/" in a combined zip, otherwise the first exe
fn pick_arch_exe_index(entries: &[String], suffix: &str) -> Option<usize> {
    let arch_dir = format!("/{suffix}/");
    let arch_prefix = format!("{suffix}/");
    let mut any_exe = None;
    for (i, raw) in entries.iter().enumerate() {
        let name = raw.replace('\\', "/");
        if name.ends_with(".exe") {
            if any_exe.is_none() {
                any_exe = Some(i);
            }
            if name.contains(&arch_dir) || name.starts_with(&arch_prefix) {
                return Some(i);
            }
        }
    }
    any_exe
}

// sha256sum format: "<hex>  <name>" (a leading '*' marks binary mode)
fn hash_from_sums(sums: &str, filename: &str) -> Option<String> {
    for line in sums.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next().unwrap_or("").trim_start_matches('*');
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        if base == filename {
            return Some(hash.to_lowercase());
        }
    }
    None
}

// "<zip>.sha256" sidecar first, then SHA256SUMS.txt
fn resolve_expected_hash(
    client: &reqwest::blocking::Client,
    assets: &[GithubAsset],
    zip_name: &str,
) -> Option<String> {
    let sidecar = format!("{zip_name}.sha256");
    if let Some(a) = assets.iter().find(|a| a.name == sidecar) {
        if let Ok(t) = client
            .get(&a.browser_download_url)
            .send()
            .and_then(|r| r.text())
        {
            if let Some(h) = t.split_whitespace().next() {
                return Some(h.to_lowercase());
            }
        }
    }
    if let Some(a) = assets.iter().find(|a| a.name == "SHA256SUMS.txt") {
        if let Ok(t) = client
            .get(&a.browser_download_url)
            .send()
            .and_then(|r| r.text())
        {
            return hash_from_sums(&t, zip_name);
        }
    }
    None
}

// how this copy gets updated
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateStrategy {
    // folder is writable (portable, per-user setup.exe/msi, scoop): swap the exe
    SelfReplace,
    // all-users msi in Program Files: run the new msi
    Msi,
    // all-users setup.exe in Program Files: run the new setup
    InnoSetup,
    // read-only folder we didn't install into
    Manual,
}

fn choose_strategy(dir_writable: bool, msi_marker: bool, inno_uninstaller: bool) -> UpdateStrategy {
    if dir_writable {
        UpdateStrategy::SelfReplace
    } else if msi_marker {
        UpdateStrategy::Msi
    } else if inno_uninstaller {
        UpdateStrategy::InnoSetup
    } else {
        UpdateStrategy::Manual
    }
}

// actually tries a write; permission bits lie on windows
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// the msi writes install.ini ([install] source=msi) next to the exe
fn ini_says_msi(ini: &str) -> bool {
    ini.lines()
        .map(|l| l.trim().to_ascii_lowercase().replace(' ', ""))
        .any(|l| l == "source=msi")
}

// "<slug>-1.2.3-x64.msi" or "<slug>-1.2.3-setup-x64.exe"
fn select_package<'a>(names: &'a [String], suffix: &str, msi: bool) -> Option<&'a String> {
    let tail = if msi {
        format!("-{suffix}.msi")
    } else {
        format!("-setup-{suffix}.exe")
    };
    names.iter().find(|n| n.ends_with(&tail))
}

// cmd line that runs the installer, waits, then relaunches the app (even if
// the install was cancelled, so the user is never left with nothing running)
fn installer_command(package: &Path, exe: &Path, msi: bool) -> String {
    let pkg = package.display();
    let exe = exe.display();
    let install = if msi {
        format!("msiexec.exe /i \"{pkg}\" ALLUSERS=2 MSIINSTALLPERUSER=\"\" /passive /norestart")
    } else {
        format!("\"{pkg}\" /SILENT /SP- /ALLUSERS /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS")
    };
    format!("start \"\" /wait {install} & start \"\" \"{exe}\" --restarted")
}

// downloads an asset and checks it against <name>.sha256 / SHA256SUMS.txt.
// require_hash: refuse to continue when no checksum is published (used for
// installers, which run elevated)
fn download_verified(
    client: &reqwest::blocking::Client,
    assets: &[GithubAsset],
    name: &str,
    require_hash: bool,
) -> Result<Vec<u8>> {
    let asset = assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| anyhow::anyhow!("Asset {name} disappeared from the list"))?;
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .with_context(|| format!("Downloading {name}"))?
        .bytes()
        .with_context(|| format!("Reading {name}"))?;
    match resolve_expected_hash(client, assets, name) {
        Some(expected) => {
            let actual = hex::encode(Sha256::digest(&bytes));
            if actual != expected {
                bail!("SHA-256 mismatch! expected={expected}, got={actual}");
            }
        }
        None if require_hash => bail!("No SHA-256 published for {name}; not installing it"),
        None => log::warn!("updater: no checksum asset found, skipping verification"),
    }
    Ok(bytes.to_vec())
}

// downloads, verifies, installs, starts the new version and exits.
// only returns if something failed.
pub fn download_and_apply(channel: &str) -> Result<()> {
    let release = fetch_latest_release(channel)?;
    let client = build_client()?;
    let exe = std::env::current_exe().context("Locating the running exe")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("The exe has no parent folder"))?;
    let msi_marker = std::fs::read_to_string(dir.join("install.ini"))
        .map(|t| ini_says_msi(&t))
        .unwrap_or(false);
    let strategy = choose_strategy(
        dir_is_writable(dir),
        msi_marker,
        dir.join("unins000.exe").is_file(),
    );
    match strategy {
        UpdateStrategy::SelfReplace => apply_portable_zip(&client, &release),
        UpdateStrategy::Msi => run_installer(&client, &release, &exe, true),
        UpdateStrategy::InnoSetup => run_installer(&client, &release, &exe, false),
        UpdateStrategy::Manual => bail!(
            "{} is read-only, so {APP_NAME} can't update itself here. \
             Download {} from https://github.com/Londopy/EraSwitch/releases",
            dir.display(),
            release.tag_name
        ),
    }
}

fn apply_portable_zip(client: &reqwest::blocking::Client, release: &GithubRelease) -> Result<()> {
    let suffix = arch_suffix();
    let names: Vec<String> = release.assets.iter().map(|a| a.name.clone()).collect();
    let zip_name = select_update_zip(&names, suffix)
        .ok_or_else(|| anyhow::anyhow!("No zip asset found for arch '{suffix}'"))?;
    let bytes = download_verified(client, &release.assets, zip_name, false)?;

    let tmp = std::env::temp_dir();
    let zip_path = tmp.join(format!("{APP_NAME}-update.zip"));
    let new_exe = tmp.join(format!("{APP_NAME}-new.exe"));
    std::fs::File::create(&zip_path)?.write_all(&bytes)?;
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&zip_path)?).context("Parsing update zip")?;
    let entries: Vec<String> = (0..archive.len())
        // zip 9: name() is a Result too
        .map(|i| {
            archive
                .by_index(i)
                .and_then(|f| f.name().map(|n| n.into_owned()))
        })
        .collect::<std::result::Result<_, _>>()?;
    let idx = pick_arch_exe_index(&entries, suffix)
        .ok_or_else(|| anyhow::anyhow!("No .exe found inside update zip"))?;
    {
        let mut f = archive.by_index(idx)?;
        std::io::copy(&mut f, &mut std::fs::File::create(&new_exe)?)?;
    }

    self_replace::self_replace(&new_exe).context("self_replace failed")?;
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_file(&new_exe);
    // --restarted: the new process waits for this one to drop the mutex
    std::process::Command::new(std::env::current_exe()?)
        .arg("--restarted")
        .spawn()
        .context("Spawning new process")?;
    std::process::exit(0);
}

fn run_installer(
    client: &reqwest::blocking::Client,
    release: &GithubRelease,
    exe: &Path,
    msi: bool,
) -> Result<()> {
    let suffix = arch_suffix();
    let names: Vec<String> = release.assets.iter().map(|a| a.name.clone()).collect();
    let kind = if msi { "msi" } else { "setup.exe" };
    let name = select_package(&names, suffix, msi)
        .ok_or_else(|| anyhow::anyhow!("No {kind} for '{suffix}' in this release"))?;
    let bytes = download_verified(client, &release.assets, name, true)?;
    let ext = if msi { "msi" } else { "exe" };
    let package = std::env::temp_dir().join(format!("{APP_NAME}-update.{ext}"));
    std::fs::File::create(&package)?.write_all(&bytes)?;
    spawn_hidden_cmd(&installer_command(&package, exe, msi))?;
    // the installer closes and replaces us; exit so no file is locked
    std::process::exit(0);
}

#[cfg(windows)]
fn spawn_hidden_cmd(line: &str) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // /s /c "..." makes cmd strip exactly the outer quotes
    std::process::Command::new("cmd.exe")
        .raw_arg(format!("/d /s /c \"{line}\""))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("Starting the installer")?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_hidden_cmd(_line: &str) -> Result<()> {
    bail!("Installer updates are Windows-only")
}

// runs a check off the UI thread. user_triggered = true also reports
// "up to date" and errors, so the Check Now button always gets an answer.
pub fn background_check(cfg: UpdaterConfig, tx: Sender<UpdateEvent>, user_triggered: bool) {
    std::thread::spawn(move || {
        if !cfg.enabled && !user_triggered {
            return;
        }
        match check_for_update(&cfg.channel) {
            Ok(Some(v)) => {
                let _ = tx.send(UpdateEvent::Available(v));
            }
            Ok(None) => {
                if user_triggered {
                    let _ = tx.send(UpdateEvent::UpToDate);
                }
            }
            Err(e) => {
                log::warn!("Update check failed: {e:#}");
                if user_triggered {
                    let _ = tx.send(UpdateEvent::Failed(format!("{e:#}")));
                }
            }
        }
    });
}

// "Download & Install Update" button. on success the process restarts,
// so only failures come back.
pub fn background_apply(channel: String, tx: Sender<UpdateEvent>) {
    std::thread::spawn(move || {
        if let Err(e) = download_and_apply(&channel) {
            log::warn!("Update failed: {e:#}");
            let _ = tx.send(UpdateEvent::Failed(format!("{e:#}")));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // mirror of installer.iss VersionInfoVersion derivation, kept here so CI
    // regression-tests the "strip prerelease suffix" rule
    fn numeric_version(v: &str) -> &str {
        v.split('-').next().unwrap_or(v)
    }

    #[test]
    fn numeric_version_strips_prerelease() {
        assert_eq!(numeric_version("1.1.4-rc1"), "1.1.4");
        assert_eq!(numeric_version("2.3.4-beta.2"), "2.3.4");
        assert_eq!(numeric_version("1.1.4"), "1.1.4");
    }

    #[test]
    fn select_zip_prefers_arch_specific_then_combined() {
        let assets = names(&[
            "App-v1.2.0-x64-setup.exe",
            "App-v1.2.0-portable.zip",
            "App-v1.2.0-x64-portable.zip",
            "SHA256SUMS.txt",
        ]);
        assert_eq!(
            select_update_zip(&assets, "x64").map(String::as_str),
            Some("App-v1.2.0-x64-portable.zip")
        );
        assert_eq!(
            select_update_zip(&assets, "arm64").map(String::as_str),
            Some("App-v1.2.0-portable.zip")
        );
    }

    #[test]
    fn pick_exe_from_combined_subfolders() {
        let entries = names(&["App/x64/App.exe", "App/x86/App.exe", "App/arm64/App.exe"]);
        assert_eq!(pick_arch_exe_index(&entries, "x86"), Some(1));
        assert_eq!(pick_arch_exe_index(&entries, "arm64"), Some(2));
    }

    #[test]
    fn pick_exe_flat_zip_and_backslashes() {
        assert_eq!(pick_arch_exe_index(&names(&["App.exe"]), "x86"), Some(0));
        assert_eq!(
            pick_arch_exe_index(&names(&["App\\arm64\\App.exe"]), "arm64"),
            Some(0)
        );
    }

    #[test]
    fn hash_from_sums_handles_star_and_blank_lines() {
        let sums = "\nABCDEF  App-v1.0.0-portable.zip\n123456 *other.zip\n";
        assert_eq!(
            hash_from_sums(sums, "App-v1.0.0-portable.zip").as_deref(),
            Some("abcdef")
        );
        assert_eq!(hash_from_sums(sums, "other.zip").as_deref(), Some("123456"));
        assert_eq!(hash_from_sums(sums, "missing.zip"), None);
    }

    #[test]
    fn strategy_prefers_self_replace_when_writable() {
        use UpdateStrategy::*;
        assert_eq!(choose_strategy(true, true, true), SelfReplace);
        assert_eq!(choose_strategy(false, true, false), Msi);
        assert_eq!(choose_strategy(false, false, true), InnoSetup);
        assert_eq!(choose_strategy(false, false, false), Manual);
    }

    #[test]
    fn ini_marker_parsing() {
        assert!(ini_says_msi("[install]\r\nsource=msi\r\n"));
        assert!(ini_says_msi("[install]\nSource = MSI\n"));
        assert!(!ini_says_msi("[install]\nsource=inno\n"));
        assert!(!ini_says_msi(""));
    }

    #[test]
    fn select_package_by_kind_and_arch() {
        let assets = names(&[
            "app-1.2.0-setup-x64.exe",
            "app-1.2.0-x64.msi",
            "app-1.2.0-arm64.msi",
            "App-v1.2.0-x64-portable.exe",
        ]);
        assert_eq!(
            select_package(&assets, "x64", true).map(String::as_str),
            Some("app-1.2.0-x64.msi")
        );
        assert_eq!(
            select_package(&assets, "arm64", true).map(String::as_str),
            Some("app-1.2.0-arm64.msi")
        );
        assert_eq!(
            select_package(&assets, "x64", false).map(String::as_str),
            Some("app-1.2.0-setup-x64.exe")
        );
        assert_eq!(select_package(&assets, "arm64", false), None);
    }

    #[test]
    fn installer_command_waits_then_relaunches() {
        let pkg = Path::new(r"C:\Users\x\AppData\Local\Temp\App-update.msi");
        let exe = Path::new(r"C:\Program Files\App\App.exe");
        let msi = installer_command(pkg, exe, true);
        assert!(msi.starts_with("start \"\" /wait msiexec.exe /i \""));
        assert!(msi.contains("ALLUSERS=2 MSIINSTALLPERUSER=\"\""));
        assert!(msi.ends_with("& start \"\" \"C:\\Program Files\\App\\App.exe\" --restarted"));
        let inno = installer_command(pkg, exe, false);
        assert!(inno.contains("/ALLUSERS"));
        assert!(inno.contains("/SILENT"));
    }

    #[test]
    fn writable_probe_cleans_up() {
        let dir = std::env::temp_dir();
        assert!(dir_is_writable(&dir));
        let leftover = dir.join(format!(".write-test-{}", std::process::id()));
        assert!(!leftover.exists());
    }

    #[test]
    fn default_config_matches_convention() {
        let c = UpdaterConfig::default();
        assert!(c.enabled);
        assert_eq!(c.channel, "stable");
        assert_eq!(c.check_interval_h, 24);
    }
}
