//! Fullscreen / game suppression.
//! Primary signal SHQueryUserNotificationState, re-queried when the window
//! layer sees a foreground / foreground-location event. Fallback: the foreground
//! window covers its own monitor and is not the shell. Per-process allow/deny lists
//! override both.
//!
//! Two strengths: `Hide` (the thin line) for a window that merely covers the monitor (a
//! maximised app with an auto-hiding taskbar, a browser or video player in fullscreen), and
//! `Game` for a real fullscreen app, where Hytte steps aside completely (see `window.rs`
//! `set_gaming`).

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

/// Browsers and media players: borderless fullscreen there is a video, not a game, so it
/// gets the thin line instead of game mode. `deny_list` still forces game mode.
const MEDIA: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
    "vlc.exe",
    "mpv.exe",
    "mpc-hc64.exe",
    "potplayermini64.exe",
    "wmplayer.exe",
    "Microsoft.Media.Player.exe",
];

/// Shell processes: Start, Search, Quick Settings, the lock screen, and explorer's own
/// full-monitor overlays (Task View, Alt-Tab, Snap). None of them is ever fullscreen.
const SHELL: &[&str] = &[
    "explorer.exe",
    "ShellExperienceHost.exe",
    "StartMenuExperienceHost.exe",
    "SearchHost.exe",
    "LockApp.exe",
    "ShellHost.exe",
];

/// What the OS says about the foreground window.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fg {
    /// SHQueryUserNotificationState reports exclusive D3D fullscreen or presentation mode.
    pub busy: bool,
    /// SHQueryUserNotificationState reports some fullscreen app (browser F11 included) or a
    /// fullscreen Store app.
    pub fs: bool,
    /// The window covers its whole monitor.
    pub covers: bool,
    /// The window has a title bar (maximised apps do; borderless fullscreen games don't).
    pub caption: bool,
    /// Resizable or dialog frame: a frameless-but-windowed app (Electron, VS Code).
    pub frame: bool,
    /// Maximised.
    pub zoomed: bool,
    /// The desktop or taskbar is in front (or nothing is): never fullscreen.
    pub shell: bool,
    /// A tool, click-through or cloaked window (Game Bar, GPU / OEM overlays): says nothing
    /// about what is underneath, so the current state is kept.
    pub ignore: bool,
    /// On the monitor the pill hangs from.
    pub same_monitor: bool,
}

/// Pure decision, separated from the OS queries so it can be tested.
/// `None`: keep whatever was decided before.
pub fn decide(cfg: &General, fg_exe: Option<&str>, fg: Fg) -> Option<Suppress> {
    let is = |e: &str, l: &[&str]| l.iter().any(|x| x.eq_ignore_ascii_case(e));
    let listed = |l: &[String]| fg_exe.is_some_and(|e| l.iter().any(|x| x.eq_ignore_ascii_case(e)));
    if listed(&cfg.deny_list) {
        return Some(Suppress::Game);
    }
    if !cfg.suppress_fullscreen || listed(&cfg.allow_list) || fg.shell {
        return Some(Suppress::Show);
    }
    if fg.ignore {
        return None;
    }
    let s = if fg.busy {
        Suppress::Game
    } else if fg.covers && !fg.zoomed && (fg.fs || (!fg.caption && !fg.frame)) {
        if fg_exe.is_some_and(|e| is(e, MEDIA)) {
            Suppress::Hide
        } else {
            Suppress::Game
        }
    } else if fg.covers {
        Suppress::Hide
    } else {
        Suppress::Show
    };
    // A maximised app on another monitor leaves this one alone; a game anywhere still costs frames.
    Some(if s == Suppress::Hide && !fg.same_monitor {
        Suppress::Show
    } else {
        s
    })
}

/// Query the OS and decide for the current foreground window. `pill_mon` is the `HMONITOR`
/// the pill hangs from.
pub fn evaluate(cfg: &General, pill_mon: isize) -> Option<Suppress> {
    #[cfg(windows)]
    {
        let exe = crate::proc::foreground_exe();
        let mut fg = win::foreground(pill_mon);
        // explorer.exe also owns File Explorer windows, which may go fullscreen (F11): those count.
        if !fg.shell
            && exe
                .as_deref()
                .is_some_and(|e| SHELL.iter().any(|x| x.eq_ignore_ascii_case(e)))
        {
            fg.shell = !win::is_file_explorer();
        }
        (fg.busy, fg.fs) = win::busy();
        let s = decide(cfg, exe.as_deref(), fg);
        let code = s.map_or(255, |s| s as u8);
        crate::logging::note_fullscreen(code, &format!("{s:?} foreground={exe:?} {fg:?}"));
        s
    }
    #[cfg(not(windows))]
    {
        let _ = (cfg, pill_mon);
        Some(Suppress::Show)
    }
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_APP, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows::Win32::UI::WindowsAndMessaging::*;

    /// `(exclusive / presentation, any fullscreen app)`.
    pub fn busy() -> (bool, bool) {
        match unsafe { SHQueryUserNotificationState() } {
            Ok(q) => (
                q == QUNS_RUNNING_D3D_FULL_SCREEN || q == QUNS_PRESENTATION_MODE,
                q == QUNS_BUSY || q == QUNS_APP,
            ),
            Err(_) => (false, false),
        }
    }

    fn class(w: HWND) -> String {
        let mut cls = [0u16; 64];
        let n = unsafe { GetClassNameW(w, &mut cls) } as usize;
        String::from_utf16_lossy(&cls[..n])
    }

    fn root_fg() -> Option<HWND> {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                return None;
            }
            // A game's child or owned popup still means the game is in front.
            let root = GetAncestor(fg, GA_ROOT);
            Some(if root.0.is_null() { fg } else { root })
        }
    }

    pub fn is_file_explorer() -> bool {
        root_fg().is_some_and(|w| class(w) == "CabinetWClass")
    }

    /// The foreground window's shape and where it sits.
    pub fn foreground(pill_mon: isize) -> super::Fg {
        let shell = super::Fg {
            shell: true,
            ..Default::default()
        };
        unsafe {
            let Some(fg) = root_fg() else { return shell };
            if matches!(
                class(fg).as_str(),
                "Progman"
                    | "WorkerW"
                    | "SHELLDLL_DefView"
                    | "Shell_TrayWnd"
                    | "Shell_SecondaryTrayWnd"
                    | "HyttePill"
            ) {
                return shell;
            }
            let style = GetWindowLongW(fg, GWL_STYLE) as u32;
            let ex = GetWindowLongW(fg, GWL_EXSTYLE) as u32;
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(
                fg,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            );
            if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0) != 0 || cloaked != 0 {
                return super::Fg {
                    ignore: true,
                    ..Default::default()
                };
            }
            let mut r = RECT::default();
            let hmon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetWindowRect(fg, &mut r).is_err() || !GetMonitorInfoW(hmon, &mut mi).as_bool() {
                return super::Fg::default();
            }
            let m = mi.rcMonitor;
            // A pixel or two short is still fullscreen (DPI rounding, apps that dodge the
            // shell's own fullscreen handling).
            const SLACK: i32 = 2;
            let covers = r.left <= m.left + SLACK
                && r.top <= m.top + SLACK
                && r.right >= m.right - SLACK
                && r.bottom >= m.bottom - SLACK;
            let has = |bits: u32| style & bits == bits;
            super::Fg {
                covers,
                caption: has(WS_CAPTION.0),
                frame: has(WS_THICKFRAME.0) || (has(WS_DLGFRAME.0) && !has(WS_CAPTION.0)),
                zoomed: IsZoomed(fg).as_bool(),
                same_monitor: hmon.0 as isize == pill_mon,
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
            same_monitor: true,
            ..Default::default()
        }
    }

    const SHOW: Option<Suppress> = Some(Suppress::Show);
    const HIDE: Option<Suppress> = Some(Suppress::Hide);
    const GAME: Option<Suppress> = Some(Suppress::Game);

    #[test]
    fn lists_override() {
        let mut g = General::default();
        let busy = fg(true, false, false, false);
        assert_eq!(decide(&g, Some("game.exe"), busy), GAME);
        g.allow_list = vec!["GAME.exe".into()];
        assert_eq!(decide(&g, Some("game.exe"), busy), SHOW);
        g.deny_list = vec!["game.exe".into()];
        assert_eq!(decide(&g, Some("game.exe"), Fg::default()), GAME);
        g.deny_list.clear();
        g.suppress_fullscreen = false;
        g.allow_list.clear();
        assert_eq!(decide(&g, None, fg(true, true, false, false)), SHOW);
    }

    #[test]
    fn desktop_is_never_fullscreen() {
        let mut g = General::default();
        let desk = fg(true, true, false, true);
        assert_eq!(decide(&g, Some("explorer.exe"), desk), SHOW);
        g.deny_list = vec!["explorer.exe".into()];
        assert_eq!(decide(&g, Some("explorer.exe"), desk), GAME);
    }

    #[test]
    fn maximised_hides_but_fullscreen_is_a_game() {
        let g = General::default();
        // Maximised with an auto-hiding taskbar: covers, keeps its title bar.
        assert_eq!(decide(&g, None, fg(false, true, true, false)), HIDE);
        // Borderless fullscreen, even when the notification state misses it.
        assert_eq!(decide(&g, None, fg(false, true, false, false)), GAME);
        // Exclusive fullscreen / presentation reported by the shell.
        assert_eq!(decide(&g, None, fg(true, false, true, false)), GAME);
        assert_eq!(decide(&g, None, fg(false, false, true, false)), SHOW);
        // Frameless but maximised (Electron, VS Code): just maximised.
        let electron = Fg {
            zoomed: true,
            ..fg(false, true, false, false)
        };
        assert_eq!(decide(&g, Some("Code.exe"), electron), HIDE);
    }

    #[test]
    fn video_gets_the_thin_line() {
        let g = General::default();
        let f11 = fg(false, true, false, false);
        assert_eq!(decide(&g, Some("chrome.exe"), f11), HIDE);
        assert_eq!(decide(&g, Some("VLC.EXE"), f11), HIDE);
        // The shell flags browser fullscreen as busy too: still a video.
        let flagged = Fg {
            fs: true,
            ..fg(false, true, true, false)
        };
        assert_eq!(decide(&g, Some("msedge.exe"), flagged), HIDE);
        assert_eq!(decide(&g, Some("game.exe"), flagged), GAME);
    }

    #[test]
    fn overlays_keep_the_state_and_other_monitors_only_hide_for_games() {
        let g = General::default();
        let overlay = Fg {
            ignore: true,
            ..fg(false, true, false, false)
        };
        assert_eq!(decide(&g, Some("GameBar.exe"), overlay), None);
        let elsewhere = |caption| Fg {
            same_monitor: false,
            ..fg(false, true, caption, false)
        };
        assert_eq!(decide(&g, None, elsewhere(true)), SHOW);
        assert_eq!(decide(&g, None, elsewhere(false)), GAME);
    }
}
