//! Tray icon: Quit / Settings / Pause.

#[cfg(windows)]
pub struct Tray {
    hwnd: windows::Win32::Foundation::HWND,
    added: bool,
}

#[cfg(windows)]
impl Tray {
    pub fn new(hwnd: windows::Win32::Foundation::HWND) -> Self {
        Self { hwnd, added: false }
    }

    pub fn add(&mut self) {
        use windows::Win32::UI::Shell::{
            Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NOTIFYICONDATAW,
        };
        use windows::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};
        unsafe {
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_TIP | NIF_ICON;
            nid.uCallbackMessage = super::window::WM_TRAY;
            nid.hIcon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
            let tip: Vec<u16> = "Hytte\0".encode_utf16().collect();
            let n = tip.len().min(127);
            nid.szTip[..n].copy_from_slice(&tip[..n]);
            if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                self.added = true;
            }
        }
    }

    pub fn remove(&mut self) {
        use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIM_DELETE, NOTIFYICONDATAW};
        unsafe {
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            self.added = false;
        }
    }

    pub fn popup(&self, paused: bool, autostart: bool) {
        use windows::Win32::UI::WindowsAndMessaging::*;
        use windows::core::w;
        unsafe {
            let menu = CreatePopupMenu().unwrap_or_default();
            if menu.0.is_null() {
                return;
            }
            let chk = |on: bool| if on { MF_STRING | MF_CHECKED } else { MF_STRING };
            let _ = AppendMenuW(menu, chk(paused), 10, w!("Pause"));
            let _ = AppendMenuW(menu, chk(autostart), 13, w!("Launch at startup"));
            let _ = AppendMenuW(menu, MF_STRING, 11, w!("Open settings folder"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, 12, w!("Quit"));
            let mut pt = windows::Win32::Foundation::POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, None, self.hwnd, None);
            let _ = DestroyMenu(menu);
        }
    }
}

#[cfg(not(windows))]
pub struct Tray;
#[cfg(not(windows))]
impl Tray {
    pub fn new() -> Self {
        Self
    }
    pub fn add(&mut self) {}
    pub fn remove(&mut self) {}
    pub fn popup(&self, _paused: bool, _autostart: bool) {}
}
