//! The Agents pane: a row per coding agent, laid out by `Model::agents_layout`.

use super::*;
use crate::ui_state::AgentDetail;

impl Renderer {
    pub(super) fn exp_agents(&self, fr: &Frame, w: f32, _h: f32) {
        let now = fr.now;
        let lay = fr.model.agents_layout();
        for r in &lay.rows {
            let t = r.task;
            let (y, cy) = (r.y, r.y + 17.0);
            let finished = !t.running();
            let right_w = 84.0;
            let close_w = if finished { 24.0 } else { 0.0 };
            // The whole row focuses the agent's terminal (buttons on top win the hit test).
            let row_hov = match t.pid {
                Some(pid) => self.hit(
                    8.0,
                    y + 2.0,
                    w - 16.0,
                    r.h - 4.0,
                    Action::FocusTerminal(pid, t.id.clone()),
                ),
                None => false,
            };
            let go = if row_hov { 18.0 } else { 0.0 };
            let label_w = w - 42.0 - right_w - close_w - 22.0 - go;
            if row_hov {
                self.fill_rr(
                    8.0,
                    y + 2.0,
                    w - 16.0,
                    r.h - 4.0,
                    10.0,
                    self.cc(self.fg(), 0.05),
                );
                self.glyph_s(
                    '\u{E8A7}',
                    42.0 + label_w + 10.0,
                    cy,
                    self.cc(self.fg(), 0.6),
                );
            }
            // An agent that wants you says so before you read it: a wash and a bar in amber.
            if matches!(t.event, TaskEvent::Failed | TaskEvent::NeedsInput) {
                let col = task_color(t);
                self.fill_rr(8.0, y + 2.0, w - 16.0, r.h - 4.0, 10.0, self.cc(col, 0.075));
                self.fill_rr(8.0, y + 9.0, 2.5, 18.0, 1.25, self.cc(col, 0.9));
            }
            // The row the compact pill shows slides its icon and name in from there.
            let first = fr.model.primary().is_some_and(|p| p.id == t.id);
            let (ic, saved) = if first {
                self.hero(
                    [20.0, 14.5, 0.0, 0.0],
                    [24.0, 27.0, 0.0, 0.0],
                    [24.0, cy, 0.0, 0.0],
                )
            } else {
                ([24.0, cy, 0.0, 0.0], None)
            };
            self.status_icon(t, ic[0], ic[1], now);
            self.hero_end(saved);
            let (lb, saved) = if first {
                let (lc, le) = self.label_ends(w, t);
                self.hero(lc, le, [42.0, y + 7.0, label_w, 20.0])
            } else {
                ([42.0, y + 7.0, label_w, 20.0], None)
            };
            let hov = saved.is_none() && self.over(8.0, y + 2.0, w - 16.0, 32.0);
            self.label(
                &crate::secrets::mask(&t.label),
                &self.f.title,
                lb[0],
                lb[1],
                lb[2],
                lb[3],
                self.cc(
                    self.fg(),
                    if t.event == TaskEvent::Done {
                        0.72
                    } else {
                        0.95
                    },
                ),
                hov,
            );
            self.hero_end(saved);
            self.text(
                &self.task_right_text(t, now),
                &self.f.small_r,
                w - 16.0 - close_w - right_w,
                y + 8.0,
                right_w,
                18.0,
                self.cc(task_color(t), 0.95),
            );
            if finished {
                let hov = self.hit(
                    w - 38.0,
                    y + 6.0,
                    24.0,
                    24.0,
                    Action::DismissTask(t.id.clone()),
                );
                let a = if hov { 1.0 } else { 0.5 };
                self.cross(w - 26.0, cy, 3.0, self.cc(self.fg(), a), 1.5);
            }
            let mut line_y = y + 26.0;
            if let Some(d) = &r.detail {
                let (text, col, font) = match d {
                    AgentDetail::Asking(m) => (m, AMBER, &self.f.small),
                    AgentDetail::Step(s) => (s, GRAY, &self.f.small),
                };
                self.label(
                    &crate::secrets::mask(text),
                    font,
                    42.0,
                    line_y,
                    w - 42.0 - 22.0,
                    16.0,
                    self.cc(col, 0.95),
                    self.over(42.0, line_y - 2.0, w - 64.0, 20.0),
                );
                line_y += crate::ui_state::AGENT_DETAIL_H;
            }
            if let Some(meta) = &r.meta {
                self.text(
                    meta,
                    &self.f.small,
                    42.0,
                    line_y,
                    w - 42.0 - 22.0,
                    14.0,
                    self.cc(GRAY, 0.7),
                );
            }
            if t.running() {
                self.filament(t, 42.0, y + r.h - 4.0, w - 42.0 - 20.0, fr.dt, now);
            }
        }
        if lay.hidden > 0 {
            let y = lay.rows.last().map_or(0.0, |r| r.y + r.h) + 2.0;
            self.text(
                &format!("{} more agents", lay.hidden),
                &self.f.small_c,
                0.0,
                y,
                w,
                16.0,
                self.cc(GRAY, 0.8),
            );
        }
    }
}
