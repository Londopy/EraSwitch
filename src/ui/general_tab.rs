// ui/general_tab.rs
// the era's pieces, the helper programs, the hotkey and startup

use egui::{Color32, Ui};

use super::SettingsApp;
use crate::era::{self, RETROBAR_XP_THEMES};
use crate::tools::{self, Tool};

const MENU_STYLES: [(u32, &str); 3] = [
    (0, "Classic"),
    (1, "Classic with two columns (XP)"),
    (2, "Windows 7"),
];

impl SettingsApp {
    pub fn general_tab(&mut self, ui: &mut Ui) {
        self.poll_install();
        if let Some(s) = &self.status_line {
            ui.label(s);
            ui.add_space(4.0);
        }
        if self.state_shared.lock().unwrap().active {
            ui.weak("Changes to the era take effect the next time you switch to it.");
            ui.add_space(4.0);
        }

        self.era_section(ui);
        ui.add_space(8.0);
        ui.separator();
        self.tools_section(ui);
        ui.add_space(8.0);
        ui.separator();
        self.behaviour_section(ui);
    }

    fn era_section(&mut self, ui: &mut Ui) {
        ui.heading("Era");
        let e = &mut self.config.era;
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("Name:");
            changed |= ui.text_edit_singleline(&mut e.name).changed();
        });

        ui.add_space(4.0);
        changed |= ui
            .checkbox(&mut e.taskbar.enabled, "Taskbar (RetroBar)")
            .changed();
        ui.add_enabled_ui(e.taskbar.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label("Theme:");
                egui::ComboBox::from_id_salt("retrobar_theme")
                    .selected_text(e.taskbar.retrobar_theme.clone())
                    .show_ui(ui, |ui| {
                        for t in RETROBAR_XP_THEMES {
                            changed |= ui
                                .selectable_value(&mut e.taskbar.retrobar_theme, t.to_string(), *t)
                                .changed();
                        }
                    });
            });
        });

        changed |= ui
            .checkbox(&mut e.start_menu.enabled, "Start menu (Open-Shell)")
            .changed();
        ui.add_enabled_ui(e.start_menu.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label("Style:");
                let current = MENU_STYLES
                    .iter()
                    .find(|(v, _)| *v == e.start_menu.menu_style)
                    .map_or("?", |(_, l)| l);
                egui::ComboBox::from_id_salt("menu_style")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (v, l) in MENU_STYLES {
                            changed |= ui
                                .selectable_value(&mut e.start_menu.menu_style, v, l)
                                .changed();
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label("Skin:");
                changed |= ui.text_edit_singleline(&mut e.start_menu.skin).changed();
            });
        });

        changed |= ui
            .checkbox(&mut e.wallpaper.enabled, "Wallpaper")
            .on_hover_text("Bring your own picture: the classic XP ones are Microsoft's")
            .changed();
        ui.add_enabled_ui(e.wallpaper.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                changed |= ui.text_edit_singleline(&mut e.wallpaper.path).changed();
                if ui.button("Browse...").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Pictures", &["jpg", "jpeg", "png", "bmp"])
                        .pick_file()
                    {
                        e.wallpaper.path = p.to_string_lossy().into_owned();
                        changed = true;
                    }
                }
            });
        });

        changed |= ui
            .checkbox(&mut e.sounds.enabled, "Sounds")
            .on_hover_text(
                "Switches to a Windows sound scheme. Make one with your XP sounds in \
                 Control Panel > Sound, then Save As.",
            )
            .changed();
        ui.add_enabled_ui(e.sounds.enabled, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label("Scheme:");
                let shown = self
                    .sound_schemes
                    .iter()
                    .find(|(id, _)| *id == e.sounds.scheme)
                    .map_or(e.sounds.scheme.clone(), |(_, name)| name.clone());
                egui::ComboBox::from_id_salt("sound_scheme")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for (id, name) in &self.sound_schemes {
                            changed |= ui
                                .selectable_value(&mut e.sounds.scheme, id.clone(), name)
                                .changed();
                        }
                    });
                if ui.button("Refresh").clicked() {
                    self.sound_schemes = crate::sounds::list_schemes();
                }
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Save era as .londo...").clicked() {
                let name = format!("{}.londo", e.name);
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("EraSwitch era", &["londo"])
                    .set_file_name(name)
                    .save_file()
                {
                    self.status_line = Some(match era::save_londo(e, &p) {
                        Ok(()) => format!("Saved {}", p.display()),
                        Err(err) => format!("Couldn't save: {err:#}"),
                    });
                }
            }
            if ui.button("Open era...").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("EraSwitch era", &["londo"])
                    .pick_file()
                {
                    match era::open_londo(&p, &crate::config::data_dir()) {
                        Ok(loaded) => {
                            self.status_line = Some(format!("Loaded '{}'", loaded.name));
                            *e = loaded;
                            changed = true;
                        }
                        Err(err) => self.status_line = Some(format!("Couldn't open: {err:#}")),
                    }
                }
            }
            if ui.button("Reset to Windows XP").clicked() {
                *e = era::Era::default();
                changed = true;
            }
        });

        if changed {
            self.dirty = true;
        }
    }

    fn tools_section(&mut self, ui: &mut Ui) {
        ui.heading("Helper programs");
        ui.weak("EraSwitch drives these; it doesn't replace them.");
        let installing = self.install_rx.is_some();
        for tool in [Tool::RetroBar, Tool::OpenShell] {
            ui.horizontal(|ui| {
                ui.label(format!("{}:", tool.label()));
                match tools::locate(tool, &self.config.tools) {
                    Some(p) => {
                        ui.colored_label(Color32::from_rgb(90, 180, 80), "found");
                        ui.weak(p.display().to_string());
                    }
                    None => {
                        ui.colored_label(Color32::from_rgb(230, 90, 80), "not found");
                        if ui
                            .add_enabled(!installing, egui::Button::new("Install with winget"))
                            .clicked()
                        {
                            self.start_install(tool);
                        }
                        ui.hyperlink_to("website", tool.homepage());
                    }
                }
                if ui.button("Browse...").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Program", &["exe"])
                        .pick_file()
                    {
                        let s = p.to_string_lossy().into_owned();
                        match tool {
                            Tool::RetroBar => self.config.tools.retrobar = s,
                            Tool::OpenShell => self.config.tools.openshell = s,
                        }
                        self.dirty = true;
                    }
                }
            });
        }
        if installing {
            ui.label(
                "Installing... (watch the console window; the installer may ask for admin rights)",
            );
        }
    }

    fn start_install(&mut self, tool: Tool) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.install_rx = Some(rx);
        std::thread::spawn(move || {
            let msg = match tools::winget_install(tool) {
                Ok(()) => format!("{} installed.", tool.label()),
                Err(e) => format!("Installing {} failed: {e:#}", tool.label()),
            };
            let _ = tx.send(msg);
        });
    }

    fn poll_install(&mut self) {
        if let Some(rx) = &self.install_rx {
            if let Ok(msg) = rx.try_recv() {
                self.status_line = Some(msg);
                self.install_rx = None;
            }
        }
    }

    fn behaviour_section(&mut self, ui: &mut Ui) {
        ui.heading("Behaviour");
        ui.horizontal(|ui| {
            ui.label("Hotkey:");
            let r = ui.text_edit_singleline(&mut self.config.hotkeys.toggle);
            let valid = crate::hotkeys::parse_hotkey(&self.config.hotkeys.toggle).is_ok();
            if valid {
                ui.weak(crate::hotkeys::display_hotkey(&self.config.hotkeys.toggle));
            } else {
                ui.colored_label(Color32::from_rgb(230, 90, 80), "like ctrl+alt+e");
            }
            // only register it once it's whole, not on every keystroke
            if r.lost_focus() && valid {
                self.dirty = true;
            }
        });
        if ui
            .checkbox(
                &mut self.config.startup.enabled,
                "Start EraSwitch when I sign in",
            )
            .changed()
        {
            self.dirty = true;
        }
        if ui
            .checkbox(&mut self.config.notifications.enabled, "Show notifications")
            .changed()
        {
            self.dirty = true;
        }
        ui.add_enabled_ui(self.config.notifications.enabled, |ui| {
            if ui
                .checkbox(
                    &mut self.config.notifications.on_update,
                    "When an update is available",
                )
                .changed()
            {
                self.dirty = true;
            }
            if ui
                .checkbox(
                    &mut self.config.notifications.on_hotkey_fail,
                    "When the hotkey can't be registered",
                )
                .changed()
            {
                self.dirty = true;
            }
        });
    }
}
