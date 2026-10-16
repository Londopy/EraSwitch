// hotkeys.rs
// the one global hotkey (default Ctrl+Alt+E) that flips the era on and off.
// parsing is the same as HideDesktopApps' so hotkey strings mean the same thing.

use anyhow::{bail, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use std::sync::mpsc::Sender;

use crate::config::DEFAULT_HOTKEY;
use crate::Cmd;

// "ctrl+alt+e" -> HotKey. needs at least one modifier and one key
pub fn parse_hotkey(s: &str) -> Result<HotKey> {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.len() < 2 {
        bail!("Hotkey '{s}' needs at least one modifier and one key");
    }
    let key_part = parts.last().unwrap().trim().to_lowercase();
    let mut modifiers = Modifiers::empty();
    for m in &parts[..parts.len() - 1] {
        match m.trim().to_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            "win" | "super" | "meta" => modifiers |= Modifiers::SUPER,
            other => bail!("Unknown modifier: '{other}'"),
        }
    }
    Ok(HotKey::new(Some(modifiers), parse_key_code(&key_part)?))
}

// "ctrl+alt+e" -> "Ctrl+Alt+E" for menus and messages. an invalid hotkey
// shows the fallback, since that's what actually gets registered
pub fn display_hotkey(s: &str) -> String {
    let s = if parse_hotkey(s).is_ok() {
        s
    } else {
        DEFAULT_HOTKEY
    };
    s.split('+')
        .map(|part| match part.trim().to_lowercase().as_str() {
            "ctrl" | "control" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "win" | "super" | "meta" => "Win".to_string(),
            key => key.trim_start_matches("digit").to_uppercase(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn parse_key_code(key: &str) -> Result<Code> {
    let letters = [
        Code::KeyA,
        Code::KeyB,
        Code::KeyC,
        Code::KeyD,
        Code::KeyE,
        Code::KeyF,
        Code::KeyG,
        Code::KeyH,
        Code::KeyI,
        Code::KeyJ,
        Code::KeyK,
        Code::KeyL,
        Code::KeyM,
        Code::KeyN,
        Code::KeyO,
        Code::KeyP,
        Code::KeyQ,
        Code::KeyR,
        Code::KeyS,
        Code::KeyT,
        Code::KeyU,
        Code::KeyV,
        Code::KeyW,
        Code::KeyX,
        Code::KeyY,
        Code::KeyZ,
    ];
    let digits = [
        Code::Digit0,
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    let fkeys = [
        Code::F1,
        Code::F2,
        Code::F3,
        Code::F4,
        Code::F5,
        Code::F6,
        Code::F7,
        Code::F8,
        Code::F9,
        Code::F10,
        Code::F11,
        Code::F12,
    ];
    let key = key.trim_start_matches("digit");
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_lowercase() => return Ok(letters[(c as u8 - b'a') as usize]),
        (Some(c), None) if c.is_ascii_digit() => return Ok(digits[(c as u8 - b'0') as usize]),
        _ => {}
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
        if (1..=12).contains(&n) {
            return Ok(fkeys[n - 1]);
        }
    }
    bail!("Unknown key: '{key}'")
}

pub struct Registered {
    pub manager: GlobalHotKeyManager,
    pub toggle_id: u32,
    toggle: HotKey,
}

fn parse_or_default(s: &str, cmd_tx: &Sender<Cmd>) -> HotKey {
    parse_hotkey(s).unwrap_or_else(|e| {
        crate::dlog!("hotkey '{s}' didn't parse ({e}); using {DEFAULT_HOTKEY}");
        let _ = cmd_tx.send(Cmd::HotkeyFailed(s.to_string()));
        parse_hotkey(DEFAULT_HOTKEY).expect("default hotkey parses")
    })
}

pub fn register(toggle: &str, cmd_tx: &Sender<Cmd>) -> Result<Registered> {
    let manager = GlobalHotKeyManager::new()?;
    let hk = parse_or_default(toggle, cmd_tx);
    if let Err(e) = manager.register(hk) {
        crate::dlog!("registering hotkey '{toggle}' failed: {e}");
        let _ = cmd_tx.send(Cmd::HotkeyFailed(toggle.to_string()));
    }
    Ok(Registered {
        manager,
        toggle_id: hk.id(),
        toggle: hk,
    })
}

pub fn reregister(reg: &mut Registered, toggle: &str, cmd_tx: &Sender<Cmd>) {
    let _ = reg.manager.unregister(reg.toggle);
    let hk = parse_or_default(toggle, cmd_tx);
    if let Err(e) = reg.manager.register(hk) {
        crate::dlog!("re-registering hotkey '{toggle}' failed: {e}");
        let _ = cmd_tx.send(Cmd::HotkeyFailed(toggle.to_string()));
    }
    reg.toggle_id = hk.id();
    reg.toggle = hk;
}

pub fn poll() -> Option<GlobalHotKeyEvent> {
    GlobalHotKeyEvent::receiver().try_recv().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        assert!(parse_hotkey("ctrl+alt+e").is_ok());
        assert!(parse_hotkey("win+shift+f12").is_ok());
        assert!(parse_hotkey("ctrl+digit7").is_ok());
    }

    #[test]
    fn rejects_bad_hotkeys() {
        assert!(parse_hotkey("e").is_err());
        assert!(parse_hotkey("ctrl+alt").is_err());
        assert!(parse_hotkey("foo+e").is_err());
        assert!(parse_hotkey("ctrl+f13").is_err());
        assert!(parse_hotkey("ctrl+enter").is_err());
    }

    #[test]
    fn case_and_aliases_match() {
        let a = parse_hotkey("ctrl+alt+e").unwrap();
        let b = parse_hotkey("CONTROL + Alt + E").unwrap();
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn display_is_for_people() {
        assert_eq!(display_hotkey("ctrl+alt+e"), "Ctrl+Alt+E");
        assert_eq!(display_hotkey("super+digit1"), "Win+1");
        assert_eq!(display_hotkey("nonsense"), "Ctrl+Alt+E");
    }
}
