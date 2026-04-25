use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

static OPEN:   AtomicBool = AtomicBool::new(false);
static RESULT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub const DEFAULT_HOTKEY: &str = "Alt+R";

// Windows message/style constants
const WM_COMMAND:         u32   = 0x0111;
const WM_DESTROY:         u32   = 0x0002;
const WM_KEYDOWN:         u32   = 0x0100;
const WM_SYSKEYDOWN:      u32   = 0x0104;
const WS_CAPTION:         u32   = 0x00C00000;
const WS_SYSMENU:         u32   = 0x00080000;
const WS_CHILD:           u32   = 0x40000000;
const WS_VISIBLE:         u32   = 0x10000000;
const WS_BORDER:          u32   = 0x00800000;
const WS_EX_TOPMOST:      u32   = 0x00000008;
const WS_EX_DLGMODALFRAME:u32   = 0x00000001;
const BS_PUSHBUTTON:      u32   = 0x00000000;
const SS_SIMPLE:          u32   = 0x0B;
const CW_USEDEFAULT:      i32   = 0x80000000u32 as i32;
const COLOR_BTNFACE:      isize = 15;

const ID_DISPLAY: i32 = 101;
const ID_APPLY:   i32 = 102;
const ID_CANCEL:  i32 = 103;
const ID_DEFAULT: i32 = 104;

// Modifier-only virtual keys to skip when capturing a hotkey
const SKIP_VKEYS: &[usize] = &[0x10, 0x11, 0x12, 0x5B, 0x5C];

thread_local! {
    static PENDING: RefCell<String> = RefCell::new(String::new());
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
        let hinst    = GetModuleHandleW(std::ptr::null());
        let cls_name = wide("PartizanSettings");

        let wc = WndClassExW {
            cb_size:         std::mem::size_of::<WndClassExW>() as u32,
            style:           0,
            lpfn_wnd_proc:   wnd_proc,
            cb_cls_extra:    0,
            cb_wnd_extra:    0,
            h_instance:      hinst,
            h_icon:          0,
            h_cursor:        LoadCursorW(0, 32512 as *const u16), // IDC_ARROW
            hbr_background:  COLOR_BTNFACE + 1,
            lpsz_menu_name:  std::ptr::null(),
            lpsz_class_name: cls_name.as_ptr(),
            h_icon_sm:       0,
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

        // Instruction label
        let instr = wide("Press a new hotkey combo, then click Apply");
        CreateWindowExW(
            0, wide("STATIC").as_ptr(), instr.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_SIMPLE,
            12, 14, 280, 18, hwnd, 0, hinst, std::ptr::null(),
        );

        // Field label
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
