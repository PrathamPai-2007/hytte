//! The rows of the settings window.

use super::*;

impl Renderer {
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
        // The window sends every key here, so the field always looks focused.
        self.fill_rr(x, y, w, 34.0, 17.0, self.cc(fg, 0.1));
        self.stroke_rr(
            x + 0.5,
            y + 0.5,
            w - 1.0,
            33.0,
            17.0,
            self.cc(acc, 0.85),
            1.3,
            false,
        );
        // A still bar, not a blinking one: no idle frames.
        let cx = (x + 42.0 + self.caret_x(query, query.len())).min(x + w - 48.0);
        self.fill_rr(cx, y + 8.0, 1.5, 18.0, 0.75, self.cc(acc, 0.95));
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
                x + 46.0,
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
                // The quick controls always show: that row only moves.
                let pinned = key == "panes.home";
                let hov =
                    !pinned && self.hit(10.0, y, w - 20.0, 30.0, Action::Set(key, Val::B(!on)));
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
                if pinned {
                    self.text(
                        "Always on",
                        &self.f.small_r,
                        kx - 20.0,
                        y,
                        54.0,
                        30.0,
                        self.cc(GRAY, 0.8),
                    );
                }
                // A pane's row also moves it along the tab strip.
                if let Some(pk) = key.strip_prefix("panes.") {
                    for (i, (glyph, dir)) in [('\u{E70E}', -1i8), ('\u{E70D}', 1)]
                        .into_iter()
                        .enumerate()
                    {
                        let cx = kx - 70.0 + i as f32 * 28.0;
                        self.round_btn(
                            cx,
                            y + 15.0,
                            11.0,
                            fg,
                            Action::PaneMove(pk_static(pk), dir),
                            |cx, cy, c| self.glyph(glyph, cx, cy, 11.0, c),
                        );
                    }
                }
                if !pinned && t < 0.99 {
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
                if !pinned && t > 0.01 {
                    self.fill_rr(kx, ky, 34.0, 18.0, 9.0, self.cc(acc, 0.95 * t));
                }
                let knob = mix3(fg, self.ink(), t);
                if !pinned {
                    self.circle(
                        kx + 9.0 + 16.0 * t,
                        ky + 9.0,
                        5.0 + t,
                        self.cc(knob, 0.7 + 0.25 * t),
                    );
                }
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
            if key == "general.skin" {
                // A disc of the skin's body with a dot of its glow; the name sits left of the row.
                let skin = crate::skin::Skin::ALL[j.min(crate::skin::Skin::ALL.len() - 1)];
                let (cx, cy) = (x + sw / 2.0, y + 15.0);
                let (_, bottom) = skin.body(self.light.get());
                self.circle(cx, cy, 8.5, self.cc(bottom, 1.0));
                self.circle(
                    cx,
                    cy,
                    3.5,
                    self.cc(skin.glow(), if on { 1.0 } else { 0.8 }),
                );
                self.ring(cx, cy, 8.5, self.cc(fg, 0.25 + 0.25 * a), 1.0);
                if on {
                    self.text(
                        name,
                        &self.f.small,
                        px - 62.0,
                        y,
                        56.0,
                        30.0,
                        self.cc(GRAY, 0.95),
                    );
                }
                continue;
            }
            self.text(name, &self.f.btn, x, y + 3.0, sw, 24.0, self.cc(fg, a));
        }
    }
}

/// The pane key as a `&'static str` (`Action` holds static names).
fn pk_static(k: &str) -> &'static str {
    crate::ui_state::Panel::from_key(k).map_or("", |p| p.key())
}
