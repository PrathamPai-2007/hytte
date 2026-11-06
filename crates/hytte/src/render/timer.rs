//! The Timer pane and the timer-done card.

use super::*;

impl Renderer {
    /// Hairline fuse along the bottom edge: burns down right to left, ember at the head.
    pub(super) fn timer_filament(
        &self,
        t: &crate::timer::Timer,
        w: f32,
        h: f32,
        centre: bool,
        lock: f32,
    ) {
        let wall = crate::timer::now_ms();
        let frac = t.frac(wall);
        let col = if t.remaining_ms(wall) < 60_000 {
            RED
        } else if frac <= 0.2 {
            AMBER
        } else {
            BLUE
        };
        // A Focus session is quiet: a thin pale line that only turns amber in the last minute.
        if t.kind == crate::timer::TimerKind::Focus {
            let (x0, y, side) = if centre {
                (20.0, h / 2.0, 20.0 + lock)
            } else {
                (14.0, h - 2.0, 14.0)
            };
            let len = (w - x0 - side).max(1.0);
            let col = if t.remaining_ms(wall) < 60_000 {
                AMBER
            } else {
                self.fg()
            };
            unsafe { self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0)) };
            self.line((x0, y), (x0 + len, y), color(self.fg(), 0.07), 1.0);
            if frac > 0.0 {
                self.line((x0, y), (x0 + len * frac, y), color(col, 0.55), 1.3);
            }
            return;
        }
        // Collapsed and empty: the fuse replaces the idle grey dash, in the middle of the pill.
        let (x0, y, side) = if centre {
            (20.0, h / 2.0, 20.0 + lock)
        } else {
            (14.0, h - 2.0, 14.0)
        };
        let len = (w - x0 - side).max(1.0);
        let head = x0 + len * frac;
        unsafe { self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0)) };
        self.line((x0, y), (x0 + len, y), color(self.fg(), 0.07), 1.5);
        if frac > 0.0 {
            self.line((x0, y), (head, y), color(col, 0.25), 4.0);
            self.line((x0, y), (head, y), color(col, 0.95), 1.5);
            self.circle(head, y, 4.0, color(col, 0.22));
            self.circle(head, y, 1.8, color([1.0, 0.97, 0.9], 1.0));
        }
    }

    pub(super) fn exp_timer(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::timer::TimerKind;
        let m = fr.model;
        let wall = crate::timer::now_ms();
        if let Some(t) = &m.timer {
            let left = t.remaining_ms(wall);
            let paused = t.paused_ms.is_some();
            let frac = t.frac(wall);
            let col = if paused {
                AMBER
            } else if left < 60_000 {
                RED
            } else if frac <= 0.2 {
                AMBER
            } else {
                BLUE
            };
            // The ring is the countdown and the pause / resume button.
            let (cx, cy, r) = (46.0, 46.0, 27.0);
            let d = 2.0 * r + 8.0;
            let hov = self.hit(cx - r - 4.0, cy - r - 4.0, d, d, Action::TimerPause);
            self.circle(cx, cy, r - 4.0, self.cc(col, if hov { 0.16 } else { 0.07 }));
            self.ring(cx, cy, r, self.cc(self.fg(), 0.09), 5.0);
            self.arc(cx, cy, r, frac, self.cc(col, 0.95), 5.0);
            let ic = self.cc(self.fg(), if hov { 1.0 } else { 0.85 });
            if paused {
                self.play_icon(cx, cy, 15.0, ic);
            } else {
                self.pause_icon(cx, cy, 14.0, ic);
            }
            self.text(
                &crate::timer::fmt(left),
                &self.f.hero,
                88.0,
                14.0,
                150.0,
                34.0,
                self.cc(self.fg(), 0.97),
            );
            let sub = if paused {
                format!("{} · paused", t.kind.name())
            } else {
                format!(
                    "{} · ends {}",
                    t.kind.name(),
                    crate::timer::clock(t.ends_ms)
                )
            };
            let sc = if paused { AMBER } else { GRAY };
            self.text(
                &sub,
                &self.f.small,
                89.0,
                47.0,
                170.0,
                16.0,
                self.cc(sc, 1.0),
            );
            // A focus cycle is a row of dots: filled for finished rounds, lit for the one running.
            if t.kind != TimerKind::Plain {
                let focus = t.kind == TimerKind::Focus;
                let (filled, total) = crate::timer::round_dots(m.focus_done, m.rounds, focus);
                for i in 0..total {
                    let on = i < filled;
                    let dx = 93.0 + i as f32 * 12.0;
                    let live = on && i + 1 == filled && focus;
                    if on {
                        self.circle(dx, 73.0, 3.4, self.cc(if live { BLUE } else { GREEN }, 1.0));
                    } else {
                        self.ring(dx, 73.0, 2.8, self.cc(GRAY, 0.6), 1.2);
                    }
                }
            }
            let fg = self.fg();
            let bx = w - 16.0 - 19.0;
            self.round_btn(bx, cy, 19.0, RED, Action::TimerStop, |x, y, c| {
                self.fill_rr(x - 5.5, y - 5.5, 11.0, 11.0, 2.5, c)
            });
            self.round_btn(bx - 46.0, cy, 19.0, fg, Action::TimerAdd(5), |x, y, c| {
                self.text("+5", &self.f.center, x - 19.0, y - 19.0, 38.0, 38.0, c)
            });
            return;
        }
        // Setup: length, presets, start.
        self.text(
            &format!("{} min", m.timer_min),
            &self.f.hero,
            20.0,
            10.0,
            200.0,
            36.0,
            self.cc(self.fg(), 0.97),
        );
        self.text(
            "Scroll or type the minutes",
            &self.f.small,
            21.0,
            44.0,
            w - 21.0 - 104.0,
            14.0,
            self.cc(GRAY, 0.9),
        );
        let fg = self.fg();
        // Close, small and quiet in the corner; − / + to its left.
        self.round_btn(
            w - 16.0 - 11.0,
            22.0,
            11.0,
            fg,
            Action::TimerClose,
            |x, y, c| self.cross(x, y, 3.5, c, 1.5),
        );
        let bx = w - 16.0 - 22.0 - 10.0 - 18.0;
        self.round_btn(bx, 30.0, 18.0, fg, Action::TimerNudge(1), |x, y, c| {
            self.line((x - 6.0, y), (x + 6.0, y), c, 2.0);
            self.line((x, y - 6.0), (x, y + 6.0), c, 2.0);
        });
        self.round_btn(
            bx - 44.0,
            30.0,
            18.0,
            fg,
            Action::TimerNudge(-1),
            |x, y, c| {
                self.line((x - 6.0, y), (x + 6.0, y), c, 2.0);
            },
        );
        let presets = [5u32, 10, 25, 45, 60];
        let (gap, py, ph) = (6.0, 68.0, 28.0);
        let pw = (w - 32.0 - gap * (presets.len() - 1) as f32) / presets.len() as f32;
        for (i, p) in presets.into_iter().enumerate() {
            let x = 16.0 + i as f32 * (pw + gap);
            let on = p == m.timer_min;
            let hov = self.hit(x, py, pw, ph, Action::TimerMinutes(p));
            let a = if on {
                0.28
            } else if hov {
                0.12
            } else {
                0.06
            };
            self.fill_rr(
                x,
                py,
                pw,
                ph,
                ph / 2.0,
                self.cc(if on { self.accent.get() } else { fg }, a),
            );
            let tc = self.cc(fg, if on { 1.0 } else { 0.75 });
            self.text(&format!("{p} min"), &self.f.btn, x, py, pw, ph, tc);
        }
        let bw = (w - 32.0 - 8.0) / 2.0;
        let start = |k| Action::TimerStart(k);
        self.primary(
            "Start timer",
            16.0,
            106.0,
            bw,
            32.0,
            self.accent.get(),
            start(TimerKind::Plain),
        );
        self.primary(
            "Start focus",
            24.0 + bw,
            106.0,
            bw,
            32.0,
            GREEN,
            start(TimerKind::Focus),
        );
    }

    pub(super) fn exp_timer_done(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::timer::TimerKind::*;
        let m = fr.model;
        let Some((kind, _)) = m.timer_done else {
            return;
        };
        // A bell in a halo; while the alarm rings it swings and sends out ripples.
        let (cx, cy) = (40.0, 38.0);
        let ringing = fr.ringing;
        let t = self.t.get();
        if ringing {
            for k in 0..2 {
                let ph = (t * 0.8 + k as f32 * 0.5).fract();
                self.ring(
                    cx,
                    cy,
                    21.0 + ph * 12.0,
                    self.cc(GREEN, (1.0 - ph) * 0.35),
                    1.5,
                );
            }
        }
        self.circle(cx, cy, 21.0, self.cc(GREEN, 0.18));
        // The alarm phrase is ~2.6 s: swing during the notes, rest in the pause.
        let phrase = (t % 2.64) / 1.5;
        let swing = if ringing && phrase < 1.0 {
            (t * 22.0).sin() * 14.0 * (1.0 - phrase)
        } else {
            0.0
        };
        let saved = self.rotate(swing, cx, cy - 9.0);
        self.glyph_l('\u{EA8F}', cx, cy, self.cc(GREEN, 1.0));
        unsafe { self.rt.SetTransform(&saved) };
        let title = match kind {
            Plain => "Time's up",
            Focus => "Focus session done",
            Break => "Break is over",
        };
        self.text(
            title,
            &self.f.big,
            74.0,
            16.0,
            w - 90.0,
            22.0,
            self.cc(self.fg(), 0.97),
        );
        if let Some(last) = &m.timer_last {
            let mins = last.total_ms / 60_000;
            let what = kind.name().to_lowercase();
            let ended = crate::timer::clock(last.ends_ms);
            let sub = format!("{mins} min {what}  ·  ended {ended}");
            self.text(
                &sub,
                &self.f.small,
                75.0,
                39.0,
                w - 90.0,
                16.0,
                self.cc(GRAY, 1.0),
            );
        }
        // What was held back during a focus session, said once.
        let note_y = if let Some(s) = &m.focus_summary {
            self.text(
                s,
                &self.f.small,
                75.0,
                56.0,
                w - 90.0,
                16.0,
                self.cc(AMBER, 0.95),
            );
            86.0
        } else {
            70.0
        };
        let (y, h) = (note_y, 30.0);
        let mut x = 16.0;
        match kind {
            Focus => {
                let label = format!("Start {} min break", m.timer_break);
                let bw = 30.0 + 6.2 * label.len() as f32;
                self.primary(
                    &label,
                    x,
                    y,
                    bw,
                    h,
                    GREEN,
                    Action::TimerBreak(m.timer_break),
                );
                x += bw + 8.0;
            }
            Break => {
                self.primary(
                    "Start focus",
                    x,
                    y,
                    104.0,
                    h,
                    GREEN,
                    Action::TimerStart(Focus),
                );
                x += 112.0;
            }
            Plain => {}
        }
        self.button("+5 min", x, y, 70.0, h, BLUE, Action::TimerAdd(5));
        let dismiss = if ringing { "Stop alarm" } else { "Dismiss" };
        self.button(
            dismiss,
            w - 16.0 - 88.0,
            y,
            88.0,
            h,
            self.fg(),
            Action::TimerDismiss,
        );
    }
}
