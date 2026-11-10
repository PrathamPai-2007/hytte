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
        self.text(
            &foot,
            &self.f.small,
            x,
            crate::ui_state::script_footer_y(s),
            tw,
            16.0,
            self.cc(fc, 1.0),
        );
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
                Some(a) => self.hit(8.0, y, w - 16.0, h - 2.0, Action::Named(a.clone())),
                None => false,
            };
            if hov {
                self.fill_rr(8.0, y, w - 16.0, h - 2.0, 9.0, self.cc(fg, 0.06));
            }
            let armed = r.action.as_ref().is_some_and(|a| fr.model.action_armed(a));
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
