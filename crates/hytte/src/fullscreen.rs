//! Fullscreen / game suppression.
//! Primary signal SHQueryUserNotificationState, re-queried when the window
//! layer sees a foreground / foreground-location event. Fallback: foreground
//! rect covers the monitor and is not the shell. Per-process allow/deny lists
//! override both.

use crate::config::General;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppress {
    Show,
    Hide,
}

/// Pure decision, separated from the OS queries so it can be tested.
pub fn decide(cfg: &General, fg_exe: Option<&str>, busy: bool, covers: bool) -> Suppress {
    let listed = |l: &[String]| fg_exe.is_some_and(|e| l.iter().any(|x| x.eq_ignore_ascii_case(e)));
    if listed(&cfg.deny_list) {
        return Suppress::Hide;
    }
    if !cfg.suppress_fullscreen || listed(&cfg.allow_list) {
        return Suppress::Show;
    }
    if busy || covers {
        Suppress::Hide
    } else {
        Suppress::Show
    }
}

/// Query the OS and decide for the current foreground window.
pub fn evaluate(cfg: &General, monitor: (i32, i32, i32, i32)) -> Suppress {
    #[cfg(windows)]
    {
        let exe = crate::proc::foreground_exe();
        let covers = win::foreground_covers(monitor);
        decide(cfg, exe.as_deref(), win::busy(), covers)
    }
    #[cfg(not(windows))]
    {
        let _ = (cfg, monitor);
        Suppress::Show
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    pub fn busy() -> bool {
        unsafe {
            matches!(
                SHQueryUserNotificationState(),
                Ok(q) if q == QUNS_RUNNING_D3D_FULL_SCREEN || q == QUNS_BUSY || q == QUNS_PRESENTATION_MODE
            )
        }
    }

    /// Foreground window covers the whole monitor and is not the shell.
    pub fn foreground_covers(mon: (i32, i32, i32, i32)) -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return false;
            }
            let mut cls = [0u16; 64];
            let n = GetClassNameW(fg, &mut cls) as usize;
            let cls = String::from_utf16_lossy(&cls[..n]);
            if matches!(
                cls.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "HyttePill"
            ) {
                return false;
            }
            let mut r = RECT::default();
            if GetWindowRect(fg, &mut r).is_err() {
                return false;
            }
            let (mx, my, mw, mh) = mon;
            r.left <= mx && r.top <= my && r.right - r.left >= mw && r.bottom - r.top >= mh
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_override() {
        let mut g = General::default();
        assert_eq!(decide(&g, Some("game.exe"), true, false), Suppress::Hide);
        g.allow_list = vec!["GAME.exe".into()];
        assert_eq!(decide(&g, Some("game.exe"), true, false), Suppress::Show);
        g.deny_list = vec!["game.exe".into()];
        assert_eq!(decide(&g, Some("game.exe"), false, false), Suppress::Hide);
        g.deny_list.clear();
        g.suppress_fullscreen = false;
        g.allow_list.clear();
        assert_eq!(decide(&g, None, true, true), Suppress::Show);
    }
}
