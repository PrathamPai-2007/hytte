//! Direct2D / DirectWrite renderer.
//!
//! A Direct2D device context draws into a composition swap chain shown through
//! DirectComposition (`gpu.rs`): no readback, no per-frame copy.
//!
//! All drawing is in logical px; one scale transform maps to device px. The
//! pill hangs from the top-centre of a fixed transparent canvas, and only the
//! pill (plus its soft glow) is visible: a window region trims the rest.

use crate::config::{Config, Val};
use crate::gpu::Gpu;
use crate::power::Mode;
use crate::ui_state::{
    Anim, Bubble, ChipAction, Field, Model, Panel, Scene, TaskView, Tone, LOCK_W, TAB_H,
};
use hytte_proto::TaskEvent;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};
use windows_numerics::{Matrix3x2, Vector2};

/// Logical canvas size (the window), pill hangs from the top centre.
mod cards;
mod compact;
mod home;
mod jot;
mod media;
mod nudge;
mod pill;
mod prims;
mod settings;
mod settings_table;
mod shelf;
mod tasks;
mod timer;
mod weather;

pub use settings_table::*;

pub const CANVAS_W: f32 = 460.0;
pub const CANVAS_H: f32 = 310.0;
const HOT_ZONE_H: f32 = 40.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    DismissTask(String),
    CopyText(String),
    OpenPath(PathBuf),
    DismissChip,
    /// One of a chip's extra buttons.
    Chip(crate::ui_state::ChipAction),
    MediaPrev,
    MediaToggle,
    MediaNext,
    /// The media timeline: a press seeks (and drags) to where the pointer is.
    Seek,
    /// Click on a task row: bring its terminal to the front.
    /// (owner pid, task id): focus the terminal window, then its tab when known.
    FocusTerminal(u32, String),
    SelectPanel(Panel),
    OpenPort(u16),
    /// First click arms ("Kill?"), second confirms.
    KillPort(u16, u32),
    /// (port, pid, task id): stop the server and run its terminal command again. First click
    /// arms ("Restart?"), second confirms, like Stop.
    RestartPort(u16, u32, String),
    ShelfTile(u64),
    /// Scroll the shelf row by this many tiles.
    ShelfScroll(i32),
    RemoveShelf(u64),
    /// Run a Drop Vault transform on a shelf item.
    ShelfOp(u64, crate::transforms::Conv),
    ToggleMic,
    ToggleSpeaker,
    /// Click on the volume bar: the level comes from where it was clicked.
    Volume,
    /// One app's bar in the per-app volume list (by exe name): drag, click or scroll.
    AppVolume(String),
    /// The pill-opacity slider in settings: drag or click; saved on release.
    Opacity,
    /// The glow-strength slider of the big settings window.
    Glow,
    /// A section in the settings window's list: scroll to it.
    Jump(usize),
    /// The settings window's search field: clear what was typed.
    ClearSearch,
    /// The pin in the tab strip: keep the pill open (or let it close again).
    Pin,
    /// "Custom" on a setting only `config.toml` can hold: open the file at it (the hint id).
    EditConfig(&'static str),
    /// The chevron at the end of the volume bar: open / close the per-app list.
    ToggleApps,
    /// Keep-awake tile: off -> an hour -> on -> off.
    CycleAwake,
    /// Timer tile: the running timer's pane, or the setup card.
    OpenTimer,
    /// Timer setup: minutes up (+1) or down (-1) one wheel notch.
    TimerNudge(i32),
    /// Set the minutes the timer panel will start with.
    TimerMinutes(u32),
    TimerStart(crate::timer::TimerKind),
    /// Start a break of this many minutes.
    TimerBreak(u32),
    TimerAdd(u32),
    TimerStop,
    TimerPause,
    TimerDismiss,
    /// Close the timer setup card.
    TimerClose,
    SetPowerMode(Mode),
    /// Run a failed command again in a new terminal window (task id).
    Rerun(String),
    /// Bring the media app (AppUserModelId) to the front.
    FocusMedia(String),
    /// Open the settings pane on this group, or close it (None).
    Settings(Option<u8>),
    /// Write one `section.key` to `config.toml`; it applies straight away.
    Set(&'static str, Val),
    /// Open `config.toml` for the settings the pane doesn't have.
    OpenConfig,
    /// A text field: click to type in it.
    Field(Field),
    /// The Notes field's 24 h / Keep switch.
    NoteKeep(bool),
    /// Tick or untick a to-do.
    ToggleJot(u64),
    /// Edit an existing note or to-do in the field.
    EditJot(u64),
    /// Keep a temporary note for good.
    KeepJot(u64),
    DeleteJot(u64),
    UndoJot,
    /// A suggestion in the Weather place chooser.
    PickPlace(usize),
    /// The place name in the Weather pane: choose another place.
    ChangePlace,
    /// Close the place chooser and keep the current place.
    CancelPlace,
    /// Fetch the forecast again after a failure.
    RetryWeather,
    /// Break reminder: remind me again in ten minutes.
    NudgeSnooze,
    /// Break reminder: done, or not now.
    NudgeSkip,
    /// The moon in the Media pane: open or close the sleep timer chips.
    SleepMenu,
    SleepSet(crate::sleep::Kind),
    SleepCancel,
    LyricsOpen,
    LyricsClose,
    LyricsRetry,
}

/// What the big settings window tells the renderer about the pointer and the scroll.
pub struct SettingsView<'a> {
    /// Pointer in client logical px.
    pub mouse: Option<(f32, f32)>,
    pub pressed: bool,
    pub light: bool,
    pub accent: [f32; 3],
    pub scroll: f32,
    /// The scroll bar was used or pointed at lately.
    pub scrolling: bool,
    /// Seconds since the last frame (drives the easing).
    pub dt: f32,
    /// 0 -> 1 over the first moments after the window opens.
    pub open: f32,
    /// A note shown over the bottom of the page, and how visible it is (0..1).
    pub toast: Option<(&'a str, f32)>,
    /// What was typed in the search field (rows and sections not matching it are hidden).
    pub query: &'a str,
}

/// What a settings-window frame laid out.
#[derive(Default)]
pub struct SettingsOut {
    /// Height of everything, for limiting the scroll.
    pub content: f32,
    /// Where each section starts (content px), for the section list's jumps.
    pub tops: Vec<f32>,
    /// Something is still easing: draw another frame soon.
    pub busy: bool,
}

#[derive(Debug, Clone)]
pub struct Hit {
    /// Canvas-logical rect: left, top, right, bottom.
    pub rect: (f32, f32, f32, f32),
    pub action: Action,
}

/// Device/logical size of the region actually drawn and presented this frame.
#[derive(Debug, Clone, Copy)]
pub struct Crop {
    pub w_px: i32,
    pub h_px: i32,
    pub w_log: f32,
    /// Device-pixel rect (left, top, right, bottom) holding the pill and its glow. The
    /// GPU path presents the whole canvas and shows only this part (a window region).
    pub clip: (i32, i32, i32, i32),
}

pub struct Frame<'a> {
    pub model: &'a Model,
    /// The settings in force (the settings pane shows them).
    pub cfg: &'a Config,
    pub anim: &'a Anim,
    pub now: Instant,
    pub dt: f32,
    pub mouse: Option<(f32, f32)>,
    /// The left button is held down (buttons under the pointer press in).
    pub pressed: bool,
    /// Light theme: a pale pill with dark text.
    pub light: bool,
    /// The Windows accent colour, in the shade that suits this theme (selection, primary
    /// buttons, the keyboard focus ring). Status colours don't follow it.
    pub accent: [f32; 3],
    /// Keyboard focus: the index of the clickable region that has it (Tab / Shift+Tab).
    pub focus: Option<usize>,
    /// A drag is in flight near the top edge: widen the (invisible) hit zone.
    pub armed: bool,
    /// The timer alarm is sounding.
    pub ringing: bool,
    /// The text caret is in its visible half of the blink.
    pub caret_on: bool,
}

struct Fmts {
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_r: IDWriteTextFormat,
    small_l: IDWriteTextFormat,
    mono: IDWriteTextFormat,
    big: IDWriteTextFormat,
    btn: IDWriteTextFormat,
    wrap: IDWriteTextFormat,
    center: IDWriteTextFormat,
    big_c: IDWriteTextFormat,
    /// Lyrics: centred, wraps to a second line.
    lyric: IDWriteTextFormat,
    small_c: IDWriteTextFormat,
    /// Segoe Fluent Icons glyphs, centred in their box (see `glyph`).
    icon: IDWriteTextFormat,
    /// The same, larger (tiles, placeholders).
    icon_l: IDWriteTextFormat,
    /// The same, smaller (tab strip).
    icon_s: IDWriteTextFormat,
    /// Big display numbers (timer, battery percent).
    hero: IDWriteTextFormat,
}

pub struct Renderer {
    factory: ID2D1Factory,
    factory1: ID2D1Factory1,
    /// What every draw call uses: the device context.
    rt: ID2D1RenderTarget,
    /// The swap chain and composition; None only for the offscreen snapshot renderer.
    gpu: Option<Gpu>,
    /// Window rect and region last applied, so they are only set on change.
    placed: Cell<(i32, i32, i32, i32)>,
    region: Cell<(i32, i32, i32, i32)>,
    /// The device was lost; the window layer calls `rebuild_gpu`.
    lost: Cell<bool>,
    br: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dashed: ID2D1StrokeStyle,
    f: Fmts,
    dw: IDWriteFactory,
    /// The label scrolling under the pointer: (its text's hash, animation time it started).
    marquee: Cell<Option<(u64, f32)>>,
    /// A marquee ran this frame: the frame loop keeps ticking (see `wants_frames`).
    scrolling: Cell<bool>,
    px: (i32, i32),
    scale: f32,
    // per-frame scratch
    ca: Cell<f32>,
    /// Settings window: the extra vertical offset hits are registered at (its scroll).
    oy: Cell<f32>,
    /// Settings window: animate (`ease`), the frame's time step, and whether a value is
    /// still on its way (another frame is due).
    fx: Cell<bool>,
    fx_dt: Cell<f32>,
    fx_busy: Cell<bool>,
    fx_vals: RefCell<HashMap<String, f32>>,
    /// The settings row the pointer is on this frame (key, row y), and how long the same one
    /// has been pointed at (for the hover note).
    tip_now: Cell<Option<(&'static str, f32)>>,
    tip: RefCell<(&'static str, f32)>,
    ox: Cell<f32>,
    t: Cell<f32>,
    mouse: Cell<Option<(f32, f32)>>,
    hits: RefCell<Vec<Hit>>,
    shown: RefCell<HashMap<String, f32>>,
    art: RefCell<Option<ID2D1Bitmap>>,
    thumbs: RefCell<HashMap<u64, ID2D1Bitmap>>,
    art_gen: Cell<u32>,
    /// Cached gradient brushes (see `body_brush` / `shimmer_brush`).
    /// One slot per `Grad`, rebuilt only when its key (colour, theme) changes.
    grads: RefCell<[Option<CachedGrad>; GRADS]>,
    /// UTF-16 scratch for `text`, reused across calls.
    wide: RefCell<Vec<u16>>,
    /// While the pill morphs between a compact scene and its expanded twin: how far along
    /// (0 = compact, 1 = expanded). Shared elements slide by it; see `hero`.
    morph: Cell<Option<f32>>,
    pressed: Cell<bool>,
    light: Cell<bool>,
    accent: Cell<[f32; 3]>,
    focus: Cell<Option<usize>>,
    /// True while the live scene (not the one fading out) is drawing: only it shows focus.
    focus_live: Cell<bool>,
    /// Right edge available to the compact task label this frame (see `exp_label_w`): both
    /// scenes of a morph must agree on the shared label's two end positions.
    hero_right: Cell<f32>,
    /// Last pill silhouette, keyed by its size: ambient frames (spinner, pulses) repeat it.
    geom: RefCell<Option<([u32; 2], ID2D1PathGeometry)>>,
}

/// Cached gradient brushes (see `Renderer::grad`).
#[derive(Clone, Copy)]
enum Grad {
    Body,
    Shimmer,
    /// The context colour pooling up from the bottom of the body.
    Tint,
    /// The rim: faint at the top edge, brighter round the bottom curve.
    Rim,
}
const GRADS: usize = 4;
/// A cached brush and the key it was built for.
type CachedGrad = ([f32; 4], ID2D1LinearGradientBrush);

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];
const BLUE: [f32; 3] = [0.35, 0.63, 1.0];
const GREEN: [f32; 3] = [0.20, 0.82, 0.48];
const RED: [f32; 3] = [1.0, 0.35, 0.37];
const PURPLE: [f32; 3] = [0.65, 0.54, 0.98];
const ORANGE: [f32; 3] = [1.0, 0.62, 0.04];
const CAM: [f32; 3] = [0.19, 0.82, 0.35];
const GRAY: [f32; 3] = [0.62, 0.62, 0.68];
const AMBER: [f32; 3] = [1.0, 0.72, 0.20];

fn color(c: [f32; 3], a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c[0],
        g: c[1],
        b: c[2],
        a: a.clamp(0.0, 1.0),
    }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn v(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn mat(s: f32, tx: f32, ty: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: s,
        M12: 0.0,
        M21: 0.0,
        M22: s,
        M31: tx * s,
        M32: ty * s,
    }
}
fn task_color(t: &TaskView) -> [f32; 3] {
    match t.event {
        TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => BLUE,
        TaskEvent::NeedsInput => AMBER,
        TaskEvent::Done => GREEN,
        TaskEvent::Failed => RED,
        TaskEvent::Lost => GRAY,
    }
}

pub fn fmt_dur(ms: u64) -> String {
    let s = ms / 1000;
    if s < 1 {
        format!("{ms}ms")
    } else if s < 60 {
        format!("{}.{}s", s, (ms % 1000) / 100)
    } else {
        format!("{}m {:02}s", s / 60, s % 60)
    }
}

fn union_rect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
}

fn contains_rect(outer: (i32, i32, i32, i32), inner: (i32, i32, i32, i32)) -> bool {
    outer.0 <= inner.0 && outer.1 <= inner.1 && outer.2 >= inner.2 && outer.3 >= inner.3
}

/// Fluent volume glyph for a level 0..1.
fn volume_glyph(v: f32) -> char {
    match v {
        v if v <= 0.0 => '\u{E992}',
        v if v < 0.34 => '\u{E993}',
        v if v < 0.67 => '\u{E994}',
        _ => '\u{E995}',
    }
}

/// Fluent glyph for a pane's tab.
fn panel_glyph(p: Panel) -> char {
    match p {
        Panel::Shelf => '\u{E7B8}',
        Panel::Media => '\u{EC4F}',
        Panel::Home => '\u{E80F}',
        Panel::Tasks => '\u{E756}',
        Panel::Timer => '\u{E916}',
        Panel::Power => '\u{E83F}',
        Panel::Notes => '\u{E70B}',
        Panel::Todo => '\u{E73A}',
        Panel::Weather => '\u{E753}',
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

/// Rough width of a button label in logical px (10.5 px semibold).
fn text_w(s: &str) -> f32 {
    s.chars().count() as f32 * 6.1
}

fn fmt_clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Renderer {
    /// Renderer presenting through DirectComposition into `hwnd`, which must have been
    /// created with `WS_EX_NOREDIRECTIONBITMAP`.
    pub fn new(hwnd: HWND) -> windows::core::Result<Self> {
        let factory1: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let g = Gpu::new(&factory1, hwnd, CANVAS_W as u32, CANVAS_H as u32)?;
        Self::build(factory1, g.dc.cast()?, Some(g))
    }

    fn build(
        factory1: ID2D1Factory1,
        rt: ID2D1RenderTarget,
        gpu: Option<Gpu>,
    ) -> windows::core::Result<Self> {
        unsafe {
            let factory: ID2D1Factory = factory1.cast()?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let br = rt.CreateSolidColorBrush(&color(WHITE, 1.0), None)?;
            let sp = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 1.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            let round = factory.CreateStrokeStyle(&sp, None)?;
            let dp = D2D1_STROKE_STYLE_PROPERTIES {
                dashStyle: D2D1_DASH_STYLE_CUSTOM,
                ..sp
            };
            let dashed = factory.CreateStrokeStyle(&dp, Some(&[2.0, 3.0]))?;

            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let f = Fmts::new(&dw)?;
            Ok(Self {
                factory,
                factory1,
                rt,
                gpu,
                placed: Cell::new((0, 0, 0, 0)),
                region: Cell::new((0, 0, 0, 0)),
                lost: Cell::new(false),
                br,
                round,
                dashed,
                f,
                dw,
                marquee: Cell::new(None),
                scrolling: Cell::new(false),
                px: (0, 0),
                scale: 1.0,
                ca: Cell::new(1.0),
                oy: Cell::new(0.0),
                fx: Cell::new(false),
                fx_dt: Cell::new(0.0),
                fx_busy: Cell::new(false),
                fx_vals: RefCell::new(HashMap::new()),
                tip_now: Cell::new(None),
                tip: RefCell::new(("", 0.0)),
                ox: Cell::new(0.0),
                t: Cell::new(0.0),
                mouse: Cell::new(None),
                hits: RefCell::new(vec![]),
                shown: RefCell::new(HashMap::new()),
                art: RefCell::new(None),
                thumbs: RefCell::new(HashMap::new()),
                art_gen: Cell::new(u32::MAX),
                grads: RefCell::new(Default::default()),
                wide: RefCell::new(Vec::new()),
                geom: RefCell::new(None),
                morph: Cell::new(None),
                pressed: Cell::new(false),
                light: Cell::new(false),
                accent: Cell::new(BLUE),
                focus: Cell::new(None),
                focus_live: Cell::new(false),
                hero_right: Cell::new(0.0),
            })
        }
    }

    /// A hovered label is scrolling: keep frames coming until the pointer leaves it.
    pub fn wants_frames(&self) -> bool {
        self.scrolling.get()
    }

    /// True once the GPU device was lost (driver reset, adapter removed).
    pub fn lost(&self) -> bool {
        self.lost.get()
    }

    /// Recreate the GPU pipeline after a device loss. Everything made by the old device
    /// (brushes, bitmaps) is dropped and rebuilt on demand.
    pub fn rebuild_gpu(&mut self, hwnd: HWND) -> windows::core::Result<()> {
        // The old DirectComposition target must go before a new one can attach to the window.
        self.gpu = None;
        let g = Gpu::new(&self.factory1, hwnd, CANVAS_W as u32, CANVAS_H as u32)?;
        let rt: ID2D1RenderTarget = g.dc.cast()?;
        unsafe {
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            self.br = rt.CreateSolidColorBrush(&color(WHITE, 1.0), None)?;
        }
        self.rt = rt;
        self.gpu = Some(g);
        *self.grads.borrow_mut() = Default::default();
        *self.art.borrow_mut() = None;
        self.art_gen.set(u32::MAX);
        self.thumbs.borrow_mut().clear();
        *self.geom.borrow_mut() = None;
        self.placed.set((0, 0, 0, 0));
        self.region.set((0, 0, 0, 0));
        self.lost.set(false);
        self.px = (0, 0);
        self.set_scale(self.scale);
        Ok(())
    }

    /// Size the swap chain for a new device scale.
    pub fn set_scale(&mut self, scale: f32) {
        let w = (CANVAS_W * scale).ceil() as i32;
        let h = (CANVAS_H * scale).ceil() as i32;
        if let Some(g) = &self.gpu {
            g.resize(w as u32, h as u32);
        }
        self.px = (w, h);
        self.scale = scale;
    }

    /// Bounding box of the pill + glow (or the whole hot zone while a drag is armed).
    /// `room` is extra width (each side, so the pill stays centred) for a bubble beside it.
    /// The box is also what eats clicks, so without a glow it hugs the pill: the thin line
    /// over a fullscreen video must not block the browser tabs around it.
    pub fn plan(&self, pw: f32, ph: f32, armed: bool, room: f32, glow: bool) -> Crop {
        let margin = if glow { 28.0 } else { 4.0 };
        let (mut w, mut h) = (pw + 2.0 * (margin + room), ph + margin);
        if armed {
            w = w.max(CANVAS_W);
            h = h.max(HOT_ZONE_H);
        }
        let w_px = ((w * self.scale).ceil() as i32).min(self.px.0);
        let h_px = ((h * self.scale).ceil() as i32).min(self.px.1);
        // The whole canvas is the surface; only the pill's box is visible and clickable.
        let x0 = (self.px.0 - w_px) / 2;
        Crop {
            w_px: self.px.0,
            h_px: self.px.1,
            w_log: self.px.0 as f32 / self.scale,
            clip: (x0, 0, x0 + w_px, h_px),
        }
    }

    /// Draw one frame into `hits`: the clickable regions it laid out. The caller's
    /// previous list is recycled as next frame's scratch, so steady state allocates nothing.
    pub fn draw(&mut self, fr: &Frame, crop: Crop, hits: &mut Vec<Hit>) {
        self.hits.borrow_mut().clear();
        self.mouse.set(fr.mouse);
        self.t.set(fr.anim.t as f32);
        self.pressed.set(fr.pressed);
        self.light.set(fr.light);
        self.accent.set(fr.accent);
        self.focus.set(fr.focus);
        self.focus_live.set(false);
        self.scrolling.set(false);
        self.sync_art(fr.model);
        let (pw, ph) = (fr.anim.rect.w.pos as f32, fr.anim.rect.h.pos as f32);
        let ox = (crop.w_log - pw) / 2.0;
        self.ox.set(ox);
        unsafe {
            // Offscreen (snapshots), the target is already bound.
            if !self.gpu.as_ref().is_none_or(|g| g.begin()) {
                hits.clear();
                return;
            }
            self.rt.BeginDraw();
            self.rt.Clear(Some(&color(WHITE, 0.0)));
            self.rt.SetTransform(&mat(self.scale, 0.0, 0.0));
            if fr.armed {
                // The deref picks the ID2D1Brush upcast; plain `&` does not satisfy the bound.
                #[allow(clippy::borrow_deref_ref)]
                self.rt.FillRectangle(
                    &rect(0.0, 0.0, crop.w_log, HOT_ZONE_H),
                    &*self.solid(color([0.0; 3], 0.008)),
                );
            }
            self.rt.SetTransform(&mat(self.scale, ox, 0.0));
            self.scene(fr, pw, ph);
            let ended = self.rt.EndDraw(None, None);
            if let Some(g) = &self.gpu {
                g.end();
                if ended.is_err() {
                    // D2DERR_RECREATE_TARGET and friends: the device has to be rebuilt.
                    self.lost.set(true);
                }
            }
        }
        // Prune progress smoothing for tasks that are gone.
        self.shown
            .borrow_mut()
            .retain(|k, _| fr.model.tasks.iter().any(|t| &t.id == k));
        std::mem::swap(hits, &mut *self.hits.borrow_mut());
    }

    /// Renderer for an ordinary window (the settings window), `w` x `h` device px.
    pub fn new_for_window(hwnd: HWND, w: u32, h: u32) -> windows::core::Result<Self> {
        let factory1: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let g = Gpu::new(&factory1, hwnd, w, h)?;
        let mut r = Self::build(factory1, g.dc.cast()?, Some(g))?;
        r.px = (w as i32, h as i32);
        Ok(r)
    }

    /// The settings window was resized or moved to a monitor with another scale.
    pub fn resize_window(&mut self, w: i32, h: i32, scale: f32) {
        if let Some(g) = &self.gpu {
            g.resize(w.max(1) as u32, h.max(1) as u32);
        }
        self.px = (w, h);
        self.scale = scale;
    }

    /// Draw the big settings window: a section list on the left (when there is room) and
    /// every setting in `settings_table` on the right, scrolled by `view.scroll`. Fills `hits`
    /// with where things are on screen.
    pub fn draw_settings_window(
        &mut self,
        cfg: &Config,
        view: &SettingsView<'_>,
        hits: &mut Vec<Hit>,
    ) -> SettingsOut {
        const COL: f32 = 560.0;
        const SIDE: f32 = 188.0;
        const ROW: f32 = 34.0;
        /// Right-hand column the reset button lives in, the same on every row.
        const RESET_GUT: f32 = 30.0;
        let (w, h) = (self.px.0 as f32 / self.scale, self.px.1 as f32 / self.scale);
        let side = if w >= 720.0 { SIDE } else { 0.0 };
        let cw = COL
            .min(w - side - if side > 0.0 { 48.0 } else { 24.0 })
            .max(280.0);
        let x0 = side + ((w - side - cw) / 2.0).max(0.0);
        self.hits.borrow_mut().clear();
        self.mouse.set(view.mouse);
        self.pressed.set(view.pressed);
        self.light.set(view.light);
        self.accent.set(view.accent);
        self.focus.set(None);
        self.focus_live.set(false);
        self.fx.set(true);
        self.fx_dt.set(view.dt);
        self.fx_busy.set(false);
        self.tip_now.set(None);
        let (fg, acc) = (self.fg(), view.accent);
        let bg = if view.light {
            [0.955, 0.955, 0.965]
        } else {
            [0.085, 0.09, 0.105]
        };
        let mut sections = settings_table(cfg);
        let q = view.query.trim().to_lowercase();
        if !q.is_empty() {
            for s in &mut sections {
                if !s.name.to_lowercase().contains(&q) {
                    s.rows.retain(|(r, _)| {
                        r.label().to_lowercase().contains(&q)
                            || help(r.key()).to_lowercase().contains(&q)
                    });
                }
            }
            sections.retain(|s| !s.rows.is_empty());
        }
        let names: Vec<&str> = sections.iter().map(|s| s.name).collect();
        // What each row holds by default, to offer a reset where it differs.
        let defaults: std::collections::HashMap<&str, (f32, Option<Action>)> =
            settings_table(&Config::default())
                .into_iter()
                .flat_map(|s| s.rows)
                .map(|(r, _)| (r.key(), (r.at(), r.set_action())))
                .collect();
        let mut tops: Vec<f32> = Vec::with_capacity(sections.len());
        let mut y = if side > 0.0 { 30.0 } else { 74.0 };
        unsafe {
            if !self.gpu.as_ref().is_none_or(|g| g.begin()) {
                hits.clear();
                return SettingsOut::default();
            }
            self.rt.BeginDraw();
            self.rt.Clear(Some(&color(bg, 1.0)));

            // ---- content, scrolled
            self.rt.SetTransform(&mat(self.scale, x0, 0.0));
            self.ox.set(x0);
            self.oy.set(-view.scroll);
            self.rt.SetTransform(&mat(self.scale, x0, -view.scroll));
            if side == 0.0 {
                self.ca.set(1.0);
                self.text(
                    "Settings",
                    &self.f.big,
                    22.0,
                    20.0,
                    cw - 200.0,
                    34.0,
                    self.cc(fg, 1.0),
                );
                self.open_button(cw - 22.0 - 132.0, 24.0, 132.0);
            }
            self.ca.set(1.0);
            self.search_field(view.query, 0.0, y, cw);
            y += 46.0;
            if sections.is_empty() {
                self.text(
                    "No setting matches",
                    &self.f.body,
                    0.0,
                    y + 10.0,
                    cw,
                    24.0,
                    self.cc(GRAY, 0.9),
                );
                y += 60.0;
            }
            for (i, sec) in sections.into_iter().enumerate() {
                // Each card eases in, one after the other, when the window opens.
                let p = (view.open * 1.7 - i as f32 * 0.12).clamp(0.0, 1.0);
                let p = 1.0 - (1.0 - p).powi(3);
                self.ca.set(p);
                let lift = (1.0 - p) * 14.0;
                tops.push(y);
                self.text(
                    &sec.name.to_uppercase(),
                    &self.f.small,
                    22.0,
                    y + lift,
                    cw - 44.0,
                    18.0,
                    self.cc(acc, 0.95),
                );
                let top = y + 24.0 + lift;
                let n = sec.rows.len() as f32;
                let ch = n * ROW + 8.0;
                self.fill_rr(0.0, top, cw, ch, 14.0, self.cc(fg, 0.045));
                self.stroke_rr(
                    0.5,
                    top + 0.5,
                    cw - 1.0,
                    ch - 1.0,
                    14.0,
                    self.cc(fg, 0.07),
                    1.0,
                    false,
                );
                let mut ry = top + 4.0;
                for (k, (row, _)) in sec.rows.into_iter().enumerate() {
                    if k > 0 {
                        self.line((22.0, ry), (cw - 22.0, ry), self.cc(fg, 0.05), 1.0);
                    }
                    let reset = defaults
                        .get(row.key())
                        .filter(|(at, _)| (at - row.at()).abs() > 0.004)
                        .and_then(|(_, a)| a.clone());
                    self.setting_row(row, reset, ry + 2.0, cw, RESET_GUT);
                    ry += ROW;
                }
                y += 24.0 + ch + 22.0;
            }
            self.ca.set(1.0);
            self.text(
                concat!(
                    "Hytte ",
                    env!("CARGO_PKG_VERSION"),
                    "  ·  lists, hotkey and folders live in config.toml"
                ),
                &self.f.small_c,
                0.0,
                y - 6.0,
                cw,
                18.0,
                self.cc(GRAY, 0.75),
            );
            y += 40.0;
            let viewport = h;
            let max = (y - viewport).max(0.0);
            // A slim scroll bar, seen while it is being used or pointed at.
            if max > 0.0 {
                self.rt.SetTransform(&mat(self.scale, 0.0, 0.0));
                self.ox.set(0.0);
                self.oy.set(0.0);
                let lit = self.ease("sb", view.scrolling as u8 as f32);
                if lit > 0.01 {
                    let th = (viewport * viewport / y).max(36.0);
                    let ty = 4.0 + (view.scroll / max) * (viewport - th - 8.0);
                    self.fill_rr(w - 9.0, ty, 4.0, th, 2.0, self.cc(fg, 0.28 * lit));
                }
            }

            // ---- section list, fixed
            if side > 0.0 {
                self.rt.SetTransform(&mat(self.scale, 0.0, 0.0));
                self.ox.set(0.0);
                self.oy.set(0.0);
                self.ca.set(1.0);
                self.fill_rr(0.0, 0.0, side, h, 0.0, self.cc(fg, 0.03));
                self.line((side, 0.0), (side, h), self.cc(fg, 0.06), 1.0);
                self.text(
                    "Settings",
                    &self.f.big,
                    24.0,
                    22.0,
                    side - 40.0,
                    30.0,
                    self.cc(fg, 1.0),
                );
                let active = tops
                    .iter()
                    .rposition(|t| *t <= view.scroll + 70.0)
                    .unwrap_or(0);
                let at = self.ease("nav", active as f32);
                let ny = 74.0;
                self.fill_rr(
                    12.0,
                    ny + at * 36.0,
                    side - 24.0,
                    32.0,
                    10.0,
                    self.cc(acc, 0.22),
                );
                for (i, name) in names.iter().enumerate() {
                    let top = ny + i as f32 * 36.0;
                    let hov = self.hit(12.0, top, side - 24.0, 32.0, Action::Jump(i));
                    let hh = self.ease(&format!("nh:{i}"), (hov && i != active) as u8 as f32);
                    if hh > 0.01 {
                        self.fill_rr(12.0, top, side - 24.0, 32.0, 10.0, self.cc(fg, 0.06 * hh));
                    }
                    let a = if i == active { 1.0 } else { 0.62 + 0.3 * hh };
                    self.text(
                        name,
                        &self.f.body,
                        26.0,
                        top,
                        side - 40.0,
                        32.0,
                        self.cc(fg, a),
                    );
                }
                self.open_button(12.0, h - 84.0, side - 24.0);
                self.text(
                    concat!("Hytte ", env!("CARGO_PKG_VERSION")),
                    &self.f.small_c,
                    0.0,
                    h - 42.0,
                    side,
                    18.0,
                    self.cc(GRAY, 0.75),
                );
            }
            // ---- the note for the setting under the pointer, after a short rest on it
            let pointed = self.tip_now.take().map(|(k, _)| k).unwrap_or("");
            {
                let mut t = self.tip.borrow_mut();
                if t.0 != pointed {
                    *t = (pointed, 0.0);
                } else if !pointed.is_empty() {
                    t.1 += view.dt;
                }
                if !pointed.is_empty() && t.1 < 0.6 {
                    self.fx_busy.set(true);
                }
                if let (false, true, Some((mx, my))) = (pointed.is_empty(), t.1 >= 0.6, view.mouse)
                {
                    let text = if pointed == "reset" {
                        "Put this setting back to its default."
                    } else {
                        help(pointed)
                    };
                    let a = ((t.1 - 0.6) * 7.0).min(1.0);
                    if a < 1.0 {
                        self.fx_busy.set(true);
                    }
                    self.rt.SetTransform(&mat(self.scale, 0.0, 0.0));
                    self.ox.set(0.0);
                    self.oy.set(0.0);
                    self.ca.set(a);
                    let tw = 290.0f32.min(w - 24.0);
                    let lines = (text.chars().count() as f32 * 6.4 / (tw - 24.0))
                        .ceil()
                        .max(1.0);
                    let th = lines * 17.0 + 18.0;
                    let tx = (mx + 12.0).min(w - tw - 12.0).max(12.0);
                    let ty = if my + 22.0 + th > h {
                        my - th - 10.0
                    } else {
                        my + 22.0
                    };
                    self.fill_rr(tx, ty, tw, th, 12.0, color(mix3(bg, fg, 0.16), a));
                    self.stroke_rr(
                        tx + 0.5,
                        ty + 0.5,
                        tw - 1.0,
                        th - 1.0,
                        12.0,
                        self.cc(fg, 0.12),
                        1.0,
                        false,
                    );
                    self.text(
                        text,
                        &self.f.wrap,
                        tx + 12.0,
                        ty + 9.0,
                        tw - 24.0,
                        th - 12.0,
                        self.cc(fg, 0.95),
                    );
                }
            }
            // ---- a note over the bottom of the page
            if let Some((text, a)) = view.toast {
                self.rt.SetTransform(&mat(self.scale, 0.0, 0.0));
                self.ox.set(0.0);
                self.oy.set(0.0);
                self.ca.set(a);
                let (tx, tw, th) = (side + 24.0, w - side - 48.0, 78.0);
                let ty = h - th - 24.0 + (1.0 - a) * 10.0;
                self.fill_rr(tx, ty, tw, th, 14.0, color(mix3(bg, fg, 0.12), a));
                self.fill_rr(tx, ty + 12.0, 3.0, th - 24.0, 1.5, self.cc(acc, 0.95));
                self.text(
                    text,
                    &self.f.wrap,
                    tx + 20.0,
                    ty + 10.0,
                    tw - 40.0,
                    th - 20.0,
                    self.cc(fg, 0.95),
                );
            }
            let ended = self.rt.EndDraw(None, None);
            if let Some(g) = &self.gpu {
                g.end();
                if ended.is_err() || !g.present(1.0) {
                    self.lost.set(true);
                }
            }
        }
        self.oy.set(0.0);
        self.fx.set(false);
        std::mem::swap(hits, &mut *self.hits.borrow_mut());
        SettingsOut {
            content: y,
            tops,
            busy: self.fx_busy.get(),
        }
    }

    /// The "Open config.toml" button of the settings window.
    fn open_button(&self, x: f32, y: f32, w: f32) {
        let fg = self.fg();
        let hov = self.hit(x, y, w, 30.0, Action::OpenConfig);
        let h = self.ease("h:open", hov as u8 as f32);
        self.fill_rr(x, y, w, 30.0, 15.0, self.cc(fg, 0.07 + 0.06 * h));
        self.text(
            "Open config.toml",
            &self.f.btn,
            x,
            y,
            w,
            30.0,
            self.cc(fg, 0.9),
        );
    }

    /// Show the frame with the window at screen position (x, y).
    /// `heading` is the clip the pill is animating towards (equal to `crop.clip` once settled).
    pub fn present(
        &self,
        hwnd: HWND,
        crop: Crop,
        x: i32,
        y: i32,
        alpha: u8,
        heading: (i32, i32, i32, i32),
    ) {
        let Some(g) = &self.gpu else { return };
        unsafe {
            let want = (x, y, crop.w_px, crop.h_px);
            if self.placed.get() != want {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    crop.w_px,
                    crop.h_px,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                self.placed.set(want);
            }
            // A region change is a window-manager round trip, so don't chase every frame
            // of a resize: grow straight to where the pill is heading, keep that while it
            // moves, and trim to the exact box once it settles.
            let cur = self.region.get();
            let want = if heading == crop.clip {
                crop.clip
            } else {
                let need = union_rect(crop.clip, heading);
                if cur.2 > cur.0 && contains_rect(cur, need) {
                    cur
                } else if cur.2 > cur.0 {
                    union_rect(cur, need)
                } else {
                    need
                }
            };
            if cur != want {
                // The system owns the region once it is set.
                SetWindowRgn(
                    hwnd,
                    Some(CreateRectRgn(want.0, want.1, want.2, want.3)),
                    false,
                );
                self.region.set(want);
            }
        }
        if !g.present(alpha as f32 / 255.0) {
            self.lost.set(true);
        }
    }

    // -------------------------------------------------------------- album art

    /// Shelf item ids that still need a thumbnail (and forget stale ones).
    pub fn missing_thumbs(&self, m: &Model) -> Vec<u64> {
        let mut t = self.thumbs.borrow_mut();
        t.retain(|id, _| m.shelf.iter().any(|i| i.id == *id));
        m.shelf_page()
            .1
            .iter()
            .filter(|i| !t.contains_key(&i.id))
            .map(|i| i.id)
            .collect()
    }

    pub fn set_thumb(&self, id: u64, a: &crate::ui_state::ArtBitmap) {
        if let Some(b) = self.upload(a) {
            self.thumbs.borrow_mut().insert(id, b);
        }
    }

    fn sync_art(&self, m: &Model) {
        if self.art_gen.get() == m.art_gen {
            return;
        }
        self.art_gen.set(m.art_gen);
        *self.art.borrow_mut() = m.art.as_ref().and_then(|a| self.upload(a));
    }

    fn upload(&self, a: &crate::ui_state::ArtBitmap) -> Option<ID2D1Bitmap> {
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let size = D2D_SIZE_U {
            width: a.w,
            height: a.h,
        };
        unsafe {
            self.rt
                .CreateBitmap(size, Some(a.bgra.as_ptr() as *const _), a.w * 4, &props)
                .ok()
        }
    }
}

impl Fmts {
    fn new(dw: &IDWriteFactory) -> windows::core::Result<Self> {
        unsafe {
            let mk = |face: PCWSTR,
                      size: f32,
                      weight: DWRITE_FONT_WEIGHT|
             -> windows::core::Result<IDWriteTextFormat> {
                let f = dw.CreateTextFormat(
                    face,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let trim = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                if let Ok(sign) = dw.CreateEllipsisTrimmingSign(&f) {
                    f.SetTrimming(&trim, &sign)?;
                }
                Ok(f)
            };
            let ui = w!("Segoe UI Variable Text");
            let title = mk(ui, 12.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let body = mk(ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            let small = mk(ui, 10.5, DWRITE_FONT_WEIGHT_NORMAL)?;
            let small_r = mk(ui, 10.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            small_r.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
            let small_l = mk(ui, 10.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let mono = mk(w!("Cascadia Mono"), 10.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            let big = mk(ui, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            let btn = mk(ui, 10.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            btn.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let center = mk(ui, 12.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            center.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let wrap = mk(ui, 12.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            wrap.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            wrap.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            let big_c = mk(ui, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            big_c.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let lyric = mk(ui, 14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
            lyric.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            lyric.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            let small_c = mk(ui, 10.5, DWRITE_FONT_WEIGHT_NORMAL)?;
            small_c.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            // Windows 11 ships Segoe Fluent Icons; the glyphs used here are also in its
            // predecessor, Segoe MDL2 Assets, which the font fallback covers on older builds.
            let icon = mk(w!("Segoe Fluent Icons"), 16.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let icon_l = mk(w!("Segoe Fluent Icons"), 20.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            icon_l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let icon_s = mk(w!("Segoe Fluent Icons"), 12.5, DWRITE_FONT_WEIGHT_NORMAL)?;
            icon_s.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let hero = mk(
                w!("Segoe UI Variable Display"),
                27.0,
                DWRITE_FONT_WEIGHT_SEMI_BOLD,
            )?;
            Ok(Self {
                title,
                body,
                small,
                small_r,
                small_l,
                mono,
                big,
                btn,
                wrap,
                center,
                big_c,
                lyric,
                small_c,
                icon,
                icon_l,
                icon_s,
                hero,
            })
        }
    }
}

/// Offscreen pictures of the expanded scenes, for eyeballing UI changes without running the
/// pill: `HYTTE_SNAP_DIR=<dir> cargo test snapshots -- --ignored` writes one BMP per scene,
/// composited over a dark wallpaper grey.
#[cfg(test)]
mod snap;
