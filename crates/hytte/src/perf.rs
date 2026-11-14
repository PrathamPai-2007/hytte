//! Frame-time telemetry, enabled with `HYTTE_PERF=1`. Disabled = one bool check per frame.
//! Logs p50/p99/max of draw+present cost and of the gap between frames, plus
//! frames that took more than 1.5x the median gap (a missed vblank). Frames that come every
//! vblank (a spring is moving, or ambient motion in an open pill on a 60 Hz display) and frames
//! that come every few (`every=N`: slower ambient motion) are kept apart: they run at different
//! rates.
//!
//! `cost_us` is what the UI thread waits for a frame. The CPU renderer (WARP) spreads the
//! rasterising over a pool of threads, so the processor time a frame really takes is several
//! times that: `cpu_us` is the whole process's CPU time per frame, and the number to budget.

pub struct Perf {
    on: bool,
    every: Series,
    slower: Series,
    /// The process's CPU time (100 ns units) when the last frame was recorded.
    cpu_mark: u64,
}

#[derive(Default)]
struct Series {
    cost_us: Vec<u32>,
    draw_us: Vec<u32>,
    gap_us: Vec<u32>,
    /// Process CPU time spent over these frames, in 100 ns units.
    cpu: u64,
}

const BATCH: usize = 240;

impl Perf {
    pub fn new() -> Self {
        let on = std::env::var_os("HYTTE_PERF").is_some_and(|v| v != "0");
        Self {
            on,
            every: Series::default(),
            slower: Series::default(),
            cpu_mark: 0,
        }
    }

    /// A frame that isn't recorded (the first after idle): CPU time starts counting from here.
    pub fn skip(&mut self) {
        if self.on {
            self.cpu_mark = process_cpu();
        }
    }

    /// `draw_us` is the paint alone; `cost_us` is paint plus present. `stride` is how many
    /// vblanks the frame clock gave this frame; `hz` is the display's refresh rate (0 when
    /// unknown).
    pub fn record(&mut self, stride: u32, hz: f64, gap_us: u32, draw_us: u32, cost_us: u32) {
        if !self.on {
            return;
        }
        let cpu = process_cpu();
        let spent = cpu.saturating_sub(self.cpu_mark);
        self.cpu_mark = cpu;
        let s = if stride <= 1 {
            &mut self.every
        } else {
            &mut self.slower
        };
        s.cost_us.push(cost_us);
        s.draw_us.push(draw_us);
        s.gap_us.push(gap_us);
        s.cpu += spent;
        if s.cost_us.len() >= BATCH {
            crate::logging::line(&s.report(stride.max(1), hz));
            *s = Series::default();
        }
    }
}

impl Series {
    fn report(&mut self, stride: u32, hz: f64) -> String {
        let (c50, c99, cmax) = pct(&mut self.cost_us);
        let (d50, d99, _) = pct(&mut self.draw_us);
        let (g50, g99, gmax) = pct(&mut self.gap_us);
        let missed = self
            .gap_us
            .iter()
            .filter(|&&g| g as f64 > g50 as f64 * 1.5)
            .count();
        let cpu_us = self.cpu / 10 / self.cost_us.len().max(1) as u64;
        format!(
            "perf every={stride} n={} hz={hz:.0} cpu_us={cpu_us} cost_us p50={c50} p99={c99} max={cmax} | draw_us p50={d50} p99={d99} | gap_us p50={g50} p99={g99} max={gmax} | missed={missed}",
            self.cost_us.len()
        )
    }
}

/// User plus kernel time of this process so far, in 100 ns units.
fn process_cpu() -> u64 {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let mut t = [FILETIME::default(); 4];
        let [a, b, k, u] = &mut t;
        // SAFETY: four valid out-structs; the pseudo-handle needs no closing.
        if unsafe { GetProcessTimes(GetCurrentProcess(), a, b, k, u) }.is_ok() {
            let ticks = |f: &FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
            return ticks(&t[2]) + ticks(&t[3]);
        }
    }
    0
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

    #[test]
    fn the_line_names_its_pace_and_the_refresh_rate() {
        let mut s = Series::default();
        for g in [6944, 6944, 6944, 13888] {
            s.cost_us.push(900);
            s.draw_us.push(700);
            s.gap_us.push(g);
        }
        s.cpu = 4 * 52_000; // 5.2 ms a frame, in 100 ns units
        let line = s.report(5, 144.0);
        assert!(
            line.starts_with("perf every=5 n=4 hz=144 cpu_us=5200 "),
            "{line}"
        );
        assert!(line.ends_with("missed=1"), "{line}");
    }
}
