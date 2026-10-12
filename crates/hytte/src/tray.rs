//! Tray icon and its menu. The glyph follows the taskbar's light/dark setting.

/// Windows' "system" (taskbar) light mode, which is separate from the apps' light mode.
#[cfg(windows)]
fn taskbar_light() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut v = 0u32;
    let mut n = std::mem::size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut _ as *mut _),
            Some(&mut n),
        )
        .is_ok()
            && v == 1
    }
}

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
        use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIM_ADD};
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &self.data()).as_bool() {
                self.added = true;
            }
        }
    }

    /// Swaps the glyph after the taskbar theme changed (`WM_SETTINGCHANGE`).
    pub fn refresh(&self) {
        use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIM_MODIFY};
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data());
        }
    }

    /// Icon, tooltip and callback: everything `add` and `refresh` send.
    fn data(&self) -> windows::Win32::UI::Shell::NOTIFYICONDATAW {
        use windows::core::PCWSTR;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NOTIFYICONDATAW};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, LoadIconW, LoadImageW, HICON, IDI_APPLICATION, IMAGE_ICON, LR_SHARED,
            SM_CXSMICON,
        };
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_TIP | NIF_ICON;
        nid.uCallbackMessage = super::window::WM_TRAY;
        // Resources from hytte.rc: 2 = dark glyph (light taskbar), 3 = white glyph (dark taskbar).
        // Loaded at the small-icon size so the 16/20/24 px frame is used, not a scaled one; shared,
        // so refreshing on every WM_SETTINGCHANGE does not leak handles.
        let id = if taskbar_light() { 2usize } else { 3 };
        unsafe {
            let s = GetSystemMetrics(SM_CXSMICON);
            nid.hIcon = GetModuleHandleW(None)
                .ok()
                .and_then(|m| {
                    LoadImageW(Some(m.into()), PCWSTR(id as _), IMAGE_ICON, s, s, LR_SHARED).ok()
                })
                .map(|h| HICON(h.0))
                .unwrap_or_else(|| LoadIconW(None, IDI_APPLICATION).unwrap_or_default());
        }
        let tip: Vec<u16> = concat!("Hytte ", env!("CARGO_PKG_VERSION"), "\0")
            .encode_utf16()
            .collect();
        let n = tip.len().min(127);
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        nid
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

    pub fn popup(&self, paused: bool, autostart: bool, terminal: bool) {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            let menu = CreatePopupMenu().unwrap_or_default();
            if menu.0.is_null() {
                return;
            }
            let chk = |on: bool| {
                if on {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                }
            };
            let _ = AppendMenuW(menu, chk(paused), 10, w!("Pause"));
            let _ = AppendMenuW(menu, chk(autostart), 13, w!("Launch at startup"));
            let _ = AppendMenuW(menu, chk(terminal), 14, w!("Set up terminal integration"));
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
    pub fn refresh(&self) {}
    pub fn remove(&mut self) {}
    pub fn popup(&self, _paused: bool, _autostart: bool, _terminal: bool) {}
}
