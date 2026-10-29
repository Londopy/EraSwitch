// londopy.rs
// .londopy files: the file format every Londopy app shares, like osu!'s .osz.
// a .londopy is a zip with a londopy.toml manifest that says which app it belongs
// to. any installed Londopy app can be the one Windows opens .londopy files
// with; it reads the manifest and hands the file to the right app, so the
// double-click always lands in the right place.
//
// format and registry layout: londopy-files.md in the londopy-style skill.
// Replace eraswitch (lowercase repo slug, e.g. "clipvault"), EraSwitch and EraSwitch.
//
// Cargo.toml:
//   serde = { version = "1", features = ["derive"] }
//   toml = "0.8"
//   zip = "9"
//   anyhow = "1"
//   [target.'cfg(windows)'.dependencies]
//   winreg = "0.55"
//   windows = { version = "0.58", features = ["Win32_UI_Shell"] }

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "eraswitch";
pub const APP_NAME: &str = "EraSwitch";
pub const HOMEPAGE: &str = "https://github.com/Londopy/EraSwitch";

pub const EXTENSION: &str = "londopy";
pub const MANIFEST_NAME: &str = "londopy.toml";
// newest container version this code understands
pub const FORMAT_VERSION: u32 = 1;
// a manifest is a few hundred bytes; anything huge is not a real .londopy
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

// londopy.toml
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    // container version (FORMAT_VERSION when written)
    pub format: u32,
    // app id that opens this file: lowercase repo slug, e.g. "clipvault"
    pub app: String,
    // what's inside; each app defines its own kinds ("profile", "theme", ...)
    pub kind: String,
    #[serde(default)]
    pub title: String,
    // version of the app that wrote it
    #[serde(default)]
    pub app_version: String,
    // RFC 3339 UTC, e.g. "2026-10-07T22:15:00Z"
    #[serde(default)]
    pub created: String,
    // where to get the app, shown when it isn't installed
    #[serde(default)]
    pub homepage: String,
}

impl Manifest {
    // a manifest for a file this app writes
    pub fn new(kind: &str, title: &str, created: &str) -> Self {
        Self {
            format: FORMAT_VERSION,
            app: APP_ID.into(),
            kind: kind.into(),
            title: title.into(),
            app_version: env!("CARGO_PKG_VERSION").into(),
            created: created.into(),
            homepage: HOMEPAGE.into(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.format == 0 {
            bail!("londopy.toml has no valid format number");
        }
        if self.format > FORMAT_VERSION {
            bail!(
                "this .londopy file uses format {} but this app understands up to {}; update the app",
                self.format,
                FORMAT_VERSION
            );
        }
        if !valid_app_id(&self.app) {
            bail!("londopy.toml has an invalid app id '{}'", self.app);
        }
        if self.kind.trim().is_empty() {
            bail!("londopy.toml has no kind");
        }
        Ok(())
    }
}

// lowercase letters, digits and hyphens, like a repo slug
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// entry names stay relative and inside the archive: no "..", no drive letters,
// no leading slash, and londopy.toml is reserved
fn safe_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name != MANIFEST_NAME
        && name != LEGACY_MANIFEST_NAME
        && !name.starts_with('/')
        && !name.starts_with('\\')
        && !name.contains(':')
        && name
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub fn is_londopy_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

// writes a .londopy: manifest first and uncompressed (cheap to sniff), then
// the payload entries, compressed. goes to a temp file first so a crash
// never leaves half a file behind.
pub fn write(path: &Path, manifest: &Manifest, entries: &[(&str, &[u8])]) -> Result<()> {
    manifest.validate()?;
    for (name, _) in entries {
        if !safe_entry_name(name) {
            bail!("bad entry name '{name}' for a .londopy file");
        }
    }
    let tmp = path.with_extension("londopy.tmp");
    {
        let file =
            std::fs::File::create(&tmp).with_context(|| format!("Creating {}", tmp.display()))?;
        let mut zip = zip::ZipWriter::new(file);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(MANIFEST_NAME, stored)?;
        zip.write_all(toml::to_string(manifest)?.as_bytes())?;
        for (name, data) in entries {
            zip.start_file(name.replace('\\', "/"), deflated)?;
            zip.write_all(data)?;
        }
        zip.finish()?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("Saving {}", path.display()))?;
    Ok(())
}

// an opened .londopy; read entries on demand with a size cap
pub struct LondopyFile {
    pub manifest: Manifest,
    archive: zip::ZipArchive<std::fs::File>,
}

impl LondopyFile {
    pub fn open(path: &Path) -> Result<Self> {
        let file =
            std::fs::File::open(path).with_context(|| format!("Opening {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("{} is not a .londopy file (not a zip)", path.display()))?;
        // legacy: a .londo from before the rename carries londo.toml instead.
        // only fall back when londopy.toml is absent, so a londopy.toml that's
        // there but bad (too big, say) reports its own error
        let has_current = archive.index_for_name(MANIFEST_NAME).is_some();
        let name = if has_current || archive.index_for_name(LEGACY_MANIFEST_NAME).is_none() {
            MANIFEST_NAME
        } else {
            LEGACY_MANIFEST_NAME
        };
        let raw = read_capped(&mut archive, name, MAX_MANIFEST_BYTES)
            .with_context(|| format!("{} has no readable {name}", path.display()))?;
        let text = String::from_utf8(raw).context("londopy.toml is not UTF-8")?;
        let manifest: Manifest = toml::from_str(&text).context("Parsing londopy.toml")?;
        manifest.validate()?;
        Ok(Self { manifest, archive })
    }

    // payload entry names (londopy.toml and folders left out)
    pub fn entry_names(&self) -> Vec<String> {
        self.archive
            .file_names()
            // zip 9 hands back a Result per name; a name it can't decode is skipped
            .filter_map(|n| n.ok().map(|n| n.into_owned()))
            .filter(|n| n != MANIFEST_NAME && n != LEGACY_MANIFEST_NAME && !n.ends_with('/'))
            .collect()
    }

    // reads one entry, refusing anything bigger than max_bytes (zip bombs)
    pub fn read_entry(&mut self, name: &str, max_bytes: u64) -> Result<Vec<u8>> {
        if !safe_entry_name(name) {
            bail!("bad entry name '{name}'");
        }
        read_capped(&mut self.archive, name, max_bytes)
    }
}

fn read_capped<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    let entry = archive
        .by_name(name)
        .with_context(|| format!("No '{name}' inside"))?;
    if entry.size() > max_bytes {
        bail!(
            "'{name}' is {} bytes, over the {max_bytes} byte limit",
            entry.size()
        );
    }
    // the header size can lie; cap the actual read too
    let mut out = Vec::new();
    entry.take(max_bytes + 1).read_to_end(&mut out)?;
    if out.len() as u64 > max_bytes {
        bail!("'{name}' is over the {max_bytes} byte limit");
    }
    Ok(out)
}

// ---- opening .londopy files from Explorer -------------------------------------

// what main() should do with the command line
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LondopyArg {
    // `--londopy-open <file>` from Explorer, or a .londopy dropped on the exe
    Open(PathBuf),
    // `--londopy-unregister`, run by the uninstaller
    Unregister,
}

// call first thing in main(), before the single-instance check
pub fn parse_args(args: &[String]) -> Option<LondopyArg> {
    match args.get(1).map(String::as_str) {
        // legacy spellings: an installed 0.1.x uninstaller and older .londo
        // associations still call these
        Some("--londopy-unregister" | "--londo-unregister") => Some(LondopyArg::Unregister),
        Some("--londopy-open" | "--londo-open") => {
            args.get(2).map(|p| LondopyArg::Open(PathBuf::from(p)))
        }
        Some(p) if is_londopy_path(Path::new(p)) || is_legacy_path(Path::new(p)) => {
            Some(LondopyArg::Open(PathBuf::from(p)))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    // ours: open it (or pass it to the running copy with send_to_inbox)
    Mine(Manifest),
    // handed to the app it belongs to
    Forwarded { app: String },
    // belongs to an app that isn't installed; offer the homepage
    NotInstalled { app: String, homepage: String },
}

// reads the manifest and routes the file
pub fn dispatch(path: &Path) -> Result<Dispatch> {
    let manifest = LondopyFile::open(path)?.manifest;
    if manifest.app == APP_ID {
        return Ok(Dispatch::Mine(manifest));
    }
    if let Some(exe) = registry::app_exe(&manifest.app).filter(|e| e.is_file()) {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        std::process::Command::new(&exe)
            .arg("--londopy-open")
            .arg(abs)
            .spawn()
            .with_context(|| format!("Starting {}", exe.display()))?;
        return Ok(Dispatch::Forwarded { app: manifest.app });
    }
    // legacy: an app from before the rename (registered under the old key)
    // still opens its own .londo files, with the old flag
    if let Some(exe) = legacy::app_exe(&legacy::Keys::real(), &manifest.app).filter(|e| e.is_file())
    {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        std::process::Command::new(&exe)
            .arg("--londo-open")
            .arg(abs)
            .spawn()
            .with_context(|| format!("Starting {}", exe.display()))?;
        return Ok(Dispatch::Forwarded { app: manifest.app });
    }
    let homepage = if manifest.homepage.starts_with("https://") {
        manifest.homepage
    } else {
        String::new()
    };
    Ok(Dispatch::NotInstalled {
        app: manifest.app,
        homepage,
    })
}

// ---- single instance: pass a file to the copy that's already running --------

// drops the path into <data_dir>/inbox; the running copy picks it up with
// drain_inbox() on its next loop tick
pub fn send_to_inbox(data_dir: &Path, file: &Path) -> Result<()> {
    let inbox = data_dir.join("inbox");
    std::fs::create_dir_all(&inbox)?;
    // absolute, not canonicalize(): that adds a \\?\ prefix on Windows
    let abs = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{stamp:032}-{}", std::process::id());
    let tmp = inbox.join(format!("{name}.tmp"));
    std::fs::write(&tmp, abs.to_string_lossy().as_bytes())?;
    std::fs::rename(&tmp, inbox.join(format!("{name}.path")))?;
    Ok(())
}

// oldest first; each entry is removed once read
pub fn drain_inbox(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(data_dir.join("inbox")) else {
        return Vec::new();
    };
    let mut items: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "path"))
        .collect();
    items.sort();
    items
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok();
            let _ = std::fs::remove_file(&p);
            text.map(|t| PathBuf::from(t.trim()))
        })
        .collect()
}

// ---- file association (HKCU, no admin) --------------------------------------

// "C:\Apps\Foo.exe" --londopy-open "%1"  ->  C:\Apps\Foo.exe
fn parse_command_exe(command: &str) -> Option<PathBuf> {
    let c = command.trim();
    if let Some(rest) = c.strip_prefix('"') {
        rest.split('"').next().map(PathBuf::from)
    } else {
        c.split_whitespace().next().map(PathBuf::from)
    }
}

// who takes over the double-click when its owner goes away: the first other
// registered app (sorted by id) whose exe still exists
fn pick_new_owner(candidates: &[(String, PathBuf)], leaving: &str) -> Option<PathBuf> {
    let mut c: Vec<&(String, PathBuf)> =
        candidates.iter().filter(|(id, _)| id != leaving).collect();
    c.sort_by(|a, b| a.0.cmp(&b.0));
    c.into_iter()
        .find(|(_, exe)| exe.is_file())
        .map(|(_, exe)| exe.clone())
}

// call at startup in installed (non-portable) mode: records where this app
// lives and claims .londopy if no live app owns it. never takes .londopy from
// another app that still exists; that app forwards our files anyway.
pub fn register_self() -> Result<()> {
    let exe = std::env::current_exe()?;
    registry::set_app(APP_ID, APP_NAME, HOMEPAGE, &exe)?;
    let owner_alive = registry::handler_command()
        .and_then(|c| parse_command_exe(&c))
        .is_some_and(|p| p.is_file());
    if !owner_alive {
        registry::set_handler(&exe)?;
    }
    Ok(())
}

// run by the uninstaller (--londopy-unregister): forget this app and pass the
// double-click to another installed Londopy app, or remove it if none is left
pub fn unregister_self() -> Result<()> {
    let exe = std::env::current_exe()?;
    let current = (|| -> Result<()> {
        registry::remove_app(APP_ID)?;
        let owner = registry::handler_command().and_then(|c| parse_command_exe(&c));
        let we_own_it = owner.as_ref().is_some_and(|o| same_path(o, &exe));
        if we_own_it {
            match pick_new_owner(&registry::apps(), APP_ID) {
                Some(next) => registry::set_handler(&next)?,
                None => registry::remove_handler()?,
            }
        }
        Ok(())
    })();
    // the old .londo registration goes too, even if the step above failed
    let old = legacy::unregister_at(&legacy::Keys::real(), &exe);
    current.and(old)
}

// ---- legacy .londo (EraSwitch-specific, not in the template) ----------------
// before .londopy, Londopy apps shared `.londo`: a `londo.toml` manifest,
// `--londo-open` / `--londo-unregister`, HKCU\Software\Londopy\Londo\Apps and
// the Londopy.Londo ProgID. EraSwitch 0.1.x saved eras that way and registered
// there, and other apps (reelsmith) may still own `.londo`, so old files keep
// opening and only EraSwitch's own old registration is ever removed.

pub const LEGACY_EXTENSION: &str = "londo";
const LEGACY_MANIFEST_NAME: &str = "londo.toml";

pub fn is_legacy_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(LEGACY_EXTENSION))
}

#[cfg(windows)]
mod legacy {
    use anyhow::Result;
    use std::path::{Path, PathBuf};
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const PROG_ID: &str = "Londopy.Londo";

    // where 0.1.x-era apps registered, under HKCU (tests point it elsewhere)
    pub struct Keys {
        pub apps: String,
        pub classes: String,
    }

    impl Keys {
        pub fn real() -> Self {
            Keys {
                apps: r"Software\Londopy\Londo\Apps".into(),
                classes: r"Software\Classes".into(),
            }
        }
    }

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    // the exe a 0.1.x-era app registered, to forward its .londo files to
    pub fn app_exe(keys: &Keys, id: &str) -> Option<PathBuf> {
        let e: String = hkcu()
            .open_subkey(format!(r"{}\{id}", keys.apps))
            .ok()?
            .get_value("Exe")
            .ok()?;
        Some(PathBuf::from(e))
    }

    fn apps(keys: &Keys) -> Vec<(String, PathBuf)> {
        let Ok(apps) = hkcu().open_subkey(&keys.apps) else {
            return Vec::new();
        };
        apps.enum_keys()
            .flatten()
            .filter_map(|id| {
                let e: String = apps.open_subkey(&id).ok()?.get_value("Exe").ok()?;
                Some((id, PathBuf::from(e)))
            })
            .collect()
    }

    // forget 0.1.x's registration; if `.londo` double-clicks still go to this
    // exe, hand them to another app registered there, or drop the association.
    // `.londo` itself is only removed while it still names our old ProgID.
    pub fn unregister_at(keys: &Keys, exe: &Path) -> Result<()> {
        let hkcu = hkcu();
        let _ = hkcu.delete_subkey_all(format!(r"{}\{}", keys.apps, super::APP_ID));
        let prog = format!(r"{}\{PROG_ID}", keys.classes);
        let owner = hkcu
            .open_subkey(format!(r"{prog}\shell\open\command"))
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
            .and_then(|c| super::parse_command_exe(&c));
        if !owner.is_some_and(|o| super::same_path(&o, exe)) {
            return Ok(());
        }
        match super::pick_new_owner(&apps(keys), super::APP_ID) {
            Some(next) => {
                let (p, _) = hkcu.create_subkey(&prog)?;
                let (icon, _) = p.create_subkey("DefaultIcon")?;
                icon.set_value("", &format!("\"{}\",0", next.display()))?;
                let (cmd, _) = p.create_subkey(r"shell\open\command")?;
                cmd.set_value("", &format!("\"{}\" --londo-open \"%1\"", next.display()))?;
            }
            None => {
                let _ = hkcu.delete_subkey_all(&prog);
                let ext = format!(r"{}\.londo", keys.classes);
                let ours = hkcu
                    .open_subkey(&ext)
                    .and_then(|k| k.get_value::<String, _>(""))
                    .is_ok_and(|v| v == PROG_ID);
                if ours {
                    let _ = hkcu.delete_subkey_all(&ext);
                }
            }
        }
        super::registry::notify_shell();
        Ok(())
    }
}

#[cfg(not(windows))]
mod legacy {
    pub struct Keys;
    impl Keys {
        pub fn real() -> Self {
            Keys
        }
    }
    pub fn app_exe(_: &Keys, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    pub fn unregister_at(_: &Keys, _: &std::path::Path) -> anyhow::Result<()> {
        Ok(())
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(windows)]
mod registry {
    use anyhow::Result;
    use std::path::{Path, PathBuf};
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    const APPS_KEY: &str = r"Software\Londopy\Londopy\Apps";
    const CLASSES: &str = r"Software\Classes";
    const PROG_ID: &str = "Londopy.Londopy";

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    pub fn set_app(id: &str, name: &str, homepage: &str, exe: &Path) -> Result<()> {
        let (key, _) = hkcu().create_subkey(format!(r"{APPS_KEY}\{id}"))?;
        key.set_value("Exe", &exe.to_string_lossy().to_string())?;
        key.set_value("Name", &name.to_string())?;
        key.set_value("Homepage", &homepage.to_string())?;
        Ok(())
    }

    pub fn remove_app(id: &str) -> Result<()> {
        match hkcu().delete_subkey_all(format!(r"{APPS_KEY}\{id}")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn app_exe(id: &str) -> Option<PathBuf> {
        let key = hkcu()
            .open_subkey_with_flags(format!(r"{APPS_KEY}\{id}"), KEY_READ)
            .ok()?;
        key.get_value::<String, _>("Exe").ok().map(PathBuf::from)
    }

    pub fn apps() -> Vec<(String, PathBuf)> {
        let Ok(root) = hkcu().open_subkey_with_flags(APPS_KEY, KEY_READ) else {
            return Vec::new();
        };
        root.enum_keys()
            .filter_map(|k| k.ok())
            .filter_map(|id| app_exe(&id).map(|exe| (id, exe)))
            .collect()
    }

    pub fn handler_command() -> Option<String> {
        hkcu()
            .open_subkey_with_flags(format!(r"{CLASSES}\{PROG_ID}\shell\open\command"), KEY_READ)
            .ok()?
            .get_value::<String, _>("")
            .ok()
    }

    // .londopy -> Londopy.Londopy -> "<exe>" --londopy-open "%1"
    pub fn set_handler(exe: &Path) -> Result<()> {
        let exe = exe.to_string_lossy();
        let (ext, _) = hkcu().create_subkey(format!(r"{CLASSES}\.londopy"))?;
        ext.set_value("", &PROG_ID.to_string())?;
        ext.set_value("Content Type", &"application/x-londopy".to_string())?;
        let (owp, _) = ext.create_subkey("OpenWithProgids")?;
        owp.set_value(PROG_ID, &String::new())?;
        let (prog, _) = hkcu().create_subkey(format!(r"{CLASSES}\{PROG_ID}"))?;
        prog.set_value("", &"Londopy file".to_string())?;
        let (icon, _) = prog.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{exe}\",0"))?;
        let (cmd, _) = prog.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &format!("\"{exe}\" --londopy-open \"%1\""))?;
        notify_shell();
        Ok(())
    }

    pub fn remove_handler() -> Result<()> {
        let _ = hkcu().delete_subkey_all(format!(r"{CLASSES}\{PROG_ID}"));
        // only drop .londopy itself if it still points at our ProgID
        if let Ok(ext) = hkcu().open_subkey_with_flags(format!(r"{CLASSES}\.londopy"), KEY_READ) {
            if ext.get_value::<String, _>("").ok().as_deref() == Some(PROG_ID) {
                let _ = hkcu().delete_subkey_all(format!(r"{CLASSES}\.londopy"));
            }
        }
        notify_shell();
        Ok(())
    }

    // tells Explorer to refresh icons and the "Open" verb right away
    // (pub(super): EraSwitch's legacy .londo cleanup uses it too)
    pub(super) fn notify_shell() {
        use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        // SAFETY: SHCNE_ASSOCCHANGED takes no item pointers
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }
}

// other platforms: no association; files still open when passed on the command line
#[cfg(not(windows))]
mod registry {
    use anyhow::Result;
    use std::path::{Path, PathBuf};

    pub fn set_app(_: &str, _: &str, _: &str, _: &Path) -> Result<()> {
        Ok(())
    }
    pub fn remove_app(_: &str) -> Result<()> {
        Ok(())
    }
    pub fn app_exe(_: &str) -> Option<PathBuf> {
        None
    }
    pub fn apps() -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    pub fn handler_command() -> Option<String> {
        None
    }
    pub fn set_handler(_: &Path) -> Result<()> {
        Ok(())
    }
    pub fn remove_handler() -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("londopy-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> Manifest {
        Manifest::new("profile", "Work setup", "2026-10-07T22:15:00Z")
    }

    #[test]
    fn write_then_read_roundtrip() {
        let d = tmpdir("rt");
        let p = d.join("work.londopy");
        write(
            &p,
            &sample(),
            &[("profile.toml", b"x = 1\n"), ("img/icon.png", &[1, 2, 3])],
        )
        .unwrap();
        let mut f = LondopyFile::open(&p).unwrap();
        assert_eq!(f.manifest, sample());
        let mut names = f.entry_names();
        names.sort();
        assert_eq!(names, vec!["img/icon.png", "profile.toml"]);
        assert_eq!(f.read_entry("profile.toml", 1024).unwrap(), b"x = 1\n");
        assert!(!d.join("work.londopy.tmp").exists());
    }

    #[test]
    fn manifest_is_first_and_uncompressed() {
        let d = tmpdir("first");
        let p = d.join("a.londopy");
        write(&p, &sample(), &[("data.bin", &[0u8; 4096])]).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&p).unwrap()).unwrap();
        let first = z.by_index(0).unwrap();
        assert_eq!(first.name().unwrap(), MANIFEST_NAME);
        assert_eq!(first.compression(), zip::CompressionMethod::Stored);
    }

    #[test]
    fn rejects_unsafe_entry_names() {
        for bad in [
            "../evil",
            "/abs",
            "C:\\x",
            "a/../b",
            "londopy.toml",
            "",
            "a//b",
        ] {
            assert!(!safe_entry_name(bad), "{bad} should be rejected");
        }
        for good in ["profile.toml", "img/icon.png", "a\\b.txt"] {
            assert!(safe_entry_name(good), "{good} should be allowed");
        }
        let d = tmpdir("unsafe");
        assert!(write(&d.join("x.londopy"), &sample(), &[("../x", b"")]).is_err());
    }

    #[test]
    fn enforces_entry_size_cap() {
        let d = tmpdir("cap");
        let p = d.join("big.londopy");
        write(&p, &sample(), &[("big.bin", &vec![0u8; 100_000])]).unwrap();
        let mut f = LondopyFile::open(&p).unwrap();
        assert!(f.read_entry("big.bin", 1000).is_err());
        assert_eq!(f.read_entry("big.bin", 100_000).unwrap().len(), 100_000);
    }

    #[test]
    fn refuses_newer_format_and_bad_ids() {
        let mut m = sample();
        m.format = FORMAT_VERSION + 1;
        assert!(m
            .validate()
            .unwrap_err()
            .to_string()
            .contains("update the app"));
        let mut m = sample();
        m.app = "Bad App".into();
        assert!(m.validate().is_err());
        assert!(valid_app_id("clip-vault2"));
        assert!(!valid_app_id("ClipVault"));
    }

    #[test]
    fn not_a_zip_is_a_clear_error() {
        let d = tmpdir("nozip");
        let p = d.join("x.londopy");
        std::fs::write(&p, b"hello").unwrap();
        let err = LondopyFile::open(&p).err().unwrap().to_string();
        assert!(err.contains("not a .londopy file"), "{err}");
    }

    #[test]
    fn args_parsing() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse_args(&a(&["app.exe", "--londopy-open", "C:\\x\\a.londopy"])),
            Some(LondopyArg::Open(PathBuf::from("C:\\x\\a.londopy")))
        );
        assert_eq!(
            parse_args(&a(&["app.exe", "D:\\b.LONDOPY"])),
            Some(LondopyArg::Open(PathBuf::from("D:\\b.LONDOPY")))
        );
        assert_eq!(
            parse_args(&a(&["app.exe", "--londopy-unregister"])),
            Some(LondopyArg::Unregister)
        );
        assert_eq!(parse_args(&a(&["app.exe", "--restarted"])), None);
        assert_eq!(parse_args(&a(&["app.exe"])), None);
    }

    #[test]
    fn dispatch_own_and_missing_apps() {
        let d = tmpdir("dispatch");
        let mine = d.join("mine.londopy");
        write(&mine, &sample(), &[]).unwrap();
        assert!(matches!(dispatch(&mine).unwrap(), Dispatch::Mine(_)));

        let mut other = sample();
        other.app = "someotherapp".into();
        other.homepage = "https://github.com/Londopy/someotherapp".into();
        let theirs = d.join("theirs.londopy");
        write(&theirs, &other, &[]).unwrap();
        assert_eq!(
            dispatch(&theirs).unwrap(),
            Dispatch::NotInstalled {
                app: "someotherapp".into(),
                homepage: "https://github.com/Londopy/someotherapp".into()
            }
        );
    }

    #[test]
    fn inbox_roundtrip_in_order() {
        let d = tmpdir("inbox");
        let f1 = d.join("one.londopy");
        let f2 = d.join("two.londopy");
        std::fs::write(&f1, b"").unwrap();
        std::fs::write(&f2, b"").unwrap();
        send_to_inbox(&d, &f1).unwrap();
        send_to_inbox(&d, &f2).unwrap();
        let got = drain_inbox(&d);
        assert_eq!(got.len(), 2);
        assert!(got[0].ends_with("one.londopy") && got[1].ends_with("two.londopy"));
        assert!(drain_inbox(&d).is_empty());
    }

    #[test]
    fn command_exe_parsing() {
        assert_eq!(
            parse_command_exe(r#""C:\Program Files\A B\App.exe" --londopy-open "%1""#),
            Some(PathBuf::from(r"C:\Program Files\A B\App.exe"))
        );
        assert_eq!(
            parse_command_exe(r#"C:\Apps\App.exe --londopy-open "%1""#),
            Some(PathBuf::from(r"C:\Apps\App.exe"))
        );
        assert_eq!(parse_command_exe("   "), None);
    }

    #[test]
    fn new_owner_skips_leaver_and_missing_exes() {
        let d = tmpdir("owner");
        let alive = d.join("b.exe");
        std::fs::write(&alive, b"").unwrap();
        let cands = vec![
            ("a-leaving".to_string(), d.join("a.exe")),
            ("c-gone".to_string(), d.join("missing.exe")),
            ("b-alive".to_string(), alive.clone()),
        ];
        assert_eq!(pick_new_owner(&cands, "a-leaving"), Some(alive));
        assert_eq!(pick_new_owner(&cands[..2], "a-leaving"), None);
    }
}

// EraSwitch-specific: the legacy .londo paths
#[cfg(all(test, windows))]
mod legacy_tests {
    use super::*;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn old_flags_and_old_files_still_parse() {
        assert_eq!(
            parse_args(&a(&["app.exe", "--londo-unregister"])),
            Some(LondopyArg::Unregister)
        );
        assert_eq!(
            parse_args(&a(&["app.exe", "--londo-open", "C:/x/old.londo"])),
            Some(LondopyArg::Open(PathBuf::from("C:/x/old.londo")))
        );
        assert_eq!(
            parse_args(&a(&["app.exe", "D:/old.LONDO"])),
            Some(LondopyArg::Open(PathBuf::from("D:/old.LONDO")))
        );
    }

    #[test]
    fn both_manifest_names_are_reserved() {
        assert!(!safe_entry_name("londopy.toml"));
        assert!(!safe_entry_name("londo.toml"));
        assert!(safe_entry_name("era.toml"));
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eraswitch-legacy-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_oversized_old_manifest_is_refused() {
        use std::io::Write;
        let d = tmp("big");
        let p = d.join("big.londo");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            z.start_file("londo.toml", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(&vec![b'#'; (MAX_MANIFEST_BYTES + 10) as usize])
                .unwrap();
            z.finish().unwrap();
        }
        let err = LondopyFile::open(&p).err().unwrap();
        assert!(format!("{err:#}").contains("limit"), "{err:#}");
        let _ = std::fs::remove_dir_all(&d);
    }

    // a throwaway HKCU tree standing in for the 0.1.x registration
    fn keys(tag: &str) -> (legacy::Keys, String) {
        let root = format!(
            r"Software\EraSwitchTests\legacy-{tag}-{}",
            std::process::id()
        );
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
        let k = legacy::Keys {
            apps: format!(r"{root}\Apps"),
            classes: format!(r"{root}\Classes"),
        };
        (k, root)
    }

    fn set(path: &str, name: &str, value: &str) {
        let (k, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(path)
            .unwrap();
        k.set_value(name, &value.to_string()).unwrap();
    }

    fn get(path: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(path)
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
    }

    const ME: &str = r"C:\Apps\EraSwitch\EraSwitch.exe";

    fn own_londo(k: &legacy::Keys, exe: &str) {
        set(&format!(r"{}\.londo", k.classes), "", "Londopy.Londo");
        set(
            &format!(r"{}\Londopy.Londo\shell\open\command", k.classes),
            "",
            &format!("\"{exe}\" --londo-open \"%1\""),
        );
        set(&format!(r"{}\eraswitch", k.apps), "Exe", ME);
    }

    #[test]
    fn the_last_old_app_removes_the_association() {
        let (k, root) = keys("last");
        own_londo(&k, ME);
        legacy::unregister_at(&k, Path::new(ME)).unwrap();
        assert_eq!(get(&format!(r"{}\.londo", k.classes)), None);
        assert_eq!(
            get(&format!(r"{}\Londopy.Londo\shell\open\command", k.classes)),
            None
        );
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }

    #[test]
    fn a_londo_owned_by_another_progid_is_kept() {
        let (k, root) = keys("other-progid");
        own_londo(&k, ME);
        set(&format!(r"{}\.londo", k.classes), "", "Someone.Else");
        legacy::unregister_at(&k, Path::new(ME)).unwrap();
        assert_eq!(
            get(&format!(r"{}\.londo", k.classes)).as_deref(),
            Some("Someone.Else")
        );
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }

    #[test]
    fn another_old_app_takes_over_the_double_click() {
        let (k, root) = keys("handoff");
        own_londo(&k, ME);
        // a real file, so pick_new_owner sees it as installed
        let other = std::env::current_exe().unwrap();
        set(
            &format!(r"{}\reelsmith", k.apps),
            "Exe",
            &other.to_string_lossy(),
        );
        legacy::unregister_at(&k, Path::new(ME)).unwrap();
        let cmd = get(&format!(r"{}\Londopy.Londo\shell\open\command", k.classes)).unwrap();
        assert!(
            cmd.contains(&*other.to_string_lossy()) && cmd.contains("--londo-open"),
            "{cmd}"
        );
        let icon = get(&format!(r"{}\Londopy.Londo\DefaultIcon", k.classes)).unwrap();
        assert!(icon.contains(&*other.to_string_lossy()), "{icon}");
        assert_eq!(
            get(&format!(r"{}\.londo", k.classes)).as_deref(),
            Some("Londopy.Londo")
        );
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }

    #[test]
    fn someone_elses_double_click_is_left_alone() {
        let (k, root) = keys("not-ours");
        own_londo(&k, r"C:\Apps\reelsmith\reelsmith.exe");
        legacy::unregister_at(&k, Path::new(ME)).unwrap();
        let cmd = get(&format!(r"{}\Londopy.Londo\shell\open\command", k.classes)).unwrap();
        assert!(cmd.contains("reelsmith.exe"), "{cmd}");
        // only our own old app entry went
        assert!(RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(format!(r"{}\eraswitch", k.apps))
            .is_err());
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }
}
