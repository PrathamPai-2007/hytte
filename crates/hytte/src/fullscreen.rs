//! Fullscreen / game suppression.
//! Primary signal SHQueryUserNotificationState, re-queried when the window
//! layer sees a foreground / foreground-location event. Fallback: foreground
//! rect covers the monitor and is not the shell. Per-process allow/deny lists
//! override both.
//!
//! Two strengths: `Hide` for a window that merely covers the monitor (a maximised
//! app with an auto-hiding taskbar), and `Game` for a real fullscreen app, where
//! Hytte steps aside completely (see `window.rs` `set_gaming`).

use crate::config::General;
use std::sync::atomic::AtomicBool;

/// True while a game (or other real fullscreen app) is in front: workers skip their polls.
pub static GAMING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suppress {
    Show,
    Hide,
    Game,
}

/// What the OS says about the foreground window.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fg {
    /// SHQueryUserNotificationState reports D3D fullscreen, busy or presentation mode.
    pub busy: bool,
    /// The window covers its whole monitor.
    pub covers: bool,
    /// The window has a title bar (maximised apps do; borderless fullscreen games don't).
    pub caption: bool,
    /// The desktop or taskbar is in front (or nothing is): never fullscreen.
    pub shell: bool,
}

/// Pure decision, separated from the OS queries so it can be tested.
pub fn decide(cfg: &General, fg_exe: Option<&str>, fg: Fg) -> Suppress {
    let listed = |l: &[String]| fg_exe.is_some_and(|e| l.iter().any(|x| x.eq_ignore_ascii_case(e)));
    if listed(&cfg.deny_list) {
        return Suppress::Game;
    }
    if !cfg.suppress_fullscreen || listed(&cfg.allow_list) || fg.shell {
        return Suppress::Show;
    }
    if fg.busy || (fg.covers && !fg.caption) {
        Suppress::Game
    } else if fg.covers {
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
        let mut fg = win::foreground(monitor);
        fg.busy = win::busy();
        let s = decide(cfg, exe.as_deref(), fg);
        crate::logging::note_fullscreen(s as u8, &format!("{s:?} foreground={exe:?} {fg:?}"));
        s
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

    /// `(covers, shell)` for the foreground window: whether it covers the whole monitor, and
    /// whether it belongs to the shell (desktop, taskbar) or doesn't exist.
    pub fn foreground(mon: (i32, i32, i32, i32)) -> super::Fg {
        let shell = super::Fg {
            shell: true,
            ..Default::default()
        };
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return shell;
            }
            let mut cls = [0u16; 64];
            let n = GetClassNameW(fg, &mut cls) as usize;
            let cls = String::from_utf16_lossy(&cls[..n]);
            if matches!(
                cls.as_str(),
                "Progman"
                    | "WorkerW"
                    | "SHELLDLL_DefView"
                    | "Shell_TrayWnd"
                    | "Shell_SecondaryTrayWnd"
                    | "HyttePill"
            ) {
                return shell;
            }
            let mut r = RECT::default();
            if GetWindowRect(fg, &mut r).is_err() {
                return super::Fg::default();
            }
            let (mx, my, mw, mh) = mon;
            let covers =
                r.left <= mx && r.top <= my && r.right - r.left >= mw && r.bottom - r.top >= mh;
            let caption = GetWindowLongW(fg, GWL_STYLE) as u32 & WS_CAPTION.0 == WS_CAPTION.0;
            super::Fg {
                covers,
                caption,
                ..Default::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(busy: bool, covers: bool, caption: bool, shell: bool) -> Fg {
        Fg {
            busy,
            covers,
            caption,
            shell,
        }
    }

    #[test]
    fn lists_override() {
        let mut g = General::default();
        let busy = fg(true, false, false, false);
        assert_eq!(decide(&g, Some("game.exe"), busy), Suppress::Game);
        g.allow_list = vec!["GAME.exe".into()];
        assert_eq!(decide(&g, Some("game.exe"), busy), Suppress::Show);
        g.deny_list = vec!["game.exe".into()];
        assert_eq!(decide(&g, Some("game.exe"), Fg::default()), Suppress::Game);
        g.deny_list.clear();
        g.suppress_fullscreen = false;
        g.allow_list.clear();
        assert_eq!(
            decide(&g, None, fg(true, true, false, false)),
            Suppress::Show
        );
    }

    #[test]
    fn desktop_is_never_fullscreen() {
        let mut g = General::default();
        let desk = fg(true, true, false, true);
        assert_eq!(decide(&g, Some("explorer.exe"), desk), Suppress::Show);
        g.deny_list = vec!["explorer.exe".into()];
        assert_eq!(decide(&g, Some("explorer.exe"), desk), Suppress::Game);
    }

    #[test]
    fn maximised_hides_but_fullscreen_is_a_game() {
        let g = General::default();
        // Maximised with an auto-hiding taskbar: covers, keeps its title bar.
        assert_eq!(
            decide(&g, None, fg(false, true, true, false)),
            Suppress::Hide
        );
        // Borderless fullscreen, even when the notification state misses it.
        assert_eq!(
            decide(&g, None, fg(false, true, false, false)),
            Suppress::Game
        );
        // Exclusive fullscreen / presentation reported by the shell.
        assert_eq!(
            decide(&g, None, fg(true, false, true, false)),
            Suppress::Game
        );
        assert_eq!(
            decide(&g, None, fg(false, false, true, false)),
            Suppress::Show
        );
    }
}
