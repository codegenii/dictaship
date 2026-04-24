use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use enigo::{Enigo, Keyboard, Settings};
use global_hotkey::{hotkey::{Code, HotKey, Modifiers}, GlobalHotKeyManager, GlobalHotKeyEvent};
use hound::{WavSpec, WavWriter};
use muda::{ContextMenu, Menu, MenuItem, MenuEvent};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{io::Cursor, sync::Arc, thread, time::Duration};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{TrayIconBuilder, TrayIconEvent};

// ── WinAPI helpers ───────────────────────────────────────────────────────────

#[link(name = "user32")]
unsafe extern "system" {
    fn SetMenuDefaultItem(h_menu: isize, u_item: u32, f_by_pos: u32) -> i32;
}

// ── tray balloon notifications ────────────────────────────────────────────────

mod tray_balloon {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicIsize, Ordering};

    static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

    const NIM_MODIFY:   u32 = 0x00000001;
    const NIF_INFO:     u32 = 0x00000010;
    const NIIF_NOSOUND: u32 = 0x00000010;

    // NOTIFYICONDATAW V2 layout (952 bytes on 64-bit Windows).
    // Explicit padding fields mirror natural #[repr(C)] alignment padding.
    #[repr(C)]
    struct NotifyIconData {
        cb_size:            u32,
        _pad0:              u32,      // pad to align hwnd to offset 8
        hwnd:               isize,
        u_id:               u32,
        u_flags:            u32,
        u_callback_message: u32,
        _pad1:              u32,      // pad to align h_icon to offset 32
        h_icon:             isize,
        sz_tip:             [u16; 128],
        dw_state:           u32,
        dw_state_mask:      u32,
        sz_info:            [u16; 256],
        u_version:          u32,
        sz_info_title:      [u16; 64],
        dw_info_flags:      u32,
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn Shell_NotifyIconW(dw_message: u32, lp_data: *mut NotifyIconData) -> i32;
    }

    fn wide_truncate<const N: usize>(s: &str) -> [u16; N] {
        let mut buf = [0u16; N];
        let encoded: Vec<u16> = OsStr::new(s).encode_wide().collect();
        let len = encoded.len().min(N - 1);
        buf[..len].copy_from_slice(&encoded[..len]);
        buf
    }

    pub fn init(hwnd: isize) {
        TRAY_HWND.store(hwnd, Ordering::SeqCst);
    }

    fn notify(text: &str) {
        let hwnd = TRAY_HWND.load(Ordering::SeqCst);
        if hwnd == 0 { return; }
        let mut nid = NotifyIconData {
            cb_size:            std::mem::size_of::<NotifyIconData>() as u32,
            _pad0:              0,
            hwnd,
            u_id:               1,  // tray-icon COUNTER starts at 1
            u_flags:            NIF_INFO,
            u_callback_message: 0,
            _pad1:              0,
            h_icon:             0,
            sz_tip:             [0; 128],
            dw_state:           0,
            dw_state_mask:      0,
            sz_info:            wide_truncate(text),
            u_version:          0,
            sz_info_title:      wide_truncate("Partizan"),
            dw_info_flags:      NIIF_NOSOUND,
        };
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut nid); }
    }

    // Show a balloon popup on the tray icon. Empty szInfo dismisses it.
    pub fn show(text: &str) { notify(text); }
    pub fn clear()           { notify(""); }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn notify_icon_data_is_952_bytes() {
            assert_eq!(std::mem::size_of::<NotifyIconData>(), 952);
        }

        #[test]
        fn wide_truncate_short_string() {
            let buf: [u16; 8] = wide_truncate("Hi");
            assert_eq!(buf[0], b'H' as u16);
            assert_eq!(buf[1], b'i' as u16);
            assert_eq!(buf[2], 0);
        }

        #[test]
        fn wide_truncate_empty_string() {
            let buf: [u16; 8] = wide_truncate("");
            assert!(buf.iter().all(|&c| c == 0));
        }

        #[test]
        fn wide_truncate_clips_and_null_terminates() {
            // 4-element buffer: max 3 chars + NUL
            let buf: [u16; 4] = wide_truncate("ABCDEF");
            assert_eq!(buf[0], b'A' as u16);
            assert_eq!(buf[1], b'B' as u16);
            assert_eq!(buf[2], b'C' as u16);
            assert_eq!(buf[3], 0);
        }
    }
}

// ── console show/hide ────────────────────────────────────────────────────────

mod console_window {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleWindow() -> isize;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, n_cmd_show: i32) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
    }

    pub fn hide() {
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd != 0 { ShowWindow(hwnd, 0); }
        }
    }

    pub fn toggle() {
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd == 0 { return; }
            if IsWindowVisible(hwnd) != 0 {
                ShowWindow(hwnd, 0);
            } else {
                ShowWindow(hwnd, 5);
            }
        }
    }
}

// ── settings dialog (WinAPI window, separate thread) ─────────────────────────

mod settings_dialog {
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    static OPEN: AtomicBool = AtomicBool::new(false);
    static RESULT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    pub const DEFAULT_HOTKEY: &str = "Alt+R";

    // Windows message/style constants
    const WM_COMMAND:    u32 = 0x0111;
    const WM_DESTROY:    u32 = 0x0002;
    const WM_KEYDOWN:    u32 = 0x0100;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WS_CAPTION:    u32 = 0x00C00000;
    const WS_SYSMENU:    u32 = 0x00080000;
    const WS_CHILD:      u32 = 0x40000000;
    const WS_VISIBLE:    u32 = 0x10000000;
    const WS_BORDER:     u32 = 0x00800000;
    const WS_EX_TOPMOST: u32 = 0x00000008;
    const WS_EX_DLGMODALFRAME: u32 = 0x00000001;
    const BS_PUSHBUTTON: u32 = 0x00000000;
    const SS_SIMPLE:     u32 = 0x0B;
    const CW_USEDEFAULT: i32 = 0x80000000u32 as i32;
    const COLOR_BTNFACE: isize = 15;

    const ID_DISPLAY: i32 = 101;
    const ID_APPLY:   i32 = 102;
    const ID_CANCEL:  i32 = 103;
    const ID_DEFAULT: i32 = 104;

    // Modifier-only virtual keys to skip when capturing a hotkey
    const SKIP_VKEYS: &[usize] = &[0x10, 0x11, 0x12, 0x5B, 0x5C];

    thread_local! {
        static PENDING: RefCell<String> = RefCell::new(String::new());
    }

    type Hwnd = isize;
    type Hinstance = isize;

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        lpfn_wnd_proc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: isize,
        h_icon: isize,
        h_cursor: isize,
        hbr_background: isize,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: isize,
    }

    #[repr(C)]
    struct Point { x: i32, y: i32 }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: Point,
        l_private: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassExW(lp: *const WndClassExW) -> u16;
        fn CreateWindowExW(
            ex: u32, cls: *const u16, title: *const u16, style: u32,
            x: i32, y: i32, w: i32, h: i32,
            parent: Hwnd, menu: Hwnd, inst: Hinstance, param: *const u8,
        ) -> Hwnd;
        fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn PostQuitMessage(code: i32);
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize;
        fn DestroyWindow(hwnd: Hwnd) -> i32;
        fn GetDlgItem(dlg: Hwnd, id: i32) -> Hwnd;
        fn LoadCursorW(inst: Hwnd, name: *const u16) -> Hwnd;
        fn GetKeyState(vk: i32) -> i16;
        fn SetFocus(hwnd: Hwnd) -> Hwnd;
        fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
        fn InvalidateRect(hwnd: Hwnd, lp_rect: *const u8, b_erase: i32) -> i32;
        fn UpdateWindow(hwnd: Hwnd) -> i32;
        fn MapVirtualKeyW(u_code: u32, u_map_type: u32) -> u32;
        fn GetKeyNameTextW(l_param: i32, lp_string: *mut u16, cch_size: i32) -> i32;
    }

    const MAPVK_VK_TO_VSC: u32 = 0;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> Hinstance;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain([0u16]).collect()
    }

    fn vkey_to_name(vk: usize) -> Option<String> {
        let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) };
        if scan == 0 { return None; }
        let lparam = (scan << 16) as i32;
        let mut buf = [0u16; 64];
        let len = unsafe { GetKeyNameTextW(lparam, buf.as_mut_ptr(), buf.len() as i32) };
        if len > 0 {
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        } else {
            None
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            if !SKIP_VKEYS.contains(&wp) {
                if let Some(key_name) = vkey_to_name(wp) {
                    let mut parts = Vec::<String>::new();
                    unsafe {
                        if (GetKeyState(0x11) as i16) < 0 { parts.push("Ctrl".to_string()); }
                        if (GetKeyState(0x12) as i16) < 0 { parts.push("Alt".to_string()); }
                        if (GetKeyState(0x10) as i16) < 0 { parts.push("Shift".to_string()); }
                    }
                    parts.push(key_name);
                    let hotkey_str = parts.join("+");
                    PENDING.with(|p| *p.borrow_mut() = hotkey_str.clone());
                    unsafe {
                        let display = GetDlgItem(hwnd, ID_DISPLAY);
                        SetWindowTextW(display, wide(&hotkey_str).as_ptr());
                        InvalidateRect(display, std::ptr::null(), 1);
                        UpdateWindow(display);
                    }
                }
            }
            return 0;
        }
        if msg == WM_COMMAND {
            let id = (wp & 0xFFFF) as i32;
            if id == ID_APPLY {
                let s = PENDING.with(|p| p.borrow().clone());
                if !s.is_empty() {
                    *RESULT.lock().unwrap() = Some(s);
                }
                unsafe { DestroyWindow(hwnd); }
                return 0;
            }
            if id == ID_DEFAULT {
                PENDING.with(|p| *p.borrow_mut() = DEFAULT_HOTKEY.to_string());
                unsafe {
                    let display = GetDlgItem(hwnd, ID_DISPLAY);
                    SetWindowTextW(display, wide(DEFAULT_HOTKEY).as_ptr());
                    InvalidateRect(display, std::ptr::null(), 1);
                    UpdateWindow(display);
                    SetFocus(hwnd);
                }
                return 0;
            }
            if id == ID_CANCEL {
                unsafe { DestroyWindow(hwnd); }
                return 0;
            }
        }
        if msg == WM_DESTROY {
            OPEN.store(false, Ordering::SeqCst);
            unsafe { PostQuitMessage(0); }
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    fn run_dialog(current: String) {
        PENDING.with(|p| *p.borrow_mut() = current.clone());

        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let cls_name = wide("PartizanSettings");

            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                lpfn_wnd_proc: wnd_proc,
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance: hinst,
                h_icon: 0,
                h_cursor: LoadCursorW(0, 32512 as *const u16), // IDC_ARROW
                hbr_background: COLOR_BTNFACE + 1,
                lpsz_menu_name: std::ptr::null(),
                lpsz_class_name: cls_name.as_ptr(),
                h_icon_sm: 0,
            };
            RegisterClassExW(&wc);

            let title = wide("Partizan \u{2013} Settings");
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_DLGMODALFRAME,
                cls_name.as_ptr(), title.as_ptr(),
                WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
                CW_USEDEFAULT, CW_USEDEFAULT, 320, 200,
                0, 0, hinst, std::ptr::null(),
            );
            if hwnd == 0 { OPEN.store(false, Ordering::SeqCst); return; }

            // Instruction
            let instr = wide("Press a new hotkey combo, then click Apply");
            CreateWindowExW(
                0, wide("STATIC").as_ptr(), instr.as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_SIMPLE,
                12, 14, 280, 18, hwnd, 0, hinst, std::ptr::null(),
            );

            // Label
            let label_text = wide("Recording hotkey:");
            CreateWindowExW(
                0, wide("STATIC").as_ptr(), label_text.as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_SIMPLE,
                12, 42, 280, 18, hwnd, 0, hinst, std::ptr::null(),
            );

            // Display showing captured hotkey
            CreateWindowExW(
                0, wide("STATIC").as_ptr(), wide(&current).as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_SIMPLE | WS_BORDER,
                12, 65, 280, 26, hwnd, ID_DISPLAY as _, hinst, std::ptr::null(),
            );

            // Buttons: Apply | Default | Cancel
            CreateWindowExW(
                0, wide("BUTTON").as_ptr(), wide("Apply").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                10, 110, 80, 26, hwnd, ID_APPLY as _, hinst, std::ptr::null(),
            );
            CreateWindowExW(
                0, wide("BUTTON").as_ptr(), wide("Default").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                110, 110, 80, 26, hwnd, ID_DEFAULT as _, hinst, std::ptr::null(),
            );
            CreateWindowExW(
                0, wide("BUTTON").as_ptr(), wide("Cancel").as_ptr(),
                WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                210, 110, 80, 26, hwnd, ID_CANCEL as _, hinst, std::ptr::null(),
            );

            SetFocus(hwnd);

            let mut msg = Msg {
                hwnd: 0, message: 0, w_param: 0, l_param: 0,
                time: 0, pt: Point { x: 0, y: 0 }, l_private: 0,
            };
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    pub fn open(current: &str) {
        if OPEN.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return;
        }
        let current = current.to_string();
        thread::spawn(move || run_dialog(current));
    }

    pub fn take_result() -> Option<String> {
        RESULT.lock().unwrap().take()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn vkey_letter_a() {
            assert_eq!(vkey_to_name(0x41), Some("A").map(str::to_owned));
        }

        #[test]
        fn vkey_letter_z() {
            assert_eq!(vkey_to_name(0x5A), Some("Z").map(str::to_owned));
        }

        #[test]
        fn vkey_f1() {
            assert_eq!(vkey_to_name(0x70), Some("F1").map(str::to_owned));
        }

        #[test]
        fn vkey_f12() {
            assert_eq!(vkey_to_name(0x7B), Some("F12").map(str::to_owned));
        }

        #[test]
        fn vkey_modifier_keys_have_os_names() {
            // Modifier keys have OS-provided names; wnd_proc skips them via SKIP_VKEYS
            assert!(vkey_to_name(0x10).is_some()); // VK_SHIFT
            assert!(vkey_to_name(0x11).is_some()); // VK_CONTROL
            assert!(vkey_to_name(0x12).is_some()); // VK_MENU (Alt)
        }

        #[test]
        fn vkey_unknown_returns_none() {
            assert_eq!(vkey_to_name(0x00), None);
            assert_eq!(vkey_to_name(0x01), None);
        }

        #[test]
        fn vkey_all_letters() {
            for (i, letter) in ('A'..='Z').enumerate() {
                let vk = 0x41 + i;
                let name = vkey_to_name(vk)
                    .unwrap_or_else(|| panic!("vk 0x{vk:02X} should map to a letter"));
                assert_eq!(name.chars().next(), Some(letter), "vk=0x{vk:02X}");
                assert_eq!(name.len(), 1, "vk=0x{vk:02X}");
            }
        }

        #[test]
        fn vkey_all_function_keys() {
            for i in 0..12usize {
                let vk = 0x70 + i;
                let expected = format!("F{}", i + 1);
                let name = vkey_to_name(vk)
                    .unwrap_or_else(|| panic!("vk 0x{vk:02X} should map to F{}", i + 1));
                assert_eq!(name, expected.as_str(), "vk=0x{vk:02X}");
            }
        }
    }
}

// ── hotkey string helpers ─────────────────────────────────────────────────────

fn parse_hotkey(s: &str) -> Option<HotKey> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (mod_parts, key_parts) = parts.split_at(parts.len().saturating_sub(1));
    let key_str = key_parts.first()?;

    let code = match key_str.to_uppercase().as_str() {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "F1"  => Code::F1,  "F2"  => Code::F2,  "F3"  => Code::F3,  "F4"  => Code::F4,
        "F5"  => Code::F5,  "F6"  => Code::F6,  "F7"  => Code::F7,  "F8"  => Code::F8,
        "F9"  => Code::F9,  "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        _ => return None,
    };

    let mut mods = Modifiers::empty();
    for m in mod_parts {
        match m.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt"              => mods |= Modifiers::ALT,
            "shift"            => mods |= Modifiers::SHIFT,
            "meta" | "win"     => mods |= Modifiers::META,
            _                  => return None,
        }
    }

    Some(HotKey::new(if mods.is_empty() { None } else { Some(mods) }, code))
}

fn save_hotkey_to_config(hotkey_str: &str) {
    let path = "config.toml";
    let Ok(content) = std::fs::read_to_string(path) else { return };
    let line = format!("hotkey = \"{}\"", hotkey_str);
    let new_content = if content.lines().any(|l| l.trim_start().starts_with("hotkey")) {
        content.lines()
            .map(|l| if l.trim_start().starts_with("hotkey") { line.as_str() } else { l })
            .collect::<Vec<_>>()
            .join("\r\n")
    } else {
        format!("{}\n{}\n", content.trim_end(), line)
    };
    let _ = std::fs::write(path, new_content);
}

// ── tray icon ─────────────────────────────────────────────────────────────────

fn make_colored_icon(r: u8, g: u8, b: u8) -> tray_icon::Icon {
    const S: u32 = 32;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    let c = S as f32 / 2.0;
    let rad = S as f32 / 2.0 - 1.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            if dx * dx + dy * dy <= rad * rad {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, S, S).expect("valid icon")
}

fn icon_for_status(status: Option<&str>) -> tray_icon::Icon {
    match status {
        Some("Recording...")  => make_colored_icon(239, 68,  68),  // red
        Some("Processing...") => make_colored_icon(251, 146, 60),  // orange
        Some("Distilling...") => make_colored_icon(168, 85,  247), // purple
        _                     => make_colored_icon(34,  197, 94),  // green (idle)
    }
}

// ── config ────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize)]
struct Config {
    whisper_url:   String,
    ollama_url:    String,
    whisper_model: String,
    llm_model:     String,
    #[serde(default)]
    hotkey:        Option<String>,
    prompt:        String,
}

fn parse_config(raw: &str) -> Result<Config> {
    toml::from_str(raw).map_err(|e| anyhow::anyhow!("invalid config.toml: {e}"))
}

fn load_config() -> Result<Config> {
    let raw = std::fs::read_to_string("config.toml")
        .map_err(|e| anyhow::anyhow!("cannot read config.toml: {e}"))?;
    parse_config(&raw)
}

// ── audio pipeline ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct WhisperResp { text: String }
#[derive(Deserialize)]
struct OllamaResp  { response: String }

struct Recorder {
    samples: Arc<Mutex<Vec<i16>>>,
    stream: Option<cpal::Stream>,
    sample_rate: u32,
}

impl Recorder {
    fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| anyhow::anyhow!("no mic"))?;
        let supported = device.default_input_config()?;
        let channels = supported.channels();
        let sample_rate = supported.sample_rate().0;
        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate: supported.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };
        let samples = Arc::new(Mutex::new(Vec::<i16>::with_capacity(sample_rate as usize * 30)));
        let samples_cb = samples.clone();
        let stream = device.build_input_stream(
            &stream_config,
            move |data: &[f32], _: &_| {
                let mut buf = samples_cb.lock();
                buf.extend(data.chunks(channels as usize).map(|frame| {
                    let mono = frame.iter().sum::<f32>() / channels as f32;
                    (mono * i16::MAX as f32) as i16
                }));
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )?;
        stream.play()?;
        Ok(Self { samples, stream: Some(stream), sample_rate })
    }

    fn stop(mut self) -> (Vec<i16>, u32) {
        drop(self.stream.take());
        let samples = std::mem::take(&mut *self.samples.lock());
        (samples, self.sample_rate)
    }
}

fn samples_to_wav(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1, sample_rate,
        bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::<u8>::new());
    {
        let mut w = WavWriter::new(&mut buf, spec)?;
        for &s in samples { w.write_sample(s)?; }
        w.finalize()?;
    }
    Ok(buf.into_inner())
}

fn transcribe(wav: Vec<u8>, cfg: &Config) -> Result<String> {
    let client = reqwest::blocking::Client::new();
    let part = reqwest::blocking::multipart::Part::bytes(wav)
        .file_name("audio.wav").mime_str("audio/wav")?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("model", cfg.whisper_model.clone())
        .part("file", part);
    let resp: WhisperResp = client.post(&cfg.whisper_url).multipart(form).send()?.json()?;
    Ok(resp.text.trim().to_string())
}

fn distill(text: &str, cfg: &Config) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120)).build()?;
    let body = serde_json::json!({
        "model": cfg.llm_model,
        "prompt": format!("{}{text}", cfg.prompt),
        "stream": false,
    });
    let resp: OllamaResp = client.post(&cfg.ollama_url).json(&body).send()?.json()?;
    Ok(resp.response.trim().to_string())
}

fn paste(text: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.text(text)?;
    Ok(())
}

fn is_too_short(samples: &[i16], sample_rate: u32) -> bool {
    samples.len() < sample_rate as usize / 2
}

fn process(samples: Vec<i16>, sample_rate: u32, cfg: Arc<Config>, status: Arc<Mutex<Option<String>>>) {
    let set = |s: Option<&str>| *status.lock() = s.map(str::to_owned);

    if is_too_short(&samples, sample_rate) {
        eprintln!("error: recording too short");
        set(None);
        return;
    }
    let wav = match samples_to_wav(&samples, sample_rate) {
        Ok(w) => w,
        Err(e) => { eprintln!("error: {e:#}"); set(None); return; }
    };
    println!("transcribing {} samples...", samples.len());
    let transcript = match transcribe(wav, &cfg) {
        Ok(t) => t,
        Err(e) => { eprintln!("error: {e:#}"); set(None); return; }
    };
    println!("raw: {transcript}");
    set(Some("Distilling..."));
    let distilled = match distill(&transcript, &cfg) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e:#}"); set(None); return; }
    };
    println!("out: {distilled}");
    if let Err(e) = paste(&distilled) { eprintln!("error: {e:#}"); }
    set(None);
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    mod config {
        use super::*;

        const VALID_TOML: &str = r#"
            whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
            ollama_url    = "http://localhost:11434/api/generate"
            whisper_model = "whisper-large-turbo"
            llm_model     = "qwen2.5:7b-instruct"
            prompt        = "Fix grammar.\n\n---\n"
        "#;

        #[test]
        fn valid_toml_parses() {
            let cfg = parse_config(VALID_TOML).unwrap();
            assert_eq!(cfg.llm_model, "qwen2.5:7b-instruct");
            assert_eq!(cfg.whisper_model, "whisper-large-turbo");
            assert_eq!(cfg.whisper_url, "http://localhost:8080/v1/audio/transcriptions");
            assert_eq!(cfg.ollama_url, "http://localhost:11434/api/generate");
            assert_eq!(cfg.prompt, "Fix grammar.\n\n---\n");
            assert_eq!(cfg.hotkey, None);
        }

        #[test]
        fn hotkey_field_parses() {
            let raw = r#"
                whisper_url   = "http://localhost:8080/v1/audio/transcriptions"
                ollama_url    = "http://localhost:11434/api/generate"
                whisper_model = "whisper-large-turbo"
                llm_model     = "qwen2.5:7b-instruct"
                hotkey        = "Alt+W"
                prompt        = "Fix.\n"
            "#;
            let cfg = parse_config(raw).unwrap();
            assert_eq!(cfg.hotkey.as_deref(), Some("Alt+W"));
        }

        #[test]
        fn missing_required_field_fails() {
            let raw = r#"
                whisper_url = "http://localhost:8080"
                ollama_url  = "http://localhost:11434"
            "#;
            assert!(parse_config(raw).is_err());
        }

        #[test]
        fn invalid_toml_fails() {
            assert!(parse_config("this is not toml :::").is_err());
        }
    }

    mod hotkey {
        use super::*;

        #[test]
        fn alt_q_parses() {
            assert!(parse_hotkey("Alt+Q").is_some());
        }

        #[test]
        fn ctrl_alt_r_parses() {
            assert!(parse_hotkey("Ctrl+Alt+R").is_some());
        }

        #[test]
        fn function_keys_parse() {
            assert!(parse_hotkey("F9").is_some());
            assert!(parse_hotkey("F12").is_some());
        }

        #[test]
        fn unknown_key_fails() {
            assert!(parse_hotkey("Alt+7").is_none());
            assert!(parse_hotkey("Foo+Q").is_none());
            assert!(parse_hotkey("").is_none());
        }

        #[test]
        fn default_hotkey_is_valid() {
            assert!(parse_hotkey(settings_dialog::DEFAULT_HOTKEY).is_some());
        }
    }

    mod wav {
        use super::*;

        const SAMPLE_RATE: u32 = 16_000;

        fn header_u16(wav: &[u8], offset: usize) -> u16 {
            u16::from_le_bytes([wav[offset], wav[offset + 1]])
        }

        fn header_u32(wav: &[u8], offset: usize) -> u32 {
            u32::from_le_bytes([wav[offset], wav[offset + 1], wav[offset + 2], wav[offset + 3]])
        }

        #[test]
        fn magic_bytes_are_correct() {
            let wav = samples_to_wav(&[0i16; 100], SAMPLE_RATE).unwrap();
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(&wav[8..12], b"WAVE");
            assert_eq!(&wav[12..16], b"fmt ");
            assert_eq!(&wav[36..40], b"data");
        }

        #[test]
        fn header_encodes_correct_format() {
            let wav = samples_to_wav(&[0i16; 100], SAMPLE_RATE).unwrap();
            assert_eq!(header_u16(&wav, 20), 1);  // PCM format
            assert_eq!(header_u16(&wav, 22), 1);  // mono
            assert_eq!(header_u32(&wav, 24), SAMPLE_RATE);
            assert_eq!(header_u16(&wav, 34), 16); // 16-bit
        }

        #[test]
        fn header_reflects_device_sample_rate() {
            let wav = samples_to_wav(&[0i16; 100], 48_000).unwrap();
            assert_eq!(header_u32(&wav, 24), 48_000);
        }

        #[test]
        fn empty_samples_is_valid() {
            let wav = samples_to_wav(&[], SAMPLE_RATE).unwrap();
            assert_eq!(&wav[0..4], b"RIFF");
            assert_eq!(header_u32(&wav, 40), 0);
        }

        #[test]
        fn samples_round_trip() {
            let original: Vec<i16> = (0..64).map(|i| (i * 100) as i16).collect();
            let wav = samples_to_wav(&original, SAMPLE_RATE).unwrap();
            let mut reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
            let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
            assert_eq!(decoded, original);
        }
    }

    mod recording {
        use super::*;

        const SAMPLE_RATE: u32 = 16_000;

        #[test]
        fn too_short_is_detected() {
            assert!(is_too_short(&vec![0i16; SAMPLE_RATE as usize / 2 - 1], SAMPLE_RATE));
        }

        #[test]
        fn at_threshold_is_accepted() {
            assert!(!is_too_short(&vec![0i16; SAMPLE_RATE as usize / 2], SAMPLE_RATE));
        }

        #[test]
        fn threshold_scales_with_sample_rate() {
            let rate = 48_000u32;
            assert!(is_too_short(&vec![0i16; rate as usize / 2 - 1], rate));
            assert!(!is_too_short(&vec![0i16; rate as usize / 2], rate));
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

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
    let show_logs_item = MenuItem::new("Show logs", true, None);
    let settings_item = MenuItem::new("Settings", true, None);
    let exit_item = MenuItem::new("Exit", true, None);
    tray_menu.append(&show_logs_item).expect("menu append");
    tray_menu.append(&settings_item).expect("menu append");
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

        // Sync icon + balloon with current status
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
            if ev.id == *exit_item.id() {
                std::process::exit(0);
            }
            if ev.id == *show_logs_item.id() {
                console_window::toggle();
            }
            if ev.id == *settings_item.id() {
                settings_dialog::open(&current_hotkey_str);
            }
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
                        let cfg = cfg.clone();
                        let status = tray_status.clone();
                        thread::spawn(move || process(samples, sample_rate, cfg, status));
                    }
                }
            }
        }
    });
}
