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
    use crate::ui_state::{Anim, Chip, Model, Panel, Scene};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::*;
    use windows::Win32::Graphics::Dwm::DwmFlush;
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::Memory::*;
    use windows::Win32::System::Ole::*;
    use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
    use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
    use windows::Win32::UI::Controls::WM_MOUSELEAVE;
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
    use windows::Win32::UI::Shell::{DragQueryFileW, IVirtualDesktopManager, ShellExecuteW, VirtualDesktopManager};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_TICK: u32 = 0x8002;
    const WM_UI: u32 = 0x8003;
    const WM_FG: u32 = 0x8004;
    const WM_DRAG_ARM: u32 = 0x8005;
    const WM_DRAG_END: u32 = 0x8006;

    const T_DWELL: usize = 1;
    const T_COLLAPSE: usize = 2;
    const T_EXPIRE: usize = 3;
    const T_FS: usize = 4;
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
    // Low-level mouse hook state (screen px zone around the top-centre).
    static LBTN: AtomicBool = AtomicBool::new(false);
    static ARMED: AtomicBool = AtomicBool::new(false);
    static ZONE_L: AtomicI32 = AtomicI32::new(0);
    static ZONE_R: AtomicI32 = AtomicI32::new(0);
    static ZONE_B: AtomicI32 = AtomicI32::new(0);
    static HWND_ADDR: AtomicIsize = AtomicIsize::new(0);

    fn hwnd_of(addr: isize) -> HWND {
        HWND(addr as *mut _)
    }

    fn post(msg: u32) {
        let a = HWND_ADDR.load(Ordering::Relaxed);
        if a != 0 {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(a)), msg, WPARAM(0), LPARAM(0));
            }
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
            self.paused || (self.fs_hidden && self.cfg.general.fullscreen_mode == "hide")
        }

        fn layout(&mut self) {
            let scene = self.model.scene(self.hover, self.sentinel());
            let size = self.model.size(scene);
            let glow = self.model.glow(scene);
            self.anim.set_scene(scene, size, glow);
            self.anim.hover.set_target(if self.inside && scene != Scene::Sentinel { 1.0 } else { 0.0 });
            self.anim.vis.set_target(if self.hidden() { 0.0 } else { 1.0 });
        }

        /// Cursor in the coordinates of the last drawn frame (logical px).
        fn mouse_logical(&self) -> Option<(f32, f32)> {
            let (sx, sy) = self.mouse?;
            Some(((sx - self.origin.0) as f32 / self.scale, (sy - self.origin.1) as f32 / self.scale))
        }

        fn set_mouse_client(&mut self, lparam: LPARAM) {
            self.mouse = Some((self.origin.0 + lo(lparam), self.origin.1 + hi(lparam)));
        }

        fn pill_contains(&self) -> bool {
            let Some((mx, my)) = self.mouse_logical() else { return false };
            let pw = self.anim.rect.w.pos as f32;
            let ph = (self.anim.rect.h.pos as f32).max(6.0);
            let ox = (self.crop_w - pw) / 2.0;
            mx >= ox && mx <= ox + pw && my >= 0.0 && my <= ph
        }

        fn crop(&self) -> Crop {
            self.rend.plan(self.anim.rect.w.pos as f32, self.anim.rect.h.pos as f32, self.armed)
        }

        fn origin_for(&self, crop: Crop) -> (i32, i32) {
            let mon_w = self.mon.2 - self.mon.0;
            (self.mon.0 + (mon_w - crop.w_px) / 2, self.mon.1)
        }

        fn refresh_monitor(&mut self) {
            unsafe {
                let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
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
                            if s.source == "shell" && s.delay_ms == 0 && s.event == hytte_proto::TaskEvent::Start {
                                s.delay_ms = self.cfg.shell.threshold_ms;
                            }
                            if s.event == hytte_proto::TaskEvent::NeedsInput {
                                self.model.peek(Panel::Tasks, now + Duration::from_secs(self.cfg.agent.peek_secs));
                                if self.cfg.agent.sound {
                                    unsafe {
                                        let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(MB_ICONASTERISK);
                                    }
                                }
                            }
                        }
                        self.model.apply_task(u, now)
                    }
                    UiEvent::Ports(p) => self.model.set_ports(p),
                    UiEvent::ShelfAdd(job) => {
                        let mut n = crate::shelf::add_paths(&self.cfg.shelf, &mut self.model.shelf, &job.paths);
                        if let Some(t) = &job.text {
                            n += crate::shelf::add_text(&self.cfg.shelf, &mut self.model.shelf, t);
                        }
                        if n > 0 {
                            self.save_shelf();
                            self.model.peek(Panel::Shelf, now + Duration::from_secs(5));
                        }
                    }
                    UiEvent::Thumb(id, bmp) => {
                        if let Some(b) = bmp {
                            self.rend.set_thumb(id, &b);
                        }
                    }
                    UiEvent::Media(m) => self.model.apply_media(m, now),
                    UiEvent::MediaArt(a) => self.model.set_art(a),
                    UiEvent::Privacy(c, m, app) => {
                        self.model.cam = c;
                        self.model.mic = m;
                        self.model.privacy_app = app;
                    }
                    UiEvent::DropDone(r) => {
                        self.model.chip = Some(Chip { summary: r.summary, open: r.open, copy: r.copy, since: now });
                    }
                    UiEvent::SetClipboard(s) => set_clipboard(hwnd, &s),
                    UiEvent::DragEnter => self.model.drop_over = true,
                    UiEvent::DragLeave => self.model.drop_over = false,
                }
            }
            self.after_model_change(hwnd);
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
                if vdm.IsWindowOnCurrentVirtualDesktop(hwnd).map(|b| b.as_bool()).unwrap_or(true) {
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
            match self.model.expire(now) {
                Some(next) => unsafe {
                    let ms = next.saturating_duration_since(now).as_millis() as u32 + 20;
                    SetTimer(Some(hwnd), T_EXPIRE, ms.max(20), None);
                },
                None => unsafe {
                    let _ = KillTimer(Some(hwnd), T_EXPIRE);
                },
            }
            self.request_thumbs();
            self.layout();
            self.kick();
        }

        fn frame(&mut self, hwnd: HWND) {
            let now = Instant::now();
            let dt = now.saturating_duration_since(self.last).as_secs_f32().min(0.05);
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
                    armed: self.armed,
                };
                self.hits = self.rend.draw(&fr, crop);
                let a = (self.anim.vis.pos.clamp(0.0, 1.0) * 255.0).round() as u8;
                self.rend.present(hwnd, crop, origin.0, origin.1, a);
            }
            let ambient = !self.anim.reduce && self.shown && self.model.ambient(self.anim.scene);
            let sh = &self.shared;
            sh.fast.store(moving, Ordering::Relaxed);
            sh.animating.store(moving || ambient, Ordering::SeqCst);
        }

        fn hit_at(&self) -> Option<&Hit> {
            let (mx, my) = self.mouse_logical()?;
            self.hits.iter().rev().find(|h| mx >= h.rect.0 && mx <= h.rect.2 && my >= h.rect.1 && my <= h.rect.3)
        }

        fn run_action(&mut self, hwnd: HWND, a: Action) {
            match a {
                Action::DismissTask(id) => self.model.dismiss_task(&id),
                Action::DismissChip => self.model.dismiss_chip(),
                Action::CopyText(s) => set_clipboard(hwnd, &s),
                Action::OpenPath(p) => {
                    let _ = std::process::Command::new("explorer").arg(p).spawn();
                }
                Action::MediaPrev => crate::media::control(crate::media::Cmd::Prev),
                Action::MediaToggle => crate::media::control(crate::media::Cmd::Toggle),
                Action::MediaNext => crate::media::control(crate::media::Cmd::Next),
                Action::FocusTerminal(pid) => {
                    if let Some(h) = crate::proc::terminal_window_for(pid) {
                        crate::proc::focus_window(h);
                    }
                }
                Action::SelectPanel(p) => {
                    self.model.selected = Some(p);
                    if p == Panel::Ports {
                        crate::ports::request_refresh();
                    }
                }
                Action::OpenPort(port) => unsafe {
                    let url: Vec<u16> = format!("http://localhost:{port}\0").encode_utf16().collect();
                    ShellExecuteW(None, windows::core::w!("open"), windows::core::PCWSTR(url.as_ptr()), None, None, SW_SHOWNORMAL);
                },
                Action::KillPort(port, pid) => {
                    if self.model.kill_armed.is_some_and(|(p, _)| p == port) {
                        self.model.kill_armed = None;
                        let msg = match hytte_proto::ports::kill(pid) {
                            Ok(()) => format!("Stopped :{port}"),
                            Err(e) => format!("Could not stop :{port} - {e}"),
                        };
                        self.model.chip = Some(Chip { summary: msg, open: None, copy: None, since: Instant::now() });
                        crate::ports::request_refresh();
                    } else {
                        self.model.kill_armed = Some((port, Instant::now() + crate::ui_state::KILL_CONFIRM));
                    }
                }
                Action::ShelfTile(id) => {
                    self.model.shelf_sel = if self.model.shelf_sel == Some(id) { None } else { Some(id) };
                }
                Action::RemoveShelf(id) => {
                    crate::shelf::remove(&mut self.model.shelf, id);
                    if self.model.shelf_sel == Some(id) {
                        self.model.shelf_sel = None;
                    }
                    self.save_shelf();
                }
                Action::ShelfOp(id) => {
                    if let Some(it) = self.model.shelf.iter().find(|i| i.id == id) {
                        let job = DropJob { paths: vec![it.path.clone()], text: None };
                        self.model.chip = Some(Chip { summary: "Working…".into(), open: None, copy: None, since: Instant::now() });
                        let _ = self.shared.drop_tx.send(job);
                    }
                }
            }
            self.after_model_change(hwnd);
        }

        fn evaluate_fullscreen(&mut self) {
            let hide = crate::fullscreen::evaluate(&self.cfg.general, self.mon) == crate::fullscreen::Suppress::Hide;
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

    pub fn run_windows(cfg: Config, task_rx: Receiver<TaskUpdate>, ui_rx: Receiver<UiEvent>, drop_tx: Sender<DropJob>) {
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
            let hinst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
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

            let hwnd = match CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
            ) {
                Ok(h) => h,
                Err(_) => {
                    eprintln!("hytte: CreateWindowExW failed");
                    return;
                }
            };
            HWND_ADDR.store(hwnd.0 as isize, Ordering::Relaxed);
            shared.hwnd.store(hwnd.0 as isize, Ordering::Relaxed);

            let rend = match Renderer::new() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("hytte: renderer init failed: {e}");
                    return;
                }
            };
            let ui_cfg_shelf = cfg.shelf.drop_action != "process";
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
                fs_hidden: false,
                paused: false,
                armed: false,
                shown: false,
                tracking: false,
                over_hit: false,
                drag: None,
                vdm: windows::Win32::System::Com::CoCreateInstance(&VirtualDesktopManager, None, windows::Win32::System::Com::CLSCTX_ALL).ok(),
                thumb_req: Default::default(),
            };
            ui.model.ignore = ui.cfg.shell.ignore.iter().map(|s| s.to_ascii_lowercase()).collect();
            if ui.cfg.shelf.persist {
                ui.model.shelf = crate::shelf::load();
            }
            ui.refresh_monitor();
            ui.layout();
            ui.anim.rect.snap();
            ui.anim.glow.snap();
            ui.frame(hwnd);
            UI.with(|c| *c.borrow_mut() = Some(ui));

            register_drop_target(hwnd, shared.clone(), ui_cfg_shelf);

            let _ = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
            let _ = SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT);
            // ponytail: permanent LL hook with an O(1) callback; install only on drag if it ever shows in profiles
            let mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_ll), Some(hinstance), 0).ok();

            let mut tray = crate::tray::Tray::new(hwnd);
            tray.add();
            post(WM_FG);

            // Bridges: worker threads -> UI thread via queue + PostMessage.
            let s2 = shared.clone();
            std::thread::spawn(move || {
                while let Ok(u) = task_rx.recv() {
                    push_event(&s2, UiEvent::Task(u));
                }
            });
            let s3 = shared.clone();
            std::thread::spawn(move || {
                while let Ok(u) = ui_rx.recv() {
                    push_event(&s3, u);
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
        if event == EVENT_OBJECT_LOCATIONCHANGE && (id_object != 0 || hwnd != unsafe { GetForegroundWindow() }) {
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
        to_shelf: bool,
    }

    fn query(data: &IDataObject, fmt: u16) -> FORMATETC {
        let _ = data;
        FORMATETC { cfFormat: fmt, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
    }

    const CF_UNICODETEXT: u16 = 13;
    const CF_HDROP: u16 = 15;

    fn accepts(data: &IDataObject) -> bool {
        unsafe {
            data.QueryGetData(&query(data, CF_HDROP)).is_ok() || data.QueryGetData(&query(data, CF_UNICODETEXT)).is_ok()
        }
    }

    fn extract(data: &IDataObject) -> DropJob {
        let mut job = DropJob { paths: vec![], text: None };
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
                        job.text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
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
        fn DragOver(&self, _k: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
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
                if self.to_shelf {
                    if !job.paths.is_empty() || job.text.is_some() {
                        push_event(&self.shared, UiEvent::ShelfAdd(job));
                    }
                } else if !job.paths.is_empty() || job.text.is_some() {
                    push_event(
                        &self.shared,
                        UiEvent::DropDone(crate::drop::DropResult { summary: "Working…".into(), open: None, copy: None }),
                    );
                    let _ = self.shared.drop_tx.send(job);
                }
            }
            Ok(())
        }
    }

    fn register_drop_target(hwnd: HWND, shared: Arc<Shared>, to_shelf: bool) {
        unsafe {
            let t: IDropTarget = Target { shared, to_shelf }.into();
            let _ = RegisterDragDrop(hwnd, &t);
        }
    }

    // --------------------------------------------------------------- drag out

    #[windows::core::implement(IDropSource)]
    struct Source;

    #[allow(non_snake_case)]
    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, escape: windows::core::BOOL, keys: MODIFIERKEYS_FLAGS) -> windows::core::HRESULT {
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
        use windows::Win32::UI::Shell::{IShellItem, SHCreateItemFromParsingName, BHID_DataObject};
        unsafe {
            let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path.as_os_str()), None) else {
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

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
                unsafe {
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
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
            WM_TIMER => {
                let id = wparam.0;
                unsafe {
                    let _ = KillTimer(Some(hwnd), id);
                }
                with_ui(|ui| match id {
                    T_DWELL => {
                        if ui.inside {
                            ui.hover = true;
                            ui.layout();
                            ui.kick();
                        }
                    }
                    T_COLLAPSE => {
                        if !ui.inside {
                            ui.hover = false;
                            ui.layout();
                            ui.kick();
                        }
                    }
                    T_EXPIRE => ui.after_model_change(hwnd),
                    T_FS => {
                        ui.evaluate_fullscreen();
                        ui.follow_desktop(hwnd);
                    }
                    _ => {}
                });
                return LRESULT(0);
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
                    ui.layout();
                    ui.kick();
                });
            }
            x if x == WM_TRAY => {
                let code = (lparam.0 & 0xFFFF) as u32;
                if code == WM_RBUTTONUP {
                    let (paused, auto) = with_ui(|ui| (ui.paused, ui.cfg.general.autostart)).unwrap_or((false, false));
                    crate::tray::Tray::new(hwnd).popup(paused, auto);
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
                            .arg(crate::config::config_path().parent().unwrap_or(std::path::Path::new(".")))
                            .spawn();
                    }
                    13 => {
                        with_ui(|ui| {
                            let g = &mut ui.cfg.general;
                            g.autostart = !g.autostart;
                            crate::config::ensure_autostart(g.autostart);
                            let _ = crate::config::save(&ui.cfg);
                        });
                    }
                    _ => {}
                }
                return LRESULT(0);
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0) };
                return LRESULT(0);
            }
            _ => {}
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}
