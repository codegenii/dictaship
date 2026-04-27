use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

const ERROR_ALREADY_EXISTS: u32 = 183;
const MB_OK:              u32 = 0x0000_0000;
const MB_ICONINFORMATION: u32 = 0x0000_0040;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateMutexW(lp_security: *const u8, b_initial_owner: i32, lp_name: *const u16) -> isize;
    fn GetLastError() -> u32;
    fn CloseHandle(h_object: isize) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, u_type: u32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain([0u16]).collect()
}

/// Held for the lifetime of the process. Releasing it allows a new instance to start.
pub struct InstanceGuard(isize);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { CloseHandle(self.0); }
        }
    }
}

fn acquire_named(name: &str, show_dialog: bool) -> Option<InstanceGuard> {
    let wname  = wide(name);
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wname.as_ptr()) };
    if handle != 0 && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if show_dialog {
            let text    = wide("Dictaphile is already running.\nCheck the system tray.");
            let caption = wide("Dictaphile");
            unsafe { MessageBoxW(0, text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONINFORMATION); }
        }
        unsafe { CloseHandle(handle); }
        return None;
    }
    // handle == 0 means CreateMutexW failed entirely — let the app proceed rather than block.
    Some(InstanceGuard(handle))
}

/// Returns a guard that must be kept alive for the process duration.
/// Returns None and shows a message box if another instance is already running.
pub fn acquire() -> Option<InstanceGuard> {
    acquire_named("Local\\DictaphileApp", true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_succeeds_after_guard_dropped() {
        { let _g = acquire_named("Local\\DictaphileTest_drop", false); }
        assert!(acquire_named("Local\\DictaphileTest_drop", false).is_some());
    }
}
