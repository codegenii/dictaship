use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::config::ModeConfig;

static OPEN:   AtomicBool = AtomicBool::new(false);
static RESULT: std::sync::Mutex<Option<SettingsResult>> = std::sync::Mutex::new(None);

pub struct SettingsResult {
    pub hotkey:    String,
    pub mode_name: String,
    pub modes:     Vec<ModeConfig>,
}

pub const DEFAULT_HOTKEY: &str = "Alt+R";

// Windows message/style constants
const WM_COMMAND:          u32   = 0x0111;
const WM_DESTROY:          u32   = 0x0002;
const WM_KEYDOWN:          u32   = 0x0100;
const WM_SYSKEYDOWN:       u32   = 0x0104;
const WS_CAPTION:          u32   = 0x00C00000;
const WS_SYSMENU:          u32   = 0x00080000;
const WS_CHILD:            u32   = 0x40000000;
const WS_VISIBLE:          u32   = 0x10000000;
const WS_BORDER:           u32   = 0x00800000;
const WS_VSCROLL:          u32   = 0x00200000;
const WS_EX_TOPMOST:       u32   = 0x00000008;
const WS_EX_DLGMODALFRAME: u32   = 0x00000001;
const WS_EX_CLIENTEDGE:    u32   = 0x00000200;
const BS_PUSHBUTTON:       u32   = 0x00000000;
const SS_SIMPLE:           u32   = 0x0B;
const ES_MULTILINE:        u32   = 0x0004;
const ES_AUTOVSCROLL:      u32   = 0x0040;
const ES_WANTRETURN:       u32   = 0x1000;
const CBS_DROPDOWNLIST:    u32   = 0x0003;
const CBS_HASSTRINGS:      u32   = 0x0200;
const CB_ADDSTRING:        u32   = 0x0143;
const CB_GETCURSEL:        u32   = 0x0147;
const CB_SETCURSEL:        u32   = 0x014E;
const CB_RESETCONTENT:     u32   = 0x014B;
const CBN_SELCHANGE:       u32   = 1;
const CW_USEDEFAULT:       i32   = 0x80000000u32 as i32;
const COLOR_BTNFACE:       isize = 15;

// Settings dialog control IDs
const ID_DISPLAY:      i32 = 101;
const ID_APPLY:        i32 = 102;
const ID_CANCEL:       i32 = 103;
const ID_DEFAULT:      i32 = 104;
const ID_MODE_COMBO:   i32 = 105;
const ID_EDIT_PRESET:  i32 = 106;

// Edit-preset dialog control IDs
const ID_EDIT_SAVE:    i32 = 201;
const ID_EDIT_CANCEL:  i32 = 202;

// Modifier-only virtual keys to skip when capturing a hotkey
const SKIP_VKEYS: &[usize] = &[0x10, 0x11, 0x12, 0x5B, 0x5C];

thread_local! {
    // Settings dialog state
    static PENDING:          RefCell<String>        = RefCell::new(String::new());
    static PENDING_MODE:     RefCell<String>        = RefCell::new(String::new());
    static MODES_DATA:       RefCell<Vec<ModeConfig>> = RefCell::new(Vec::new());
    static COMBO_HWND:       Cell<Hwnd>             = Cell::new(0);

    // Edit-preset dialog state
    static EDIT_NAME_HWND:   Cell<Hwnd>   = Cell::new(0);
    static EDIT_PROMPT_HWND: Cell<Hwnd>   = Cell::new(0);
    static EDIT_MODE_IDX:    Cell<usize>  = Cell::new(0);
    static EDIT_SAVED:       Cell<bool>   = Cell::new(false);
}

type Hwnd      = isize;
type Hinstance = isize;

#[repr(C)]
struct WndClassExW {
    cb_size:         u32,
    style:           u32,
    lpfn_wnd_proc:   unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cb_cls_extra:    i32,
    cb_wnd_extra:    i32,
    h_instance:      isize,
    h_icon:          isize,
    h_cursor:        isize,
    hbr_background:  isize,
    lpsz_menu_name:  *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm:       isize,
}

#[repr(C)]
struct Point { x: i32, y: i32 }

#[repr(C)]
struct Msg {
    hwnd:      Hwnd,
    message:   u32,
    w_param:   usize,
    l_param:   isize,
    time:      u32,
    pt:        Point,
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
    fn EnableWindow(hwnd: Hwnd, b_enable: i32) -> i32;
    fn GetDlgItem(dlg: Hwnd, id: i32) -> Hwnd;
    fn LoadCursorW(inst: Hwnd, name: *const u16) -> Hwnd;
    fn GetKeyState(vk: i32) -> i16;
    fn SetFocus(hwnd: Hwnd) -> Hwnd;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
    fn GetWindowTextW(hwnd: Hwnd, lp_string: *mut u16, n_max_count: i32) -> i32;
    fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
    fn InvalidateRect(hwnd: Hwnd, lp_rect: *const u8, b_erase: i32) -> i32;
    fn UpdateWindow(hwnd: Hwnd) -> i32;
    fn MapVirtualKeyW(u_code: u32, u_map_type: u32) -> u32;
    fn GetKeyNameTextW(l_param: i32, lp_string: *mut u16, cch_size: i32) -> i32;
    fn SendMessageW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize;
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

fn read_hwnd_text(hwnd: Hwnd) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 { return String::new(); }
        let mut buf = vec![0u16; len as usize + 1];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn rebuild_combo(combo: Hwnd) {
    unsafe { SendMessageW(combo, CB_RESETCONTENT, 0, 0); }
    MODES_DATA.with(|m| {
        for mode in m.borrow().iter() {
            let w = wide(&mode.name);
            unsafe { SendMessageW(combo, CB_ADDSTRING, 0, w.as_ptr() as isize); }
        }
    });
}

// ── Edit-preset nested dialog ────────────────────────────────────────────────

unsafe extern "system" fn edit_dlg_proc(hwnd: isize, msg: u32, wp: usize, _lp: isize) -> isize {
    if msg == WM_COMMAND {
        let id = (wp & 0xFFFF) as i32;
        if id == ID_EDIT_SAVE {
            let name   = read_hwnd_text(EDIT_NAME_HWND.with(|h| h.get()));
            let prompt = read_hwnd_text(EDIT_PROMPT_HWND.with(|h| h.get()))
                .replace("\r\n", "\n");
            let idx = EDIT_MODE_IDX.with(|i| i.get());
            MODES_DATA.with(|m| {
                if let Some(mode) = m.borrow_mut().get_mut(idx) {
                    mode.name   = name;
                    mode.prompt = prompt;
                }
            });
            EDIT_SAVED.with(|s| s.set(true));
            unsafe { DestroyWindow(hwnd); }
            return 0;
        }
        if id == ID_EDIT_CANCEL {
            EDIT_SAVED.with(|s| s.set(false));
            unsafe { DestroyWindow(hwnd); }
            return 0;
        }
    }
    if msg == WM_DESTROY {
        unsafe { PostQuitMessage(0); }
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, _lp) }
}

fn run_edit_dialog(parent: Hwnd, mode_idx: usize) {
    EDIT_MODE_IDX.with(|i| i.set(mode_idx));
    EDIT_SAVED.with(|s| s.set(false));

    let (mode_name, mode_prompt) = MODES_DATA.with(|m| {
        m.borrow().get(mode_idx)
            .map(|m| (m.name.clone(), m.prompt.clone()))
            .unwrap_or_default()
    });

    unsafe {
        EnableWindow(parent, 0);

        let hinst    = GetModuleHandleW(std::ptr::null());
        let cls_name = wide("DictaphileModeEdit");
        let wc = WndClassExW {
            cb_size:         std::mem::size_of::<WndClassExW>() as u32,
            style:           0,
            lpfn_wnd_proc:   edit_dlg_proc,
            cb_cls_extra:    0, cb_wnd_extra: 0,
            h_instance:      hinst, h_icon: 0,
            h_cursor:        LoadCursorW(0, 32512 as *const u16),
            hbr_background:  COLOR_BTNFACE + 1,
            lpsz_menu_name:  std::ptr::null(),
            lpsz_class_name: cls_name.as_ptr(),
            h_icon_sm:       0,
        };
        RegisterClassExW(&wc); // ignore return; class may already exist

        let title = wide("Dictaphile \u{2013} Edit Preset");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_DLGMODALFRAME,
            cls_name.as_ptr(), title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT, CW_USEDEFAULT, 430, 320,
            0, 0, hinst, std::ptr::null(),
        );
        if hwnd == 0 { EnableWindow(parent, 1); return; }

        CreateWindowExW(0, wide("STATIC").as_ptr(), wide("Mode name:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 14, 395, 18, hwnd, 0, hinst, std::ptr::null());

        let name_edit = CreateWindowExW(
            WS_EX_CLIENTEDGE, wide("EDIT").as_ptr(), wide(&mode_name).as_ptr(),
            WS_CHILD | WS_VISIBLE,
            12, 34, 395, 24, hwnd, 0, hinst, std::ptr::null(),
        );
        EDIT_NAME_HWND.with(|h| h.set(name_edit));

        CreateWindowExW(0, wide("STATIC").as_ptr(), wide("Prompt:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 70, 395, 18, hwnd, 0, hinst, std::ptr::null());

        // Convert \n to \r\n for the edit control
        let display_prompt = mode_prompt.replace('\n', "\r\n");
        let prompt_edit = CreateWindowExW(
            WS_EX_CLIENTEDGE, wide("EDIT").as_ptr(), wide(&display_prompt).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL
                | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN,
            12, 90, 395, 170, hwnd, 0, hinst, std::ptr::null(),
        );
        EDIT_PROMPT_HWND.with(|h| h.set(prompt_edit));

        CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Save").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            12, 274, 80, 26, hwnd, ID_EDIT_SAVE as _, hinst, std::ptr::null());

        CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Cancel").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            327, 274, 80, 26, hwnd, ID_EDIT_CANCEL as _, hinst, std::ptr::null());

        SetFocus(name_edit);

        let mut msg = Msg {
            hwnd: 0, message: 0, w_param: 0, l_param: 0,
            time: 0, pt: Point { x: 0, y: 0 }, l_private: 0,
        };
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        EnableWindow(parent, 1);
        SetFocus(parent);
    }

    // If user saved, sync PENDING_MODE and rebuild combo
    if EDIT_SAVED.with(|s| s.get()) {
        let new_name = MODES_DATA.with(|m| {
            m.borrow().get(mode_idx).map(|m| m.name.clone()).unwrap_or_default()
        });
        PENDING_MODE.with(|p| *p.borrow_mut() = new_name.clone());
        let combo = COMBO_HWND.with(|c| c.get());
        rebuild_combo(combo);
        let new_idx = MODES_DATA.with(|m| {
            m.borrow().iter().position(|m| m.name == new_name).unwrap_or(mode_idx)
        });
        unsafe { SendMessageW(combo, CB_SETCURSEL, new_idx, 0); }
    }
}

// ── Settings dialog ──────────────────────────────────────────────────────────

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
        let id    = (wp & 0xFFFF) as i32;
        let notif = ((wp >> 16) & 0xFFFF) as u32;

        if id == ID_MODE_COMBO && notif == CBN_SELCHANGE {
            let combo = COMBO_HWND.with(|c| c.get());
            let sel = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) } as usize;
            MODES_DATA.with(|m| {
                if let Some(mode) = m.borrow().get(sel) {
                    PENDING_MODE.with(|p| *p.borrow_mut() = mode.name.clone());
                }
            });
            return 0;
        }
        if id == ID_EDIT_PRESET {
            let combo = COMBO_HWND.with(|c| c.get());
            let sel = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) } as usize;
            run_edit_dialog(hwnd, sel);
            return 0;
        }
        if id == ID_APPLY {
            let hotkey    = PENDING.with(|p| p.borrow().clone());
            let mode_name = PENDING_MODE.with(|p| p.borrow().clone());
            let modes     = MODES_DATA.with(|m| m.borrow().clone());
            *RESULT.lock().unwrap() = Some(SettingsResult { hotkey, mode_name, modes });
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

fn run_dialog(current_hotkey: String, modes: Vec<ModeConfig>, active_mode: String) {
    PENDING.with(|p| *p.borrow_mut() = current_hotkey.clone());
    PENDING_MODE.with(|p| *p.borrow_mut() = active_mode.clone());
    MODES_DATA.with(|m| *m.borrow_mut() = modes.clone());

    unsafe {
        let hinst    = GetModuleHandleW(std::ptr::null());
        let cls_name = wide("DictaphileSettings");

        let wc = WndClassExW {
            cb_size:         std::mem::size_of::<WndClassExW>() as u32,
            style:           0,
            lpfn_wnd_proc:   wnd_proc,
            cb_cls_extra:    0, cb_wnd_extra: 0,
            h_instance:      hinst, h_icon: 0,
            h_cursor:        LoadCursorW(0, 32512 as *const u16),
            hbr_background:  COLOR_BTNFACE + 1,
            lpsz_menu_name:  std::ptr::null(),
            lpsz_class_name: cls_name.as_ptr(),
            h_icon_sm:       0,
        };
        RegisterClassExW(&wc);

        let title = wide("Dictaphile \u{2013} Settings");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_DLGMODALFRAME,
            cls_name.as_ptr(), title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT, CW_USEDEFAULT, 330, 250,
            0, 0, hinst, std::ptr::null(),
        );
        if hwnd == 0 { OPEN.store(false, Ordering::SeqCst); return; }

        // Instruction
        CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide("Press a new hotkey combo, then click Apply").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 12, 296, 18, hwnd, 0, hinst, std::ptr::null());

        // Hotkey label
        CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide("Recording hotkey:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 36, 296, 18, hwnd, 0, hinst, std::ptr::null());

        // Hotkey display (read via keyboard events, not directly editable)
        CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide(&current_hotkey).as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE | WS_BORDER,
            12, 56, 296, 26, hwnd, ID_DISPLAY as _, hinst, std::ptr::null());

        // Mode label
        CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide("Distillation mode:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 98, 296, 18, hwnd, 0, hinst, std::ptr::null());

        // Mode combo box (h=120 gives room for ~5 items in the dropdown)
        let combo = CreateWindowExW(
            0, wide("COMBOBOX").as_ptr(), std::ptr::null(),
            WS_CHILD | WS_VISIBLE | CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            12, 118, 218, 120,
            hwnd, ID_MODE_COMBO as _, hinst, std::ptr::null(),
        );
        for mode in &modes {
            let w = wide(&mode.name);
            SendMessageW(combo, CB_ADDSTRING, 0, w.as_ptr() as isize);
        }
        let sel_idx = modes.iter().position(|m| m.name == active_mode).unwrap_or(0);
        SendMessageW(combo, CB_SETCURSEL, sel_idx, 0);
        COMBO_HWND.with(|c| c.set(combo));

        // Edit preset button
        CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Edit").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            238, 118, 70, 26, hwnd, ID_EDIT_PRESET as _, hinst, std::ptr::null());

        // Action buttons
        CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Apply").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            10, 165, 80, 26, hwnd, ID_APPLY as _, hinst, std::ptr::null());
        CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Default").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            110, 165, 80, 26, hwnd, ID_DEFAULT as _, hinst, std::ptr::null());
        CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Cancel").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            210, 165, 80, 26, hwnd, ID_CANCEL as _, hinst, std::ptr::null());

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

pub fn open(current_hotkey: &str, modes: Vec<ModeConfig>, active_mode: &str) {
    if OPEN.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    let current_hotkey = current_hotkey.to_string();
    let active_mode    = active_mode.to_string();
    thread::spawn(move || run_dialog(current_hotkey, modes, active_mode));
}

pub fn take_result() -> Option<SettingsResult> {
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

    #[test]
    fn settings_result_carries_all_fields() {
        let result = SettingsResult {
            hotkey:    "Alt+R".to_string(),
            mode_name: "Distill".to_string(),
            modes: vec![
                ModeConfig { name: "Distill".to_string(), prompt: "p".to_string() },
            ],
        };
        assert_eq!(result.hotkey, "Alt+R");
        assert_eq!(result.mode_name, "Distill");
        assert_eq!(result.modes.len(), 1);
    }
}
