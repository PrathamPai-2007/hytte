//! Direct2D / DirectWrite renderer.
//!
//! Two ways to put pixels on screen, chosen once at startup (`[general] renderer`):
//!
//! * **gpu** (default): a Direct2D device context draws into a composition swap
//!   chain shown through DirectComposition (`gpu.rs`). No readback, no per-frame copy.
//! * **classic**: draws onto a 32-bpp premultiplied DIB through an
//!   `ID2D1DCRenderTarget` and pushes it with `UpdateLayeredWindow`. Per-pixel alpha
//!   makes transparent pixels click-through. Used when the GPU path can't start.
//!
//! All drawing is in logical px; one scale transform maps to device px. The
//! pill hangs from the top-centre of a fixed transparent canvas, and only the
//! pill (plus its soft glow) has non-zero alpha.

use crate::gpu::Gpu;
use crate::power::Mode;
use crate::ui_state::{Anim, Bubble, ChipAction, Model, Panel, Scene, TaskView, Tone, LOCK_W};
use hytte_proto::TaskEvent;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, UpdateLayeredWindow, SWP_NOACTIVATE, SWP_NOZORDER, ULW_ALPHA,
};
const AC_SRC_ALPHA: u32 = 1;
use windows_numerics::{Matrix3x2, Vector2};

/// Logical canvas size (the window), pill hangs from the top centre.
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
    /// Click on a task row: bring its terminal to the front.
    /// (owner pid, task id): focus the terminal window, then its tab when known.
    FocusTerminal(u32, String),
    SelectPanel(Panel),
    OpenPort(u16),
    /// First click arms ("Kill?"), second confirms.
    KillPort(u16, u32),
    ShelfTile(u64),
    RemoveShelf(u64),
    /// Run a Drop Vault transform on a shelf item.
    ShelfOp(u64, crate::transforms::Conv),
    ToggleMic,
    ToggleSpeaker,
    /// Click on the volume bar: the level comes from where it was clicked.
    Volume,
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
    SetPowerMode(Mode),
    /// Bring the media app (AppUserModelId) to the front.
    FocusMedia(String),
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
    pub anim: &'a Anim,
    pub now: Instant,
    pub dt: f32,
    pub mouse: Option<(f32, f32)>,
    pub acrylic: bool,
    /// The left button is held down (buttons under the pointer press in).
    pub pressed: bool,
    /// Light theme: a pale pill with dark text.
    pub light: bool,
    /// Keyboard focus: the index of the clickable region that has it (Tab / Shift+Tab).
    pub focus: Option<usize>,
    /// A drag is in flight near the top edge: widen the (invisible) hit zone.
    pub armed: bool,
    /// The timer alarm is sounding.
    pub ringing: bool,
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
    small_c: IDWriteTextFormat,
    /// Segoe Fluent Icons glyphs, centred in their box (see `glyph`).
    icon: IDWriteTextFormat,
    /// The same, larger (tiles, placeholders).
    icon_l: IDWriteTextFormat,
    /// Big display numbers (timer, battery percent).
    hero: IDWriteTextFormat,
}

pub struct Renderer {
    factory: ID2D1Factory,
    factory1: ID2D1Factory1,
    /// What every draw call uses: the DC target (classic) or the device context (gpu).
    rt: ID2D1RenderTarget,
    /// Classic only: the target that is bound to the DIB each frame.
    dc_rt: Option<ID2D1DCRenderTarget>,
    gpu: Option<Gpu>,
    /// GPU only: window rect and region last applied, so they are only set on change.
    placed: Cell<(i32, i32, i32, i32)>,
    region: Cell<(i32, i32, i32, i32)>,
    /// GPU only: the device was lost; the window layer calls `rebuild_gpu`.
    lost: Cell<bool>,
    br: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dashed: ID2D1StrokeStyle,
    f: Fmts,
    mem: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    px: (i32, i32),
    scale: f32,
    // per-frame scratch
    ca: Cell<f32>,
    ox: Cell<f32>,
    t: Cell<f32>,
    mouse: Cell<Option<(f32, f32)>>,
    hits: RefCell<Vec<Hit>>,
    shown: RefCell<HashMap<String, f32>>,
    art: RefCell<Option<ID2D1Bitmap>>,
    thumbs: RefCell<HashMap<u64, ID2D1Bitmap>>,
    art_gen: Cell<u32>,
    /// Cached gradient brushes (see `body_brush` / `shimmer_brush`).
    body: RefCell<Option<(u8, ID2D1LinearGradientBrush)>>,
    shimmer: RefCell<Option<([f32; 3], ID2D1LinearGradientBrush)>>,
    /// UTF-16 scratch for `text`, reused across calls.
    wide: RefCell<Vec<u16>>,
    /// While the pill morphs between a compact scene and its expanded twin: how far along
    /// (0 = compact, 1 = expanded). Shared elements slide by it; see `hero`.
    morph: Cell<Option<f32>>,
    pressed: Cell<bool>,
    light: Cell<bool>,
    focus: Cell<Option<usize>>,
    /// True while the live scene (not the one fading out) is drawing: only it shows focus.
    focus_live: Cell<bool>,
    /// Right edge available to the compact task label this frame (see `exp_label_w`): both
    /// scenes of a morph must agree on the shared label's two end positions.
    hero_right: Cell<f32>,
    /// Last pill silhouette, keyed by its size: ambient frames (spinner, pulses) repeat it.
    geom: RefCell<Option<([u32; 2], ID2D1PathGeometry)>>,
}

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
    /// Classic renderer: DIB + `UpdateLayeredWindow`.
    pub fn new() -> windows::core::Result<Self> {
        Self::build(None)
    }

    /// GPU renderer presenting through DirectComposition into `hwnd`, which must have
    /// been created with `WS_EX_NOREDIRECTIONBITMAP`.
    pub fn new_gpu(hwnd: HWND) -> windows::core::Result<Self> {
        Self::build(Some(hwnd))
    }

    fn build(hwnd: Option<HWND>) -> windows::core::Result<Self> {
        unsafe {
            let factory1: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let factory: ID2D1Factory = factory1.cast()?;
            let (rt, dc_rt, gpu): (ID2D1RenderTarget, _, _) = match hwnd {
                Some(h) => {
                    let g = Gpu::new(&factory1, h, CANVAS_W as u32, CANVAS_H as u32)?;
                    (g.dc.cast()?, None, Some(g))
                }
                None => {
                    let props = D2D1_RENDER_TARGET_PROPERTIES {
                        // The pill is a few hundred px: drawing on the CPU beats a GPU round trip
                        // (measured p50 6.5 ms -> 3.6 ms). HYTTE_HARDWARE=1 restores the GPU target.
                        r#type: if std::env::var_os("HYTTE_HARDWARE").is_some_and(|v| v != "0") {
                            D2D1_RENDER_TARGET_TYPE_DEFAULT
                        } else {
                            D2D1_RENDER_TARGET_TYPE_SOFTWARE
                        },
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.0,
                        dpiY: 96.0,
                        usage: D2D1_RENDER_TARGET_USAGE_NONE,
                        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                    };
                    let dc = factory.CreateDCRenderTarget(&props)?;
                    (dc.cast()?, Some(dc), None)
                }
            };
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
            let mem = if gpu.is_some() {
                HDC::default()
            } else {
                CreateCompatibleDC(None)
            };
            Ok(Self {
                factory,
                factory1,
                rt,
                dc_rt,
                gpu,
                placed: Cell::new((0, 0, 0, 0)),
                region: Cell::new((0, 0, 0, 0)),
                lost: Cell::new(false),
                br,
                round,
                dashed,
                f,
                mem,
                bmp: HBITMAP::default(),
                old: HGDIOBJ::default(),
                px: (0, 0),
                scale: 1.0,
                ca: Cell::new(1.0),
                ox: Cell::new(0.0),
                t: Cell::new(0.0),
                mouse: Cell::new(None),
                hits: RefCell::new(vec![]),
                shown: RefCell::new(HashMap::new()),
                art: RefCell::new(None),
                thumbs: RefCell::new(HashMap::new()),
                art_gen: Cell::new(u32::MAX),
                body: RefCell::new(None),
                shimmer: RefCell::new(None),
                wide: RefCell::new(Vec::new()),
                geom: RefCell::new(None),
                morph: Cell::new(None),
                pressed: Cell::new(false),
                light: Cell::new(false),
                focus: Cell::new(None),
                focus_live: Cell::new(false),
                hero_right: Cell::new(0.0),
            })
        }
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
        *self.body.borrow_mut() = None;
        *self.shimmer.borrow_mut() = None;
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

    /// (Re)allocate the DIB for a new device scale.
    pub fn set_scale(&mut self, scale: f32) {
        let w = (CANVAS_W * scale).ceil() as i32;
        let h = (CANVAS_H * scale).ceil() as i32;
        if let Some(g) = &self.gpu {
            g.resize(w as u32, h as u32);
            self.px = (w, h);
            self.scale = scale;
            return;
        }
        if self.px == (w, h) && !self.bmp.is_invalid() {
            return;
        }
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.mem, self.old);
                let _ = DeleteObject(self.bmp.into());
            }
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            self.bmp = CreateDIBSection(Some(self.mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0)
                .unwrap_or_default();
            self.old = SelectObject(self.mem, self.bmp.into());
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
        if self.gpu.is_some() {
            // The whole canvas is the surface; only the pill's box is visible and clickable.
            let x0 = (self.px.0 - w_px) / 2;
            return Crop {
                w_px: self.px.0,
                h_px: self.px.1,
                w_log: self.px.0 as f32 / self.scale,
                clip: (x0, 0, x0 + w_px, h_px),
            };
        }
        Crop {
            w_px,
            h_px,
            w_log: w_px as f32 / self.scale,
            clip: (0, 0, w_px, h_px),
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
        self.focus.set(fr.focus);
        self.focus_live.set(false);
        self.sync_art(fr.model);
        let (pw, ph) = (fr.anim.rect.w.pos as f32, fr.anim.rect.h.pos as f32);
        let ox = (crop.w_log - pw) / 2.0;
        self.ox.set(ox);
        unsafe {
            let rc = RECT {
                left: 0,
                top: 0,
                right: crop.w_px,
                bottom: crop.h_px,
            };
            let ready = match (&self.gpu, &self.dc_rt) {
                (Some(g), _) => g.begin(),
                (None, Some(dc)) => dc.BindDC(self.mem, &rc).is_ok(),
                _ => false,
            };
            if !ready {
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

    /// Push the DIB to the layered window at screen position (x, y).
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
        if let Some(g) = &self.gpu {
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
            return;
        }
        unsafe {
            let dst = POINT { x, y };
            let size = SIZE {
                cx: crop.w_px,
                cy: crop.h_px,
            };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                None,
                Some(&dst),
                Some(&size),
                Some(self.mem),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
        }
    }

    // ---------------------------------------------------------------- helpers

    /// Pill body gradient, built once per acrylic setting; only its end point changes per frame.
    fn body_brush(&self, acrylic: bool) -> Option<ID2D1LinearGradientBrush> {
        let key = acrylic as u8 | (self.light.get() as u8) << 1;
        let mut slot = self.body.borrow_mut();
        if let Some((k, b)) = slot.as_ref() {
            if *k == key {
                return Some(b.clone());
            }
        }
        let alpha = if acrylic { 0.84 } else { 1.0 };
        let (top, bottom) = self.body_colors();
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: color(top, alpha),
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: color(bottom, alpha),
            },
        ];
        let b = self.linear_brush(&stops)?;
        *slot = Some((key, b.clone()));
        Some(b)
    }

    /// Text and line colour: white on the dark pill, near-black on the light one.
    fn fg(&self) -> [f32; 3] {
        if self.light.get() {
            [0.07, 0.07, 0.09]
        } else {
            WHITE
        }
    }

    /// The opposite of `fg`: ink for things drawn on top of an `fg`-coloured shape.
    fn ink(&self) -> [f32; 3] {
        if self.light.get() {
            [0.97, 0.97, 0.98]
        } else {
            [0.04, 0.04, 0.05]
        }
    }

    /// Pill body gradient (top, bottom).
    fn body_colors(&self) -> ([f32; 3], [f32; 3]) {
        if self.light.get() {
            ([0.97, 0.97, 0.985], [0.89, 0.89, 0.92])
        } else {
            ([0.027, 0.027, 0.035], [0.075, 0.075, 0.095])
        }
    }

    /// Shimmer for indeterminate progress, built once per colour; the content fade is
    /// applied through the brush opacity and the position through its end points.
    fn shimmer_brush(&self, col: [f32; 3]) -> Option<ID2D1LinearGradientBrush> {
        let mut slot = self.shimmer.borrow_mut();
        if let Some((c, b)) = slot.as_ref() {
            if *c == col {
                return Some(b.clone());
            }
        }
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: color(col, 0.0),
            },
            D2D1_GRADIENT_STOP {
                position: 0.5,
                color: color(col, 1.0),
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: color(col, 0.0),
            },
        ];
        let b = self.linear_brush(&stops)?;
        *slot = Some((col, b.clone()));
        Some(b)
    }

    fn linear_brush(&self, stops: &[D2D1_GRADIENT_STOP]) -> Option<ID2D1LinearGradientBrush> {
        unsafe {
            let c = self
                .rt
                .CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .ok()?;
            self.rt
                .CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: v(0.0, 0.0),
                        endPoint: v(0.0, 1.0),
                    },
                    None,
                    &c,
                )
                .ok()
        }
    }

    fn solid(&self, c: D2D1_COLOR_F) -> &ID2D1SolidColorBrush {
        unsafe { self.br.SetColor(&c) };
        &self.br
    }

    /// Colour with the content fade applied.
    fn cc(&self, c: [f32; 3], a: f32) -> D2D1_COLOR_F {
        color(c, a * self.ca.get())
    }

    fn fill_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: D2D1_COLOR_F) {
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(x, y, w, h),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.solid(c)) };
    }

    fn stroke_rr(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        c: D2D1_COLOR_F,
        width: f32,
        dashed: bool,
    ) {
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(x, y, w, h),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            self.rt.DrawRoundedRectangle(
                &rr,
                self.solid(c),
                width,
                if dashed { &self.dashed } else { &self.round },
            )
        };
    }

    fn circle(&self, cx: f32, cy: f32, r: f32, c: D2D1_COLOR_F) {
        let e = D2D1_ELLIPSE {
            point: v(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.FillEllipse(&e, self.solid(c)) };
    }

    /// A Segoe Fluent Icons glyph centred on (cx, cy) in a square box of side `s`.
    fn glyph(&self, cp: char, cx: f32, cy: f32, s: f32, c: D2D1_COLOR_F) {
        let mut b = [0u8; 4];
        self.text(
            cp.encode_utf8(&mut b),
            &self.f.icon,
            cx - s / 2.0,
            cy - s / 2.0,
            s,
            s,
            c,
        );
    }

    fn ring(&self, cx: f32, cy: f32, r: f32, c: D2D1_COLOR_F, width: f32) {
        let e = D2D1_ELLIPSE {
            point: v(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.DrawEllipse(&e, self.solid(c), width, &self.round) };
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), c: D2D1_COLOR_F, width: f32) {
        unsafe {
            self.rt
                .DrawLine(v(a.0, a.1), v(b.0, b.1), self.solid(c), width, &self.round)
        };
    }

    fn poly(&self, pts: &[(f32, f32)], c: D2D1_COLOR_F) {
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(s) = g.Open() else { return };
            s.BeginFigure(v(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_FILLED);
            for p in &pts[1..] {
                s.AddLine(v(p.0, p.1));
            }
            s.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = s.Close();
            self.rt.FillGeometry(&g, self.solid(c), None::<&ID2D1Brush>);
        }
    }

    fn text(
        &self,
        s: &str,
        f: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: D2D1_COLOR_F,
    ) {
        if w <= 1.0 {
            return;
        }
        let mut wide = self.wide.borrow_mut();
        wide.clear();
        wide.extend(s.encode_utf16());
        unsafe {
            self.rt.DrawText(
                &wide,
                f,
                &rect(x, y, w, h),
                self.solid(c),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    /// Register a clickable region (pill-relative); true when hovered.
    fn hit(&self, x: f32, y: f32, w: f32, h: f32, action: Action) -> bool {
        let ox = self.ox.get();
        let index = self.hits.borrow().len();
        self.hits.borrow_mut().push(Hit {
            rect: (x + ox, y, x + ox + w, y + h),
            action,
        });
        if self.focus_live.get() && self.focus.get() == Some(index) {
            self.stroke_rr(
                x - 2.0,
                y - 2.0,
                w + 4.0,
                h + 4.0,
                (h / 2.0).min(10.0) + 2.0,
                color([0.45, 0.72, 1.0], 0.95),
                1.6,
                false,
            );
        }
        self.mouse
            .get()
            .is_some_and(|(mx, my)| mx >= x + ox && mx <= x + ox + w && my >= y && my <= y + h)
    }

    fn button(
        &self,
        label: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        accent: [f32; 3],
        action: Action,
    ) {
        let hov = self.hit(x, y, w, h, action);
        // Pressed in: a touch smaller and brighter while the button is held.
        let down = hov && self.pressed.get();
        let (x, y, w, h) = if down {
            (x + w * 0.03, y + h * 0.03, w * 0.94, h * 0.94)
        } else {
            (x, y, w, h)
        };
        let a = if down {
            0.36
        } else if hov {
            0.26
        } else {
            0.13
        };
        self.fill_rr(x, y, w, h, h / 2.0, self.cc(accent, a));
        self.text(
            label,
            &self.f.btn,
            x,
            y,
            w,
            h,
            self.cc(self.fg(), if hov { 1.0 } else { 0.86 }),
        );
    }

    /// A filled call-to-action: the accent itself with dark ink on it.
    fn primary(
        &self,
        label: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        accent: [f32; 3],
        action: Action,
    ) {
        let hov = self.hit(x, y, w, h, action);
        let down = hov && self.pressed.get();
        let (x, y, w, h) = if down {
            (x + w * 0.02, y + h * 0.03, w * 0.96, h * 0.94)
        } else {
            (x, y, w, h)
        };
        self.fill_rr(
            x,
            y,
            w,
            h,
            h / 2.0,
            self.cc(accent, if hov { 1.0 } else { 0.86 }),
        );
        self.text(
            label,
            &self.f.btn,
            x,
            y,
            w,
            h,
            self.cc([0.03, 0.03, 0.05], 0.92),
        );
    }

    /// A round icon button; `draw` paints the icon centred on (cx, cy) in the given colour.
    fn round_btn(
        &self,
        cx: f32,
        cy: f32,
        r: f32,
        accent: [f32; 3],
        action: Action,
        draw: impl Fn(f32, f32, D2D1_COLOR_F),
    ) {
        let hov = self.hit(cx - r, cy - r, r * 2.0, r * 2.0, action);
        let down = hov && self.pressed.get();
        let rr = if down { r * 0.94 } else { r };
        let a = if down {
            0.34
        } else if hov {
            0.24
        } else {
            0.12
        };
        self.circle(cx, cy, rr, self.cc(accent, a));
        draw(cx, cy, self.cc(self.fg(), if hov { 1.0 } else { 0.88 }));
    }

    /// A larger Segoe Fluent Icons glyph centred on (cx, cy).
    fn glyph_l(&self, cp: char, cx: f32, cy: f32, c: D2D1_COLOR_F) {
        let mut b = [0u8; 4];
        self.text(
            cp.encode_utf8(&mut b),
            &self.f.icon_l,
            cx - 16.0,
            cy - 16.0,
            32.0,
            32.0,
            c,
        );
    }

    /// Clockwise arc from 12 o'clock covering `frac` of the circle.
    fn arc(&self, cx: f32, cy: f32, r: f32, frac: f32, c: D2D1_COLOR_F, width: f32) {
        if frac <= 0.002 {
            return;
        }
        if frac >= 0.998 {
            self.ring(cx, cy, r, c, width);
            return;
        }
        let a = frac * std::f32::consts::TAU;
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(sk) = g.Open() else { return };
            sk.BeginFigure(v(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
            sk.AddArc(&D2D1_ARC_SEGMENT {
                point: v(cx + r * a.sin(), cy - r * a.cos()),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sk.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sk.Close();
            self.rt.DrawGeometry(&g, self.solid(c), width, &self.round);
        }
    }

    /// Two rounded bars.
    fn pause_icon(&self, cx: f32, cy: f32, s: f32, c: D2D1_COLOR_F) {
        let bw = s * 0.3;
        self.fill_rr(cx - s * 0.38, cy - s / 2.0, bw, s, bw / 2.5, c);
        self.fill_rr(cx + s * 0.08, cy - s / 2.0, bw, s, bw / 2.5, c);
    }

    /// A play triangle, nudged right so it looks centred.
    fn play_icon(&self, cx: f32, cy: f32, s: f32, c: D2D1_COLOR_F) {
        let h = s / 2.0;
        self.poly(
            &[(cx - h * 0.7, cy - h), (cx - h * 0.7, cy + h), (cx + h, cy)],
            c,
        );
    }

    /// Previous (dir -1) or next (dir +1) track: a triangle against a bar.
    fn skip_icon(&self, cx: f32, cy: f32, dir: f32, c: D2D1_COLOR_F) {
        let h = 6.5;
        self.poly(
            &[
                (cx - 5.0 * dir, cy - h),
                (cx - 5.0 * dir, cy + h),
                (cx + 4.0 * dir, cy),
            ],
            c,
        );
        let bx = if dir > 0.0 { cx + 4.5 } else { cx - 7.0 };
        self.fill_rr(bx, cy - h, 2.5, h * 2.0, 1.2, c);
    }

    /// Rotate what is drawn next by `deg` around (cx, cy); returns the transform to restore.
    fn rotate(&self, deg: f32, cx: f32, cy: f32) -> Matrix3x2 {
        let mut cur = Matrix3x2::default();
        unsafe { self.rt.GetTransform(&mut cur) };
        let (s, c) = deg.to_radians().sin_cos();
        let r = Matrix3x2 {
            M11: c,
            M12: s,
            M21: -s,
            M22: c,
            M31: cx - c * cx + s * cy,
            M32: cy - s * cx - c * cy,
        };
        let m = Matrix3x2 {
            M11: r.M11 * cur.M11 + r.M12 * cur.M21,
            M12: r.M11 * cur.M12 + r.M12 * cur.M22,
            M21: r.M21 * cur.M11 + r.M22 * cur.M21,
            M22: r.M21 * cur.M12 + r.M22 * cur.M22,
            M31: r.M31 * cur.M11 + r.M32 * cur.M21 + cur.M31,
            M32: r.M31 * cur.M12 + r.M32 * cur.M22 + cur.M32,
        };
        unsafe { self.rt.SetTransform(&m) };
        cur
    }

    /// A quick-control tile: icon, name and state. `on` tints it with that accent.
    fn tile(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        icon: char,
        label: &str,
        sub: &str,
        on: Option<[f32; 3]>,
        action: Action,
    ) {
        let hov = self.hit(x, y, w, h, action);
        let down = hov && self.pressed.get();
        let (x, y, w, h) = if down {
            (x + w * 0.03, y + h * 0.03, w * 0.94, h * 0.94)
        } else {
            (x, y, w, h)
        };
        match on {
            Some(c) => {
                self.fill_rr(x, y, w, h, 14.0, self.cc(c, if hov { 0.30 } else { 0.22 }));
                self.stroke_rr(
                    x + 0.5,
                    y + 0.5,
                    w - 1.0,
                    h - 1.0,
                    13.5,
                    self.cc(c, 0.35),
                    1.0,
                    false,
                );
            }
            None => self.fill_rr(
                x,
                y,
                w,
                h,
                14.0,
                self.cc(self.fg(), if hov { 0.11 } else { 0.06 }),
            ),
        }
        let ic = on.unwrap_or(self.fg());
        self.glyph_l(icon, x + w / 2.0, y + 20.0, self.cc(ic, 0.95));
        self.text(
            label,
            &self.f.btn,
            x,
            y + 33.0,
            w,
            14.0,
            self.cc(self.fg(), 0.95),
        );
        let sc = if on.is_some() { ic } else { GRAY };
        self.text(
            sub,
            &self.f.small_c,
            x,
            y + 46.0,
            w,
            13.0,
            self.cc(sc, 0.95),
        );
    }

    // ------------------------------------------------------------------ shape

    fn pill_geometry(&self, w: f32, h: f32, rb: f32, e: f32) -> Option<ID2D1PathGeometry> {
        let key = [w.to_bits(), h.to_bits()];
        if let Some((k, g)) = self.geom.borrow().as_ref() {
            if *k == key {
                return Some(g.clone());
            }
        }
        let g = self.build_pill_geometry(w, h, rb, e)?;
        *self.geom.borrow_mut() = Some((key, g.clone()));
        Some(g)
    }

    fn build_pill_geometry(&self, w: f32, h: f32, rb: f32, e: f32) -> Option<ID2D1PathGeometry> {
        unsafe {
            let g = self.factory.CreatePathGeometry().ok()?;
            let s = g.Open().ok()?;
            let arc = |x: f32, y: f32, r: f32, cw: bool| D2D1_ARC_SEGMENT {
                point: v(x, y),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: if cw {
                    D2D1_SWEEP_DIRECTION_CLOCKWISE
                } else {
                    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                },
                arcSize: D2D1_ARC_SIZE_SMALL,
            };
            s.BeginFigure(v(-e, 0.0), D2D1_FIGURE_BEGIN_FILLED);
            s.AddLine(v(w + e, 0.0));
            if e > 0.5 {
                s.AddArc(&arc(w, e, e, false));
            }
            s.AddLine(v(w, h - rb));
            s.AddArc(&arc(w - rb, h, rb, true));
            s.AddLine(v(rb, h));
            s.AddArc(&arc(0.0, h - rb, rb, true));
            s.AddLine(v(0.0, e));
            if e > 0.5 {
                s.AddArc(&arc(-e, 0.0, e, false));
            }
            s.EndFigure(D2D1_FIGURE_END_CLOSED);
            s.Close().ok()?;
            Some(g)
        }
    }

    fn background(&self, fr: &Frame, w: f32, h: f32) {
        let an = fr.anim;
        let sentinel = an.scene == Scene::Sentinel;
        let rb = (h * 0.5).min(24.0);
        let e = if h > 18.0 { (h * 0.3).min(8.0) } else { 0.0 };
        if sentinel {
            self.fill_rr(0.0, 0.0, w, h, h / 2.0, color(self.fg(), 0.30));
            return;
        }
        let Some(g) = self.pill_geometry(w, h, rb, e) else {
            return;
        };
        unsafe {
            // Soft glow: stacked widening strokes under the body.
            let mut gl = an.glow.pos as f32 * fr.model.glow_gain;
            if fr.model.primary().is_some_and(|t| t.needs_input())
                && matches!(an.scene, Scene::CompactTask | Scene::ExpTasks)
            {
                gl *= 0.72 + 0.28 * (self.t.get() * 4.0).sin();
            }
            if gl > 0.01 {
                // Five strokes, each standing in for a pair of the nine it replaced: stroking
                // the silhouette is the costliest thing a frame does.
                for j in 1..=5 {
                    let k = |i: i32| (1.0 - i as f32 / 10.0).powi(2);
                    let a = gl * 0.075 * (k(2 * j - 1) + k(2 * j));
                    self.rt.DrawGeometry(
                        &g,
                        self.solid(color(an.glow_rgb, a)),
                        j as f32 * 4.4,
                        &self.round,
                    );
                }
            }
            // Body: near-black with a faint vertical gradient.
            let alpha = if fr.acrylic { 0.84 } else { 1.0 };
            match self.body_brush(fr.acrylic) {
                Some(b) => {
                    b.SetEndPoint(v(0.0, h.max(1.0)));
                    self.rt.FillGeometry(&g, &b, None::<&ID2D1Brush>)
                }
                None => self.rt.FillGeometry(
                    &g,
                    self.solid(color(self.body_colors().0, alpha)),
                    None::<&ID2D1Brush>,
                ),
            }
            // Hairline rim + hover brighten.
            self.rt
                .DrawGeometry(&g, self.solid(color(self.fg(), 0.07)), 1.0, &self.round);
            let hv = an.hover.pos as f32;
            if hv > 0.01 {
                self.rt.FillGeometry(
                    &g,
                    self.solid(color(self.fg(), 0.025 * hv)),
                    None::<&ID2D1Brush>,
                );
            }
        }
    }

    // ----------------------------------------------------------------- scenes

    /// The small pill split off to the right of a compact one: it slides out from behind the
    /// main pill as the spring runs, and slides back in when the second activity ends.
    fn bubble(&self, fr: &Frame, pw: f32, ph: f32) {
        let an = fr.anim;
        let s = an.bubble.pos as f32;
        let Some(kind) = an.bubble_kind.filter(|_| s > 0.01) else {
            return;
        };
        let bw = match kind {
            Bubble::Timer => 58.0,
            Bubble::Media => 34.0,
        };
        let bh = ph.min(32.0);
        let x = (pw - bw - 6.0) + (bw + 14.0) * s;
        let a = s.clamp(0.0, 1.0);
        let (_, bottom) = self.body_colors();
        // Hangs from the top edge like the pill: its top corners are above the screen.
        self.fill_rr(x, -bh / 2.0, bw, bh * 1.5, bh / 2.0, color(bottom, a));
        self.stroke_rr(
            x,
            -bh / 2.0,
            bw,
            bh * 1.5,
            bh / 2.0,
            color(self.fg(), 0.07 * a),
            1.0,
            false,
        );
        let cy = bh / 2.0;
        match kind {
            Bubble::Timer => {
                let Some(t) = fr.model.timer.as_ref() else {
                    return;
                };
                let wall = crate::timer::now_ms();
                let left = t.remaining_ms(wall);
                let col = if left < 60_000 { RED } else { BLUE };
                self.ca.set(a);
                let clock = if left >= 3_600_000 {
                    format!("{}:{:02}h", left / 3_600_000, (left / 60_000) % 60)
                } else {
                    fmt_clock(left)
                };
                self.text(
                    &clock,
                    &self.f.btn,
                    x,
                    cy - 8.0,
                    bw,
                    16.0,
                    self.cc(col, 1.0),
                );
            }
            Bubble::Media => {
                self.ca.set(a);
                self.art_tile(fr.model, x + (bw - 20.0) / 2.0, cy - 10.0, 20.0, 10.0);
            }
        }
        self.ca.set(1.0);
    }

    fn scene(&self, fr: &Frame, w: f32, h: f32) {
        self.bubble(fr, w, h);
        self.background(fr, w, h);
        let an = fr.anim;
        if an.scene == Scene::Sentinel {
            return;
        }
        if let Some(t) = fr
            .model
            .timer
            .as_ref()
            // Collapsed only: expanded, the timer pane and the Timer tile show it.
            .filter(|_| {
                matches!(
                    an.scene,
                    Scene::Idle | Scene::CompactTask | Scene::CompactMedia
                )
            })
        {
            let m = fr.model;
            let centre = an.scene == Scene::Idle && m.ports.is_empty() && m.shelf.is_empty();
            self.timer_filament(t, w, h, centre, if m.mic_muted { LOCK_W } else { 0.0 });
        }
        let m = fr.model;
        let lock = if m.mic_muted { LOCK_W } else { 0.0 };
        let pad_r = lock
            + if m.privacy_active() {
                12.0 + 14.0 * (m.cam as u8 + m.mic as u8) as f32
            } else {
                0.0
            };
        let fit = ((h - 14.0) / 10.0).clamp(0.0, 1.0);
        self.morph.set(self.morph_for(fr));
        self.hero_right
            .set(w - 14.0 - pad_r - if m.visible_count() > 1 { 28.0 } else { 0.0 });
        // The scene being left fades out and drifts up, clipped to the pill as it
        // changes size. It is not interactive: no hover, and its hit areas are dropped.
        let o = an.out.pos.clamp(0.0, 1.0) as f32;
        if o > 0.01 && an.prev != an.scene && an.prev != Scene::Sentinel {
            self.ca.set(o * fit);
            let mouse = self.mouse.take();
            let n = self.hits.borrow().len();
            unsafe {
                let (dx, dy) = if an.slide != 0.0 {
                    (-an.slide * (1.0 - o) * 16.0, 0.0)
                } else {
                    (0.0, -(1.0 - o) * 4.0)
                };
                self.rt
                    .SetTransform(&mat(self.scale, self.ox.get() + dx, dy));
                self.rt
                    .PushAxisAlignedClip(&rect(0.0, 0.0, w, h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
            // dt = 0: progress smoothing already advances once this frame, in the live scene.
            let old = Frame { dt: 0.0, ..*fr };
            self.content(&old, an.prev, w, h, pad_r);
            unsafe { self.rt.PopAxisAlignedClip() };
            self.hits.borrow_mut().truncate(n);
            self.mouse.set(mouse);
        }
        // Content fades/slides in on scene change.
        let c = an.content.pos.clamp(0.0, 1.0) as f32;
        self.ca.set(c * fit);
        let (dx, dy) = if an.slide != 0.0 {
            (an.slide * (1.0 - c) * 16.0, 0.0)
        } else {
            (0.0, (1.0 - c) * 6.0)
        };
        unsafe {
            self.rt
                .SetTransform(&mat(self.scale, self.ox.get() + dx, dy))
        };
        self.focus_live.set(true);
        self.content(fr, an.scene, w, h, pad_r);
        self.focus_live.set(false);
        self.morph.set(None);
        // The quick controls spell out the mic state (its tile), not the camera's: keep the dots.
        let home = an.scene == Scene::ExpHome;
        if m.privacy_active() {
            let cy = if h < 40.0 { h / 2.0 } else { 16.0 };
            self.privacy_dots(
                m,
                w - 14.0 - lock,
                cy,
                !matches!(
                    an.scene,
                    Scene::Idle | Scene::CompactTask | Scene::CompactMedia
                ),
            );
        }
        if m.mic_muted
            && !home
            && !matches!(
                an.scene,
                Scene::ExpDrop | Scene::ExpChip | Scene::ExpTimer | Scene::ExpTimerDone
            )
        {
            let cy = if h < 40.0 { h / 2.0 } else { 16.0 };
            self.mute_badge(w - 14.0 - 6.0, cy);
        }
    }

    /// Body of one scene (and its tab strip), in pill-local coordinates.
    fn content(&self, fr: &Frame, scene: Scene, w: f32, h: f32, pad_r: f32) {
        match scene {
            Scene::Sentinel => {}
            Scene::Idle => self.idle(fr, w, h),
            Scene::CompactTask => self.compact_task(fr, w, h, pad_r),
            Scene::CompactMedia => self.compact_media(fr, w, h, pad_r),
            Scene::ExpTasks => self.exp_tasks(fr, w, h),
            Scene::ExpMedia => self.exp_media(fr, w, h),
            Scene::ExpShelf => self.exp_shelf(fr, w, h),
            Scene::ExpHome => self.exp_home(fr, w, h),
            Scene::ExpPower => self.exp_power(fr, w, h),
            Scene::ExpDrop => self.exp_drop(fr, w, h),
            Scene::ExpChip => self.exp_chip(fr, w, h),
            Scene::ExpTimer => self.exp_timer(fr, w, h),
            Scene::ExpTimerDone => self.exp_timer_done(fr, w, h),
            Scene::ExpCharge => self.exp_charge(fr, w, h),
        }
        if fr.model.tabs()
            && matches!(
                scene,
                Scene::ExpTasks
                    | Scene::ExpMedia
                    | Scene::ExpShelf
                    | Scene::ExpHome
                    | Scene::ExpPower
                    | Scene::ExpTimer
            )
        {
            self.tabs(fr.model, w, h);
        }
    }

    /// Progress of a compact <-> expanded transition of the same subject (media with media,
    /// the task list with the compact task), taken from the pill's own height spring so the
    /// shared elements move exactly as fast as the pill grows. None for any other change.
    fn morph_for(&self, fr: &Frame) -> Option<f32> {
        let an = fr.anim;
        let exp = match (an.prev, an.scene) {
            (Scene::CompactMedia, Scene::ExpMedia) | (Scene::ExpMedia, Scene::CompactMedia) => {
                Scene::ExpMedia
            }
            (Scene::CompactTask, Scene::ExpTasks) | (Scene::ExpTasks, Scene::CompactTask) => {
                Scene::ExpTasks
            }
            _ => return None,
        };
        let he = fr.model.size(exp).1 as f32;
        Some(((an.rect.h.pos as f32 - 32.0) / (he - 32.0)).clamp(0.0, 1.0))
    }

    /// Where an element shared by a compact scene and its expanded twin goes. While the pill
    /// morphs it slides between `compact` and `exp` and is drawn fully opaque (so the
    /// cross-fading old and new scenes don't dim it); otherwise it is at `natural`. Returns
    /// the rect and what to restore with `hero_end`. The scenes' own slide-and-drift offsets
    /// are dropped too, or the outgoing and incoming copies would sit a few pixels apart.
    fn hero(
        &self,
        compact: [f32; 4],
        exp: [f32; 4],
        natural: [f32; 4],
    ) -> ([f32; 4], Option<(f32, Matrix3x2)>) {
        match self.morph.get() {
            Some(p) => {
                let mut r = [0.0; 4];
                for i in 0..4 {
                    r[i] = compact[i] + (exp[i] - compact[i]) * p;
                }
                let mut m = Matrix3x2::default();
                // SAFETY: `m` is a valid out-struct for the call.
                unsafe {
                    self.rt.GetTransform(&mut m);
                    self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0));
                }
                (r, Some((self.ca.replace(1.0), m)))
            }
            None => (natural, None),
        }
    }

    /// Width of a task row's label in the expanded list.
    fn exp_label_w(w: f32, t: &TaskView) -> f32 {
        w - 42.0 - 74.0 - if t.running() { 0.0 } else { 24.0 } - 22.0
    }

    /// The compact task label's rect and the same label's rect in the first expanded row.
    fn label_ends(&self, w: f32, t: &TaskView) -> ([f32; 4], [f32; 4]) {
        (
            [38.0, 6.0, self.hero_right.get() - 62.0 - 46.0, 17.0],
            [42.0, 17.0, Self::exp_label_w(w, t), 20.0],
        )
    }

    fn hero_end(&self, saved: Option<(f32, Matrix3x2)>) {
        if let Some((a, m)) = saved {
            self.ca.set(a);
            unsafe { self.rt.SetTransform(&m) };
        }
    }

    fn idle(&self, fr: &Frame, w: f32, h: f32) {
        let m = fr.model;
        let chips = (!m.ports.is_empty()) as u8 + (!m.shelf.is_empty()) as u8;
        let lock = if m.mic_muted { LOCK_W } else { 0.0 };
        if chips == 0 {
            if m.timer.is_some() {
                return;
            }
            self.fill_rr(
                (w - lock) / 2.0 - 13.0,
                h / 2.0 - 1.5,
                26.0,
                3.0,
                1.5,
                self.cc(self.fg(), 0.16),
            );
            return;
        }
        let right_pad = lock + if m.privacy_active() { 34.0 } else { 0.0 };
        let mut x = (w - right_pad - 58.0 * chips as f32) / 2.0 + 4.0;
        let cy = h / 2.0;
        if let Some(p) = m.ports.first() {
            self.circle(x + 3.0, cy, 2.6, self.cc(GREEN, 1.0));
            let t = if m.ports.len() > 1 {
                format!(":{} +{}", p.port, m.ports.len() - 1)
            } else {
                format!(":{}", p.port)
            };
            self.text(
                &t,
                &self.f.small_l,
                x + 10.0,
                cy - 8.0,
                52.0,
                16.0,
                self.cc(self.fg(), 0.82),
            );
            x += 58.0;
        }
        if !m.shelf.is_empty() {
            self.fill_rr(x, cy - 3.5, 9.0, 8.0, 2.0, self.cc(PURPLE, 0.45));
            self.fill_rr(x + 2.0, cy - 5.5, 9.0, 8.0, 2.0, self.cc(PURPLE, 1.0));
            self.text(
                &m.shelf.len().to_string(),
                &self.f.small_l,
                x + 15.0,
                cy - 8.0,
                28.0,
                16.0,
                self.cc(self.fg(), 0.82),
            );
        }
    }

    /// Padlock: filled body plus a stroked shackle.
    fn lock_icon(&self, cx: f32, cy: f32, col: [f32; 3], a: f32) {
        self.stroke_rr(
            cx - 3.2,
            cy - 6.2,
            6.4,
            9.0,
            3.2,
            self.cc(col, a),
            1.7,
            false,
        );
        self.fill_rr(cx - 5.5, cy - 1.5, 11.0, 8.5, 2.2, self.cc(col, a));
        self.circle(cx, cy + 2.2, 1.3, self.cc([0.05; 3], a));
    }

    /// Bold red lock shown on every scene while the microphone is muted; click unmutes.
    fn mute_badge(&self, cx: f32, cy: f32) {
        let hov = self.hit(cx - 11.0, cy - 11.0, 22.0, 22.0, Action::ToggleMic);
        let p = (self.t.get() * 2.5).sin() * 0.5 + 0.5;
        self.circle(
            cx,
            cy,
            10.0,
            self.cc(RED, if hov { 0.34 } else { 0.20 + 0.04 * p }),
        );
        self.lock_icon(cx, cy, RED, 1.0);
    }

    fn privacy_dots(&self, m: &Model, right: f32, cy: f32, pulse: bool) {
        let t = self.t.get();
        let mut x = right - 3.5;
        for (on, col) in [(m.cam, CAM), (m.mic, ORANGE)] {
            if !on {
                continue;
            }
            let p = if pulse {
                (t * 2.2).sin() * 0.5 + 0.5
            } else {
                0.0
            };
            self.circle(x, cy, 3.5 + 3.0 * p, self.cc(col, 0.22 * (1.0 - p)));
            self.circle(x, cy, 3.5, self.cc(col, 1.0));
            x -= 14.0;
        }
    }

    fn status_icon(&self, t: &TaskView, cx: f32, cy: f32, now: Instant) {
        let col = task_color(t);
        let since = now.saturating_duration_since(t.changed).as_secs_f32();
        let draw_p = (since / 0.28).clamp(0.0, 1.0);
        let tm = self.t.get();
        match t.event {
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => {
                self.ring(cx, cy, 6.5, self.cc(col, 0.18), 1.8);
                let a0 = tm * 5.5;
                let n = 14;
                let mut prev = (cx + 6.5 * a0.cos(), cy + 6.5 * a0.sin());
                for i in 1..=n {
                    let a = a0 + (i as f32 / n as f32) * 1.9;
                    let p = (cx + 6.5 * a.cos(), cy + 6.5 * a.sin());
                    self.line(
                        prev,
                        p,
                        self.cc(col, 0.35 + 0.65 * i as f32 / n as f32),
                        1.8,
                    );
                    prev = p;
                }
            }
            TaskEvent::Done => {
                self.circle(cx, cy, 8.0, self.cc(col, 0.16));
                let pts = [
                    (cx - 3.6, cy + 0.2),
                    (cx - 1.0, cy + 3.0),
                    (cx + 3.8, cy - 2.8),
                ];
                let seg = |a: (f32, f32), b: (f32, f32), k: f32| {
                    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
                };
                let p1 = (draw_p * 2.0).min(1.0);
                self.line(pts[0], seg(pts[0], pts[1], p1), self.cc(col, 1.0), 2.0);
                if draw_p > 0.5 {
                    self.line(
                        pts[1],
                        seg(pts[1], pts[2], (draw_p - 0.5) * 2.0),
                        self.cc(col, 1.0),
                        2.0,
                    );
                }
            }
            TaskEvent::Failed => {
                let pulse = 0.5 + (tm * 3.2).sin() * 0.5 * t.fail_pulse(now);
                self.circle(cx, cy, 8.0 + pulse, self.cc(col, 0.12 + 0.12 * pulse));
                let d = 3.1;
                self.line((cx - d, cy - d), (cx + d, cy + d), self.cc(col, 1.0), 2.0);
                self.line((cx - d, cy + d), (cx + d, cy - d), self.cc(col, 1.0), 2.0);
            }
            TaskEvent::NeedsInput => {
                // Pulsing halo + bell that rings in short bursts.
                let pulse = (tm * 4.0).sin() * 0.5 + 0.5;
                self.circle(cx, cy, 8.0 + 2.0 * pulse, self.cc(col, 0.14 + 0.16 * pulse));
                let burst = ((tm * 0.9) % 1.0 < 0.3) as u8 as f32;
                let sw = (tm * 22.0).sin() * 0.28 * burst;
                let (s, c) = (sw.sin(), sw.cos());
                let p = |x: f32, y: f32| (cx + x * c - y * s, cy - 1.5 + x * s + y * c);
                self.poly(
                    &[
                        p(-4.2, 3.0),
                        p(-2.6, -2.6),
                        p(0.0, -4.6),
                        p(2.6, -2.6),
                        p(4.2, 3.0),
                    ],
                    self.cc(col, 1.0),
                );
                self.line(p(-5.2, 3.2), p(5.2, 3.2), self.cc(col, 1.0), 1.8);
                self.circle(cx + 0.0, cy + 5.2, 1.5, self.cc(col, 1.0));
            }
            TaskEvent::Lost => {
                self.circle(cx, cy, 8.0, self.cc(col, 0.16));
                self.text(
                    "?",
                    &self.f.center,
                    cx - 8.0,
                    cy - 8.0,
                    16.0,
                    16.0,
                    self.cc(col, 1.0),
                );
            }
        }
    }

    fn task_right_text(&self, t: &TaskView, now: Instant) -> String {
        match t.event {
            TaskEvent::NeedsInput => "waiting".into(),
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => match t.progress {
                Some(p) => format!("{p}%"),
                None => fmt_dur(now.saturating_duration_since(t.started).as_millis() as u64),
            },
            TaskEvent::Done => fmt_dur(t.duration_ms.unwrap_or(0)),
            TaskEvent::Failed => match (t.stderr.is_empty(), t.exit_code) {
                (true, Some(c)) => format!("exit {c} · {}", fmt_dur(t.duration_ms.unwrap_or(0))),
                _ => format!("failed · {}", fmt_dur(t.duration_ms.unwrap_or(0))),
            },
            TaskEvent::Lost => "lost".into(),
        }
    }

    /// Thin bar under a running task: determinate (eased) or shimmer.
    fn filament(&self, t: &TaskView, x: f32, y: f32, w: f32, dt: f32, now: Instant) {
        let col = task_color(t);
        self.fill_rr(x, y, w, 2.0, 1.0, self.cc(self.fg(), 0.08));
        if !t.running() {
            if t.event == TaskEvent::Failed {
                self.fill_rr(
                    x,
                    y,
                    w,
                    2.0,
                    1.0,
                    self.cc(
                        RED,
                        0.55 + 0.25 * (self.t.get() * 3.2).sin() * t.fail_pulse(now),
                    ),
                );
            }
            return;
        }
        match t.progress {
            Some(p) => {
                let mut sh = self.shown.borrow_mut();
                let e = sh.entry(t.id.clone()).or_insert(0.0);
                *e += (p as f32 / 100.0 - *e) * (dt * 10.0).min(1.0);
                self.fill_rr(x, y, (w * *e).max(2.0), 2.0, 1.0, self.cc(col, 1.0));
            }
            None => unsafe {
                let ph = (self.t.get() * 0.85) % 1.0;
                let seg = w * 0.35;
                let sx = x - seg + (w + seg) * ph;
                if let Some(gb) = self.shimmer_brush(col) {
                    gb.SetStartPoint(v(sx, 0.0));
                    gb.SetEndPoint(v(sx + seg, 0.0));
                    gb.SetOpacity(self.ca.get().clamp(0.0, 1.0));
                    self.rt.PushAxisAlignedClip(
                        &rect(x, y, w, 2.0),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    self.rt.FillRectangle(&rect(sx, y, seg, 2.0), &gb);
                    self.rt.PopAxisAlignedClip();
                }
            },
        }
    }

    fn compact_task(&self, fr: &Frame, w: f32, h: f32, pad_r: f32) {
        let m = fr.model;
        let Some(t) = m.primary() else { return };
        let now = fr.now;
        // Shake once when a task has just failed.
        let since = now.saturating_duration_since(t.changed).as_secs_f32();
        if t.failed() && since < 0.5 {
            let dx = (since * 55.0).sin() * 3.5 * (1.0 - since / 0.5);
            unsafe {
                self.rt
                    .SetTransform(&mat(self.scale, self.ox.get() + dx, 0.0))
            };
        }
        // The icon and label are shared with the first row of the expanded list.
        let (ic, saved) = self.hero(
            [20.0, 14.5, 0.0, 0.0],
            [24.0, 27.0, 0.0, 0.0],
            [20.0, h / 2.0 - 1.5, 0.0, 0.0],
        );
        self.status_icon(t, ic[0], ic[1], now);
        self.hero_end(saved);
        let mut right = w - 14.0 - pad_r;
        let n = m.visible_count();
        if n > 1 {
            self.fill_rr(
                right - 20.0,
                h / 2.0 - 9.0,
                20.0,
                16.0,
                8.0,
                self.cc(self.fg(), 0.14),
            );
            self.text(
                &n.to_string(),
                &self.f.btn,
                right - 20.0,
                h / 2.0 - 9.0,
                20.0,
                16.0,
                self.cc(self.fg(), 0.9),
            );
            right -= 28.0;
        }
        let rt = self.task_right_text(t, now);
        let rw = 62.0;
        self.text(
            &rt,
            &self.f.small_r,
            right - rw,
            h / 2.0 - 10.0,
            rw,
            17.0,
            self.cc(task_color(t), 0.95),
        );
        let (lc, le) = self.label_ends(w, t);
        let (lb, saved) = self.hero(lc, le, [38.0, h / 2.0 - 10.0, right - rw - 46.0, 17.0]);
        self.text(
            &t.label,
            &self.f.title,
            lb[0],
            lb[1],
            lb[2],
            lb[3],
            self.cc(self.fg(), 0.94),
        );
        self.hero_end(saved);
        self.filament(t, 14.0, h - 5.0, w - 28.0, fr.dt, now);
    }

    fn eq_bars(&self, x: f32, y: f32, h: f32, playing: bool, col: [f32; 3]) {
        let t = self.t.get();
        for i in 0..4 {
            let k = if playing {
                0.28 + 0.72 * ((t * (3.1 + i as f32 * 1.3) + i as f32 * 1.7).sin().abs())
            } else {
                0.25
            };
            let bh = (h * k).max(2.0);
            self.fill_rr(
                x + i as f32 * 4.5,
                y + h - bh,
                2.6,
                bh,
                1.3,
                self.cc(col, 0.95),
            );
        }
    }

    fn art_tile(&self, m: &Model, x: f32, y: f32, s: f32, r: f32) {
        if let Some(bmp) = self.art.borrow().as_ref() {
            unsafe {
                let props = D2D1_BRUSH_PROPERTIES {
                    opacity: self.ca.get(),
                    transform: Matrix3x2 {
                        M11: s / 64.0,
                        M12: 0.0,
                        M21: 0.0,
                        M22: s / 64.0,
                        M31: x,
                        M32: y,
                    },
                };
                if let Ok(b) = self.rt.CreateBitmapBrush(bmp, None, Some(&props)) {
                    let rr = D2D1_ROUNDED_RECT {
                        rect: rect(x, y, s, s),
                        radiusX: r,
                        radiusY: r,
                    };
                    self.rt.FillRoundedRectangle(&rr, &b);
                    return;
                }
            }
        }
        let _ = m;
        self.fill_rr(x, y, s, s, r, self.cc(PURPLE, 0.20));
        self.stroke_rr(
            x + 0.5,
            y + 0.5,
            s - 1.0,
            s - 1.0,
            r,
            self.cc(PURPLE, 0.30),
            1.0,
            false,
        );
        if s >= 40.0 {
            self.glyph_l('\u{EC4F}', x + s / 2.0, y + s / 2.0, self.cc(PURPLE, 0.95));
        } else {
            self.glyph(
                '\u{EC4F}',
                x + s / 2.0,
                y + s / 2.0,
                s,
                self.cc(PURPLE, 0.95),
            );
        }
    }

    fn compact_media(&self, fr: &Frame, w: f32, h: f32, pad_r: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        let (a, saved) = self.hero(
            [8.0, 6.0, 20.0, 5.0],
            [16.0, 16.0, 72.0, 12.0],
            [8.0, h / 2.0 - 10.0, 20.0, 5.0],
        );
        self.art_tile(fr.model, a[0], a[1], a[2], a[3]);
        self.hero_end(saved);
        let right = w - 14.0 - pad_r;
        self.eq_bars(right - 16.0, h / 2.0 - 7.0, 14.0, md.playing, GREEN);
        let label = if md.artist.is_empty() {
            md.title.clone()
        } else {
            format!("{} — {}", md.title, md.artist)
        };
        self.text(
            &label,
            &self.f.body,
            38.0,
            h / 2.0 - 9.0,
            right - 16.0 - 46.0,
            17.0,
            self.cc(self.fg(), 0.92),
        );
    }

    fn exp_tasks(&self, fr: &Frame, w: f32, _h: f32) {
        let now = fr.now;
        let mut y = 10.0;
        for (row, t) in fr.model.rows().into_iter().enumerate() {
            let cy = y + 17.0;
            let finished = !t.running();
            let right_w = 74.0;
            let close_w = if finished { 24.0 } else { 0.0 };
            let label_w = w - 42.0 - right_w - close_w - 22.0;
            // Whole row focuses the owning terminal (buttons below win the hit test).
            let row_hov = match t.pid {
                Some(pid) => self.hit(
                    8.0,
                    y + 2.0,
                    w - 16.0,
                    32.0,
                    Action::FocusTerminal(pid, t.id.clone()),
                ),
                None => false,
            };
            if row_hov {
                self.fill_rr(8.0, y + 2.0, w - 16.0, 32.0, 10.0, self.cc(self.fg(), 0.05));
            }
            // The first row is the compact task's twin: its icon and label slide in from there.
            let first = row == 0;
            let (ic, saved) = if first {
                self.hero(
                    [20.0, 14.5, 0.0, 0.0],
                    [24.0, 27.0, 0.0, 0.0],
                    [24.0, cy, 0.0, 0.0],
                )
            } else {
                ([24.0, cy, 0.0, 0.0], None)
            };
            self.status_icon(t, ic[0], ic[1], now);
            self.hero_end(saved);
            let (lb, saved) = if first {
                let (lc, le) = self.label_ends(w, t);
                self.hero(lc, le, [42.0, y + 7.0, label_w, 20.0])
            } else {
                ([42.0, y + 7.0, label_w, 20.0], None)
            };
            self.text(
                &t.label,
                &self.f.title,
                lb[0],
                lb[1],
                lb[2],
                lb[3],
                self.cc(self.fg(), 0.95),
            );
            self.hero_end(saved);
            self.text(
                &self.task_right_text(t, now),
                &self.f.small_r,
                w - 16.0 - close_w - right_w,
                y + 8.0,
                right_w,
                18.0,
                self.cc(task_color(t), 0.95),
            );
            if finished {
                let hov = self.hit(
                    w - 38.0,
                    y + 6.0,
                    24.0,
                    24.0,
                    Action::DismissTask(t.id.clone()),
                );
                let a = if hov { 1.0 } else { 0.5 };
                let (cx, cyy) = (w - 26.0, cy);
                self.line(
                    (cx - 3.0, cyy - 3.0),
                    (cx + 3.0, cyy + 3.0),
                    self.cc(self.fg(), a),
                    1.5,
                );
                self.line(
                    (cx - 3.0, cyy + 3.0),
                    (cx + 3.0, cyy - 3.0),
                    self.cc(self.fg(), a),
                    1.5,
                );
            }
            if let Some(msg) = &t.attention {
                self.text(
                    msg,
                    &self.f.small,
                    42.0,
                    y + 26.0,
                    w - 42.0 - 22.0,
                    16.0,
                    self.cc(AMBER, 0.95),
                );
                y += 20.0;
            }
            if let (true, Some(line)) = (t.shows_line() && t.attention.is_none(), t.line.as_ref()) {
                self.text(
                    line,
                    &self.f.mono,
                    42.0,
                    y + 26.0,
                    w - 42.0 - 22.0,
                    crate::ui_state::LINE_H,
                    self.cc(GRAY, 0.85),
                );
                y += crate::ui_state::LINE_H;
            }
            if t.running() {
                self.filament(t, 42.0, y + 30.0, w - 42.0 - 20.0, fr.dt, now);
            }
            y += 34.0;
            if t.failed() {
                let lines = t.stderr.len().min(4);
                if lines > 0 {
                    let bh = lines as f32 * 15.0 + 8.0;
                    self.fill_rr(14.0, y, w - 28.0, bh, 8.0, self.cc(self.fg(), 0.055));
                    for (i, l) in t.stderr.iter().rev().take(lines).rev().enumerate() {
                        self.text(
                            l,
                            &self.f.mono,
                            22.0,
                            y + 4.0 + i as f32 * 15.0,
                            w - 44.0,
                            15.0,
                            self.cc(RED, 0.9),
                        );
                    }
                    y += bh + 6.0;
                }
                self.button(
                    "Copy error",
                    14.0,
                    y,
                    78.0,
                    24.0,
                    RED,
                    Action::CopyText(t.stderr.join("\n")),
                );
                self.button(
                    "Dismiss",
                    100.0,
                    y,
                    66.0,
                    24.0,
                    self.fg(),
                    Action::DismissTask(t.id.clone()),
                );
                y += 28.0;
            }
        }
        if !fr.model.ports.is_empty() {
            self.servers(fr, w, y);
        }
    }

    /// The dev servers listening right now, under the tasks: the port, what runs it, and
    /// the terminal task that started it when there is one.
    fn servers(&self, fr: &Frame, w: f32, y0: f32) {
        let m = fr.model;
        let mut y = y0 + 4.0;
        if !m.rows().is_empty() {
            self.line((16.0, y), (w - 16.0, y), self.cc(self.fg(), 0.07), 1.0);
        }
        self.text(
            "SERVERS",
            &self.f.small_l,
            18.0,
            y + 4.0,
            120.0,
            16.0,
            self.cc(GRAY, 0.75),
        );
        y += 22.0;
        let row = crate::ui_state::SERVER_ROW;
        for p in m.ports.iter().take(5) {
            let cy = y + row / 2.0;
            let task = m.task_for_port(p);
            // Click the row: the terminal that runs it (as a task row does).
            let row_hov = match task.and_then(|t| t.pid.map(|pid| (pid, t.id.clone()))) {
                Some((pid, id)) => self.hit(
                    8.0,
                    y + 1.0,
                    w - 16.0,
                    row - 2.0,
                    Action::FocusTerminal(pid, id),
                ),
                None => false,
            };
            if row_hov {
                self.fill_rr(
                    8.0,
                    y + 1.0,
                    w - 16.0,
                    row - 2.0,
                    10.0,
                    self.cc(self.fg(), 0.05),
                );
            }
            // A soft live dot.
            self.circle(24.0, cy, 6.0, self.cc(GREEN, 0.16));
            self.circle(24.0, cy, 3.2, self.cc(GREEN, 1.0));
            self.text(
                &format!(":{}", p.port),
                &self.f.big,
                40.0,
                y + 2.0,
                74.0,
                20.0,
                self.cc(self.fg(), 0.97),
            );
            let exe = p.exe.trim_end_matches(".exe");
            let sub = match task {
                Some(t) => format!("{exe}  ·  {}", t.label),
                None => exe.to_string(),
            };
            self.text(
                &sub,
                &self.f.small,
                41.0,
                y + 19.0,
                w - 41.0 - 130.0,
                14.0,
                self.cc(GRAY, 1.0),
            );
            let armed = m.kill_armed.is_some_and(|(port, _)| port == p.port);
            self.button(
                "Open",
                w - 16.0 - 54.0 - 6.0 - 54.0,
                cy - 12.0,
                54.0,
                24.0,
                BLUE,
                Action::OpenPort(p.port),
            );
            self.button(
                if armed { "Kill?" } else { "Stop" },
                w - 16.0 - 54.0,
                cy - 12.0,
                54.0,
                24.0,
                RED,
                Action::KillPort(p.port, p.pid),
            );
            y += row;
        }
    }

    fn exp_media(&self, fr: &Frame, w: f32, h: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        let (a, saved) = self.hero(
            [8.0, 6.0, 20.0, 5.0],
            [16.0, 16.0, 72.0, 12.0],
            [16.0, 16.0, 72.0, 12.0],
        );
        self.art_tile(fr.model, a[0], a[1], a[2], a[3]);
        self.hero_end(saved);
        let tx = 102.0;
        let right = w - 16.0;
        // Art + title open the playing app.
        if !md.app.is_empty() {
            let hov = self.hit(16.0, 12.0, 72.0, 76.0, Action::FocusMedia(md.app.clone()))
                | self.hit(
                    tx,
                    12.0,
                    right - tx - 34.0,
                    48.0,
                    Action::FocusMedia(md.app.clone()),
                );
            if hov {
                self.fill_rr(16.0, 16.0, 72.0, 72.0, 12.0, self.cc(self.fg(), 0.10));
            }
        }
        self.text(
            &md.title,
            &self.f.big,
            tx,
            14.0,
            right - tx - 34.0,
            22.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            &md.artist,
            &self.f.body,
            tx,
            37.0,
            right - tx - 34.0,
            18.0,
            self.cc(GRAY, 1.0),
        );
        self.eq_bars(right - 18.0, 18.0, 14.0, md.playing, GREEN);
        // Timeline with live extrapolation while playing.
        let mut pos = md.pos_ms;
        if md.playing {
            if let Some(s) = fr.model.media_stamp {
                pos += fr.now.saturating_duration_since(s).as_millis() as u64;
            }
        }
        if md.dur_ms > 0 {
            pos = pos.min(md.dur_ms);
        }
        let frac = if md.dur_ms > 0 {
            pos as f32 / md.dur_ms as f32
        } else {
            0.0
        };
        let bx = tx;
        let bw = right - tx;
        // The album's own colour when adaptive glow is on, plain ink otherwise.
        let acc = fr
            .model
            .art_accent
            .filter(|_| fr.model.adaptive_glow && !self.light.get())
            .unwrap_or(self.fg());
        self.fill_rr(bx, 65.0, bw, 5.0, 2.5, self.cc(self.fg(), 0.12));
        if md.dur_ms > 0 {
            let fw = (bw * frac).max(5.0);
            self.fill_rr(bx, 65.0, fw, 5.0, 2.5, self.cc(acc, 0.92));
            if md.playing {
                self.circle(bx + fw - 2.5, 67.5, 5.0, self.cc(self.fg(), 1.0));
            }
        }
        self.text(
            &fmt_clock(pos),
            &self.f.small,
            bx,
            73.0,
            50.0,
            14.0,
            self.cc(GRAY, 1.0),
        );
        self.text(
            &fmt_clock(md.dur_ms),
            &self.f.small_r,
            right - 50.0,
            73.0,
            50.0,
            14.0,
            self.cc(GRAY, 1.0),
        );
        // Transport controls, clear of the tab dots.
        let cx = tx + bw / 2.0;
        let cy = h - if fr.model.tabs() { 14.0 } else { 0.0 } - 26.0;
        for (dir, action) in [(-1.0, Action::MediaPrev), (1.0, Action::MediaNext)] {
            let x = cx + dir * 46.0;
            let hov = self.hit(x - 16.0, cy - 16.0, 32.0, 32.0, action);
            if hov {
                self.circle(x, cy, 16.0, self.cc(self.fg(), 0.09));
            }
            self.skip_icon(x, cy, dir, self.cc(self.fg(), if hov { 1.0 } else { 0.8 }));
        }
        let hov = self.hit(cx - 18.0, cy - 18.0, 36.0, 36.0, Action::MediaToggle);
        let down = hov && self.pressed.get();
        let r = if down {
            16.0
        } else if hov {
            18.0
        } else {
            17.0
        };
        self.circle(cx, cy, r, self.cc(self.fg(), 0.95));
        let dark = self.cc(self.ink(), 1.0);
        if md.playing {
            self.pause_icon(cx, cy, 13.0, dark);
        } else {
            self.play_icon(cx + 1.0, cy, 14.0, dark);
        }
    }

    // ------------------------------------------------------- tabs / ports / shelf

    fn tabs(&self, m: &Model, w: f32, h: f32) {
        let ps = m.panels();
        let cur = m.panel();
        let step = 16.0;
        let total = step * ps.len() as f32;
        let mut x = (w - total) / 2.0;
        let cy = h - 8.0;
        for p in ps {
            let hov = self.hit(x, cy - 7.0, step, 14.0, Action::SelectPanel(p));
            let on = p == cur;
            let bw = if on { 12.0 } else { 5.0 };
            let a = if on {
                0.9
            } else if hov {
                0.6
            } else {
                0.28
            };
            self.fill_rr(
                x + (step - bw) / 2.0,
                cy - 2.0,
                bw,
                4.0,
                2.0,
                self.cc(self.fg(), a),
            );
            x += step;
        }
    }

    fn exp_shelf(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let thumbs = self.thumbs.borrow();
        // The row of tiles is centred (70 px a tile, 60 of it the tile itself).
        let x0 = (w - (m.shelf.len().min(5) as f32 * 70.0 - 10.0)) / 2.0;
        for (i, it) in m.shelf.iter().take(5).enumerate() {
            let (x, y, s) = (x0 + i as f32 * 70.0, 14.0, 60.0);
            let sel = m.shelf_sel == Some(it.id);
            let hov = self.hit(x, y, s, s, Action::ShelfTile(it.id));
            self.fill_rr(
                x,
                y,
                s,
                s,
                11.0,
                self.cc(
                    if sel { PURPLE } else { self.fg() },
                    if sel {
                        0.22
                    } else if hov {
                        0.11
                    } else {
                        0.07
                    },
                ),
            );
            if sel {
                self.stroke_rr(
                    x + 0.5,
                    y + 0.5,
                    s - 1.0,
                    s - 1.0,
                    11.0,
                    self.cc(PURPLE, 0.8),
                    1.2,
                    false,
                );
            }
            if let Some(b) = thumbs.get(&it.id) {
                unsafe {
                    let props = D2D1_BRUSH_PROPERTIES {
                        opacity: self.ca.get(),
                        transform: Matrix3x2 {
                            M11: 48.0 / 64.0,
                            M12: 0.0,
                            M21: 0.0,
                            M22: 48.0 / 64.0,
                            M31: x + 6.0,
                            M32: y + 6.0,
                        },
                    };
                    if let Ok(br) = self.rt.CreateBitmapBrush(b, None, Some(&props)) {
                        let rr = D2D1_ROUNDED_RECT {
                            rect: rect(x + 6.0, y + 6.0, 48.0, 48.0),
                            radiusX: 7.0,
                            radiusY: 7.0,
                        };
                        self.rt.FillRoundedRectangle(&rr, &br);
                    }
                }
            } else if it.is_dir {
                self.fill_rr(x + 14.0, y + 20.0, 32.0, 22.0, 4.0, self.cc(AMBER, 0.85));
                self.fill_rr(x + 14.0, y + 16.0, 14.0, 8.0, 3.0, self.cc(AMBER, 0.85));
            } else {
                self.fill_rr(
                    x + 18.0,
                    y + 12.0,
                    24.0,
                    32.0,
                    4.0,
                    self.cc(self.fg(), 0.82),
                );
                for k in 0..3 {
                    self.fill_rr(
                        x + 22.0,
                        y + 19.0 + k as f32 * 7.0,
                        16.0,
                        2.0,
                        1.0,
                        self.cc([0.2; 3], 0.8),
                    );
                }
            }
            self.text(
                &it.name,
                &self.f.small_c,
                x - 2.0,
                y + s + 3.0,
                s + 4.0,
                14.0,
                self.cc(self.fg(), if sel { 0.95 } else { 0.7 }),
            );
            if hov || sel {
                let h2 = self.hit(
                    x + s - 14.0,
                    y - 5.0,
                    18.0,
                    18.0,
                    Action::RemoveShelf(it.id),
                );
                self.circle(x + s - 5.0, y + 4.0, 7.5, self.cc([0.12; 3], 1.0));
                let a = if h2 { 1.0 } else { 0.7 };
                self.line(
                    (x + s - 8.0, y + 1.0),
                    (x + s - 2.0, y + 7.0),
                    self.cc(self.fg(), a),
                    1.4,
                );
                self.line(
                    (x + s - 8.0, y + 7.0),
                    (x + s - 2.0, y + 1.0),
                    self.cc(self.fg(), a),
                    1.4,
                );
            }
        }
        if m.shelf.len() > 5 {
            self.text(
                &format!("+{}", m.shelf.len() - 5),
                &self.f.small_r,
                w - 44.0,
                4.0,
                34.0,
                14.0,
                self.cc(PURPLE, 1.0),
            );
        }
        if let Some(it) = m
            .shelf_sel
            .and_then(|id| m.shelf.iter().find(|i| i.id == id))
        {
            let chips = crate::transforms::chips_for(&it.path);
            // Chip width from the label length; centred as a row.
            let wid: Vec<f32> = chips
                .iter()
                .map(|(l, _)| 28.0 + l.chars().count() as f32 * 6.0)
                .collect();
            let mut x = (w - wid.iter().sum::<f32>() - 6.0 * (chips.len() - 1) as f32) / 2.0;
            for ((label, op), cw) in chips.iter().zip(&wid) {
                self.button(
                    label,
                    x,
                    98.0,
                    *cw,
                    26.0,
                    PURPLE,
                    Action::ShelfOp(it.id, *op),
                );
                x += cw + 6.0;
            }
        } else {
            self.text(
                "Drag a tile out to drop it anywhere",
                &self.f.small_c,
                0.0,
                96.0,
                w,
                16.0,
                self.cc(GRAY, 0.7),
            );
        }
    }

    /// Quick controls: microphone, sound, keep awake and the timer as tiles, the speaker
    /// volume as a bar under them.
    fn exp_home(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::ui_state::Awake;
        let m = fr.model;
        let now = fr.now;
        let (gap, y, th) = (8.0, 12.0, 62.0);
        let tw = (w - 32.0 - gap * 3.0) / 4.0;
        let x = |i: f32| 16.0 + i * (tw + gap);
        let (mic_icon, mic_sub, mic_on) = if m.mic_muted {
            ('\u{F781}', "Muted", Some(RED))
        } else if m.mic {
            ('\u{E720}', "In use", Some(CAM))
        } else {
            ('\u{E720}', "On", None)
        };
        self.tile(
            x(0.0),
            y,
            tw,
            th,
            mic_icon,
            "Mic",
            mic_sub,
            mic_on,
            Action::ToggleMic,
        );
        let (snd_icon, snd_sub, snd_on) = match m.speaker {
            Some((_, true)) => ('\u{E74F}', "Muted".to_string(), Some(AMBER)),
            Some((v, false)) => (volume_glyph(v), format!("{}%", (v * 100.0).round()), None),
            None => ('\u{E74F}', "No output".to_string(), None),
        };
        self.tile(
            x(1.0),
            y,
            tw,
            th,
            snd_icon,
            "Sound",
            &snd_sub,
            snd_on,
            Action::ToggleSpeaker,
        );
        let awake_sub = match m.awake {
            Awake::Off => "Off".to_string(),
            Awake::On => "On".to_string(),
            Awake::Until(t) => {
                let mins = t.saturating_duration_since(now).as_secs().div_ceil(60);
                format!("{mins} min")
            }
        };
        let awake_on = m.awake.active(now).then_some(ORANGE);
        self.tile(
            x(2.0),
            y,
            tw,
            th,
            '\u{E890}',
            "Awake",
            &awake_sub,
            awake_on,
            Action::CycleAwake,
        );
        let wall = crate::timer::now_ms();
        let (timer_sub, timer_on) = match &m.timer {
            Some(t) => (crate::timer::fmt(t.remaining_ms(wall)), Some(BLUE)),
            None => ("Start".to_string(), None),
        };
        self.tile(
            x(3.0),
            y,
            tw,
            th,
            '\u{E916}',
            "Timer",
            &timer_sub,
            timer_on,
            Action::OpenTimer,
        );

        // Volume bar: click to set, scroll to nudge.
        let (bx, by, bw, bh) = (16.0, y + th + 12.0, w - 32.0, 32.0);
        let hov = self.hit(bx, by, bw, bh, Action::Volume);
        self.fill_rr(
            bx,
            by,
            bw,
            bh,
            bh / 2.0,
            self.cc(self.fg(), if hov { 0.10 } else { 0.06 }),
        );
        match m.speaker {
            Some((vol, muted)) => {
                let fw = (bw * vol).max(bh);
                let (fill, a) = if muted {
                    (GRAY, 0.30)
                } else {
                    (self.fg(), 0.9)
                };
                self.fill_rr(bx, by, fw, bh, bh / 2.0, self.cc(fill, a));
                let icon = if muted { '\u{E74F}' } else { volume_glyph(vol) };
                let on_fill = if muted { self.fg() } else { self.ink() };
                self.glyph(icon, bx + 17.0, by + bh / 2.0, 20.0, self.cc(on_fill, 0.9));
            }
            None => {
                self.text(
                    "No output device",
                    &self.f.small_c,
                    bx,
                    by,
                    bw,
                    bh,
                    self.cc(GRAY, 0.9),
                );
            }
        }
    }

    /// Battery and power mode.
    fn exp_power(&self, fr: &Frame, w: f32, _h: f32) {
        let Some(b) = fr.model.power else { return };
        let low = b.pct <= 20 && !b.plugged;
        let col = if low {
            RED
        } else if b.plugged {
            GREEN
        } else {
            self.fg()
        };
        // A big battery: outline, nub, charge, and a bolt while plugged in.
        let (ix, iy, iw, ih) = (22.0, 22.0, 40.0, 21.0);
        self.stroke_rr(ix, iy, iw, ih, 6.0, self.cc(self.fg(), 0.55), 1.6, false);
        self.fill_rr(
            ix + iw + 2.0,
            iy + 6.5,
            3.0,
            ih - 13.0,
            1.4,
            self.cc(self.fg(), 0.55),
        );
        let fw = ((iw - 6.0) * b.pct as f32 / 100.0).max(3.0);
        self.fill_rr(ix + 3.0, iy + 3.0, fw, ih - 6.0, 3.5, self.cc(col, 0.95));
        if b.plugged {
            let (cx, cy) = (ix + iw / 2.0, iy + ih / 2.0);
            let bolt = [
                (cx + 1.5, cy - 8.0),
                (cx - 4.5, cy + 1.0),
                (cx - 0.5, cy + 1.0),
                (cx - 1.5, cy + 8.0),
                (cx + 4.5, cy - 1.0),
                (cx + 0.5, cy - 1.0),
            ];
            self.poly(&bolt, self.cc(self.ink(), 0.9));
        }
        let pct = format!("{}%", b.pct);
        self.text(
            &pct,
            &self.f.hero,
            80.0,
            10.0,
            120.0,
            34.0,
            self.cc(self.fg(), 0.97),
        );
        let sub = match b.left() {
            Some(l) => format!("{}  ·  {l}", capitalise(&b.flow())),
            None => capitalise(&b.flow()),
        };
        let sc = if low { RED } else { GRAY };
        self.text(
            &sub,
            &self.f.small,
            81.0,
            41.0,
            w - 97.0,
            16.0,
            self.cc(sc, 1.0),
        );

        // Power mode: one segmented control.
        let (sx, sy, sw, sh) = (16.0, 68.0, w - 32.0, 32.0);
        self.fill_rr(sx, sy, sw, sh, sh / 2.0, self.cc(self.fg(), 0.06));
        let seg = sw / 3.0;
        let modes = [
            (Mode::Saver, "Saver", GREEN),
            (Mode::Balanced, "Balanced", BLUE),
            (Mode::Performance, "Performance", ORANGE),
        ];
        for (i, (mode, label, accent)) in modes.into_iter().enumerate() {
            let x = sx + i as f32 * seg;
            let on = b.mode == mode;
            let hov = self.hit(x, sy, seg, sh, Action::SetPowerMode(mode));
            let (ix, iy, iw, ih) = (x + 3.0, sy + 3.0, seg - 6.0, sh - 6.0);
            if on {
                self.fill_rr(ix, iy, iw, ih, ih / 2.0, self.cc(accent, 0.26));
                self.stroke_rr(
                    ix + 0.5,
                    iy + 0.5,
                    iw - 1.0,
                    ih - 1.0,
                    ih / 2.0,
                    self.cc(accent, 0.6),
                    1.0,
                    false,
                );
            } else if hov {
                self.fill_rr(ix, iy, iw, ih, ih / 2.0, self.cc(self.fg(), 0.07));
            }
            let dot_x = x + seg / 2.0 - text_w(label) / 2.0 - 5.0;
            self.circle(
                dot_x,
                sy + sh / 2.0,
                3.0,
                self.cc(accent, if on { 1.0 } else { 0.55 }),
            );
            self.text(
                label,
                &self.f.btn,
                x + 6.0,
                sy,
                seg,
                sh,
                self.cc(self.fg(), if on { 1.0 } else { 0.7 }),
            );
        }
    }

    fn exp_drop(&self, fr: &Frame, w: f32, h: f32) {
        let t = self.t.get();
        let pulse = (t * 3.0).sin() * 0.5 + 0.5;
        self.fill_rr(
            12.0,
            12.0,
            w - 24.0,
            h - 24.0,
            14.0,
            self.cc(PURPLE, 0.06 + 0.05 * pulse),
        );
        self.stroke_rr(
            12.0,
            12.0,
            w - 24.0,
            h - 24.0,
            14.0,
            self.cc(PURPLE, 0.55 + 0.35 * pulse),
            1.6,
            true,
        );
        let bob = (t * 4.0).sin() * 2.5;
        let (cx, cy) = (w / 2.0, 34.0 + bob);
        self.line((cx, cy - 8.0), (cx, cy + 6.0), self.cc(PURPLE, 1.0), 2.2);
        self.line((cx - 6.0, cy), (cx, cy + 7.0), self.cc(PURPLE, 1.0), 2.2);
        self.line((cx + 6.0, cy), (cx, cy + 7.0), self.cc(PURPLE, 1.0), 2.2);
        self.text(
            "Drop to add to your shelf",
            &self.f.big_c,
            0.0,
            50.0,
            w,
            22.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            "Drag it back out anywhere later",
            &self.f.small_c,
            0.0,
            74.0,
            w,
            16.0,
            self.cc(GRAY, 1.0),
        );
        let _ = fr;
    }

    fn exp_chip(&self, fr: &Frame, w: f32, _h: f32) {
        let Some(c) = fr.model.chip.as_ref() else {
            return;
        };
        let accent = match c.tone {
            Tone::Done => PURPLE,
            Tone::Warn => AMBER,
            Tone::Info => BLUE,
        };
        self.circle(26.0, 24.0, 9.0, self.cc(accent, 0.2));
        match c.tone {
            Tone::Done => {
                self.line((22.2, 24.4), (25.0, 27.4), self.cc(accent, 1.0), 2.0);
                self.line((25.0, 27.4), (30.2, 21.2), self.cc(accent, 1.0), 2.0);
            }
            Tone::Warn => {
                self.line((26.0, 19.0), (26.0, 25.0), self.cc(accent, 1.0), 2.0);
                self.circle(26.0, 28.5, 1.2, self.cc(accent, 1.0));
            }
            Tone::Info => {
                self.circle(26.0, 19.8, 1.2, self.cc(accent, 1.0));
                self.line((26.0, 23.0), (26.0, 29.0), self.cc(accent, 1.0), 2.0);
            }
        }
        self.text(
            &c.summary,
            &self.f.wrap,
            46.0,
            10.0,
            w - 46.0 - 20.0,
            40.0,
            self.cc(self.fg(), 0.95),
        );
        let by = if c.one_line() { 44.0 } else { 60.0 };
        let mut x = 16.0;
        if let Some(p) = &c.open {
            self.button(
                "Open",
                x,
                by,
                62.0,
                24.0,
                PURPLE,
                Action::OpenPath(p.clone()),
            );
            x += 70.0;
        }
        if let Some(t) = &c.copy {
            self.button(
                "Copy",
                x,
                by,
                62.0,
                24.0,
                PURPLE,
                Action::CopyText(t.clone()),
            );
            x += 70.0;
        }
        for (label, act) in &c.extra {
            let armed = matches!(act, ChipAction::KillPid(_)) && fr.model.chip_armed.is_some();
            let label = if armed { "Kill?" } else { label.as_str() };
            let bw = (label.chars().count() as f32 * 6.6 + 26.0).max(62.0);
            let col = match act {
                ChipAction::KillPid(_) | ChipAction::Answer(_, false) => RED,
                ChipAction::Answer(_, true) => GREEN,
                _ => accent,
            };
            self.button(label, x, by, bw, 24.0, col, Action::Chip(act.clone()));
            x += bw + 8.0;
        }
        self.button("Dismiss", x, by, 70.0, 24.0, self.fg(), Action::DismissChip);
    }

    /// The card shown for a moment after the charger is plugged in (a battery that fills up)
    /// or unplugged (one that drains to its level, red when low).
    fn exp_charge(&self, fr: &Frame, w: f32, h: f32) {
        let m = fr.model;
        let Some(b) = m.power else { return };
        let since = m
            .charge_since
            .map_or(1.0, |s| fr.now.saturating_duration_since(s).as_secs_f32());
        let k = (since / 0.7).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - k).powi(3);
        let plugged = m.charge_in;
        let tone = match (plugged, b.pct) {
            (true, _) => GREEN,
            (false, 0..=20) => RED,
            (false, _) => AMBER,
        };
        let cy = h / 2.0;
        let (bx, bw, bh) = (20.0, 30.0, 15.0);
        self.stroke_rr(
            bx,
            cy - bh / 2.0,
            bw,
            bh,
            4.0,
            self.cc(tone, 0.9),
            1.4,
            false,
        );
        self.fill_rr(bx + bw + 1.5, cy - 3.0, 2.5, 6.0, 1.2, self.cc(tone, 0.9));
        let level = b.pct as f32 / 100.0;
        // Plugging in fills up from empty; unplugging drains down from full.
        let shown = if plugged {
            level * ease
        } else {
            1.0 + (level - 1.0) * ease
        };
        let fill = (bw - 5.0) * shown;
        if fill > 0.5 {
            self.fill_rr(
                bx + 2.5,
                cy - bh / 2.0 + 2.5,
                fill,
                bh - 5.0,
                2.0,
                self.cc(tone, 0.9),
            );
        }
        let (bolt_x, bolt_a) = (bx + bw / 2.0, 0.55 + 0.45 * ease);
        let bolt = [
            (1.6, -5.5),
            (-3.6, 0.6),
            (-0.3, 0.6),
            (-1.6, 5.5),
            (3.6, -0.8),
            (0.3, -0.8),
        ];
        let pts: Vec<(f32, f32)> = bolt.iter().map(|(x, y)| (bolt_x + x, cy + y)).collect();
        if plugged {
            self.poly(&pts, self.cc([1.0; 3], bolt_a));
        }
        self.text(
            if plugged { "Plugged in" } else { "Unplugged" },
            &self.f.title,
            66.0,
            cy - 17.0,
            w - 66.0 - 16.0,
            20.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            &b.status(),
            &self.f.small,
            66.0,
            cy + 1.0,
            w - 66.0 - 16.0,
            16.0,
            self.cc(tone, 1.0),
        );
    }

    /// Hairline fuse along the bottom edge: burns down right to left, ember at the head.
    fn timer_filament(&self, t: &crate::timer::Timer, w: f32, h: f32, centre: bool, lock: f32) {
        let wall = crate::timer::now_ms();
        let frac = t.frac(wall);
        let col = if t.remaining_ms(wall) < 60_000 {
            RED
        } else if frac <= 0.2 {
            AMBER
        } else {
            BLUE
        };
        // Collapsed and empty: the fuse replaces the idle grey dash, in the middle of the pill.
        let (x0, y, side) = if centre {
            (20.0, h / 2.0, 20.0 + lock)
        } else {
            (14.0, h - 2.0, 14.0)
        };
        let len = (w - x0 - side).max(1.0);
        let head = x0 + len * frac;
        unsafe { self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0)) };
        self.line((x0, y), (x0 + len, y), color(self.fg(), 0.07), 1.5);
        if frac > 0.0 {
            self.line((x0, y), (head, y), color(col, 0.25), 4.0);
            self.line((x0, y), (head, y), color(col, 0.95), 1.5);
            self.circle(head, y, 4.0, color(col, 0.22));
            self.circle(head, y, 1.8, color([1.0, 0.97, 0.9], 1.0));
        }
    }

    fn exp_timer(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::timer::TimerKind;
        let m = fr.model;
        let wall = crate::timer::now_ms();
        if let Some(t) = &m.timer {
            let left = t.remaining_ms(wall);
            let paused = t.paused_ms.is_some();
            let frac = t.frac(wall);
            let col = if paused {
                AMBER
            } else if left < 60_000 {
                RED
            } else if frac <= 0.2 {
                AMBER
            } else {
                BLUE
            };
            // The ring is the countdown and the pause / resume button.
            let (cx, cy, r) = (46.0, 46.0, 27.0);
            let d = 2.0 * r + 8.0;
            let hov = self.hit(cx - r - 4.0, cy - r - 4.0, d, d, Action::TimerPause);
            self.circle(cx, cy, r - 4.0, self.cc(col, if hov { 0.16 } else { 0.07 }));
            self.ring(cx, cy, r, self.cc(self.fg(), 0.09), 5.0);
            self.arc(cx, cy, r, frac, self.cc(col, 0.95), 5.0);
            let ic = self.cc(self.fg(), if hov { 1.0 } else { 0.85 });
            if paused {
                self.play_icon(cx, cy, 15.0, ic);
            } else {
                self.pause_icon(cx, cy, 14.0, ic);
            }
            self.text(
                &crate::timer::fmt(left),
                &self.f.hero,
                88.0,
                14.0,
                150.0,
                34.0,
                self.cc(self.fg(), 0.97),
            );
            let sub = if paused {
                format!("{} · paused", t.kind.name())
            } else {
                format!(
                    "{} · ends {}",
                    t.kind.name(),
                    crate::timer::clock(t.ends_ms)
                )
            };
            let sc = if paused { AMBER } else { GRAY };
            self.text(
                &sub,
                &self.f.small,
                89.0,
                47.0,
                170.0,
                16.0,
                self.cc(sc, 1.0),
            );
            // A focus cycle is a row of dots: filled for finished rounds, lit for the one running.
            if t.kind != TimerKind::Plain {
                let focus = t.kind == TimerKind::Focus;
                let (filled, total) = crate::timer::round_dots(m.focus_done, m.rounds, focus);
                for i in 0..total {
                    let on = i < filled;
                    let dx = 93.0 + i as f32 * 12.0;
                    let live = on && i + 1 == filled && focus;
                    if on {
                        self.circle(dx, 73.0, 3.4, self.cc(if live { BLUE } else { GREEN }, 1.0));
                    } else {
                        self.ring(dx, 73.0, 2.8, self.cc(GRAY, 0.6), 1.2);
                    }
                }
            }
            let fg = self.fg();
            let bx = w - 16.0 - 19.0;
            self.round_btn(bx, cy, 19.0, RED, Action::TimerStop, |x, y, c| {
                self.fill_rr(x - 5.5, y - 5.5, 11.0, 11.0, 2.5, c)
            });
            self.round_btn(bx - 46.0, cy, 19.0, fg, Action::TimerAdd(5), |x, y, c| {
                self.text("+5", &self.f.center, x - 19.0, y - 19.0, 38.0, 38.0, c)
            });
            return;
        }
        // Setup: length, presets, start.
        self.text(
            &format!("{} min", m.timer_min),
            &self.f.hero,
            20.0,
            10.0,
            200.0,
            36.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            "Press − / + or scroll to change the minutes",
            &self.f.small,
            21.0,
            44.0,
            w - 21.0 - 104.0,
            14.0,
            self.cc(GRAY, 0.9),
        );
        let fg = self.fg();
        let bx = w - 16.0 - 18.0;
        self.round_btn(bx, 30.0, 18.0, fg, Action::TimerNudge(1), |x, y, c| {
            self.line((x - 6.0, y), (x + 6.0, y), c, 2.0);
            self.line((x, y - 6.0), (x, y + 6.0), c, 2.0);
        });
        self.round_btn(
            bx - 44.0,
            30.0,
            18.0,
            fg,
            Action::TimerNudge(-1),
            |x, y, c| {
                self.line((x - 6.0, y), (x + 6.0, y), c, 2.0);
            },
        );
        let presets = [5u32, 10, 25, 45, 60];
        let (gap, py, ph) = (6.0, 68.0, 28.0);
        let pw = (w - 32.0 - gap * (presets.len() - 1) as f32) / presets.len() as f32;
        for (i, p) in presets.into_iter().enumerate() {
            let x = 16.0 + i as f32 * (pw + gap);
            let on = p == m.timer_min;
            let hov = self.hit(x, py, pw, ph, Action::TimerMinutes(p));
            let a = if on {
                0.28
            } else if hov {
                0.12
            } else {
                0.06
            };
            self.fill_rr(
                x,
                py,
                pw,
                ph,
                ph / 2.0,
                self.cc(if on { BLUE } else { fg }, a),
            );
            let tc = self.cc(fg, if on { 1.0 } else { 0.75 });
            self.text(&format!("{p} min"), &self.f.btn, x, py, pw, ph, tc);
        }
        let bw = (w - 32.0 - 8.0) / 2.0;
        let start = |k| Action::TimerStart(k);
        self.primary(
            "Start timer",
            16.0,
            106.0,
            bw,
            32.0,
            BLUE,
            start(TimerKind::Plain),
        );
        self.primary(
            "Start focus",
            24.0 + bw,
            106.0,
            bw,
            32.0,
            GREEN,
            start(TimerKind::Focus),
        );
        let close = "Right-click the pill to close";
        self.text(
            close,
            &self.f.small,
            ((w - text_w(close)) / 2.0).max(16.0),
            144.0,
            text_w(close) + 12.0,
            14.0,
            self.cc(GRAY, 0.75),
        );
    }

    fn exp_timer_done(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::timer::TimerKind::*;
        let m = fr.model;
        let Some((kind, _)) = m.timer_done else {
            return;
        };
        // A bell in a halo; while the alarm rings it swings and sends out ripples.
        let (cx, cy) = (40.0, 38.0);
        let ringing = fr.ringing;
        let t = self.t.get();
        if ringing {
            for k in 0..2 {
                let ph = (t * 0.8 + k as f32 * 0.5).fract();
                self.ring(
                    cx,
                    cy,
                    21.0 + ph * 12.0,
                    self.cc(GREEN, (1.0 - ph) * 0.35),
                    1.5,
                );
            }
        }
        self.circle(cx, cy, 21.0, self.cc(GREEN, 0.18));
        // The alarm phrase is ~2.6 s: swing during the notes, rest in the pause.
        let phrase = (t % 2.64) / 1.5;
        let swing = if ringing && phrase < 1.0 {
            (t * 22.0).sin() * 14.0 * (1.0 - phrase)
        } else {
            0.0
        };
        let saved = self.rotate(swing, cx, cy - 9.0);
        self.glyph_l('\u{EA8F}', cx, cy, self.cc(GREEN, 1.0));
        unsafe { self.rt.SetTransform(&saved) };
        let title = match kind {
            Plain => "Time's up",
            Focus => "Focus session done",
            Break => "Break is over",
        };
        self.text(
            title,
            &self.f.big,
            74.0,
            16.0,
            w - 90.0,
            22.0,
            self.cc(self.fg(), 0.97),
        );
        if let Some(last) = &m.timer_last {
            let mins = last.total_ms / 60_000;
            let what = kind.name().to_lowercase();
            let ended = crate::timer::clock(last.ends_ms);
            let sub = format!("{mins} min {what}  ·  ended {ended}");
            self.text(
                &sub,
                &self.f.small,
                75.0,
                39.0,
                w - 90.0,
                16.0,
                self.cc(GRAY, 1.0),
            );
        }
        let (y, h) = (70.0, 30.0);
        let mut x = 16.0;
        match kind {
            Focus => {
                let label = format!("Start {} min break", m.timer_break);
                let bw = 30.0 + 6.2 * label.len() as f32;
                self.primary(
                    &label,
                    x,
                    y,
                    bw,
                    h,
                    GREEN,
                    Action::TimerBreak(m.timer_break),
                );
                x += bw + 8.0;
            }
            Break => {
                self.primary(
                    "Start focus",
                    x,
                    y,
                    104.0,
                    h,
                    GREEN,
                    Action::TimerStart(Focus),
                );
                x += 112.0;
            }
            Plain => {}
        }
        self.button("+5 min", x, y, 70.0, h, BLUE, Action::TimerAdd(5));
        let dismiss = if ringing { "Stop alarm" } else { "Dismiss" };
        self.button(
            dismiss,
            w - 16.0 - 88.0,
            y,
            88.0,
            h,
            self.fg(),
            Action::TimerDismiss,
        );
    }

    // -------------------------------------------------------------- album art

    /// Shelf item ids that still need a thumbnail (and forget stale ones).
    pub fn missing_thumbs(&self, m: &Model) -> Vec<u64> {
        let mut t = self.thumbs.borrow_mut();
        t.retain(|id, _| m.shelf.iter().any(|i| i.id == *id));
        m.shelf
            .iter()
            .take(5)
            .filter(|i| !t.contains_key(&i.id))
            .map(|i| i.id)
            .collect()
    }

    pub fn set_thumb(&self, id: u64, a: &crate::ui_state::ArtBitmap) {
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            if let Ok(b) = self.rt.CreateBitmap(
                D2D_SIZE_U {
                    width: a.w,
                    height: a.h,
                },
                Some(a.bgra.as_ptr() as *const _),
                a.w * 4,
                &props,
            ) {
                self.thumbs.borrow_mut().insert(id, b);
            }
        }
    }

    fn sync_art(&self, m: &Model) {
        if self.art_gen.get() == m.art_gen {
            return;
        }
        self.art_gen.set(m.art_gen);
        let mut slot = self.art.borrow_mut();
        *slot = None;
        let Some(a) = &m.art else { return };
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            if let Ok(b) = self.rt.CreateBitmap(
                D2D_SIZE_U {
                    width: a.w,
                    height: a.h,
                },
                Some(a.bgra.as_ptr() as *const _),
                a.w * 4,
                &props,
            ) {
                *slot = Some(b);
            }
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            if !self.bmp.is_invalid() {
                SelectObject(self.mem, self.old);
                let _ = DeleteObject(self.bmp.into());
            }
            let _ = DeleteDC(self.mem);
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
            let small_c = mk(ui, 10.5, DWRITE_FONT_WEIGHT_NORMAL)?;
            small_c.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            // Windows 11 ships Segoe Fluent Icons; the glyphs used here are also in its
            // predecessor, Segoe MDL2 Assets, which the font fallback covers on older builds.
            let icon = mk(w!("Segoe Fluent Icons"), 16.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            let icon_l = mk(w!("Segoe Fluent Icons"), 20.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            icon_l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
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
                small_c,
                icon,
                icon_l,
                hero,
            })
        }
    }
}

/// Offscreen pictures of the expanded scenes, for eyeballing UI changes without running the
/// pill: `HYTTE_SNAP_DIR=<dir> cargo test snapshots -- --ignored` writes one BMP per scene,
/// composited over a dark wallpaper grey.
#[cfg(test)]
mod snap {
    use super::*;
    use crate::ui_state::MediaInfo;
    use hytte_proto::ports::PortInfo;

    fn shoot(
        r: &mut Renderer,
        dir: &std::path::Path,
        name: &str,
        m: &Model,
        scene: Scene,
        light: bool,
    ) {
        let mut an = Anim::new(120.0, 24.0);
        an.set_scene(scene, m.size(scene), m.glow(scene));
        for _ in 0..240 {
            an.step(1.0 / 60.0);
        }
        let fr = Frame {
            model: m,
            anim: &an,
            now: Instant::now(),
            dt: 1.0 / 60.0,
            mouse: None,
            acrylic: false,
            pressed: false,
            light,
            focus: None,
            armed: false,
            ringing: true,
        };
        let crop = r.plan(an.rect.w.pos as f32, an.rect.h.pos as f32, false, 0.0, true);
        let mut hits = vec![];
        r.draw(&fr, crop, &mut hits);
        let (w, h) = (crop.w_px, crop.h_px);
        let cw = w;
        // The renderer's DIB is selected into its DC, which GetDIBits can't read: copy it out.
        let px: Vec<u8> = unsafe {
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let dc = CreateCompatibleDC(Some(r.mem));
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(dc, bmp.into());
            BitBlt(dc, 0, 0, w, h, Some(r.mem), 0, 0, SRCCOPY).unwrap();
            let _ = GdiFlush();
            let v = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize).to_vec();
            SelectObject(dc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(dc);
            v
        };
        // Premultiplied BGRA over the wallpaper, written as a bottom-up 24-bit BMP.
        let bg = if light {
            [236.0, 230.0, 226.0]
        } else {
            [58.0, 52.0, 46.0]
        };
        let row = (w * 3 + 3) / 4 * 4;
        let mut out = Vec::new();
        out.extend(b"BM");
        out.extend((54 + row * h).to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(54u32.to_le_bytes());
        out.extend(40u32.to_le_bytes());
        out.extend(w.to_le_bytes());
        out.extend(h.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(24u16.to_le_bytes());
        out.extend([0u8; 24]);
        for y in (0..h).rev() {
            for x in 0..w {
                let i = ((y * cw + x) * 4) as usize;
                let a = px[i + 3] as f32 / 255.0;
                for c in 0..3 {
                    out.push((px[i + c] as f32 + bg[c] * (1.0 - a)).min(255.0) as u8);
                }
            }
            out.extend(std::iter::repeat_n(0u8, (row - w * 3) as usize));
        }
        std::fs::write(dir.join(format!("{name}.bmp")), out).unwrap();
    }

    #[test]
    #[ignore]
    fn snapshots() {
        let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Renderer::new().unwrap();
        r.set_scale(2.0);
        let now = Instant::now();
        let wall = crate::timer::now_ms();

        let mut m = Model::default();
        m.now = Some(now);
        m.timer_min = 25;
        m.rounds = 4;
        m.timer_break = 5;
        for light in [false, true] {
            let sfx = if light { "-light" } else { "" };
            shoot(
                &mut r,
                &dir,
                &format!("timer-setup{sfx}"),
                &m,
                Scene::ExpTimer,
                light,
            );
        }
        let mut t =
            crate::timer::Timer::start(crate::timer::TimerKind::Focus, 25, wall - 9 * 60_000);
        m.timer = Some(t.clone());
        m.focus_done = 1;
        shoot(&mut r, &dir, "timer-running", &m, Scene::ExpTimer, false);
        t.toggle_pause(wall);
        m.timer = Some(t);
        shoot(&mut r, &dir, "timer-paused", &m, Scene::ExpTimer, false);
        m.timer = None;
        m.timer_last = Some(crate::timer::Timer::start(
            crate::timer::TimerKind::Focus,
            25,
            wall - 25 * 60_000,
        ));
        for k in [
            crate::timer::TimerKind::Focus,
            crate::timer::TimerKind::Plain,
        ] {
            m.timer_done = Some((k, now));
            shoot(
                &mut r,
                &dir,
                &format!("timer-done-{k:?}"),
                &m,
                Scene::ExpTimerDone,
                false,
            );
        }
        m.timer_done = None;

        m.media = Some(MediaInfo {
            title: "Midnight City".into(),
            artist: "M83".into(),
            playing: true,
            app: "Spotify.exe".into(),
            pos_ms: 95_000,
            dur_ms: 243_000,
        });
        m.media_stamp = Some(now);
        for light in [false, true] {
            let sfx = if light { "-light" } else { "" };
            shoot(
                &mut r,
                &dir,
                &format!("media{sfx}"),
                &m,
                Scene::ExpMedia,
                light,
            );
        }
        shoot(
            &mut r,
            &dir,
            "media-compact",
            &m,
            Scene::CompactMedia,
            false,
        );
        m.media = None;

        m.power = Some(crate::power::Battery {
            pct: 64,
            plugged: false,
            watts: Some(-7.4),
            mode: Mode::Balanced,
            mins_left: Some(190),
        });
        shoot(&mut r, &dir, "power", &m, Scene::ExpPower, false);
        shoot(&mut r, &dir, "power-light", &m, Scene::ExpPower, true);
        m.power.as_mut().unwrap().plugged = true;
        m.power.as_mut().unwrap().watts = Some(24.5);
        m.power.as_mut().unwrap().mins_left = None;
        shoot(&mut r, &dir, "power-charging", &m, Scene::ExpPower, false);
        m.speaker = Some((0.64, false));
        shoot(&mut r, &dir, "home", &m, Scene::ExpHome, false);
        shoot(&mut r, &dir, "home-light", &m, Scene::ExpHome, true);
        m.mic_muted = true;
        m.speaker = Some((0.92, false));
        m.awake = crate::ui_state::Awake::Until(now + std::time::Duration::from_secs(52 * 60));
        m.timer = Some(crate::timer::Timer::start(
            crate::timer::TimerKind::Plain,
            10,
            wall,
        ));
        shoot(&mut r, &dir, "home-active", &m, Scene::ExpHome, false);
        m.timer = None;
        shoot(&mut r, &dir, "sentinel", &m, Scene::Sentinel, false);

        let mut m = Model::default();
        m.now = Some(now);
        shoot(&mut r, &dir, "idle", &m, Scene::Idle, false);
        let task = |id: &str, label: &str, ev, progress, line: Option<&str>, err: Option<&str>| {
            crate::tasks::TaskUpdate::Upsert(crate::tasks::TaskState {
                task_id: id.into(),
                label: label.into(),
                progress,
                event: ev,
                duration_ms: Some(83_000),
                stderr_tail: err.map(Into::into),
                source: "run".into(),
                pid: None,
                delay_ms: 0,
                message: None,
                exit_code: None,
                line: line.map(Into::into),
                updated: now,
            })
        };
        let t0 = now - std::time::Duration::from_secs(90);
        m.apply_task(
            task(
                "a",
                "cargo build --release",
                TaskEvent::Progress,
                Some(62),
                Some("Compiling hytte v1.1.0"),
                None,
            ),
            t0,
        );
        shoot(&mut r, &dir, "task-compact", &m, Scene::CompactTask, false);
        m.apply_task(
            task(
                "b",
                "npm test",
                TaskEvent::Start,
                None,
                Some("PASS src/app.test.ts"),
                None,
            ),
            t0,
        );
        m.apply_task(
            task(
                "c",
                "pytest -q",
                TaskEvent::Failed,
                None,
                None,
                Some("E   AssertionError: 3 != 4\nFAILED tests/test_math.py::test_add"),
            ),
            t0,
        );
        m.now = Some(now);
        shoot(&mut r, &dir, "tasks", &m, Scene::ExpTasks, false);
        // One pane: a dev server started from a terminal task sits under it, linked.
        let mut m = Model::default();
        m.now = Some(now);
        let mut dev = task(
            "d",
            "npm run dev",
            TaskEvent::Start,
            None,
            Some("ready in 412 ms"),
            None,
        );
        if let crate::tasks::TaskUpdate::Upsert(st) = &mut dev {
            st.pid = Some(700);
        }
        m.apply_task(dev, t0);
        m.ports = vec![
            PortInfo {
                port: 5173,
                pid: 4388,
                exe: "node.exe".into(),
            },
            PortInfo {
                port: 8000,
                pid: 9012,
                exe: "python.exe".into(),
            },
        ];
        m.port_parents = [(4388, vec![4300, 700, 12]), (9012, vec![1])]
            .into_iter()
            .collect();
        m.now = Some(now);
        shoot(&mut r, &dir, "dev", &m, Scene::ExpTasks, false);
        m.tasks.clear();
        shoot(&mut r, &dir, "dev-servers-only", &m, Scene::ExpTasks, false);
        m.ports.clear();
        m.shelf = [
            "report-final.pdf",
            "screenshot 2026-10-19.png",
            "notes.txt",
            "build",
        ]
        .iter()
        .enumerate()
        .map(|(i, n)| crate::shelf::ShelfItem {
            id: i as u64 + 1,
            path: n.into(),
            name: (*n).into(),
            is_dir: *n == "build",
            owned: false,
        })
        .collect();
        shoot(&mut r, &dir, "shelf", &m, Scene::ExpShelf, false);
        m.shelf_sel = Some(2);
        shoot(&mut r, &dir, "shelf-selected", &m, Scene::ExpShelf, false);
        let mut m = Model::default();
        m.now = Some(now);
        let mut c = crate::ui_state::Chip::new("Saved invoice.pdf to Downloads", now);
        c.open = Some("C:\\x".into());
        m.chip = Some(c);
        shoot(&mut r, &dir, "chip", &m, Scene::ExpChip, false);
        m.chip = None;
        m.drop_over = true;
        shoot(&mut r, &dir, "drop", &m, Scene::ExpDrop, false);
        m.drop_over = false;
        m.power = Some(crate::power::Battery {
            pct: 41,
            plugged: true,
            watts: Some(31.0),
            mode: Mode::Balanced,
            mins_left: None,
        });
        m.charge_in = true;
        shoot(&mut r, &dir, "charge", &m, Scene::ExpCharge, false);
    }
}
