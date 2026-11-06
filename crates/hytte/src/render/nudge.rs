//! The break reminder card: an icon (the eye break's is a 20 second countdown ring), one line,
//! and Snooze and Skip.

use super::*;
use crate::breaks::Kind;

impl Renderer {
    pub(super) fn exp_nudge(&self, fr: &Frame, w: f32, h: f32) {
        let Some(n) = fr.model.nudge else { return };
        let fg = self.fg();
        let cy = h / 2.0;
        let cx = 30.0;
        match n.kind {
            Kind::Eyes => {
                // The ring drains over the 20 seconds; the number counts down inside it.
                let left = n.kind.hold().as_secs_f32()
                    - fr.now.saturating_duration_since(n.since).as_secs_f32();
                let frac = (left / n.kind.hold().as_secs_f32()).clamp(0.0, 1.0);
                self.ring(cx, cy, 14.0, self.cc(fg, 0.10), 3.0);
                self.arc(cx, cy, 14.0, frac, self.cc(BLUE, 0.95), 3.0);
                self.text(
                    &format!("{}", left.ceil().max(0.0) as u32),
                    &self.f.btn,
                    cx - 14.0,
                    cy - 8.0,
                    28.0,
                    16.0,
                    self.cc(fg, 0.95),
                );
            }
            Kind::Water => {
                self.circle(cx, cy, 16.0, self.cc([0.45, 0.85, 1.0], 0.16));
                // A drop: a point above a round belly.
                let c = self.cc([0.45, 0.85, 1.0], 1.0);
                self.circle(cx, cy + 2.5, 6.0, c);
                self.poly(&[(cx, cy - 9.0), (cx - 5.0, cy), (cx + 5.0, cy)], c);
            }
            Kind::Stretch => {
                self.circle(cx, cy, 16.0, self.cc(GREEN, 0.16));
                self.glyph_l('\u{E77B}', cx, cy, self.cc(GREEN, 1.0));
            }
        }
        let tx = 56.0;
        let bw = 132.0;
        self.text(
            n.kind.title(),
            &self.f.title,
            tx,
            cy - 17.0,
            w - tx - bw - 16.0,
            18.0,
            self.cc(fg, 0.97),
        );
        self.text(
            n.kind.detail(),
            &self.f.small,
            tx,
            cy + 1.0,
            w - tx - bw - 16.0,
            16.0,
            self.cc(GRAY, 1.0),
        );
        let by = cy - 12.0;
        self.button(
            "Snooze",
            w - 16.0 - 58.0 - 6.0 - 48.0,
            by,
            58.0,
            24.0,
            fg,
            Action::NudgeSnooze,
        );
        self.button(
            "Skip",
            w - 16.0 - 48.0,
            by,
            48.0,
            24.0,
            fg,
            Action::NudgeSkip,
        );
    }
}
