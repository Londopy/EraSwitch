// tray.rs
// tray icon and menu. left click flips the era; the icon is drawn here so the
// app needs no image files: blue for Windows 11, XP green while the era is on.

use anyhow::Result;
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};

use crate::era::Preset;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

pub const ICON_SIZE: usize = 32;

const WIN11_BLUE: [u8; 4] = [0, 103, 192, 255];
const XP_GREEN: [u8; 4] = [60, 141, 47, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

pub struct MenuIds {
    pub toggle: MenuId,
    pub update: MenuId,
    pub settings: MenuId,
    pub restart: MenuId,
    pub exit: MenuId,
    // the Era submenu: one entry per built-in era
    pub eras: Vec<(MenuId, Preset)>,
}

pub struct TrayHandle {
    pub tray: TrayIcon,
    pub ids: MenuIds,
    toggle_item: MenuItem,
    update_item: MenuItem,
    era_items: Vec<(CheckMenuItem, Preset)>,
}

// what the tray shows; main.rs keeps this in step with reality
pub struct TrayState<'a> {
    pub active: bool,
    pub era_name: &'a str,
    pub hotkey: &'a str,
    pub update: Option<&'a str>,
}

fn toggle_label(s: &TrayState) -> String {
    let what = if s.active {
        "Back to Windows 11".to_string()
    } else {
        format!("Switch to {}", s.era_name)
    };
    format!("{what}\t{}", crate::hotkeys::display_hotkey(s.hotkey))
}

fn tooltip(s: &TrayState) -> String {
    if s.active {
        format!("EraSwitch: {} is on", s.era_name)
    } else {
        "EraSwitch: Windows 11".to_string()
    }
}

// rounded square in the mode's colour with a white "E" on it
pub fn icon_rgba(active: bool) -> Vec<u8> {
    let n = ICON_SIZE;
    let bg = if active { XP_GREEN } else { WIN11_BLUE };
    let mut buf = vec![0u8; n * n * 4];
    let r = 6isize; // corner radius
    for y in 0..n {
        for x in 0..n {
            let (xi, yi, last) = (x as isize, y as isize, (n - 1) as isize);
            // distance to the nearest corner centre, only inside the corner boxes
            let cx = if xi < r {
                r
            } else if xi > last - r {
                last - r
            } else {
                xi
            };
            let cy = if yi < r {
                r
            } else if yi > last - r {
                last - r
            } else {
                yi
            };
            let (dx, dy) = (xi - cx, yi - cy);
            if dx * dx + dy * dy <= r * r {
                let i = (y * n + x) * 4;
                buf[i..i + 4].copy_from_slice(&bg);
            }
        }
    }
    // the E: a spine and three arms
    let mut fill = |x0: usize, y0: usize, w: usize, h: usize| {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let i = (y * n + x) * 4;
                buf[i..i + 4].copy_from_slice(&WHITE);
            }
        }
    };
    fill(9, 7, 4, 18);
    fill(9, 7, 14, 4);
    fill(9, 14, 11, 4);
    fill(9, 21, 14, 4);
    buf
}

fn icon(active: bool) -> Result<Icon> {
    Ok(Icon::from_rgba(
        icon_rgba(active),
        ICON_SIZE as u32,
        ICON_SIZE as u32,
    )?)
}

pub fn build(s: &TrayState) -> Result<TrayHandle> {
    let toggle_item = MenuItem::new(toggle_label(s), true, None);
    let update_item = MenuItem::new("Update available", false, None);
    let settings = MenuItem::new("Settings...", true, None);
    let restart = MenuItem::new("Restart", true, None);
    let exit = MenuItem::new("Exit", true, None);

    // pick an era; ticked when the current one is that preset
    let era_menu = Submenu::new("Era", true);
    let era_items: Vec<(CheckMenuItem, Preset)> = Preset::ALL
        .into_iter()
        .map(|p| (CheckMenuItem::new(p.name(), true, false, None), p))
        .collect();
    for (item, _) in &era_items {
        era_menu.append(item)?;
    }

    let menu = Menu::new();
    menu.append(&toggle_item)?;
    menu.append(&era_menu)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&update_item)?;
    menu.append(&settings)?;
    menu.append(&restart)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&exit)?;

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(tooltip(s))
        .with_icon(icon(s.active)?)
        // left click toggles; the menu stays on right click
        .with_menu_on_left_click(false)
        .build()?;

    let ids = MenuIds {
        toggle: toggle_item.id().clone(),
        update: update_item.id().clone(),
        settings: settings.id().clone(),
        restart: restart.id().clone(),
        exit: exit.id().clone(),
        eras: era_items
            .iter()
            .map(|(i, p)| (i.id().clone(), *p))
            .collect(),
    };
    let handle = TrayHandle {
        tray,
        ids,
        toggle_item,
        update_item,
        era_items,
    };
    update(&handle, s);
    Ok(handle)
}

pub fn update(h: &TrayHandle, s: &TrayState) {
    h.toggle_item.set_text(toggle_label(s));
    for (item, preset) in &h.era_items {
        item.set_checked(preset.name() == s.era_name);
    }
    match s.update {
        Some(v) => {
            h.update_item.set_text(format!("Update to v{v}..."));
            h.update_item.set_enabled(true);
        }
        None => {
            h.update_item.set_text("No update available");
            h.update_item.set_enabled(false);
        }
    }
    let _ = h.tray.set_tooltip(Some(tooltip(s)));
    if let Ok(i) = icon(s.active) {
        let _ = h.tray.set_icon(Some(i));
    }
}

pub fn poll_menu() -> Option<MenuEvent> {
    MenuEvent::receiver().try_recv().ok()
}

pub fn poll_tray() -> Option<TrayIconEvent> {
    TrayIconEvent::receiver().try_recv().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_the_right_size_and_colours() {
        let off = icon_rgba(false);
        let on = icon_rgba(true);
        assert_eq!(off.len(), ICON_SIZE * ICON_SIZE * 4);
        // a pixel inside the square but off the E
        let i = (16 * ICON_SIZE + 28) * 4;
        assert_eq!(&off[i..i + 4], &WIN11_BLUE);
        assert_eq!(&on[i..i + 4], &XP_GREEN);
        // the very corner is transparent (rounded)
        assert_eq!(off[3], 0);
    }

    #[test]
    fn labels_follow_state() {
        let mut s = TrayState {
            active: false,
            era_name: "Windows XP",
            hotkey: "ctrl+alt+e",
            update: None,
        };
        assert_eq!(toggle_label(&s), "Switch to Windows XP\tCtrl+Alt+E");
        assert_eq!(tooltip(&s), "EraSwitch: Windows 11");
        s.active = true;
        assert_eq!(toggle_label(&s), "Back to Windows 11\tCtrl+Alt+E");
        assert_eq!(tooltip(&s), "EraSwitch: Windows XP is on");
    }
}
