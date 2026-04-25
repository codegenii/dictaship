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
