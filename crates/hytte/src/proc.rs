//! Process / window helpers shared by fullscreen detection, task liveness,
//! click-to-focus-terminal and the port watcher.

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Mark the calling thread as background work (EcoQoS): Windows may run it on efficiency
/// cores at a lower clock. For watchers whose latency doesn't matter; never the UI or frame clock.
pub fn eco_thread() {
    let state = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
    };
    // SAFETY: `state` outlives the call and the size passed is its size.
    unsafe {
        let _ = SetThreadInformation(
            GetCurrentThread(),
            ThreadPowerThrottling,
            &state as *const _ as *const _,
            std::mem::size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

/// Bytes of RAM Windows could hand out now (free plus cached), 0 if it can't say.
pub fn free_ram() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut s = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `s` is a valid MEMORYSTATUSEX with its length set.
    match unsafe { GlobalMemoryStatusEx(&mut s) } {
        Ok(()) => s.ullAvailPhys,
        Err(_) => 0,
    }
}

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
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
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
        || !unsafe { GetWindow(h, GW_OWNER) }
            .map(|o| o.0.is_null())
            .unwrap_or(true)
        || unsafe { GetWindowTextW(h, &mut [0u16; 4]) } == 0
    {
        return BOOL(1);
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    let hit = match &f.stem {
        Some(s) => {
            exe_name(pid).is_some_and(|e| e.to_ascii_lowercase().trim_end_matches(".exe") == s)
        }
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
    find_window(Find {
        pid,
        stem: None,
        hwnd: HWND::default(),
    })
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
    if let Some(h) = find_window(Find {
        pid: 0,
        stem: Some(stem),
        hwnd: HWND::default(),
    }) {
        focus_window(h);
    } else if aumid.contains('!') {
        let _ = std::process::Command::new("explorer")
            .arg(format!("shell:AppsFolder\\{aumid}"))
            .spawn();
    }
}

/// Whether `pid` owns a visible, captioned top-level window (what the Windows mixer lists).
pub fn has_window(pid: u32) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    // Suspended UWP apps keep a "visible" but cloaked window; that isn't one you can see.
    top_window_of(pid).is_some_and(|h| {
        let mut cloaked = 0u32;
        let ok = unsafe {
            DwmGetWindowAttribute(h, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), 4)
        };
        ok.is_err() || cloaked == 0
    })
}

/// The "file description" of an exe (`Spotify`, `Wallpaper Engine`), if it carries one.
pub fn file_description(path: &str) -> Option<String> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };
    let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let p = PCWSTR(wide.as_ptr());
    unsafe {
        let n = GetFileVersionInfoSizeW(p, None);
        if n == 0 {
            return None;
        }
        let mut data = vec![0u8; n as usize];
        GetFileVersionInfoW(p, None, n, data.as_mut_ptr().cast()).ok()?;
        let mut tr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            data.as_ptr().cast(),
            w!(r"\VarFileInfo\Translation"),
            &mut tr,
            &mut len,
        )
        .as_bool()
            || len < 4
        {
            return None;
        }
        let t = *(tr as *const [u16; 2]);
        let q: Vec<u16> = format!(r"\StringFileInfo\{:04x}{:04x}\FileDescription", t[0], t[1])
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut s: *mut core::ffi::c_void = std::ptr::null_mut();
        if !VerQueryValueW(data.as_ptr().cast(), PCWSTR(q.as_ptr()), &mut s, &mut len).as_bool()
            || len == 0
        {
            return None;
        }
        // Up to the first NUL: some exes (Spotify's) report a length past the end of the string,
        // which would read the next resource entry's text into the name.
        let raw = std::slice::from_raw_parts(s as *const u16, len as usize);
        let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        let d = String::from_utf16_lossy(&raw[..end]).trim().to_owned();
        (!d.is_empty()).then_some(d)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_stems() {
        assert_eq!(super::app_stem("Spotify.exe"), "spotify");
        assert_eq!(
            super::app_stem("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            "spotify"
        );
        assert_eq!(super::app_stem("Chrome"), "chrome");
    }

    use super::*;

    #[test]
    fn file_description_has_no_stray_text() {
        // A real exe with a description; nothing may follow the name's own end.
        let d = file_description(r"C:\Windows\System32\notepad.exe").unwrap();
        assert!(!d.is_empty() && !d.contains('\0'), "{d:?}");
    }

    #[test]
    fn self_is_alive_and_named() {
        let me = std::process::id();
        assert!(is_alive(me));
        assert!(exe_name(me).is_some());
        assert!(parent_of(me).is_some());
        assert!(!is_alive(0xFFFF_FFF0));
    }
}
