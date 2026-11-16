//! A script pane (`[[script_pane]]`): the lines its command printed or a script sent, and a
//! quiet line saying how fresh they are (or what went wrong).

use super::*;

/// Height of one output line.
pub(super) const LINE: f32 = 18.0;

impl Renderer {
    pub(super) fn exp_script(&self, fr: &Frame, slot: u8, w: f32, _h: f32) {
        let Some(s) = fr.model.scripts.get(slot as usize) else {
            return;
        };
        let fg = self.fg();
        let (x, tw) = (18.0, w - 36.0);
        if !s.rows.is_empty() {
            self.script_rows(fr, s, w);
            self.script_buttons(fr, s);
        } else if s.text.is_empty() {
            let (a, b) = if s.busy {
                ("Running…".to_string(), s.command.clone())
            } else if s.command.is_empty() {
                (
                    "Nothing sent yet".to_string(),
                    format!("hytte pane set \"{}\" --text \"…\"", s.name),
                )
            } else {
                ("Waiting for output".to_string(), s.command.clone())
            };
            self.text(&a, &self.f.body, x, 12.0, tw, LINE, self.cc(fg, 0.85));
            self.text(
                &crate::secrets::mask(&b),
                &self.f.small,
                x,
                12.0 + LINE,
                tw,
                16.0,
                self.cc(GRAY, 1.0),
            );
        } else {
            let col = if s.problem.is_some() { AMBER } else { fg };
            for (i, line) in s.text.lines().enumerate() {
                self.text(
                    &crate::secrets::mask(line),
                    &self.f.body,
                    x,
                    12.0 + i as f32 * LINE,
                    tw,
                    LINE,
                    self.cc(col, if i == 0 { 0.97 } else { 0.85 }),
                );
            }
        }
        let foot = crate::ui_state::script_footer(s, fr.now);
        let fc = if s.problem.is_some() { AMBER } else { GRAY };
        let fy = crate::ui_state::script_footer_y(s);
        // A pane with a command can be run now: a small refresh button ends the footer.
        let can_refresh = !s.command.is_empty();
        self.text(
            &foot,
            &self.f.small,
            x,
            fy,
            if can_refresh { tw - 26.0 } else { tw },
            16.0,
            self.cc(fc, 1.0),
        );
        if can_refresh {
            let (cx, cy) = (w - 18.0 - 8.0, fy + 8.0);
            let hov = !s.busy
                && self.hit(
                    cx - 12.0,
                    cy - 12.0,
                    24.0,
                    24.0,
                    Action::ScriptRefresh(slot),
                );
            if hov {
                self.fill_rr(cx - 11.0, cy - 11.0, 22.0, 22.0, 11.0, self.cc(fg, 0.08));
            }
            let a = if s.busy {
                0.25
            } else if hov {
                0.9
            } else {
                0.5
            };
            self.glyph_s('\u{E72C}', cx, cy, self.cc(fg, a));
        }
    }

    /// The pane's buttons, side by side under its rows. Each runs its action with its `arg`;
    /// one that needs confirming says "Sure?" after the first click.
    fn script_buttons(&self, fr: &Frame, s: &crate::ui_state::ScriptView) {
        let fg = self.fg();
        let y = crate::ui_state::script_buttons_y(s);
        let h = crate::ui_state::SCRIPT_BTN_H - 6.0;
        let mut x = 18.0;
        for b in &s.buttons {
            let key = crate::ui_state::action_key(&b.action, b.arg.as_deref());
            let armed = fr.model.action_armed(&key);
            let label = if armed { "Sure?" } else { b.label.as_str() };
            let bw = text_w(label).max(text_w(&b.label)) + 24.0;
            let hov = self.hit(x, y, bw, h, Action::Named(b.action.clone(), b.arg.clone()));
            let bg = if armed { AMBER } else { fg };
            self.fill_rr(
                x,
                y,
                bw,
                h,
                h / 2.0,
                self.cc(
                    bg,
                    if armed {
                        0.22
                    } else if hov {
                        0.16
                    } else {
                        0.08
                    },
                ),
            );
            self.text(
                label,
                &self.f.small,
                x + (bw - text_w(label)) / 2.0,
                y + (h - 16.0) / 2.0,
                bw,
                16.0,
                self.cc(if armed { AMBER } else { fg }, 0.95),
            );
            x += bw + 8.0;
        }
    }

    /// A pane whose script gave rows: a dot in the tone, the text, the value on the right, a
    /// thin bar for progress. A row with an action is a button: it lights on hover and runs it.
    fn script_rows(&self, fr: &Frame, s: &crate::ui_state::ScriptView, w: f32) {
        use crate::scriptpane::RowTone;
        let fg = self.fg();
        let value_w = 96.0;
        for (r, (y, h)) in s
            .rows
            .iter()
            .zip(crate::ui_state::script_row_boxes(&s.rows))
        {
            let tone = match r.tone {
                RowTone::Plain => None,
                RowTone::Good => Some(GREEN),
                RowTone::Warn => Some(AMBER),
                RowTone::Bad => Some(RED),
            };
            let hov = match &r.action {
                Some(a) => self.hit(
                    8.0,
                    y,
                    w - 16.0,
                    h - 2.0,
                    Action::Named(a.clone(), r.arg.clone()),
                ),
                None => false,
            };
            if hov {
                self.fill_rr(8.0, y, w - 16.0, h - 2.0, 9.0, self.cc(fg, 0.06));
            }
            let armed = r.action.as_ref().is_some_and(|a| {
                fr.model
                    .action_armed(&crate::ui_state::action_key(a, r.arg.as_deref()))
            });
            if let Some(c) = tone {
                self.circle(24.0, y + 13.0, 3.5, self.cc(c, 0.95));
            }
            let tx = if tone.is_some() { 38.0 } else { 20.0 };
            let value = if armed { "Sure?" } else { r.value.as_str() };
            let vcol = if armed { AMBER } else { tone.unwrap_or(GRAY) };
            let text_w = (w - tx - 16.0 - if value.is_empty() { 0.0 } else { value_w }).max(40.0);
            self.text(
                &crate::secrets::mask(&r.text),
                &self.f.body,
                tx,
                y + 3.0,
                text_w,
                LINE,
                self.cc(fg, 0.95),
            );
            if !value.is_empty() {
                self.text(
                    &crate::secrets::mask(value),
                    &self.f.small_r,
                    w - 16.0 - value_w,
                    y + 4.0,
                    value_w,
                    LINE,
                    self.cc(vcol, 0.95),
                );
            }
            if let Some(p) = r.progress {
                let (bx, bw) = (tx, w - tx - 16.0);
                let by = y + LINE + 5.0;
                self.fill_rr(bx, by, bw, 3.0, 1.5, self.cc(fg, 0.1));
                let fill = bw * p as f32 / 100.0;
                if fill > 0.5 {
                    self.fill_rr(
                        bx,
                        by,
                        fill.max(3.0),
                        3.0,
                        1.5,
                        self.cc(tone.unwrap_or(BLUE), 0.95),
                    );
                }
            }
        }
    }
}
