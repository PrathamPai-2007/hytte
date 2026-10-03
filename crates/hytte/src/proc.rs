//! Process / window helpers shared by fullscreen detection, task liveness,
//! click-to-focus-terminal and the port watcher.

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// File name of the executable behind a pid (e.g. `node.exe`).
pub fn exe_name(pid: u32) -> Option<String> {
    hytte_proto::ports::exe_name(pid)
}

pub fn foreground_exe() -> Option<String> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        exe_name(pid)
    }
}

/// False once the process has exited (or can't be opened at all).
pub fn is_alive(pid: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let ok = windows::Win32::System::Threading::GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        ok && code == 259 // STILL_ACTIVE
    }
}

fn parent_of(pid: u32) -> Option<u32> {
    hytte_proto::sys::parent_pid(pid)
}

struct Find {
    pid: u32,
    /// Match by executable stem (lower-case, no `.exe`) instead of pid.
    stem: Option<String>,
    hwnd: HWND,
}

unsafe extern "system" fn enum_cb(h: HWND, lp: LPARAM) -> BOOL {
    let f = unsafe { &mut *(lp.0 as *mut Find) };
    // Visible, un-owned, captioned top-level windows only (skips tool/hidden helpers).
    if !unsafe { IsWindowVisible(h) }.as_bool()
        || !unsafe { GetWindow(h, GW_OWNER) }.map(|o| o.0.is_null()).unwrap_or(true)
        || unsafe { GetWindowTextW(h, &mut [0u16; 4]) } == 0
    {
        return BOOL(1);
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    let hit = match &f.stem {
        Some(s) => exe_name(pid).is_some_and(|e| e.to_ascii_lowercase().trim_end_matches(".exe") == s),
        None => pid == f.pid,
    };
    if hit {
        f.hwnd = h;
        return BOOL(0);
    }
    BOOL(1)
}

fn find_window(mut f: Find) -> Option<HWND> {
    unsafe {
        let _ = EnumWindows(Some(enum_cb), LPARAM(&mut f as *mut _ as isize));
    }
    (!f.hwnd.0.is_null()).then_some(f.hwnd)
}

fn top_window_of(pid: u32) -> Option<HWND> {
    find_window(Find { pid, stem: None, hwnd: HWND::default() })
}

/// The window hosting the terminal/agent that owns `pid`: walk up the process
/// tree until an ancestor owns a visible, captioned top-level window.
pub fn terminal_window_for(pid: u32) -> Option<HWND> {
    let mut cur = pid;
    for _ in 0..8 {
        if let Some(h) = top_window_of(cur) {
            return Some(h);
        }
        cur = parent_of(cur).filter(|p| *p != 0 && *p != cur)?;
    }
    None
}

/// Bring a window to the front. Only call from a user click on our own window,
/// which is what makes Windows allow it.
pub fn focus_window(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
        let _ = SetForegroundWindow(h);
    }
}

/// Executable stem to look for from a media session's AppUserModelId:
/// `Spotify.exe` -> `spotify`, `SpotifyAB.SpotifyMusic_x!Spotify` -> `spotify`, `Chrome` -> `chrome`.
pub fn app_stem(aumid: &str) -> String {
    let last = aumid.rsplit('!').next().unwrap_or(aumid);
    let l = last.to_ascii_lowercase();
    l.strip_suffix(".exe").unwrap_or(&l).to_string()
}

/// Bring the app behind a media session to the front: its existing window if we
/// can find one, otherwise (packaged apps) activate it through the shell.
pub fn focus_app(aumid: &str) {
    if aumid.is_empty() {
        return;
    }
    let stem = app_stem(aumid);
    if let Some(h) = find_window(Find { pid: 0, stem: Some(stem), hwnd: HWND::default() }) {
        focus_window(h);
    } else if aumid.contains('!') {
        let _ = std::process::Command::new("explorer").arg(format!("shell:AppsFolder\\{aumid}")).spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_stems() {
        assert_eq!(super::app_stem("Spotify.exe"), "spotify");
        assert_eq!(super::app_stem("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "spotify");
        assert_eq!(super::app_stem("Chrome"), "chrome");
    }

    use super::*;

    #[test]
    fn self_is_alive_and_named() {
        let me = std::process::id();
        assert!(is_alive(me));
        assert!(exe_name(me).is_some());
        assert!(parent_of(me).is_some());
        assert!(!is_alive(0xFFFF_FFF0));
    }
}
