// ui/updater_tab.rs
// update settings, Check Now, and Download & Install (never automatic)

use egui::{Color32, Ui};

use super::SettingsApp;
use crate::updater::UpdateEvent;

impl SettingsApp {
    pub fn updater_tab(&mut self, ui: &mut Ui) {
        ui.heading("Updates");
        ui.add_space(6.0);

        if ui
            .checkbox(
                &mut self.config.updater.enabled,
                "Enable automatic update checks",
            )
            .changed()
        {
            self.dirty = true;
        }
        ui.add_enabled_ui(self.config.updater.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.label("Channel:");
                let before = self.config.updater.channel.clone();
                let shown = if before == "beta" {
                    "Beta (pre-releases)"
                } else {
                    "Stable"
                };
                egui::ComboBox::from_id_salt("channel")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.updater.channel,
                            "stable".into(),
                            "Stable",
                        );
                        ui.selectable_value(
                            &mut self.config.updater.channel,
                            "beta".into(),
                            "Beta (pre-releases)",
                        );
                    });
                if self.config.updater.channel != before {
                    self.dirty = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Check every:");
                if ui
                    .add(
                        egui::Slider::new(&mut self.config.updater.check_interval_h, 1..=168)
                            .suffix(" h"),
                    )
                    .changed()
                {
                    self.dirty = true;
                }
            });
        });

        ui.add_space(6.0);
        ui.separator();
        ui.label(format!("Current version: v{}", env!("CARGO_PKG_VERSION")));

        if let Some(rx) = &self.update_check_rx {
            if let Ok(result) = rx.try_recv() {
                self.update_check_rx = None;
                self.update_status = Some(match result {
                    Ok(Some(v)) => {
                        let _ = self.cmd_tx.send(crate::Cmd::UpdateAvailable(v.clone()));
                        format!("Update available: v{v}")
                    }
                    Ok(None) => "You're up to date.".into(),
                    Err(e) => format!("Check failed: {e}"),
                });
            }
        }
        if let Some(rx) = &self.update_apply_rx {
            // success restarts the app, so only failures arrive here
            if let Ok(UpdateEvent::Failed(e)) = rx.try_recv() {
                self.update_apply_rx = None;
                self.update_status = Some(format!("Update failed: {e}"));
            }
        }

        if let Some(status) = &self.update_status {
            let colour = if status.starts_with("Update available") {
                Color32::YELLOW
            } else if status.contains("failed") {
                Color32::from_rgb(230, 90, 80)
            } else {
                Color32::from_rgb(90, 180, 80)
            };
            ui.colored_label(colour, status);
        }

        ui.horizontal(|ui| {
            let checking = self.update_check_rx.is_some();
            if ui
                .add_enabled(!checking, egui::Button::new("Check Now"))
                .clicked()
            {
                let channel = self.config.updater.channel.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                self.update_check_rx = Some(rx);
                std::thread::spawn(move || {
                    let _ = tx.send(
                        crate::updater::check_for_update(&channel).map_err(|e| format!("{e:#}")),
                    );
                });
                self.update_status = Some("Checking...".into());
            }
            let ready = self
                .update_status
                .as_deref()
                .is_some_and(|s| s.starts_with("Update available"));
            if ready && ui.button("Download & Install Update").clicked() {
                let (tx, rx) = std::sync::mpsc::channel();
                self.update_apply_rx = Some(rx);
                crate::updater::background_apply(self.config.updater.channel.clone(), tx);
                self.update_status = Some("Downloading update...".into());
            }
        });

        ui.add_space(8.0);
        ui.weak("Updates are verified with SHA-256 before applying.");
    }
}
