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
        let n = self.scripts.get(slot as usize).map_or(2, script_rows) as f64;
        18.0 + n * 18.0 + 16.0 + 12.0
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
        s.text = crate::scriptpane::clean(text);
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
        self.script_text(name, &run.text, problem, now);
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
