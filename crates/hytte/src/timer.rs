//! One glanceable timer (plain / focus / break). Wall-clock based so sleep and
//! restarts stay correct; the UI redraws it at 1 Hz only while it is active.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerKind {
    Plain,
    Focus,
    Break,
}

impl TimerKind {
    pub fn name(self) -> &'static str {
        match self {
            TimerKind::Plain => "Timer",
            TimerKind::Focus => "Focus",
            TimerKind::Break => "Break",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timer {
    pub kind: TimerKind,
    pub total_ms: u64,
    /// Unix epoch ms.
    pub ends_ms: u64,
    /// Remaining ms while paused.
    #[serde(default)]
    pub paused_ms: Option<u64>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Timer {
    pub fn start(kind: TimerKind, minutes: u32, now: u64) -> Self {
        let total_ms = minutes.max(1) as u64 * 60_000;
        Self { kind, total_ms, ends_ms: now + total_ms, paused_ms: None }
    }
    pub fn remaining_ms(&self, now: u64) -> u64 {
        self.paused_ms.unwrap_or_else(|| self.ends_ms.saturating_sub(now))
    }
    pub fn toggle_pause(&mut self, now: u64) {
        match self.paused_ms.take() {
            Some(left) => self.ends_ms = now + left,
            None => self.paused_ms = Some(self.remaining_ms(now)),
        }
    }
    /// Fraction left, 0..1.
    pub fn frac(&self, now: u64) -> f32 {
        (self.remaining_ms(now) as f32 / self.total_ms.max(1) as f32).clamp(0.0, 1.0)
    }
    pub fn add_minutes(&mut self, m: u64) {
        self.ends_ms += m * 60_000;
        self.total_ms += m * 60_000;
        if let Some(p) = &mut self.paused_ms {
            *p += m * 60_000;
        }
    }
}

/// "m:ss", or "h:mm:ss" past an hour. Rounds up so 0:00 only shows at the end.
pub fn fmt(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Minutes of the break that follows focus session number `done` (1-based).
pub fn break_minutes(done: u32, rounds: u32, short: u32, long: u32) -> u32 {
    if rounds > 0 && done.is_multiple_of(rounds) { long } else { short }
}

/// Wheel notches -> minutes: 1-minute steps under 10, 5-minute steps above.
pub fn nudge(m: u32, steps: i32) -> u32 {
    let mut m = m as i32;
    for _ in 0..steps.abs() {
        m += match (steps > 0, m) {
            (true, 0..=9) => 1,
            (true, _) => 5,
            (false, 0..=10) => -1,
            (false, _) => -5,
        };
    }
    m.clamp(1, 180) as u32
}

fn path() -> std::path::PathBuf {
    crate::config::data_dir().join("timer.json")
}

pub fn save(t: Option<&Timer>) {
    match t.and_then(|t| serde_json::to_string(t).ok()) {
        Some(s) => {
            let _ = std::fs::create_dir_all(crate::config::data_dir());
            let _ = std::fs::write(path(), s);
        }
        None => {
            let _ = std::fs::remove_file(path());
        }
    }
}

pub fn load() -> Option<Timer> {
    serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()
}

/// Two soft decaying sine notes (E5 then A5), 22.05 kHz mono 16-bit WAV.
pub fn chime_wav() -> Vec<u8> {
    const SR: u32 = 22_050;
    let mut pcm: Vec<i16> = vec![];
    for (hz, secs) in [(659.25f32, 0.55f32), (880.0, 0.85)] {
        let n = (SR as f32 * secs) as usize;
        for i in 0..n {
            let t = i as f32 / SR as f32;
            let env = (t / 0.01).min(1.0) * (-5.5 * t).exp();
            pcm.push(((t * hz * std::f32::consts::TAU).sin() * env * 0.35 * i16::MAX as f32) as i16);
        }
    }
    let data = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend(b"RIFF");
    w.extend((36 + data).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes()); // PCM
    w.extend(1u16.to_le_bytes()); // mono
    w.extend(SR.to_le_bytes());
    w.extend((SR * 2).to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(data.to_le_bytes());
    for s in pcm {
        w.extend(s.to_le_bytes());
    }
    w
}

pub fn play_chime() {
    #[cfg(windows)]
    {
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY};
        static WAV: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        let w = WAV.get_or_init(chime_wav);
        unsafe {
            let _ = PlaySoundW(windows::core::PCWSTR(w.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_survives_a_sleep_gap() {
        let t = Timer::start(TimerKind::Focus, 25, 1_000);
        assert_eq!(t.remaining_ms(1_000), 25 * 60_000);
        assert_eq!(t.remaining_ms(1_000 + 10 * 60_000), 15 * 60_000);
        assert_eq!(t.remaining_ms(u64::MAX), 0, "past the end clamps to zero");
        assert!((t.frac(1_000 + 5 * 60_000) - 0.8).abs() < 1e-4);
    }

    #[test]
    fn pause_freezes_and_resume_continues() {
        let mut t = Timer::start(TimerKind::Plain, 10, 0);
        t.toggle_pause(60_000);
        assert_eq!(t.remaining_ms(999_999), 9 * 60_000);
        t.add_minutes(1);
        t.toggle_pause(100_000);
        assert_eq!(t.remaining_ms(100_000 + 60_000), 9 * 60_000);
    }

    #[test]
    fn extend_and_format() {
        let mut t = Timer::start(TimerKind::Plain, 1, 0);
        t.add_minutes(5);
        assert_eq!(t.remaining_ms(0), 6 * 60_000);
        assert_eq!(fmt(61_000), "1:01");
        assert_eq!(fmt(1), "0:01");
        assert_eq!(fmt(3_600_000), "1:00:00");
    }

    #[test]
    fn long_break_every_fourth_focus() {
        let b: Vec<u32> = (1..=8).map(|n| break_minutes(n, 4, 5, 15)).collect();
        assert_eq!(b, [5, 5, 5, 15, 5, 5, 5, 15]);
    }

    #[test]
    fn wheel_steps_by_one_then_five() {
        assert_eq!(nudge(25, 2), 35);
        assert_eq!(nudge(25, -3), 10);
        assert_eq!(nudge(10, -1), 9);
        assert_eq!(nudge(1, -9), 1);
        assert_eq!(nudge(178, 5), 180);
    }

    #[test]
    fn persistence_round_trips() {
        let t = Timer::start(TimerKind::Break, 5, 42);
        let back: Timer = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!((back.kind, back.ends_ms), (TimerKind::Break, t.ends_ms));
    }

    #[test]
    fn chime_is_a_valid_wav() {
        let w = chime_wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert!(w.len() < 100_000);
    }
}
