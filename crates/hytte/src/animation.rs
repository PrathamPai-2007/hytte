//! Critically-damped-ish spring integrator.
//! Ticks only while animating; idle = no timer, no frames.

/// Global tempo: >1 slows every spring (user found the defaults a touch fast).
const SLOW: f64 = 1.18;

/// Spring params: zeta ≈ 0.78, period ≈ 0.28 s.
#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub zeta: f64,
    pub period: f64,
    pub pos: f64,
    pub vel: f64,
    pub target: f64,
    /// Settle threshold (position and velocity).
    pub eps: f64,
}

impl Spring {
    pub fn new(pos: f64) -> Self {
        Self {
            zeta: 0.78,
            period: 0.28,
            pos,
            vel: 0.0,
            target: pos,
            eps: 0.15,
        }
    }

    /// Spring for normalised 0..1 values (opacity, glow, progress).
    pub fn unit(pos: f64, zeta: f64, period: f64) -> Self {
        Self { zeta, period, eps: 0.002, ..Self::new(pos) }
    }

    pub fn snap(&mut self) {
        self.pos = self.target;
        self.vel = 0.0;
    }

    /// Advance by a real frame delta. Exact for any dt, so frame jitter and
    /// refresh rate do not change the motion.
    pub fn advance(&mut self, dt: f64) -> bool {
        self.tick(dt.clamp(0.0, 0.05))
    }

    pub fn set_target(&mut self, t: f64) {
        self.target = t;
    }

    pub fn settled(&self) -> bool {
        (self.pos - self.target).abs() < self.eps && self.vel.abs() < self.eps * 4.0
    }

    /// Closed-form damped-spring step (under, critically and over damped).
    /// Returns true while still moving.
    pub fn tick(&mut self, dt: f64) -> bool {
        let w = 2.0 * std::f64::consts::PI / (self.period * SLOW);
        let z = self.zeta;
        let (x, v) = (self.pos - self.target, self.vel);
        let (nx, nv) = if (z - 1.0).abs() < 1e-6 {
            let e = (-w * dt).exp();
            let c = v + w * x;
            ((x + c * dt) * e, (v - w * c * dt) * e)
        } else if z < 1.0 {
            let wd = w * (1.0 - z * z).sqrt();
            let e = (-z * w * dt).exp();
            let (s, c) = (wd * dt).sin_cos();
            let b = (v + z * w * x) / wd;
            let q = x * c + b * s;
            (e * q, e * (-z * w * q + wd * (b * c - x * s)))
        } else {
            let r = w * (z * z - 1.0).sqrt();
            let (r1, r2) = (-w * z + r, -w * z - r);
            let c2 = (v - r1 * x) / (r2 - r1);
            let c1 = x - c2;
            let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        };
        self.pos = self.target + nx;
        self.vel = nv;
        if self.settled() {
            self.pos = self.target;
            self.vel = 0.0;
            return false;
        }
        true
    }
}

/// Rectangular spring for window bounds (x is fixed centre; we animate w/h).
#[derive(Debug, Clone, Copy)]
pub struct RectSpring {
    pub w: Spring,
    pub h: Spring,
}

impl RectSpring {
    pub fn new(w: f64, h: f64) -> Self {
        Self {
            w: Spring::new(w),
            h: Spring::new(h),
        }
    }
    pub fn set_target(&mut self, w: f64, h: f64) {
        self.w.set_target(w);
        self.h.set_target(h);
    }
    pub fn advance(&mut self, dt: f64) -> bool {
        let a = self.w.advance(dt);
        let b = self.h.advance(dt);
        a || b
    }
    pub fn snap(&mut self) {
        self.w.snap();
        self.h.snap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converges() {
        let mut s = Spring::new(0.0);
        s.set_target(200.0);
        let mut moving = true;
        for _ in 0..600 {
            moving = s.tick(1.0 / 120.0);
            if !moving {
                break;
            }
        }
        assert!(!moving);
        assert!((s.pos - 200.0).abs() < 0.5);
    }

    #[test]
    fn unit_spring_settles_and_snap_works() {
        let mut s = Spring::unit(0.0, 1.0, 0.2);
        s.set_target(1.0);
        let mut n = 0;
        while s.advance(1.0 / 60.0) && n < 600 {
            n += 1;
        }
        assert!(n < 600 && (s.pos - 1.0).abs() < 0.01);
        s.set_target(0.0);
        s.snap();
        assert_eq!(s.pos, 0.0);
    }

    #[test]
    fn step_size_does_not_change_the_motion() {
        for zeta in [0.78, 1.0, 1.4] {
            let run = |dt: f64| {
                let mut s = Spring::new(0.0);
                s.zeta = zeta;
                s.eps = 0.0;
                s.set_target(100.0);
                for _ in 0..(0.2 / dt).round() as usize {
                    s.tick(dt);
                }
                s.pos
            };
            assert!((run(1.0 / 240.0) - run(1.0 / 30.0)).abs() < 1e-6, "zeta {zeta}");
        }
    }

    #[test]
    fn retarget_keeps_velocity() {
        let mut s = Spring::new(0.0);
        s.set_target(100.0);
        s.advance(0.05);
        let v = s.vel;
        assert!(v > 0.0);
        s.set_target(0.0);
        s.advance(0.001);
        assert!(s.vel > 0.0 && s.vel < v, "momentum carries through a reversal");
    }
}
