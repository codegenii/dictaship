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
mod tray;
mod tray_balloon;

use audio::{process, Recorder};
use config::load_config;
use hotkey::{parse_hotkey, save_hotkey_to_config};
use tray::icon_for_status;

#[link(name = "user32")]
unsafe extern "system" {
    fn SetMenuDefaultItem(h_menu: isize, u_item: u32, f_by_pos: u32) -> i32;
}

fn main() -> Result<()> {
    let cfg = Arc::new(load_config()?);

    console_window::hide();

    let event_loop = EventLoopBuilder::new().build();

    let manager = GlobalHotKeyManager::new()?;
    let hotkey_str = cfg.hotkey.clone().unwrap_or_else(|| "Alt+Q".to_string());
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
        .with_tooltip(format!("Partizan – {current_hotkey_str} to record"))
        .build()
        .expect("tray icon");

    tray_balloon::init(tray.window_handle() as isize);

    let menu_rx = MenuEvent::receiver();
    let tray_rx = TrayIconEvent::receiver();

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
                    tray.set_tooltip(Some(format!("Partizan – {current_hotkey_str} to record"))).ok();
                    tray_balloon::clear();
                }
            }
            last_status = current_status;
        }

        // Settings dialog result
        if let Some(new_hotkey_str) = settings_dialog::take_result() {
            if let Some(new_hotkey) = parse_hotkey(&new_hotkey_str) {
                let _ = manager.unregister(toggle);
                toggle = new_hotkey;
                current_hotkey_str = new_hotkey_str.clone();
                tray.set_tooltip(Some(format!("Partizan – {new_hotkey_str} to record"))).ok();
                if manager.register(toggle).is_err() {
                    eprintln!("failed to register hotkey {new_hotkey_str}");
                } else {
                    println!("hotkey changed to {new_hotkey_str}");
                    save_hotkey_to_config(&new_hotkey_str);
                }
            }
        }

        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == *exit_item.id()      { std::process::exit(0); }
            if ev.id == *show_logs_item.id() { console_window::toggle(); }
            if ev.id == *settings_item.id()  { settings_dialog::open(&current_hotkey_str); }
        }

        while let Ok(ev) = tray_rx.try_recv() {
            match ev {
                TrayIconEvent::Click { button: tray_icon::MouseButton::Left, .. }
                | TrayIconEvent::DoubleClick { button: tray_icon::MouseButton::Left, .. } => {
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
                        Err(e) => eprintln!("mic error: {e}"),
                    },
                    Some(r) => {
                        println!("stopping.");
                        *tray_status.lock() = Some("Processing...".to_owned());
                        let (samples, sample_rate) = r.stop();
                        let cfg         = cfg.clone();
                        let status      = tray_status.clone();
                        let passthrough = passthrough_item.is_checked();
                        thread::spawn(move || process(samples, sample_rate, cfg, status, passthrough));
                    }
                }
            }
        }
    });
}
