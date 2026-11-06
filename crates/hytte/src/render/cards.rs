//! The Drop Vault result, chip and charging cards.

use super::*;

impl Renderer {
    pub(super) fn exp_drop(&self, w: f32, h: f32) {
        let t = self.t.get();
        let pulse = (t * 3.0).sin() * 0.5 + 0.5;
        self.fill_rr(
            12.0,
            12.0,
            w - 24.0,
            h - 24.0,
            14.0,
            self.cc(PURPLE, 0.06 + 0.05 * pulse),
        );
        self.stroke_rr(
            12.0,
            12.0,
            w - 24.0,
            h - 24.0,
            14.0,
            self.cc(PURPLE, 0.55 + 0.35 * pulse),
            1.6,
            true,
        );
        let bob = (t * 4.0).sin() * 2.5;
        let (cx, cy) = (w / 2.0, 34.0 + bob);
        self.line((cx, cy - 8.0), (cx, cy + 6.0), self.cc(PURPLE, 1.0), 2.2);
        self.line((cx - 6.0, cy), (cx, cy + 7.0), self.cc(PURPLE, 1.0), 2.2);
        self.line((cx + 6.0, cy), (cx, cy + 7.0), self.cc(PURPLE, 1.0), 2.2);
        self.text(
            "Drop to add to your shelf",
            &self.f.big_c,
            0.0,
            50.0,
            w,
            22.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            "Drag it back out anywhere later",
            &self.f.small_c,
            0.0,
            74.0,
            w,
            16.0,
            self.cc(GRAY, 1.0),
        );
    }

    pub(super) fn exp_chip(&self, fr: &Frame, w: f32, _h: f32) {
        let Some(c) = fr.model.chip.as_ref() else {
            return;
        };
        let accent = match c.tone {
            Tone::Done => PURPLE,
            Tone::Warn => AMBER,
            Tone::Info => BLUE,
        };
        self.circle(26.0, 24.0, 9.0, self.cc(accent, 0.2));
        match c.tone {
            Tone::Done => {
                self.line((22.2, 24.4), (25.0, 27.4), self.cc(accent, 1.0), 2.0);
                self.line((25.0, 27.4), (30.2, 21.2), self.cc(accent, 1.0), 2.0);
            }
            Tone::Warn => {
                self.line((26.0, 19.0), (26.0, 25.0), self.cc(accent, 1.0), 2.0);
                self.circle(26.0, 28.5, 1.2, self.cc(accent, 1.0));
            }
            Tone::Info => {
                self.circle(26.0, 19.8, 1.2, self.cc(accent, 1.0));
                self.line((26.0, 23.0), (26.0, 29.0), self.cc(accent, 1.0), 2.0);
            }
        }
        self.text(
            &crate::secrets::mask(&c.summary),
            &self.f.wrap,
            46.0,
            10.0,
            w - 46.0 - 20.0,
            40.0,
            self.cc(self.fg(), 0.95),
        );
        let by = if c.one_line() { 44.0 } else { 60.0 };
        let mut x = 16.0;
        if let Some(p) = &c.open {
            self.button(
                "Open",
                x,
                by,
                62.0,
                24.0,
                PURPLE,
                Action::OpenPath(p.clone()),
            );
            x += 70.0;
        }
        if let Some(t) = &c.copy {
            self.button(
                "Copy",
                x,
                by,
                62.0,
                24.0,
                PURPLE,
                Action::CopyText(t.clone()),
            );
            x += 70.0;
        }
        for (label, act) in &c.extra {
            let armed = matches!(act, ChipAction::KillPid(..)) && fr.model.chip_armed.is_some();
            let label = if armed { "Kill?" } else { label.as_str() };
            let bw = (label.chars().count() as f32 * 6.6 + 26.0).max(62.0);
            let col = match act {
                ChipAction::KillPid(..) | ChipAction::Answer(_, false) => RED,
                ChipAction::Answer(_, true) => GREEN,
                _ => accent,
            };
            self.button(label, x, by, bw, 24.0, col, Action::Chip(act.clone()));
            x += bw + 8.0;
        }
        self.button("Dismiss", x, by, 70.0, 24.0, self.fg(), Action::DismissChip);
    }

    /// The card shown for a moment after the charger is plugged in (a battery that fills up)
    /// or unplugged (one that drains to its level, red when low).
    pub(super) fn exp_charge(&self, fr: &Frame, w: f32, h: f32) {
        let m = fr.model;
        let Some(b) = m.power else { return };
        let since = m
            .charge_since
            .map_or(1.0, |s| fr.now.saturating_duration_since(s).as_secs_f32());
        let k = (since / 0.7).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - k).powi(3);
        let plugged = m.charge_in;
        let tone = match (plugged, b.pct) {
            (true, _) => GREEN,
            (false, 0..=20) => RED,
            (false, _) => AMBER,
        };
        let cy = h / 2.0;
        let (bx, bw, bh) = (20.0, 30.0, 15.0);
        self.stroke_rr(
            bx,
            cy - bh / 2.0,
            bw,
            bh,
            4.0,
            self.cc(tone, 0.9),
            1.4,
            false,
        );
        self.fill_rr(bx + bw + 1.5, cy - 3.0, 2.5, 6.0, 1.2, self.cc(tone, 0.9));
        let level = b.pct as f32 / 100.0;
        // Plugging in fills up from empty; unplugging drains down from full.
        let shown = if plugged {
            level * ease
        } else {
            1.0 + (level - 1.0) * ease
        };
        let fill = (bw - 5.0) * shown;
        if fill > 0.5 {
            self.fill_rr(
                bx + 2.5,
                cy - bh / 2.0 + 2.5,
                fill,
                bh - 5.0,
                2.0,
                self.cc(tone, 0.9),
            );
        }
        if plugged {
            let a = 0.55 + 0.45 * ease;
            self.bolt(bx + bw / 2.0, cy, 0.7, self.cc([1.0; 3], a));
        }
        self.text(
            if plugged { "Plugged in" } else { "Unplugged" },
            &self.f.title,
            66.0,
            cy - 17.0,
            w - 66.0 - 16.0,
            20.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            &b.status(),
            &self.f.small,
            66.0,
            cy + 1.0,
            w - 66.0 - 16.0,
            16.0,
            self.cc(tone, 1.0),
        );
    }
}
