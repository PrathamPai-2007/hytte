//! The pill's springs and the two-finger swipe detector.

use super::*;
use crate::animation::motion;

/// One scene as it is drawn this frame: how opaque, and how far it is shifted from its place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shown {
    pub scene: Scene,
    pub alpha: f32,
    pub dx: f32,
    pub dy: f32,
    /// The scene the pill is showing (the rest are leaving): only it takes the pointer.
    pub live: bool,
}

/// Most scenes drawn at once.
pub const MAX_SHOWN: usize = 4;

/// The scenes drawn this frame, back to front. A fixed array: nothing is allocated per frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShownList {
    items: [Option<Shown>; MAX_SHOWN],
}

impl ShownList {
    fn push(&mut self, s: Shown) {
        if let Some(slot) = self.items.iter_mut().find(|x| x.is_none()) {
            *slot = Some(s);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Shown> {
        self.items.iter().flatten()
    }
}

/// Most scenes on their way in or out at once. A change past this drops the faintest.
pub const MAX_LAYERS: usize = 3;

/// One scene on screen: its opacity and how far its content is shifted, each a spring. A
/// change of scene only retargets springs, so however often the scene changes (and changes
/// back), nothing jumps.
#[derive(Debug, Clone, Copy)]
pub struct Layer {
    pub scene: Scene,
    pub alpha: Spring,
    pub dx: Spring,
    pub dy: Spring,
}

impl Layer {
    /// A scene arriving: transparent and `from` px away, heading for its place.
    fn arriving(scene: Scene, from: (f64, f64)) -> Self {
        let mut l = Self {
            scene,
            alpha: Spring::unit(0.0, motion::ENTER),
            dx: offset(from.0),
            dy: offset(from.1),
        };
        l.enter();
        l
    }

    /// Head for fully shown, in place (from wherever it is, at whatever speed).
    fn enter(&mut self) {
        for s in [&mut self.alpha, &mut self.dx, &mut self.dy] {
            s.zeta = motion::ENTER.zeta;
            s.period = motion::ENTER.period;
        }
        self.alpha.set_target(1.0);
        self.dx.set_target(0.0);
        self.dy.set_target(0.0);
    }

    /// Head for gone, drifting `to` px away.
    fn leave(&mut self, to: (f64, f64)) {
        for s in [&mut self.alpha, &mut self.dx, &mut self.dy] {
            s.zeta = motion::EXIT.zeta;
            s.period = motion::EXIT.period;
        }
        self.alpha.set_target(0.0);
        self.dx.set_target(to.0);
        self.dy.set_target(to.1);
    }

    fn snap(&mut self) {
        self.alpha.snap();
        self.dx.snap();
        self.dy.snap();
    }

    fn advance(&mut self, dt: f64) -> bool {
        let a = self.alpha.advance(dt);
        let b = self.dx.advance(dt);
        let c = self.dy.advance(dt);
        a || b || c
    }
}

/// A spring for a few px of offset: settles to a twentieth of a pixel.
fn offset(at: f64) -> Spring {
    Spring {
        eps: 0.05,
        ..Spring::unit(at, motion::ENTER)
    }
}

/// Scalar animation state, advanced with real frame deltas.
pub struct Anim {
    pub rect: RectSpring,
    pub glow: Spring,
    /// The glow's colour as drawn: it blends to a new colour (`glow_col`) rather than jumping.
    pub glow_rgb: [f32; 3],
    glow_col: [Spring; 3],
    pub hover: Spring,
    pub vis: Spring,
    pub t: f64,
    /// The scene the pill is showing.
    pub scene: Scene,
    /// The scene shown before it (its layer may still be fading out, or gone already).
    pub prev: Scene,
    /// Every scene still on screen, oldest first; the one for `scene` is last.
    layers: [Option<Layer>; MAX_LAYERS],
    pub reduce: bool,
    /// Which way the user is paging (-1 previous, +1 next), set just before a layout that
    /// changes the scene and consumed by it. 0 = not a user page change.
    pub nav: f32,
    /// 0..1 how far the bubble has split off (springs a little past 1).
    pub bubble: Spring,
    /// The bubble being shown, kept while it retracts so it can still be drawn.
    pub bubble_kind: Option<Bubble>,
    /// `nav` as it was when the current scene started: the content slid in from that side.
    pub slide: f32,
}

impl Anim {
    pub fn new(w: f64, h: f64) -> Self {
        let mut vis = Spring::unit(1.0, motion::FADE);
        vis.snap();
        let mut idle = Layer::arriving(Scene::Idle, (0.0, 0.0));
        idle.snap();
        Self {
            rect: RectSpring::new(w, h),
            glow: Spring::unit(0.0, motion::GLIDE),
            glow_rgb: [1.0; 3],
            glow_col: [Spring::unit(1.0, motion::GLIDE); 3],
            hover: Spring::unit(0.0, motion::QUICK),
            vis,
            t: 0.0,
            scene: Scene::Idle,
            prev: Scene::Idle,
            layers: [Some(idle), None, None],
            reduce: false,
            nav: 0.0,
            slide: 0.0,
            bubble: Spring::unit(0.0, motion::POP),
            bubble_kind: None,
        }
    }

    /// A quick squash-and-settle of the pill: widens and flattens a little, then the
    /// springs carry it back. Used when a drop lands.
    pub fn squash(&mut self) {
        if !self.reduce {
            self.rect.w.vel += 160.0;
            self.rect.h.vel -= 110.0;
        }
    }

    /// Point the bubble spring at the wanted state (`Some` = split off).
    pub fn set_bubble(&mut self, kind: Option<Bubble>) {
        if kind.is_some() {
            self.bubble_kind = kind;
        }
        self.bubble.set_target(kind.is_some() as u8 as f64);
        if self.reduce {
            self.bubble.snap();
        }
    }

    /// Logical px of room the bubble needs beside the pill right now.
    pub fn bubble_room(&self) -> f32 {
        if self.bubble.pos > 0.001 {
            BUBBLE_ROOM
        } else {
            0.0
        }
    }

    pub fn set_scene(&mut self, scene: Scene, size: (f64, f64), glow: ([f32; 3], f32)) {
        if scene != self.scene {
            let slide = std::mem::take(&mut self.nav) as f64;
            // The scene being left fades out from wherever it had got to, drifting up, or
            // away from the page being turned to.
            let away = if slide != 0.0 {
                (-slide * 16.0, 0.0)
            } else {
                (0.0, -4.0)
            };
            for l in self.layers.iter_mut().flatten() {
                if l.scene == self.scene {
                    l.leave(away);
                }
            }
            // A scene still on its way out comes back from where it is; a new one fades in
            // from a little above, or from the side it was paged from.
            match self.take_layer(scene) {
                Some(mut l) => {
                    l.enter();
                    self.push_layer(l);
                }
                None => {
                    let from = if slide != 0.0 {
                        (slide * 16.0, 0.0)
                    } else {
                        (0.0, 6.0)
                    };
                    self.push_layer(Layer::arriving(scene, from));
                }
            }
            self.prev = self.scene;
            self.scene = scene;
            self.slide = slide as f32;
        }
        self.rect.set_target(size.0, size.1);
        self.glow.set_target(glow.1 as f64);
        for (s, c) in self.glow_col.iter_mut().zip(glow.0) {
            s.set_target(c as f64);
        }
        // A colour nobody can see doesn't blend: the glow comes up in its own colour.
        if self.glow.pos < 0.01 {
            self.glow_col.iter_mut().for_each(Spring::snap);
        }
        if self.reduce {
            self.rect.snap();
            self.glow.snap();
            self.glow_col.iter_mut().for_each(Spring::snap);
            self.settle_layers();
        }
        self.sync_glow_rgb();
    }

    fn sync_glow_rgb(&mut self) {
        for (c, s) in self.glow_rgb.iter_mut().zip(&self.glow_col) {
            *c = s.pos.clamp(0.0, 1.0) as f32;
        }
    }

    /// Remove and return the layer showing `scene`, if it is on screen.
    fn take_layer(&mut self, scene: Scene) -> Option<Layer> {
        let i = self
            .layers
            .iter()
            .position(|l| l.is_some_and(|l| l.scene == scene))?;
        let l = self.layers[i].take();
        self.compact();
        l
    }

    /// Put `l` on top. With every slot in use the faintest layer leaving makes room.
    fn push_layer(&mut self, l: Layer) {
        if self.layers.iter().all(Option::is_some) {
            let faintest = (0..MAX_LAYERS)
                .filter(|&i| self.layers[i].is_some_and(|x| x.scene != self.scene))
                .min_by(|&a, &b| {
                    let o = |i: usize| self.layers[i].map_or(0.0, |x| x.alpha.pos);
                    o(a).total_cmp(&o(b))
                })
                .unwrap_or(0);
            self.layers[faintest] = None;
            self.compact();
        }
        if let Some(slot) = self.layers.iter_mut().find(|x| x.is_none()) {
            *slot = Some(l);
        }
    }

    /// Close the gaps, keeping the order (oldest first).
    fn compact(&mut self) {
        let mut out = [None; MAX_LAYERS];
        for (slot, l) in out.iter_mut().zip(self.layers.iter().flatten()) {
            *slot = Some(*l);
        }
        self.layers = out;
    }

    /// Reduced motion: everything where it is going, and the scenes that were leaving, gone.
    fn settle_layers(&mut self) {
        for l in self.layers.iter_mut().flatten() {
            l.snap();
        }
        let scene = self.scene;
        for l in &mut self.layers {
            if l.is_some_and(|x| x.scene != scene) {
                *l = None;
            }
        }
        self.compact();
    }

    /// How far the current scene has faded in (0..1).
    pub fn content(&self) -> f32 {
        self.layers
            .iter()
            .flatten()
            .find(|l| l.scene == self.scene)
            .map_or(1.0, |l| l.alpha.pos.clamp(0.0, 1.0) as f32)
    }

    /// How much of the scene shown before is still visible (0..1).
    pub fn out(&self) -> f32 {
        if self.prev == self.scene {
            return 0.0;
        }
        self.layers
            .iter()
            .flatten()
            .find(|l| l.scene == self.prev)
            .map_or(0.0, |l| l.alpha.pos.clamp(0.0, 1.0) as f32)
    }

    /// What to draw: every scene still visible, back to front, with its opacity and offset.
    /// The renderer draws exactly this, and the continuity test measures it.
    pub fn shown(&self) -> ShownList {
        let mut list = ShownList::default();
        let mut live = None;
        for l in self.layers.iter().flatten() {
            let s = Shown {
                scene: l.scene,
                alpha: l.alpha.pos.clamp(0.0, 1.0) as f32,
                dx: l.dx.pos as f32,
                dy: l.dy.pos as f32,
                live: l.scene == self.scene,
            };
            if s.live {
                live = Some(s);
            } else if s.alpha > 0.01 && s.scene != Scene::Sentinel {
                list.push(s);
            }
        }
        if let Some(s) = live {
            list.push(s);
        }
        list
    }

    /// Advance everything; true while any spring is still moving.
    pub fn step(&mut self, dt: f64) -> bool {
        self.t += dt;
        if self.reduce {
            self.hover.snap();
            self.vis.snap();
            self.bubble.snap();
            self.settle_layers();
            return false;
        }
        let a = self.rect.advance(dt);
        let mut b = self.glow.advance(dt);
        for s in &mut self.glow_col {
            b |= s.advance(dt);
        }
        self.sync_glow_rgb();
        let mut c = false;
        for l in self.layers.iter_mut().flatten() {
            c |= l.advance(dt);
        }
        // A scene that has finished leaving is gone.
        let scene = self.scene;
        let before = self.layers.iter().flatten().count();
        for l in &mut self.layers {
            if l.is_some_and(|x| x.scene != scene && x.alpha.pos <= 0.0 && x.alpha.settled()) {
                *l = None;
            }
        }
        if self.layers.iter().flatten().count() != before {
            self.compact();
        }
        let d = self.hover.advance(dt);
        let e = self.vis.advance(dt);
        let g = self.bubble.advance(dt);
        a || b || c || d || e || g
    }
}

/// Turns sideways scroll (touchpad swipe, tilt wheel) into pane pages: one per swipe, as
/// soon as it has moved a little.
///
/// A fast touchpad swipe is noisy: its deltas alternate high and low while the finger moves,
/// a lone event can point the wrong way, and after the finger lifts it keeps sending decaying
/// deltas (inertia) for half a second. So the deltas are smoothed, a single opposite event
/// inside a stream is ignored, and after a page the stream only pages again once it has
/// clearly slowed down and then sped up (a new swipe in the old one's tail), or after a pause.
#[derive(Debug, Default)]
pub struct Swipe {
    /// Movement since the gesture started (or re-armed).
    acc: i32,
    /// When the last delta came, and which way the gesture goes (+1 / -1).
    last: Option<Instant>,
    dir: i32,
    /// Opposite deltas in a row.
    flip: u8,
    /// Smoothed |delta|.
    ema: f32,
    /// After a page: when, the fastest it has gone since, whether it has slowed down, and
    /// the slowest since then.
    paged: Option<Instant>,
    peak: f32,
    slowed: bool,
    trough: f32,
}

/// Movement before a gesture pages (a tilt-wheel notch is 120).
pub(super) const SWIPE_STEP: i32 = 80;

/// A pause this long between deltas ends a gesture.
pub(super) const SWIPE_GAP: Duration = Duration::from_millis(100);

/// No second page sooner than this after one, however the stream behaves.
pub(super) const SWIPE_LOCK: Duration = Duration::from_millis(160);

impl Swipe {
    /// Feed one delta; `Some(+1 / -1)` when it pages (right = next).
    pub fn feed(&mut self, delta: i32, now: Instant) -> Option<i32> {
        if delta == 0 {
            return None;
        }
        let gap = self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) > SWIPE_GAP);
        self.last = Some(now);
        let sign = delta.signum();
        let mag = delta.unsigned_abs() as f32;
        if !gap && sign != self.dir {
            // One stray event the other way is noise; two in a row turn the gesture round.
            self.flip += 1;
            if self.flip < 2 {
                return None;
            }
        }
        if gap || sign != self.dir {
            *self = Swipe {
                last: self.last,
                dir: sign,
                ema: mag,
                ..Default::default()
            };
        }
        self.flip = 0;
        self.ema += 0.3 * (mag - self.ema);
        if let Some(at) = self.paged {
            self.peak = self.peak.max(self.ema);
            if self.ema < 0.45 * self.peak {
                self.slowed = true;
            }
            if self.slowed {
                self.trough = self.trough.min(self.ema);
            }
            let again = now.saturating_duration_since(at) >= SWIPE_LOCK
                && self.slowed
                && self.ema >= 1.8 * self.trough
                && mag >= 24.0;
            if !again {
                return None;
            }
            self.paged = None;
            self.acc = 0;
        }
        self.acc += delta;
        if self.acc.abs() < SWIPE_STEP {
            return None;
        }
        self.paged = Some(now);
        self.peak = self.ema;
        self.slowed = false;
        self.trough = f32::MAX;
        Some(self.dir)
    }
}
