// ui/mod.rs
// the settings window: an eframe app on its own thread. closing it only
// minimizes, so reopening from the tray is instant.

mod about_tab;
mod general_tab;
mod updater_tab;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use crate::config::AppConfig;
use crate::{AppState, Cmd};

static SETTINGS_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);
static SETTINGS_SHOW: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Updater,
    About,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::General, Tab::Updater, Tab::About];
    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Updater => "Updater",
            Tab::About => "About",
        }
    }
}

pub struct SettingsApp {
    pub config: AppConfig,
    pub config_shared: Arc<Mutex<AppConfig>>,
    pub state_shared: Arc<Mutex<AppState>>,
    pub cmd_tx: mpsc::Sender<Cmd>,
    pub tab: Tab,
    // set by any tab that changed config; saved at the end of the frame
    pub dirty: bool,
    pub update_status: Option<String>,
    pub update_check_rx: Option<mpsc::Receiver<Result<Option<String>, String>>>,
    pub update_apply_rx: Option<mpsc::Receiver<crate::updater::UpdateEvent>>,
    pub install_rx: Option<mpsc::Receiver<String>>,
    pub status_line: Option<String>,
    pub sound_schemes: Vec<(String, String)>,
    // the era as the main loop last had it
    seen_era: crate::era::Era,
}

impl SettingsApp {
    fn new(
        config_shared: Arc<Mutex<AppConfig>>,
        state_shared: Arc<Mutex<AppState>>,
        cmd_tx: mpsc::Sender<Cmd>,
    ) -> Self {
        let config = config_shared.lock().unwrap().clone();
        Self {
            seen_era: config.era.clone(),
            config,
            config_shared,
            state_shared,
            cmd_tx,
            tab: Tab::General,
            dirty: false,
            update_status: None,
            update_check_rx: None,
            update_apply_rx: None,
            install_rx: None,
            status_line: None,
            sound_schemes: crate::sounds::list_schemes(),
        }
    }

    // write config.toml and hand the new config to the main loop
    fn save_now(&mut self) {
        if let Err(e) = crate::config::save_config(&self.config) {
            self.status_line = Some(format!("Couldn't save settings: {e:#}"));
        }
        // the main loop owns config_shared: it compares old and new to decide
        // what to redo (hotkey, startup), so it has to be the one writing it
        let _ = self
            .cmd_tx
            .send(Cmd::ConfigUpdated(Box::new(self.config.clone())));
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if SETTINGS_SHOW.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        // an era opened from Explorer while the window is open. only a change
        // in the main loop's copy counts, so our own unsent edits aren't undone
        {
            let shared = self.config_shared.lock().unwrap();
            if shared.era != self.seen_era {
                self.seen_era = shared.era.clone();
                if !self.dirty {
                    self.config.era = shared.era.clone();
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let (active, era_name) = {
                let st = self.state_shared.lock().unwrap();
                (st.active, self.config.era.name.clone())
            };
            ui.horizontal(|ui| {
                ui.label("Now:");
                if active {
                    ui.colored_label(
                        egui::Color32::from_rgb(90, 180, 80),
                        format!("{era_name} is on"),
                    );
                } else {
                    ui.label("Windows 11");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if active {
                        "Back to Windows 11".to_string()
                    } else {
                        format!("Switch to {era_name}")
                    };
                    if ui.button(label).clicked() {
                        let _ = self.cmd_tx.send(Cmd::Toggle);
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.tab, tab, tab.label());
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::General => self.general_tab(ui),
                Tab::Updater => self.updater_tab(ui),
                Tab::About => self.about_tab(ui),
            });
        });

        if self.dirty {
            self.dirty = false;
            self.save_now();
        }
    }
}

// open the settings window, or bring back the one already running
pub fn open_settings(
    config_shared: Arc<Mutex<AppConfig>>,
    state_shared: Arc<Mutex<AppState>>,
    cmd_tx: mpsc::Sender<Cmd>,
) {
    if let Some(ctx) = SETTINGS_CTX.lock().unwrap().clone() {
        SETTINGS_SHOW.store(true, Ordering::SeqCst);
        ctx.request_repaint();
        return;
    }

    std::thread::spawn(move || {
        // forget the context when this thread ends, however it ends
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                *SETTINGS_CTX.lock().unwrap() = None;
                SETTINGS_SHOW.store(false, Ordering::SeqCst);
            }
        }
        let _guard = Guard;

        let size = crate::tray::ICON_SIZE as u32;
        let icon = Arc::new(egui::IconData {
            rgba: crate::tray::icon_rgba(false),
            width: size,
            height: size,
        });
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("EraSwitch Settings")
                .with_inner_size([560.0, 560.0])
                .with_icon(icon)
                .with_taskbar(false),
            event_loop_builder: Some(Box::new(|builder| {
                use winit::platform::windows::EventLoopBuilderExtWindows;
                builder.with_any_thread(true);
            })),
            ..Default::default()
        };
        let result = eframe::run_native(
            "EraSwitch Settings",
            options,
            Box::new(move |cc| {
                *SETTINGS_CTX.lock().unwrap() = Some(cc.egui_ctx.clone());
                Ok(Box::new(SettingsApp::new(
                    config_shared,
                    state_shared,
                    cmd_tx,
                )))
            }),
        );
        if let Err(e) = result {
            crate::dlog!("settings window error: {e}");
        }
    });
}
