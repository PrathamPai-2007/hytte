//! The compact (closed) pill: idle, status icons, a running task, music.

use super::*;

impl Renderer {
    pub(super) fn idle(&self, fr: &Frame, w: f32, h: f32) {
        let m = fr.model;
        let chips = (!m.ports.is_empty()) as u8 + (!m.shelf.is_empty()) as u8;
        let lock = if m.mic_muted { LOCK_W } else { 0.0 };
        if chips == 0 {
            if m.timer.is_some() {
                return;
            }
            self.fill_rr(
                (w - lock) / 2.0 - 13.0,
                h / 2.0 - 1.5,
                26.0,
                3.0,
                1.5,
                self.cc(self.fg(), 0.16),
            );
            return;
        }
        let right_pad = lock + if m.privacy_active() { 34.0 } else { 0.0 };
        let mut x = (w - right_pad - 58.0 * chips as f32) / 2.0 + 4.0;
        let cy = h / 2.0;
        if let Some(p) = m.ports.first() {
            self.circle(x + 3.0, cy, 2.6, self.cc(GREEN, 1.0));
            let t = if m.ports.len() > 1 {
                format!(":{} +{}", p.port, m.ports.len() - 1)
            } else {
                format!(":{}", p.port)
            };
            self.text(
                &t,
                &self.f.small_l,
                x + 10.0,
                cy - 8.0,
                52.0,
                16.0,
                self.cc(self.fg(), 0.82),
            );
            x += 58.0;
        }
        if !m.shelf.is_empty() {
            self.fill_rr(x, cy - 3.5, 9.0, 8.0, 2.0, self.cc(PURPLE, 0.45));
            self.fill_rr(x + 2.0, cy - 5.5, 9.0, 8.0, 2.0, self.cc(PURPLE, 1.0));
            self.text(
                &m.shelf.len().to_string(),
                &self.f.small_l,
                x + 15.0,
                cy - 8.0,
                28.0,
                16.0,
                self.cc(self.fg(), 0.82),
            );
        }
    }

    /// Padlock: filled body plus a stroked shackle.
    pub(super) fn lock_icon(&self, cx: f32, cy: f32, col: [f32; 3], a: f32) {
        self.stroke_rr(
            cx - 3.2,
            cy - 6.2,
            6.4,
            9.0,
            3.2,
            self.cc(col, a),
            1.7,
            false,
        );
        self.fill_rr(cx - 5.5, cy - 1.5, 11.0, 8.5, 2.2, self.cc(col, a));
        self.circle(cx, cy + 2.2, 1.3, self.cc([0.05; 3], a));
    }

    /// Bold red lock shown on every scene while the microphone is muted; click unmutes.
    pub(super) fn mute_badge(&self, cx: f32, cy: f32) {
        let hov = self.hit(cx - 11.0, cy - 11.0, 22.0, 22.0, Action::ToggleMic);
        let p = (self.t.get() * 2.5).sin() * 0.5 + 0.5;
        self.circle(
            cx,
            cy,
            10.0,
            self.cc(RED, if hov { 0.34 } else { 0.20 + 0.04 * p }),
        );
        self.lock_icon(cx, cy, RED, 1.0);
    }

    pub(super) fn privacy_dots(&self, m: &Model, right: f32, cy: f32, pulse: bool) {
        let t = self.t.get();
        let mut x = right - 3.5;
        for (on, col) in [(m.cam, CAM), (m.mic, ORANGE)] {
            if !on {
                continue;
            }
            let p = if pulse {
                (t * 2.2).sin() * 0.5 + 0.5
            } else {
                0.0
            };
            self.circle(x, cy, 3.5 + 3.0 * p, self.cc(col, 0.22 * (1.0 - p)));
            self.circle(x, cy, 3.5, self.cc(col, 1.0));
            x -= 14.0;
        }
    }

    pub(super) fn status_icon(&self, t: &TaskView, cx: f32, cy: f32, now: Instant) {
        let col = task_color(t);
        let since = now.saturating_duration_since(t.changed).as_secs_f32();
        let draw_p = (since / 0.28).clamp(0.0, 1.0);
        let tm = self.t.get();
        match t.event {
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => {
                self.ring(cx, cy, 6.5, self.cc(col, 0.18), 1.8);
                let a0 = tm * 5.5;
                let n = 14;
                let mut prev = (cx + 6.5 * a0.cos(), cy + 6.5 * a0.sin());
                for i in 1..=n {
                    let a = a0 + (i as f32 / n as f32) * 1.9;
                    let p = (cx + 6.5 * a.cos(), cy + 6.5 * a.sin());
                    self.line(
                        prev,
                        p,
                        self.cc(col, 0.35 + 0.65 * i as f32 / n as f32),
                        1.8,
                    );
                    prev = p;
                }
            }
            TaskEvent::Done => {
                self.circle(cx, cy, 8.0, self.cc(col, 0.16));
                let pts = [
                    (cx - 3.6, cy + 0.2),
                    (cx - 1.0, cy + 3.0),
                    (cx + 3.8, cy - 2.8),
                ];
                let seg = |a: (f32, f32), b: (f32, f32), k: f32| {
                    (a.0 + (b.0 - a.0) * k, a.1 + (b.1 - a.1) * k)
                };
                let p1 = (draw_p * 2.0).min(1.0);
                self.line(pts[0], seg(pts[0], pts[1], p1), self.cc(col, 1.0), 2.0);
                if draw_p > 0.5 {
                    self.line(
                        pts[1],
                        seg(pts[1], pts[2], (draw_p - 0.5) * 2.0),
                        self.cc(col, 1.0),
                        2.0,
                    );
                }
            }
            TaskEvent::Failed => {
                let pulse = 0.5 + (tm * 3.2).sin() * 0.5 * t.fail_pulse(now);
                self.circle(cx, cy, 8.0 + pulse, self.cc(col, 0.12 + 0.12 * pulse));
                self.cross(cx, cy, 3.1, self.cc(col, 1.0), 2.0);
            }
            TaskEvent::NeedsInput => {
                // Pulsing halo + bell that rings in short bursts.
                let pulse = (tm * 4.0).sin() * 0.5 + 0.5;
                self.circle(cx, cy, 8.0 + 2.0 * pulse, self.cc(col, 0.14 + 0.16 * pulse));
                let burst = ((tm * 0.9) % 1.0 < 0.3) as u8 as f32;
                let sw = (tm * 22.0).sin() * 0.28 * burst;
                let (s, c) = (sw.sin(), sw.cos());
                let p = |x: f32, y: f32| (cx + x * c - y * s, cy - 1.5 + x * s + y * c);
                self.poly(
                    &[
                        p(-4.2, 3.0),
                        p(-2.6, -2.6),
                        p(0.0, -4.6),
                        p(2.6, -2.6),
                        p(4.2, 3.0),
                    ],
                    self.cc(col, 1.0),
                );
                self.line(p(-5.2, 3.2), p(5.2, 3.2), self.cc(col, 1.0), 1.8);
                self.circle(cx + 0.0, cy + 5.2, 1.5, self.cc(col, 1.0));
            }
            TaskEvent::Lost => {
                self.circle(cx, cy, 8.0, self.cc(col, 0.16));
                self.text(
                    "?",
                    &self.f.center,
                    cx - 8.0,
                    cy - 8.0,
                    16.0,
                    16.0,
                    self.cc(col, 1.0),
                );
            }
        }
    }

    pub(super) fn task_right_text(&self, t: &TaskView, now: Instant) -> String {
        match t.event {
            // After a minute, how long it has waited: the oldest wait is the one to answer.
            TaskEvent::NeedsInput => {
                match now.saturating_duration_since(t.changed).as_secs() / 60 {
                    0 => "waiting".into(),
                    m => format!("waiting · {m}m"),
                }
            }
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => match t.progress {
                Some(p) => format!("{p}%"),
                None => fmt_dur(now.saturating_duration_since(t.started).as_millis() as u64),
            },
            TaskEvent::Done => fmt_dur(t.duration_ms.unwrap_or(0)),
            TaskEvent::Failed => match (t.stderr.is_empty(), t.exit_code) {
                (true, Some(c)) => format!("exit {c} · {}", fmt_dur(t.duration_ms.unwrap_or(0))),
                _ => format!("failed · {}", fmt_dur(t.duration_ms.unwrap_or(0))),
            },
            TaskEvent::Lost => "lost".into(),
        }
    }

    /// Thin bar under a running task: determinate (eased) or shimmer.
    pub(super) fn filament(&self, t: &TaskView, x: f32, y: f32, w: f32, dt: f32, now: Instant) {
        let col = task_color(t);
        self.fill_rr(x, y, w, 2.0, 1.0, self.cc(self.fg(), 0.08));
        if !t.running() {
            if t.event == TaskEvent::Failed {
                self.fill_rr(
                    x,
                    y,
                    w,
                    2.0,
                    1.0,
                    self.cc(
                        RED,
                        0.55 + 0.25 * (self.t.get() * 3.2).sin() * t.fail_pulse(now),
                    ),
                );
            }
            return;
        }
        match t.progress {
            Some(p) => {
                let mut sh = self.shown.borrow_mut();
                let e = sh.entry(t.id.clone()).or_insert(0.0);
                *e += (p as f32 / 100.0 - *e) * (dt * 10.0).min(1.0);
                self.fill_rr(x, y, (w * *e).max(2.0), 2.0, 1.0, self.cc(col, 1.0));
            }
            None => unsafe {
                let ph = (self.t.get() * 0.85) % 1.0;
                let seg = w * 0.35;
                let sx = x - seg + (w + seg) * ph;
                if let Some(gb) = self.shimmer_brush(col) {
                    gb.SetStartPoint(v(sx, 0.0));
                    gb.SetEndPoint(v(sx + seg, 0.0));
                    gb.SetOpacity(self.ca.get().clamp(0.0, 1.0));
                    self.rt.PushAxisAlignedClip(
                        &rect(x, y, w, 2.0),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    self.rt.FillRectangle(&rect(sx, y, seg, 2.0), &gb);
                    self.rt.PopAxisAlignedClip();
                }
            },
        }
    }

    pub(super) fn compact_task(&self, fr: &Frame, w: f32, h: f32, pad_r: f32) {
        let m = fr.model;
        let Some(t) = m.primary() else { return };
        let now = fr.now;
        // Shake once when a task has just failed.
        let since = now.saturating_duration_since(t.changed).as_secs_f32();
        if t.failed() && since < 0.5 {
            let dx = (since * 55.0).sin() * 3.5 * (1.0 - since / 0.5);
            unsafe {
                self.rt
                    .SetTransform(&mat(self.scale, self.ox.get() + dx, 0.0))
            };
        }
        // The icon and label are shared with the first row of the expanded list.
        let (ic, saved) = self.hero(
            [20.0, 14.5, 0.0, 0.0],
            [24.0, 27.0, 0.0, 0.0],
            [20.0, h / 2.0 - 1.5, 0.0, 0.0],
        );
        self.status_icon(t, ic[0], ic[1], now);
        self.hero_end(saved);
        let mut right = w - 14.0 - pad_r;
        let n = m.visible_count();
        if n > 1 {
            self.fill_rr(
                right - 20.0,
                h / 2.0 - 9.0,
                20.0,
                16.0,
                8.0,
                self.cc(self.fg(), 0.14),
            );
            self.text(
                &n.to_string(),
                &self.f.btn,
                right - 20.0,
                h / 2.0 - 9.0,
                20.0,
                16.0,
                self.cc(self.fg(), 0.9),
            );
            right -= 28.0;
        }
        let rt = self.task_right_text(t, now);
        let rw = 62.0;
        self.text(
            &rt,
            &self.f.small_r,
            right - rw,
            h / 2.0 - 10.0,
            rw,
            17.0,
            self.cc(task_color(t), 0.95),
        );
        let (lc, le) = self.label_ends(w, t);
        let (lb, saved) = self.hero(lc, le, [38.0, h / 2.0 - 10.0, right - rw - 46.0, 17.0]);
        self.text(
            &crate::secrets::mask(&t.label),
            &self.f.title,
            lb[0],
            lb[1],
            lb[2],
            lb[3],
            self.cc(self.fg(), 0.94),
        );
        self.hero_end(saved);
        self.filament(t, 14.0, h - 5.0, w - 28.0, fr.dt, now);
    }

    pub(super) fn eq_bars(&self, x: f32, y: f32, h: f32, playing: bool, col: [f32; 3]) {
        let t = self.t.get();
        for i in 0..4 {
            let k = if playing {
                0.28 + 0.72 * ((t * (3.1 + i as f32 * 1.3) + i as f32 * 1.7).sin().abs())
            } else {
                0.25
            };
            let bh = (h * k).max(2.0);
            self.fill_rr(
                x + i as f32 * 4.5,
                y + h - bh,
                2.6,
                bh,
                1.3,
                self.cc(col, 0.95),
            );
        }
    }

    /// A 64 x 64 bitmap scaled into a rounded square of side `s`; false if it can't be drawn.
    pub(super) fn paint_bitmap(&self, bmp: &ID2D1Bitmap, x: f32, y: f32, s: f32, r: f32) -> bool {
        let props = D2D1_BRUSH_PROPERTIES {
            opacity: self.ca.get(),
            transform: Matrix3x2 {
                M11: s / 64.0,
                M12: 0.0,
                M21: 0.0,
                M22: s / 64.0,
                M31: x,
                M32: y,
            },
        };
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(x, y, s, s),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            let Ok(b) = self.rt.CreateBitmapBrush(bmp, None, Some(&props)) else {
                return false;
            };
            self.rt.FillRoundedRectangle(&rr, &b);
        }
        true
    }

    pub(super) fn art_tile(&self, x: f32, y: f32, s: f32, r: f32) {
        if let Some(bmp) = self.art.borrow().as_ref() {
            if self.paint_bitmap(bmp, x, y, s, r) {
                return;
            }
        }
        self.fill_rr(x, y, s, s, r, self.cc(PURPLE, 0.20));
        self.stroke_rr(
            x + 0.5,
            y + 0.5,
            s - 1.0,
            s - 1.0,
            r,
            self.cc(PURPLE, 0.30),
            1.0,
            false,
        );
        if s >= 40.0 {
            self.glyph_l('\u{EC4F}', x + s / 2.0, y + s / 2.0, self.cc(PURPLE, 0.95));
        } else {
            self.glyph(
                '\u{EC4F}',
                x + s / 2.0,
                y + s / 2.0,
                s,
                self.cc(PURPLE, 0.95),
            );
        }
    }

    pub(super) fn compact_media(&self, fr: &Frame, w: f32, h: f32, pad_r: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        let (a, saved) = self.hero(
            [8.0, 6.0, 20.0, 5.0],
            [16.0, 16.0, 72.0, 12.0],
            [8.0, h / 2.0 - 10.0, 20.0, 5.0],
        );
        self.art_tile(a[0], a[1], a[2], a[3]);
        self.hero_end(saved);
        let right = w - 14.0 - pad_r;
        self.eq_bars(right - 16.0, h / 2.0 - 7.0, 14.0, md.playing, GREEN);
        let label = if md.artist.is_empty() {
            md.title.clone()
        } else {
            format!("{} — {}", md.title, md.artist)
        };
        self.text(
            &label,
            &self.f.body,
            38.0,
            h / 2.0 - 9.0,
            right - 16.0 - 46.0,
            17.0,
            self.cc(self.fg(), 0.92),
        );
    }
}
