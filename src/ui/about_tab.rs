// ui/about_tab.rs
// version, build info and links

use egui::Ui;

use super::SettingsApp;

const BUILD_TARGET: &str = if cfg!(target_arch = "x86_64") {
    "x64"
} else if cfg!(target_arch = "aarch64") {
    "ARM64"
} else if cfg!(target_arch = "x86") {
    "x86"
} else {
    "unknown"
};

const BUILD_DATE: &str = env!("BUILD_DATE");
const REPO: &str = "https://github.com/Londopy/EraSwitch";

impl SettingsApp {
    pub fn about_tab(&mut self, ui: &mut Ui) {
        ui.heading("EraSwitch");
        ui.label(format!(
            "Version {} ({BUILD_TARGET}), built {BUILD_DATE}",
            env!("CARGO_PKG_VERSION")
        ));
        ui.add_space(6.0);
        ui.label(
            "Switches your Windows 11 desktop to an older look and back with one click, \
             by driving RetroBar and Open-Shell and remembering what to put back.",
        );
        ui.add_space(6.0);
        ui.hyperlink_to("GitHub repository", REPO);
        ui.hyperlink_to("Report a problem", format!("{REPO}/issues"));
        ui.hyperlink_to("Releases", format!("{REPO}/releases"));
        ui.add_space(6.0);
        ui.separator();
        ui.label("MIT for personal and open-source use; companies need a commercial license.");
        ui.label("Taskbar by RetroBar (dremin), Start menu by Open-Shell.");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Copy debug info").clicked() {
                let info = format!(
                    "EraSwitch v{} ({BUILD_TARGET}) built {BUILD_DATE}\nData: {}",
                    env!("CARGO_PKG_VERSION"),
                    crate::config::data_dir().display()
                );
                ui.output_mut(|o| o.copied_text = info);
            }
            if ui.button("Open data folder").clicked() {
                let _ = std::process::Command::new("explorer")
                    .arg(crate::config::data_dir())
                    .spawn();
            }
            if ui.button("Restart").clicked() {
                let _ = self.cmd_tx.send(crate::Cmd::Restart);
            }
            if ui.button("Exit").clicked() {
                let _ = self.cmd_tx.send(crate::Cmd::Exit);
            }
        });
    }
}
