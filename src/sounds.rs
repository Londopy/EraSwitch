// sounds.rs
// the sound piece: switches Windows to a named sound scheme, the same way
// Control Panel > Sound does. each event lives at
//   HKCU\AppEvents\Schemes\Apps\<app>\<event>\<scheme id>   (default value = .wav path)
// and the one playing is the `.Current` copy beside them. the scheme's name
// goes in HKCU\AppEvents\Schemes's default value.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

pub const SCHEMES_KEY: &str = r"AppEvents\Schemes";
const CURRENT: &str = ".Current";

// "<app>\<event>" -> scheme id -> wav path
pub type Tree = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub scheme: String,
    // "<app>\<event>" -> what .Current held; missing = no .Current key
    pub current: BTreeMap<String, String>,
}

// what every event's .Current becomes under `scheme`: its sound if the scheme
// has one, silence otherwise (that's what Control Panel does too)
pub fn current_for_scheme(tree: &Tree, scheme: &str) -> BTreeMap<String, String> {
    tree.iter()
        .map(|(event, by_scheme)| {
            let wav = by_scheme.get(scheme).cloned().unwrap_or_default();
            (event.clone(), wav)
        })
        .collect()
}

// (id, display name) for every scheme Windows knows
pub fn list_schemes() -> Vec<(String, String)> {
    list_schemes_at(SCHEMES_KEY)
}

pub fn list_schemes_at(root: &str) -> Vec<(String, String)> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(names) = hkcu.open_subkey_with_flags(format!(r"{root}\Names"), KEY_READ) else {
        return Vec::new();
    };
    names
        .enum_keys()
        .flatten()
        .map(|id| {
            let display = names
                .open_subkey(&id)
                .and_then(|k| k.get_value::<String, _>(""))
                .unwrap_or_else(|_| id.clone());
            (id, display)
        })
        .collect()
}

// every event and the sound each scheme gives it
pub fn read_tree_at(root: &str) -> Result<Tree> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let apps = hkcu
        .open_subkey_with_flags(format!(r"{root}\Apps"), KEY_READ)
        .with_context(|| format!("Opening HKCU\\{root}\\Apps"))?;
    let mut tree = Tree::new();
    for app in apps.enum_keys().flatten() {
        let Ok(app_key) = apps.open_subkey(&app) else {
            continue;
        };
        for event in app_key.enum_keys().flatten() {
            let Ok(event_key) = app_key.open_subkey(&event) else {
                continue;
            };
            let mut by_scheme = BTreeMap::new();
            for scheme in event_key.enum_keys().flatten() {
                let wav = event_key
                    .open_subkey(&scheme)
                    .and_then(|k| k.get_value::<String, _>(""))
                    .unwrap_or_default();
                by_scheme.insert(scheme, wav);
            }
            tree.insert(format!(r"{app}\{event}"), by_scheme);
        }
    }
    Ok(tree)
}

pub fn snapshot_at(root: &str) -> Result<Saved> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let scheme = hkcu
        .open_subkey_with_flags(root, KEY_READ)
        .and_then(|k| k.get_value::<String, _>(""))
        .unwrap_or_default();
    let current = read_tree_at(root)?
        .into_iter()
        .filter_map(|(event, by_scheme)| by_scheme.get(CURRENT).map(|w| (event, w.clone())))
        .collect();
    Ok(Saved { scheme, current })
}

fn write_current_at(
    root: &str,
    scheme: &str,
    values: &BTreeMap<String, String>,
    remove_others: bool,
) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let apps = hkcu.open_subkey_with_flags(format!(r"{root}\Apps"), KEY_READ | KEY_WRITE)?;
    if remove_others {
        // events that had no .Current before we started lose the one we added
        for event in read_tree_at(root)?.keys() {
            if !values.contains_key(event) {
                let _ = apps.delete_subkey(format!(r"{event}\{CURRENT}"));
            }
        }
    }
    for (event, wav) in values {
        let (k, _) = apps.create_subkey(format!(r"{event}\{CURRENT}"))?;
        k.set_value("", wav)?;
    }
    let (schemes, _) = hkcu.create_subkey(root)?;
    schemes.set_value("", &scheme.to_string())?;
    Ok(())
}

pub fn apply_at(root: &str, scheme: &str) -> Result<()> {
    if !list_schemes_at(root).iter().any(|(id, _)| id == scheme) {
        bail!("There's no sound scheme called '{scheme}'. Make one in Control Panel > Sound (Save As...) first.");
    }
    let tree = read_tree_at(root)?;
    write_current_at(root, scheme, &current_for_scheme(&tree, scheme), false)
}

pub fn restore_at(root: &str, saved: &Saved) -> Result<()> {
    write_current_at(root, &saved.scheme, &saved.current, true)
}

pub fn snapshot() -> Result<Saved> {
    snapshot_at(SCHEMES_KEY)
}

pub fn apply(scheme: &str) -> Result<()> {
    apply_at(SCHEMES_KEY, scheme)
}

pub fn restore(saved: &Saved) -> Result<()> {
    restore_at(SCHEMES_KEY, saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(entries: &[(&str, &[(&str, &str)])]) -> Tree {
        entries
            .iter()
            .map(|(e, s)| {
                (
                    e.to_string(),
                    s.iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn scheme_sounds_win_and_gaps_go_silent() {
        let t = tree(&[
            (
                r".Default\SystemExit",
                &[(".Default", "exit10.wav"), ("XP", "xpexit.wav")],
            ),
            (r".Default\Notify", &[(".Default", "notify.wav")]),
        ]);
        let cur = current_for_scheme(&t, "XP");
        assert_eq!(cur[r".Default\SystemExit"], "xpexit.wav");
        assert_eq!(cur[r".Default\Notify"], "");
    }

    // builds a tiny fake AppEvents tree under a throwaway key
    fn fake_root(tag: &str) -> String {
        let root = format!(
            r"Software\EraSwitchTests\sounds-{tag}-{}",
            std::process::id()
        );
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(&root);
        let set = |path: &str, val: &str| {
            let full = if path.is_empty() {
                root.clone()
            } else {
                format!(r"{root}\{path}")
            };
            let (k, _) = hkcu.create_subkey(full).unwrap();
            k.set_value("", &val.to_string()).unwrap();
        };
        set("", ".Default");
        set(r"Names\.Default", "Windows Default");
        set(r"Names\XP", "Windows XP");
        set(r"Apps\.Default\SystemExit\.Default", "exit10.wav");
        set(r"Apps\.Default\SystemExit\.Current", "exit10.wav");
        set(r"Apps\.Default\SystemExit\XP", "xpexit.wav");
        set(r"Apps\.Default\Notify\.Default", "notify.wav");
        // Notify has no .Current at all
        root
    }

    fn current_of(root: &str, event: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(format!(r"{root}\Apps\{event}\.Current"))
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
    }

    #[test]
    fn apply_and_restore_against_a_fake_tree() {
        let root = fake_root("roundtrip");
        let saved = snapshot_at(&root).unwrap();
        assert_eq!(saved.scheme, ".Default");
        assert_eq!(saved.current.len(), 1);

        apply_at(&root, "XP").unwrap();
        assert_eq!(
            current_of(&root, r".Default\SystemExit").as_deref(),
            Some("xpexit.wav")
        );
        assert_eq!(current_of(&root, r".Default\Notify").as_deref(), Some(""));

        restore_at(&root, &saved).unwrap();
        assert_eq!(
            current_of(&root, r".Default\SystemExit").as_deref(),
            Some("exit10.wav")
        );
        assert_eq!(current_of(&root, r".Default\Notify"), None);
        let after = snapshot_at(&root).unwrap();
        assert_eq!(after, saved);

        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }

    #[test]
    fn unknown_scheme_is_refused() {
        let root = fake_root("unknown");
        assert!(apply_at(&root, "Nope").is_err());
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }

    #[test]
    fn lists_schemes_with_display_names() {
        let root = fake_root("list");
        let mut got = list_schemes_at(&root);
        got.sort();
        assert_eq!(
            got,
            vec![
                (".Default".to_string(), "Windows Default".to_string()),
                ("XP".to_string(), "Windows XP".to_string())
            ]
        );
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&root);
    }
}
