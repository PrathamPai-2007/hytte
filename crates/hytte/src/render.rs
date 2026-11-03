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
    Anim, Bubble, ChipAction, Model, Panel, Scene, TaskView, Tone, LOCK_W, TAB_H,
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
pub const CANVAS_W: f32 = 460.0;
pub const CANVAS_H: f32 = 360.0;
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
    /// A section in the settings window's list: scroll to it.
    Jump(usize),
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
    /// `[general] top_offset`: logical px between the top of the window and the pill. The
    /// strip is part of the window, so pointing at it still reaches the pill.
    gap: Cell<f32>,
    /// Settings window: animate (`ease`), the frame's time step, and whether a value is
    /// still on its way (another frame is due).
    fx: Cell<bool>,
    fx_dt: Cell<f32>,
    fx_busy: Cell<bool>,
    fx_vals: RefCell<HashMap<String, f32>>,
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

type Choice = (&'static str, Val);

/// One line of settings: a switch, a few steps, or the opacity slider.
pub enum Row {
    Switch(&'static str, &'static str, bool),
    Pick(&'static str, &'static str, Vec<Choice>, usize),
    /// Steps plus a last "Custom" segment for a value only `config.toml` can hold: the
    /// segment opens the file with a note (`Action::EditConfig(hint)`). `String` is what the
    /// file holds when it is none of the steps (shown beside the label), empty otherwise.
    PickCustom(
        &'static str,
        &'static str,
        Vec<Choice>,
        usize,
        &'static str,
        String,
    ),
    /// Pill opacity slider, current value.
    Opacity(f32),
}

/// A titled run of rows. Each row also says which page of the pill's small settings pane
/// it appears on (-1: only in the big settings window).
pub struct Section {
    pub name: &'static str,
    pub rows: Vec<(Row, i8)>,
}

fn pick_s(
    label: &'static str,
    key: &'static str,
    opts: &[(&'static str, &'static str)],
    cur: &str,
) -> Row {
    let i = opts.iter().position(|(_, v)| *v == cur).unwrap_or(0);
    Row::Pick(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::S(v))).collect(),
        i,
    )
}

/// `pick_s` with a trailing "Custom" segment (see `Row::PickCustom`).
fn pick_s_custom(
    label: &'static str,
    key: &'static str,
    opts: &[(&'static str, &'static str)],
    cur: &str,
    hint: &'static str,
) -> Row {
    let found = opts.iter().position(|(_, v)| *v == cur);
    let custom = if found.is_none() {
        cur.to_string()
    } else {
        String::new()
    };
    Row::PickCustom(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::S(v))).collect(),
        found.unwrap_or(opts.len()),
        hint,
        custom,
    )
}

/// Whole-number steps; the one nearest to what the file says is lit.
fn pick_i(label: &'static str, key: &'static str, opts: &[(&'static str, i64)], cur: i64) -> Row {
    let i = (0..opts.len())
        .min_by_key(|&i| (opts[i].1 - cur).abs())
        .unwrap_or(0);
    Row::Pick(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::I(*v))).collect(),
        i,
    )
}

fn pick_f(label: &'static str, key: &'static str, opts: &[(&'static str, f64)], cur: f64) -> Row {
    let d = |i: usize| (opts[i].1 - cur).abs();
    let i = (0..opts.len())
        .min_by(|&a, &b| d(a).total_cmp(&d(b)))
        .unwrap_or(0);
    Row::Pick(
        label,
        key,
        opts.iter().map(|(n, v)| (*n, Val::F(*v))).collect(),
        i,
    )
}

/// Every setting that is a switch, a slider or a short list of steps. Lists and text
/// (allow / deny lists, ports, hotkey, folders) stay in config.toml.
pub fn settings_table(c: &Config) -> Vec<Section> {
    let (g, sh) = (&c.general, &c.shelf);
    let sw = Row::Switch;
    vec![
        Section {
            name: "Look",
            rows: vec![
                (
                    pick_s(
                        "Theme",
                        "general.theme",
                        &[("Dark", "dark"), ("Light", "light"), ("Auto", "auto")],
                        &g.theme,
                    ),
                    0,
                ),
                (
                    pick_i(
                        "Hover delay",
                        "general.hover_delay_ms",
                        &[("Instant", 0), ("0.2s", 200), ("0.5s", 500), ("1s", 1000)],
                        g.hover_delay_ms as i64,
                    ),
                    0,
                ),
                (
                    pick_s(
                        "Open with",
                        "general.open_on",
                        &[("Hover", "hover"), ("Click", "click")],
                        &g.open_on,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Close delay",
                        "general.collapse_delay_ms",
                        &[("Quick", 100), ("0.3s", 300), ("0.6s", 600), ("1s", 1000)],
                        g.collapse_delay_ms as i64,
                    ),
                    -1,
                ),
                (Row::Opacity(g.pill_opacity), 0),
                (
                    pick_f(
                        "Idle opacity",
                        "general.idle_opacity",
                        &[("Full", 1.0), ("High", 0.75), ("Low", 0.5), ("Faint", 0.3)],
                        g.idle_opacity as f64,
                    ),
                    -1,
                ),
                (
                    pick_f(
                        "Size",
                        "general.pill_scale",
                        &[
                            ("Small", 0.85),
                            ("Normal", 1.0),
                            ("Large", 1.2),
                            ("XL", 1.4),
                        ],
                        g.pill_scale as f64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Gap from top",
                        "general.top_offset",
                        &[("None", 0), ("4 px", 4), ("10 px", 10), ("20 px", 20)],
                        g.top_offset as i64,
                    ),
                    -1,
                ),
                (
                    pick_s(
                        "Motion",
                        "general.animation",
                        &[("Calm", "calm"), ("Normal", "normal"), ("Snappy", "snappy")],
                        &g.animation,
                    ),
                    -1,
                ),
                (
                    pick_f(
                        "Glow",
                        "general.glow_strength",
                        &[
                            ("Off", 0.0),
                            ("Soft", 0.5),
                            ("Normal", 1.0),
                            ("Strong", 1.6),
                        ],
                        g.glow_strength as f64,
                    ),
                    0,
                ),
                (
                    sw("Album-art glow", "general.adaptive_glow", g.adaptive_glow),
                    0,
                ),
            ],
        },
        Section {
            name: "Behaviour",
            rows: vec![
                (
                    sw("Start with Windows", "general.autostart", g.autostart),
                    1,
                ),
                (
                    pick_s(
                        "Start on",
                        "general.start_panel",
                        &[("Last", "last"), ("Active", "auto"), ("Home", "home")],
                        &g.start_panel,
                    ),
                    -1,
                ),
                (
                    pick_s_custom(
                        "Quiet hours",
                        "general.quiet_hours",
                        &[
                            ("Off", ""),
                            ("Night", "22:00-07:00"),
                            ("Late", "00:00-08:00"),
                            ("Work", "09:00-17:00"),
                        ],
                        &g.quiet_hours,
                        "general.quiet_hours",
                    ),
                    -1,
                ),
                (
                    pick_s(
                        "Over fullscreen",
                        "general.fullscreen",
                        &[("Line", "line"), ("Hide", "hide"), ("Off", "off")],
                        &g.fullscreen,
                    ),
                    1,
                ),
                (
                    pick_s(
                        "Screen",
                        "general.monitor",
                        &[
                            ("Primary", "primary"),
                            ("Active", "active"),
                            ("Cursor", "cursor"),
                        ],
                        &g.monitor,
                    ),
                    1,
                ),
                (
                    pick_i(
                        "Open on finish",
                        "general.finish_peek_secs",
                        &[("Off", 0), ("3 s", 3), ("5 s", 5), ("10 s", 10)],
                        g.finish_peek_secs as i64,
                    ),
                    1,
                ),
                (
                    sw(
                        "Hide from screen capture",
                        "general.hide_from_capture",
                        g.hide_from_capture,
                    ),
                    -1,
                ),
                (
                    sw("Hytte on PATH", "general.add_to_path", g.add_to_path),
                    -1,
                ),
                (
                    sw("Update checks", "general.check_updates", g.check_updates),
                    2,
                ),
            ],
        },
        Section {
            name: "Timer",
            rows: vec![
                (sw("Timer sound", "timer.sound", c.timer.sound), 1),
                (
                    pick_i(
                        "Break",
                        "timer.break_min",
                        &[("3 min", 3), ("5 min", 5), ("10 min", 10)],
                        c.timer.break_min as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Long break",
                        "timer.long_break_min",
                        &[
                            ("10 min", 10),
                            ("15 min", 15),
                            ("20 min", 20),
                            ("30 min", 30),
                        ],
                        c.timer.long_break_min as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Rounds to long break",
                        "timer.rounds",
                        &[("2", 2), ("3", 3), ("4", 4), ("6", 6)],
                        c.timer.rounds as i64,
                    ),
                    -1,
                ),
            ],
        },
        Section {
            name: "Agents",
            rows: vec![
                (
                    pick_i(
                        "Task rows",
                        "general.task_rows",
                        &[("3", 3), ("4", 4), ("6", 6), ("8", 8)],
                        g.task_rows as i64,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Sound when an agent needs you",
                        "agent.sound",
                        c.agent.sound,
                    ),
                    -1,
                ),
            ],
        },
        Section {
            name: "Shelf",
            rows: vec![
                (
                    pick_s(
                        "Keep files by",
                        "shelf.mode",
                        &[("Reference", "reference"), ("Copy", "copy")],
                        &sh.mode,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Max items",
                        "shelf.max_items",
                        &[("6", 6), ("12", 12), ("24", 24), ("48", 48)],
                        sh.max_items as i64,
                    ),
                    -1,
                ),
                (sw("Remember between runs", "shelf.persist", sh.persist), -1),
                (
                    pick_i(
                        "Screenshots stay",
                        "shelf.screenshot_secs",
                        &[("Keep", 0), ("30 s", 30), ("1 min", 60), ("5 min", 300)],
                        sh.screenshot_secs as i64,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Delete screenshot after paste",
                        "shelf.delete_after_paste",
                        sh.delete_after_paste,
                    ),
                    -1,
                ),
                (
                    sw(
                        "Remove after dragging out",
                        "shelf.remove_after_drag",
                        sh.remove_after_drag,
                    ),
                    -1,
                ),
                (
                    sw("Shelve screenshots", "shelf.screenshots", sh.screenshots),
                    2,
                ),
            ],
        },
        Section {
            name: "Watchers",
            rows: vec![
                (sw("Downloads", "downloads.enabled", c.downloads.enabled), 2),
                (
                    sw("Calendar heads-up", "calendar.enabled", c.calendar.enabled),
                    2,
                ),
                (
                    pick_i(
                        "Heads-up lead",
                        "calendar.lead_min",
                        &[("2 min", 2), ("5 min", 5), ("10 min", 10), ("15 min", 15)],
                        c.calendar.lead_min as i64,
                    ),
                    -1,
                ),
                (sw("Resource alerts", "hog.enabled", c.hog.enabled), 2),
                (
                    pick_i(
                        "Alert above CPU",
                        "hog.cpu_pct",
                        &[("50%", 50), ("70%", 70), ("85%", 85), ("95%", 95)],
                        c.hog.cpu_pct as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Alert above memory",
                        "hog.mem_mb",
                        &[
                            ("0.5 GB", 512),
                            ("1 GB", 1024),
                            ("2 GB", 2048),
                            ("4 GB", 4096),
                        ],
                        c.hog.mem_mb as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "For at least",
                        "hog.secs",
                        &[("10 s", 10), ("30 s", 30), ("60 s", 60), ("2 min", 120)],
                        c.hog.secs as i64,
                    ),
                    -1,
                ),
            ],
        },
        Section {
            name: "Ports and shell",
            rows: vec![
                (
                    sw("Show every server", "ports.show_all", c.ports.show_all),
                    -1,
                ),
                (
                    pick_i(
                        "Port check every",
                        "ports.poll_secs",
                        &[("2 s", 2), ("5 s", 5), ("10 s", 10), ("30 s", 30)],
                        c.ports.poll_secs as i64,
                    ),
                    -1,
                ),
                (
                    pick_i(
                        "Show commands over",
                        "shell.threshold_ms",
                        &[("1 s", 1000), ("3 s", 3000), ("5 s", 5000), ("10 s", 10000)],
                        c.shell.threshold_ms as i64,
                    ),
                    -1,
                ),
            ],
        },
    ]
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
                gap: Cell::new(0.0),
                fx: Cell::new(false),
                fx_dt: Cell::new(0.0),
                fx_busy: Cell::new(false),
                fx_vals: RefCell::new(HashMap::new()),
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

    /// The gap above the pill, in logical px (see `gap`).
    pub fn set_gap(&self, g: f32) {
        self.gap.set(g);
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
        let (mut w, mut h) = (pw + 2.0 * (margin + room), ph + margin + self.gap.get());
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
            self.rt.SetTransform(&mat(self.scale, ox, self.gap.get()));
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
        let (fg, acc) = (self.fg(), view.accent);
        let bg = if view.light {
            [0.955, 0.955, 0.965]
        } else {
            [0.085, 0.09, 0.105]
        };
        let sections = settings_table(cfg);
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
                    self.setting_row(row, ry + 2.0, cw);
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
                let names: Vec<&str> = settings_table(cfg).iter().map(|s| s.name).collect();
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
            // ---- a note over the bottom of the page
            if let Some((text, a)) = view.toast {
                self.rt.SetTransform(&mat(self.scale, 0.0, 0.0));
                self.ox.set(0.0);
                self.oy.set(0.0);
                self.ca.set(a);
                let (tx, tw, th) = (side + 24.0, w - side - 48.0, 64.0);
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

    // ---------------------------------------------------------------- helpers

    /// A vertical gradient brush from cache `slot`, rebuilt only when `key` changes. Its end
    /// points and opacity are the caller's to set per frame.
    fn grad(
        &self,
        slot: Grad,
        key: [f32; 4],
        stops: &[(f32, [f32; 3], f32)],
    ) -> Option<ID2D1LinearGradientBrush> {
        let mut g = self.grads.borrow_mut();
        let cell = &mut g[slot as usize];
        if let Some((k, b)) = cell.as_ref() {
            if *k == key {
                return Some(b.clone());
            }
        }
        let stops: Vec<_> = stops
            .iter()
            .map(|&(position, c, a)| D2D1_GRADIENT_STOP {
                position,
                color: color(c, a),
            })
            .collect();
        let b = self.linear_brush(&stops)?;
        *cell = Some((key, b.clone()));
        Some(b)
    }

    /// Pill body gradient.
    fn body_brush(&self) -> Option<ID2D1LinearGradientBrush> {
        let (top, bottom) = self.body_colors();
        let key = [self.light.get() as u8 as f32, 0.0, 0.0, 0.0];
        self.grad(Grad::Body, key, &[(0.0, top, 1.0), (1.0, bottom, 1.0)])
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

    /// Shimmer for indeterminate progress; the content fade is applied through the brush
    /// opacity and the position through its end points.
    fn shimmer_brush(&self, col: [f32; 3]) -> Option<ID2D1LinearGradientBrush> {
        let key = [col[0], col[1], col[2], 0.0];
        self.grad(
            Grad::Shimmer,
            key,
            &[(0.0, col, 0.0), (0.5, col, 1.0), (1.0, col, 0.0)],
        )
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
    /// On the light pill, saturated accents are drawn darker so their text stays readable;
    /// greys and the fg / ink colours are left alone.
    fn cc(&self, c: [f32; 3], a: f32) -> D2D1_COLOR_F {
        let hi = c.iter().fold(0.0f32, |m, &x| m.max(x));
        let lo = c.iter().fold(1.0f32, |m, &x| m.min(x));
        let c = if self.light.get() && hi - lo > 0.25 {
            c.map(|x| x * 0.78)
        } else {
            c
        };
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

    /// Text that doesn't fit is cut with an ellipsis; while `hov`, it instead scrolls to show
    /// the rest: a pause, a glide to the end, a pause, then back to the start.
    #[allow(clippy::too_many_arguments)]
    fn label(
        &self,
        s: &str,
        f: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: D2D1_COLOR_F,
        hov: bool,
    ) {
        let layout = hov
            .then(|| {
                let wide: Vec<u16> = s.encode_utf16().collect();
                let l = unsafe { self.dw.CreateTextLayout(&wide, f, 4096.0, h).ok()? };
                let mut m = DWRITE_TEXT_METRICS::default();
                unsafe { l.GetMetrics(&mut m).ok()? };
                (m.widthIncludingTrailingWhitespace > w)
                    .then_some((l, m.widthIncludingTrailingWhitespace))
            })
            .flatten();
        let Some((l, tw)) = layout else {
            return self.text(s, f, x, y, w, h, c);
        };
        // Restart from the beginning whenever a different label is hovered.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut hs);
            hs.finish()
        };
        let t = self.t.get();
        let t0 = match self.marquee.get() {
            Some((k, t0)) if k == key => t0,
            _ => {
                self.marquee.set(Some((key, t)));
                t
            }
        };
        let travel = tw - w + 8.0;
        let glide = travel / 38.0;
        let (rest, cycle) = (1.1, 2.2 + glide);
        let ph = (t - t0) % (cycle + 0.9);
        let k = ((ph - rest) / glide).clamp(0.0, 1.0);
        let ease = k * k * (3.0 - 2.0 * k);
        self.scrolling.set(true);
        unsafe {
            let _ = l.SetMaxWidth(tw + 1.0);
            let _ = l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            self.rt
                .PushAxisAlignedClip(&rect(x, y, w, h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt.DrawTextLayout(
                v(x - travel * ease, y),
                &l,
                self.solid(c),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            self.rt.PopAxisAlignedClip();
        }
    }

    /// Register a clickable region (pill-relative); true when hovered.
    fn hit(&self, x: f32, y: f32, w: f32, h: f32, action: Action) -> bool {
        let (ox, oy) = (self.ox.get(), self.oy.get());
        let index = self.hits.borrow().len();
        self.hits.borrow_mut().push(Hit {
            rect: (x + ox, y + oy, x + ox + w, y + oy + h),
            action,
        });
        if self.focus_live.get() && self.focus.get() == Some(index) {
            self.stroke_rr(
                x - 2.0,
                y - 2.0,
                w + 4.0,
                h + 4.0,
                (h / 2.0).min(10.0) + 2.0,
                color(self.accent.get(), 0.95),
                1.6,
                false,
            );
        }
        self.over(x, y, w, h)
    }

    /// The pointer is over this pill-relative box (nothing is registered).
    fn over(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (ox, oy) = (self.ox.get(), self.oy.get());
        self.mouse.get().is_some_and(|(mx, my)| {
            mx >= x + ox && mx <= x + ox + w && my >= y + oy && my <= y + oy + h
        })
    }

    /// Register a pressable region: (hovered, held down, the box shrunk 6 % while held).
    fn press(&self, x: f32, y: f32, w: f32, h: f32, action: Action) -> (bool, bool, [f32; 4]) {
        let hov = self.hit(x, y, w, h, action);
        let down = hov && self.pressed.get();
        let k = if down { 0.03 } else { 0.0 };
        (
            hov,
            down,
            [
                x + w * k,
                y + h * k,
                w * (1.0 - 2.0 * k),
                h * (1.0 - 2.0 * k),
            ],
        )
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
        // Pressed in: a touch smaller and brighter while the button is held.
        let (hov, down, [x, y, w, h]) = self.press(x, y, w, h, action);
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
        let (hov, _, [x, y, w, h]) = self.press(x, y, w, h, action);
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

    /// A small Segoe Fluent Icons glyph (tab strip) centred on (cx, cy).
    fn glyph_s(&self, cp: char, cx: f32, cy: f32, c: D2D1_COLOR_F) {
        let mut b = [0u8; 4];
        let f = &self.f.icon_s;
        self.text(cp.encode_utf8(&mut b), f, cx - 9.0, cy - 9.0, 18.0, 18.0, c);
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

    /// A lightning bolt, 16 px tall at `k` = 1.
    fn bolt(&self, cx: f32, cy: f32, k: f32, c: D2D1_COLOR_F) {
        let pts = [
            (1.5, -8.0),
            (-4.5, 1.0),
            (-0.5, 1.0),
            (-1.5, 8.0),
            (4.5, -1.0),
            (0.5, -1.0),
        ]
        .map(|(x, y)| (cx + x * k, cy + y * k));
        self.poly(&pts, c);
    }

    /// An x of half-size `d` centred on (cx, cy).
    fn cross(&self, cx: f32, cy: f32, d: f32, c: D2D1_COLOR_F, width: f32) {
        self.line((cx - d, cy - d), (cx + d, cy + d), c, width);
        self.line((cx - d, cy + d), (cx + d, cy - d), c, width);
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
        let (hov, _, [x, y, w, h]) = self.press(x, y, w, h, action);
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
            match self.body_brush() {
                Some(b) => {
                    b.SetEndPoint(v(0.0, h.max(1.0)));
                    b.SetOpacity(fr.model.body_alpha);
                    self.rt.FillGeometry(&g, &b, None::<&ID2D1Brush>)
                }
                None => self.rt.FillGeometry(
                    &g,
                    self.solid(color(self.body_colors().0, fr.model.body_alpha)),
                    None::<&ID2D1Brush>,
                ),
            }
            // Tinted glass: the glow's colour pools up from the bottom of the body. It
            // follows the glow spring (and its pulse), so it never needs frames of its own.
            if gl > 0.01 {
                let c = an.glow_rgb;
                let k = if self.light.get() { 0.09 } else { 0.14 };
                if let Some(b) = self.grad(
                    Grad::Tint,
                    [c[0], c[1], c[2], k],
                    &[(0.0, c, 0.0), (1.0, c, k)],
                ) {
                    b.SetStartPoint(v(0.0, h * 0.2));
                    b.SetEndPoint(v(0.0, h));
                    b.SetOpacity(gl.min(1.0));
                    self.rt.FillGeometry(&g, &b, None::<&ID2D1Brush>);
                }
            }
            // Rim: barely there at the screen edge, catching the light round the bottom.
            let fg = self.fg();
            let rim = self.grad(
                Grad::Rim,
                [fg[0], 0.0, 0.0, 0.0],
                &[(0.0, fg, 0.03), (1.0, fg, 0.14)],
            );
            match rim {
                Some(b) => {
                    b.SetEndPoint(v(0.0, h.max(1.0)));
                    self.rt.DrawGeometry(&g, &b, 1.0, &self.round);
                }
                None => self
                    .rt
                    .DrawGeometry(&g, self.solid(color(fg, 0.07)), 1.0, &self.round),
            }
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
                self.art_tile(x + (bw - 20.0) / 2.0, cy - 10.0, 20.0, 10.0);
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
        self.tab_strip(fr, w, h, fit);
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
                Scene::ExpDrop
                    | Scene::ExpChip
                    | Scene::ExpTimer
                    | Scene::ExpTimerDone
                    | Scene::ExpSettings
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
            Scene::ExpDrop => self.exp_drop(w, h),
            Scene::ExpChip => self.exp_chip(fr, w, h),
            Scene::ExpTimer => self.exp_timer(fr, w, h),
            Scene::ExpTimerDone => self.exp_timer_done(fr, w, h),
            Scene::ExpCharge => self.exp_charge(fr, w, h),
            Scene::ExpSettings => self.exp_settings(fr, w, h),
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
                self.cross(cx, cy, 3.1, self.cc(col, 1.0), 2.0);
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
            // After a minute, how long it has waited: the oldest wait is the one to answer.
            TaskEvent::NeedsInput => {
                match now.saturating_duration_since(t.changed).as_secs() / 60 {
                    0 => "waiting".into(),
                    m => format!("waiting · {m}m"),
                }
            }
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

    /// A 64 x 64 bitmap scaled into a rounded square of side `s`; false if it can't be drawn.
    fn paint_bitmap(&self, bmp: &ID2D1Bitmap, x: f32, y: f32, s: f32, r: f32) -> bool {
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
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(x, y, s, s),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            let Ok(b) = self.rt.CreateBitmapBrush(bmp, None, Some(&props)) else {
                return false;
            };
            self.rt.FillRoundedRectangle(&rr, &b);
        }
        true
    }

    fn art_tile(&self, x: f32, y: f32, s: f32, r: f32) {
        if let Some(bmp) = self.art.borrow().as_ref() {
            if self.paint_bitmap(bmp, x, y, s, r) {
                return;
            }
        }
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
        self.art_tile(a[0], a[1], a[2], a[3]);
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
            // Hovered, a row that leads somewhere says so: an "open" arrow after its label.
            let go = if row_hov { 18.0 } else { 0.0 };
            let label_w = w - 42.0 - right_w - close_w - 22.0 - go;
            if row_hov {
                self.fill_rr(8.0, y + 2.0, w - 16.0, 32.0, 10.0, self.cc(self.fg(), 0.05));
                let gx = 42.0 + label_w + 10.0;
                self.glyph_s('\u{E8A7}', gx, y + 17.0, self.cc(self.fg(), 0.6));
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
            let hov = saved.is_none() && self.over(8.0, y + 2.0, w - 16.0, 32.0);
            self.label(
                &t.label,
                &self.f.title,
                lb[0],
                lb[1],
                lb[2],
                lb[3],
                self.cc(self.fg(), 0.95),
                hov,
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
                self.cross(w - 26.0, cy, 3.0, self.cc(self.fg(), a), 1.5);
            }
            if let Some(msg) = &t.attention {
                self.label(
                    msg,
                    &self.f.small,
                    42.0,
                    y + 26.0,
                    w - 42.0 - 22.0,
                    16.0,
                    self.cc(AMBER, 0.95),
                    self.over(42.0, y + 24.0, w - 64.0, 20.0),
                );
                y += 20.0;
            }
            if let (true, Some(line)) = (t.shows_line() && t.attention.is_none(), t.line.as_ref()) {
                self.label(
                    line,
                    &self.f.mono,
                    42.0,
                    y + 26.0,
                    w - 42.0 - 22.0,
                    crate::ui_state::LINE_H,
                    self.cc(GRAY, 0.85),
                    self.over(42.0, y + 24.0, w - 64.0, crate::ui_state::LINE_H + 4.0),
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
                let mut bx = 14.0;
                if t.rerunnable() {
                    let acc = self.accent.get();
                    self.button("Rerun", bx, y, 62.0, 24.0, acc, Action::Rerun(t.id.clone()));
                    bx += 70.0;
                }
                let err = Action::CopyText(t.stderr.join("\n"));
                self.button("Copy error", bx, y, 78.0, 24.0, RED, err);
                let dismiss = Action::DismissTask(t.id.clone());
                self.button("Dismiss", bx + 86.0, y, 66.0, 24.0, self.fg(), dismiss);
                y += 28.0;
            }
        }
        let (up, down) = fr.model.rows_hidden();
        if up + down > 0 {
            let note = match (up, down) {
                (0, d) => format!("{d} more below · scroll"),
                (u, 0) => format!("{u} above · scroll"),
                (u, d) => format!("{u} above, {d} below · scroll"),
            };
            self.text(&note, &self.f.small_c, 0.0, y, w, 16.0, self.cc(GRAY, 0.8));
            y += 16.0;
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
            self.label(
                &sub,
                &self.f.small,
                41.0,
                y + 19.0,
                w - 41.0 - 130.0,
                14.0,
                self.cc(GRAY, 1.0),
                self.over(8.0, y + 1.0, w - 146.0, row - 2.0),
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
        self.art_tile(a[0], a[1], a[2], a[3]);
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
        let hov = self.over(tx, 12.0, right - tx - 34.0, 48.0);
        self.label(
            &md.title,
            &self.f.big,
            tx,
            14.0,
            right - tx - 34.0,
            22.0,
            self.cc(self.fg(), 0.97),
            hov,
        );
        self.label(
            &md.artist,
            &self.f.body,
            tx,
            37.0,
            right - tx - 34.0,
            18.0,
            self.cc(GRAY, 1.0),
            hov,
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
        // Seekable: a press anywhere on it (a little taller than the bar) jumps there.
        let hov = md.dur_ms > 0 && self.hit(bx - 4.0, 58.0, bw + 8.0, 19.0, Action::Seek);
        let (bh, by) = if hov { (7.0, 64.0) } else { (5.0, 65.0) };
        self.fill_rr(bx, by, bw, bh, bh / 2.0, self.cc(self.fg(), 0.12));
        if md.dur_ms > 0 {
            let fw = (bw * frac).max(bh);
            self.fill_rr(bx, by, fw, bh, bh / 2.0, self.cc(acc, 0.92));
            if md.playing || hov {
                let r = if hov { 6.5 } else { 5.0 };
                self.circle(
                    bx + fw - bh / 2.0,
                    by + bh / 2.0,
                    r,
                    self.cc(self.fg(), 1.0),
                );
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
        let tab = if fr.model.tabs() { TAB_H } else { 0.0 };
        let cy = h - tab - 26.0;
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

    /// The pane switcher under a pane with neighbours: one icon per pane, the current one a
    /// pill with its name. Drawn once, as chrome that stays put while the pages slide. While
    /// paging, the old tab shrinks as the new one grows (the content spring drives both), so
    /// the highlight travels without any state of its own.
    fn tab_strip(&self, fr: &Frame, w: f32, h: f32, fit: f32) {
        let (m, an) = (fr.model, fr.anim);
        if !m.tabs() {
            return;
        }
        let live = Panel::of(an.scene).is_some();
        let gone = an.prev != an.scene && Panel::of(an.prev).is_some();
        let (c, o) = (
            an.content.pos.clamp(0.0, 1.0) as f32,
            an.out.pos.clamp(0.0, 1.0) as f32,
        );
        // Fade with the pane when the strip arrives or leaves; stay solid between two panes.
        let a = match (live, gone) {
            (true, true) => 1.0,
            (true, false) => c,
            (false, true) => o,
            (false, false) => return,
        };
        // Leaving (collapsing to a compact scene): visible, but not clickable.
        let (mouse, n) = (self.mouse.get(), self.hits.borrow().len());
        if !live {
            self.mouse.set(None);
        }
        unsafe { self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0)) };
        self.ca.set(a * fit);
        let ps = m.panels();
        // The pane on screen, not the selection: the timer setup card is no pane at all.
        let on = |s: Scene| Panel::of(s).filter(|p| ps.contains(p));
        let cur = on(an.scene);
        let prev = on(an.prev).filter(|_| live && gone);
        let weight = |p: Panel| match prev {
            Some(q) if Some(p) == cur && Some(q) != cur => c,
            Some(q) if p == q && Some(q) != cur => 1.0 - c,
            _ => (Some(p) == cur) as u8 as f32,
        };
        let icon = 26.0;
        let label = |p: Panel| text_w(p.name()) + 10.0;
        let total: f32 = ps.iter().map(|&p| icon + label(p) * weight(p)).sum();
        let mut x = (w - total) / 2.0;
        let (cy, th) = (h - TAB_H / 2.0 - 2.0, 20.0);
        let fg = self.fg();
        let mut tip = None;
        for &p in &ps {
            let k = weight(p);
            let tw = icon + label(p) * k;
            let hov = self.hit(x, cy - th / 2.0, tw, th, Action::SelectPanel(p));
            if k > 0.01 {
                let acc = self.accent.get();
                self.fill_rr(x, cy - th / 2.0, tw, th, th / 2.0, self.cc(acc, 0.24 * k));
            } else if hov {
                self.fill_rr(x, cy - th / 2.0, tw, th, th / 2.0, self.cc(fg, 0.06));
                tip = Some((p, x + tw / 2.0));
            }
            let ia = 0.42 + 0.53 * k.max(hov as u8 as f32 * 0.6);
            self.glyph_s(panel_glyph(p), x + icon / 2.0, cy, self.cc(fg, ia));
            if k > 0.01 {
                unsafe {
                    self.rt.PushAxisAlignedClip(
                        &rect(x, cy - th / 2.0, tw - 6.0, th),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                }
                self.text(
                    p.name(),
                    &self.f.small_l,
                    x + icon - 3.0,
                    cy - 8.0,
                    label(p) + 4.0,
                    16.0,
                    self.cc(fg, 0.95 * k),
                );
                unsafe { self.rt.PopAxisAlignedClip() };
            }
            x += tw;
        }
        // The hovered tab's name, in a small label just above it.
        if let Some((p, tx)) = tip {
            let lw = text_w(p.name()) + 16.0;
            let (lx, ly) = (
                (tx - lw / 2.0).clamp(6.0, w - lw - 6.0),
                cy - th / 2.0 - 24.0,
            );
            let (_, bottom) = self.body_colors();
            self.fill_rr(lx, ly, lw, 20.0, 10.0, self.cc(bottom, 0.96));
            self.stroke_rr(lx, ly, lw, 20.0, 10.0, self.cc(fg, 0.14), 1.0, false);
            self.text(p.name(), &self.f.btn, lx, ly, lw, 20.0, self.cc(fg, 0.95));
        }
        if !live {
            self.hits.borrow_mut().truncate(n);
            self.mouse.set(mouse);
        }
    }

    fn exp_shelf(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let thumbs = self.thumbs.borrow();
        // The row of tiles is centred (70 px a tile, 60 of it the tile itself).
        let (off, page) = m.shelf_page();
        let x0 = (w - (page.len() as f32 * 70.0 - 10.0)) / 2.0;
        // More than fit: chevrons in the margins scroll the row (so does the wheel).
        if m.shelf.len() > page.len() {
            let more = [
                (off > 0, -1, '\u{E76B}', 2.0),
                (off + page.len() < m.shelf.len(), 1, '\u{E76C}', w - 18.0),
            ];
            for (on, by, icon, x) in more {
                if on {
                    let hov = self.hit(x, 14.0, 16.0, 60.0, Action::ShelfScroll(by));
                    let a = if hov { 0.95 } else { 0.5 };
                    self.glyph_s(icon, x + 8.0, 44.0, self.cc(self.fg(), a));
                }
            }
        }
        for (i, it) in page.iter().enumerate() {
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
                self.paint_bitmap(b, x + 6.0, y + 6.0, 48.0, 7.0);
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
            self.label(
                &it.name,
                &self.f.small_c,
                x - 2.0,
                y + s + 3.0,
                s + 4.0,
                14.0,
                self.cc(self.fg(), if sel || hov { 0.95 } else { 0.7 }),
                hov,
            );
            if hov || sel {
                let h2 = self.hit(
                    x + s - 14.0,
                    y - 5.0,
                    18.0,
                    18.0,
                    Action::RemoveShelf(it.id),
                );
                self.circle(
                    x + s - 5.0,
                    y + 4.0,
                    7.5,
                    self.cc(self.body_colors().1, 1.0),
                );
                self.ring(x + s - 5.0, y + 4.0, 7.5, self.cc(self.fg(), 0.12), 1.0);
                let a = if h2 { 1.0 } else { 0.7 };
                self.cross(x + s - 5.0, y + 4.0, 3.0, self.cc(self.fg(), a), 1.4);
            }
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

        // Volume bar: click to set, scroll to nudge. The per-app chevron and the settings
        // gear sit at its end.
        let (bx, by, bw, bh) = (16.0, y + th + 12.0, w - 32.0 - 76.0, 32.0);
        let chev = if m.apps_open { '\u{E70E}' } else { '\u{E70D}' };
        self.round_btn(
            w - 68.0,
            by + bh / 2.0,
            16.0,
            self.fg(),
            Action::ToggleApps,
            |cx, cy, c| self.glyph(chev, cx, cy, 14.0, c),
        );
        if m.apps_open {
            self.app_rows(m, by + bh + 8.0, w);
        }
        self.round_btn(
            w - 32.0,
            by + bh / 2.0,
            16.0,
            self.fg(),
            Action::Settings(Some(0)),
            |cx, cy, c| self.glyph('\u{E713}', cx, cy, 16.0, c),
        );
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
                // Hovered: the exact level at the end of the bar (inside the fill when it
                // reaches that far, so it takes the fill's ink).
                if hov {
                    let tx = bx + bw - 54.0;
                    let ink = if fw > bw - 14.0 && !muted {
                        self.ink()
                    } else {
                        self.fg()
                    };
                    let pct = format!("{}%", (vol * 100.0).round());
                    self.text(&pct, &self.f.small_r, tx, by, 42.0, bh, self.cc(ink, 0.8));
                }
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

    /// The per-app volume list: a name and a thin bar per app, or a note when none plays.
    fn app_rows(&self, m: &crate::ui_state::Model, y: f32, w: f32) {
        use crate::ui_state::APP_ROW_H;
        let fg = self.fg();
        if m.apps.is_empty() {
            let note = "No apps are using the speaker";
            self.text(
                note,
                &self.f.small_c,
                16.0,
                y,
                w - 32.0,
                APP_ROW_H,
                self.cc(GRAY, 0.9),
            );
            return;
        }
        let (bx, bw, bh) = (120.0, w - 136.0, 18.0);
        for (i, a) in m.apps.iter().enumerate() {
            let ry = y + i as f32 * APP_ROW_H;
            let label = crate::mic::app_label(&a.exe);
            let alpha = if a.active { 0.9 } else { 0.6 };
            self.text(
                &label,
                &self.f.small_l,
                20.0,
                ry,
                bx - 28.0,
                APP_ROW_H,
                self.cc(fg, alpha),
            );
            let by = ry + (APP_ROW_H - bh) / 2.0;
            // The hit box is the full row height, so a thin bar is still easy to grab.
            let hov = self.hit(bx, ry, bw, APP_ROW_H, Action::AppVolume(a.exe.clone()));
            self.fill_rr(
                bx,
                by,
                bw,
                bh,
                bh / 2.0,
                self.cc(fg, if hov { 0.10 } else { 0.06 }),
            );
            let (fill, fa) = if a.muted { (GRAY, 0.30) } else { (fg, 0.8) };
            self.fill_rr(
                bx,
                by,
                (bw * a.vol).max(bh),
                bh,
                bh / 2.0,
                self.cc(fill, fa),
            );
            if hov {
                let pct = format!("{}%", (a.vol * 100.0).round());
                let ink = if a.vol > 0.85 && !a.muted {
                    self.ink()
                } else {
                    fg
                };
                self.text(
                    &pct,
                    &self.f.small_r,
                    bx + bw - 50.0,
                    by,
                    42.0,
                    bh,
                    self.cc(ink, 0.8),
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
            self.bolt(cx, cy, 1.0, self.cc(self.ink(), 0.9));
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

    /// Settings: a header (back, the group switch, open the config file) over up to five
    /// rows. Each row is a switch or a short choice that writes `config.toml` on click.
    fn exp_settings(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::ui_state::SETTINGS_GROUPS;
        let c = fr.cfg;
        let group = fr.model.settings.unwrap_or(0);
        let (fg, acc) = (self.fg(), self.accent.get());
        let back = Action::Settings(None);
        self.round_btn(28.0, 23.0, 13.0, fg, back, |cx, cy, col| {
            self.glyph_s('\u{E72B}', cx, cy, col)
        });
        self.round_btn(
            w - 28.0,
            23.0,
            13.0,
            fg,
            Action::OpenConfig,
            |cx, cy, col| self.glyph_s('\u{E8A7}', cx, cy, col),
        );
        let (sx, sw) = (50.0, w - 100.0);
        self.fill_rr(sx, 10.0, sw, 26.0, 13.0, self.cc(fg, 0.06));
        let seg = sw / SETTINGS_GROUPS.len() as f32;
        for (i, name) in SETTINGS_GROUPS.iter().enumerate() {
            let x = sx + i as f32 * seg;
            let on = i as u8 == group;
            let hov = self.hit(x, 10.0, seg, 26.0, Action::Settings(Some(i as u8)));
            if on {
                self.fill_rr(x + 2.0, 12.0, seg - 4.0, 22.0, 11.0, self.cc(acc, 0.26));
            } else if hov {
                self.fill_rr(x + 2.0, 12.0, seg - 4.0, 22.0, 11.0, self.cc(fg, 0.07));
            }
            let a = if on || hov { 1.0 } else { 0.66 };
            self.text(name, &self.f.btn, x, 10.0, seg, 26.0, self.cc(fg, a));
        }

        let rows: Vec<Row> = settings_table(c)
            .into_iter()
            .flat_map(|s| s.rows)
            .filter(|(_, q)| *q == group as i8)
            .map(|(r, _)| r)
            .collect();
        let n = rows.len();
        for (i, row) in rows.into_iter().enumerate() {
            self.setting_row(row, 46.0 + i as f32 * 32.0, w);
        }
        // Spare room at the bottom: where the rest of the settings live.
        if n < 5 {
            let note = concat!(
                "Hytte ",
                env!("CARGO_PKG_VERSION"),
                "  ·  more in config.toml"
            );
            let y = 46.0 + 4.0 * 32.0;
            self.text(note, &self.f.small_c, 0.0, y, w, 30.0, self.cc(GRAY, 0.75));
        }
    }

    /// `key`'s value eased towards `target` (settings window only: the pill's pane snaps).
    /// The first sight of a key starts at the target, so nothing animates on open.
    fn ease(&self, key: &str, target: f32) -> f32 {
        if !self.fx.get() {
            return target;
        }
        let mut m = self.fx_vals.borrow_mut();
        let v = m.entry(key.to_string()).or_insert(target);
        let k = 1.0 - (-self.fx_dt.get() * 16.0).exp();
        *v += (target - *v) * k;
        if (target - *v).abs() < 0.003 {
            *v = target;
        } else {
            self.fx_busy.set(true);
        }
        *v
    }

    /// One settings row at `y`, `w` wide: a switch, a few choices or the opacity slider.
    fn setting_row(&self, row: Row, y: f32, w: f32) {
        let (fg, acc) = (self.fg(), self.accent.get());
        match row {
            Row::Switch(label, key, on) => {
                let hov = self.hit(10.0, y, w - 20.0, 30.0, Action::Set(key, Val::B(!on)));
                let h = self.ease(&format!("h:{key}"), hov as u8 as f32);
                if h > 0.01 {
                    self.fill_rr(10.0, y, w - 20.0, 30.0, 12.0, self.cc(fg, 0.06 * h));
                }
                self.text(
                    label,
                    &self.f.body,
                    22.0,
                    y,
                    w - 90.0,
                    30.0,
                    self.cc(fg, 0.9),
                );
                let t = self.ease(&format!("s:{key}"), on as u8 as f32);
                let (kx, ky) = (w - 22.0 - 34.0, y + 6.0);
                if t < 0.99 {
                    self.stroke_rr(
                        kx,
                        ky,
                        34.0,
                        18.0,
                        9.0,
                        self.cc(fg, 0.4 * (1.0 - t)),
                        1.2,
                        false,
                    );
                }
                if t > 0.01 {
                    self.fill_rr(kx, ky, 34.0, 18.0, 9.0, self.cc(acc, 0.95 * t));
                }
                let knob = mix3(fg, self.ink(), t);
                self.circle(
                    kx + 9.0 + 16.0 * t,
                    ky + 9.0,
                    5.0 + t,
                    self.cc(knob, 0.7 + 0.25 * t),
                );
            }
            Row::Opacity(v) => {
                self.text(
                    "Opacity",
                    &self.f.body,
                    22.0,
                    y,
                    100.0,
                    30.0,
                    self.cc(fg, 0.9),
                );
                let pct = format!("{}%", (v * 100.0).round());
                self.text(&pct, &self.f.small, 84.0, y, 44.0, 30.0, self.cc(GRAY, 0.9));
                let (bx, bw, bh) = (w - 22.0 - 206.0 + 6.0, 194.0, 6.0);
                let by = y + 12.0;
                let hov = self.hit(bx - 6.0, y, bw + 12.0, 30.0, Action::Opacity);
                let h = self.ease("h:opacity", (hov || self.pressed.get() && hov) as u8 as f32);
                self.fill_rr(bx, by, bw, bh, bh / 2.0, self.cc(fg, 0.08 + 0.04 * h));
                let f = ((v - 0.4) / 0.6).clamp(0.0, 1.0);
                self.fill_rr(bx, by, (bw * f).max(bh), bh, bh / 2.0, self.cc(acc, 0.9));
                self.circle(
                    bx + 3.0 + (bw - 6.0) * f,
                    by + bh / 2.0,
                    6.5 + 1.5 * h,
                    self.cc(fg, 0.95),
                );
            }
            Row::Pick(label, key, opts, cur) => self.pick_row(label, key, opts, cur, None, y, w),
            Row::PickCustom(label, key, opts, cur, hint, shown) => {
                self.pick_row(label, key, opts, cur, Some((hint, shown)), y, w)
            }
        }
    }

    /// Steps in a pill, the lit one sliding to the chosen step. With `custom`, one more
    /// segment ("Custom") opens `config.toml` at the setting; it is lit, and the file's value
    /// shown beside the label, when the value is none of the steps.
    #[allow(clippy::too_many_arguments)]
    fn pick_row(
        &self,
        label: &str,
        key: &'static str,
        opts: Vec<Choice>,
        cur: usize,
        custom: Option<(&'static str, String)>,
        y: f32,
        w: f32,
    ) {
        let (fg, acc) = (self.fg(), self.accent.get());
        self.text(label, &self.f.body, 22.0, y, 150.0, 30.0, self.cc(fg, 0.9));
        let extra = custom.is_some() as usize;
        let pw = 206.0 + 40.0 * extra as f32;
        let px = w - 22.0 - pw;
        if let Some((_, shown)) = &custom {
            if !shown.is_empty() {
                self.text(
                    shown,
                    &self.f.small,
                    108.0,
                    y,
                    px - 112.0,
                    30.0,
                    self.cc(GRAY, 0.9),
                );
            }
        }
        self.fill_rr(px, y + 3.0, pw, 24.0, 12.0, self.cc(fg, 0.06));
        let sw = pw / (opts.len() + extra) as f32;
        // The lit segment slides to the chosen one.
        let at = self.ease(&format!("p:{key}"), cur as f32);
        self.fill_rr(
            px + at * sw + 2.0,
            y + 5.0,
            sw - 4.0,
            20.0,
            10.0,
            self.cc(acc, 0.26),
        );
        let segments = opts
            .into_iter()
            .map(|(name, v)| (name, Action::Set(key, v)))
            .chain(custom.map(|(hint, _)| ("Custom", Action::EditConfig(hint))));
        for (j, (name, action)) in segments.enumerate() {
            let x = px + j as f32 * sw;
            let hov = self.hit(x, y + 3.0, sw, 24.0, action);
            let on = j == cur;
            let h = self.ease(&format!("ph:{key}:{j}"), (hov && !on) as u8 as f32);
            if h > 0.01 {
                self.fill_rr(
                    x + 2.0,
                    y + 5.0,
                    sw - 4.0,
                    20.0,
                    10.0,
                    self.cc(fg, 0.07 * h),
                );
            }
            let a = if on { 1.0 } else { 0.62 + 0.38 * h };
            self.text(name, &self.f.btn, x, y + 3.0, sw, 24.0, self.cc(fg, a));
        }
    }

    fn exp_drop(&self, w: f32, h: f32) {
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
        if plugged {
            let a = 0.55 + 0.45 * ease;
            self.bolt(bx + bw / 2.0, cy, 0.7, self.cc([1.0; 3], a));
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
            "Scroll or type the minutes",
            &self.f.small,
            21.0,
            44.0,
            w - 21.0 - 104.0,
            14.0,
            self.cc(GRAY, 0.9),
        );
        let fg = self.fg();
        // Close, small and quiet in the corner; − / + to its left.
        self.round_btn(
            w - 16.0 - 11.0,
            22.0,
            11.0,
            fg,
            Action::TimerClose,
            |x, y, c| self.cross(x, y, 3.5, c, 1.5),
        );
        let bx = w - 16.0 - 22.0 - 10.0 - 18.0;
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
                self.cc(if on { self.accent.get() } else { fg }, a),
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
            self.accent.get(),
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
mod snap {
    use super::*;
    use crate::ui_state::MediaInfo;
    use hytte_proto::ports::PortInfo;

    fn bitmap_props(opts: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
        D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: opts,
            ..Default::default()
        }
    }

    impl Renderer {
        /// A renderer drawing into an offscreen canvas-sized bitmap (no window).
        fn offscreen(scale: f32) -> windows::core::Result<Self> {
            let factory1: ID2D1Factory1 =
                unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
            let dc = crate::gpu::offscreen(&factory1)?;
            let mut r = Self::build(factory1, dc.cast()?, None)?;
            r.set_scale(scale);
            let size = D2D_SIZE_U {
                width: r.px.0 as u32,
                height: r.px.1 as u32,
            };
            unsafe {
                let target =
                    dc.CreateBitmap(size, None, 0, &bitmap_props(D2D1_BITMAP_OPTIONS_TARGET))?;
                dc.SetTarget(&target);
            }
            Ok(r)
        }

        /// An offscreen renderer of an arbitrary size (the settings window).
        fn offscreen_sized(w: i32, h: i32, scale: f32) -> windows::core::Result<Self> {
            let factory1: ID2D1Factory1 =
                unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
            let dc = crate::gpu::offscreen(&factory1)?;
            let mut r = Self::build(factory1, dc.cast()?, None)?;
            r.px = (w, h);
            r.scale = scale;
            let size = D2D_SIZE_U {
                width: w as u32,
                height: h as u32,
            };
            unsafe {
                let target =
                    dc.CreateBitmap(size, None, 0, &bitmap_props(D2D1_BITMAP_OPTIONS_TARGET))?;
                dc.SetTarget(&target);
            }
            Ok(r)
        }

        /// The canvas as premultiplied BGRA rows.
        fn read_back(&self) -> Vec<u8> {
            let dc: ID2D1DeviceContext = self.rt.cast().unwrap();
            let (w, h) = (self.px.0 as u32, self.px.1 as u32);
            unsafe {
                let target: ID2D1Bitmap = dc.GetTarget().unwrap().cast().unwrap();
                let cpu = dc
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: w,
                            height: h,
                        },
                        None,
                        0,
                        &bitmap_props(
                            D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                        ),
                    )
                    .unwrap();
                cpu.CopyFromBitmap(None, &target, None).unwrap();
                let map = cpu.Map(D2D1_MAP_OPTIONS_READ).unwrap();
                let mut out = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    let row = map.bits.add((y * map.pitch) as usize);
                    out.extend_from_slice(std::slice::from_raw_parts(row, (w * 4) as usize));
                }
                cpu.Unmap().unwrap();
                out
            }
        }
    }

    fn shoot(
        r: &mut Renderer,
        dir: &std::path::Path,
        name: &str,
        m: &Model,
        scene: Scene,
        light: bool,
    ) {
        shoot_at(r, dir, name, m, scene, light, None);
    }

    /// `shoot` with the pointer at a pill-relative point.
    fn shoot_at(
        r: &mut Renderer,
        dir: &std::path::Path,
        name: &str,
        m: &Model,
        scene: Scene,
        light: bool,
        at: Option<(f32, f32)>,
    ) {
        let mut an = Anim::new(120.0, 24.0);
        an.set_scene(scene, m.size(scene), m.glow(scene));
        for _ in 0..240 {
            an.step(1.0 / 60.0);
        }
        let fr = Frame {
            model: m,
            cfg: &Config::default(),
            anim: &an,
            now: Instant::now(),
            dt: 1.0 / 60.0,
            mouse: at.map(|(x, y)| (x + (CANVAS_W - an.rect.w.pos as f32) / 2.0, y)),
            pressed: false,
            light,
            accent: BLUE,
            focus: None,
            armed: false,
            ringing: true,
        };
        let crop = r.plan(an.rect.w.pos as f32, an.rect.h.pos as f32, false, 0.0, true);
        let mut hits = vec![];
        r.draw(&fr, crop, &mut hits);
        // Only the pill's box, read back from the canvas-sized target.
        let (x0, w, h) = (crop.clip.0, crop.clip.2 - crop.clip.0, crop.clip.3);
        let px = r.read_back();
        let cw = crop.w_px;
        // Premultiplied BGRA over the wallpaper, as an RGB PNG.
        let bg = if light {
            [226.0, 230.0, 236.0]
        } else {
            [46.0, 52.0, 58.0]
        };
        let img = image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
            let i = ((y as i32 * cw + x0 + x as i32) * 4) as usize;
            let a = px[i + 3] as f32 / 255.0;
            // BGRA -> RGB
            let ch = |c: usize, b: f32| (px[i + c] as f32 + b * (1.0 - a)).min(255.0) as u8;
            image::Rgb([ch(2, bg[0]), ch(1, bg[1]), ch(0, bg[2])])
        });
        img.save(dir.join(format!("{name}.png"))).unwrap();
        SHOT.with(|v| v.borrow_mut().push(name.to_string()));
    }

    thread_local! {
        /// Every picture taken this run, in order, for the contact sheet.
        static SHOT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// `index.html` beside the pictures: all of them on one page, named, for review.
    fn contact_sheet(dir: &std::path::Path) {
        let cells: String = SHOT.with(|v| {
            v.borrow()
                .iter()
                .map(|n| {
                    format!("<figure><img src=\"{n}.png\"><figcaption>{n}</figcaption></figure>\n")
                })
                .collect()
        });
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>Hytte snapshots</title>\n\
             <style>body{{background:#1d2026;color:#cfd5de;font:13px system-ui;margin:24px}}\
             main{{display:grid;grid-template-columns:repeat(auto-fill,minmax(420px,1fr));gap:20px}}\
             figure{{margin:0}}img{{width:100%;border-radius:6px}}</style>\n<main>\n{cells}</main>\n"
        );
        std::fs::write(dir.join("index.html"), page).unwrap();
    }

    /// A model with the shipped glow strength (`Model::default` has none).
    fn model() -> Model {
        Model {
            glow_gain: 1.2,
            body_alpha: 1.0,
            ..Default::default()
        }
    }

    /// Every row of the settings table writes a key that `config.toml` accepts, and the file
    /// still parses afterwards (a typo in a key would silently write nothing useful).
    #[test]
    fn settings_rows_write_valid_config() {
        let cfg = Config::default();
        let mut keys = std::collections::HashSet::new();
        for sec in settings_table(&cfg) {
            for (row, _) in sec.rows {
                let writes: Vec<(&str, Val)> = match row {
                    Row::Switch(_, k, _) => vec![(k, Val::B(true)), (k, Val::B(false))],
                    Row::Pick(_, k, opts, _) | Row::PickCustom(_, k, opts, ..) => {
                        opts.into_iter().map(|(_, v)| (k, v)).collect()
                    }
                    Row::Opacity(_) => vec![("general.pill_opacity", Val::F(0.7))],
                };
                for (k, v) in writes {
                    keys.insert(k);
                    let text = crate::config::with_value("", k, v).expect(k);
                    assert!(crate::config::parse(&text).is_some(), "{k} broke the file");
                    let leaf = k.rsplit('.').next().unwrap();
                    assert!(text.contains(leaf), "{k} wrote nothing: {text}");
                }
            }
        }
        // No key twice in the table (a row would fight another).
        let rows: usize = settings_table(&cfg).iter().map(|s| s.rows.len()).sum();
        assert_eq!(keys.len(), rows);
    }

    /// The settings window, light and dark, at the top, scrolled, and with the pointer on a row.
    #[test]
    #[ignore]
    fn settings_window_snapshots() {
        let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h, sc) = (1720, 1400, 2.0);
        let cfg = Config::default();
        type Shot = (&'static str, bool, f32, Option<(f32, f32)>);
        let shots: [Shot; 5] = [
            ("settings-dark", false, 0.0, None),
            ("settings-dark-hover", false, 0.0, Some((420.0, 150.0))),
            ("settings-dark-scrolled", false, 640.0, None),
            ("settings-light", true, 0.0, Some((560.0, 118.0))),
            ("settings-narrow-light", true, 0.0, None),
        ];
        for (name, light, scroll, mouse) in shots {
            let narrow = name.contains("narrow");
            // A fresh renderer each: eased values must not carry over from the last shot.
            let mut r = Renderer::offscreen_sized(if narrow { 1120 } else { w }, h, sc).unwrap();
            let view = SettingsView {
                mouse,
                pressed: false,
                light,
                accent: BLUE,
                scroll,
                scrolling: scroll > 0.0,
                dt: 0.016,
                open: 1.0,
                toast: None,
            };
            let mut hits = vec![];
            // Twice: the first frame seeds the eased values, the second settles them.
            r.draw_settings_window(&cfg, &view, &mut hits);
            r.draw_settings_window(&cfg, &view, &mut hits);
            let (pw, ph) = r.px;
            let px = r.read_back();
            let img = image::RgbImage::from_fn(pw as u32, ph as u32, |x, y| {
                let i = ((y as i32 * pw + x as i32) * 4) as usize;
                image::Rgb([px[i + 2], px[i + 1], px[i]])
            });
            img.save(dir.join(format!("{name}.png"))).unwrap();
            SHOT.with(|v| v.borrow_mut().push(name.to_string()));
        }
        // A custom quiet-hours value in the file, and the note "Custom" shows.
        let mut custom = Config::default();
        custom.general.quiet_hours = "22:30-06:00".into();
        let mut r = Renderer::offscreen_sized(w, h, sc).unwrap();
        let view = SettingsView {
            mouse: None,
            pressed: false,
            light: false,
            accent: BLUE,
            scroll: 330.0,
            scrolling: false,
            dt: 0.016,
            open: 1.0,
            toast: Some(("In config.toml, under [general], set quiet_hours to your times as \"HH:MM-HH:MM\" (24-hour clock, may wrap past midnight), for example \"22:30-06:00\". Save the file and it applies.", 1.0)),
        };
        let mut hits = vec![];
        r.draw_settings_window(&custom, &view, &mut hits);
        r.draw_settings_window(&custom, &view, &mut hits);
        let (pw, ph) = r.px;
        let px = r.read_back();
        image::RgbImage::from_fn(pw as u32, ph as u32, |x, y| {
            let i = ((y as i32 * pw + x as i32) * 4) as usize;
            image::Rgb([px[i + 2], px[i + 1], px[i]])
        })
        .save(dir.join("settings-custom.png"))
        .unwrap();
        SHOT.with(|v| v.borrow_mut().push("settings-custom".to_string()));
        contact_sheet(&dir);
    }

    /// The gap above the pill is part of the window: taller by that much, so pointing at the
    /// strip still reaches the pill.
    #[test]
    fn the_gap_above_the_pill_is_inside_the_window() {
        let r = Renderer::offscreen(2.0).unwrap();
        let plain = r.plan(380.0, 100.0, false, 0.0, true);
        r.set_gap(20.0);
        let gapped = r.plan(380.0, 100.0, false, 0.0, true);
        assert_eq!(gapped.clip.3 - plain.clip.3, 40);
        assert_eq!(
            gapped.clip.1, 0,
            "the window region still starts at the top"
        );
    }

    #[test]
    #[ignore]
    fn snapshots() {
        let dir = std::path::PathBuf::from(std::env::var("HYTTE_SNAP_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let mut r = Renderer::offscreen(2.0).unwrap();
        let now = Instant::now();
        let wall = crate::timer::now_ms();

        let mut m = model();
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
        // The pointer on the timeline: it thickens and shows its knob (seekable).
        shoot_at(
            &mut r,
            &dir,
            "media-seek-hover",
            &m,
            Scene::ExpMedia,
            false,
            Some((240.0, 67.0)),
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
        for (g, name) in [
            (0, "settings-look"),
            (1, "settings-behaviour"),
            (2, "settings-features"),
        ] {
            m.settings = Some(g);
            shoot(&mut r, &dir, name, &m, Scene::ExpSettings, false);
        }
        shoot(
            &mut r,
            &dir,
            "settings-features-light",
            &m,
            Scene::ExpSettings,
            true,
        );
        m.settings = None;
        shoot(&mut r, &dir, "sentinel", &m, Scene::Sentinel, false);

        let mut m = model();
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
                cwd: Some(r"C:\src\app".into()),
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
        shoot(&mut r, &dir, "tasks-light", &m, Scene::ExpTasks, true);
        // Every pane at once, the pointer resting on a tab (its name shows above it).
        let all = &mut m;
        all.media = Some(MediaInfo {
            title: "A very long song title that will never fit in the pill".into(),
            artist: "Someone".into(),
            playing: true,
            app: String::new(),
            pos_ms: 1_000,
            dur_ms: 200_000,
        });
        all.shelf = vec![crate::shelf::ShelfItem {
            id: 9,
            path: "a.txt".into(),
            name: "a.txt".into(),
            is_dir: false,
            owned: false,
        }];
        all.timer = Some(crate::timer::Timer::start(
            crate::timer::TimerKind::Plain,
            10,
            wall,
        ));
        all.power = Some(crate::power::Battery {
            pct: 80,
            plugged: true,
            watts: None,
            mode: Mode::Balanced,
            mins_left: None,
        });
        all.selected = Some(Panel::Media);
        let h = all.size(Scene::ExpMedia).1 as f32;
        shoot_at(
            &mut r,
            &dir,
            "tabs-hover",
            all,
            Scene::ExpMedia,
            false,
            Some((236.0, h - 14.0)),
        );
        // One pane: a dev server started from a terminal task sits under it, linked.
        let mut m = model();
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
        // More than fit: scrolled two along, chevrons on both sides.
        let more = m.shelf.len() as u64;
        m.shelf.extend((0..4).map(|i| crate::shelf::ShelfItem {
            id: more + i + 1,
            path: format!("extra-{i}.txt").into(),
            name: format!("extra-{i}.txt"),
            is_dir: false,
            owned: false,
        }));
        m.shelf_sel = None;
        m.shelf_off = 2;
        shoot(&mut r, &dir, "shelf-scrolled", &m, Scene::ExpShelf, false);
        m.shelf_off = 0;
        let mut m = model();
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
        contact_sheet(&dir);
    }
}
