use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::config::{load_settings_size, save_settings_size_to_config, ModeConfig};

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
const WM_SIZE:             u32   = 0x0005;
const WM_GETMINMAXINFO:    u32   = 0x0024;
const WM_DPICHANGED:       u32   = 0x02E0;
const SWP_NOMOVE:          u32   = 0x0002;
const SWP_NOZORDER:        u32   = 0x0004;
const SWP_NOACTIVATE:      u32   = 0x0010;
const WS_CAPTION:          u32   = 0x00C00000;
const WS_SYSMENU:          u32   = 0x00080000;
const WS_THICKFRAME:       u32   = 0x00040000;
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
const CBN_CLOSEUP:         u32   = 8;
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

// Add/Delete mode buttons
const ID_ADD_MODE:     i32 = 107;
const ID_DELETE_MODE:  i32 = 108;

// Modifier-only virtual keys to skip when capturing a hotkey
const SKIP_VKEYS: &[usize] = &[0x10, 0x11, 0x12, 0x5B, 0x5C];

thread_local! {
    // Settings dialog state
    static PENDING:          RefCell<String>           = RefCell::new(String::new());
    static PENDING_MODE:     RefCell<String>           = RefCell::new(String::new());
    static MODES_DATA:       RefCell<Vec<ModeConfig>>  = RefCell::new(Vec::new());
    static COMBO_HWND:       Cell<Hwnd>                = Cell::new(0);
    static DISPLAY_HWND:     Cell<Hwnd>                = Cell::new(0);
    static SETTINGS_CTRLS:  RefCell<Option<SettingsControls>> = RefCell::new(None);
    static CURRENT_DPI:      Cell<u32>                 = Cell::new(96);

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
struct Rect { left: i32, top: i32, right: i32, bottom: i32 }

#[repr(C)]
struct MinMaxInfo {
    pt_reserved:       Point,
    pt_max_size:       Point,
    pt_max_position:   Point,
    pt_min_track_size: Point,
    pt_max_track_size: Point,
}

#[derive(Clone)]
struct SettingsControls {
    instruction:  Hwnd,
    hotkey_label: Hwnd,
    mode_label:   Hwnd,
    edit_btn:     Hwnd,
    add_btn:      Hwnd,
    del_btn:      Hwnd,
    apply_btn:    Hwnd,
    default_btn:  Hwnd,
    cancel_btn:   Hwnd,
}

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
    fn LoadCursorW(inst: Hwnd, name: *const u16) -> Hwnd;
    fn GetKeyState(vk: i32) -> i16;
    fn SetFocus(hwnd: Hwnd) -> Hwnd;
    fn SetWindowTextW(hwnd: Hwnd, text: *const u16) -> i32;
    fn GetWindowTextW(hwnd: Hwnd, lp_string: *mut u16, n_max_count: i32) -> i32;
    fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
    fn UpdateWindow(hwnd: Hwnd) -> i32;
    fn MapVirtualKeyW(u_code: u32, u_map_type: u32) -> u32;
    fn GetKeyNameTextW(l_param: i32, lp_string: *mut u16, cch_size: i32) -> i32;
    fn SendMessageW(hwnd: Hwnd, msg: u32, wp: usize, lp: isize) -> isize;
    fn GetDC(hwnd: Hwnd) -> isize;
    fn ReleaseDC(hwnd: Hwnd, hdc: isize) -> i32;
    fn FillRect(hdc: isize, lp_rc: *const Rect, h_brush: isize) -> i32;
    fn GetClientRect(hwnd: Hwnd, lp_rect: *mut Rect) -> i32;
    fn GetWindowRect(hwnd: Hwnd, lp_rect: *mut Rect) -> i32;
    fn GetSysColorBrush(n_index: i32) -> isize;
    fn MoveWindow(hwnd: Hwnd, x: i32, y: i32, w: i32, h: i32, repaint: i32) -> i32;
    fn InvalidateRect(hwnd: Hwnd, lp_rect: *const Rect, b_erase: i32) -> i32;
    fn SetWindowPos(hwnd: Hwnd, hwnd_insert_after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn GetDpiForSystem() -> u32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
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

fn scale(px: i32, dpi: u32) -> i32 {
    px * dpi as i32 / 96
}

fn layout_settings(client_w: i32, client_h: i32) {
    let snap = SETTINGS_CTRLS.with(|c| c.borrow().clone());
    let Some(c) = snap else { return; };
    let dpi   = CURRENT_DPI.with(|d| d.get());
    let s     = |px: i32| scale(px, dpi);
    let m     = s(12);
    let cw    = client_w - 2 * m;
    let btn_h = s(26);
    let btn_y = client_h - m - btn_h;
    unsafe {
        MoveWindow(c.instruction,                         m,           s(12),  cw,     s(18),  1);
        MoveWindow(c.hotkey_label,                        m,           s(36),  cw,     s(18),  1);
        MoveWindow(DISPLAY_HWND.with(|h| h.get()),        m,           s(56),  cw,     s(26),  1);
        MoveWindow(c.mode_label,                          m,           s(98),  cw,     s(18),  1);
        MoveWindow(COMBO_HWND.with(|h| h.get()),          m,           s(118), cw,     s(120), 1);
        MoveWindow(c.edit_btn,                            m,           s(148), s(68),  btn_h,  1);
        MoveWindow(c.add_btn,                             m + s(76),   s(148), s(68),  btn_h,  1);
        MoveWindow(c.del_btn,                             m + s(152),  s(148), s(68),  btn_h,  1);
        MoveWindow(c.apply_btn,                           m,           btn_y,  s(80),  btn_h,  1);
        MoveWindow(c.default_btn,                         m + s(90),   btn_y,  s(80),  btn_h,  1);
        MoveWindow(c.cancel_btn,                          m + s(180),  btn_y,  s(80),  btn_h,  1);
        InvalidateRect(DISPLAY_HWND.with(|h| h.get()), std::ptr::null(), 1);
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

// Erase the entire hotkey display control before writing new text.
// SS_SIMPLE only invalidates the bounding rect of the new (shorter) text, so
// switching from a longer value like "Alt+R" to a shorter one like "A" would
// leave the tail ("lt+R") visible without this full-area erase.
fn refresh_display(text: &str) {
    let display = DISPLAY_HWND.with(|h| h.get());
    if display == 0 { return; }
    unsafe {
        let dc = GetDC(display);
        let mut rc = Rect { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(display, &mut rc);
        FillRect(dc, &rc, GetSysColorBrush(COLOR_BTNFACE as i32));
        ReleaseDC(display, dc);
        SetWindowTextW(display, wide(text).as_ptr());
        UpdateWindow(display);
    }
}

// ── Edit-preset nested dialog ────────────────────────────────────────────────

unsafe extern "system" fn edit_dlg_proc(hwnd: isize, msg: u32, wp: usize, _lp: isize) -> isize {
    if msg == WM_COMMAND {
        let id = (wp & 0xFFFF) as i32;
        if id == ID_EDIT_SAVE {
            let name   = read_hwnd_text(EDIT_NAME_HWND.with(|h| h.get()));
            let prompt = read_hwnd_text(EDIT_PROMPT_HWND.with(|h| h.get()))
                .replace("\r\n", "\n");
            if name.is_empty() { return 0; }
            let idx = EDIT_MODE_IDX.with(|i| i.get());
            MODES_DATA.with(|m| {
                let mut modes = m.borrow_mut();
                if idx == usize::MAX {
                    modes.push(ModeConfig { name: name.clone(), prompt });
                } else if let Some(mode) = modes.get_mut(idx) {
                    mode.name   = name.clone();
                    mode.prompt = prompt;
                }
            });
            // Make the saved name visible to run_edit_dialog's post-save block
            PENDING_MODE.with(|p| *p.borrow_mut() = name);
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
            CW_USEDEFAULT, CW_USEDEFAULT, 430, 380,
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

    // If user saved, rebuild combo and select the saved/new mode.
    // PENDING_MODE was already updated by edit_dlg_proc to the saved name.
    if EDIT_SAVED.with(|s| s.get()) {
        let sel_name = PENDING_MODE.with(|p| p.borrow().clone());
        let combo = COMBO_HWND.with(|c| c.get());
        rebuild_combo(combo);
        let sel_idx = MODES_DATA.with(|m| {
            m.borrow().iter().position(|m| m.name == sel_name).unwrap_or(0)
        });
        unsafe { SendMessageW(combo, CB_SETCURSEL, sel_idx, 0); }
    }
}

// ── Settings dialog ──────────────────────────────────────────────────────────

unsafe extern "system" fn wnd_proc(hwnd: isize, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_SIZE {
        let w = (lp as usize & 0xFFFF) as i32;
        let h = ((lp as usize >> 16) & 0xFFFF) as i32;
        layout_settings(w, h);
        return 0;
    }
    if msg == WM_GETMINMAXINFO {
        let dpi = CURRENT_DPI.with(|d| d.get());
        unsafe {
            let info = &mut *(lp as *mut MinMaxInfo);
            info.pt_min_track_size = Point { x: scale(330, dpi), y: scale(270, dpi) };
        }
        return 0;
    }
    if msg == WM_DPICHANGED {
        let new_dpi = (wp & 0xFFFF) as u32;
        CURRENT_DPI.with(|d| d.set(new_dpi));
        unsafe {
            let rect = &*(lp as *const Rect);
            SetWindowPos(
                hwnd, 0,
                rect.left, rect.top,
                rect.right  - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        return 0;
    }
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
                refresh_display(&hotkey_str);
            }
        }
        return 0;
    }
    if msg == WM_COMMAND {
        let id    = (wp & 0xFFFF) as i32;
        let notif = ((wp >> 16) & 0xFFFF) as u32;

        if id == ID_MODE_COMBO {
            if notif == CBN_SELCHANGE {
                let combo = COMBO_HWND.with(|c| c.get());
                let sel = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) } as usize;
                MODES_DATA.with(|m| {
                    if let Some(mode) = m.borrow().get(sel) {
                        PENDING_MODE.with(|p| *p.borrow_mut() = mode.name.clone());
                    }
                });
            }
            if notif == CBN_CLOSEUP {
                // Return focus to the dialog so subsequent key presses are
                // captured as hotkey input rather than going to the combo box.
                unsafe { SetFocus(hwnd); }
            }
            return 0;
        }
        if id == ID_EDIT_PRESET {
            let combo = COMBO_HWND.with(|c| c.get());
            let sel = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) } as usize;
            run_edit_dialog(hwnd, sel);
            return 0;
        }
        if id == ID_ADD_MODE {
            run_edit_dialog(hwnd, usize::MAX); // usize::MAX = "new mode" sentinel
            return 0;
        }
        if id == ID_DELETE_MODE {
            let n = MODES_DATA.with(|m| m.borrow().len());
            if n <= 1 { return 0; } // always keep at least one mode
            let combo = COMBO_HWND.with(|c| c.get());
            let sel = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) } as usize;
            MODES_DATA.with(|m| m.borrow_mut().remove(sel));
            let new_sel = if sel > 0 { sel - 1 } else { 0 };
            let new_name = MODES_DATA.with(|m| {
                m.borrow().get(new_sel).map(|m| m.name.clone()).unwrap_or_default()
            });
            PENDING_MODE.with(|p| *p.borrow_mut() = new_name);
            rebuild_combo(combo);
            unsafe { SendMessageW(combo, CB_SETCURSEL, new_sel, 0); }
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
            refresh_display(DEFAULT_HOTKEY);
            unsafe { SetFocus(hwnd); }
            return 0;
        }
        if id == ID_CANCEL {
            unsafe { DestroyWindow(hwnd); }
            return 0;
        }
    }
    if msg == WM_DESTROY {
        unsafe {
            let dpi = CURRENT_DPI.with(|d| d.get());
            let mut rc = Rect { left: 0, top: 0, right: 0, bottom: 0 };
            GetWindowRect(hwnd, &mut rc);
            let w_phys = (rc.right  - rc.left) as i32;
            let h_phys = (rc.bottom - rc.top)  as i32;
            // Convert physical → logical so the saved size is DPI-independent
            let w_log = (w_phys * 96 / dpi as i32) as u32;
            let h_log = (h_phys * 96 / dpi as i32) as u32;
            save_settings_size_to_config(w_log, h_log);
        }
        SETTINGS_CTRLS.with(|c| *c.borrow_mut() = None);
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

        // Scale logical window size to physical pixels for the system DPI.
        // WM_DPICHANGED handles subsequent moves to monitors with different DPI.
        let sys_dpi = GetDpiForSystem();
        CURRENT_DPI.with(|d| d.set(sys_dpi));
        let (init_w_log, init_h_log) = load_settings_size();
        let init_w = scale(init_w_log as i32, sys_dpi);
        let init_h = scale(init_h_log as i32, sys_dpi);

        let title = wide("Dictaphile \u{2013} Settings");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            cls_name.as_ptr(), title.as_ptr(),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE | WS_THICKFRAME,
            CW_USEDEFAULT, CW_USEDEFAULT, init_w, init_h,
            0, 0, hinst, std::ptr::null(),
        );
        if hwnd == 0 { OPEN.store(false, Ordering::SeqCst); return; }

        // Refine to the actual monitor DPI (may differ from system DPI on multi-monitor setups)
        let mon_dpi = GetDpiForWindow(hwnd);
        if mon_dpi > 0 && mon_dpi != sys_dpi {
            CURRENT_DPI.with(|d| d.set(mon_dpi));
            let w = scale(init_w_log as i32, mon_dpi);
            let h = scale(init_h_log as i32, mon_dpi);
            SetWindowPos(hwnd, 0, 0, 0, w, h, SWP_NOZORDER | SWP_NOMOVE | SWP_NOACTIVATE);
        }

        // Instruction
        let instruction = CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide("Press a new hotkey combo, then click Apply").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 12, 100, 18, hwnd, 0, hinst, std::ptr::null());

        // Hotkey label
        let hotkey_label = CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide("Recording hotkey:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 36, 100, 18, hwnd, 0, hinst, std::ptr::null());

        // Hotkey display (read via keyboard events, not directly editable)
        let display = CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide(&current_hotkey).as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE | WS_BORDER,
            12, 56, 100, 26, hwnd, ID_DISPLAY as _, hinst, std::ptr::null());
        DISPLAY_HWND.with(|h| h.set(display));

        // Mode label
        let mode_label = CreateWindowExW(0, wide("STATIC").as_ptr(),
            wide("Distillation mode:").as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 98, 100, 18, hwnd, 0, hinst, std::ptr::null());

        // Mode combo box (h=120 gives room for ~5 items in the dropdown)
        let combo = CreateWindowExW(
            0, wide("COMBOBOX").as_ptr(), std::ptr::null(),
            WS_CHILD | WS_VISIBLE | CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_VSCROLL,
            12, 118, 100, 120,
            hwnd, ID_MODE_COMBO as _, hinst, std::ptr::null(),
        );
        for mode in &modes {
            let w = wide(&mode.name);
            SendMessageW(combo, CB_ADDSTRING, 0, w.as_ptr() as isize);
        }
        let sel_idx = modes.iter().position(|m| m.name == active_mode).unwrap_or(0);
        SendMessageW(combo, CB_SETCURSEL, sel_idx, 0);
        COMBO_HWND.with(|c| c.set(combo));

        // Mode action buttons: Edit | Add | Delete
        let edit_btn = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Edit").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            12, 148, 68, 26, hwnd, ID_EDIT_PRESET as _, hinst, std::ptr::null());
        let add_btn = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Add").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            88, 148, 68, 26, hwnd, ID_ADD_MODE as _, hinst, std::ptr::null());
        let del_btn = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Delete").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            164, 148, 68, 26, hwnd, ID_DELETE_MODE as _, hinst, std::ptr::null());

        // Apply / Default / Cancel
        let apply_btn = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Apply").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            12, 195, 80, 26, hwnd, ID_APPLY as _, hinst, std::ptr::null());
        let default_btn = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Default").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            102, 195, 80, 26, hwnd, ID_DEFAULT as _, hinst, std::ptr::null());
        let cancel_btn = CreateWindowExW(0, wide("BUTTON").as_ptr(), wide("Cancel").as_ptr(),
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            192, 195, 80, 26, hwnd, ID_CANCEL as _, hinst, std::ptr::null());

        // Wire up resize layout — apply initial positions based on actual client size
        SETTINGS_CTRLS.with(|ctrls_cell| {
            *ctrls_cell.borrow_mut() = Some(SettingsControls {
                instruction, hotkey_label, mode_label,
                edit_btn, add_btn, del_btn,
                apply_btn, default_btn, cancel_btn,
            });
        });
        let mut rc = Rect { left: 0, top: 0, right: 0, bottom: 0 };
        GetClientRect(hwnd, &mut rc);
        layout_settings(rc.right - rc.left, rc.bottom - rc.top);

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
    fn dpi_scale_at_100pct_is_identity() {
        assert_eq!(scale(100, 96), 100);
        assert_eq!(scale(480, 96), 480);
        assert_eq!(scale(290, 96), 290);
        assert_eq!(scale(0,   96), 0);
    }

    #[test]
    fn dpi_scale_at_200pct_doubles() {
        assert_eq!(scale(100, 192), 200);
        assert_eq!(scale(480, 192), 960);
        assert_eq!(scale(290, 192), 580);
    }

    #[test]
    fn dpi_scale_at_150pct() {
        assert_eq!(scale(100, 144), 150);
        assert_eq!(scale(480, 144), 720);
    }

    #[test]
    fn dpi_scale_round_trips_via_unscale() {
        for dpi in [96u32, 120, 144, 192] {
            let logical = 480i32;
            let physical = scale(logical, dpi);
            // physical * 96 / dpi should recover logical
            assert_eq!(physical * 96 / dpi as i32, logical, "dpi={dpi}");
        }
    }

    #[test]
    fn dpi_scale_zero_input() {
        for dpi in [96u32, 120, 144, 192] {
            assert_eq!(scale(0, dpi), 0, "dpi={dpi}");
        }
    }

    #[test]
    fn dpi_scale_large_realistic_value() {
        // 8K display width (7680 logical px) at common DPI settings
        assert_eq!(scale(7680, 96),  7680);
        assert_eq!(scale(7680, 192), 15360);
        assert_eq!(scale(7680, 144), 11520);
    }
}
