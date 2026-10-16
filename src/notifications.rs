// notifications.rs
// Windows toasts. the AppUserModelID registered in startup.rs is what makes
// them show EraSwitch's name instead of a blank sender.

use anyhow::Result;
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

use crate::config::NotificationsConfig;
use crate::startup::AUMID;

pub fn show_toast(title: &str, body: &str) -> Result<()> {
    let xml_str = format!(
        r#"<toast><visual><binding template="ToastGeneric"><text>{}</text><text>{}</text></binding></visual></toast>"#,
        escape_xml(title),
        escape_xml(body)
    );
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(xml_str))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)?;
    Ok(())
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn toast(cfg: &NotificationsConfig, title: &str, body: &str) {
    if !cfg.enabled {
        return;
    }
    if let Err(e) = show_toast(title, body) {
        crate::dlog!("toast failed: {e}");
    }
}

pub fn update_available(version: &str, cfg: &NotificationsConfig) {
    if cfg.on_update {
        toast(
            cfg,
            "EraSwitch update",
            &format!("Version {version} is available. Open Settings to update."),
        );
    }
}

pub fn up_to_date(cfg: &NotificationsConfig) {
    if cfg.on_update {
        toast(
            cfg,
            "EraSwitch",
            &format!("You're up to date (v{}).", env!("CARGO_PKG_VERSION")),
        );
    }
}

pub fn hotkey_failed(hotkey: &str, cfg: &NotificationsConfig) {
    if cfg.on_hotkey_fail {
        toast(
            cfg,
            "EraSwitch",
            &format!("Couldn't register the hotkey {hotkey}. Pick another in Settings."),
        );
    }
}

// shown whatever the settings say: failed switches (you need to know your
// desktop didn't switch, or didn't switch back) and things you just asked for
pub fn always(title: &str, body: &str) {
    if let Err(e) = show_toast(title, body) {
        crate::dlog!("toast failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_is_escaped() {
        assert_eq!(escape_xml(r#"<a & "b">"#), "&lt;a &amp; &quot;b&quot;&gt;");
    }
}
