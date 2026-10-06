//! Direct2D / DirectWrite renderer.
//!
//! Draws the notch onto a 32-bpp premultiplied DIB through an
//! `ID2D1DCRenderTarget` and pushes it with `UpdateLayeredWindow`, which gives
//! anti-aliased rounded shapes, per-pixel alpha (transparent pixels are
//! click-through), gradients and glow without a DComp/D3D swapchain.
//!
//! All drawing is in logical px; one scale transform maps to device px. The
//! pill hangs from the top-centre of a fixed transparent canvas, and only the
//! pill (plus its soft glow) has non-zero alpha.

use crate::power::Mode;
use crate::ui_state::{Anim, Model, Panel, Scene, TaskView, LOCK_W};
use hytte_proto::TaskEvent;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};
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
}

pub struct Frame<'a> {
    pub model: &'a Model,
    pub anim: &'a Anim,
    pub now: Instant,
    pub dt: f32,
    pub mouse: Option<(f32, f32)>,
    pub acrylic: bool,
    /// A drag is in flight near the top edge: widen the (invisible) hit zone.
    pub armed: bool,
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
}

pub struct Renderer {
    factory: ID2D1Factory,
    rt: ID2D1DCRenderTarget,
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
    body: RefCell<Option<(bool, ID2D1LinearGradientBrush)>>,
    shimmer: RefCell<Option<([f32; 3], ID2D1LinearGradientBrush)>>,
    /// UTF-16 scratch for `text`, reused across calls.
    wide: RefCell<Vec<u16>>,
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

fn fmt_clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Renderer {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
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
            let rt = factory.CreateDCRenderTarget(&props)?;
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
            let mem = CreateCompatibleDC(None);
            Ok(Self {
                factory,
                rt,
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
            })
        }
    }

    /// (Re)allocate the DIB for a new device scale.
    pub fn set_scale(&mut self, scale: f32) {
        let w = (CANVAS_W * scale).ceil() as i32;
        let h = (CANVAS_H * scale).ceil() as i32;
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
    pub fn plan(&self, pw: f32, ph: f32, armed: bool) -> Crop {
        const MARGIN: f32 = 28.0;
        let (mut w, mut h) = (pw + 2.0 * MARGIN, ph + MARGIN);
        if armed {
            w = w.max(CANVAS_W);
            h = h.max(HOT_ZONE_H);
        }
        let w_px = ((w * self.scale).ceil() as i32).min(self.px.0);
        let h_px = ((h * self.scale).ceil() as i32).min(self.px.1);
        Crop {
            w_px,
            h_px,
            w_log: w_px as f32 / self.scale,
        }
    }

    /// Draw one frame into `hits`: the clickable regions it laid out. The caller's
    /// previous list is recycled as next frame's scratch, so steady state allocates nothing.
    pub fn draw(&mut self, fr: &Frame, crop: Crop, hits: &mut Vec<Hit>) {
        self.hits.borrow_mut().clear();
        self.mouse.set(fr.mouse);
        self.t.set(fr.anim.t as f32);
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
            if self.rt.BindDC(self.mem, &rc).is_err() {
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
            let _ = self.rt.EndDraw(None, None);
        }
        // Prune progress smoothing for tasks that are gone.
        self.shown
            .borrow_mut()
            .retain(|k, _| fr.model.tasks.iter().any(|t| &t.id == k));
        std::mem::swap(hits, &mut *self.hits.borrow_mut());
    }

    /// Push the DIB to the layered window at screen position (x, y).
    pub fn present(&self, hwnd: HWND, crop: Crop, x: i32, y: i32, alpha: u8) {
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
        let mut slot = self.body.borrow_mut();
        if let Some((a, b)) = slot.as_ref() {
            if *a == acrylic {
                return Some(b.clone());
            }
        }
        let alpha = if acrylic { 0.84 } else { 1.0 };
        let stops = [
            D2D1_GRADIENT_STOP {
                position: 0.0,
                color: color([0.027, 0.027, 0.035], alpha),
            },
            D2D1_GRADIENT_STOP {
                position: 1.0,
                color: color([0.075, 0.075, 0.095], alpha),
            },
        ];
        let b = self.linear_brush(&stops)?;
        *slot = Some((acrylic, b.clone()));
        Some(b)
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
        self.hits.borrow_mut().push(Hit {
            rect: (x + ox, y, x + ox + w, y + h),
            action,
        });
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
        let a = if hov { 0.26 } else { 0.13 };
        self.fill_rr(x, y, w, h, h / 2.0, self.cc(accent, a));
        self.text(
            label,
            &self.f.btn,
            x,
            y,
            w,
            h,
            self.cc(WHITE, if hov { 1.0 } else { 0.86 }),
        );
    }

    // ------------------------------------------------------------------ shape

    fn pill_geometry(&self, w: f32, h: f32, rb: f32, e: f32) -> Option<ID2D1PathGeometry> {
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
            self.fill_rr(0.0, 0.0, w, h, h / 2.0, color(WHITE, 0.30));
            return;
        }
        let Some(g) = self.pill_geometry(w, h, rb, e) else {
            return;
        };
        unsafe {
            // Soft glow: stacked widening strokes under the body.
            let mut gl = an.glow.pos as f32;
            if fr.model.primary().is_some_and(|t| t.needs_input())
                && matches!(an.scene, Scene::CompactTask | Scene::ExpTasks)
            {
                gl *= 0.72 + 0.28 * (self.t.get() * 4.0).sin();
            }
            if gl > 0.01 {
                for i in 1..=9 {
                    let k = 1.0 - i as f32 / 10.0;
                    let a = gl * 0.075 * k * k;
                    self.rt.DrawGeometry(
                        &g,
                        self.solid(color(an.glow_rgb, a)),
                        i as f32 * 2.2,
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
                    self.solid(color([0.03, 0.03, 0.04], alpha)),
                    None::<&ID2D1Brush>,
                ),
            }
            // Hairline rim + hover brighten.
            self.rt
                .DrawGeometry(&g, self.solid(color(WHITE, 0.07)), 1.0, &self.round);
            let hv = an.hover.pos as f32;
            if hv > 0.01 {
                self.rt.FillGeometry(
                    &g,
                    self.solid(color(WHITE, 0.025 * hv)),
                    None::<&ID2D1Brush>,
                );
            }
        }
    }

    // ----------------------------------------------------------------- scenes

    fn scene(&self, fr: &Frame, w: f32, h: f32) {
        self.background(fr, w, h);
        let an = fr.anim;
        if an.scene == Scene::Sentinel {
            return;
        }
        if let Some(t) = fr
            .model
            .timer
            .as_ref()
            .filter(|_| an.scene != Scene::ExpTimerDone)
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
        // The scene being left fades out and drifts up, clipped to the pill as it
        // changes size. It is not interactive: no hover, and its hit areas are dropped.
        let o = an.out.pos.clamp(0.0, 1.0) as f32;
        if o > 0.01 && an.prev != an.scene && an.prev != Scene::Sentinel {
            self.ca.set(o * fit);
            let mouse = self.mouse.take();
            let n = self.hits.borrow().len();
            unsafe {
                self.rt
                    .SetTransform(&mat(self.scale, self.ox.get(), -(1.0 - o) * 4.0));
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
        unsafe {
            self.rt
                .SetTransform(&mat(self.scale, self.ox.get(), (1.0 - c) * 6.0))
        };
        self.content(fr, an.scene, w, h, pad_r);
        // The Home card already spells out mic/camera state.
        let home = an.scene == Scene::ExpHome;
        if m.privacy_active() && !home {
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
            Scene::ExpPorts => self.exp_ports(fr, w, h),
            Scene::ExpShelf => self.exp_shelf(fr, w, h),
            Scene::ExpHome => self.exp_home(fr, w, h),
            Scene::ExpDrop => self.exp_drop(fr, w, h),
            Scene::ExpChip => self.exp_chip(fr, w, h),
            Scene::ExpTimer => self.exp_timer(fr, w, h),
            Scene::ExpTimerDone => self.exp_timer_done(fr, w, h),
        }
        if fr.model.tabs()
            && matches!(
                scene,
                Scene::ExpTasks
                    | Scene::ExpMedia
                    | Scene::ExpPorts
                    | Scene::ExpShelf
                    | Scene::ExpHome
                    | Scene::ExpTimer
            )
        {
            self.tabs(fr.model, w, h);
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
                self.cc(WHITE, 0.16),
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
                self.cc(WHITE, 0.82),
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
                self.cc(WHITE, 0.82),
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
        self.fill_rr(x, y, w, 2.0, 1.0, self.cc(WHITE, 0.08));
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
        self.status_icon(t, 20.0, h / 2.0 - 1.5, now);
        let mut right = w - 14.0 - pad_r;
        let n = m.visible_count();
        if n > 1 {
            self.fill_rr(
                right - 20.0,
                h / 2.0 - 9.0,
                20.0,
                16.0,
                8.0,
                self.cc(WHITE, 0.14),
            );
            self.text(
                &n.to_string(),
                &self.f.btn,
                right - 20.0,
                h / 2.0 - 9.0,
                20.0,
                16.0,
                self.cc(WHITE, 0.9),
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
        self.text(
            &t.label,
            &self.f.title,
            38.0,
            h / 2.0 - 10.0,
            right - rw - 46.0,
            17.0,
            self.cc(WHITE, 0.94),
        );
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
        self.fill_rr(x, y, s, s, r, self.cc(PURPLE, 0.22));
        self.text("♪", &self.f.center, x, y, s, s, self.cc(WHITE, 0.8));
    }

    fn compact_media(&self, fr: &Frame, w: f32, h: f32, pad_r: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        self.art_tile(fr.model, 8.0, h / 2.0 - 10.0, 20.0, 5.0);
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
            self.cc(WHITE, 0.92),
        );
    }

    fn exp_tasks(&self, fr: &Frame, w: f32, _h: f32) {
        let now = fr.now;
        let mut y = 10.0;
        for t in fr.model.rows() {
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
                self.fill_rr(8.0, y + 2.0, w - 16.0, 32.0, 10.0, self.cc(WHITE, 0.05));
            }
            self.status_icon(t, 24.0, cy, now);
            self.text(
                &t.label,
                &self.f.title,
                42.0,
                y + 7.0,
                label_w,
                20.0,
                self.cc(WHITE, 0.95),
            );
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
                    self.cc(WHITE, a),
                    1.5,
                );
                self.line(
                    (cx - 3.0, cyy + 3.0),
                    (cx + 3.0, cyy - 3.0),
                    self.cc(WHITE, a),
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
            if t.running() {
                self.filament(t, 42.0, y + 30.0, w - 42.0 - 20.0, fr.dt, now);
            }
            y += 34.0;
            if t.failed() {
                let lines = t.stderr.len().min(4);
                if lines > 0 {
                    let bh = lines as f32 * 15.0 + 8.0;
                    self.fill_rr(14.0, y, w - 28.0, bh, 8.0, self.cc(WHITE, 0.055));
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
                    WHITE,
                    Action::DismissTask(t.id.clone()),
                );
                y += 28.0;
            }
        }
    }

    fn exp_media(&self, fr: &Frame, w: f32, h: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        self.art_tile(fr.model, 16.0, 16.0, 72.0, 12.0);
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
                self.fill_rr(16.0, 16.0, 72.0, 72.0, 12.0, self.cc(WHITE, 0.10));
            }
        }
        self.text(
            &md.title,
            &self.f.big,
            tx,
            14.0,
            right - tx - 34.0,
            22.0,
            self.cc(WHITE, 0.97),
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
        self.fill_rr(bx, 66.0, bw, 4.0, 2.0, self.cc(WHITE, 0.14));
        self.fill_rr(
            bx,
            66.0,
            (bw * frac).max(if md.dur_ms > 0 { 3.0 } else { 0.0 }),
            4.0,
            2.0,
            self.cc(WHITE, 0.92),
        );
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
        // Transport controls.
        let cx = tx + bw / 2.0;
        let cy = h - 26.0;
        let hov = self.hit(cx - 56.0, cy - 14.0, 28.0, 28.0, Action::MediaPrev);
        let a = if hov { 1.0 } else { 0.78 };
        self.poly(
            &[
                (cx - 36.0, cy - 6.0),
                (cx - 44.0, cy),
                (cx - 36.0, cy + 6.0),
            ],
            self.cc(WHITE, a),
        );
        self.poly(
            &[
                (cx - 44.0, cy - 6.0),
                (cx - 52.0, cy),
                (cx - 44.0, cy + 6.0),
            ],
            self.cc(WHITE, a),
        );
        let hov = self.hit(cx + 28.0, cy - 14.0, 28.0, 28.0, Action::MediaNext);
        let a = if hov { 1.0 } else { 0.78 };
        self.poly(
            &[
                (cx + 36.0, cy - 6.0),
                (cx + 44.0, cy),
                (cx + 36.0, cy + 6.0),
            ],
            self.cc(WHITE, a),
        );
        self.poly(
            &[
                (cx + 44.0, cy - 6.0),
                (cx + 52.0, cy),
                (cx + 44.0, cy + 6.0),
            ],
            self.cc(WHITE, a),
        );
        let hov = self.hit(cx - 16.0, cy - 16.0, 32.0, 32.0, Action::MediaToggle);
        self.circle(cx, cy, if hov { 16.0 } else { 15.0 }, self.cc(WHITE, 0.95));
        let dark = self.cc([0.04, 0.04, 0.05], 1.0);
        if md.playing {
            self.fill_rr(cx - 5.0, cy - 6.0, 3.6, 12.0, 1.2, dark);
            self.fill_rr(cx + 1.4, cy - 6.0, 3.6, 12.0, 1.2, dark);
        } else {
            self.poly(
                &[(cx - 3.5, cy - 7.0), (cx + 6.5, cy), (cx - 3.5, cy + 7.0)],
                dark,
            );
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
                self.cc(WHITE, a),
            );
            x += step;
        }
    }

    fn exp_ports(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let mut y = 10.0;
        for p in m.ports.iter().take(5) {
            let cy = y + 17.0;
            self.circle(24.0, cy, 3.2, self.cc(GREEN, 1.0));
            self.text(
                &format!(":{}", p.port),
                &self.f.big,
                36.0,
                y + 5.0,
                70.0,
                24.0,
                self.cc(WHITE, 0.97),
            );
            self.text(
                &p.exe,
                &self.f.body,
                106.0,
                y + 8.0,
                w - 106.0 - 140.0,
                18.0,
                self.cc(GRAY, 1.0),
            );
            let armed = m.kill_armed.is_some_and(|(port, _)| port == p.port);
            self.button(
                "Open",
                w - 16.0 - 56.0 - 8.0 - 56.0,
                y + 5.0,
                56.0,
                24.0,
                BLUE,
                Action::OpenPort(p.port),
            );
            self.button(
                if armed { "Kill?" } else { "Kill" },
                w - 16.0 - 56.0,
                y + 5.0,
                56.0,
                24.0,
                RED,
                Action::KillPort(p.port, p.pid),
            );
            y += 34.0;
        }
    }

    fn exp_shelf(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let thumbs = self.thumbs.borrow();
        for (i, it) in m.shelf.iter().take(5).enumerate() {
            let (x, y, s) = (20.0 + i as f32 * 70.0, 14.0, 60.0);
            let sel = m.shelf_sel == Some(it.id);
            let hov = self.hit(x, y, s, s, Action::ShelfTile(it.id));
            self.fill_rr(
                x,
                y,
                s,
                s,
                11.0,
                self.cc(
                    if sel { PURPLE } else { WHITE },
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
                self.fill_rr(x + 18.0, y + 12.0, 24.0, 32.0, 4.0, self.cc(WHITE, 0.82));
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
                x - 5.0,
                y + s + 2.0,
                s + 10.0,
                14.0,
                self.cc(WHITE, 0.85),
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
                    self.cc(WHITE, a),
                    1.4,
                );
                self.line(
                    (x + s - 8.0, y + 7.0),
                    (x + s - 2.0, y + 1.0),
                    self.cc(WHITE, a),
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
                    94.0,
                    *cw,
                    22.0,
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

    fn exp_home(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        // Microphone row.
        let muted = m.mic_muted;
        let col = if muted { RED } else { GRAY };
        self.fill_rr(
            12.0,
            8.0,
            w - 24.0,
            38.0,
            12.0,
            self.cc(col, if muted { 0.16 } else { 0.06 }),
        );
        if muted {
            self.lock_icon(32.0, 27.0, RED, 1.0);
        } else {
            self.fill_rr(28.0, 17.0, 8.0, 13.0, 4.0, self.cc(WHITE, 0.85));
            self.stroke_rr(
                25.0,
                21.0,
                14.0,
                11.0,
                7.0,
                self.cc(WHITE, 0.85),
                1.4,
                false,
            );
            self.line((32.0, 33.0), (32.0, 36.0), self.cc(WHITE, 0.85), 1.4);
        }
        let sub = if muted {
            "Muted · apps hear nothing".to_string()
        } else if m.cam || m.mic {
            let what = match (m.cam, m.mic) {
                (true, true) => "Camera + microphone",
                (true, false) => "Camera",
                _ => "Microphone",
            };
            match &m.privacy_app {
                Some(a) => format!("{what} in use · {a}"),
                None => format!("{what} in use"),
            }
        } else {
            "Live · click Mute to silence every mic".to_string()
        };
        self.text(
            "Microphone",
            &self.f.title,
            52.0,
            10.0,
            w - 52.0 - 100.0,
            20.0,
            self.cc(WHITE, 0.97),
        );
        self.text(
            &sub,
            &self.f.small,
            52.0,
            27.0,
            w - 52.0 - 100.0,
            16.0,
            self.cc(if muted { RED } else { GRAY }, 1.0),
        );
        self.button(
            if muted { "Unmute" } else { "Mute" },
            w - 24.0 - 68.0,
            15.0,
            68.0,
            24.0,
            if muted { RED } else { WHITE },
            Action::ToggleMic,
        );
        let mut hint_y = 56.0;
        // Battery row (laptops only).
        if let Some(b) = &m.power {
            let low = b.pct <= 20 && !b.plugged;
            let bc = if low {
                RED
            } else if b.plugged {
                GREEN
            } else {
                WHITE
            };
            self.stroke_rr(20.5, 59.5, 24.0, 12.0, 3.0, self.cc(WHITE, 0.7), 1.2, false);
            self.fill_rr(45.0, 63.0, 2.5, 5.0, 1.0, self.cc(WHITE, 0.7));
            self.fill_rr(
                22.5,
                61.5,
                (20.0 * b.pct as f32 / 100.0).max(2.0),
                8.0,
                1.8,
                self.cc(bc, 0.95),
            );
            self.text(
                &b.status(),
                &self.f.body,
                58.0,
                56.0,
                w - 58.0 - 16.0,
                20.0,
                self.cc(WHITE, 0.92),
            );
            let modes = [
                (Mode::Saver, "Saver"),
                (Mode::Balanced, "Balanced"),
                (Mode::Performance, "Performance"),
            ];
            let (bw, gap) = (96.0, 6.0);
            let mut x = (w - (bw * 3.0 + gap * 2.0)) / 2.0;
            for (mode, label) in modes {
                let on = b.mode == mode;
                let hov = self.hit(x, 82.0, bw, 24.0, Action::SetPowerMode(mode));
                let accent = match mode {
                    Mode::Saver => GREEN,
                    Mode::Balanced => BLUE,
                    Mode::Performance => ORANGE,
                };
                self.fill_rr(
                    x,
                    82.0,
                    bw,
                    24.0,
                    12.0,
                    self.cc(
                        accent,
                        if on {
                            0.34
                        } else if hov {
                            0.16
                        } else {
                            0.07
                        },
                    ),
                );
                if on {
                    self.stroke_rr(
                        x + 0.5,
                        82.5,
                        bw - 1.0,
                        23.0,
                        11.5,
                        self.cc(accent, 0.8),
                        1.2,
                        false,
                    );
                }
                self.text(
                    label,
                    &self.f.btn,
                    x,
                    82.0,
                    bw,
                    24.0,
                    self.cc(WHITE, if on { 1.0 } else { 0.78 }),
                );
                x += bw + gap;
            }
            hint_y = 110.0;
        }
        self.text(
            "Drop files here  ·  notch run -- <cmd>",
            &self.f.small_c,
            0.0,
            hint_y,
            w,
            16.0,
            self.cc(GRAY, 0.6),
        );
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
            self.cc(WHITE, 0.97),
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
        self.circle(26.0, 24.0, 9.0, self.cc(PURPLE, 0.2));
        self.line((22.2, 24.4), (25.0, 27.4), self.cc(PURPLE, 1.0), 2.0);
        self.line((25.0, 27.4), (30.2, 21.2), self.cc(PURPLE, 1.0), 2.0);
        self.text(
            &c.summary,
            &self.f.wrap,
            46.0,
            10.0,
            w - 46.0 - 20.0,
            40.0,
            self.cc(WHITE, 0.95),
        );
        let mut x = 16.0;
        if let Some(p) = &c.open {
            self.button(
                "Open",
                x,
                60.0,
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
                60.0,
                62.0,
                24.0,
                PURPLE,
                Action::CopyText(t.clone()),
            );
            x += 70.0;
        }
        self.button("Dismiss", x, 60.0, 70.0, 24.0, WHITE, Action::DismissChip);
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
        self.line((x0, y), (x0 + len, y), color(WHITE, 0.07), 1.5);
        if frac > 0.0 {
            self.line((x0, y), (head, y), color(col, 0.25), 4.0);
            self.line((x0, y), (head, y), color(col, 0.95), 1.5);
            self.circle(head, y, 4.0, color(col, 0.22));
            self.circle(head, y, 1.8, color([1.0, 0.97, 0.9], 1.0));
        }
    }

    fn exp_timer(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        if let Some(t) = &m.timer {
            let left = t.remaining_ms(crate::timer::now_ms());
            self.text(
                &crate::timer::fmt(left),
                &self.f.big,
                20.0,
                10.0,
                140.0,
                28.0,
                self.cc(WHITE, 1.0),
            );
            self.text(
                &if t.paused_ms.is_some() {
                    format!("{} · paused", t.kind.name())
                } else {
                    t.kind.name().to_string()
                },
                &self.f.small_r,
                w - 190.0,
                14.0,
                150.0,
                20.0,
                self.cc(GRAY, 1.0),
            );
            let pause = if t.paused_ms.is_some() {
                "Resume"
            } else {
                "Pause"
            };
            self.button(pause, 16.0, 50.0, 70.0, 26.0, AMBER, Action::TimerPause);
            self.button("+5 min", 92.0, 50.0, 66.0, 26.0, BLUE, Action::TimerAdd(5));
            self.button("Stop", 164.0, 50.0, 56.0, 26.0, RED, Action::TimerStop);
            return;
        }
        self.text(
            &format!("{} min", m.timer_min),
            &self.f.big,
            20.0,
            10.0,
            140.0,
            28.0,
            self.cc(WHITE, 1.0),
        );
        self.text(
            "Scroll to adjust",
            &self.f.small_r,
            w - 190.0,
            14.0,
            150.0,
            20.0,
            self.cc(GRAY, 1.0),
        );
        let mut x = 16.0;
        for p in [5u32, 10, 15, 30, 45] {
            let accent = if p == m.timer_min { BLUE } else { WHITE };
            self.button(
                &p.to_string(),
                x,
                50.0,
                36.0,
                26.0,
                accent,
                Action::TimerMinutes(p),
            );
            x += 40.0;
        }
        self.button(
            "Timer",
            w - 16.0 - 62.0 - 6.0 - 62.0,
            50.0,
            62.0,
            26.0,
            BLUE,
            Action::TimerStart(crate::timer::TimerKind::Plain),
        );
        self.button(
            "Focus",
            w - 16.0 - 62.0,
            50.0,
            62.0,
            26.0,
            GREEN,
            Action::TimerStart(crate::timer::TimerKind::Focus),
        );
    }

    fn exp_timer_done(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::timer::TimerKind::*;
        let m = fr.model;
        let Some((kind, _)) = m.timer_done else {
            return;
        };
        self.circle(26.0, 24.0, 9.0, self.cc(GREEN, 0.2));
        self.line((22.2, 24.4), (25.0, 27.4), self.cc(GREEN, 1.0), 2.0);
        self.line((25.0, 27.4), (30.2, 21.2), self.cc(GREEN, 1.0), 2.0);
        let title = match kind {
            Plain => "Timer finished",
            Focus => "Focus session finished",
            Break => "Break is over",
        };
        self.text(
            title,
            &self.f.big,
            46.0,
            10.0,
            w - 60.0,
            28.0,
            self.cc(WHITE, 0.97),
        );
        let mut x = 16.0;
        match kind {
            Focus => {
                let label = format!("Start {} min break", m.timer_break);
                let bw = 28.0 + 6.0 * label.len() as f32;
                self.button(
                    &label,
                    x,
                    56.0,
                    bw,
                    26.0,
                    GREEN,
                    Action::TimerBreak(m.timer_break),
                );
                x += bw + 8.0;
            }
            Break => {
                self.button(
                    "Start focus",
                    x,
                    56.0,
                    92.0,
                    26.0,
                    GREEN,
                    Action::TimerStart(Focus),
                );
                x += 100.0;
            }
            Plain => {}
        }
        self.button("+5 min", x, 56.0, 64.0, 26.0, BLUE, Action::TimerAdd(5));
        self.button(
            "Dismiss",
            x + 72.0,
            56.0,
            70.0,
            26.0,
            WHITE,
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
            })
        }
    }
}
