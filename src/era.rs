// era.rs
// what an "era" looks like (which RetroBar theme, which Open-Shell skin, which
// wallpaper and sound scheme), and saving/opening one as a .londopy file
// (kind = "era", payload era.toml plus the wallpaper image if there is one).

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::londopy;

pub const LONDO_KIND: &str = "era";
const ERA_ENTRY: &str = "era.toml";
const MAX_ERA_TOML: u64 = 64 * 1024;
const MAX_WALLPAPER: u64 = 25 * 1024 * 1024;
const WALLPAPER_EXTS: &[&str] = &["jpg", "jpeg", "png", "bmp"];
// bump when era.toml changes shape; older files keep loading through serde defaults
const ERA_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Era {
    pub schema: u32,
    pub name: String,
    pub taskbar: TaskbarPiece,
    pub start_menu: StartMenuPiece,
    pub wallpaper: WallpaperPiece,
    pub sounds: SoundsPiece,
}

impl Default for Era {
    fn default() -> Self {
        Self {
            schema: ERA_SCHEMA,
            name: "Windows XP".into(),
            taskbar: TaskbarPiece::default(),
            start_menu: StartMenuPiece::default(),
            wallpaper: WallpaperPiece::default(),
            sounds: SoundsPiece::default(),
        }
    }
}

// RetroBar replaces the taskbar
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TaskbarPiece {
    pub enabled: bool,
    // one of RetroBar's built-in themes
    pub retrobar_theme: String,
}

impl Default for TaskbarPiece {
    fn default() -> Self {
        Self {
            enabled: true,
            retrobar_theme: "Windows XP Blue".into(),
        }
    }
}

// RetroBar's themes for the eras EraSwitch knows, as named in its Themes folder
pub const RETROBAR_THEMES: &[&str] = &[
    "Windows XP Blue",
    "Windows XP Olive Green",
    "Windows XP Silver",
    "Windows XP Royale",
    "Windows XP Royale Noir",
    "Windows XP Zune Style",
    "Windows XP Embedded Style",
    "Windows XP Classic",
    "Windows Vista Aero",
    "Windows Vista Basic",
    "Windows Vista Classic",
    "Windows Longhorn Aero",
];

// the built-in eras. a preset sets the name, the taskbar and the Start menu;
// wallpaper and sounds are yours (Microsoft's originals can't ship), so
// switching preset keeps whatever you picked for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    WindowsXp,
    WindowsVista,
}

impl Preset {
    pub const ALL: [Preset; 2] = [Preset::WindowsXp, Preset::WindowsVista];

    pub fn name(self) -> &'static str {
        match self {
            Preset::WindowsXp => "Windows XP",
            Preset::WindowsVista => "Windows Vista",
        }
    }

    fn pieces(self) -> (TaskbarPiece, StartMenuPiece) {
        match self {
            Preset::WindowsXp => (TaskbarPiece::default(), StartMenuPiece::default()),
            // the Vista orb and black glass bar come from RetroBar's theme;
            // Open-Shell's Windows 7 style with its Aero skin is the closest
            // thing to Vista's two-column glass Start menu
            Preset::WindowsVista => (
                TaskbarPiece {
                    enabled: true,
                    retrobar_theme: "Windows Vista Aero".into(),
                },
                StartMenuPiece {
                    enabled: true,
                    menu_style: 2,
                    skin: "Windows Aero".into(),
                },
            ),
        }
    }

    // the preset this era is: its name and its look (theme, menu style, skin)
    // both match. a renamed or restyled era is custom. on/off switches don't
    // count, since picking a preset keeps them.
    pub fn of(era: &Era) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| {
            let (taskbar, start) = p.pieces();
            p.name() == era.name
                && taskbar.retrobar_theme == era.taskbar.retrobar_theme
                && start.menu_style == era.start_menu.menu_style
                && start.skin == era.start_menu.skin
        })
    }
}

impl Era {
    // turns this era into `preset`: its name and look. wallpaper, sounds and
    // which pieces are switched on stay as you set them.
    pub fn apply_preset(&mut self, preset: Preset) {
        let (taskbar, start_menu) = preset.pieces();
        self.name = preset.name().into();
        self.taskbar.retrobar_theme = taskbar.retrobar_theme;
        self.start_menu.menu_style = start_menu.menu_style;
        self.start_menu.skin = start_menu.skin;
    }
}

// Open-Shell replaces the Start menu
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StartMenuPiece {
    pub enabled: bool,
    // Open-Shell's MenuStyle: 0 classic, 1 classic with two columns (XP), 2 Windows 7
    pub menu_style: u32,
    // skin name as shipped in Open-Shell's Skins folder
    pub skin: String,
}

impl Default for StartMenuPiece {
    fn default() -> Self {
        Self {
            enabled: true,
            menu_style: 1,
            skin: "Windows XP Luna".into(),
        }
    }
}

// off by default: Bliss is Microsoft's, so you bring your own picture
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WallpaperPiece {
    pub enabled: bool,
    pub path: String,
}

// off by default: the XP sounds are Microsoft's too. names a Windows sound
// scheme (Control Panel > Sound > "Save As...") holding your XP sounds
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SoundsPiece {
    pub enabled: bool,
    pub scheme: String,
}

// theme and skin names end up in other programs' file lookups: RetroBar joins
// its theme name onto its Themes folder and loads that XAML, so a name like
// `\\host\share\x` or `..\..\x` would make it load (and run) someone else's
// file. only plain names get through.
pub fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('.')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || " ()._-".contains(c))
}

impl Era {
    // refuses anything that could point a helper program outside its own folders
    pub fn validate(&self) -> Result<()> {
        if !safe_name(&self.taskbar.retrobar_theme) {
            bail!(
                "'{}' isn't a valid RetroBar theme name",
                self.taskbar.retrobar_theme
            );
        }
        if !safe_name(&self.start_menu.skin) {
            bail!(
                "'{}' isn't a valid Open-Shell skin name",
                self.start_menu.skin
            );
        }
        if self.start_menu.menu_style > 2 {
            bail!(
                "Start menu style {} doesn't exist",
                self.start_menu.menu_style
            );
        }
        Ok(())
    }
}

// file name for a wallpaper inside the .londopy, if its extension is allowed
fn wallpaper_entry_name(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    WALLPAPER_EXTS
        .contains(&ext.as_str())
        .then(|| format!("wallpaper.{ext}"))
}

// a folder-safe version of the era name
fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "era".into()
    } else {
        s.chars().take(48).collect()
    }
}

pub fn save_londopy(era: &Era, path: &Path) -> Result<()> {
    let created = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let manifest = londopy::Manifest::new(LONDO_KIND, &era.name, &created);

    let mut payload = era.clone();
    let mut image: Option<(String, Vec<u8>)> = None;
    if !era.wallpaper.path.is_empty() {
        let src = Path::new(&era.wallpaper.path);
        let name = wallpaper_entry_name(src)
            .with_context(|| format!("Wallpaper must be one of: {}", WALLPAPER_EXTS.join(", ")))?;
        let bytes = std::fs::read(src).with_context(|| format!("Reading {}", src.display()))?;
        if bytes.len() as u64 > MAX_WALLPAPER {
            bail!("The wallpaper is over {} MB", MAX_WALLPAPER / 1024 / 1024);
        }
        // the path only means something on this PC; the file travels inside
        payload.wallpaper.path = name.clone();
        image = Some((name, bytes));
    }

    let toml_text = toml::to_string_pretty(&payload)?;
    let mut entries: Vec<(&str, &[u8])> = vec![(ERA_ENTRY, toml_text.as_bytes())];
    if let Some((name, bytes)) = &image {
        entries.push((name.as_str(), bytes.as_slice()));
    }
    londopy::write(path, &manifest, &entries)
}

// opens an era file; a bundled wallpaper is extracted under <data dir>\eras\<name>\
pub fn open_londopy(path: &Path, data_dir: &Path) -> Result<Era> {
    let mut file = londopy::LondopyFile::open(path)?;
    if file.manifest.app != londopy::APP_ID || file.manifest.kind != LONDO_KIND {
        bail!(
            "This .londopy file is a '{}' for '{}', not an EraSwitch era",
            file.manifest.kind,
            file.manifest.app
        );
    }
    let raw = file.read_entry(ERA_ENTRY, MAX_ERA_TOML)?;
    let text = String::from_utf8(raw).context("era.toml is not UTF-8")?;
    let mut era: Era = toml::from_str(&text).context("Parsing era.toml")?;
    if era.schema > ERA_SCHEMA {
        bail!("This era was made by a newer EraSwitch; update the app to open it");
    }
    // an era file is untrusted: check it before anything from it is used
    era.validate()?;

    // only ever extract the one wallpaper name we'd have written ourselves
    let wanted = wallpaper_entry_name(Path::new(&era.wallpaper.path));
    era.wallpaper.path = String::new();
    if let Some(name) = wanted {
        if file.entry_names().contains(&name) {
            let bytes = file.read_entry(&name, MAX_WALLPAPER)?;
            let dir: PathBuf = data_dir.join("eras").join(slug(&era.name));
            std::fs::create_dir_all(&dir)?;
            let out = dir.join(&name);
            std::fs::write(&out, bytes).with_context(|| format!("Writing {}", out.display()))?;
            era.wallpaper.path = out.to_string_lossy().into_owned();
        }
    }
    if era.wallpaper.path.is_empty() {
        era.wallpaper.enabled = false;
    }
    Ok(era)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("eraswitch-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn default_era_is_xp() {
        let e = Era::default();
        assert_eq!(e.taskbar.retrobar_theme, "Windows XP Blue");
        assert_eq!(e.start_menu.menu_style, 1);
        assert_eq!(e.start_menu.skin, "Windows XP Luna");
        assert!(!e.wallpaper.enabled && !e.sounds.enabled);
    }

    #[test]
    fn every_built_in_name_is_safe() {
        for t in RETROBAR_THEMES {
            assert!(safe_name(t), "{t}");
        }
        assert!(Era::default().validate().is_ok());
        for p in Preset::ALL {
            let mut e = Era::default();
            e.apply_preset(p);
            assert!(e.validate().is_ok(), "{}", p.name());
            assert!(
                RETROBAR_THEMES.contains(&e.taskbar.retrobar_theme.as_str()),
                "{} uses a theme the picker doesn't list",
                p.name()
            );
        }
    }

    #[test]
    fn the_default_era_is_the_xp_preset() {
        let mut xp = Era::default();
        xp.apply_preset(Preset::WindowsXp);
        assert_eq!(xp, Era::default());
        assert_eq!(Preset::of(&Era::default()), Some(Preset::WindowsXp));
    }

    #[test]
    fn vista_uses_the_windows_7_style_aero_menu() {
        let mut e = Era::default();
        e.apply_preset(Preset::WindowsVista);
        assert_eq!(e.name, "Windows Vista");
        assert_eq!(e.taskbar.retrobar_theme, "Windows Vista Aero");
        assert_eq!(e.start_menu.menu_style, 2);
        assert_eq!(e.start_menu.skin, "Windows Aero");
        // Open-Shell keeps the Windows 7 style's skin under its own value
        assert_eq!(crate::openshell::skin_value_name(2), "SkinW7");
        assert_eq!(Preset::of(&e), Some(Preset::WindowsVista));
    }

    #[test]
    fn presets_keep_your_wallpaper_and_sounds() {
        let mut e = Era {
            wallpaper: WallpaperPiece {
                enabled: true,
                path: "C:/pics/bliss.jpg".into(),
            },
            sounds: SoundsPiece {
                enabled: true,
                scheme: "XP".into(),
            },
            ..Default::default()
        };
        e.apply_preset(Preset::WindowsVista);
        assert_eq!(e.wallpaper.path, "C:/pics/bliss.jpg");
        assert_eq!(e.sounds.scheme, "XP");
        // a renamed era is nobody's preset
        e.name = "My Vista".into();
        assert_eq!(Preset::of(&e), None);
    }

    #[test]
    fn presets_keep_which_pieces_are_on() {
        let mut e = Era::default();
        e.start_menu.enabled = false;
        e.apply_preset(Preset::WindowsVista);
        assert!(
            !e.start_menu.enabled,
            "turning Open-Shell off must survive a preset"
        );
        assert!(e.taskbar.enabled);
        // and on/off doesn't stop it being the preset
        assert_eq!(Preset::of(&e), Some(Preset::WindowsVista));
    }

    #[test]
    fn a_restyled_era_is_custom_until_reset() {
        // an untouched-name 0.1.x era with another theme isn't "Windows XP"
        let mut e = Era::default();
        e.taskbar.retrobar_theme = "Windows XP Silver".into();
        assert_eq!(Preset::of(&e), None);
        // so picking Windows XP is a reset again
        e.apply_preset(Preset::WindowsXp);
        assert_eq!(e, Era::default());
    }

    #[test]
    fn path_like_names_are_refused() {
        for bad in [
            r"\\host\share\x",
            r"C:\Users\Public\x",
            r"..\..\x",
            "../x",
            ".hidden",
            "a/b",
            "",
            &"x".repeat(65),
        ] {
            assert!(!safe_name(bad), "{bad}");
        }
        let mut e = Era::default();
        e.taskbar.retrobar_theme = r"\\evil\share\theme".into();
        assert!(e.validate().is_err());
        let mut e = Era::default();
        e.start_menu.skin = r"..\..\skin".into();
        assert!(e.validate().is_err());
        let mut e = Era::default();
        e.start_menu.menu_style = 7;
        assert!(e.validate().is_err());
    }

    #[test]
    fn hostile_era_files_are_refused() {
        let dir = temp_dir("hostile");
        let file = dir.join("evil.londopy");
        let m = londopy::Manifest::new(LONDO_KIND, "evil", "2026-10-16T00:00:00Z");
        let toml = "[taskbar]\nenabled = true\nretrobar_theme = '\\\\attacker\\share\\x'\n";
        londopy::write(&file, &m, &[("era.toml", toml.as_bytes())]).unwrap();
        assert!(open_londopy(&file, &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slug_is_folder_safe() {
        assert_eq!(slug("Windows XP"), "windows-xp");
        assert_eq!(slug("../../evil"), "evil");
        assert_eq!(slug("???"), "era");
    }

    #[test]
    fn only_image_wallpapers_are_bundled() {
        assert_eq!(
            wallpaper_entry_name(Path::new("C:/x/Bliss.JPG")).as_deref(),
            Some("wallpaper.jpg")
        );
        assert_eq!(wallpaper_entry_name(Path::new("C:/x/evil.exe")), None);
        assert_eq!(wallpaper_entry_name(Path::new("C:/x/noext")), None);
    }

    #[test]
    fn era_roundtrips_through_londo_with_wallpaper() {
        let dir = temp_dir("roundtrip");
        let img = dir.join("bliss.png");
        std::fs::write(&img, b"not really a png").unwrap();

        let era = Era {
            name: "My XP".into(),
            taskbar: TaskbarPiece {
                enabled: true,
                retrobar_theme: "Windows XP Silver".into(),
            },
            wallpaper: WallpaperPiece {
                enabled: true,
                path: img.to_string_lossy().into(),
            },
            ..Default::default()
        };

        let file = dir.join("my.londopy");
        save_londopy(&era, &file).unwrap();

        let data = dir.join("data");
        let back = open_londopy(&file, &data).unwrap();
        assert_eq!(back.taskbar, era.taskbar);
        assert!(back.wallpaper.enabled);
        let extracted = PathBuf::from(&back.wallpaper.path);
        assert!(extracted.starts_with(data.join("eras").join("my-xp")));
        assert_eq!(std::fs::read(extracted).unwrap(), b"not really a png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn era_without_wallpaper_turns_the_piece_off() {
        let dir = temp_dir("nowall");
        let mut era = Era::default();
        era.wallpaper.enabled = true; // enabled but no file: nothing to carry
        let file = dir.join("plain.londopy");
        save_londopy(&era, &file).unwrap();
        let back = open_londopy(&file, &dir).unwrap();
        assert!(!back.wallpaper.enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_kinds_are_refused() {
        let dir = temp_dir("kind");
        let file = dir.join("p.londopy");
        let m = londopy::Manifest::new("profile", "x", "2026-10-16T00:00:00Z");
        londopy::write(&file, &m, &[("era.toml", b"name = 'x'")]).unwrap();
        assert!(open_londopy(&file, &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // an era saved by EraSwitch 0.1.x: a .londo with londo.toml as its manifest
    #[test]
    fn legacy_londo_eras_still_open() {
        use std::io::Write;
        let dir = temp_dir("legacy");
        let file = dir.join("old.londo");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("londo.toml", opts).unwrap();
            z.write_all(b"format = 1\napp = \"eraswitch\"\nkind = \"era\"\ntitle = \"Old XP\"\n")
                .unwrap();
            z.start_file("era.toml", opts).unwrap();
            z.write_all(b"name = \"Old XP\"\n[taskbar]\nretrobar_theme = \"Windows XP Silver\"\n")
                .unwrap();
            z.finish().unwrap();
        }
        let back = open_londopy(&file, &dir).unwrap();
        assert_eq!(back.name, "Old XP");
        assert_eq!(back.taskbar.retrobar_theme, "Windows XP Silver");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
