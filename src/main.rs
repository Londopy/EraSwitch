// EraSwitch: one click between Windows 11 and an older look (Windows XP by default).
// tray app; RetroBar draws the taskbar, Open-Shell the Start menu, and
// switch.rs remembers what to put back.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod config;
mod era;
mod hotkeys;
#[macro_use]
mod log_util;
mod londopy;
mod notifications;
mod openshell;
mod retrobar;
mod sounds;
mod startup;
mod switch;
mod tools;
mod tray;
mod ui;
mod updater;
mod win_ops;

use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use config::AppConfig;

// commands for the main loop, from the tray, the hotkey, settings and workers
pub enum Cmd {
    Toggle,
    PickEra(era::Preset),
    SwitchDone(Result<bool, String>),
    OpenSettings,
    ConfigUpdated(Box<AppConfig>),
    OpenLondopy(PathBuf),
    UpdateAvailable(String),
    UpToDate,
    UpdateFailed(String),
    HotkeyFailed(String),
    Restart,
    Exit,
}

// what the tray and settings window show
#[derive(Debug, Default)]
pub struct AppState {
    pub active: bool,
    // a switch is running on its worker thread
    pub busy: bool,
    pub update: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // the uninstaller (and anyone stuck) can put Windows 11 back without the tray
    if args.iter().any(|a| a == "--restore") {
        if let Err(e) = switch::restore(&config::data_dir()) {
            dlog!("--restore failed: {e:#}");
            // no console in a GUI exe, so say it where it'll be seen
            rfd::MessageDialog::new()
                .set_title("EraSwitch")
                .set_description(format!("Couldn't put Windows 11 back:\n{e:#}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
            std::process::exit(1);
        }
        return;
    }

    // the installer's "helper programs" page: `--install-helpers retrobar,openshell`
    if let Some(i) = args.iter().position(|a| a == "--install-helpers") {
        let wanted = args
            .get(i + 1)
            .map(|s| tools::parse_list(s))
            .unwrap_or_default();
        let cfg = config::load_config().unwrap_or_default();
        let (ok, lines) = tools::install(&wanted, &cfg.tools, &config::data_dir());
        // the installer offered them, so don't offer again at first launch
        let mut cfg = cfg;
        cfg.tools.offered = true;
        let _ = config::save_config(&cfg);
        rfd::MessageDialog::new()
            .set_title("EraSwitch setup")
            .set_description(install_summary(ok, &lines))
            .set_level(if ok {
                rfd::MessageLevel::Info
            } else {
                rfd::MessageLevel::Warning
            })
            .show();
        std::process::exit(if ok { 0 } else { 1 });
    }

    // .londopy first, before the single-instance check
    let mut open_now: Option<PathBuf> = None;
    match londopy::parse_args(&args) {
        Some(londopy::LondopyArg::Unregister) => {
            let _ = londopy::unregister_self();
            return;
        }
        Some(londopy::LondopyArg::Open(path)) => match londopy::dispatch(&path) {
            Ok(londopy::Dispatch::Mine(_)) => open_now = Some(path),
            Ok(londopy::Dispatch::Forwarded { .. }) => return,
            Ok(londopy::Dispatch::NotInstalled { app, homepage }) => {
                not_installed_dialog(&app, &homepage);
                return;
            }
            Err(e) => {
                rfd::MessageDialog::new()
                    .set_title("EraSwitch")
                    .set_description(format!("Couldn't open that file:\n{e:#}"))
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                return;
            }
        },
        None => {}
    }

    let restarted = args.iter().any(|a| a == "--restarted");
    let Some(_instance) = win_ops::acquire_single_instance(restarted) else {
        // already running: hand it the file, if there is one, and quit
        if let Some(p) = &open_now {
            let _ = londopy::send_to_inbox(&config::data_dir(), p);
        }
        return;
    };

    // a crash must not strand you in the era
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        dlog!("PANIC: {info}");
        // never waits: the panicking thread may be the one mid-switch
        switch::try_restore(&config::data_dir());
        default_hook(info);
    }));

    if let Err(e) = run(open_now) {
        dlog!("fatal: {e:#}");
        let _ = switch::restore(&config::data_dir());
        std::process::exit(1);
    }
}

fn not_installed_dialog(app: &str, homepage: &str) {
    let mut d = rfd::MessageDialog::new()
        .set_title("EraSwitch")
        .set_description(format!(
            "This file belongs to {app}, which isn't installed."
        ));
    if homepage.starts_with("https://") {
        d = d.set_buttons(rfd::MessageButtons::OkCancelCustom(
            "Open download page".into(),
            "Close".into(),
        ));
        if d.show() == rfd::MessageDialogResult::Custom("Open download page".into()) {
            let _ = std::process::Command::new("explorer").arg(homepage).spawn();
        }
    } else {
        d.show();
    }
}

enum Switch {
    On,
    Off,
    Across,
}

// runs a switch off the main thread (it takes a few seconds and the tray
// should stay responsive), always reporting back with SwitchDone
fn spawn_switch(kind: Switch, cfg: AppConfig, dir: PathBuf, tx: mpsc::Sender<Cmd>) {
    std::thread::spawn(move || {
        // a panic in here must still report back, or `busy` would stay set
        // and the toggle would never work again
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match kind {
            Switch::On => switch::apply(&cfg, &dir),
            Switch::Off => switch::restore(&dir),
            Switch::Across => switch::switch_across(&cfg, &dir),
        }));
        let done = match r {
            Ok(Ok(())) => Ok(switch::is_active(&dir)),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(_) => Err("The switch crashed; see debug.log".to_string()),
        };
        let _ = tx.send(Cmd::SwitchDone(done));
    });
}

fn install_summary(ok: bool, lines: &[String]) -> String {
    let head = if ok {
        "The helper programs are ready. Click the EraSwitch tray icon (or press Ctrl+Alt+E) to switch."
    } else {
        "Not everything could be installed. You can retry from EraSwitch's Settings."
    };
    format!("{head}\n\n{}", lines.join("\n"))
}

// "EraSwitch needs RetroBar and Open-Shell: install them now?", on its own
// thread so the tray keeps working. one offer at a time.
fn offer_helpers(
    missing: Vec<tools::Tool>,
    tools_cfg: config::ToolsConfig,
    data_dir: PathBuf,
    first_run: bool,
) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static OFFERING: AtomicBool = AtomicBool::new(false);
    if OFFERING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let names = missing
            .iter()
            .map(|t| t.label())
            .collect::<Vec<_>>()
            .join(" and ");
        let why = if first_run {
            format!("Welcome to EraSwitch! It draws the older Windows looks with {names}, which aren't installed yet.")
        } else {
            format!("Switching needs {names}, which aren't installed yet.")
        };

        if !tools::winget_available() {
            // no winget to install with: point at the download pages
            let open = rfd::MessageDialog::new()
                .set_title("EraSwitch")
                .set_description(format!("{why}\n\nwinget isn't available on this PC, so install them from their download pages, then click the tray icon again."))
                .set_buttons(rfd::MessageButtons::OkCancelCustom("Open download pages".into(), "Later".into()))
                .show();
            if open == rfd::MessageDialogResult::Custom("Open download pages".into()) {
                for t in &missing {
                    let _ = std::process::Command::new("explorer")
                        .arg(t.homepage())
                        .spawn();
                }
            }
            OFFERING.store(false, Ordering::SeqCst);
            return;
        }

        let yes = rfd::MessageDialog::new()
            .set_title("EraSwitch")
            .set_description(format!(
                "{why}\n\nInstall them now with winget? A console window shows the progress, and their installers may ask for admin rights."
            ))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes;
        if yes {
            let (ok, lines) = tools::install(&missing, &tools_cfg, &data_dir);
            notifications::always(
                if ok {
                    "EraSwitch is ready"
                } else {
                    "EraSwitch setup"
                },
                &install_summary(ok, &lines),
            );
        }
        OFFERING.store(false, Ordering::SeqCst);
    });
}

fn tray_state<'a>(state: &'a AppState, cfg: &'a AppConfig) -> tray::TrayState<'a> {
    tray::TrayState {
        active: state.active,
        era_name: &cfg.era.name,
        preset: era::Preset::of(&cfg.era),
        hotkey: &cfg.hotkeys.toggle,
        update: state.update.as_deref(),
    }
}

fn run(open_now: Option<PathBuf>) -> Result<()> {
    startup::setup_aumid();
    dlog!("--- EraSwitch {} starting ---", env!("CARGO_PKG_VERSION"));
    let data_dir = config::data_dir();

    let mut config = config::load_config()?;
    if !config::is_portable() {
        if let Err(e) = londopy::register_self() {
            dlog!("londopy register failed: {e:#}");
        }
    }
    // the installers can write the Run value (Inno's startup task, MSI
    // AUTOSTART=1): adopt it instead of wiping it with a fresh config's default
    if startup::is_enabled() && !config.startup.enabled {
        config.startup.enabled = true;
        if let Err(e) = config::save_config(&config) {
            dlog!("saving the installer's startup choice failed: {e:#}");
        }
    } else if config.startup.enabled {
        // keep it pointing at this exe (it may have moved)
        startup::sync(true);
    }

    let config_shared = Arc::new(Mutex::new(config.clone()));
    let state_shared = Arc::new(Mutex::new(AppState {
        active: switch::is_active(&data_dir),
        ..Default::default()
    }));

    // started with the era on: after a reboot RetroBar and Open-Shell aren't running
    if state_shared.lock().unwrap().active {
        dlog!("era was on at startup; resuming");
        if let Err(e) = switch::resume(&data_dir) {
            dlog!("resume failed: {e:#}");
            notifications::always("EraSwitch couldn't resume the era", &format!("{e:#}"));
        }
    }

    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

    // Ctrl+C / termination (dev runs, shutdown): put Windows 11 back first
    {
        let dir = data_dir.clone();
        ctrlc::set_handler(move || {
            let _ = switch::restore(&dir);
            std::process::exit(0);
        })
        .ok();
    }

    let mut hotkey_reg = hotkeys::register(&config.hotkeys.toggle, &cmd_tx)?;
    let tray_handle = {
        let st = state_shared.lock().unwrap();
        tray::build(&tray_state(&st, &config))?
    };

    if let Some(p) = open_now {
        let _ = cmd_tx.send(Cmd::OpenLondopy(p));
    }

    // first launch (the MSI, the portable exe and Scoop never showed the
    // installer's page): offer to install whatever the era needs, once
    {
        let mut cfg = config_shared.lock().unwrap();
        if !cfg.tools.offered {
            cfg.tools.offered = true;
            if let Err(e) = config::save_config(&cfg) {
                dlog!("saving the helpers offer flag failed: {e:#}");
            }
            let missing = tools::missing(&cfg);
            if !missing.is_empty() {
                offer_helpers(missing, cfg.tools.clone(), data_dir.clone(), true);
            }
        }
    }

    let (update_tx, update_rx) = mpsc::channel::<updater::UpdateEvent>();
    let interval =
        |c: &AppConfig| Duration::from_secs(c.updater.check_interval_h.max(1) as u64 * 3600);
    // back-dated so the first loop iteration checks
    let mut last_update_check = Instant::now()
        .checked_sub(interval(&config))
        .unwrap_or_else(Instant::now);
    let mut last_inbox = Instant::now();
    // windows repeats WM_HOTKEY while the keys are held
    let debounce = Duration::from_millis(400);
    let mut last_hotkey = Instant::now()
        .checked_sub(debounce)
        .unwrap_or_else(Instant::now);
    // Exit or Restart asked for mid-switch: run once the switch is done
    let mut deferred: Option<Cmd> = None;
    let mut last_sync = Instant::now();

    loop {
        // pump win32 messages so the tray and hotkey work
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
            };
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(16));

        while let Some(ev) = hotkeys::poll() {
            if ev.id() == hotkey_reg.toggle_id
                && ev.state() == global_hotkey::HotKeyState::Pressed
                && last_hotkey.elapsed() >= debounce
            {
                last_hotkey = Instant::now();
                let _ = cmd_tx.send(Cmd::Toggle);
            }
        }

        while let Some(ev) = tray::poll_menu() {
            let ids = &tray_handle.ids;
            let cmd = if ev.id == ids.toggle {
                Some(Cmd::Toggle)
            } else if ev.id == ids.update || ev.id == ids.settings {
                Some(Cmd::OpenSettings)
            } else if ev.id == ids.restart {
                Some(Cmd::Restart)
            } else if ev.id == ids.exit {
                Some(Cmd::Exit)
            } else {
                ids.eras
                    .iter()
                    .find(|(id, _)| *id == ev.id)
                    .map(|(_, p)| Cmd::PickEra(*p))
            };
            if let Some(c) = cmd {
                let _ = cmd_tx.send(c);
            }
        }

        while let Some(ev) = tray::poll_tray() {
            use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                let _ = cmd_tx.send(Cmd::Toggle);
            }
        }

        // .londopy files handed over by a second copy started from Explorer
        if last_inbox.elapsed() >= Duration::from_millis(500) {
            last_inbox = Instant::now();
            for p in londopy::drain_inbox(&data_dir) {
                let _ = cmd_tx.send(Cmd::OpenLondopy(p));
            }
        }

        {
            let cfg = config_shared.lock().unwrap().clone();
            if cfg.updater.enabled && last_update_check.elapsed() >= interval(&cfg) {
                last_update_check = Instant::now();
                updater::background_check(cfg.updater.clone(), update_tx.clone(), false);
            }
        }
        while let Ok(ev) = update_rx.try_recv() {
            let _ = cmd_tx.send(match ev {
                updater::UpdateEvent::Available(v) => Cmd::UpdateAvailable(v),
                updater::UpdateEvent::UpToDate => Cmd::UpToDate,
                updater::UpdateEvent::Failed(e) => Cmd::UpdateFailed(e),
            });
        }

        // someone ran `--restore` (or the snapshot otherwise changed): follow it
        if last_sync.elapsed() >= Duration::from_secs(1) {
            last_sync = Instant::now();
            let mut st = state_shared.lock().unwrap();
            let on_disk = switch::is_active(&data_dir);
            if !st.busy && st.active != on_disk {
                st.active = on_disk;
                let cfg = config_shared.lock().unwrap().clone();
                tray::update(&tray_handle, &tray_state(&st, &cfg));
            }
        }

        while let Ok(cmd) = cmd_rx.try_recv() {
            // a switch is mid-way: Exit and Restart wait for it, not cut it off
            if matches!(cmd, Cmd::Exit | Cmd::Restart) && state_shared.lock().unwrap().busy {
                deferred = Some(cmd);
                continue;
            }
            match cmd {
                Cmd::Toggle => {
                    let mut st = state_shared.lock().unwrap();
                    if st.busy {
                        continue;
                    }
                    let turn_on = !st.active;
                    let cfg = config_shared.lock().unwrap().clone();
                    // switching on with a helper missing: offer to install it
                    // instead of failing with an error
                    if turn_on {
                        let missing = tools::missing(&cfg);
                        if !missing.is_empty() {
                            drop(st);
                            offer_helpers(missing, cfg.tools.clone(), data_dir.clone(), false);
                            continue;
                        }
                    }
                    st.busy = true;
                    drop(st);
                    let kind = if turn_on { Switch::On } else { Switch::Off };
                    spawn_switch(kind, cfg, data_dir.clone(), cmd_tx.clone());
                }

                // the tray's Era menu: make that era the one, and if an era is
                // on right now, go straight across to it
                Cmd::PickEra(preset) => {
                    let mut st = state_shared.lock().unwrap();
                    if st.busy {
                        // the menu ticked the clicked item itself: put it back
                        let cfg = config_shared.lock().unwrap().clone();
                        tray::update(&tray_handle, &tray_state(&st, &cfg));
                        continue;
                    }
                    let (cfg, changed) = {
                        let mut cfg = config_shared.lock().unwrap();
                        let changed = era::Preset::of(&cfg.era) != Some(preset);
                        if changed {
                            cfg.era.apply_preset(preset);
                            if let Err(e) = config::save_config(&cfg) {
                                dlog!("saving the picked era failed: {e:#}");
                            }
                        }
                        (cfg.clone(), changed)
                    };
                    // always redraw: the menu flips a check item on every click
                    tray::update(&tray_handle, &tray_state(&st, &cfg));
                    // what's on screen right now, which the config can be ahead of
                    let applied = switch::load(&data_dir).ok().flatten().map(|s| s.era_name);
                    let needs_switch = changed || applied.as_deref() != Some(preset.name());
                    if st.active && needs_switch {
                        let missing = tools::missing(&cfg);
                        if !missing.is_empty() {
                            drop(st);
                            offer_helpers(missing, cfg.tools.clone(), data_dir.clone(), false);
                            continue;
                        }
                        st.busy = true;
                        drop(st);
                        spawn_switch(Switch::Across, cfg, data_dir.clone(), cmd_tx.clone());
                    }
                }

                Cmd::SwitchDone(result) => {
                    let mut st = state_shared.lock().unwrap();
                    st.busy = false;
                    st.active = switch::is_active(&data_dir);
                    if let Err(e) = result {
                        dlog!("switch failed: {e}");
                        notifications::always("EraSwitch", &e);
                    }
                    let cfg = config_shared.lock().unwrap().clone();
                    tray::update(&tray_handle, &tray_state(&st, &cfg));
                    if let Some(c) = deferred.take() {
                        let _ = cmd_tx.send(c);
                    }
                }

                Cmd::OpenSettings => {
                    ui::open_settings(
                        Arc::clone(&config_shared),
                        Arc::clone(&state_shared),
                        cmd_tx.clone(),
                    );
                }

                Cmd::ConfigUpdated(new_cfg) => {
                    let new_cfg = *new_cfg;
                    let old =
                        std::mem::replace(&mut *config_shared.lock().unwrap(), new_cfg.clone());
                    if old.hotkeys.toggle != new_cfg.hotkeys.toggle {
                        hotkeys::reregister(&mut hotkey_reg, &new_cfg.hotkeys.toggle, &cmd_tx);
                    }
                    if old.startup.enabled != new_cfg.startup.enabled {
                        startup::sync(new_cfg.startup.enabled);
                    }
                    let st = state_shared.lock().unwrap();
                    tray::update(&tray_handle, &tray_state(&st, &new_cfg));
                }

                Cmd::OpenLondopy(path) => match era::open_londopy(&path, &data_dir) {
                    Ok(loaded) => {
                        let mut cfg = config_shared.lock().unwrap();
                        cfg.era = loaded;
                        if let Err(e) = config::save_config(&cfg) {
                            dlog!("saving opened era failed: {e:#}");
                        }
                        let name = cfg.era.name.clone();
                        let st = state_shared.lock().unwrap();
                        tray::update(&tray_handle, &tray_state(&st, &cfg));
                        let note = if st.active {
                            "It'll be used the next time you switch."
                        } else {
                            "Click the tray icon to switch to it."
                        };
                        notifications::always(
                            "EraSwitch",
                            &format!("Loaded the era '{name}'. {note}"),
                        );
                    }
                    Err(e) => {
                        notifications::always("EraSwitch couldn't open that era", &format!("{e:#}"))
                    }
                },

                Cmd::UpdateAvailable(v) => {
                    let cfg = config_shared.lock().unwrap().clone();
                    let mut st = state_shared.lock().unwrap();
                    if st.update.as_deref() != Some(v.as_str()) {
                        notifications::update_available(&v, &cfg.notifications);
                    }
                    st.update = Some(v);
                    tray::update(&tray_handle, &tray_state(&st, &cfg));
                }

                Cmd::UpToDate => {
                    let cfg = config_shared.lock().unwrap().clone();
                    notifications::up_to_date(&cfg.notifications);
                }

                Cmd::UpdateFailed(e) => dlog!("update check failed: {e}"),

                Cmd::HotkeyFailed(hk) => {
                    let cfg = config_shared.lock().unwrap().clone();
                    notifications::hotkey_failed(&hk, &cfg.notifications);
                }

                Cmd::Restart => {
                    // the era stays on; the new copy resumes it
                    let exe =
                        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("EraSwitch.exe"));
                    let _ = std::process::Command::new(exe).arg("--restarted").spawn();
                    return Ok(());
                }

                Cmd::Exit => {
                    // nothing would be left to switch RetroBar and Open-Shell
                    // off, so put Windows 11 back first, and don't leave if that
                    // failed: the tray is how you'd retry
                    if let Err(e) = switch::restore(&data_dir) {
                        notifications::always(
                            "EraSwitch is still running",
                            &format!("{e:#} Exit again once it's sorted, or run EraSwitch.exe --restore."),
                        );
                        let mut st = state_shared.lock().unwrap();
                        st.active = switch::is_active(&data_dir);
                        let cfg = config_shared.lock().unwrap().clone();
                        tray::update(&tray_handle, &tray_state(&st, &cfg));
                        continue;
                    }
                    // the tray icon goes away with the handle
                    drop(tray_handle);
                    return Ok(());
                }
            }
        }
    }
}
