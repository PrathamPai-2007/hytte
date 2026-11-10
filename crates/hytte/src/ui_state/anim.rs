//! The pill's springs and the two-finger swipe detector.

use super::*;

/// Scalar animation state, advanced with real frame deltas.
pub struct Anim {
    pub rect: RectSpring,
    pub glow: Spring,
    pub glow_rgb: [f32; 3],
    pub content: Spring,
    pub hover: Spring,
    pub vis: Spring,
    pub t: f64,
    pub scene: Scene,
    /// The scene being left; its content fades out (`out`) under the new one.
    pub prev: Scene,
    pub out: Spring,
    pub reduce: bool,
    /// Which way the user is paging (-1 previous, +1 next), set just before a layout that
    /// changes the scene and consumed by it. 0 = not a user page change.
    pub nav: f32,
    /// 0..1 how far the bubble has split off (springs a little past 1).
    pub bubble: Spring,
    /// The bubble being shown, kept while it retracts so it can still be drawn.
    pub bubble_kind: Option<Bubble>,
    /// `nav` as it was when the current scene started: the content slides in from that side.
    pub slide: f32,
}

impl Anim {
    pub fn new(w: f64, h: f64) -> Self {
        let mut vis = Spring::unit(1.0, 1.0, 0.22);
        vis.snap();
        Self {
            rect: RectSpring::new(w, h),
            glow: Spring::unit(0.0, 1.0, 0.30),
            glow_rgb: [1.0; 3],
            content: Spring::unit(1.0, 1.0, 0.21),
            hover: Spring::unit(0.0, 1.0, 0.16),
            vis,
            t: 0.0,
            scene: Scene::Idle,
            prev: Scene::Idle,
            out: Spring::unit(0.0, 1.0, 0.17),
            reduce: false,
            nav: 0.0,
            slide: 0.0,
            bubble: Spring::unit(0.0, 0.72, 0.34),
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
            // Fade the old content out from wherever its own fade-in had got to,
            // instead of cutting it in one frame.
            self.prev = self.scene;
            self.out.pos = self.content.pos.clamp(0.0, 1.0);
            self.out.vel = 0.0;
            self.out.set_target(0.0);
            self.scene = scene;
            self.slide = std::mem::take(&mut self.nav);
            self.content.pos = 0.0;
            self.content.vel = 0.0;
        }
        self.content.set_target(1.0);
        self.rect.set_target(size.0, size.1);
        self.glow.set_target(glow.1 as f64);
        self.glow_rgb = glow.0;
        if self.reduce {
            self.rect.snap();
            self.glow.snap();
            self.content.snap();
            self.out.snap();
        }
    }

    /// Advance everything; true while any spring is still moving.
    pub fn step(&mut self, dt: f64) -> bool {
        self.t += dt;
        if self.reduce {
            self.hover.snap();
            self.vis.snap();
            self.bubble.snap();
            return false;
        }
        let a = self.rect.advance(dt);
        let b = self.glow.advance(dt);
        let c = self.content.advance(dt);
        let d = self.hover.advance(dt);
        let e = self.vis.advance(dt);
        let f = self.out.advance(dt);
        let g = self.bubble.advance(dt);
        a || b || c || d || e || f || g
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
