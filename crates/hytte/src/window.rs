//! Windowing & interaction engine.
//!
//! One layered, NOACTIVATE, topmost window hanging from the top-centre of the
//! primary monitor. The pill is animated *inside* a fixed transparent canvas
//! (see `render.rs`), so there is no per-frame window resize and transparent
//! pixels stay click-through. UI state lives in a thread-local `Ui`; other
//! threads only push `UiEvent`s and post a wake-up message.

use crate::config::Config;
use crate::drop::DropJob;
use crate::tasks::TaskUpdate;
use crate::ui_state::UiEvent;
use crossbeam_channel::{Receiver, Sender};

pub const WM_TRAY: u32 = 0x8001;

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
    use windows::Win32::System::Power::RegisterPowerSettingNotification;
    use windows::Win32::System::SystemServices::{
        GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    use windows::Win32::System::Ole::*;
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
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
    /// Idle this long (no frames) before the working set is trimmed.
    const TRIM_MS: u32 = 30_000;
    const DWELL_MS: u32 = 120;
    const COLLAPSE_MS: u32 = 300;

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
    static LIVE: AtomicBool = AtomicBool::new(false);
    /// Registered `TaskbarCreated` message: Explorer restarted and our tray icon is gone.
    static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

    fn hwnd_of(addr: isize) -> HWND {
        HWND(addr as *mut _)
    }

    fn post(msg: u32) {
        post_w(msg, 0);
    }

    fn post_w(msg: u32, wparam: usize) {
        let a = HWND_ADDR.load(Ordering::Relaxed);
        if a != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(a)), msg, WPARAM(wparam), LPARAM(0));
            }
        }
    }

    /// `WM_FG` wparam: the foreground window only moved/resized (no z-order change).
    const FG_MOVED: usize = 1;

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
        /// Keyboard mode (opened by the hotkey): the pill is held open, has focus, and the
        /// window that had focus before is remembered to give it back.
        kb: Option<HWND>,
        /// Which clickable region has keyboard focus.
        kb_focus: Option<usize>,
        /// Accumulated wheel delta (a notch is 120; precision wheels send fractions).
        wheel: i32,
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
            self.fs_hidden && self.cfg.general.fullscreen_mode != "hide"
        }

        fn hidden(&self) -> bool {
            // A drag near the top edge or a fresh drop must stay reachable even over fullscreen.
            self.paused
                || (self.fs_hidden
                    && self.cfg.general.fullscreen_mode == "hide"
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
            )
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
                if GetMonitorInfoW(hmon, &mut mi).as_bool() {
                    let r = mi.rcMonitor;
                    self.mon = (r.left, r.top, r.right, r.bottom);
                }
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                self.scale = dx as f32 / 96.0;
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
                            if s.source == "shell"
                                && s.delay_ms == 0
                                && s.event == hytte_proto::TaskEvent::Start
                            {
                                s.delay_ms = self.cfg.shell.threshold_ms;
                            }
                            if s.event == hytte_proto::TaskEvent::NeedsInput {
                                self.model.peek(
                                    Panel::Tasks,
                                    now + Duration::from_secs(self.cfg.agent.peek_secs),
                                );
                                if self.cfg.agent.sound {
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
                    UiEvent::Ports(p) => self.model.set_ports(p),
                    UiEvent::TaskGone(id) => self.model.remove_task(&id),
                    UiEvent::Calendar(ev) => self.model.set_calendar(
                        ev,
                        crate::timer::now_ms() as i64,
                        self.cfg.calendar.lead_min as i64 * 60_000,
                        now,
                    ),
                    UiEvent::DownloadDone { name, path } => {
                        if self.model.chip.is_none() {
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
                        let n =
                            crate::shelf::insert(&self.cfg.shelf, &mut self.model.shelf, staged);
                        if n > 0 {
                            self.save_shelf();
                            self.model.peek(Panel::Shelf, now + Duration::from_secs(5));
                            self.model.force_until = Some(now + Duration::from_secs(5));
                            self.anim.squash();
                        }
                    }
                    UiEvent::Thumb(id, bmp) => {
                        if let Some(b) = bmp {
                            self.rend.set_thumb(id, &b);
                        }
                    }
                    UiEvent::Media(m) => self.model.apply_media(m, now),
                    UiEvent::MediaArt(a) => self.model.set_art(a),
                    UiEvent::MicMute(m) => self.model.mic_muted = m,
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
                }
            }
            self.after_model_change(hwnd);
        }

        /// A live edit of `config.toml`. Most settings are read where they're used, so only
        /// state derived from the config needs refreshing. Lowering `[shelf] max_items` keeps
        /// the tiles already there and only limits new ones.
        fn apply_config(&mut self, new: Config) {
            let hotkey_changed = new.general.hotkey != self.cfg.general.hotkey;
            if new.general.autostart != self.cfg.general.autostart {
                crate::config::ensure_autostart(new.general.autostart);
            }
            if new.ports != self.cfg.ports {
                crate::ports::reconfigure(new.ports.clone());
            }
            if new.hog != self.cfg.hog {
                crate::ports::reconfigure_hog(new.hog.clone());
            }
            self.model.ignore = ignore_list(&new);
            self.light = theme_is_light(&new.general.theme);
            self.model.adaptive_glow = new.general.adaptive_glow;
            self.model.rounds = new.timer.rounds;
            let monitor_changed = new.general.monitor != self.cfg.general.monitor;
            let general = new.general != self.cfg.general;
            self.cfg = new;
            if hotkey_changed {
                self.register_hotkey(HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _));
            }
            if monitor_changed {
                self.refresh_monitor();
            }
            if general {
                self.evaluate_fullscreen();
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

        /// (Re)register the global hotkey from the config.
        fn register_hotkey(&self, hwnd: HWND) {
            use windows::Win32::UI::Input::KeyboardAndMouse::{
                RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
            };
            unsafe {
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
                let spec = &self.cfg.general.hotkey;
                if spec.is_empty() {
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
                            crate::logging::line(&format!("hotkey {spec}: already taken by another app"));
                        }
                    }
                    None => crate::logging::line(&format!("hotkey {spec}: not understood")),
                }
            }
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
                self.model.force_until = Some(now + Duration::from_secs(8));
                if t.sound {
                    crate::timer::play_chime();
                }
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
            match self.model.expire(now) {
                Some(next) => unsafe {
                    let ms = next.saturating_duration_since(now).as_millis() as u32 + 20;
                    SetTimer(Some(hwnd), T_EXPIRE, ms.max(20), None);
                },
                None => unsafe {
                    let _ = KillTimer(Some(hwnd), T_EXPIRE);
                },
            }
            // A calendar heads-up that has come due becomes a card (never over another one).
            if self.model.chip.is_none() {
                if let Some(ev) = self.model.calendar_due(now) {
                    let text = crate::calendar::summary(&ev.subject, ev.start_ms, crate::timer::now_ms() as i64);
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
            let moving = self.anim.step(dt as f64);

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
                    anim: &self.anim,
                    now,
                    dt,
                    mouse: self.mouse_logical(),
                    acrylic: self.cfg.general.acrylic,
                    pressed: self.pressed,
                    light: self.light,
                    focus: self.kb.and(self.kb_focus),
                    armed: self.armed,
                };
                self.rend.draw(&fr, crop, &mut self.hits);
                let drawn = now.elapsed().as_micros() as u32;
                let a = (self.anim.vis.pos.clamp(0.0, 1.0) * 255.0).round() as u8;
                let heading = self
                    .rend
                    .plan(
                        self.anim.rect.w.target as f32,
                        self.anim.rect.h.target as f32,
                        self.armed,
                        self.anim.bubble_room(),
                    )
                    .clip;
                self.rend.present(hwnd, crop, origin.0, origin.1, a, heading);
                if self.rend.lost() {
                    // Driver reset or adapter removed: rebuild the GPU pipeline and redraw.
                    crate::logging::line("gpu renderer: device lost, rebuilding");
                    if let Err(e) = self.rend.rebuild_gpu(hwnd) {
                        crate::logging::line(&format!("gpu renderer: rebuild failed: {e}"));
                    }
                    self.thumb_req.clear();
                    self.kick();
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
            let ambient =
                !self.anim.reduce && self.shown && self.model.ambient(self.anim.scene, now);
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
                Action::DismissChip => self.model.dismiss_chip(),
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
                        self.model.chip_armed = Some(Instant::now() + crate::ui_state::KILL_CONFIRM);
                    }
                }
                ChipAction::IgnoreExe(exe) => {
                    self.model.hog_ignore.insert(exe.to_ascii_lowercase());
                    self.model.dismiss_chip();
                }
            }
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
            self.model.selected = Some(p);
            match p {
                Panel::Ports => crate::ports::request_refresh(),
                Panel::Home => self.model.power = crate::power::read().or(self.model.power),
                _ => {}
            }
        }

        fn toggle_mic(&mut self) {
            if let Some(m) = crate::mic::toggle() {
                self.model.mic_muted = m;
            }
        }

        /// Wheel over the pill flips panes; one step per notch.
        fn wheel(&mut self, delta: i32) {
            self.wheel += delta;
            let steps = self.wheel / 120;
            if steps == 0 {
                return;
            }
            self.wheel -= steps * 120;
            if self.model.timer_panel && self.model.timer.is_none() {
                self.model.timer_min = crate::timer::nudge(self.model.timer_min, steps);
                return;
            }
            // Wheel down = next pane. Collapsed pills peek open so the change is visible.
            let p = self.model.step_panel(-steps.signum());
            self.select_panel(p);
            // `step_panel` already moved the selection, so give the slide its direction here.
            self.anim.nav = -steps.signum() as f32;
            if !self.hover {
                self.model.peek(p, Instant::now() + Duration::from_secs(3));
            }
        }

        fn evaluate_fullscreen(&mut self) {
            let hide = crate::fullscreen::evaluate(&self.cfg.general, self.mon)
                == crate::fullscreen::Suppress::Hide;
            if hide != self.fs_hidden {
                self.fs_hidden = hide;
                if !hide {
                    self.hover = false;
                }
                self.layout();
                self.kick();
            }
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
                lpszClassName: cls,
                ..Default::default()
            };
            RegisterClassW(&wc);

            let make = |ex: WINDOW_EX_STYLE| {
                CreateWindowExW(
                    ex | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
                )
            };
            // The window's style decides how it can be drawn, so pick the renderer first:
            // GPU needs a window without a redirection bitmap, classic needs a layered one.
            let mut made = None;
            if cfg.general.renderer != "classic" {
                if let Ok(h) = make(WS_EX_NOREDIRECTIONBITMAP) {
                    match Renderer::new_gpu(h) {
                        Ok(r) => made = Some((h, r)),
                        Err(e) => {
                            crate::logging::line(&format!("gpu renderer unavailable: {e}"));
                            let _ = DestroyWindow(h);
                        }
                    }
                }
            }
            let (hwnd, rend) = match made {
                Some(m) => m,
                None => {
                    let Ok(h) = make(WS_EX_LAYERED) else {
                        eprintln!("hytte: CreateWindowExW failed");
                        return;
                    };
                    match Renderer::new() {
                        Ok(r) => (h, r),
                        Err(e) => {
                            eprintln!("hytte: renderer init failed: {e}");
                            return;
                        }
                    }
                }
            };
            LIVE.store(true, Ordering::Relaxed);
            HWND_ADDR.store(hwnd.0 as isize, Ordering::Relaxed);
            shared.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);
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
                pressed: false,
                light: false,
                kb: None,
                kb_focus: None,
            };
            ui.model.ignore = ignore_list(&ui.cfg);
            ui.light = theme_is_light(&ui.cfg.general.theme);
            ui.model.adaptive_glow = ui.cfg.general.adaptive_glow;
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

            let _ = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            let _ = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(win_event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            // ponytail: permanent LL hook with an O(1) callback; install only on drag if it ever shows in profiles
            let mouse_hook =
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_ll), Some(hinstance), 0).ok();

            let mut tray = crate::tray::Tray::new(hwnd);
            tray.add();
            with_ui(|ui| ui.register_hotkey(hwnd));
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
            if let Some(h) = mouse_hook {
                let _ = UnhookWindowsHookEx(h);
            }
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
    fn theme_is_light(theme: &str) -> bool {
        match theme {
            "light" => true,
            "auto" => unsafe {
                use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
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
        if event == EVENT_OBJECT_LOCATIONCHANGE {
            if id_object != 0 || hwnd != unsafe { GetForegroundWindow() } {
                return;
            }
            // Dragging a window fires this per mouse move: only re-arm the debounce.
            post_w(WM_FG, FG_MOVED);
            return;
        }
        post(WM_FG);
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
                if wparam.0 != FG_MOVED {
                    reassert_topmost(hwnd);
                }
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
                            if !ui.hover {
                                SetTimer(Some(hwnd), T_DWELL, DWELL_MS, None);
                            }
                        }
                    } else if !inside && ui.inside {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), T_DWELL);
                            if ui.hover {
                                SetTimer(Some(hwnd), T_COLLAPSE, COLLAPSE_MS, None);
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
            WM_MOUSEWHEEL => {
                let delta = (wparam.0 >> 16) as i16 as i32;
                with_ui(|ui| {
                    ui.wheel(delta);
                    ui.after_model_change(hwnd);
                });
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
                            SetTimer(Some(hwnd), T_COLLAPSE, COLLAPSE_MS, None);
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
                    if ui.drag.is_some() {
                        // Capture so a fast flick off the pill still reaches us and starts the drag.
                        unsafe {
                            SetCapture(hwnd);
                        }
                    }
                });
                return LRESULT(0);
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
                            ui.hover = true;
                            // Opening the pill: make sure the port list and charge rate are not stale.
                            crate::ports::request_refresh();
                            if ui.model.panel() == Panel::Home && ui.model.power.is_some() {
                                ui.model.power = crate::power::read().or(ui.model.power);
                            }
                            ui.layout();
                            ui.kick();
                        }
                    }
                    T_COLLAPSE => {
                        if !ui.inside && ui.kb.is_none() {
                            ui.hover = false;
                            ui.model.timer_panel = false;
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
                    T_FS => {
                        // Settled after a foreground change or move: stay above it.
                        reassert_topmost(hwnd);
                        ui.follow_monitor();
                        ui.evaluate_fullscreen();
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
                    ui.layout();
                    ui.kick();
                });
            }
            x if x == WM_TRAY => {
                let code = (lparam.0 & 0xFFFF) as u32;
                if code == WM_RBUTTONUP {
                    let (paused, auto) = with_ui(|ui| (ui.paused, ui.cfg.general.autostart))
                        .unwrap_or((false, false));
                    let terminal = crate::setup::is_set_up();
                    crate::tray::Tray::new(hwnd).popup(paused, auto, terminal);
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
                    13 => {
                        with_ui(|ui| {
                            let g = &mut ui.cfg.general;
                            g.autostart = !g.autostart;
                            crate::config::ensure_autostart(g.autostart);
                            let _ = crate::config::set_autostart(g.autostart);
                        });
                    }
                    _ => {}
                }
                return LRESULT(0);
            }
            x if x != 0 && x == TASKBAR_CREATED.load(Ordering::Relaxed) => {
                crate::tray::Tray::new(hwnd).add();
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
