//! The settings pane and its rows.

use super::*;

impl Renderer {
    /// Settings: a header (back, the group switch, open the config file) over up to five
    /// rows. Each row is a switch or a short choice that writes `config.toml` on click.
    pub(super) fn exp_settings(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::ui_state::SETTINGS_GROUPS;
        let c = fr.cfg;
        let group = fr.model.settings.unwrap_or(0);
        let (fg, acc) = (self.fg(), self.accent.get());
        let back = Action::Settings(None);
        self.round_btn(28.0, 23.0, 13.0, fg, back, |cx, cy, col| {
            self.glyph_s('\u{E72B}', cx, cy, col)
        });
        self.round_btn(
            w - 28.0,
            23.0,
            13.0,
            fg,
            Action::OpenConfig,
            |cx, cy, col| self.glyph_s('\u{E8A7}', cx, cy, col),
        );
        let (sx, sw) = (50.0, w - 100.0);
        self.fill_rr(sx, 10.0, sw, 26.0, 13.0, self.cc(fg, 0.06));
        let seg = sw / SETTINGS_GROUPS.len() as f32;
        for (i, name) in SETTINGS_GROUPS.iter().enumerate() {
            let x = sx + i as f32 * seg;
            let on = i as u8 == group;
            let hov = self.hit(x, 10.0, seg, 26.0, Action::Settings(Some(i as u8)));
            if on {
                self.fill_rr(x + 2.0, 12.0, seg - 4.0, 22.0, 11.0, self.cc(acc, 0.26));
            } else if hov {
                self.fill_rr(x + 2.0, 12.0, seg - 4.0, 22.0, 11.0, self.cc(fg, 0.07));
            }
            let a = if on || hov { 1.0 } else { 0.66 };
            self.text(name, &self.f.btn, x, 10.0, seg, 26.0, self.cc(fg, a));
        }

        let rows: Vec<Row> = settings_table(c)
            .into_iter()
            .flat_map(|s| s.rows)
            .filter(|(_, q)| *q == group as i8)
            .map(|(r, _)| match r {
                // The pane has no room for a slider: steps, the last being the slider's end.
                Row::Glow(v) => pick_f(
                    "Glow",
                    "general.glow_strength",
                    &[
                        ("Off", 0.0),
                        ("Soft", 0.6),
                        ("Normal", 1.2),
                        ("Max", GLOW_MAX as f64),
                    ],
                    v as f64,
                ),
                r => r,
            })
            .map(|r| match pane_label(r.key()) {
                Some(l) => r.relabel(l),
                None => r,
            })
            .collect();
        let n = rows.len();
        for (i, row) in rows.into_iter().enumerate() {
            self.setting_row(row, None, 46.0 + i as f32 * 32.0, w, 0.0);
        }
        // Spare room at the bottom: where the rest of the settings live.
        if n < 5 {
            let note = concat!(
                "Hytte ",
                env!("CARGO_PKG_VERSION"),
                "  ·  more in config.toml"
            );
            let y = 46.0 + 4.0 * 32.0;
            self.text(note, &self.f.small_c, 0.0, y, w, 30.0, self.cc(GRAY, 0.75));
        }
    }

    /// `key`'s value eased towards `target` (settings window only: the pill's pane snaps).
    /// The first sight of a key starts at the target, so nothing animates on open.
    pub(super) fn ease(&self, key: &str, target: f32) -> f32 {
        if !self.fx.get() {
            return target;
        }
        let mut m = self.fx_vals.borrow_mut();
        let v = m.entry(key.to_string()).or_insert(target);
        let k = 1.0 - (-self.fx_dt.get() * 16.0).exp();
        *v += (target - *v) * k;
        if (target - *v).abs() < 0.003 {
            *v = target;
        } else {
            self.fx_busy.set(true);
        }
        *v
    }

    /// One settings row at `y`, `w` wide: a switch, a few choices or the opacity slider. A
    /// `gut` (the big window) keeps a column at the right for the reset, and turns on the
    /// hover note and the "Custom" step of numbers.
    pub(super) fn setting_row(&self, row: Row, reset: Option<Action>, y: f32, w: f32, gut: f32) {
        let (fg, acc) = (self.fg(), self.accent.get());
        let key = row.key();
        // How wide the control is, to know where the label's room ends.
        let wide = gut > 0.0;
        let ctrl = match &row {
            Row::Switch(..) => 34.0,
            Row::Opacity(_) | Row::Glow(_) => 206.0,
            Row::Pick(_, _, o, ..) if wide && is_numeric(o) => 246.0,
            Row::Pick(..) => 206.0,
            Row::PickCustom(..) => 246.0,
        };
        let label_w = w - gut - 22.0 - ctrl - 30.0;
        if wide && !help(key).is_empty() && self.over(0.0, y, label_w.max(60.0), 30.0) {
            self.tip_now.set(Some((key, y)));
        }
        self.setting_row_body(row, y, w, gut);
        if let Some(a) = reset {
            // Always the same spot: the right edge of the row.
            let rx = w - 22.0 - 24.0;
            // A true circle, centred on the row, with a font icon centred in it.
            let hov = self.hit(rx, y + 3.0, 24.0, 24.0, a);
            let h = self.ease(&format!("r:{key}"), hov as u8 as f32);
            self.fill_rr(rx, y + 3.0, 24.0, 24.0, 12.0, self.cc(fg, 0.1 * h));
            self.glyph_s('\u{E7A7}', rx + 12.0, y + 15.0, self.cc(acc, 0.6 + 0.4 * h));
            if hov {
                self.tip_now.set(Some(("reset", y)));
            }
        }
    }

    /// The search field at the top of the settings window.
    pub(super) fn search_field(&self, query: &str, x: f32, y: f32, w: f32) {
        let (fg, acc) = (self.fg(), self.accent.get());
        self.fill_rr(x, y, w, 34.0, 17.0, self.cc(fg, 0.06));
        if !query.is_empty() {
            self.stroke_rr(
                x + 0.5,
                y + 0.5,
                w - 1.0,
                33.0,
                17.0,
                self.cc(acc, 0.7),
                1.0,
                false,
            );
        }
        // A magnifier.
        self.ring(x + 24.0, y + 16.0, 5.0, self.cc(fg, 0.55), 1.6);
        self.line(
            (x + 28.0, y + 20.0),
            (x + 32.0, y + 24.0),
            self.cc(fg, 0.55),
            1.6,
        );
        if query.is_empty() {
            self.text(
                "Search settings",
                &self.f.body,
                x + 42.0,
                y,
                w - 90.0,
                34.0,
                self.cc(GRAY, 0.9),
            );
        } else {
            self.text(
                query,
                &self.f.body,
                x + 42.0,
                y,
                w - 90.0,
                34.0,
                self.cc(fg, 0.95),
            );
            let hov = self.hit(x + w - 34.0, y + 5.0, 24.0, 24.0, Action::ClearSearch);
            let h = self.ease("h:clear", hov as u8 as f32);
            self.fill_rr(
                x + w - 34.0,
                y + 5.0,
                24.0,
                24.0,
                12.0,
                self.cc(fg, 0.1 * h),
            );
            let (cx, cy, r) = (x + w - 22.0, y + 17.0, 4.0);
            let c = self.cc(fg, 0.6 + 0.3 * h);
            self.line((cx - r, cy - r), (cx + r, cy + r), c, 1.5);
            self.line((cx - r, cy + r), (cx + r, cy - r), c, 1.5);
        }
    }

    pub(super) fn setting_row_body(&self, row: Row, y: f32, w: f32, gut: f32) {
        let (fg, acc) = (self.fg(), self.accent.get());
        let rw = w - gut;
        match row {
            Row::Switch(label, key, on) => {
                let hov = self.hit(10.0, y, w - 20.0, 30.0, Action::Set(key, Val::B(!on)));
                let h = self.ease(&format!("h:{key}"), hov as u8 as f32);
                if h > 0.01 {
                    self.fill_rr(10.0, y, w - 20.0, 30.0, 12.0, self.cc(fg, 0.06 * h));
                }
                self.text(
                    label,
                    &self.f.body,
                    22.0,
                    y,
                    rw - 90.0,
                    30.0,
                    self.cc(fg, 0.9),
                );
                let t = self.ease(&format!("s:{key}"), on as u8 as f32);
                let (kx, ky) = (rw - 22.0 - 34.0, y + 6.0);
                if t < 0.99 {
                    self.stroke_rr(
                        kx,
                        ky,
                        34.0,
                        18.0,
                        9.0,
                        self.cc(fg, 0.4 * (1.0 - t)),
                        1.2,
                        false,
                    );
                }
                if t > 0.01 {
                    self.fill_rr(kx, ky, 34.0, 18.0, 9.0, self.cc(acc, 0.95 * t));
                }
                let knob = mix3(fg, self.ink(), t);
                self.circle(
                    kx + 9.0 + 16.0 * t,
                    ky + 9.0,
                    5.0 + t,
                    self.cc(knob, 0.7 + 0.25 * t),
                );
            }
            Row::Opacity(v) => {
                let name = if gut > 0.0 {
                    row_label_opacity()
                } else {
                    "Opacity"
                };
                let f = ((v - 0.4) / 0.6).clamp(0.0, 1.0);
                self.slider(
                    name,
                    "opacity",
                    f,
                    (v * 100.0).round(),
                    Action::Opacity,
                    y,
                    rw,
                );
            }
            Row::Glow(v) => {
                let f = (v / GLOW_MAX).clamp(0.0, 1.0);
                self.slider(
                    "Glow strength",
                    "glow",
                    f,
                    (f * 100.0).round(),
                    Action::Glow,
                    y,
                    rw,
                );
            }
            // Numbers also get "Custom": lit, with the file's value beside it, when the value
            // is none of the steps.
            Row::Pick(label, key, opts, cur, off) if gut > 0.0 && is_numeric(&opts) => {
                let (cur, shown) = match off {
                    None => (cur, String::new()),
                    Some((text, _)) => (opts.len(), text),
                };
                self.pick_row(label, key, opts, cur, Some((key, shown)), y, w, gut)
            }
            Row::Pick(label, key, opts, cur, _) => {
                self.pick_row(label, key, opts, cur, None, y, w, gut)
            }
            Row::PickCustom(label, key, opts, cur, hint, shown) => {
                self.pick_row(label, key, opts, cur, Some((hint, shown)), y, w, gut)
            }
        }
    }

    /// A labelled slider, `f` (0..1) of the way along, with `pct` written beside it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn slider(
        &self,
        name: &str,
        id: &str,
        f: f32,
        pct: f32,
        action: Action,
        y: f32,
        rw: f32,
    ) {
        let (fg, acc) = (self.fg(), self.accent.get());
        let (bx, bw, bh) = (rw - 22.0 - 206.0 + 6.0, 194.0, 6.0);
        self.text(
            name,
            &self.f.body,
            22.0,
            y,
            (bx - 100.0).max(60.0),
            30.0,
            self.cc(fg, 0.9),
        );
        let pct = format!("{pct}%");
        self.text(
            &pct,
            &self.f.small,
            bx - 46.0,
            y,
            40.0,
            30.0,
            self.cc(GRAY, 0.9),
        );
        let by = y + 12.0;
        let hov = self.hit(bx - 6.0, y, bw + 12.0, 30.0, action);
        let h = self.ease(
            &format!("h:{id}"),
            (hov || self.pressed.get() && hov) as u8 as f32,
        );
        self.fill_rr(bx, by, bw, bh, bh / 2.0, self.cc(fg, 0.08 + 0.04 * h));
        self.fill_rr(bx, by, (bw * f).max(bh), bh, bh / 2.0, self.cc(acc, 0.9));
        self.circle(
            bx + 3.0 + (bw - 6.0) * f,
            by + bh / 2.0,
            6.5 + 1.5 * h,
            self.cc(fg, 0.95),
        );
    }

    /// Steps in a pill, the lit one sliding to the chosen step. With `custom`, one more
    /// segment ("Custom") opens `config.toml` at the setting; it is lit, and the file's value
    /// shown beside the label, when the value is none of the steps.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn pick_row(
        &self,
        label: &str,
        key: &'static str,
        opts: Vec<Choice>,
        cur: usize,
        custom: Option<(&'static str, String)>,
        y: f32,
        w: f32,
        gut: f32,
    ) {
        let (fg, acc) = (self.fg(), self.accent.get());
        let extra = custom.is_some() as usize;
        let pw = 206.0 + 40.0 * extra as f32;
        let px = w - gut - 22.0 - pw;
        // The value in the file, when it is none of the steps, sits just left of the control.
        let shown_w =
            custom.as_ref().map_or(
                0.0,
                |(_, s)| if s.is_empty() { 0.0 } else { text_w(s) + 14.0 },
            );
        self.text(
            label,
            &self.f.body,
            22.0,
            y,
            (px - 30.0 - shown_w).max(60.0),
            30.0,
            self.cc(fg, 0.9),
        );
        if let Some((_, shown)) = &custom {
            if shown_w > 0.0 {
                self.text(
                    shown,
                    &self.f.small,
                    px - shown_w,
                    y,
                    shown_w,
                    30.0,
                    self.cc(acc, 0.95),
                );
            }
        }
        self.fill_rr(px, y + 3.0, pw, 24.0, 12.0, self.cc(fg, 0.06));
        let sw = pw / (opts.len() + extra) as f32;
        // The lit segment slides to the chosen one.
        let at = self.ease(&format!("p:{key}"), cur as f32);
        self.fill_rr(
            px + at * sw + 2.0,
            y + 5.0,
            sw - 4.0,
            20.0,
            10.0,
            self.cc(acc, 0.26),
        );
        let segments = opts
            .into_iter()
            .map(|(name, v)| (name, Action::Set(key, v)))
            .chain(custom.map(|(hint, _)| ("Custom", Action::EditConfig(hint))));
        for (j, (name, action)) in segments.enumerate() {
            let x = px + j as f32 * sw;
            let hov = self.hit(x, y + 3.0, sw, 24.0, action);
            let on = j == cur;
            let h = self.ease(&format!("ph:{key}:{j}"), (hov && !on) as u8 as f32);
            if h > 0.01 {
                self.fill_rr(
                    x + 2.0,
                    y + 5.0,
                    sw - 4.0,
                    20.0,
                    10.0,
                    self.cc(fg, 0.07 * h),
                );
            }
            let a = if on { 1.0 } else { 0.62 + 0.38 * h };
            self.text(name, &self.f.btn, x, y + 3.0, sw, 24.0, self.cc(fg, a));
        }
    }
}
