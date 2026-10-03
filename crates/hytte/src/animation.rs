//! Critically-damped-ish spring integrator.
//! Ticks only while animating; idle = no timer, no frames.

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

    /// Advance by a real frame delta using stable sub-steps.
    pub fn advance(&mut self, dt: f64) -> bool {
        let mut left = dt.min(0.05);
        let mut moving = true;
        while left > 1e-6 && moving {
            let h = left.min(1.0 / 240.0);
            moving = self.tick(h);
            left -= h;
        }
        moving
    }

    pub fn set_target(&mut self, t: f64) {
        self.target = t;
    }

    pub fn settled(&self) -> bool {
        (self.pos - self.target).abs() < self.eps && self.vel.abs() < self.eps * 4.0
    }

    /// Semi-implicit Euler at display rate. Returns true while still moving.
    pub fn tick(&mut self, dt: f64) -> bool {
        let omega = 2.0 * std::f64::consts::PI / self.period;
        let k = omega * omega;
        let c = 2.0 * self.zeta * omega;
        let acc = -k * (self.pos - self.target) - c * self.vel;
        self.vel += acc * dt;
        self.pos += self.vel * dt;
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
}
