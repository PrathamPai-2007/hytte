//! The big settings window: a section list and every switch, slider and step on one
//! scrolling page.
//!
//! It is the pill's popout. It shares the pill's UI thread and message loop and draws with the
//! same Direct2D renderer (`Renderer::draw_settings_window`), redrawing on input and, while
//! anything eases (hover fades, the scroll, the first fade-in), on a 60 Hz timer that stops
//! as soon as everything has settled. A change is written to `config.toml` and the pill picks
//! it up through its config watcher, exactly as if the file had been edited by hand.

use crate::config::{self, Config, Val};
use crate::render::{Action, Hit, Renderer, SettingsView};
use std::cell::RefCell;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const T_ANIM: usize = 1;
/// How long a note stays over the page.
const TOAST_SECS: f32 = 10.0;

/// What "Custom" does for a setting that only `config.toml` can hold: the value to write if
/// the key is missing, the note written above it in the file, and the one shown over the window.
fn custom(id: &str) -> Option<(Val, String, String)> {
    if id == "general.quiet_hours" {
        return Some((
            Val::S(""),
            "Quiet hours: no sounds (agent alerts, the timer alarm) between these times.
Write them as \"HH:MM-HH:MM\" on a 24-hour clock; the window may wrap past midnight,
for example \"22:30-06:00\". Leave it empty (\"\") for no quiet hours.".into(),
            "In config.toml, under [general], set quiet_hours to your times as \"HH:MM-HH:MM\" (24-hour clock, may wrap past midnight), for example \"22:30-06:00\". Save the file and it applies.".into(),
        ));
    }
    let what = crate::render::help(id);
    if what.is_empty() {
        return None;
    }
    // Where the key sits now: its default is what is written if the file has no line for it.
    let default = crate::render::settings_table(&Config::default())
        .into_iter()
        .flat_map(|s| s.rows)
        .find(|(r, _)| r.key() == id)
        .and_then(|(r, _)| match r.set_action() {
            Some(Action::Set(_, v)) => Some(v),
            _ => None,
        })?;
    let (section, name) = id.split_once('.')?;
    Some((
        default,
        format!(
            "{what}
Type your own value and save the file; it applies at once."
        ),
        format!(
            "In config.toml, under [{section}], set {name} to your own value, then save. {what}"
        ),
    ))
}

struct State {
    rend: Renderer,
    cfg: Config,
    hits: Vec<Hit>,
    /// Pointer in client logical px.
    mouse: Option<(f32, f32)>,
    pressed: bool,
    /// The scroll as drawn, and where it is heading (the wheel and the section list set it).
    scroll: f32,
    target: f32,
    content: f32,
    tops: Vec<f32>,
    /// Dragging the opacity slider: its hit box's left and right edges.
    scrub: Option<(f32, f32)>,
    tracking: bool,
    light: bool,
    accent: [f32; 3],
    opened: Instant,
    last_frame: Instant,
    last_scroll: Instant,
    /// A note over the page (what "Custom" asked for) and when it appeared.
    toast: Option<(String, Instant)>,
    /// What was typed to filter the rows.
    query: String,
}

thread_local! {
    static S: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State, HWND) -> R, hwnd: HWND) -> Option<R> {
    S.with(|c| {
        c.try_borrow_mut()
            .ok()
            .and_then(|mut g| g.as_mut().map(|s| f(s, hwnd)))
    })
}

/// Match the title bar to the theme.
fn title_bar(hwnd: HWND, light: bool) {
    let dark: i32 = (!light) as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

/// Open the window, or bring the one that is already open to the front.
pub fn open(cfg: &Config) {
    unsafe {
        let cls = w!("HytteSettings");
        let existing = S.with(|c| c.try_borrow().map(|g| g.is_some()).unwrap_or(true));
        if existing {
            if let Ok(h) = FindWindowW(cls, None) {
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
            }
            return;
        }
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst.into()), windows::core::PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: cls,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let s = GetDpiForSystem() as f32 / 96.0;
        // No redirection bitmap: DirectComposition supplies the pixels, as for the pill.
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            cls,
            w!("Hytte settings"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (860.0 * s) as i32,
            (700.0 * s) as i32,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            crate::logging::line("settings window: CreateWindowExW failed");
            return;
        };
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let rend = match Renderer::new_for_window(
            hwnd,
            (r.right - r.left).max(1) as u32,
            (r.bottom - r.top).max(1) as u32,
        ) {
            Ok(r) => r,
            Err(e) => {
                crate::logging::line(&format!("settings window: renderer failed: {e}"));
                let _ = DestroyWindow(hwnd);
                return;
            }
        };
        let (light, accent) = crate::window::look(&cfg.general.theme);
        title_bar(hwnd, light);
        let now = Instant::now();
        S.with(|c| {
            *c.borrow_mut() = Some(State {
                rend,
                cfg: cfg.clone(),
                hits: vec![],
                mouse: None,
                pressed: false,
                scroll: 0.0,
                target: 0.0,
                content: 0.0,
                tops: vec![],
                scrub: None,
                tracking: false,
                light,
                accent,
                opened: now,
                last_frame: now,
                last_scroll: now - Duration::from_secs(10),
                toast: None,
                query: String::new(),
            })
        });
        with(redraw, hwnd);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Close the window if it is open. Quit calls this so the window's GPU objects are released on
/// the UI thread, not at process exit (see `window.rs`, where `Ui` is dropped for the same reason).
pub fn close() {
    if let Ok(h) = unsafe { FindWindowW(w!("HytteSettings"), None) } {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

/// The settings were changed elsewhere (the pill's pane, the file): show them.
pub fn refresh(cfg: &Config) {
    if let Ok(h) = unsafe { FindWindowW(w!("HytteSettings"), None) } {
        with(
            |s, hwnd| {
                // Not while the slider is being dragged: the drag owns the value.
                if s.scrub.is_none() {
                    s.cfg = cfg.clone();
                    (s.light, s.accent) = crate::window::look(&cfg.general.theme);
                    title_bar(hwnd, s.light);
                    redraw(s, hwnd);
                }
            },
            h,
        );
    }
}

/// Open `config.toml` in the user's editor (creating it first when it isn't there yet).
fn open_config(cfg: &Config) {
    let path = config::config_path();
    if !path.exists() {
        let _ = config::save(cfg);
    }
    let _ = std::process::Command::new("explorer").arg(&path).spawn();
}

fn scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

/// Draw a frame, then keep the 60 Hz timer going only while something is still moving.
fn redraw(s: &mut State, hwnd: HWND) {
    let now = Instant::now();
    let dt = now.duration_since(s.last_frame).as_secs_f32().min(0.05);
    s.last_frame = now;
    let sc = scale(hwnd);
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    let (cw, ch) = ((r.right - r.left).max(1), (r.bottom - r.top).max(1));
    s.rend.resize_window(cw, ch, sc);
    let max = (s.content - ch as f32 / sc).max(0.0);
    s.target = s.target.clamp(0.0, max);
    s.scroll += (s.target - s.scroll) * (1.0 - (-dt * 14.0).exp());
    if (s.target - s.scroll).abs() < 0.4 {
        s.scroll = s.target;
    }
    let since_scroll = now.duration_since(s.last_scroll);
    let toast_age = s
        .toast
        .as_ref()
        .map(|(_, at)| now.duration_since(*at).as_secs_f32());
    if toast_age.is_some_and(|a| a > TOAST_SECS) {
        s.toast = None;
    }
    let toast = s.toast.as_ref().zip(toast_age).map(|((t, _), a)| {
        (
            t.as_str(),
            (a / 0.2).min(1.0).min(((TOAST_SECS - a) / 0.5).max(0.0)),
        )
    });
    let view = SettingsView {
        mouse: s.mouse,
        pressed: s.pressed,
        light: s.light,
        accent: s.accent,
        scroll: s.scroll,
        scrolling: since_scroll < Duration::from_millis(900)
            || s.mouse.is_some_and(|(x, _)| x > cw as f32 / sc - 16.0),
        dt,
        open: (now.duration_since(s.opened).as_secs_f32() / 0.5).min(1.0),
        toast,
        query: &s.query,
    };
    let out = s.rend.draw_settings_window(&s.cfg, &view, &mut s.hits);
    s.content = out.content;
    s.tops = out.tops;
    if s.rend.lost() {
        // Device lost: a fresh renderer on the same window.
        if let Ok(n) = Renderer::new_for_window(hwnd, cw as u32, ch as u32) {
            s.rend = n;
        }
    }
    let moving = out.busy
        || s.scroll != s.target
        || view.open < 1.0
        || s.toast.is_some()
        || since_scroll < Duration::from_millis(1500);
    unsafe {
        if moving {
            SetTimer(Some(hwnd), T_ANIM, 16, None);
        } else {
            let _ = KillTimer(Some(hwnd), T_ANIM);
        }
    }
}

fn hit_at(s: &State) -> Option<&Hit> {
    let (mx, my) = s.mouse?;
    s.hits
        .iter()
        .rfind(|h| mx >= h.rect.0 && mx <= h.rect.2 && my >= h.rect.1 && my <= h.rect.3)
}

fn set_mouse(s: &mut State, hwnd: HWND, lparam: LPARAM) {
    let sc = scale(hwnd);
    let x = (lparam.0 & 0xFFFF) as i16 as f32 / sc;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32 / sc;
    s.mouse = Some((x, y));
}

/// The opacity slider follows the pointer; written to the file on release.
fn scrub(s: &mut State) {
    let (Some((l, r)), Some((mx, _))) = (s.scrub, s.mouse) else {
        return;
    };
    let f = ((mx - l - 6.0) / (r - l - 12.0)).clamp(0.0, 1.0);
    s.cfg.general.pill_opacity = 0.4 + 0.6 * ((f * 100.0).round() / 100.0);
}

fn set(s: &mut State, key: &'static str, v: Val) {
    match config::set(key, v) {
        Ok(Some(cfg)) => {
            (s.light, s.accent) = crate::window::look(&cfg.general.theme);
            s.cfg = cfg;
        }
        Ok(None) => crate::logging::line("settings window: config.toml has a mistake in it"),
        Err(e) => crate::logging::line(&format!("settings window: {e}")),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            with(redraw, hwnd);
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            return LRESULT(0);
        }
        WM_SIZE => {
            with(redraw, hwnd);
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == T_ANIM => {
            with(redraw, hwnd);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            unsafe {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            with(
                |s, hwnd| {
                    set_mouse(s, hwnd, lparam);
                    if !s.tracking {
                        let mut t = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        unsafe {
                            let _ = TrackMouseEvent(&mut t);
                        }
                        s.tracking = true;
                    }
                    scrub(s);
                    redraw(s, hwnd);
                },
                hwnd,
            );
            return LRESULT(0);
        }
        WM_MOUSELEAVE => {
            with(
                |s, hwnd| {
                    s.tracking = false;
                    s.mouse = None;
                    redraw(s, hwnd);
                },
                hwnd,
            );
            return LRESULT(0);
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as f32;
            with(
                |s, hwnd| {
                    s.target -= delta / 120.0 * 84.0;
                    s.last_scroll = Instant::now();
                    redraw(s, hwnd);
                },
                hwnd,
            );
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            with(
                |s, hwnd| {
                    set_mouse(s, hwnd, lparam);
                    s.pressed = true;
                    if let Some(h) = hit_at(s).filter(|h| h.action == Action::Opacity) {
                        s.scrub = Some((h.rect.0, h.rect.2));
                        scrub(s);
                        unsafe {
                            SetCapture(hwnd);
                        }
                    }
                    redraw(s, hwnd);
                },
                hwnd,
            );
            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            with(|s, _| s.scrub = None, hwnd);
        }
        WM_LBUTTONUP => {
            with(
                |s, hwnd| {
                    set_mouse(s, hwnd, lparam);
                    s.pressed = false;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if s.scrub.take().is_some() {
                        let v = s.cfg.general.pill_opacity as f64;
                        set(s, "general.pill_opacity", Val::F(v));
                    } else if let Some(a) = hit_at(s).map(|h| h.action.clone()) {
                        match a {
                            Action::Set(k, v) => set(s, k, v),
                            Action::Jump(i) => {
                                if let Some(t) = s.tops.get(i) {
                                    s.target = (*t - 30.0).max(0.0);
                                    s.last_scroll = Instant::now();
                                }
                            }
                            Action::EditConfig(id) => {
                                if let Some((default, note, toast)) = custom(id) {
                                    let _ = config::ensure_noted(id, default, &note);
                                    open_config(&s.cfg);
                                    s.toast = Some((toast, Instant::now()));
                                }
                            }
                            Action::ClearSearch => {
                                s.query.clear();
                                s.target = 0.0;
                            }
                            Action::OpenConfig => {
                                open_config(&s.cfg);
                            }
                            _ => {}
                        }
                    }
                    redraw(s, hwnd);
                },
                hwnd,
            );
            return LRESULT(0);
        }
        // Typing filters the settings; Esc clears the search first, then closes.
        WM_CHAR => {
            let c = char::from_u32(wparam.0 as u32).unwrap_or('\0');
            with(
                |s, hwnd| {
                    if c == '\u{8}' {
                        s.query.pop();
                    } else if !c.is_control() && s.query.chars().count() < 40 {
                        s.query.push(c);
                    } else {
                        return;
                    }
                    s.target = 0.0;
                    redraw(s, hwnd);
                },
                hwnd,
            );
            return LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            if with(
                |s, hwnd| {
                    let had = !s.query.is_empty();
                    s.query.clear();
                    s.target = 0.0;
                    redraw(s, hwnd);
                    had
                },
                hwnd,
            )
            .unwrap_or(false)
            {
                return LRESULT(0);
            }
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return LRESULT(0);
        }
        // The Windows theme or accent changed: "auto" follows it live.
        WM_SETTINGCHANGE => {
            with(
                |s, hwnd| {
                    (s.light, s.accent) = crate::window::look(&s.cfg.general.theme);
                    title_bar(hwnd, s.light);
                    redraw(s, hwnd);
                },
                hwnd,
            );
        }
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let hand = with(|s, _| hit_at(s).is_some(), hwnd).unwrap_or(false);
            unsafe {
                if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(c));
                }
            }
            return LRESULT(1);
        }
        WM_DESTROY => {
            unsafe {
                let _ = KillTimer(Some(hwnd), T_ANIM);
            }
            // Release the GPU objects here, on the UI thread, not at process exit.
            S.with(|c| drop(c.borrow_mut().take()));
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
