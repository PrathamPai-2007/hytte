use super::*;
use crate::render::{Action, Crop, Frame, Hit, Renderer};
use crate::ui_state::{Anim, Bubble, Chip, ChipAction, Model, Panel, Scene};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmFlush, DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Power::RegisterPowerSettingNotification;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
};
use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::{
    DragQueryFileW, IVirtualDesktopManager, ShellExecuteW, VirtualDesktopManager,
};
use windows::Win32::UI::WindowsAndMessaging::*;

mod actions;
mod controls;
mod dnd;
mod events;
mod forecast;
mod frame;
mod input;
mod mediactl;
mod scripts;
mod shots;
mod typing;
mod view;
mod wndproc;

use dnd::*;
use wndproc::*;

const WM_TICK: u32 = 0x8002;
const WM_UI: u32 = 0x8003;
const WM_FG: u32 = 0x8004;
const WM_DRAG_ARM: u32 = 0x8005;
const WM_DRAG_END: u32 = 0x8006;
const HOTKEY_ID: i32 = 1;
const MONITOR_HOTKEY_ID: i32 = 2;
const NOTE_HOTKEY_ID: i32 = 3;

const T_DWELL: usize = 1;
const T_COLLAPSE: usize = 2;
const T_EXPIRE: usize = 3;
const T_FS: usize = 4;
const T_TIMER: usize = 5;
const T_TRIM: usize = 6;
const T_GPU: usize = 7;
const T_SHOT: usize = 8;
const T_CARET: usize = 9;
const T_PLACE: usize = 10;
const T_WIDE: usize = 11;
/// A script pane on screen is due to run its command again.
const T_SCRIPT: usize = 12;
/// Idle this long (no frames) before the working set is trimmed.
const TRIM_MS: u32 = 30_000;

/// Cross-thread state: queue + ticker control only.
struct Shared {
    pending: Mutex<Vec<UiEvent>>,
    animating: AtomicBool,
    fast: AtomicBool,
    tick_pending: AtomicBool,
    hwnd: AtomicIsize,
    ticker: OnceLock<std::thread::Thread>,
    drop_tx: Sender<DropJob>,
}

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();
// Latest vblank from DWM (QPC ticks), its period in ticks, and the refresh period in seconds (f64 bits).
static VBLANK: AtomicU64 = AtomicU64::new(0);
static VPERIOD: AtomicU64 = AtomicU64::new(0);
static REFRESH_S: AtomicU64 = AtomicU64::new(0);

/// Sample DWM's vblank clock. Called by the ticker right after `DwmFlush`.
fn sample_vblank() {
    let mut ti = DWM_TIMING_INFO {
        cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
        ..Default::default()
    };
    // SAFETY: ti is a valid, correctly sized out-struct. A null HWND asks for the desktop compositor.
    if unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut ti) }.is_ok()
        && ti.qpcRefreshPeriod > 0
        && ti.rateRefresh.uiNumerator > 0
    {
        let hz = ti.rateRefresh.uiNumerator as f64 / ti.rateRefresh.uiDenominator.max(1) as f64;
        REFRESH_S.store((1.0 / hz).to_bits(), Ordering::Relaxed);
        VPERIOD.store(ti.qpcRefreshPeriod, Ordering::Relaxed);
        VBLANK.store(ti.qpcVBlank, Ordering::Relaxed);
    } else {
        VBLANK.store(0, Ordering::Relaxed);
    }
}

/// Seconds between two vblanks, from DWM's own clock (None if unavailable or implausible).
fn vblank_dt(prev: u64, now: u64) -> Option<f64> {
    let (period, refresh) = (
        VPERIOD.load(Ordering::Relaxed),
        f64::from_bits(REFRESH_S.load(Ordering::Relaxed)),
    );
    if prev == 0 || now <= prev || period == 0 || refresh <= 0.0 {
        return None;
    }
    let d = (now - prev) as f64 / period as f64 * refresh;
    (d < 0.1).then_some(d)
}
// Low-level mouse hook state (screen px zone around the top-centre).
static LBTN: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static ZONE_L: AtomicI32 = AtomicI32::new(0);
static ZONE_R: AtomicI32 = AtomicI32::new(0);
static ZONE_B: AtomicI32 = AtomicI32::new(0);
static HWND_ADDR: AtomicIsize = AtomicIsize::new(0);
/// False while a window that failed to get a renderer is torn down at startup, so its
/// WM_DESTROY doesn't end the message loop before it exists.
/// A minutes digit typed within this of the last one extends the number.
const TYPE_GAP: Duration = Duration::from_millis(1500);
static LIVE: AtomicBool = AtomicBool::new(false);
/// Registered `TaskbarCreated` message: Explorer restarted and our tray icon is gone.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn hwnd_of(addr: isize) -> HWND {
    HWND(addr as *mut _)
}

fn post(msg: u32) {
    post_w(msg, 0);
}

pub fn quit() {
    post_w(WM_COMMAND, 12);
}

fn post_w(msg: u32, wparam: usize) {
    let a = HWND_ADDR.load(Ordering::Relaxed);
    if a != 0 {
        unsafe {
            let _ = PostMessageW(Some(hwnd_of(a)), msg, WPARAM(wparam), LPARAM(0));
        }
    }
}

static MOUSE_HOOK: AtomicIsize = AtomicIsize::new(0);
static LOC_HOOK: AtomicIsize = AtomicIsize::new(0);

/// The system-wide hooks every mouse move passes through: the low-level mouse hook (arms
/// the drop zone during drags) and foreground LOCATIONCHANGE (fullscreen detection when a
/// window resizes). Off while a game is in front.
// ponytail: permanent LL hook with an O(1) callback outside games; install only on drag if it ever shows in profiles
fn input_hooks(on: bool) {
    use windows::Win32::UI::Accessibility::UnhookWinEvent;
    unsafe {
        if on {
            if MOUSE_HOOK.load(Ordering::Relaxed) == 0 {
                let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .map(|m| HINSTANCE(m.0))
                    .ok();
                if let Ok(h) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_ll), inst, 0) {
                    MOUSE_HOOK.store(h.0 as isize, Ordering::Relaxed);
                }
            }
            if LOC_HOOK.load(Ordering::Relaxed) == 0 {
                let h = SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    None,
                    Some(win_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                LOC_HOOK.store(h.0 as isize, Ordering::Relaxed);
            }
        } else {
            let h = MOUSE_HOOK.swap(0, Ordering::Relaxed);
            if h != 0 {
                let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
            }
            let h = LOC_HOOK.swap(0, Ordering::Relaxed);
            if h != 0 {
                let _ = UnhookWinEvent(HWINEVENTHOOK(h as *mut _));
            }
        }
    }
}

fn reassert_topmost(hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// The break intervals from the settings: eyes, stretch, water.
fn break_minutes(c: &Config) -> [u32; 3] {
    [c.breaks.eyes_min, c.breaks.stretch_min, c.breaks.water_min]
}

fn push_event(sh: &Shared, e: UiEvent) {
    sh.pending.lock().unwrap().push(e);
    post(WM_UI);
}

/// Where the pointer is in the strip a compact pill and its bubble occupied.
#[derive(Clone, Copy, PartialEq)]
enum Spot {
    Pill,
    Bubble,
}

/// The two spots of a compact pill and its bubble, kept while the pane they opened is up,
/// so that gliding from one to the other switches the pane, and neither spot closes it
/// (the pane is wider than the compact pill but not always wide enough to cover the
/// bubble, and leaving that spot would close the pane, bring the bubble back under the
/// pointer and open it again).
struct BubbleZone {
    kind: Bubble,
    /// The bubble's left edge from the pill's centre, and its width.
    dx: f32,
    w: f32,
    /// The compact pill's width.
    pill_w: f32,
    /// The panel the pill was on before a bubble chose one (restored on the pill's spot
    /// and when the pane closes).
    base: Option<Panel>,
    at: Option<Spot>,
}

struct Ui {
    cfg: Config,
    model: Model,
    anim: Anim,
    rend: Renderer,
    shared: Arc<Shared>,
    scale: f32,
    mon: (i32, i32, i32, i32),
    /// The `HMONITOR` the pill hangs from (fullscreen checks compare against it).
    hmon: isize,
    hover: bool,
    inside: bool,
    /// The bubble the pointer was last on while the pill was closed (see `open_hover`).
    bubble_pref: Option<Bubble>,
    /// The compact pill's and the bubble's spots while the pane they opened is up.
    zone: Option<BubbleZone>,
    /// Cursor in screen px (stable while the window moves/resizes).
    mouse: Option<(i32, i32)>,
    origin: (i32, i32),
    crop_w: f32,
    hits: Vec<Hit>,
    last: Instant,
    perf: crate::perf::Perf,
    prev_vblank: u64,
    fs_hidden: bool,
    /// A game (real fullscreen app) is in front: hidden, no input hooks, background priority.
    gaming: bool,
    paused: bool,
    armed: bool,
    shown: bool,
    tracking: bool,
    over_hit: bool,
    /// Mouse went down on a shelf tile: (item id, screen point). Becomes a drag past 4 px.
    drag: Option<(u64, (i32, i32))>,
    vdm: Option<IVirtualDesktopManager>,
    thumb_req: std::collections::HashSet<u64>,
    tabs: Sender<crate::tabs::Cmd>,
    /// The left button is down over a clickable region.
    pressed: bool,
    /// Resolved `[general] theme`.
    light: bool,
    /// The Windows accent palette: (light-2, dark-1) shades; see `accent_for`.
    accent: Option<([f32; 3], [f32; 3])>,
    /// Keyboard mode (opened by the hotkey): the pill is held open, has focus, and the
    /// window that had focus before is remembered to give it back.
    kb: Option<HWND>,
    /// Which clickable region has keyboard focus.
    kb_focus: Option<usize>,
    /// Accumulated wheel delta (a notch is 120; precision wheels send fractions).
    wheel: i32,
    /// Touchpad scroll gathered for the lyrics page (it moves a line per big stretch).
    lyric_wheel: i32,
    /// Sideways scroll (touchpad swipe, tilt wheel), turned into one page per gesture.
    swipe: crate::ui_state::Swipe,
    /// Agent asks waiting for an answer, oldest first; the front one is on the chip.
    /// Each: id, summary, arrival, and where the answer goes. Dropping the sender sends
    /// that agent back to its own terminal prompt.
    asks: std::collections::VecDeque<(u64, String, Instant, Sender<bool>)>,
    /// When the current ask appeared on the chip: clicks right after are ignored.
    ask_shown: Option<Instant>,
    /// A newer version waiting for a free moment on the chip: (tag, note).
    update_offer: Option<(String, String)>,
    /// Scripts waiting on a live card's button (`hytte card --wait`): card id and where the
    /// label goes. Dropped when the card is gone, which tells the script "no press".
    card_waits: Vec<(String, Sender<String>)>,
    /// The script panes have had their one run at start.
    scripts_primed: bool,
    /// The clipboard sequence number last looked at (the same change is not read twice).
    clip_seq: u32,
    /// Pane packs on disk, and the actions now in force (config plus approved packs).
    packs: Vec<crate::packs::Pack>,
    actions: Vec<crate::config::NamedAction>,
    /// Packs already asked about this session (folder:hash), so a dismissed card stays gone.
    packs_asked: Vec<String>,
    /// The welcome tour still to show, and the card it is on.
    welcome: Option<(crate::welcome::Tour, usize)>,
    /// GPU rebuild backoff in ms; 0 while the renderer is healthy.
    gpu_retry: u32,
    /// The timer alarm is playing.
    ringing: bool,
    /// The text caret is in the visible half of its blink.
    caret_on: bool,
    /// The first half of a typed emoji, waiting for its second.
    high: Option<u16>,
    /// Counts place-field edits; a search result for an older one is ignored.
    place_seq: u64,
    /// Break reminder schedule.
    breaks: crate::breaks::Breaks,
    /// Do Not Disturb is on for a Focus session: the value to put back afterwards.
    dnd: Option<crate::dnd::Raw>,
    /// Turning it on was tried this session (a failure is not retried on every change).
    dnd_tried: bool,
    /// Hash of the last screenshot shelved from the clipboard: the Snipping Tool can
    /// announce one image more than once.
    last_snip: u64,
    /// Idle-pill dimming as drawn (1 = full), easing towards `[general] idle_opacity`.
    dim: f32,
    /// Clipboard sequence number when that screenshot was copied.
    snip_seq: u32,
    /// Screenshots on the shelf and when they arrived (they leave after
    /// `[shelf] screenshot_secs`).
    shots: Vec<(std::path::PathBuf, Instant)>,
    /// The newest screenshot, waiting for a Ctrl+V (`[shelf] delete_after_paste`).
    paste: Option<Paste>,
    /// Keep-awake is held (SetThreadExecutionState).
    awake_on: bool,
    /// Dragging on the volume bar or the media timeline: which, and its left and right
    /// edge (logical px). The bar follows the pointer; Windows' own volume notifications
    /// wait until the drag ends, and the player is told where to seek on release.
    scrub: Option<(Action, f32, f32)>,
}

/// Time since the last keyboard or mouse input anywhere in the session.
fn input_idle() -> Duration {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut li = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if !unsafe { GetLastInputInfo(&mut li) }.as_bool() {
        return Duration::ZERO;
    }
    Duration::from_millis(unsafe { GetTickCount() }.wrapping_sub(li.dwTime) as u64)
}

/// A screenshot that is deleted once it has been pasted: where it is, when to stop
/// waiting for a paste, whether Ctrl+V was already down, and when a paste is due.
struct Paste {
    path: std::path::PathBuf,
    until: Instant,
    key_down: bool,
    due: Option<Instant>,
}

/// What the next look at a screenshot waiting for its paste decides.
#[derive(Debug, PartialEq)]
enum PasteStep {
    /// Keep waiting.
    Wait,
    /// Stop waiting: it can't be pasted any more (something else was copied) or took too long.
    Drop,
    /// It was pasted and the target has had its moment: delete the screenshot.
    Delete,
}

/// One look at a screenshot waiting for a Ctrl+V. `clip_changed`: something else was copied
/// since the screenshot. `down`: Ctrl+V is held now (`None` while a game is running and the
/// keys aren't polled). A paste is a press, not a held key: a Ctrl+V already down when
/// the screenshot arrived doesn't count until it has been let go and pressed again.
fn paste_step(p: &mut Paste, now: Instant, clip_changed: bool, down: Option<bool>) -> PasteStep {
    if now >= p.until {
        return PasteStep::Drop;
    }
    if let Some(due) = p.due {
        // Pasted: give the target a moment to take the image, then delete.
        return if now >= due {
            PasteStep::Delete
        } else {
            PasteStep::Wait
        };
    }
    if clip_changed {
        return PasteStep::Drop;
    }
    if let Some(down) = down {
        if down && !p.key_down {
            p.due = Some(now + PASTE_GRACE);
        }
        p.key_down = down;
    }
    PasteStep::Wait
}

/// How long a screenshot waits to be pasted before it is left alone.
const PASTE_WINDOW: Duration = Duration::from_secs(600);

/// How long after a paste the screenshot is deleted (the target app must read the image first).
const PASTE_GRACE: Duration = Duration::from_secs(2);

#[cfg(test)]
mod paste_tests {
    use super::*;

    fn waiting(now: Instant, key_down: bool) -> Paste {
        Paste {
            path: "shot.png".into(),
            until: now + PASTE_WINDOW,
            key_down,
            due: None,
        }
    }

    #[test]
    fn a_paste_deletes_the_screenshot_after_a_short_wait() {
        let t0 = Instant::now();
        let mut p = waiting(t0, false);
        // Nothing pressed: keep waiting.
        assert_eq!(paste_step(&mut p, t0, false, Some(false)), PasteStep::Wait);
        // Ctrl+V pressed: not deleted yet (the target app is still reading the image)...
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(paste_step(&mut p, t1, false, Some(true)), PasteStep::Wait);
        assert!(p.due.is_some());
        // ...held on, or released, it is deleted after the grace and not before.
        let almost = t1 + PASTE_GRACE - Duration::from_millis(1);
        assert_eq!(
            paste_step(&mut p, almost, false, Some(false)),
            PasteStep::Wait
        );
        assert_eq!(
            paste_step(&mut p, t1 + PASTE_GRACE, false, Some(false)),
            PasteStep::Delete
        );
    }

    #[test]
    fn a_paste_after_something_else_was_copied_does_not_count() {
        let t0 = Instant::now();
        let mut p = waiting(t0, false);
        assert_eq!(paste_step(&mut p, t0, true, Some(true)), PasteStep::Drop);
        assert!(p.due.is_none());
    }

    #[test]
    fn an_already_pressed_ctrl_v_is_not_a_paste_until_it_is_pressed_again() {
        let t0 = Instant::now();
        let mut p = waiting(t0, true); // Ctrl+V was down when the screenshot arrived
        assert_eq!(paste_step(&mut p, t0, false, Some(true)), PasteStep::Wait);
        assert!(p.due.is_none());
        assert_eq!(paste_step(&mut p, t0, false, Some(false)), PasteStep::Wait);
        assert_eq!(paste_step(&mut p, t0, false, Some(true)), PasteStep::Wait);
        assert!(p.due.is_some());
    }

    #[test]
    fn nothing_is_polled_in_a_game_and_a_forgotten_screenshot_is_let_go() {
        let t0 = Instant::now();
        let mut p = waiting(t0, false);
        assert_eq!(paste_step(&mut p, t0, false, None), PasteStep::Wait);
        assert!(p.due.is_none());
        assert_eq!(
            paste_step(&mut p, t0 + PASTE_WINDOW, false, Some(false)),
            PasteStep::Drop
        );
    }
}

/// Ctrl and V are both held right now.
fn ctrl_v_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 && GetAsyncKeyState(0x56) < 0 }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|c| c.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn ignore_list(cfg: &Config) -> Vec<String> {
    cfg.shell
        .ignore
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

pub fn run_windows(
    cfg: Config,
    task_rx: Receiver<TaskUpdate>,
    ui_rx: Receiver<UiEvent>,
    drop_tx: Sender<DropJob>,
) {
    let shared = Arc::new(Shared {
        pending: Mutex::new(vec![]),
        animating: AtomicBool::new(false),
        fast: AtomicBool::new(false),
        tick_pending: AtomicBool::new(false),
        hwnd: AtomicIsize::new(0),
        ticker: OnceLock::new(),
        drop_tx,
    });
    let _ = SHARED.set(shared.clone());

    unsafe {
        let _ = OleInitialize(None);
        let hinst =
            windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
        let hinstance = HINSTANCE(hinst.0);
        let cls = windows::core::w!("HyttePill");
        let wc = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // Icon 1 in hytte.rc: shown for the window in Alt+Tab-like shell surfaces.
            hIcon: LoadIconW(Some(hinstance), windows::core::PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: cls,
            ..Default::default()
        };
        RegisterClassW(&wc);

        // No redirection bitmap: DirectComposition supplies the pixels (`gpu.rs`).
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            cls,
            windows::core::w!("Hytte"),
            WS_POPUP,
            0,
            0,
            16,
            16,
            None,
            None,
            Some(hinstance),
            None,
        ) else {
            crate::logging::line("CreateWindowExW failed; exiting");
            return;
        };
        let rend = match Renderer::new(hwnd) {
            Ok(r) => r,
            Err(e) => {
                crate::logging::line(&format!("renderer init failed: {e}; exiting"));
                let _ = DestroyWindow(hwnd);
                return;
            }
        };
        LIVE.store(true, Ordering::Relaxed);
        HWND_ADDR.store(hwnd.0 as isize, Ordering::Relaxed);
        shared.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
        // WM_CLIPBOARDUPDATE: Snipping Tool screenshots go on the shelf.
        let _ = AddClipboardFormatListener(hwnd);
        // Locking the PC forgets the clipboard history (WM_WTSSESSION_CHANGE).
        let _ = windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(
            hwnd,
            windows::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION,
        );
        // Power changes arrive as WM_POWERBROADCAST and wake the power worker.
        for g in [&GUID_ACDC_POWER_SOURCE, &GUID_BATTERY_PERCENTAGE_REMAINING] {
            let _ =
                RegisterPowerSettingNotification(HANDLE(hwnd.0), g, DEVICE_NOTIFY_WINDOW_HANDLE);
        }

        let mut anim = Anim::new(120.0, 24.0);
        anim.reduce = reduce_motion();
        let mut ui = Ui {
            cfg,
            model: Model::default(),
            anim,
            rend,
            shared: shared.clone(),
            scale: 1.0,
            mon: (0, 0, 1920, 1080),
            hmon: 0,
            hover: false,
            inside: false,
            bubble_pref: None,
            zone: None,
            mouse: None,
            origin: (0, 0),
            crop_w: 0.0,
            hits: vec![],
            last: Instant::now(),
            perf: crate::perf::Perf::new(),
            prev_vblank: 0,
            fs_hidden: false,
            gaming: false,
            paused: false,
            armed: false,
            shown: false,
            tracking: false,
            over_hit: false,
            drag: None,
            vdm: windows::Win32::System::Com::CoCreateInstance(
                &VirtualDesktopManager,
                None,
                windows::Win32::System::Com::CLSCTX_ALL,
            )
            .ok(),
            thumb_req: Default::default(),
            tabs: crate::tabs::spawn(),
            wheel: 0,
            lyric_wheel: 0,
            swipe: Default::default(),
            pressed: false,
            light: false,
            accent: None,
            kb: None,
            kb_focus: None,
            asks: Default::default(),
            ask_shown: None,
            update_offer: None,
            card_waits: vec![],
            scripts_primed: false,
            clip_seq: 0,
            packs: vec![],
            actions: vec![],
            packs_asked: vec![],
            welcome: crate::welcome::pending().map(|t| (t, 0)),
            gpu_retry: 0,
            ringing: false,
            caret_on: true,
            high: None,
            place_seq: 0,
            breaks: Default::default(),
            dnd: None,
            dnd_tried: false,
            last_snip: 0,
            dim: 1.0,
            snip_seq: 0,
            shots: Vec::new(),
            paste: None,
            awake_on: false,
            scrub: None,
        };
        ui.model.ignore = ignore_list(&ui.cfg);
        ui.light = theme_is_light(&ui.cfg.general.theme);
        ui.accent = windows_accent();
        ui.model.adaptive_glow = ui.cfg.general.adaptive_glow;
        ui.model.finish_peek = Duration::from_secs(ui.cfg.general.finish_peek_secs);
        ui.model.glow_gain = ui.cfg.general.glow_strength.clamp(0.0, 3.0);
        ui.model.body_alpha = ui.cfg.general.pill_opacity.clamp(0.4, 1.0);
        ui.model.task_rows = ui.cfg.general.task_rows;
        ui.model.pin_button = ui.cfg.general.pin_button;
        ui.breaks.configure(break_minutes(&ui.cfg), Instant::now());
        ui.model.lyrics_on = ui.cfg.media.lyrics;
        ui.model.lyrics_still = !ui.cfg.media.lyrics_live;
        ui.model.skin = crate::skin::Skin::from_name(&ui.cfg.general.skin);
        ui.model.init_jot(&crate::jot::path());
        ui.model.weather_place = crate::weather::Place::load();
        ui.model.imperial = crate::weather::imperial(&ui.cfg.weather.units);
        ui.model.apply_panes(&ui.cfg.panes.clone());
        ui.refresh_panes();
        ui.model
            .apply_clipboard(ui.cfg.clipboard.history, ui.cfg.clipboard.expire_secs);
        if ui.cfg.general.remember_panel && ui.cfg.general.start_panel == "last" {
            ui.model.selected = std::fs::read_to_string(panel_file())
                .ok()
                .and_then(|s| Panel::from_name(&s));
        }
        ui.model.rounds = ui.cfg.timer.rounds;
        if ui.cfg.shelf.persist {
            ui.model.shelf = crate::shelf::load();
        }
        ui.model.timer_min = 25;
        ui.model.timer = crate::timer::load();
        ui.refresh_monitor();
        ui.layout();
        ui.anim.rect.snap();
        ui.anim.glow.snap();
        ui.frame(hwnd);
        UI.with(|c| *c.borrow_mut() = Some(ui));

        register_drop_target(hwnd, shared.clone());
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(windows::core::w!("TaskbarCreated")),
            Ordering::Relaxed,
        );

        // The foreground hook stays on even in game mode: it is how leaving the game is seen.
        let _ = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        input_hooks(true);

        let mut tray = crate::tray::Tray::new(hwnd);
        tray.add();
        with_ui(|ui| {
            ui.register_hotkey(hwnd);
            set_capture_hidden(hwnd, ui.cfg.general.hide_from_capture);
        });
        post(WM_FG);

        // Bridge: worker threads -> UI thread via queue + PostMessage (one thread for both channels).
        let s2 = shared.clone();
        std::thread::spawn(move || {
            // A closed channel is swapped for one that never fires, so select! cannot spin on it.
            let (mut tasks, mut events) = (task_rx, ui_rx);
            loop {
                crossbeam_channel::select! {
                    recv(tasks) -> u => match u {
                        Ok(u) => push_event(&s2, UiEvent::Task(u)),
                        Err(_) => tasks = crossbeam_channel::never(),
                    },
                    recv(events) -> u => match u {
                        Ok(u) => push_event(&s2, u),
                        Err(_) => events = crossbeam_channel::never(),
                    },
                }
            }
        });

        // Frame clock: parked at idle. vblank-aligned while springs
        // move, ~30 fps for ambient effects (spinner, dots, EQ).
        let s4 = shared.clone();
        std::thread::spawn(move || {
            let _ = s4.ticker.set(std::thread::current());
            loop {
                if !s4.animating.load(Ordering::SeqCst) {
                    std::thread::park();
                    continue;
                }
                if !s4.tick_pending.swap(true, Ordering::SeqCst) {
                    post(WM_TICK);
                }
                if s4.fast.load(Ordering::Relaxed) {
                    let _ = DwmFlush();
                    sample_vblank();
                } else {
                    std::thread::sleep(Duration::from_millis(33));
                }
            }
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        input_hooks(false);
        // Release the D3D/DXGI/DirectComposition objects here, on this thread. Left to the
        // thread-local's destructor they are freed inside ExitProcess under the loader lock,
        // where GPU drivers deadlock: the process then never exits (and holds the single-
        // instance mutex, so neither a restart nor an update can start a new one).
        crate::settings_win::close();
        UI.with(|c| drop(c.borrow_mut().take()));
        tray.remove();
        let _ = RevokeDragDrop(hwnd);
        OleUninitialize();
    }
}

/// True while the process we listed is still the one listening on `port`.
fn port_alive(m: &Model, port: u16) -> bool {
    let Some(p) = m.ports.iter().find(|p| p.port == port) else {
        return false;
    };
    hytte_proto::ports::listeners().contains(&(port, p.pid))
}

/// `[general] theme`: "light", "auto" (follow the Windows "app mode" setting), else dark.
pub fn look(theme: &str) -> (bool, [f32; 3]) {
    let light = theme_is_light(theme);
    (light, accent_for(windows_accent(), light))
}

fn theme_is_light(theme: &str) -> bool {
    match theme {
        "light" => true,
        "auto" => unsafe {
            use windows::Win32::System::Registry::{
                RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
            };
            let mut v = 0u32;
            let mut n = std::mem::size_of::<u32>() as u32;
            RegGetValueW(
                HKEY_CURRENT_USER,
                windows::core::w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
                windows::core::w!("AppsUseLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut v as *mut _ as *mut _),
                Some(&mut n),
            )
            .is_ok()
                && v == 1
        },
        _ => false,
    }
}

/// The Windows accent palette from the registry (`AccentPalette`: eight RGBA shades,
/// lightest first, the accent itself fourth). Read there rather than through WinRT
/// `UISettings`, which would load the XAML colour stack into the pill for one value.
fn windows_accent() -> Option<([f32; 3], [f32; 3])> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};
    let mut b = [0u8; 32];
    let mut n = b.len() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"),
            windows::core::w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(b.as_mut_ptr() as *mut _),
            Some(&mut n),
        )
        .ok()
        .ok()?;
    }
    (n >= 32).then(|| {
        let shade = |i: usize| [b[i * 4], b[i * 4 + 1], b[i * 4 + 2]].map(|c| c as f32 / 255.0);
        // The palette runs light 3, 2, 1, the accent itself, dark 1, 2, 3. The accent skin
        // takes the accent; every read (at start, on ImmersiveColorSet) passes it on.
        crate::skin::set_accent(Some(shade(3)));
        (shade(1), shade(4))
    })
}

/// The shade for this pill: Windows uses light-2 on dark surfaces, dark-1 on light ones.
fn accent_for(a: Option<([f32; 3], [f32; 3])>, light: bool) -> [f32; 3] {
    match a {
        Some((l2, d1)) => {
            if light {
                d1
            } else {
                l2
            }
        }
        None => [0.35, 0.63, 1.0],
    }
}

/// The monitors, left to right (then top to bottom): "monitor 2" means the same one each time.
fn monitors() -> Vec<HMONITOR> {
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC};
    unsafe extern "system" fn each(
        m: HMONITOR,
        _: HDC,
        _: *mut RECT,
        list: LPARAM,
    ) -> windows::core::BOOL {
        // SAFETY: `list` is the Vec passed to EnumDisplayMonitors below.
        unsafe { (*(list.0 as *mut Vec<HMONITOR>)).push(m) };
        true.into()
    }
    let mut list: Vec<HMONITOR> = vec![];
    // SAFETY: the callback runs inside this call, while `list` is alive.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    let origin = |m: &HMONITOR| {
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: mi is a correctly sized out-struct.
        unsafe {
            let _ = GetMonitorInfoW(*m, &mut mi);
        }
        (mi.rcMonitor.left, mi.rcMonitor.top)
    };
    list.sort_by_key(origin);
    list
}

fn reduce_motion() -> bool {
    unsafe {
        let mut on = windows::core::BOOL(1);
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        !on.as_bool()
    }
}

/// Keep the pill out of screenshots, recordings and screen share (`[general] hide_from_capture`).
fn set_capture_hidden(hwnd: HWND, on: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
    };
    let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    let _ = unsafe { SetWindowDisplayAffinity(hwnd, affinity) };
}

fn open_url(url: &str) {
    // Only web links: a chip's text can come from outside (a calendar entry).
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let wide: Vec<u16> = url.encode_utf16().chain([0]).collect();
    unsafe {
        ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}

/// A copy of one clipboard format's data, or None.
unsafe fn clipboard_bytes(hwnd: HWND, format: u32) -> Option<Vec<u8>> {
    OpenClipboard(Some(hwnd)).ok()?;
    let out = GetClipboardData(format).ok().and_then(|h| {
        let h = HGLOBAL(h.0);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
        let _ = GlobalUnlock(h);
        Some(v)
    });
    let _ = CloseClipboard();
    out
}

fn set_clipboard(hwnd: HWND, s: &str) {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        let wide: Vec<u16> = s.encode_utf16().chain([0]).collect();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
                let _ = GlobalUnlock(h);
                if SetClipboardData(13 /* CF_UNICODETEXT */, Some(HANDLE(h.0))).is_err() {
                    let _ = GlobalFree(Some(h));
                }
            }
        }
        let _ = CloseClipboard();
    }
}

unsafe extern "system" fn win_event(
    _h: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    // LOCATIONCHANGE fires for every moving window and caret: only the
    // foreground window's own rect matters for fullscreen detection.
    if event == EVENT_OBJECT_LOCATIONCHANGE
        && (id_object != 0 || hwnd != unsafe { GetForegroundWindow() })
    {
        return;
    }
    // Dragging a window fires this per mouse move: WM_FG only re-arms the debounce.
    post(WM_FG);
}

/// A window is being dragged by its title bar or resized: that left-drag is not a file.
fn moving_window() -> bool {
    let mut gi = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(0, &mut gi).is_ok() && gi.flags.contains(GUI_INMOVESIZE) }
}

unsafe extern "system" fn mouse_ll(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        match wp.0 as u32 {
            WM_LBUTTONDOWN => LBTN.store(true, Ordering::Relaxed),
            WM_LBUTTONUP => {
                LBTN.store(false, Ordering::Relaxed);
                if ARMED.swap(false, Ordering::Relaxed) {
                    post(WM_DRAG_END);
                }
            }
            WM_MOUSEMOVE if LBTN.load(Ordering::Relaxed) && !ARMED.load(Ordering::Relaxed) => {
                let ms = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
                if ms.pt.y < ZONE_B.load(Ordering::Relaxed)
                    && ms.pt.x >= ZONE_L.load(Ordering::Relaxed)
                    && ms.pt.x <= ZONE_R.load(Ordering::Relaxed)
                    && !moving_window()
                {
                    ARMED.store(true, Ordering::Relaxed);
                    post(WM_DRAG_ARM);
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}
