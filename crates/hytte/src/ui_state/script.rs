//! Script panes (`[[script_pane]]`): what each shows and when it runs.

use super::*;

/// A script pane as the pill knows it: its settings and what it last showed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptView {
    pub name: String,
    /// Empty: only `hytte pane set` fills it.
    pub command: String,
    pub every: Duration,
    /// The cleaned text it shows.
    pub text: String,
    /// Or, when the script gave `{"rows": [...]}`, these rows (then `text` is empty).
    pub rows: Vec<crate::scriptpane::Row>,
    /// When the last run went wrong: "exit 3", "stopped after 10 s". None: fine.
    pub problem: Option<String>,
    /// When the text last changed (a run finished or a script pushed).
    pub updated: Option<Instant>,
    /// When its command last started, for the "every" schedule.
    pub ran: Option<Instant>,
    pub busy: bool,
}

/// Text rows a script pane shows: its lines, or two for the empty state's hint.
pub fn script_rows(s: &ScriptView) -> usize {
    match s.text.lines().count() {
        0 => 2,
        n => n,
    }
}

/// Height of one row of a pane that shows rows, and what a progress bar adds.
pub const SCRIPT_ROW_H: f32 = 26.0;
pub const SCRIPT_BAR_H: f32 = 5.0;

/// Where each of a rows pane's rows goes: top and height, from the top of the pane's content.
pub fn script_row_boxes(rows: &[crate::scriptpane::Row]) -> Vec<(f32, f32)> {
    let mut y = 10.0;
    rows.iter()
        .map(|r| {
            let h = SCRIPT_ROW_H
                + if r.progress.is_some() {
                    SCRIPT_BAR_H
                } else {
                    0.0
                };
            let b = (y, h);
            y += h;
            b
        })
        .collect()
}

/// Where the footer of a script pane starts: under the last row or text line.
pub fn script_footer_y(s: &ScriptView) -> f32 {
    if s.rows.is_empty() {
        18.0 + script_rows(s) as f32 * 18.0
    } else {
        script_row_boxes(&s.rows)
            .last()
            .map_or(10.0, |(y, h)| y + h)
            + 6.0
    }
}

/// The quiet line under a script pane: what went wrong, or how fresh the text is and when
/// the command runs again.
pub fn script_footer(s: &ScriptView, now: Instant) -> String {
    if let Some(p) = &s.problem {
        return p.clone();
    }
    let fresh = match s
        .updated
        .map(|u| now.saturating_duration_since(u).as_secs())
    {
        None => return String::new(),
        Some(0..60) => "Updated just now".to_string(),
        Some(t @ 60..3600) => format!("Updated {} min ago", t / 60),
        Some(t) => format!("Updated {} h ago", t / 3600),
    };
    if s.command.is_empty() {
        return fresh;
    }
    match s.every.as_secs() {
        t if t % 60 == 0 => format!("{fresh} · every {} min", t / 60),
        t => format!("{fresh} · every {t} s"),
    }
}

impl Model {
    /// Height of a script pane without the tab strip: its rows and the footer.
    pub fn script_h(&self, slot: u8) -> f64 {
        self.scripts
            .get(slot as usize)
            .map_or(18.0 + 2.0 * 18.0, |s| script_footer_y(s) as f64)
            + 16.0
            + 12.0
    }

    /// Apply `[[script_pane]]`s. A pane that keeps its name keeps what it showed.
    pub fn apply_scripts(&mut self, list: &[crate::config::ScriptPane]) {
        let old = std::mem::take(&mut self.scripts);
        self.scripts = list
            .iter()
            .map(|p| {
                let mut v = old
                    .iter()
                    .find(|o| o.name == p.name)
                    .cloned()
                    .unwrap_or_default();
                if v.command != p.command {
                    v.ran = None; // a new command runs at the next chance
                }
                v.name = p.name.clone();
                v.command = p.command.clone();
                v.every = Duration::from_secs(p.every);
                v
            })
            .collect();
    }

    /// The tab label of a pane: a script pane's own name, else the pane's name.
    pub fn panel_label(&self, p: Panel) -> &str {
        match p {
            Panel::Script(i) => self
                .scripts
                .get(i as usize)
                .map_or("Script", |s| s.name.as_str()),
            _ => p.name(),
        }
    }

    /// Text for script pane `name` (pushed, or a finished run). False: no such pane.
    pub fn script_text(
        &mut self,
        name: &str,
        text: &str,
        problem: Option<String>,
        now: Instant,
    ) -> bool {
        let Some(s) = self
            .scripts
            .iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))
        else {
            return false;
        };
        // Pushed text may be rows too (`hytte pane set --json`).
        match crate::scriptpane::parse_rows(text) {
            Some(rows) => {
                s.rows = rows;
                s.text.clear();
            }
            None => {
                s.rows.clear();
                s.text = crate::scriptpane::clean(text);
            }
        }
        s.problem = problem;
        s.updated = Some(now);
        true
    }

    /// A pane's command finished: show what it printed, and say so when it failed.
    pub fn script_done(&mut self, name: &str, run: crate::scriptpane::Run, now: Instant) {
        let problem = match run.code {
            Some(0) => None,
            Some(n) => Some(format!("Exited with code {n}")),
            None => Some(format!(
                "Stopped after {} s",
                crate::scriptpane::TIMEOUT.as_secs()
            )),
        };
        match run.rows {
            // A run that printed rows already was parsed on its worker thread.
            Some(rows) => {
                if let Some(s) = self.scripts.iter_mut().find(|s| s.name == name) {
                    s.rows = rows;
                    s.text.clear();
                    s.problem = problem;
                    s.updated = Some(now);
                }
            }
            None => {
                self.script_text(name, &run.text, problem, now);
            }
        }
        if let Some(s) = self.scripts.iter_mut().find(|s| s.name == name) {
            s.busy = false;
        }
    }

    /// Script pane slots whose command should run now (`only`: just the pane showing).
    /// Marks them busy and started. Each: slot, name, command.
    pub fn scripts_due(&mut self, only: Option<u8>, now: Instant) -> Vec<(u8, String, String)> {
        let mut out = vec![];
        for (i, s) in self.scripts.iter_mut().enumerate() {
            let i = i as u8;
            if only.is_some_and(|o| o != i) || s.command.is_empty() || s.busy {
                continue;
            }
            if crate::scriptpane::due(s.ran, s.every, now) {
                s.busy = true;
                s.ran = Some(now);
                out.push((i, s.name.clone(), s.command.clone()));
            }
        }
        out
    }
}
