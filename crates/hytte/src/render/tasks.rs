//! The Tasks pane: task rows and the Servers section.

use super::*;

impl Renderer {
    pub(super) fn exp_tasks(&self, fr: &Frame, w: f32, _h: f32) {
        let now = fr.now;
        let mut y = 10.0;
        for t in fr.model.rows() {
            let cy = y + 17.0;
            let finished = !t.running();
            let right_w = 74.0;
            let close_w = if finished { 24.0 } else { 0.0 };
            // Whole row focuses the owning terminal (buttons below win the hit test).
            let row_hov = match t.pid {
                Some(pid) => self.hit(
                    8.0,
                    y + 2.0,
                    w - 16.0,
                    32.0,
                    Action::FocusTerminal(pid, t.id.clone()),
                ),
                None => false,
            };
            // Hovered, a row that leads somewhere says so: an "open" arrow after its label.
            let go = if row_hov { 18.0 } else { 0.0 };
            let label_w = w - 42.0 - right_w - close_w - 22.0 - go;
            if row_hov {
                self.fill_rr(8.0, y + 2.0, w - 16.0, 32.0, 10.0, self.cc(self.fg(), 0.05));
                let gx = 42.0 + label_w + 10.0;
                self.glyph_s('\u{E8A7}', gx, y + 17.0, self.cc(self.fg(), 0.6));
            }
            // A row that wants you says so before you read it: a wash and a bar in its colour.
            if matches!(t.event, TaskEvent::Failed | TaskEvent::NeedsInput) {
                let col = task_color(t);
                self.fill_rr(8.0, y + 2.0, w - 16.0, 32.0, 10.0, self.cc(col, 0.075));
                self.fill_rr(8.0, y + 9.0, 2.5, 18.0, 1.25, self.cc(col, 0.9));
            }
            // The first row is the compact task's twin: its icon and label slide in from there.
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
                // A finished task recedes; the ones still going or wanting you stay bright.
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
            if let Some(msg) = &t.attention {
                self.label(
                    &crate::secrets::mask(msg),
                    &self.f.small,
                    42.0,
                    y + 26.0,
                    w - 42.0 - 22.0,
                    16.0,
                    self.cc(AMBER, 0.95),
                    self.over(42.0, y + 24.0, w - 64.0, 20.0),
                );
                y += 20.0;
            }
            if let (true, Some(line)) = (t.shows_line() && t.attention.is_none(), t.line.as_ref()) {
                self.label(
                    &crate::secrets::mask(line),
                    &self.f.mono,
                    42.0,
                    y + 26.0,
                    w - 42.0 - 22.0,
                    crate::ui_state::LINE_H,
                    self.cc(GRAY, 0.85),
                    self.over(42.0, y + 24.0, w - 64.0, crate::ui_state::LINE_H + 4.0),
                );
                y += crate::ui_state::LINE_H;
            }
            if t.running() {
                self.filament(t, 42.0, y + 30.0, w - 42.0 - 20.0, fr.dt, now);
            }
            y += 34.0;
            if t.failed() {
                let lines = t.stderr.len().min(4);
                if lines > 0 {
                    let bh = lines as f32 * 15.0 + 8.0;
                    self.fill_rr(14.0, y, w - 28.0, bh, 8.0, self.cc(self.fg(), 0.055));
                    for (i, l) in t.stderr.iter().rev().take(lines).rev().enumerate() {
                        self.text(
                            &crate::secrets::mask(l),
                            &self.f.mono,
                            22.0,
                            y + 4.0 + i as f32 * 15.0,
                            w - 44.0,
                            15.0,
                            self.cc(RED, 0.9),
                        );
                    }
                    y += bh + 6.0;
                }
                let mut bx = 14.0;
                if t.rerunnable() {
                    let acc = self.accent.get();
                    self.button("Rerun", bx, y, 62.0, 24.0, acc, Action::Rerun(t.id.clone()));
                    bx += 70.0;
                }
                let err = Action::CopyText(t.stderr.join("\n"));
                self.button("Copy error", bx, y, 78.0, 24.0, RED, err);
                let dismiss = Action::DismissTask(t.id.clone());
                self.button("Dismiss", bx + 86.0, y, 66.0, 24.0, self.fg(), dismiss);
                y += 28.0;
            }
        }
        let (up, down) = fr.model.rows_hidden();
        if up + down > 0 {
            let note = match (up, down) {
                (0, d) => format!("{d} more below · scroll"),
                (u, 0) => format!("{u} above · scroll"),
                (u, d) => format!("{u} above, {d} below · scroll"),
            };
            self.text(&note, &self.f.small_c, 0.0, y, w, 16.0, self.cc(GRAY, 0.8));
            y += 16.0;
        }
        if !fr.model.ports.is_empty() {
            self.servers(fr, w, y);
        }
    }

    /// The dev servers listening right now, under the tasks: the port, what runs it, and
    /// the terminal task that started it when there is one.
    pub(super) fn servers(&self, fr: &Frame, w: f32, y0: f32) {
        let m = fr.model;
        let mut y = y0 + 4.0;
        if !m.rows().is_empty() {
            self.line((16.0, y), (w - 16.0, y), self.cc(self.fg(), 0.07), 1.0);
        }
        self.text(
            "SERVERS",
            &self.f.small_l,
            18.0,
            y + 4.0,
            120.0,
            16.0,
            self.cc(GRAY, 0.75),
        );
        y += 22.0;
        let row = crate::ui_state::SERVER_ROW;
        for p in m.ports.iter().take(5) {
            let cy = y + row / 2.0;
            let task = m.task_for_port(p);
            // Restart only for a server a tracked terminal command started: that command is
            // what runs again.
            let restart = m.restart_task(p).map(|t| t.id.clone());
            // Click the row: the terminal that runs it (as a task row does).
            let row_hov = match task.and_then(|t| t.pid.map(|pid| (pid, t.id.clone()))) {
                Some((pid, id)) => self.hit(
                    8.0,
                    y + 1.0,
                    w - 16.0,
                    row - 2.0,
                    Action::FocusTerminal(pid, id),
                ),
                None => false,
            };
            if row_hov {
                self.fill_rr(
                    8.0,
                    y + 1.0,
                    w - 16.0,
                    row - 2.0,
                    10.0,
                    self.cc(self.fg(), 0.05),
                );
            }
            // A soft live dot.
            self.circle(24.0, cy, 6.0, self.cc(GREEN, 0.16));
            self.circle(24.0, cy, 3.2, self.cc(GREEN, 1.0));
            self.text(
                &format!(":{}", p.port),
                &self.f.big,
                40.0,
                y + 2.0,
                74.0,
                20.0,
                self.cc(self.fg(), 0.97),
            );
            let name = crate::ui_state::port_name(&p.exe, task.and_then(|t| t.cwd.as_deref()));
            let sub = match task {
                Some(t) => format!("{name}  ·  {}", crate::secrets::mask(&t.label)),
                None => name,
            };
            self.label(
                &sub,
                &self.f.small,
                41.0,
                y + 19.0,
                w - 41.0 - 130.0 - if restart.is_some() { 68.0 } else { 0.0 },
                14.0,
                self.cc(GRAY, 1.0),
                self.over(8.0, y + 1.0, w - 146.0, row - 2.0),
            );
            let armed = m.kill_armed.is_some_and(|(port, _)| port == p.port);
            if let Some(id) = restart {
                self.button(
                    if armed { "Restart?" } else { "Restart" },
                    w - 16.0 - 54.0 - 6.0 - 54.0 - 6.0 - 62.0,
                    cy - 12.0,
                    62.0,
                    24.0,
                    self.accent.get(),
                    Action::RestartPort(p.port, p.pid, id),
                );
            }
            self.button(
                "Open",
                w - 16.0 - 54.0 - 6.0 - 54.0,
                cy - 12.0,
                54.0,
                24.0,
                BLUE,
                Action::OpenPort(p.port),
            );
            self.button(
                if armed { "Kill?" } else { "Stop" },
                w - 16.0 - 54.0,
                cy - 12.0,
                54.0,
                24.0,
                RED,
                Action::KillPort(p.port, p.pid),
            );
            y += row;
        }
    }
}
