//! Resource-hog alert: a process that keeps using a lot of CPU or memory.
//!
//! Sampling rides on the port watcher's existing tick (`ports.rs`), so it adds no wake-up
//! of its own. `Tracker` is the pure part (thresholds and "for how long"); `Sampler` is the
//! Windows part that reads per-process CPU time and memory.

use crate::config::Hog;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// One process at one sample.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub pid: u32,
    pub exe: String,
    /// Percent of the whole machine (all cores = 100).
    pub cpu: f32,
    pub mem_mb: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    Cpu(u32),
    Memory(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub pid: u32,
    pub exe: String,
    pub why: Why,
    pub secs: u64,
}

impl Alert {
    /// The sentence the chip shows.
    pub fn summary(&self) -> String {
        match self.why {
            Why::Cpu(p) => format!("{} has used {p}% CPU for {} s", self.exe, self.secs),
            Why::Memory(mb) => format!(
                "{} is using {:.1} GB of memory",
                self.exe,
                mb as f64 / 1024.0
            ),
        }
    }
}

#[derive(Default)]
struct Over {
    since: Option<Instant>,
    alerted: bool,
}

/// Decides when a process has been over a limit long enough to mention it, once.
#[derive(Default)]
pub struct Tracker {
    over: HashMap<u32, Over>,
}

impl Tracker {
    /// Feed one sample of every running process. Returns the alerts that just became due.
    pub fn update(&mut self, now: Instant, samples: &[Sample], cfg: &Hog) -> Vec<Alert> {
        let hold = Duration::from_secs(cfg.secs);
        let seen: HashSet<u32> = samples.iter().map(|s| s.pid).collect();
        self.over.retain(|pid, _| seen.contains(pid));
        let mut out = vec![];
        for s in samples {
            let cpu_hot = s.cpu >= cfg.cpu_pct as f32;
            let mem_hot = s.mem_mb >= cfg.mem_mb;
            if !(cpu_hot || mem_hot) {
                self.over.remove(&s.pid);
                continue;
            }
            let o = self.over.entry(s.pid).or_default();
            let since = *o.since.get_or_insert(now);
            // Memory is a level, not a burst: it only has to hold for a moment (one sample
            // pair), where CPU has to stay high for the configured time.
            let needed = if cpu_hot { hold } else { Duration::ZERO };
            if !o.alerted && now.saturating_duration_since(since) >= needed {
                o.alerted = true;
                out.push(Alert {
                    pid: s.pid,
                    exe: s.exe.clone(),
                    why: if cpu_hot {
                        Why::Cpu(s.cpu.round() as u32)
                    } else {
                        Why::Memory(s.mem_mb)
                    },
                    secs: now.saturating_duration_since(since).as_secs().max(cfg.secs),
                });
            }
        }
        out
    }
}

/// Never worth an alert: the idle process, System, and ourselves.
pub fn exempt(pid: u32, exe: &str) -> bool {
    pid <= 4
        || pid == std::process::id()
        || exe.eq_ignore_ascii_case("hytte.exe")
        || exe.eq_ignore_ascii_case("System Idle Process")
}

#[cfg(windows)]
pub use win::Sampler;

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn ticks(ft: FILETIME) -> u64 {
        (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64
    }

    /// Reads per-process CPU time and memory; CPU% needs two reads, so the first sample of a
    /// process reports 0.
    pub struct Sampler {
        prev: HashMap<u32, u64>,
        at: Option<Instant>,
        cores: f32,
    }

    impl Sampler {
        pub fn new() -> Self {
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f32;
            Self {
                prev: HashMap::new(),
                at: None,
                cores,
            }
        }

        pub fn sample(&mut self, now: Instant) -> Vec<Sample> {
            let elapsed = self.at.map(|t| now.saturating_duration_since(t).as_secs_f32());
            // An early wake-up (the panel opened) gives a noisy interval: skip CPU that time.
            let usable = elapsed.filter(|e| *e >= 1.0);
            let mut out = vec![];
            let mut cpu_now: HashMap<u32, u64> = HashMap::new();
            // SAFETY: plain Win32 snapshot/iteration with correctly sized structs; every
            // opened handle is closed.
            unsafe {
                let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                    return out;
                };
                let mut e = PROCESSENTRY32W {
                    dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                    ..Default::default()
                };
                let mut ok = Process32FirstW(snap, &mut e).is_ok();
                while ok {
                    let pid = e.th32ProcessID;
                    let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
                    let exe = String::from_utf16_lossy(&e.szExeFile[..len]);
                    if !exempt(pid, &exe) {
                        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                            let (mut c, mut x, mut k, mut u) = (
                                FILETIME::default(),
                                FILETIME::default(),
                                FILETIME::default(),
                                FILETIME::default(),
                            );
                            if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u).is_ok() {
                                let t = ticks(k) + ticks(u);
                                cpu_now.insert(pid, t);
                                let cpu = match (usable, self.prev.get(&pid)) {
                                    (Some(el), Some(p)) if t >= *p => {
                                        // 100 ns ticks over the interval, spread over all cores.
                                        (t - p) as f32 / (el * 1e7) / self.cores * 100.0
                                    }
                                    _ => 0.0,
                                };
                                let mut mc = PROCESS_MEMORY_COUNTERS {
                                    cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                                    ..Default::default()
                                };
                                let mem = if K32GetProcessMemoryInfo(h, &mut mc, mc.cb).as_bool() {
                                    mc.WorkingSetSize as u64 / (1024 * 1024)
                                } else {
                                    0
                                };
                                out.push(Sample {
                                    pid,
                                    exe,
                                    cpu,
                                    mem_mb: mem,
                                });
                            }
                            let _ = CloseHandle(h);
                        }
                    }
                    ok = Process32NextW(snap, &mut e).is_ok();
                }
                let _ = CloseHandle(snap);
            }
            // Keep the baseline only when the interval was long enough to use, or there is none.
            if usable.is_some() || self.at.is_none() {
                self.prev = cpu_now;
                self.at = Some(now);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Hog {
        Hog {
            enabled: true,
            cpu_pct: 80,
            mem_mb: 4096,
            secs: 30,
        }
    }

    fn s(pid: u32, cpu: f32, mem_mb: u64) -> Sample {
        Sample {
            pid,
            exe: format!("p{pid}.exe"),
            cpu,
            mem_mb,
        }
    }

    #[test]
    fn cpu_must_stay_high_for_the_configured_time_and_alerts_once() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        assert!(t.update(at(0), &[s(1, 95.0, 100)], &cfg()).is_empty());
        assert!(t.update(at(20), &[s(1, 90.0, 100)], &cfg()).is_empty());
        let a = t.update(at(31), &[s(1, 92.0, 100)], &cfg());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].why, Why::Cpu(92));
        assert!(a[0].summary().contains("92% CPU"));
        // Still hot: no second alert.
        assert!(t.update(at(60), &[s(1, 92.0, 100)], &cfg()).is_empty());
    }

    #[test]
    fn a_dip_resets_the_clock_and_a_new_burst_alerts_again() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        t.update(at(0), &[s(1, 95.0, 0)], &cfg());
        t.update(at(20), &[s(1, 10.0, 0)], &cfg());
        // Hot again from 25: not due until 55.
        t.update(at(25), &[s(1, 95.0, 0)], &cfg());
        assert!(t.update(at(40), &[s(1, 95.0, 0)], &cfg()).is_empty());
        assert_eq!(t.update(at(56), &[s(1, 95.0, 0)], &cfg()).len(), 1);
    }

    #[test]
    fn memory_alerts_at_once_and_a_gone_process_is_forgotten() {
        let mut t = Tracker::default();
        let t0 = Instant::now();
        let a = t.update(t0, &[s(2, 1.0, 5200)], &cfg());
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].why, Why::Memory(5200));
        assert!(a[0].summary().contains("5.1 GB"));
        // The process exits and its pid is reused by a quiet one, then a hot one: alerts again.
        t.update(t0, &[], &cfg());
        assert!(t.update(t0, &[s(2, 1.0, 10)], &cfg()).is_empty());
        assert_eq!(t.update(t0, &[s(2, 1.0, 5000)], &cfg()).len(), 1);
    }

    #[test]
    fn exemptions() {
        assert!(exempt(0, "System Idle Process"));
        assert!(exempt(4, "System"));
        assert!(exempt(std::process::id(), "anything.exe"));
        assert!(exempt(999_999, "Hytte.exe"));
        assert!(!exempt(999_999, "node.exe"));
    }
}
