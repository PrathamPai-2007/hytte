//! Keyed springs for what only the renderer knows about: a control under the pointer, a bar's
//! value, where a row sits. A scene asks `Renderer::tw(key, target, feel)` while it draws and
//! gets back where that value is this frame.
//!
//! - A key seen for the first time starts at its target: nothing animates when a pane opens.
//! - A key not asked for during a frame is forgotten, so nothing is kept for what is gone.
//! - While any value is still moving `Renderer::tweening` says so, and the frame clock keeps
//!   ticking at the refresh rate, as it does for the pill's own springs.
//! - With reduced motion every value is its target at once.
//!
//! Steady state allocates nothing: the map only grows when a new key appears.

use crate::animation::{Feel, Spring};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// A key for `Renderer::tw`: a tag naming the value, and whatever tells its instances apart
/// (a task's id, a button's action, a row's index).
pub fn key(tag: &str, id: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    tag.hash(&mut h);
    id.hash(&mut h);
    h.finish()
}

#[derive(Default)]
pub struct Tweens {
    /// Each value's spring and the frame it was last asked for.
    map: HashMap<u64, (Spring, u32)>,
    frame: u32,
    dt: f64,
    /// False: every value is its target (reduced motion).
    live: bool,
    moving: bool,
}

impl Tweens {
    /// A frame starts: `dt` is its time step (already scaled by the speed setting).
    pub fn begin(&mut self, dt: f64, live: bool) {
        self.frame = self.frame.wrapping_add(1);
        self.dt = dt;
        self.live = live;
        self.moving = false;
    }

    /// The frame is drawn: forget what it didn't ask for.
    pub fn end(&mut self) {
        let f = self.frame;
        self.map.retain(|_, (_, seen)| *seen == f);
    }

    /// Where the value `key` is this frame, heading for `target` with `feel`. `eps` is how close
    /// counts as arrived (0.002 for 0..1 values, a twentieth of a pixel for positions).
    pub fn get(&mut self, key: u64, target: f32, feel: Feel, eps: f64) -> f32 {
        if !self.live {
            self.map.remove(&key);
            return target;
        }
        let frame = self.frame;
        let (s, seen) = self.map.entry(key).or_insert_with(|| {
            let s = Spring {
                eps,
                ..Spring::unit(target as f64, feel)
            };
            (s, frame)
        });
        // Asked twice in one frame (a scene drawn twice while it fades): advance once.
        if *seen != frame {
            *seen = frame;
            s.zeta = feel.zeta;
            s.period = feel.period;
            s.set_target(target as f64);
            s.advance(self.dt);
        } else {
            s.set_target(target as f64);
        }
        if !s.settled() {
            self.moving = true;
        }
        s.pos as f32
    }

    /// Put the value `key` at `v` now, settled (a drag: it follows the pointer exactly, and
    /// eases from there once let go).
    pub fn set(&mut self, key: u64, v: f32, feel: Feel, eps: f64) {
        let s = Spring {
            eps,
            ..Spring::unit(v as f64, feel)
        };
        self.map.insert(key, (s, self.frame));
    }

    /// Where the value `key` is, without moving it (None if it isn't being drawn).
    pub fn peek(&self, key: u64) -> Option<f32> {
        self.map.get(&key).map(|(s, _)| s.pos as f32)
    }

    /// Give the value `key` a push (px or units per second): it springs back to its target.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "first used by the scroll bounce (Task 16)")
    )]
    pub fn kick(&mut self, key: u64, velocity: f32) {
        if !self.live {
            return;
        }
        if let Some((s, _)) = self.map.get_mut(&key) {
            s.vel += velocity as f64;
            self.moving = true;
        }
    }

    /// A value moved this frame and hasn't arrived.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Forget every value (the next frame starts each at its target).
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.map.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::motion;

    fn frame(t: &mut Tweens, target: f32) -> f32 {
        t.begin(1.0 / 120.0, true);
        let v = t.get(key("x", 1), target, motion::QUICK, 0.002);
        t.end();
        v
    }

    #[test]
    fn a_new_value_starts_at_its_target_and_then_eases() {
        let mut t = Tweens::default();
        assert_eq!(frame(&mut t, 0.0), 0.0);
        assert!(!t.moving(), "nothing moves on first sight");
        let v = frame(&mut t, 1.0);
        assert!(v > 0.0 && v < 0.2, "{v}");
        assert!(t.moving());
        let mut n = 0;
        while t.moving() && n < 600 {
            frame(&mut t, 1.0);
            n += 1;
        }
        assert!(n < 120, "settles within a second: {n} frames");
        assert_eq!(frame(&mut t, 1.0), 1.0);
        assert!(!t.moving(), "settled: the frame clock may stop");
    }

    #[test]
    fn what_is_not_asked_for_is_forgotten() {
        let mut t = Tweens::default();
        frame(&mut t, 1.0);
        assert_eq!(t.len(), 1);
        t.begin(1.0 / 120.0, true);
        t.end();
        assert_eq!(t.len(), 0);
        // So it comes back at its target, not easing from the old value.
        assert_eq!(frame(&mut t, 0.3), 0.3);
    }

    #[test]
    fn reduced_motion_jumps_and_a_kick_springs_back() {
        let mut t = Tweens::default();
        frame(&mut t, 0.0);
        t.begin(1.0 / 120.0, false);
        assert_eq!(t.get(key("x", 1), 1.0, motion::QUICK, 0.002), 1.0);
        assert!(!t.moving());
        t.end();
        frame(&mut t, 0.0);
        t.kick(key("x", 1), 5.0);
        let v = frame(&mut t, 0.0);
        assert!(v > 0.0, "pushed off its target: {v}");
        for _ in 0..240 {
            frame(&mut t, 0.0);
        }
        assert_eq!(frame(&mut t, 0.0), 0.0, "and back");
    }
}
