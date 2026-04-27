use anyhow::Result;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use muda::{CheckMenuItem, ContextMenu, Menu, MenuItem, MenuEvent};
use parking_lot::Mutex;
use std::{sync::Arc, thread, time::Duration};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

mod audio;
mod config;
mod console_window;
mod hotkey;
mod settings_dialog;
mod single_instance;
mod tray;
mod tray_balloon;

use audio::{process, set_error_status, Recorder};
use config::{load_config, save_distill_mode_to_config, save_modes_to_config};
use hotkey::{parse_hotkey, save_hotkey_to_config};
use tray::icon_for_status;

#[link(name = "user32")]
unsafe extern "system" {
    fn SetMenuDefaultItem(h_menu: isize, u_item: u32, f_by_pos: u32) -> i32;
}

fn main() -> Result<()> {
    // Exit early (with a message box) if another instance is already running.
    let _instance_guard = match single_instance::acquire() {
        Some(g) => g,
        None    => return Ok(()),
    };

    let cfg = Arc::new(load_config()?);

    console_window::hide();

    let event_loop = EventLoopBuilder::new().build();

    let manager = GlobalHotKeyManager::new()?;
    let hotkey_str = cfg.hotkey.clone().unwrap_or_else(|| settings_dialog::DEFAULT_HOTKEY.to_string());
    let mut toggle = parse_hotkey(&hotkey_str)
        .ok_or_else(|| anyhow::anyhow!("invalid hotkey in config: {hotkey_str}"))?;
    let mut current_hotkey_str = hotkey_str.clone();
    manager.register(toggle)?;
    let rx = GlobalHotKeyEvent::receiver();

    let tray_menu = Menu::new();
    let show_logs_item   = MenuItem::new("Show logs",        true, None);
    let settings_item    = MenuItem::new("Settings",         true, None);
    let passthrough_item = CheckMenuItem::new("Passthrough mode", true, false, None);
    let exit_item        = MenuItem::new("Exit",             true, None);
    tray_menu.append(&show_logs_item).expect("menu append");
    tray_menu.append(&settings_item).expect("menu append");
    tray_menu.append(&passthrough_item).expect("menu append");
    tray_menu.append(&exit_item).expect("menu append");

    // Bold the first item as the Windows default menu action
    unsafe { SetMenuDefaultItem(tray_menu.hpopupmenu(), 0, 1); }

    let tray = TrayIconBuilder::new()
        .with_icon(icon_for_status(None))
        .with_menu(Box::new(tray_menu))
        .with_tooltip(format!("Dictaship – {current_hotkey_str} to record"))
        .build()
        .expect("tray icon");

    tray_balloon::init(tray.window_handle() as isize);

    let menu_rx = MenuEvent::receiver();
    let tray_rx = TrayIconEvent::receiver();

    // Distillation mode state — kept separately so it can change without reloading cfg
    let initial_mode = cfg.distill_mode.clone()
        .unwrap_or_else(|| cfg.modes.first().map(|m| m.name.clone()).unwrap_or_default());
    let modes_state: Arc<Mutex<Vec<config::ModeConfig>>> = Arc::new(Mutex::new(cfg.modes.clone()));
    let current_mode: Arc<Mutex<String>> = Arc::new(Mutex::new(initial_mode));

    let tray_status: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let mut last_status: Option<String> = None;
    let mut recorder: Option<Recorder> = None;
    println!("ready. {} to toggle recording.", current_hotkey_str);

    event_loop.run(move |_, _, cf| {
        *cf = ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_millis(50));

        // Sync icon + balloon + tooltip with current status
        let current_status = tray_status.lock().clone();
        if current_status != last_status {
            tray.set_icon(Some(icon_for_status(current_status.as_deref()))).ok();
            match &current_status {
                Some(text) => {
                    tray.set_tooltip(Some(text.as_str())).ok();
                    tray_balloon::show(text);
                }
                None => {
                    tray.set_tooltip(Some(format!("Dictaship – {current_hotkey_str} to record"))).ok();
                    tray_balloon::clear();
                }
            }
            last_status = current_status;
        }

        // Settings dialog result
        if let Some(result) = settings_dialog::take_result() {
            if let Some(new_hotkey) = parse_hotkey(&result.hotkey) {
                let _ = manager.unregister(toggle);
                toggle = new_hotkey;
                current_hotkey_str = result.hotkey.clone();
                tray.set_tooltip(Some(format!("Dictaship – {} to record", result.hotkey))).ok();
                if manager.register(toggle).is_err() {
                    eprintln!("failed to register hotkey {}", result.hotkey);
                } else {
                    println!("hotkey changed to {}", result.hotkey);
                    save_hotkey_to_config(&result.hotkey);
                }
            }
            *current_mode.lock() = result.mode_name.clone();
            save_distill_mode_to_config(&result.mode_name);
            if !result.modes.is_empty() {
                *modes_state.lock() = result.modes.clone();
                save_modes_to_config(&result.modes);
            }
        }

        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == *exit_item.id()      { std::process::exit(0); }
            if ev.id == *show_logs_item.id() { console_window::toggle(); }
            if ev.id == *settings_item.id()  {
                let modes_snap = modes_state.lock().clone();
                let mode_snap  = current_mode.lock().clone();
                settings_dialog::open(&current_hotkey_str, modes_snap, &mode_snap);
            }
        }

        while let Ok(ev) = tray_rx.try_recv() {
            match ev {
                TrayIconEvent::DoubleClick { button: tray_icon::MouseButton::Left, .. } => {
                    console_window::toggle();
                }
                _ => {}
            }
        }

        while let Ok(ev) = rx.try_recv() {
            if ev.id == toggle.id() && ev.state == global_hotkey::HotKeyState::Pressed {
                match recorder.take() {
                    None => match Recorder::start() {
                        Ok(r) => {
                            *tray_status.lock() = Some("Recording...".to_owned());
                            recorder = Some(r);
                            println!("recording...");
                        }
                        Err(e) => set_error_status(&tray_status, "Microphone error", e),
                    },
                    Some(r) => {
                        println!("stopping.");
                        *tray_status.lock() = Some("Processing...".to_owned());
                        let (samples, sample_rate) = r.stop();
                        let cfg         = cfg.clone();
                        let status      = tray_status.clone();
                        let passthrough = passthrough_item.is_checked();
                        let mode_name   = current_mode.lock().clone();
                        let active_prompt = {
                            let ms = modes_state.lock();
                            ms.iter().find(|m| m.name == mode_name)
                                .map(|m| m.prompt.clone())
                                .unwrap_or_else(|| cfg.prompt.clone())
                        };
                        thread::spawn(move || {
                            process(samples, sample_rate, cfg, status, passthrough, active_prompt)
                        });
                    }
                }
            }
        }
    });
}
