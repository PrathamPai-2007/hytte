//! Windowing & interaction engine.
//!
//! One layered, NOACTIVATE, topmost window hanging from the top-centre of the
//! monitor chosen by `[general] monitor`. The pill is animated *inside* a fixed transparent canvas
//! (see `render.rs`), so there is no per-frame window resize and transparent
//! pixels stay click-through. UI state lives in a thread-local `Ui`; other
//! threads only push `UiEvent`s and post a wake-up message.

use crate::config::Config;
use crate::drop::DropJob;
use crate::tasks::TaskUpdate;
use crate::ui_state::UiEvent;
use crossbeam_channel::{Receiver, Sender};

pub const WM_TRAY: u32 = 0x8001;

/// Whether the theme is light, and the accent colour that suits it.
#[cfg(windows)]
pub fn look(theme: &str) -> (bool, [f32; 3]) {
    win::look(theme)
}

/// Ends the daemon the way the tray's Quit does (tray icon removed, message loop left).
/// Safe from any thread; does nothing before the window exists.
pub fn quit() {
    #[cfg(windows)]
    win::quit();
}

pub fn run(
    cfg: Config,
    task_rx: Receiver<TaskUpdate>,
    ui_rx: Receiver<UiEvent>,
    drop_tx: Sender<DropJob>,
    _pipe_handle: std::thread::JoinHandle<()>,
    _tasks_handle: std::thread::JoinHandle<()>,
) {
    #[cfg(not(windows))]
    {
        let _ = (cfg, task_rx, ui_rx, drop_tx);
        eprintln!("hytte: Windows-only daemon");
    }
    #[cfg(windows)]
    win::run_windows(cfg, task_rx, ui_rx, drop_tx);
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::render::{Action, Crop, Frame, Hit, Renderer};
    use crate::ui_state::{Anim, Chip, ChipAction, Model, Panel, Scene};
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

    const WM_TICK: u32 = 0x8002;
    const WM_UI: u32 = 0x8003;
    const WM_FG: u32 = 0x8004;
    const WM_DRAG_ARM: u32 = 0x8005;
    const WM_DRAG_END: u32 = 0x8006;
    const HOTKEY_ID: i32 = 1;

    const T_DWELL: usize = 1;
    const T_COLLAPSE: usize = 2;
    const T_EXPIRE: usize = 3;
    const T_FS: usize = 4;
    const T_TIMER: usize = 5;
    const T_TRIM: usize = 6;
    const T_GPU: usize = 7;
    const T_SHOT: usize = 8;
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

    fn push_event(sh: &Shared, e: UiEvent) {
        sh.pending.lock().unwrap().push(e);
        post(WM_UI);
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
        /// Sideways scroll (touchpad swipe, tilt wheel), turned into one page per gesture.
        swipe: crate::ui_state::Swipe,
        /// Agent asks waiting for an answer, oldest first; the front one is on the chip.
        /// Each: id, summary, arrival, and where the answer goes. Dropping the sender sends
        /// that agent back to its own terminal prompt.
        asks: std::collections::VecDeque<(u64, String, Instant, Sender<bool>)>,
        /// GPU rebuild backoff in ms; 0 while the renderer is healthy.
        gpu_retry: u32,
        /// The timer alarm is playing.
        ringing: bool,
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

    impl Ui {
        fn kick(&mut self) {
            if !self.shared.animating.swap(true, Ordering::SeqCst) {
                self.last = Instant::now();
            }
            if let Some(t) = self.shared.ticker.get() {
                t.unpark();
            }
            post(WM_TICK);
        }

        fn sentinel(&self) -> bool {
            self.fs_hidden && self.cfg.general.fullscreen != "hide"
        }

        fn hidden(&self) -> bool {
            // A drag near the top edge or a fresh drop must stay reachable even over fullscreen.
            self.paused
                || self.gaming
                || (self.fs_hidden
                    && self.cfg.general.fullscreen == "hide"
                    && !self.armed
                    && !self.model.forced())
        }

        fn layout(&mut self) {
            let scene = self.model.scene(self.hover, self.sentinel());
            let size = self.model.size(scene);
            let glow = self.model.glow(scene);
            self.anim.set_scene(scene, size, glow);
            let bubble = self.model.bubble(scene);
            self.anim.set_bubble(bubble);
            self.anim
                .hover
                .set_target(if self.inside && scene != Scene::Sentinel {
                    1.0
                } else {
                    0.0
                });
            self.anim
                .vis
                .set_target(if self.hidden() { 0.0 } else { 1.0 });
        }

        /// Cursor in the coordinates of the last drawn frame (logical px).
        fn mouse_logical(&self) -> Option<(f32, f32)> {
            let (sx, sy) = self.mouse?;
            Some((
                (sx - self.origin.0) as f32 / self.scale,
                (sy - self.origin.1) as f32 / self.scale,
            ))
        }

        fn set_mouse_client(&mut self, lparam: LPARAM) {
            self.mouse = Some((self.origin.0 + lo(lparam), self.origin.1 + hi(lparam)));
        }

        fn pill_contains(&self) -> bool {
            let Some((mx, my)) = self.mouse_logical() else {
                return false;
            };
            let pw = self.anim.rect.w.pos as f32;
            let ph = (self.anim.rect.h.pos as f32).max(6.0);
            let ox = (self.crop_w - pw) / 2.0;
            mx >= ox && mx <= ox + pw && my >= 0.0 && my <= ph
        }

        fn crop(&self) -> Crop {
            self.rend.plan(
                self.anim.rect.w.pos as f32,
                self.anim.rect.h.pos as f32,
                self.armed,
                self.anim.bubble_room(),
                self.glowing(),
            )
        }

        fn glowing(&self) -> bool {
            self.anim.glow.pos.max(self.anim.glow.target) > 0.01
        }

        fn origin_for(&self, crop: Crop) -> (i32, i32) {
            let mon_w = self.mon.2 - self.mon.0;
            (self.mon.0 + (mon_w - crop.w_px) / 2, self.mon.1)
        }

        /// The monitor the pill should hang from, per `[general] monitor`: "active" is the one
        /// holding the foreground window, "cursor" the one under the pointer, anything else
        /// the primary.
        fn wanted_monitor(&self) -> HMONITOR {
            unsafe {
                let primary = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
                match self.cfg.general.monitor.as_str() {
                    "active" => {
                        let fg = GetForegroundWindow();
                        if fg.0.is_null() {
                            primary
                        } else {
                            MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
                        }
                    }
                    "cursor" => {
                        let mut p = POINT::default();
                        if GetCursorPos(&mut p).is_ok() {
                            MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
                        } else {
                            primary
                        }
                    }
                    _ => primary,
                }
            }
        }

        /// Move the pill to another monitor when `[general] monitor` says it should follow.
        fn follow_monitor(&mut self) {
            if matches!(self.cfg.general.monitor.as_str(), "active" | "cursor") {
                let before = (self.mon, self.scale);
                self.refresh_monitor();
                if before != (self.mon, self.scale) {
                    self.kick();
                }
            }
        }

        fn refresh_monitor(&mut self) {
            unsafe {
                let hmon = self.wanted_monitor();
                let mut mi = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                self.hmon = hmon.0 as isize;
                if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                    let r = mi.rcMonitor;
                    // A taskbar docked at the top (any edge works since 24H2): hang below it.
                    let top = mi.rcWork.top.max(r.top);
                    self.mon = (r.left, top, r.right, r.bottom);
                }
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                self.scale = dx as f32 / 96.0 * self.cfg.general.pill_scale.clamp(0.7, 1.6);
            }
            self.rend.set_scale(self.scale);
            // Hot zone for drag arming, in screen px.
            let half = (230.0 * self.scale) as i32;
            let mid = (self.mon.0 + self.mon.2) / 2;
            ZONE_L.store(mid - half, Ordering::Relaxed);
            ZONE_R.store(mid + half, Ordering::Relaxed);
            ZONE_B.store(self.mon.1 + (40.0 * self.scale) as i32, Ordering::Relaxed);
        }

        fn handle_events(&mut self, hwnd: HWND) {
            let now = Instant::now();
            let events: Vec<UiEvent> = std::mem::take(&mut *self.shared.pending.lock().unwrap());
            for e in events {
                match e {
                    UiEvent::Task(mut u) => {
                        if let crate::tasks::TaskUpdate::Upsert(s) = &mut u {
                            // Downloads switched off in settings: its watcher may still run.
                            if s.source == "download" && !self.cfg.downloads.enabled {
                                continue;
                            }
                            if s.source == "shell"
                                && s.delay_ms == 0
                                && s.event == hytte_proto::TaskEvent::Start
                            {
                                s.delay_ms = self.cfg.shell.threshold_ms;
                            }
                            if s.event == hytte_proto::TaskEvent::NeedsInput {
                                let until = now + Duration::from_secs(self.cfg.agent.peek_secs);
                                self.model.peek(Panel::Tasks, until);
                                // An agent waiting on you shows even over a maximised terminal.
                                self.model.force_until = Some(until);
                                if self.cfg.agent.sound && !self.quiet() {
                                    unsafe {
                                        let _ =
                                            windows::Win32::System::Diagnostics::Debug::MessageBeep(
                                                MB_ICONASTERISK,
                                            );
                                    }
                                }
                            }
                        }
                        // First sight of a task with an owner: remember which terminal tab it ran in.
                        if let crate::tasks::TaskUpdate::Upsert(s) = &u {
                            if let (Some(pid), false) =
                                (s.pid, self.model.tasks.iter().any(|t| t.id == s.task_id))
                            {
                                let _ = self
                                    .tabs
                                    .send(crate::tabs::Cmd::Snap(s.task_id.clone(), pid));
                            }
                        }
                        let finished = match &u {
                            crate::tasks::TaskUpdate::Upsert(s)
                                if matches!(
                                    s.event,
                                    hytte_proto::TaskEvent::Done | hytte_proto::TaskEvent::Failed
                                ) =>
                            {
                                Some(s.task_id.clone())
                            }
                            _ => None,
                        };
                        self.model.apply_task(u, now);
                        // Open the pill on the result, unless the task was hidden (a short
                        // shell command, or Ctrl-C) and so never made it into the list.
                        let secs = self.cfg.general.finish_peek_secs;
                        if secs > 0
                            && finished
                                .is_some_and(|id| self.model.tasks.iter().any(|t| t.id == id))
                        {
                            self.model
                                .peek(Panel::Tasks, now + Duration::from_secs(secs));
                        }
                    }
                    UiEvent::Ports(p, parents) => self.model.set_ports(p, parents),
                    UiEvent::TaskGone(id) => self.model.remove_task(&id),
                    UiEvent::Calendar(ev) => self.model.set_calendar(
                        ev,
                        crate::timer::now_ms() as i64,
                        self.cfg.calendar.lead_min as i64 * 60_000,
                        now,
                    ),
                    UiEvent::DownloadDone { name, path } => {
                        if self.model.chip.is_none() && self.cfg.downloads.enabled {
                            let mut c = Chip::new(format!("{name} finished downloading"), now);
                            c.open = Some(path.clone());
                            c.extra = vec![("Shelve".into(), ChipAction::Shelve(path))];
                            self.model.chip = Some(c);
                        }
                    }
                    UiEvent::Hog(a) => {
                        // Never over another card, and never for a process the user muted.
                        let muted = self.model.hog_ignore.contains(&a.exe.to_ascii_lowercase());
                        if !muted && self.model.chip.is_none() {
                            let mut c = Chip::new(a.summary(), now);
                            c.tone = crate::ui_state::Tone::Warn;
                            c.hold = Duration::from_secs(20);
                            c.extra = vec![
                                ("Kill".into(), ChipAction::KillPid(a.pid)),
                                ("Ignore".into(), ChipAction::IgnoreExe(a.exe)),
                            ];
                            self.model.chip = Some(c);
                        }
                    }
                    UiEvent::ShelfAdd(job) => {
                        // Copying (shelf.mode = "copy") and writing text snippets can take
                        // a while: do the file work on a worker so the pill keeps animating.
                        let cfg = self.cfg.shelf.clone();
                        let existing = crate::shelf::paths(&self.model.shelf);
                        let room = crate::shelf::room(&cfg, &self.model.shelf);
                        let sh = self.shared.clone();
                        std::thread::spawn(move || {
                            let staged = crate::shelf::stage(
                                &cfg,
                                &job.paths,
                                job.text.as_deref(),
                                &existing,
                                room,
                            );
                            if !staged.is_empty() {
                                push_event(&sh, UiEvent::ShelfStaged(staged));
                            }
                        });
                    }
                    UiEvent::ShelfStaged(staged) => {
                        self.add_staged(staged, now);
                    }
                    UiEvent::Screenshot(it) => {
                        let path = it.path.clone();
                        if self.add_staged(vec![it], now) > 0 {
                            self.track_shot(hwnd, path, now);
                        }
                    }
                    UiEvent::Thumb(id, bmp) => {
                        if let Some(b) = bmp {
                            self.rend.set_thumb(id, &b);
                        }
                    }
                    // Mid-seek, a late position update would yank the knob back.
                    UiEvent::Media(m) if !self.scrubbing(&Action::Seek) => {
                        self.model.apply_media(m, now)
                    }
                    UiEvent::Media(_) => {}
                    UiEvent::MediaArt(a) => self.model.set_art(a),
                    UiEvent::MicMute(m) => self.model.mic_muted = m,
                    // Mid-drag, a late notification for an earlier level would pull the bar back.
                    UiEvent::Speaker(s) if !self.scrubbing(&Action::Volume) => {
                        self.model.speaker = s
                    }
                    UiEvent::Speaker(_) => {}
                    UiEvent::Power(p) => self.model.set_power(p, now),
                    UiEvent::Privacy(c, m, app) => {
                        self.model.cam = c;
                        self.model.mic = m;
                        self.model.privacy_app = app;
                    }
                    UiEvent::DropDone(r) => {
                        self.anim.squash();
                        self.model.chip = Some(Chip {
                            open: r.open,
                            copy: r.copy,
                            ..Chip::new(r.summary, now)
                        });
                    }
                    UiEvent::SetClipboard(s) => set_clipboard(hwnd, &s),
                    UiEvent::DragEnter => self.model.drop_over = true,
                    UiEvent::DragLeave => self.model.drop_over = false,
                    UiEvent::Config(cfg) => self.apply_config(*cfg),
                    // Nowhere to show it: dropping `reply` sends the agent to its terminal prompt now.
                    UiEvent::Ask { .. } if self.paused || self.gaming => {}
                    UiEvent::Ask { id, summary, reply } => {
                        // Urgent: it takes the chip over; a second agent's ask queues behind.
                        self.asks.push_back((id, summary, now, reply));
                        self.show_ask(now);
                        if self.cfg.agent.sound && !self.quiet() {
                            unsafe {
                                let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
                                    MB_ICONASTERISK,
                                );
                            }
                        }
                    }
                }
            }
            self.after_model_change(hwnd);
        }

        /// A live edit of `config.toml`. Most settings are read where they're used, so only
        /// state derived from the config needs refreshing. Lowering `[shelf] max_items` keeps
        /// the tiles already there and only limits new ones.
        fn apply_config(&mut self, new: Config) {
            let hotkey_changed = new.general.hotkey != self.cfg.general.hotkey;
            if new.general.hide_from_capture != self.cfg.general.hide_from_capture {
                set_capture_hidden(
                    HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _),
                    new.general.hide_from_capture,
                );
            }
            if new.general.autostart != self.cfg.general.autostart {
                crate::config::ensure_autostart(new.general.autostart);
            }
            if new.ports != self.cfg.ports {
                crate::ports::reconfigure(new.ports.clone());
            }
            if new.hog != self.cfg.hog {
                crate::ports::reconfigure_hog(new.hog.clone());
            }
            if new.downloads.enabled {
                crate::downloads::start(new.downloads.clone());
            } else {
                self.model.tasks.retain(|t| t.source != "download");
            }
            if new.calendar.enabled {
                crate::calendar::start();
            }
            crate::update::ENABLED.store(new.general.check_updates, Ordering::Relaxed);
            self.model.ignore = ignore_list(&new);
            self.light = theme_is_light(&new.general.theme);
            self.model.adaptive_glow = new.general.adaptive_glow;
            self.model.finish_peek = Duration::from_secs(new.general.finish_peek_secs);
            self.model.glow_gain = new.general.glow_strength.clamp(0.0, 3.0);
            self.model.body_alpha = new.general.pill_opacity.clamp(0.4, 1.0);
            self.model.task_rows = new.general.task_rows;
            self.model.rounds = new.timer.rounds;
            let monitor_changed = new.general.monitor != self.cfg.general.monitor
                || new.general.pill_scale != self.cfg.general.pill_scale;
            let general = new.general != self.cfg.general;
            crate::settings_win::refresh(&new);
            self.cfg = new;
            if hotkey_changed {
                self.register_hotkey(HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _));
            }
            if monitor_changed {
                self.refresh_monitor();
            }
            if general {
                self.evaluate_fullscreen(HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _));
            }
        }

        /// Open the pill for keyboard use (or close it again): take focus, hold it open.
        fn toggle_keyboard(&mut self, hwnd: HWND) {
            if self.kb.is_some() {
                self.end_keyboard(true);
                return;
            }
            unsafe {
                let prev = GetForegroundWindow();
                self.kb = Some(prev);
                self.kb_focus = None;
                self.hover = true;
                let _ = SetForegroundWindow(hwnd);
            }
            crate::ports::request_refresh();
            self.after_model_change(hwnd);
        }

        /// The pointer has rested on the pill long enough: open it.
        fn open_hover(&mut self) {
            self.hover = true;
            // Opening the pill: make sure the port list and charge rate are not stale.
            crate::ports::request_refresh();
            if self.model.panel() == Panel::Power && self.model.power.is_some() {
                self.model.power = crate::power::read().or(self.model.power);
            }
            self.layout();
            self.kick();
        }

        /// Leave keyboard mode; `restore` hands focus back to the window that had it.
        fn end_keyboard(&mut self, restore: bool) {
            let Some(prev) = self.kb.take() else { return };
            self.kb_focus = None;
            self.hover = self.inside;
            if restore && !prev.0.is_null() {
                unsafe {
                    let _ = SetForegroundWindow(prev);
                }
            }
            self.layout();
            self.kick();
        }

        /// A key pressed while the pill has focus.
        fn key(&mut self, hwnd: HWND, vk: u16) {
            use windows::Win32::UI::Input::KeyboardAndMouse::*;
            if self.kb.is_none() {
                return;
            }
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            match VIRTUAL_KEY(vk) {
                VK_ESCAPE => self.end_keyboard(true),
                VK_LEFT | VK_RIGHT => {
                    let dir = if VIRTUAL_KEY(vk) == VK_RIGHT { 1 } else { -1 };
                    let p = self.model.step_panel(dir);
                    self.select_panel(p);
                    self.anim.nav = dir as f32;
                    self.kb_focus = None;
                    self.after_model_change(hwnd);
                }
                // On the timer setup card, digits type the minutes and Backspace erases one.
                VIRTUAL_KEY(k @ (0x30..=0x39 | 0x60..=0x69)) if self.setting_timer() => {
                    let now = Instant::now();
                    let fresh = self
                        .model
                        .timer_typed
                        .is_none_or(|t| now.saturating_duration_since(t) > TYPE_GAP);
                    let d = (k % 0x30) as u32;
                    self.model.timer_min = crate::timer::typed(self.model.timer_min, d, fresh);
                    self.model.timer_typed = Some(now);
                    self.after_model_change(hwnd);
                }
                VK_BACK if self.setting_timer() => {
                    self.model.timer_min = (self.model.timer_min / 10).max(1);
                    self.model.timer_typed = Some(Instant::now());
                    self.after_model_change(hwnd);
                }
                // 1-9: jump straight to that pane (left to right, as the tab strip shows them).
                VIRTUAL_KEY(k @ 0x31..=0x39) => {
                    let ps = self.model.panels();
                    let Some(&p) = ps.get((k - 0x31) as usize) else {
                        return;
                    };
                    self.select_panel(p);
                    self.kb_focus = None;
                    self.after_model_change(hwnd);
                }
                VK_TAB => {
                    let n = self.hits.len();
                    if n > 0 {
                        self.kb_focus = Some(match (self.kb_focus, shift) {
                            (None, false) => 0,
                            (None, true) => n - 1,
                            (Some(i), false) => (i + 1) % n,
                            (Some(i), true) => (i + n - 1) % n,
                        });
                    }
                    self.kick();
                }
                VK_RETURN | VK_SPACE => {
                    if let Some(a) = self
                        .kb_focus
                        .and_then(|i| self.hits.get(i))
                        .map(|h| h.action.clone())
                    {
                        self.run_action(hwnd, a);
                    }
                }
                _ => {}
            }
        }

        /// Run a failed command again in a new terminal, and clear its row: the new run
        /// reports itself (a `hytte run` one, or a shell command once it passes the threshold).
        fn rerun(&mut self, id: &str) {
            let Some(t) = self
                .model
                .tasks
                .iter()
                .find(|t| t.id == id && t.rerunnable())
            else {
                return;
            };
            let shell = t.pid.and_then(hytte_proto::ports::exe_path);
            let hytte = std::env::current_exe().unwrap_or_default();
            let plan = crate::rerun::plan(
                &t.source,
                &t.label,
                shell.as_deref(),
                &hytte.to_string_lossy(),
            );
            let cwd = t.cwd.clone().unwrap_or_default();
            match crate::rerun::start(&plan, &cwd) {
                Ok(()) => self.model.remove_task(id),
                Err(e) => {
                    crate::logging::line(&format!("rerun failed: {e}"));
                    let mut c = Chip::new(format!("Couldn't rerun it: {e}"), Instant::now());
                    c.tone = crate::ui_state::Tone::Warn;
                    self.model.chip = Some(c);
                }
            }
        }

        /// The timer setup card is open (no timer running): digits type its minutes.
        fn setting_timer(&self) -> bool {
            self.model.timer_panel && self.model.timer.is_none()
        }

        /// (Re)register the global hotkey from the config.
        fn register_hotkey(&self, hwnd: HWND) {
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
            };
            unsafe {
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
                let spec = &self.cfg.general.hotkey;
                // A game may want the same chord: give it back while one is in front.
                if spec.is_empty() || self.gaming {
                    return;
                }
                match crate::config::parse_hotkey(spec) {
                    Some((mods, vk)) => {
                        if RegisterHotKey(
                            Some(hwnd),
                            HOTKEY_ID,
                            HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT,
                            vk,
                        )
                        .is_err()
                        {
                            crate::logging::line(&format!(
                                "hotkey {spec}: already taken by another app"
                            ));
                        }
                    }
                    None => crate::logging::line(&format!("hotkey {spec}: not understood")),
                }
            }
        }

        /// Put staged items on the shelf and open it to show them.
        fn add_staged(&mut self, staged: Vec<crate::shelf::ShelfItem>, now: Instant) -> usize {
            let n = crate::shelf::insert(&self.cfg.shelf, &mut self.model.shelf, staged);
            if n > 0 {
                self.save_shelf();
                self.model.peek(Panel::Shelf, now + Duration::from_secs(5));
                self.model.force_until = Some(now + Duration::from_secs(5));
                self.anim.squash();
            }
            n
        }

        /// A screenshot went on the shelf: it leaves after `screenshot_secs`, and with
        /// `delete_after_paste` it is deleted once pasted.
        fn track_shot(&mut self, hwnd: HWND, path: std::path::PathBuf, now: Instant) {
            self.shots.push((path.clone(), now));
            self.paste = self.cfg.shelf.delete_after_paste.then(|| Paste {
                path,
                until: now + Duration::from_secs(600),
                key_down: ctrl_v_down(),
                due: None,
            });
            self.arm_shots(hwnd);
        }

        fn arm_shots(&self, hwnd: HWND) {
            let ms = if self.paste.is_some() && !self.gaming {
                60
            } else if !self.shots.is_empty() && self.cfg.shelf.screenshot_secs > 0 {
                1000
            } else {
                0
            };
            if ms > 0 {
                unsafe {
                    SetTimer(Some(hwnd), T_SHOT, ms, None);
                }
            }
        }

        /// Take screenshots off the shelf that outstayed `screenshot_secs` or were pasted.
        fn tick_shots(&mut self, hwnd: HWND) {
            let now = Instant::now();
            let life = Duration::from_secs(self.cfg.shelf.screenshot_secs);
            let mut gone: Vec<std::path::PathBuf> = Vec::new();
            if !life.is_zero() {
                self.shots.retain(|(p, at)| {
                    let old = now.duration_since(*at) >= life;
                    if old {
                        gone.push(p.clone());
                    }
                    !old
                });
            }
            if !self.cfg.shelf.delete_after_paste
                || self.paste.as_ref().is_some_and(|p| now >= p.until)
            {
                self.paste = None;
            }
            if let Some(p) = &mut self.paste {
                if let Some(due) = p.due {
                    // Pasted: give the target a moment to take the image, then delete.
                    if now >= due {
                        gone.push(p.path.clone());
                        self.paste = None;
                    }
                } else if unsafe { GetClipboardSequenceNumber() } != self.snip_seq {
                    // Something else was copied: that screenshot can't be pasted any more.
                    self.paste = None;
                } else if !self.gaming {
                    let down = ctrl_v_down();
                    if down && !p.key_down {
                        p.due = Some(now + Duration::from_secs(2));
                    }
                    p.key_down = down;
                }
            }
            if !gone.is_empty() {
                for p in gone {
                    // Only what is still on the shelf: a file the user dragged out is theirs.
                    if let Some(id) = self
                        .model
                        .shelf
                        .iter()
                        .find(|i| i.owned && i.path == p)
                        .map(|i| i.id)
                    {
                        crate::shelf::remove(&mut self.model.shelf, id);
                        if self.model.shelf_sel == Some(id) {
                            self.model.shelf_sel = None;
                        }
                    }
                    self.shots.retain(|(q, _)| *q != p);
                }
                self.save_shelf();
                self.layout();
                self.after_model_change(hwnd);
            }
            self.arm_shots(hwnd);
        }

        /// Sounds are silent right now (`[general] quiet_hours`).
        fn quiet(&self) -> bool {
            let spec = &self.cfg.general.quiet_hours;
            if spec.is_empty() {
                return false;
            }
            let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
            crate::config::in_quiet_hours(spec, t.wHour as u32 * 60 + t.wMinute as u32)
        }

        fn save_shelf(&self) {
            if self.cfg.shelf.persist {
                crate::shelf::save(&self.model.shelf);
            }
        }

        /// Ask a worker for thumbnails of visible shelf tiles we do not have yet.
        fn request_thumbs(&mut self) {
            for id in self.rend.missing_thumbs(&self.model) {
                if !self.thumb_req.insert(id) {
                    continue;
                }
                if let Some(it) = self.model.shelf.iter().find(|i| i.id == id) {
                    let path = it.path.clone();
                    let sh = self.shared.clone();
                    std::thread::spawn(move || {
                        let b = crate::shelf::thumbnail(&path).map(Arc::new);
                        push_event(&sh, UiEvent::Thumb(id, b));
                    });
                }
            }
        }

        /// Keep the pill on the virtual desktop the user is looking at.
        fn follow_desktop(&self, hwnd: HWND) {
            let Some(vdm) = &self.vdm else { return };
            unsafe {
                if vdm
                    .IsWindowOnCurrentVirtualDesktop(hwnd)
                    .map(|b| b.as_bool())
                    .unwrap_or(true)
                {
                    return;
                }
                let fg = GetForegroundWindow();
                if fg.0.is_null() {
                    return;
                }
                if let Ok(id) = vdm.GetWindowDesktopId(fg) {
                    let _ = vdm.MoveWindowToDesktop(hwnd, &id);
                }
            }
        }

        fn after_model_change(&mut self, hwnd: HWND) {
            let now = Instant::now();
            let wall = crate::timer::now_ms();
            if self.model.finish_timer(wall, now).is_some() {
                let t = &self.cfg.timer;
                self.model.timer_break = crate::timer::break_minutes(
                    self.model.focus_done,
                    t.rounds,
                    t.break_min,
                    t.long_break_min,
                );
                self.model.timer_panel = false;
                crate::timer::save(None);
            }
            // 1 Hz redraw for the fuse, only while a timer runs; no timer = no wake-ups.
            match &self.model.timer {
                Some(t) if t.paused_ms.is_none() => unsafe {
                    SetTimer(
                        Some(hwnd),
                        T_TIMER,
                        t.remaining_ms(wall).min(1000) as u32 + 15,
                        None,
                    );
                },
                _ => unsafe {
                    let _ = KillTimer(Some(hwnd), T_TIMER);
                },
            }
            self.model.set_idle(input_idle(), now);
            let mut next = self.model.expire(now);
            if self.model.chip.is_none() && !self.asks.is_empty() {
                // The front ask timed out (its agent asked in the terminal): show the next one.
                self.show_ask(now);
                next = self.model.expire(now);
            }
            match next {
                Some(next) => unsafe {
                    let ms = next.saturating_duration_since(now).as_millis() as u32 + 20;
                    SetTimer(Some(hwnd), T_EXPIRE, ms.max(20), None);
                },
                None => unsafe {
                    let _ = KillTimer(Some(hwnd), T_EXPIRE);
                },
            }
            self.sync_ring();
            // Keep awake: held by this thread, released by Windows if Hytte exits.
            let awake = self.model.awake.active(now);
            if awake != self.awake_on {
                self.awake_on = awake;
                use windows::Win32::System::Power::{
                    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
                };
                let flags = if awake {
                    ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
                } else {
                    ES_CONTINUOUS
                };
                unsafe { SetThreadExecutionState(flags) };
            }
            // A calendar heads-up that has come due becomes a card (never over another one).
            // Switched off in settings: a due heads-up is dropped, not saved for later.
            let calendar = self.cfg.calendar.enabled;
            if self.model.chip.is_none() || !calendar {
                if let Some(ev) = self.model.calendar_due(now).filter(|_| calendar) {
                    let text = crate::calendar::summary(
                        &ev.subject,
                        ev.start_ms,
                        crate::timer::now_ms() as i64,
                    );
                    let mut c = Chip::new(text, now);
                    c.tone = crate::ui_state::Tone::Info;
                    c.hold = Duration::from_secs(60);
                    if let Some(url) = ev.url {
                        c.extra = vec![("Join".into(), ChipAction::OpenUrl(url))];
                    }
                    self.model.chip = Some(c);
                }
            }
            self.request_thumbs();
            self.layout();
            // `nav` only applies to the layout it was set for.
            self.anim.nav = 0.0;
            self.kick();
        }

        fn frame(&mut self, hwnd: HWND) {
            let now = Instant::now();
            let gap = now.saturating_duration_since(self.last);
            // While springs move the ticker paces on vblank: use DWM's own clock so dt is a
            // whole number of refresh periods instead of scheduler-jittered wall time.
            let vb = if self.shared.fast.load(Ordering::Relaxed) {
                VBLANK.load(Ordering::Relaxed)
            } else {
                0
            };
            let dt = vblank_dt(self.prev_vblank, vb)
                .map_or(gap.as_secs_f32(), |d| d as f32)
                .min(0.05);
            self.prev_vblank = vb;
            self.last = now;
            self.layout();
            let speed = match self.cfg.general.animation.as_str() {
                "calm" => 0.7,
                "snappy" => 1.4,
                _ => 1.0,
            };
            let mut moving = self.anim.step((dt * speed) as f64);
            // The idle pill fades towards `idle_opacity` and back when it is pointed at.
            let idle = self.model.scene(self.hover, self.sentinel()) == Scene::Idle
                && self.kb.is_none()
                && !self.inside;
            let dim_to = if idle {
                self.cfg.general.idle_opacity.clamp(0.2, 1.0)
            } else {
                1.0
            };
            self.dim += (dim_to - self.dim) * (1.0 - (-dt * 6.0).exp());
            if (dim_to - self.dim).abs() < 0.004 {
                self.dim = dim_to;
            } else {
                moving = true;
            }

            // Visibility: fade out then hide; show before fading in.
            let want_hidden = self.hidden();
            if !want_hidden && !self.shown {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                self.shown = true;
            }
            let faded_out = want_hidden && self.anim.vis.pos < 0.01;
            if faded_out && self.shown {
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                self.shown = false;
            }
            if self.shown {
                let crop = self.crop();
                let origin = self.origin_for(crop);
                self.origin = origin;
                self.crop_w = crop.w_log;
                let fr = Frame {
                    model: &self.model,
                    cfg: &self.cfg,
                    anim: &self.anim,
                    now,
                    dt,
                    mouse: self.mouse_logical(),
                    pressed: self.pressed,
                    light: self.light,
                    accent: accent_for(self.accent, self.light),
                    focus: self.kb.and(self.kb_focus),
                    armed: self.armed,
                    ringing: self.ringing,
                };
                self.rend.draw(&fr, crop, &mut self.hits);
                let drawn = now.elapsed().as_micros() as u32;
                let a = (self.anim.vis.pos.clamp(0.0, 1.0) * self.dim as f64 * 255.0).round() as u8;
                let heading = self
                    .rend
                    .plan(
                        self.anim.rect.w.target as f32,
                        self.anim.rect.h.target as f32,
                        self.armed,
                        self.anim.bubble_room(),
                        self.glowing(),
                    )
                    .clip;
                self.rend
                    .present(hwnd, crop, origin.0, origin.1, a, heading);
                if self.rend.lost() && self.gpu_retry == 0 {
                    // Driver reset or adapter removed: rebuild the GPU pipeline and redraw.
                    crate::logging::line("gpu renderer: device lost, rebuilding");
                    self.rebuild_gpu(hwnd);
                }
                // Skip the first frame after idle: its gap is the idle time, not a frame.
                if moving && gap.as_millis() < 100 {
                    self.perf.record(
                        gap.as_micros() as u32,
                        drawn,
                        now.elapsed().as_micros() as u32,
                    );
                }
            }
            // A hovered label scrolling (`Renderer::label`) runs only while it is hovered.
            let ambient = !self.anim.reduce
                && self.shown
                && (self.model.ambient(self.anim.scene, now) || self.rend.wants_frames());
            let sh = &self.shared;
            sh.fast.store(moving, Ordering::Relaxed);
            sh.animating.store(moving || ambient, Ordering::SeqCst);
            if !(moving || ambient) {
                // Settled: give the pages back once it has stayed that way.
                unsafe {
                    SetTimer(Some(hwnd), T_TRIM, TRIM_MS, None);
                }
            }
        }

        /// Rebuild a lost GPU pipeline. On failure (driver mid-update, adapter gone) retry on
        /// `T_GPU` with backoff instead of hammering the driver every frame.
        fn rebuild_gpu(&mut self, hwnd: HWND) {
            match self.rend.rebuild_gpu(hwnd) {
                Ok(()) => {
                    self.gpu_retry = 0;
                    self.thumb_req.clear();
                    self.kick();
                }
                Err(e) => {
                    self.gpu_retry = (self.gpu_retry * 2).clamp(1_000, 30_000);
                    crate::logging::line(&format!(
                        "gpu renderer: rebuild failed: {e}; retry in {} ms",
                        self.gpu_retry
                    ));
                    unsafe {
                        SetTimer(Some(hwnd), T_GPU, self.gpu_retry, None);
                    }
                }
            }
        }

        fn hit_at(&self) -> Option<&Hit> {
            let (mx, my) = self.mouse_logical()?;
            self.hits
                .iter()
                .rev()
                .find(|h| mx >= h.rect.0 && mx <= h.rect.2 && my >= h.rect.1 && my <= h.rect.3)
        }

        fn run_action(&mut self, hwnd: HWND, a: Action) {
            match a {
                Action::DismissTask(id) => self.model.dismiss_task(&id),
                Action::DismissChip => {
                    let was_ask = self.model.chip.as_ref().is_some_and(|c| {
                        c.extra
                            .iter()
                            .any(|(_, a)| matches!(a, ChipAction::Answer(..)))
                    });
                    self.model.dismiss_chip();
                    if was_ask {
                        self.asks.pop_front();
                        self.show_ask(Instant::now());
                    }
                }
                Action::Chip(a) => self.chip_action(a),
                Action::CopyText(s) => set_clipboard(hwnd, &s),
                Action::OpenPath(p) => {
                    let _ = std::process::Command::new("explorer").arg(p).spawn();
                }
                Action::MediaPrev => crate::media::control(crate::media::Cmd::Prev),
                Action::MediaToggle => crate::media::control(crate::media::Cmd::Toggle),
                Action::MediaNext => crate::media::control(crate::media::Cmd::Next),
                Action::FocusTerminal(pid, id) => {
                    if let Some(h) = crate::proc::terminal_window_for(pid) {
                        crate::proc::focus_window(h);
                        let _ = self.tabs.send(crate::tabs::Cmd::Select(id));
                    }
                }
                Action::SelectPanel(p) => self.select_panel(p),
                Action::ToggleMic => self.toggle_mic(),
                Action::ToggleSpeaker => {
                    if let Some(s) = crate::mic::toggle_speaker() {
                        self.model.speaker = Some(s);
                    }
                }
                // Mouse presses on the bar are handled as a drag (`scrub_to`); this is the keyboard.
                Action::Volume => {
                    if let Some(s) = crate::mic::toggle_speaker() {
                        self.model.speaker = Some(s);
                    }
                }
                Action::ToggleApps => {
                    self.model.apps_open = !self.model.apps_open;
                    if self.model.apps_open {
                        self.model.apps = crate::mic::apps();
                    }
                }
                // Dragged with the mouse (`scrub_to`); from the keyboard: mute / unmute it.
                Action::AppVolume(exe) => {
                    if let Some(a) = self.model.apps.iter().find(|a| a.exe == exe) {
                        let v = if a.muted || a.vol == 0.0 { 1.0 } else { 0.0 };
                        self.set_app_volume(&exe, v);
                    }
                }
                // Mouse presses seek by dragging (`scrub_to`); from the keyboard: play / pause.
                Action::Seek => crate::media::control(crate::media::Cmd::Toggle),
                Action::CycleAwake => self.model.awake = self.model.awake.next(Instant::now()),
                Action::OpenTimer => {
                    if self.model.timer.is_some() {
                        self.select_panel(Panel::Timer);
                    } else {
                        self.model.timer_panel = true;
                    }
                }
                Action::TimerNudge(d) => {
                    self.model.timer_min = crate::timer::nudge(self.model.timer_min, d);
                }
                Action::TimerMinutes(m) => self.model.timer_min = m,
                Action::TimerStart(k) => self.start_timer(k, self.model.timer_min),
                Action::TimerBreak(m) => self.start_timer(crate::timer::TimerKind::Break, m),
                Action::TimerAdd(m) => {
                    if let Some(t) = &mut self.model.timer {
                        t.add_minutes(m as u64);
                        crate::timer::save(Some(t));
                    } else if self.model.timer_done.is_some() {
                        self.start_timer(crate::timer::TimerKind::Plain, m);
                    }
                }
                Action::TimerPause => {
                    if let Some(t) = &mut self.model.timer {
                        t.toggle_pause(crate::timer::now_ms());
                        crate::timer::save(Some(t));
                    }
                }
                Action::TimerStop => {
                    self.model.timer = None;
                    crate::timer::save(None);
                }
                Action::TimerDismiss => self.model.timer_done = None,
                Action::TimerClose => self.model.timer_panel = false,
                Action::Settings(g) => self.model.settings = g,
                // Dragged: handled by `scrub_to`.
                Action::Opacity | Action::Jump(_) | Action::EditConfig(_) => {}
                Action::Set(key, v) => match crate::config::set(key, v) {
                    Ok(Some(cfg)) => self.apply_config(cfg),
                    Ok(None) => {
                        let mut c = Chip::new(
                            "config.toml has a mistake in it: fix it first",
                            Instant::now(),
                        );
                        c.tone = crate::ui_state::Tone::Warn;
                        c.open = Some(crate::config::config_path());
                        self.model.chip = Some(c);
                    }
                    Err(e) => crate::logging::line(&format!("settings: {e}")),
                },
                Action::OpenConfig => {
                    self.model.settings = None;
                    crate::settings_win::open(&self.cfg);
                }
                Action::Rerun(id) => self.rerun(&id),
                Action::ShelfScroll(by) => self.model.scroll_shelf(by),
                Action::SetPowerMode(m) => {
                    crate::power::set_mode(m);
                    self.model.power = crate::power::read();
                }
                Action::FocusMedia(app) => crate::proc::focus_app(&app),
                Action::OpenPort(port) | Action::KillPort(port, _)
                    if !port_alive(&self.model, port) =>
                {
                    // The process is gone since the last scan: drop it instead of erroring.
                    self.model.ports.retain(|p| p.port != port);
                    self.model.kill_armed = None;
                    crate::ports::request_refresh();
                }
                Action::OpenPort(port) => unsafe {
                    let url: Vec<u16> = format!("http://localhost:{port}\0")
                        .encode_utf16()
                        .collect();
                    ShellExecuteW(
                        None,
                        windows::core::w!("open"),
                        windows::core::PCWSTR(url.as_ptr()),
                        None,
                        None,
                        SW_SHOWNORMAL,
                    );
                },
                Action::KillPort(port, pid) => {
                    if self.model.kill_armed.is_some_and(|(p, _)| p == port) {
                        self.model.kill_armed = None;
                        let msg = match hytte_proto::ports::kill(pid) {
                            Ok(()) => format!("Stopped :{port}"),
                            Err(e) => format!("Could not stop :{port} - {e}"),
                        };
                        self.model.chip = Some(Chip::new(msg, Instant::now()));
                        crate::ports::request_refresh();
                    } else {
                        self.model.kill_armed =
                            Some((port, Instant::now() + crate::ui_state::KILL_CONFIRM));
                    }
                }
                Action::ShelfTile(id) => {
                    self.model.shelf_sel = if self.model.shelf_sel == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                }
                Action::RemoveShelf(id) => {
                    crate::shelf::remove(&mut self.model.shelf, id);
                    if self.model.shelf_sel == Some(id) {
                        self.model.shelf_sel = None;
                    }
                    self.save_shelf();
                }
                Action::ShelfOp(id, op) => {
                    if let Some(it) = self.model.shelf.iter().find(|i| i.id == id) {
                        let job = DropJob {
                            paths: vec![it.path.clone()],
                            text: None,
                            op: Some(op),
                        };
                        self.model.chip = Some(Chip::new("Working…", Instant::now()));
                        let _ = self.shared.drop_tx.send(job);
                    }
                }
            }
            self.after_model_change(hwnd);
        }

        /// One of a chip's extra buttons.
        fn chip_action(&mut self, a: ChipAction) {
            match a {
                ChipAction::OpenUrl(url) => {
                    open_url(&url);
                    self.model.dismiss_chip();
                }
                ChipAction::Shelve(path) => {
                    let job = DropJob {
                        paths: vec![path],
                        text: None,
                        op: None,
                    };
                    push_event(&self.shared, UiEvent::ShelfAdd(job));
                    self.model.dismiss_chip();
                }
                ChipAction::KillPid(pid) => {
                    if self.model.chip_armed.take().is_some() {
                        let msg = match hytte_proto::ports::kill(pid) {
                            Ok(()) => format!("Stopped process {pid}"),
                            Err(e) => format!("Could not stop process {pid} - {e}"),
                        };
                        self.model.chip = Some(Chip::new(msg, Instant::now()));
                    } else {
                        self.model.chip_armed =
                            Some(Instant::now() + crate::ui_state::KILL_CONFIRM);
                    }
                }
                ChipAction::IgnoreExe(exe) => {
                    self.model.hog_ignore.insert(exe.to_ascii_lowercase());
                    self.model.dismiss_chip();
                }
                ChipAction::Answer(id, allow) => {
                    if let Some(i) = self.asks.iter().position(|a| a.0 == id) {
                        if let Some((.., reply)) = self.asks.remove(i) {
                            let _ = reply.send(allow);
                        }
                    }
                    self.model.dismiss_chip();
                    self.show_ask(Instant::now());
                }
            }
        }

        /// Put the oldest live agent ask on the chip, "(1/N)" when more wait behind it. Asks
        /// past `ASK_WAIT_SECS` are dropped: their pipe already gave up and the agent asked in
        /// its terminal.
        fn show_ask(&mut self, now: Instant) {
            let wait = Duration::from_secs(hytte_proto::ASK_WAIT_SECS);
            self.asks.retain(|a| now.duration_since(a.2) < wait);
            let Some((id, summary, at, _)) = self.asks.front() else {
                return;
            };
            let summary = match self.asks.len() {
                1 => summary.clone(),
                n => format!("(1/{n}) {summary}"),
            };
            let mut c = Chip::new(summary, *at);
            c.tone = crate::ui_state::Tone::Warn;
            c.hold = wait;
            c.extra = vec![
                ("Allow".into(), ChipAction::Answer(*id, true)),
                ("Deny".into(), ChipAction::Answer(*id, false)),
            ];
            self.model.chip = Some(c);
            self.model.chip_armed = None;
        }

        /// The clipboard changed: a screenshot from the Snipping Tool (Win+Shift+S) goes on
        /// the shelf as a PNG. Other apps' images are left alone, and nothing is shelved over
        /// a game or while paused (the shelf would pop open).
        fn on_clipboard(&mut self, hwnd: HWND) {
            const CF_DIB: u32 = 8;
            if !self.cfg.shelf.screenshots || self.gaming || self.paused {
                return;
            }
            let mut pid = 0u32;
            unsafe {
                let Ok(owner) = GetClipboardOwner() else {
                    return;
                };
                GetWindowThreadProcessId(owner, Some(&mut pid));
            }
            let snip = crate::proc::exe_name(pid).is_some_and(|e| {
                let e = e.to_ascii_lowercase();
                e == "snippingtool.exe" || e == "screenclippinghost.exe"
            });
            if !snip || unsafe { IsClipboardFormatAvailable(CF_DIB) }.is_err() {
                return;
            }
            let Some(dib) = (unsafe { clipboard_bytes(hwnd, CF_DIB) }) else {
                return;
            };
            let hash = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                dib.hash(&mut h);
                h.finish()
            };
            if std::mem::replace(&mut self.last_snip, hash) == hash {
                return;
            }
            self.snip_seq = unsafe { GetClipboardSequenceNumber() };
            let sh = self.shared.clone();
            std::thread::spawn(move || {
                if let Some(it) = crate::shelf::stage_dib(&dib) {
                    push_event(&sh, UiEvent::Screenshot(it));
                }
            });
        }

        fn start_timer(&mut self, kind: crate::timer::TimerKind, minutes: u32) {
            let t = crate::timer::Timer::start(kind, minutes, crate::timer::now_ms());
            crate::timer::save(Some(&t));
            self.model.timer = Some(t);
            self.model.timer_done = None;
            self.model.timer_panel = false;
        }

        fn select_panel(&mut self, p: Panel) {
            // Page direction for the slide: towards a later panel is "next".
            let ps = self.model.panels();
            let at = |q: Panel| ps.iter().position(|x| *x == q);
            self.anim.nav = match (at(self.model.panel()), at(p)) {
                (Some(a), Some(b)) if b > a => 1.0,
                (Some(a), Some(b)) if b < a => -1.0,
                _ => 0.0,
            };
            self.model.timer_panel = false;
            self.model.settings = None;
            self.model.selected = Some(p);
            match p {
                Panel::Tasks => crate::ports::request_refresh(),
                Panel::Power => self.model.power = crate::power::read().or(self.model.power),
                Panel::Home if self.model.apps_open => self.model.apps = crate::mic::apps(),
                _ => {}
            }
        }

        /// Set the volume from where the pointer is on the bar being dragged.
        fn scrubbing(&self, what: &Action) -> bool {
            self.scrub.as_ref().is_some_and(|(a, _, _)| a == what)
        }

        fn scrub_to(&mut self) {
            let (Some((what, l, r)), Some((mx, _))) = (self.scrub.clone(), self.mouse_logical())
            else {
                return;
            };
            if what == Action::Seek {
                // The bar sits 4 px inside its hit box. Shown at once; sought on release.
                let f = ((mx - l - 4.0) / (r - l - 8.0)).clamp(0.0, 1.0) as f64;
                if let Some(md) = self.model.media.as_mut() {
                    md.pos_ms = (md.dur_ms as f64 * f) as u64;
                    self.model.media_stamp = Some(Instant::now());
                }
                self.kick();
                return;
            }
            let f = ((mx - l - 6.0) / (r - l - 12.0)).clamp(0.0, 1.0);
            let f = (f * 100.0).round() / 100.0;
            if what == Action::Opacity {
                // Live on the pill; written to config.toml on release.
                let v = 0.4 + 0.6 * f;
                self.cfg.general.pill_opacity = v;
                self.model.body_alpha = v;
                self.kick();
                return;
            }
            if let Action::AppVolume(exe) = &what {
                self.set_app_volume(exe, f);
                self.kick();
                return;
            }
            // Shown at once; Windows is only told when the level actually changed.
            let muted = self.model.speaker.is_some_and(|(_, m)| m) && f == 0.0;
            if self.model.speaker.map(|(v, _)| v) != Some(f) {
                self.model.speaker = Some((f, muted));
                let _ = crate::mic::set_volume(f);
            }
            self.kick();
        }

        /// Set one app's volume and show it at once (the list isn't re-read while dragging).
        fn set_app_volume(&mut self, exe: &str, v: f32) {
            let v = (v.clamp(0.0, 1.0) * 100.0).round() / 100.0;
            crate::mic::set_app_volume(exe, v);
            if let Some(a) = self.model.apps.iter_mut().find(|a| a.exe == exe) {
                a.vol = v;
                a.muted &= v == 0.0;
            }
        }

        fn toggle_mic(&mut self) {
            if let Some(m) = crate::mic::toggle() {
                self.model.mic_muted = m;
            }
        }

        /// Vertical wheel: the volume, the shelf row or the timer minutes under the pointer;
        /// anywhere else a notched mouse wheel pages the panes, one per notch (down = next).
        /// A precision touchpad's vertical scroll (deltas that aren't whole notches) doesn't
        /// page: it swipes sideways for that (`hwheel`).
        fn wheel(&mut self, delta: i32) {
            // ponytail: notch size tells mouse from touchpad; free-spin mice count as touchpads
            let notched = delta != 0 && delta % 120 == 0;
            self.wheel += delta;
            let steps = self.wheel / 120;
            if steps == 0 {
                return;
            }
            self.wheel -= steps * 120;
            // Over the volume bar or the sound tile, the wheel is the volume (2% a notch, as
            // in the Windows flyout).
            // Over the shelf row, the wheel scrolls it (down = further along).
            let over_shelf = self
                .hit_at()
                .is_some_and(|h| matches!(h.action, Action::ShelfTile(_) | Action::ShelfScroll(_)));
            if over_shelf {
                self.model.scroll_shelf(-steps);
                return;
            }
            let over_app = self.hit_at().and_then(|h| match &h.action {
                Action::AppVolume(exe) => Some(exe.clone()),
                _ => None,
            });
            if let Some(exe) = over_app {
                if let Some(a) = self.model.apps.iter().find(|a| a.exe == exe) {
                    let v = a.vol + steps as f32 * 0.02;
                    self.set_app_volume(&exe, v);
                }
                return;
            }
            let over_volume = self
                .hit_at()
                .is_some_and(|h| matches!(h.action, Action::Volume | Action::ToggleSpeaker));
            if over_volume {
                if let Some((v, _)) = self.model.speaker {
                    if let Some(s) = crate::mic::set_volume(v + steps as f32 * 0.02) {
                        self.model.speaker = Some(s);
                    }
                }
                return;
            }
            if self.hover
                && self.model.panel() == Panel::Tasks
                && self.model.scroll_tasks(-steps.signum())
            {
                return;
            }
            if self.model.timer_panel && self.model.timer.is_none() {
                self.model.timer_min = crate::timer::nudge(self.model.timer_min, steps);
                return;
            }
            if notched && self.model.settings.is_none() {
                self.page(-steps.signum());
            }
        }

        /// Sideways scroll pages the panes: right = next, one pane per swipe (`Swipe`).
        fn hwheel(&mut self, delta: i32) {
            if let Some(dir) = self.swipe.feed(delta, Instant::now()) {
                self.page(dir);
            }
        }

        /// One pane along: 1 = next, -1 = previous.
        fn page(&mut self, dir: i32) {
            let now = Instant::now();
            // Collapsed pills peek open so the change is visible.
            let p = self.model.step_panel(dir);
            self.select_panel(p);
            // `step_panel` already moved the selection, so give the slide its direction here.
            self.anim.nav = dir as f32;
            if !self.hover {
                self.model.peek(p, now + Duration::from_secs(3));
            }
        }

        /// The alarm rings as long as the finished card is up (dismissed, or its hold ran out),
        /// but not over a game: the card is hidden there, so nothing could stop it.
        fn sync_ring(&mut self) {
            let ring = self.cfg.timer.sound
                && self.model.timer_done.is_some()
                && !self.gaming
                && !self.quiet();
            if ring != self.ringing {
                self.ringing = ring;
                crate::timer::ring(ring);
            }
        }

        fn evaluate_fullscreen(&mut self, hwnd: HWND) {
            use crate::fullscreen::Suppress;
            // None: an overlay (Game Bar, GPU / OEM tools) is in front; keep the current state.
            let Some(s) = crate::fullscreen::evaluate(&self.cfg.general, self.hmon) else {
                return;
            };
            let gaming = s == Suppress::Game;
            let hide = s != Suppress::Show;
            let changed = gaming != self.gaming;
            let entering = changed && gaming;
            if changed {
                self.set_gaming(hwnd, gaming);
            }
            // Game -> maximised keeps fs_hidden true but must still bring the window back.
            if hide != self.fs_hidden || changed {
                self.fs_hidden = hide;
                if !hide {
                    self.hover = false;
                }
                self.layout();
                if entering {
                    // Gone at once: no fade frames drawn over the game.
                    self.anim.vis.snap();
                }
                self.kick();
            }
        }

        /// Step completely aside for a game, or come back. Nothing of Hytte should cost the
        /// game a frame or a millisecond of input latency: no window over it (an overlay
        /// forces DWM composition), no system-wide mouse / location hooks, no global hotkey,
        /// background CPU / IO / memory priority, and the pollers pause (`GAMING`).
        fn set_gaming(&mut self, hwnd: HWND, on: bool) {
            use windows::Win32::System::Threading::{
                SetPriorityClass, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
                PROCESS_MODE_BACKGROUND_BEGIN, PROCESS_MODE_BACKGROUND_END,
            };
            self.gaming = on;
            if on {
                // Pending agent asks go back to their terminals rather than wait unseen.
                self.asks.clear();
            }
            crate::fullscreen::GAMING.store(on, Ordering::Relaxed);
            input_hooks(!on);
            self.register_hotkey(hwnd);
            self.armed = false;
            ARMED.store(false, Ordering::Relaxed);
            // Background mode lowers I/O and memory priority; the idle class lowers CPU priority
            // (background mode alone left the thread priorities unchanged in testing).
            let (mode, class) = if on {
                (PROCESS_MODE_BACKGROUND_BEGIN, IDLE_PRIORITY_CLASS)
            } else {
                (PROCESS_MODE_BACKGROUND_END, NORMAL_PRIORITY_CLASS)
            };
            for m in [mode, class] {
                if let Err(e) = unsafe { SetPriorityClass(GetCurrentProcess(), m) } {
                    crate::logging::line(&format!("game mode: priority {:#x}: {e}", m.0));
                }
            }
            if !on {
                crate::ports::request_refresh();
            }
            self.sync_ring();
            crate::logging::line(if on {
                "game mode on: hidden, input hooks off, background priority"
            } else {
                "game mode off"
            });
        }
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
                hIcon: LoadIconW(Some(hinstance), windows::core::PCWSTR(1 as _))
                    .unwrap_or_default(),
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
            // Power changes arrive as WM_POWERBROADCAST and wake the power worker.
            for g in [&GUID_ACDC_POWER_SOURCE, &GUID_BATTERY_PERCENTAGE_REMAINING] {
                let _ = RegisterPowerSettingNotification(
                    HANDLE(hwnd.0),
                    g,
                    DEVICE_NOTIFY_WINDOW_HANDLE,
                );
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
                swipe: Default::default(),
                pressed: false,
                light: false,
                accent: None,
                kb: None,
                kb_focus: None,
                asks: Default::default(),
                gpu_retry: 0,
                ringing: false,
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
                    windows::core::w!(
                        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
                    ),
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
        use windows::Win32::System::Registry::{
            RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY,
        };
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

    // ------------------------------------------------------------ drop target

    #[windows::core::implement(IDropTarget)]
    struct Target {
        shared: Arc<Shared>,
    }

    fn query(data: &IDataObject, fmt: u16) -> FORMATETC {
        let _ = data;
        FORMATETC {
            cfFormat: fmt,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        }
    }

    const CF_UNICODETEXT: u16 = 13;
    const CF_HDROP: u16 = 15;

    fn accepts(data: &IDataObject) -> bool {
        unsafe {
            data.QueryGetData(&query(data, CF_HDROP)).is_ok()
                || data.QueryGetData(&query(data, CF_UNICODETEXT)).is_ok()
        }
    }

    fn extract(data: &IDataObject) -> DropJob {
        let mut job = DropJob {
            paths: vec![],
            text: None,
            op: None,
        };
        unsafe {
            if let Ok(mut stg) = data.GetData(&query(data, CF_HDROP)) {
                let h = windows::Win32::UI::Shell::HDROP(stg.u.hGlobal.0);
                let n = DragQueryFileW(h, 0xFFFF_FFFF, None);
                for i in 0..n {
                    let len = DragQueryFileW(h, i, None) as usize;
                    let mut buf = vec![0u16; len + 1];
                    let got = DragQueryFileW(h, i, Some(&mut buf)) as usize;
                    job.paths.push(String::from_utf16_lossy(&buf[..got]).into());
                }
                ReleaseStgMedium(&mut stg);
            }
            if job.paths.is_empty() {
                if let Ok(mut stg) = data.GetData(&query(data, CF_UNICODETEXT)) {
                    let h = stg.u.hGlobal;
                    let p = GlobalLock(h) as *const u16;
                    if !p.is_null() {
                        let mut len = 0;
                        while *p.add(len) != 0 && len < 4 * 1024 * 1024 {
                            len += 1;
                        }
                        job.text =
                            Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
                        let _ = GlobalUnlock(h);
                    }
                    ReleaseStgMedium(&mut stg);
                }
            }
        }
        job
    }

    #[allow(non_snake_case)]
    impl IDropTarget_Impl for Target_Impl {
        fn DragEnter(
            &self,
            data: windows::core::Ref<IDataObject>,
            _k: MODIFIERKEYS_FLAGS,
            _pt: &POINTL,
            effect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            let ok = data.as_ref().is_some_and(accepts);
            unsafe {
                if !effect.is_null() {
                    *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
                }
            }
            if ok {
                push_event(&self.shared, UiEvent::DragEnter);
            }
            Ok(())
        }
        fn DragOver(
            &self,
            _k: MODIFIERKEYS_FLAGS,
            _pt: &POINTL,
            effect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            unsafe {
                if !effect.is_null() && *effect != DROPEFFECT_NONE {
                    *effect = DROPEFFECT_COPY;
                }
            }
            Ok(())
        }
        fn DragLeave(&self) -> windows::core::Result<()> {
            push_event(&self.shared, UiEvent::DragLeave);
            Ok(())
        }
        fn Drop(
            &self,
            data: windows::core::Ref<IDataObject>,
            _k: MODIFIERKEYS_FLAGS,
            _pt: &POINTL,
            effect: *mut DROPEFFECT,
        ) -> windows::core::Result<()> {
            unsafe {
                if !effect.is_null() {
                    *effect = DROPEFFECT_COPY;
                }
            }
            push_event(&self.shared, UiEvent::DragLeave);
            if let Some(d) = data.as_ref() {
                let job = extract(d);
                if !job.paths.is_empty() || job.text.is_some() {
                    push_event(&self.shared, UiEvent::ShelfAdd(job));
                }
            }
            Ok(())
        }
    }

    fn register_drop_target(hwnd: HWND, shared: Arc<Shared>) {
        unsafe {
            let t: IDropTarget = Target { shared }.into();
            let _ = RegisterDragDrop(hwnd, &t);
        }
    }

    // --------------------------------------------------------------- drag out

    #[windows::core::implement(IDropSource)]
    struct Source;

    #[allow(non_snake_case)]
    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(
            &self,
            escape: windows::core::BOOL,
            keys: MODIFIERKEYS_FLAGS,
        ) -> windows::core::HRESULT {
            if escape.as_bool() {
                DRAGDROP_S_CANCEL
            } else if keys.0 & 1 == 0 {
                DRAGDROP_S_DROP
            } else {
                windows::core::HRESULT(0)
            }
        }
        fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// Start an OLE drag of one file and return the effect the target applied.
    fn shelf_drag(path: &std::path::Path) -> DROPEFFECT {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName};
        unsafe {
            let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                &HSTRING::from(path.as_os_str()),
                None,
            ) else {
                return DROPEFFECT_NONE;
            };
            let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else {
                return DROPEFFECT_NONE;
            };
            let src: IDropSource = Source.into();
            let mut effect = DROPEFFECT_NONE;
            let _ = DoDragDrop(&data, &src, DROPEFFECT_COPY | DROPEFFECT_MOVE, &mut effect);
            let _ = Interface::as_raw(&data);
            effect
        }
    }

    // ----------------------------------------------------------------- wndproc

    fn lo(l: LPARAM) -> i32 {
        (l.0 & 0xFFFF) as i16 as i32
    }
    fn hi(l: LPARAM) -> i32 {
        ((l.0 >> 16) & 0xFFFF) as i16 as i32
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_TICK => {
                if let Some(sh) = SHARED.get() {
                    sh.tick_pending.store(false, Ordering::SeqCst);
                }
                with_ui(|ui| ui.frame(hwnd));
                return LRESULT(0);
            }
            WM_UI => {
                with_ui(|ui| ui.handle_events(hwnd));
                return LRESULT(0);
            }
            WM_FG => {
                // No reassert_topmost here: the new foreground may be a game going fullscreen,
                // and a topmost window pushed over it can knock it out of exclusive mode.
                // T_FS reasserts once the decision is made.
                unsafe {
                    SetTimer(Some(hwnd), T_FS, 150, None);
                }
                return LRESULT(0);
            }
            WM_DRAG_ARM | WM_DRAG_END => {
                with_ui(|ui| {
                    ui.armed = msg == WM_DRAG_ARM;
                    if !ui.armed {
                        ui.model.drop_over = false;
                    }
                    ui.layout();
                    ui.kick();
                });
                return LRESULT(0);
            }
            WM_MOUSEMOVE => {
                let scrubbing = with_ui(|ui| {
                    ui.scrub.is_some() && {
                        ui.set_mouse_client(lparam);
                        ui.scrub_to();
                        true
                    }
                });
                if scrubbing == Some(true) {
                    return LRESULT(0);
                }
                // Past a 4 px threshold with the button held, a shelf tile becomes a real drag.
                let start = with_ui(|ui| {
                    ui.set_mouse_client(lparam);
                    let (id, (sx, sy)) = ui.drag?;
                    let (mx, my) = ui.mouse?;
                    if wparam.0 & 1 == 0 {
                        ui.drag = None;
                        return None;
                    }
                    if (mx - sx).abs() <= 4 && (my - sy).abs() <= 4 {
                        return None;
                    }
                    ui.drag = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    let it = ui.model.shelf.iter().find(|i| i.id == id)?;
                    Some((id, it.path.clone()))
                })
                .flatten();
                if let Some((id, path)) = start {
                    // DoDragDrop pumps messages: run it outside the UI borrow so frames keep flowing.
                    let effect = shelf_drag(&path);
                    with_ui(|ui| {
                        if effect != DROPEFFECT_NONE && ui.cfg.shelf.remove_after_drag {
                            crate::shelf::remove_keep_file(&mut ui.model.shelf, id);
                            ui.save_shelf();
                        }
                        ui.model.shelf.retain(|i| i.path.exists());
                        ui.after_model_change(hwnd);
                    });
                    return LRESULT(0);
                }
                with_ui(|ui| {
                    ui.set_mouse_client(lparam);
                    let inside = ui.pill_contains();
                    if !ui.tracking {
                        let mut tme = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        unsafe {
                            let _ = TrackMouseEvent(&mut tme);
                        }
                        ui.tracking = true;
                    }
                    if inside && !ui.inside {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), T_COLLAPSE);
                            if !ui.hover && ui.cfg.general.open_on != "click" {
                                let ms = ui.cfg.general.hover_delay_ms.min(2000);
                                if ms == 0 {
                                    ui.open_hover();
                                } else {
                                    SetTimer(Some(hwnd), T_DWELL, ms, None);
                                }
                            }
                        }
                    } else if !inside && ui.inside {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), T_DWELL);
                            if ui.hover {
                                SetTimer(Some(hwnd), T_COLLAPSE, ui.cfg.general.collapse_delay_ms.clamp(20, 5000), None);
                            }
                        }
                    }
                    ui.inside = inside;
                    let over = ui.hit_at().is_some();
                    ui.over_hit = over;
                    ui.layout();
                    ui.kick();
                });
                return LRESULT(0);
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = (wparam.0 >> 16) as i16 as i32;
                with_ui(|ui| {
                    if msg == WM_MOUSEHWHEEL {
                        ui.hwheel(delta);
                    } else {
                        ui.wheel(delta);
                    }
                    ui.after_model_change(hwnd);
                });
                return LRESULT(0);
            }
            WM_CLIPBOARDUPDATE => {
                with_ui(|ui| ui.on_clipboard(hwnd));
                return LRESULT(0);
            }
            WM_MOUSELEAVE => {
                with_ui(|ui| {
                    ui.tracking = false;
                    ui.inside = false;
                    ui.mouse = None;
                    unsafe {
                        let _ = KillTimer(Some(hwnd), T_DWELL);
                        if ui.hover {
                            SetTimer(Some(hwnd), T_COLLAPSE, ui.cfg.general.collapse_delay_ms.clamp(20, 5000), None);
                        }
                    }
                    ui.layout();
                    ui.kick();
                });
                return LRESULT(0);
            }
            WM_SETCURSOR => {
                if (lparam.0 & 0xFFFF) as u32 == HTCLIENT {
                    let hand = with_ui(|ui| ui.over_hit).unwrap_or(false);
                    unsafe {
                        if let Ok(c) = LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }) {
                            SetCursor(Some(c));
                        }
                    }
                    return LRESULT(1);
                }
            }
            WM_LBUTTONDOWN => {
                with_ui(|ui| {
                    ui.set_mouse_client(lparam);
                    ui.pressed = ui.hit_at().is_some();
                    ui.kick();
                    ui.drag = match ui.hit_at().map(|h| h.action.clone()) {
                        Some(Action::ShelfTile(id)) => ui.mouse.map(|p| (id, p)),
                        _ => None,
                    };
                    ui.scrub = ui
                        .hit_at()
                        .filter(|h| {
                            matches!(
                                h.action,
                                Action::Volume | Action::Seek | Action::AppVolume(_) | Action::Opacity
                            )
                        })
                        .map(|h| (h.action.clone(), h.rect.0, h.rect.2));
                    if ui.scrub.is_some() {
                        ui.scrub_to();
                    }
                    if ui.drag.is_some() || ui.scrub.is_some() {
                        // Capture so a fast flick off the pill still reaches us and starts the drag.
                        unsafe {
                            SetCapture(hwnd);
                        }
                    }
                });
                return LRESULT(0);
            }
            WM_CAPTURECHANGED => {
                // Capture taken away mid-drag (Alt+Tab, a dialog): the drag is over.
                with_ui(|ui| ui.scrub = None);
            }
            WM_LBUTTONUP => {
                with_ui(|ui| {
                    ui.set_mouse_client(lparam);
                    ui.pressed = false;
                    ui.kick();
                    // A press that never became a drag is a click.
                    ui.drag = None;
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if ui.scrub.is_some() {
                        // The level was applied while dragging; settle on the final one. A
                        // seek is sent now, once, to where the knob was let go.
                        ui.scrub_to();
                        if ui.scrubbing(&Action::Opacity) {
                            let v = ui.cfg.general.pill_opacity as f64;
                            ui.scrub = None;
                            ui.run_action(hwnd, Action::Set("general.pill_opacity", crate::config::Val::F(v)));
                            return;
                        }
                        if ui.scrubbing(&Action::Seek) {
                            if let Some(md) = &ui.model.media {
                                crate::media::control(crate::media::Cmd::Seek(md.pos_ms));
                            }
                        } else {
                            ui.model.speaker = crate::mic::speaker().or(ui.model.speaker);
                        }
                        ui.scrub = None;
                        ui.after_model_change(hwnd);
                        return;
                    }
                    // Click mode: the first click on the closed pill opens it and does nothing else.
                    if ui.cfg.general.open_on == "click"
                        && !ui.hover
                        && ui.pill_contains()
                        && ui.hit_at().is_none()
                    {
                        ui.open_hover();
                        return;
                    }
                    if let Some(a) = ui.hit_at().map(|h| h.action.clone()) {
                        ui.run_action(hwnd, a);
                    }
                });
                return LRESULT(0);
            }
            WM_RBUTTONUP => {
                with_ui(|ui| {
                    ui.set_mouse_client(lparam);
                    if ui.pill_contains() {
                        ui.model.timer_panel = !ui.model.timer_panel;
                        if ui.model.timer_panel {
                            ui.hover = true;
                            unsafe {
                                let _ = KillTimer(Some(hwnd), T_COLLAPSE);
                            }
                        }
                        ui.after_model_change(hwnd);
                    }
                });
                return LRESULT(0);
            }
            WM_TIMER => {
                let id = wparam.0;
                unsafe {
                    let _ = KillTimer(Some(hwnd), id);
                }
                with_ui(|ui| match id {
                    T_DWELL => {
                        if ui.inside {
                            ui.open_hover();
                        }
                    }
                    T_COLLAPSE => {
                        if !ui.inside && ui.kb.is_none() {
                            ui.hover = false;
                            ui.model.timer_panel = false;
                            ui.model.settings = None;
                            match ui.cfg.general.start_panel.as_str() {
                                "auto" => ui.model.selected = None,
                                "home" => ui.model.selected = Some(Panel::Home),
                                _ => {}
                            }
                            ui.layout();
                            ui.kick();
                        }
                    }
                    T_EXPIRE | T_TIMER => ui.after_model_change(hwnd),
                    T_TRIM => {
                        if !ui.shared.animating.load(Ordering::SeqCst) {
                            // Drops cold pages (decoder and COM leftovers); they fault back in on use.
                            unsafe {
                                let _ = SetProcessWorkingSetSize(
                                    GetCurrentProcess(),
                                    usize::MAX,
                                    usize::MAX,
                                );
                            }
                        }
                    }
                    T_GPU => ui.rebuild_gpu(hwnd),
                    T_SHOT => ui.tick_shots(hwnd),
                    T_FS => {
                        // Settled after a foreground change or move: stay above it,
                        // unless it is a game.
                        ui.follow_monitor();
                        ui.evaluate_fullscreen(hwnd);
                        if !ui.gaming {
                            reassert_topmost(hwnd);
                        }
                        ui.follow_desktop(hwnd);
                    }
                    _ => {}
                });
                return LRESULT(0);
            }
            0x0312 /* WM_HOTKEY */ => {
                with_ui(|ui| ui.toggle_keyboard(hwnd));
                return LRESULT(0);
            }
            WM_KEYDOWN => {
                with_ui(|ui| ui.key(hwnd, wparam.0 as u16));
                return LRESULT(0);
            }
            WM_KILLFOCUS => {
                // Clicked or tabbed elsewhere: keyboard mode ends without stealing focus back.
                with_ui(|ui| ui.end_keyboard(false));
            }
            WM_POWERBROADCAST => {
                // Power source / charge level changed, or the PC resumed.
                crate::power::poke();
                if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                    // The foreground may have changed fullscreen state while asleep.
                    post(WM_FG);
                }
                return LRESULT(1);
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED => {
                with_ui(|ui| {
                    ui.refresh_monitor();
                    ui.kick();
                });
                post(WM_FG);
                return LRESULT(0);
            }
            WM_SETTINGCHANGE => {
                with_ui(|ui| {
                    ui.anim.reduce = reduce_motion();
                    ui.light = theme_is_light(&ui.cfg.general.theme);
                    // The accent colour changing also arrives here (ImmersiveColorSet).
                    ui.accent = windows_accent();
                    ui.layout();
                    ui.kick();
                });
                crate::tray::Tray::new(hwnd).refresh();
            }
            x if x == WM_TRAY => {
                let code = (lparam.0 & 0xFFFF) as u32;
                // Left or right click: the same menu (there is no window to open).
                if code == WM_RBUTTONUP || code == WM_LBUTTONUP {
                    let (paused, auto) = with_ui(|ui| (ui.paused, ui.cfg.general.autostart))
                        .unwrap_or((false, false));
                    let terminal = crate::setup::is_set_up();
                    let agents = crate::agents::is_connected();
                    crate::tray::Tray::new(hwnd).popup(paused, auto, terminal, agents);
                }
                return LRESULT(0);
            }
            WM_COMMAND => {
                match (wparam.0 & 0xFFFF) as u32 {
                    12 => unsafe {
                        let _ = DestroyWindow(hwnd);
                    },
                    10 => {
                        with_ui(|ui| {
                            ui.paused = !ui.paused;
                            ui.layout();
                            ui.kick();
                        });
                    }
                    11 => {
                        let _ = std::process::Command::new("explorer")
                            .arg(
                                crate::config::config_path()
                                    .parent()
                                    .unwrap_or(std::path::Path::new(".")),
                            )
                            .spawn();
                    }
                    14 => crate::setup::tray_setup(),
                    16 => crate::agents::tray(),
                    15 => crate::update::check_now(),
                    13 => {
                        with_ui(|ui| {
                            let g = &mut ui.cfg.general;
                            g.autostart = !g.autostart;
                            crate::config::ensure_autostart(g.autostart);
                            let _ = crate::config::set("general.autostart", crate::config::Val::B(g.autostart));
                        });
                    }
                    _ => {}
                }
                return LRESULT(0);
            }
            x if x != 0 && x == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                crate::tray::Tray::new(hwnd).add();
                // Explorer restarted: the taskbar is new, re-check fullscreen and stay on top.
                post(WM_FG);
                return LRESULT(0);
            }
            WM_DESTROY => {
                if LIVE.load(Ordering::Relaxed) {
                    unsafe { PostQuitMessage(0) };
                }
                return LRESULT(0);
            }
            _ => {}
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}
