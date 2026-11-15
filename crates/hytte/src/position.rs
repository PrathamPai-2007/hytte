//! Where the pill sits along the top of each monitor, set by dragging its top strip.
//!
//! Each monitor keeps its own spot, as how far the pill's centre is from the monitor's centre,
//! in fractions of the monitor's width, so it survives a change of resolution, scale or
//! pill size. Kept in `position.json` (state, not a setting). A monitor with no entry is
//! centred, which is how the pill always sat before.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The least gap between the pill and either edge of its monitor, in logical px.
pub const MARGIN: f32 = 8.0;

/// Pixels the pointer moves before a press on the top strip becomes a drag, in logical px.
pub const THRESHOLD: f32 = 6.0;

/// The narrowest the pill gets (the bare idle pill), in logical px. A drag may go as far as
/// this one could, so a small pill can sit at the very edge; a wider pane or the drag card
/// itself is kept on screen when laid out, and the pill returns to its spot when it shrinks.
pub const MIN_PILL: f32 = 100.0;

/// Height of the strip that grabs the pill, in logical px (a thin idle pill is all strip).
pub const STRIP: f32 = 8.0;

/// Per-monitor offsets, by monitor name (`\\.\DISPLAY1`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Positions {
    map: BTreeMap<String, f32>,
}

impl Positions {
    pub fn load() -> Positions {
        let text = std::fs::read_to_string(path()).unwrap_or_default();
        Positions::parse(&text)
    }

    pub fn parse(text: &str) -> Positions {
        let map: BTreeMap<String, f32> = serde_json::from_str(text).unwrap_or_default();
        // A hand-edited or damaged value can't push the pill off its monitor: clamp on read.
        Positions {
            map: map
                .into_iter()
                .filter(|(_, v)| v.is_finite())
                .map(|(k, v)| (k, v.clamp(-0.5, 0.5)))
                .collect(),
        }
    }

    pub fn save(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.map) {
            let _ = crate::setup::write_file(&path(), t.as_bytes());
        }
    }

    /// The saved offset for a monitor (0 when it has none).
    pub fn get(&self, monitor: &str) -> f32 {
        self.map.get(monitor).copied().unwrap_or(0.0)
    }

    /// Remember an offset; an offset of (almost) zero forgets the entry.
    pub fn set(&mut self, monitor: &str, frac: f32) {
        if frac.abs() < 0.0005 {
            self.map.remove(monitor);
        } else {
            self.map.insert(monitor.to_string(), frac.clamp(-0.5, 0.5));
        }
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Hytte").join("position.json")
}

/// How far the pill may move from the monitor's centre either way, in px: it must stay
/// `margin` px inside both edges. Zero when it is as wide as the monitor.
pub fn max_shift(mon_w: i32, pill_w: i32, margin: i32) -> i32 {
    ((mon_w - pill_w) / 2 - margin).max(0)
}

/// A saved fraction as pixels for this monitor and pill, clamped to stay on the monitor.
/// `right_extra` is how far something (the second activity's bubble) sticks out to the right
/// of the pill. The saved fraction itself is untouched, so a wide pane near an edge slides in
/// and the pill returns to its spot when the pane closes.
pub fn offset_px(frac: f32, mon_w: i32, pill_w: i32, right_extra: i32, margin: i32) -> i32 {
    let want = (frac * mon_w as f32).round() as i32;
    let lo = -max_shift(mon_w, pill_w, margin);
    // The right side also has to hold what sticks out; never less room than the left needs.
    let hi = (max_shift(mon_w, pill_w, margin) - right_extra.max(0)).max(lo);
    want.clamp(lo, hi)
}

/// A pixel offset as a fraction of the monitor's width.
pub fn offset_frac(px: i32, mon_w: i32) -> f32 {
    if mon_w <= 0 {
        return 0.0;
    }
    (px as f32 / mon_w as f32).clamp(-0.5, 0.5)
}

/// A drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// Cursor x (screen px) when the button went down.
    pub start_x: i32,
    /// The pill's offset (px) when the button went down.
    pub start_off: i32,
    /// Past the threshold: drag mode is showing.
    pub active: bool,
}

impl Drag {
    pub fn new(start_x: i32, start_off: i32) -> Drag {
        Drag {
            start_x,
            start_off,
            active: false,
        }
    }

    /// Whether the cursor has moved far enough from the press to start dragging.
    pub fn past_threshold(&self, x: i32, threshold_px: i32) -> bool {
        (x - self.start_x).abs() > threshold_px
    }

    /// The offset (px) the pill takes with the cursor at `x`, kept on the monitor.
    pub fn offset_at(&self, x: i32, max: i32) -> i32 {
        (self.start_off + x - self.start_x).clamp(-max, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_stays_inside_the_monitor() {
        // 1920 wide, 380 pill, 8 margin: it can shift (1920-380)/2 - 8 = 762 either way.
        assert_eq!(max_shift(1920, 380, 8), 762);
        assert_eq!(offset_px(0.5, 1920, 380, 0, 8), 762);
        assert_eq!(offset_px(-0.5, 1920, 380, 0, 8), -762);
        // A wider pane near the edge slides in; the saved fraction is not touched.
        assert_eq!(offset_px(0.4, 1920, 1000, 0, 8), 452);
        assert_eq!(offset_px(0.2, 1920, 380, 0, 8), 384);
        // A bubble beside the pill costs room on the right only: the left edge is unchanged.
        assert_eq!(offset_px(0.5, 1920, 380, 80, 8), 682);
        assert_eq!(offset_px(-0.5, 1920, 380, 80, 8), -762);
        // Never an empty range, even when nothing fits.
        assert_eq!(offset_px(0.5, 1920, 1900, 300, 8), -2);
        // As wide as the monitor: no room to move.
        assert_eq!(max_shift(1920, 1920, 8), 0);
        assert_eq!(offset_px(0.3, 1920, 2400, 0, 8), 0);
    }

    #[test]
    fn fractions_survive_a_change_of_resolution() {
        let f = offset_frac(480, 1920);
        assert_eq!(f, 0.25);
        assert_eq!(offset_px(f, 2560, 380, 0, 8), 640);
        assert_eq!(offset_frac(5, 0), 0.0);
    }

    #[test]
    fn each_monitor_keeps_its_own_spot() {
        let mut p = Positions::default();
        p.set(r"\\.\DISPLAY1", 0.2);
        p.set(r"\\.\DISPLAY2", -0.3);
        assert_eq!(p.get(r"\\.\DISPLAY1"), 0.2);
        assert_eq!(p.get(r"\\.\DISPLAY2"), -0.3);
        assert_eq!(p.get(r"\\.\DISPLAY3"), 0.0);
        // Back to the middle forgets the entry.
        p.set(r"\\.\DISPLAY1", 0.0);
        assert_eq!(p.get(r"\\.\DISPLAY1"), 0.0);
        assert!(!p.is_empty());
        p.clear();
        assert!(p.is_empty());
    }

    #[test]
    fn a_damaged_file_cannot_move_the_pill_off_screen() {
        assert_eq!(Positions::parse("not json"), Positions::default());
        let p = Positions::parse(r#"{"a": 9.0, "b": -9.0, "c": 0.1}"#);
        assert_eq!(p.get("a"), 0.5);
        assert_eq!(p.get("b"), -0.5);
        assert_eq!(p.get("c"), 0.1);
    }

    #[test]
    fn a_press_becomes_a_drag_past_the_threshold() {
        let mut d = Drag::new(1000, 50);
        assert!(!d.past_threshold(1005, 6));
        assert!(d.past_threshold(1007, 6));
        assert!(d.past_threshold(990, 6));
        d.active = true;
        // Follows the cursor from where the pill was, and stops at the edges.
        assert_eq!(d.offset_at(1100, 762), 150);
        assert_eq!(d.offset_at(5000, 762), 762);
        assert_eq!(d.offset_at(-5000, 762), -762);
    }
}
