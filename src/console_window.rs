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
