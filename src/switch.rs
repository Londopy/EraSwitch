// switch.rs
// turns the era on and off. before anything changes, what Windows 11 looked
// like is written to <data dir>\snapshot.toml; turning the era off puts each
// piece back from that file and deletes it. so the file existing means "the
// era is on", which survives crashes, kills and reboots:
//  - a panic, Ctrl+C or a fatal error restores from it (main.rs hooks)
//  - `EraSwitch.exe --restore` restores from it (the uninstallers run this)
//  - starting up with it present resumes the era (RetroBar and Open-Shell
//    don't start themselves at login), or finishes an interrupted restore
//
// restoring drops each piece from the file as soon as it's back, so a piece
// that keeps failing never drags the others along, and `restoring = true`
// stops a half-restored era from being resumed.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::config::AppConfig;
use crate::era::StartMenuPiece;
use crate::tools::{self, Tool};
use crate::{openshell, retrobar, sounds, win_ops};

const SNAPSHOT_SCHEMA: u32 = 1;

// one switch at a time: the tray worker, Exit, Ctrl+C and --restore can all
// end up here
static SWITCH: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    // a panic mid-switch poisons the lock; the snapshot on disk is still the
    // source of truth, so carry on
    SWITCH.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Snapshot {
    pub schema: u32,
    pub taken: String,
    pub era_name: String,
    // a restore started; pieces still listed are the ones not back yet
    pub restoring: bool,
    pub taskbar: Option<retrobar::Saved>,
    pub start_menu: Option<openshell::Saved>,
    // the wallpaper path before; "" means a solid colour
    pub wallpaper: Option<String>,
    pub sounds: Option<sounds::Saved>,
}

// every piece, behind one seam so tests can swap in fakes
pub trait Pieces {
    fn snapshot_taskbar(&self, exe: &Path) -> retrobar::Saved;
    fn snapshot_start(&self, exe: &Path, piece: &StartMenuPiece) -> openshell::Saved;
    fn snapshot_wallpaper(&self) -> Result<String>;
    fn snapshot_sounds(&self) -> Result<sounds::Saved>;
    fn apply_start(&self, saved: &openshell::Saved, piece: &StartMenuPiece) -> Result<()>;
    fn apply_taskbar(&self, saved: &retrobar::Saved, theme: &str) -> Result<()>;
    fn apply_wallpaper(&self, path: &str) -> Result<()>;
    fn apply_sounds(&self, scheme: &str) -> Result<()>;
    fn restore_taskbar(&self, saved: &retrobar::Saved) -> Result<()>;
    fn restore_start(&self, saved: &openshell::Saved) -> Result<()>;
    fn restore_wallpaper(&self, path: &str) -> Result<()>;
    fn restore_sounds(&self, saved: &sounds::Saved) -> Result<()>;
    fn resume_start(&self, saved: &openshell::Saved) -> Result<()>;
    fn resume_taskbar(&self, saved: &retrobar::Saved) -> Result<()>;
}

// the real thing
pub struct Real;

impl Pieces for Real {
    fn snapshot_taskbar(&self, exe: &Path) -> retrobar::Saved {
        retrobar::snapshot(exe)
    }
    fn snapshot_start(&self, exe: &Path, piece: &StartMenuPiece) -> openshell::Saved {
        openshell::snapshot(exe, piece)
    }
    fn snapshot_wallpaper(&self) -> Result<String> {
        win_ops::get_wallpaper()
    }
    fn snapshot_sounds(&self) -> Result<sounds::Saved> {
        sounds::snapshot()
    }
    fn apply_start(&self, saved: &openshell::Saved, piece: &StartMenuPiece) -> Result<()> {
        openshell::apply(saved, piece)
    }
    fn apply_taskbar(&self, saved: &retrobar::Saved, theme: &str) -> Result<()> {
        retrobar::apply(saved, theme)
    }
    fn apply_wallpaper(&self, path: &str) -> Result<()> {
        win_ops::set_wallpaper(path)
    }
    fn apply_sounds(&self, scheme: &str) -> Result<()> {
        sounds::apply(scheme)
    }
    fn restore_taskbar(&self, saved: &retrobar::Saved) -> Result<()> {
        retrobar::restore(saved)
    }
    fn restore_start(&self, saved: &openshell::Saved) -> Result<()> {
        openshell::restore(saved)
    }
    fn restore_wallpaper(&self, path: &str) -> Result<()> {
        // a picture that's gone (a rotated Spotlight cache file, say) can't
        // come back; a plain background beats being stuck
        if !path.is_empty() && !Path::new(path).is_file() {
            crate::dlog!("switch: old wallpaper {path} is gone; clearing instead");
            return win_ops::set_wallpaper("");
        }
        win_ops::set_wallpaper(path)
    }
    fn restore_sounds(&self, saved: &sounds::Saved) -> Result<()> {
        sounds::restore(saved)
    }
    fn resume_start(&self, saved: &openshell::Saved) -> Result<()> {
        openshell::resume(saved)
    }
    fn resume_taskbar(&self, saved: &retrobar::Saved) -> Result<()> {
        retrobar::resume(saved)
    }
}

pub fn snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join("snapshot.toml")
}

pub fn is_active(data_dir: &Path) -> bool {
    snapshot_path(data_dir).exists()
}

// None when the era is off. a snapshot that can't be used is moved aside to
// snapshot.toml.bad, so EraSwitch isn't stuck believing the era is on.
pub fn load(data_dir: &Path) -> Result<Option<Snapshot>> {
    let path = snapshot_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let parsed: Result<Snapshot, String> = toml::from_str(&text).map_err(|e| e.to_string());
    match parsed {
        Ok(snap) if snap.schema > SNAPSHOT_SCHEMA => {
            bail!("snapshot.toml was written by a newer EraSwitch; update the app before switching back")
        }
        Ok(snap) if snap.schema >= 1 => Ok(Some(snap)),
        // schema 0: empty or not ours. nothing in it to restore.
        Ok(_) => {
            set_aside(&path);
            crate::dlog!("switch: snapshot.toml had no schema; moved aside");
            Ok(None)
        }
        Err(e) => {
            set_aside(&path);
            bail!(
                "snapshot.toml was damaged ({e}) and has been moved to snapshot.toml.bad. \
                 Anything EraSwitch changed may need putting back by hand."
            )
        }
    }
}

fn set_aside(path: &Path) {
    let _ = std::fs::rename(path, path.with_extension("toml.bad"));
}

fn save(data_dir: &Path, snap: &Snapshot) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let path = snapshot_path(data_dir);
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(toml::to_string_pretty(snap)?.as_bytes())?;
        // on disk before we touch anything it describes
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn apply(cfg: &AppConfig, data_dir: &Path) -> Result<()> {
    let _g = lock();
    apply_with(&Real, cfg, data_dir)
}

pub fn restore(data_dir: &Path) -> Result<()> {
    let _g = lock();
    restore_with(&Real, data_dir)
}

// from one era straight to another (the tray's Era menu while one is on)
pub fn switch_across(cfg: &AppConfig, data_dir: &Path) -> Result<()> {
    let _g = lock();
    across_with(&Real, cfg, data_dir)
}

// the new era's checks run first, so a missing helper or wallpaper keeps you
// in the old era instead of dropping you on Windows 11. then Windows 11 comes
// back, so the new era's snapshot records Windows 11, not the old era; if the
// new era still fails, apply's own rollback leaves you on Windows 11 rather
// than half of each.
pub fn across_with(p: &dyn Pieces, cfg: &AppConfig, data_dir: &Path) -> Result<()> {
    preflight(cfg).context("The new era isn't ready, so nothing changed")?;
    restore_with(p, data_dir).context("Switching the old era off")?;
    apply_with(p, cfg, data_dir)
}

// for the panic hook: if a switch is mid-way on another thread, don't wait
// for it (that thread may be the one panicking); the next start finishes it
pub fn try_restore(data_dir: &Path) {
    if let Ok(_g) = SWITCH.try_lock() {
        let _ = restore_with(&Real, data_dir);
    }
}

pub fn resume(data_dir: &Path) -> Result<()> {
    let _g = lock();
    resume_with(&Real, data_dir)
}

// switches to the era in `cfg`. on a failure halfway, whatever was already
// changed is put back before the error comes back.
// everything an era needs, checked without changing anything: valid names,
// the helper programs (their paths come back), the wallpaper file, the sound
// scheme, and at least one piece switched on
fn preflight(cfg: &AppConfig) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let era = &cfg.era;
    era.validate()?;
    let retrobar_exe = if era.taskbar.enabled {
        Some(tools::require(Tool::RetroBar, &cfg.tools)?)
    } else {
        None
    };
    let openshell_exe = if era.start_menu.enabled {
        Some(tools::require(Tool::OpenShell, &cfg.tools)?)
    } else {
        None
    };
    if era.wallpaper.enabled && !Path::new(&era.wallpaper.path).is_file() {
        bail!(
            "The era's wallpaper file is missing: {}",
            era.wallpaper.path
        );
    }
    if era.sounds.enabled
        && !sounds::list_schemes()
            .iter()
            .any(|(id, _)| *id == era.sounds.scheme)
    {
        bail!(
            "There's no sound scheme called '{}'. Pick one in Settings, or turn sounds off.",
            era.sounds.scheme
        );
    }
    if !era.taskbar.enabled
        && !era.start_menu.enabled
        && !era.wallpaper.enabled
        && !era.sounds.enabled
    {
        bail!("Every piece of this era is switched off in Settings");
    }
    Ok((retrobar_exe, openshell_exe))
}

pub fn apply_with(p: &dyn Pieces, cfg: &AppConfig, data_dir: &Path) -> Result<()> {
    if is_active(data_dir) {
        return Ok(());
    }
    let era = &cfg.era;
    // find everything we need before touching anything
    let (retrobar_exe, openshell_exe) = preflight(cfg)?;

    // 1. remember Windows 11, on disk, first
    let snap = Snapshot {
        schema: SNAPSHOT_SCHEMA,
        taken: chrono::Local::now().to_rfc3339(),
        era_name: era.name.clone(),
        restoring: false,
        taskbar: retrobar_exe.as_deref().map(|e| p.snapshot_taskbar(e)),
        start_menu: openshell_exe
            .as_deref()
            .map(|e| p.snapshot_start(e, &era.start_menu)),
        wallpaper: if era.wallpaper.enabled {
            Some(p.snapshot_wallpaper()?)
        } else {
            None
        },
        sounds: if era.sounds.enabled {
            Some(p.snapshot_sounds()?)
        } else {
            None
        },
    };
    save(data_dir, &snap)?;
    crate::dlog!("switch: snapshot saved, applying '{}'", era.name);

    // 2. change things; undo everything on the first failure
    let result = (|| -> Result<()> {
        // start menu before the taskbar, so RetroBar's start button finds it
        if let Some(s) = &snap.start_menu {
            p.apply_start(s, &era.start_menu).context("Start menu")?;
        }
        if let Some(s) = &snap.taskbar {
            p.apply_taskbar(s, &era.taskbar.retrobar_theme)
                .context("Taskbar")?;
        }
        if snap.wallpaper.is_some() {
            p.apply_wallpaper(&era.wallpaper.path)
                .context("Wallpaper")?;
        }
        if snap.sounds.is_some() {
            p.apply_sounds(&era.sounds.scheme).context("Sounds")?;
        }
        Ok(())
    })();

    if let Err(e) = result {
        crate::dlog!("switch: apply failed ({e:#}), rolling back");
        if let Err(r) = restore_with(p, data_dir) {
            crate::dlog!("switch: rollback also failed: {r:#}");
            return Err(e.context(format!("and putting things back failed too: {r:#}")));
        }
        return Err(e);
    }
    Ok(())
}

// back to Windows 11. every piece is tried; each one that's back is dropped
// from the snapshot straight away, and the file goes once all of them are
pub fn restore_with(p: &dyn Pieces, data_dir: &Path) -> Result<()> {
    let Some(mut snap) = load(data_dir)? else {
        return Ok(());
    };
    if !snap.restoring {
        snap.restoring = true;
        save(data_dir, &snap)?;
    }
    let mut errors = Vec::new();

    if let Some(s) = snap.taskbar.clone() {
        match p.restore_taskbar(&s) {
            Ok(()) => snap.taskbar = None,
            Err(e) => errors.push(format!("taskbar: {e:#}")),
        }
        save(data_dir, &snap)?;
    }
    if let Some(s) = snap.start_menu.clone() {
        match p.restore_start(&s) {
            Ok(()) => snap.start_menu = None,
            Err(e) => errors.push(format!("start menu: {e:#}")),
        }
        save(data_dir, &snap)?;
    }
    if let Some(w) = snap.wallpaper.clone() {
        match p.restore_wallpaper(&w) {
            Ok(()) => snap.wallpaper = None,
            Err(e) => errors.push(format!("wallpaper: {e:#}")),
        }
        save(data_dir, &snap)?;
    }
    if let Some(s) = snap.sounds.clone() {
        match p.restore_sounds(&s) {
            Ok(()) => snap.sounds = None,
            Err(e) => errors.push(format!("sounds: {e:#}")),
        }
        save(data_dir, &snap)?;
    }

    if errors.is_empty() {
        match std::fs::remove_file(snapshot_path(data_dir)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        crate::dlog!("switch: back to Windows 11");
        Ok(())
    } else {
        bail!(
            "Couldn't put everything back ({}). Try again from the tray.",
            errors.join("; ")
        )
    }
}

// started with the era on: bring back the programs that don't start
// themselves, or finish a restore that was cut off
pub fn resume_with(p: &dyn Pieces, data_dir: &Path) -> Result<()> {
    let Some(snap) = load(data_dir)? else {
        return Ok(());
    };
    if snap.restoring {
        crate::dlog!("switch: a restore was cut off; finishing it");
        return restore_with(p, data_dir);
    }
    if let Some(s) = &snap.start_menu {
        p.resume_start(s)?;
    }
    if let Some(s) = &snap.taskbar {
        p.resume_taskbar(s)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eraswitch-switch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // records every call, fails the ones named in `fail`, and notes whether
    // the snapshot was already on disk when each change happened
    struct Fake {
        dir: PathBuf,
        fail: RefCell<Vec<&'static str>>,
        calls: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(dir: &Path) -> Self {
            Self {
                dir: dir.to_path_buf(),
                fail: RefCell::new(Vec::new()),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn act(&self, name: &'static str) -> Result<()> {
            let on_disk = is_active(&self.dir);
            self.calls.borrow_mut().push(format!(
                "{name}{}",
                if on_disk { "" } else { " (NO SNAPSHOT)" }
            ));
            if self.fail.borrow().contains(&name) {
                bail!("{name} failed");
            }
            Ok(())
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Pieces for Fake {
        fn snapshot_taskbar(&self, exe: &Path) -> retrobar::Saved {
            retrobar::Saved {
                exe: exe.to_string_lossy().into(),
                appbar_state: 1,
                ..Default::default()
            }
        }
        fn snapshot_start(&self, exe: &Path, _: &StartMenuPiece) -> openshell::Saved {
            openshell::Saved {
                exe: exe.to_string_lossy().into(),
                ..Default::default()
            }
        }
        fn snapshot_wallpaper(&self) -> Result<String> {
            Ok("old.jpg".into())
        }
        fn snapshot_sounds(&self) -> Result<sounds::Saved> {
            Ok(Default::default())
        }
        fn apply_start(&self, _: &openshell::Saved, _: &StartMenuPiece) -> Result<()> {
            self.act("apply_start")
        }
        fn apply_taskbar(&self, _: &retrobar::Saved, _: &str) -> Result<()> {
            self.act("apply_taskbar")
        }
        fn apply_wallpaper(&self, _: &str) -> Result<()> {
            self.act("apply_wallpaper")
        }
        fn apply_sounds(&self, _: &str) -> Result<()> {
            self.act("apply_sounds")
        }
        fn restore_taskbar(&self, _: &retrobar::Saved) -> Result<()> {
            self.act("restore_taskbar")
        }
        fn restore_start(&self, _: &openshell::Saved) -> Result<()> {
            self.act("restore_start")
        }
        fn restore_wallpaper(&self, _: &str) -> Result<()> {
            self.act("restore_wallpaper")
        }
        fn restore_sounds(&self, _: &sounds::Saved) -> Result<()> {
            self.act("restore_sounds")
        }
        fn resume_start(&self, _: &openshell::Saved) -> Result<()> {
            self.act("resume_start")
        }
        fn resume_taskbar(&self, _: &retrobar::Saved) -> Result<()> {
            self.act("resume_taskbar")
        }
    }

    // taskbar + start menu on, with "tools" that exist (this test binary)
    fn cfg() -> AppConfig {
        let me = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut c = AppConfig::default();
        c.tools.retrobar = me.clone();
        c.tools.openshell = me;
        c
    }

    #[test]
    fn snapshot_is_on_disk_before_the_first_change() {
        let d = temp_dir("first");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        assert_eq!(f.calls(), vec!["apply_start", "apply_taskbar"]);
        assert!(is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_piece_rolls_everything_back() {
        let d = temp_dir("rollback");
        let f = Fake::new(&d);
        f.fail.borrow_mut().push("apply_taskbar");
        assert!(apply_with(&f, &cfg(), &d).is_err());
        assert_eq!(
            f.calls(),
            vec![
                "apply_start",
                "apply_taskbar",
                "restore_taskbar",
                "restore_start"
            ]
        );
        assert!(!is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stuck_piece_keeps_only_itself_in_the_snapshot() {
        let d = temp_dir("stuck");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        f.fail.borrow_mut().push("restore_start");
        assert!(restore_with(&f, &d).is_err());

        let snap = load(&d).unwrap().unwrap();
        assert!(snap.restoring);
        assert!(snap.taskbar.is_none(), "the taskbar is back, so it's gone");
        assert!(
            snap.start_menu.is_some(),
            "the start menu isn't, so it stays"
        );

        // a retry only redoes what's left
        f.fail.borrow_mut().clear();
        f.calls.borrow_mut().clear();
        restore_with(&f, &d).unwrap();
        assert_eq!(f.calls(), vec!["restore_start"]);
        assert!(!is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resume_finishes_a_cut_off_restore_instead_of_relaunching() {
        let d = temp_dir("resume");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        f.fail.borrow_mut().push("restore_start");
        let _ = restore_with(&f, &d);

        f.fail.borrow_mut().clear();
        f.calls.borrow_mut().clear();
        resume_with(&f, &d).unwrap();
        assert_eq!(f.calls(), vec!["restore_start"]);
        assert!(!is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn switching_across_goes_through_windows_11() {
        let d = temp_dir("across");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        f.calls.borrow_mut().clear();

        let mut vista = cfg();
        vista.era.apply_preset(crate::era::Preset::WindowsVista);
        across_with(&f, &vista, &d).unwrap();
        assert_eq!(
            f.calls(),
            vec![
                "restore_taskbar",
                "restore_start",
                "apply_start",
                "apply_taskbar"
            ]
        );
        let snap = load(&d).unwrap().unwrap();
        assert_eq!(snap.era_name, "Windows Vista");
        assert!(!snap.restoring);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_new_era_that_isnt_ready_leaves_the_old_one_on() {
        let d = temp_dir("across-not-ready");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        f.calls.borrow_mut().clear();
        let mut vista = cfg();
        vista.era.apply_preset(crate::era::Preset::WindowsVista);
        vista.era.wallpaper.enabled = true;
        vista.era.wallpaper.path = r"Z:\no\such\picture.jpg".into();
        assert!(across_with(&f, &vista, &d).is_err());
        assert!(f.calls().is_empty(), "nothing may change: {:?}", f.calls());
        let snap = load(&d).unwrap().unwrap();
        assert_eq!(snap.era_name, "Windows XP");
        assert!(!snap.restoring);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_switch_across_ends_on_windows_11() {
        let d = temp_dir("across-fail");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        f.fail.borrow_mut().push("apply_taskbar");
        let mut vista = cfg();
        vista.era.apply_preset(crate::era::Preset::WindowsVista);
        assert!(across_with(&f, &vista, &d).is_err());
        assert!(!is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resume_relaunches_when_the_era_is_simply_on() {
        let d = temp_dir("relaunch");
        let f = Fake::new(&d);
        apply_with(&f, &cfg(), &d).unwrap();
        f.calls.borrow_mut().clear();
        resume_with(&f, &d).unwrap();
        assert_eq!(f.calls(), vec!["resume_start", "resume_taskbar"]);
        assert!(is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hostile_names_never_reach_a_piece() {
        let d = temp_dir("hostile");
        let f = Fake::new(&d);
        let mut c = cfg();
        c.era.taskbar.retrobar_theme = r"\\attacker\share\x".into();
        assert!(apply_with(&f, &c, &d).is_err());
        assert!(f.calls().is_empty());
        assert!(!is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn snapshot_roundtrips_through_toml() {
        let snap = Snapshot {
            schema: SNAPSHOT_SCHEMA,
            taken: "2026-10-16T01:00:00-07:00".into(),
            era_name: "Windows XP".into(),
            restoring: false,
            taskbar: Some(retrobar::Saved {
                exe: r"C:\RetroBar\RetroBar.exe".into(),
                was_running: false,
                appbar_state: 1,
                settings_json: None,
            }),
            start_menu: Some(openshell::Saved {
                exe: r"C:\Program Files\Open-Shell\StartMenu.exe".into(),
                was_running: false,
                values: vec![openshell::SavedValue {
                    name: "AutoStart".into(),
                    dword: Some(1),
                    text: None,
                }],
            }),
            wallpaper: Some(String::new()),
            sounds: None,
        };
        let text = toml::to_string_pretty(&snap).unwrap();
        let back: Snapshot = toml::from_str(&text).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn no_snapshot_means_not_active_and_restore_is_a_no_op() {
        let d = temp_dir("none");
        assert!(!is_active(&d));
        assert!(load(&d).unwrap().is_none());
        restore_with(&Fake::new(&d), &d).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn empty_and_damaged_snapshots_are_moved_aside() {
        let d = temp_dir("damaged");
        std::fs::write(snapshot_path(&d), "").unwrap();
        assert!(load(&d).unwrap().is_none());
        assert!(!is_active(&d));
        std::fs::write(snapshot_path(&d), "this is [not toml").unwrap();
        assert!(load(&d).is_err());
        assert!(!is_active(&d));
        assert!(d.join("snapshot.toml.bad").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_snapshots_are_refused_and_kept() {
        let d = temp_dir("newer");
        std::fs::write(snapshot_path(&d), "schema = 99\n").unwrap();
        assert!(load(&d).is_err());
        assert!(is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn apply_refuses_when_everything_is_off() {
        let d = temp_dir("alloff");
        let mut c = cfg();
        c.era.taskbar.enabled = false;
        c.era.start_menu.enabled = false;
        assert!(apply_with(&Fake::new(&d), &c, &d).is_err());
        assert!(!is_active(&d));
        let _ = std::fs::remove_dir_all(&d);
    }
}
