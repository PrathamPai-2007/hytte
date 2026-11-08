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
        if s.text.is_empty() {
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
        let n = crate::ui_state::script_rows(s) as f32;
        let fc = if s.problem.is_some() { AMBER } else { GRAY };
        self.text(
            &foot,
            &self.f.small,
            x,
            18.0 + n * LINE,
            tw,
            16.0,
            self.cc(fc, 1.0),
        );
    }
}
