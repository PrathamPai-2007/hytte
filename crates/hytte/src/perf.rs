//! Frame-time telemetry, enabled with `HYTTE_PERF=1`. Disabled = one bool check per frame.
//! Logs p50/p99/max of draw+present cost and of the gap between frames, plus
//! frames that took more than 1.5x the median gap (a missed vblank).

pub struct Perf {
    on: bool,
    cost_us: Vec<u32>,
    gap_us: Vec<u32>,
}

const BATCH: usize = 240;

impl Perf {
    pub fn new() -> Self {
        let on = std::env::var_os("HYTTE_PERF").is_some_and(|v| v != "0");
        Self {
            on,
            cost_us: Vec::new(),
            gap_us: Vec::new(),
        }
    }

    pub fn record(&mut self, gap_us: u32, cost_us: u32) {
        if !self.on {
            return;
        }
        self.cost_us.push(cost_us);
        self.gap_us.push(gap_us);
        if self.cost_us.len() >= BATCH {
            crate::logging::line(&self.report());
            self.cost_us.clear();
            self.gap_us.clear();
        }
    }

    fn report(&mut self) -> String {
        let (c50, c99, cmax) = pct(&mut self.cost_us);
        let (g50, g99, gmax) = pct(&mut self.gap_us);
        let missed = self
            .gap_us
            .iter()
            .filter(|&&g| g as f64 > g50 as f64 * 1.5)
            .count();
        format!(
            "perf n={} cost_us p50={c50} p99={c99} max={cmax} | gap_us p50={g50} p99={g99} max={gmax} | missed={missed}",
            self.cost_us.len()
        )
    }
}

fn pct(v: &mut [u32]) -> (u32, u32, u32) {
    v.sort_unstable();
    let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
    (at(0.5), at(0.99), *v.last().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let mut v: Vec<u32> = (1..=100).collect();
        assert_eq!(pct(&mut v), (50, 99, 100));
    }
}
