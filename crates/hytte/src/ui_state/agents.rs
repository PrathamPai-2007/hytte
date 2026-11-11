//! The Agents pane: one row per coding agent (Claude Code, Codex ...) with what it is doing now,
//! how long it has waited for you, and what it has touched this turn.
//!
//! [`Model::agents_layout`] is the pane's one description: `size` takes its height from it and
//! the renderer draws its boxes, so the two cannot drift apart.

use super::*;

/// Height of an agent row without its extra lines: the title with its state on the right.
pub const AGENT_ROW_H: f32 = 34.0;
/// Extra height for the line under the title: the project and what the agent is doing or asking.
pub const AGENT_DETAIL_H: f32 = 17.0;
/// Extra height for the "3 files · 1 subagent · 84k context" line.
pub const AGENT_META_H: f32 = 15.0;
/// Room above the first row and below the last.
pub const AGENT_PAD_H: f32 = 22.0;
/// The "N more" note when agents don't all fit.
pub const AGENT_MORE_H: f32 = 16.0;

/// The line under an agent's name.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentDetail {
    /// It is waiting for you; the text is what it asked (amber).
    Asking(String),
    /// The project, and the step it is on while it works.
    Step(String),
}

/// One agent row: where it goes in the pane and what it says.
#[derive(Debug)]
pub struct AgentRow<'a> {
    pub task: &'a TaskView,
    /// Top and height, in pane px.
    pub y: f32,
    pub h: f32,
    pub detail: Option<AgentDetail>,
    /// "3 files · 1 subagent · 84k context", only the parts that are known.
    pub meta: Option<String>,
}

#[derive(Debug)]
pub struct AgentLayout<'a> {
    pub rows: Vec<AgentRow<'a>>,
    /// Agents past the rows shown.
    pub hidden: usize,
    /// Height of everything above the tab strip.
    pub height: f64,
}

impl AgentStats {
    /// The files part: "1 file", "4 files".
    fn files_text(&self) -> Option<String> {
        match self.files.len() {
            0 => None,
            1 => Some("1 file".into()),
            n => Some(format!("{n} files")),
        }
    }

    /// The context part: "950", "84k", "1.2M" tokens, then the word.
    fn context_text(&self) -> Option<String> {
        self.context.filter(|c| *c > 0).map(|c| match c {
            0..1000 => format!("{c} context"),
            1000..1_000_000 => format!("{}k context", (c + 500) / 1000),
            _ => format!("{:.1}M context", c as f64 / 1e6),
        })
    }

    /// "Edited 4 files · 84k context", or only the part that is known; None when neither is.
    pub fn line(&self) -> Option<String> {
        let parts: Vec<String> = [
            self.files_text().map(|f| format!("Edited {f}")),
            self.context_text(),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// "4 files · 1 subagent · 84k context", or only the parts that are known.
    pub fn meta(&self) -> Option<String> {
        let subagents = match self.subagents {
            0 => None,
            1 => Some("1 subagent".to_string()),
            n => Some(format!("{n} subagents")),
        };
        let parts: Vec<String> = [self.files_text(), subagents, self.context_text()]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// The folder an agent works in, as a project name; None without a usable path.
pub fn project_of(t: &TaskView) -> Option<String> {
    t.cwd
        .as_deref()?
        .trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .filter(|f| !f.is_empty() && !f.ends_with(':'))
        .map(str::to_owned)
}

impl Model {
    /// The rows of the Agents pane, top to bottom: those waiting on you first, then the rest as
    /// the task list orders them. At most [`Model::row_cap`] are shown.
    pub fn agents_layout(&self) -> AgentLayout<'_> {
        let mut agents = self.agent_tasks();
        let hidden = agents.len().saturating_sub(self.row_cap());
        agents.truncate(self.row_cap());
        let mut y = AGENT_PAD_H / 2.0;
        let rows: Vec<AgentRow> = agents
            .into_iter()
            .map(|task| {
                // A long folder name must not push what the agent asks out of the line.
                let project = project_of(task).map(|p| match p.chars().count() {
                    0..=20 => p,
                    _ => format!("{}…", p.chars().take(19).collect::<String>()),
                });
                let join = |step: &str| match &project {
                    Some(p) => format!("{p} · {step}"),
                    None => step.to_string(),
                };
                let detail = match (&task.attention, task.running().then_some(&task.line)) {
                    (Some(msg), _) => Some(AgentDetail::Asking(join(msg))),
                    (None, Some(Some(step))) => Some(AgentDetail::Step(join(step))),
                    _ => project.clone().map(AgentDetail::Step),
                };
                let meta = task.agent.meta();
                let h = AGENT_ROW_H
                    + if detail.is_some() {
                        AGENT_DETAIL_H
                    } else {
                        0.0
                    }
                    + if meta.is_some() { AGENT_META_H } else { 0.0 };
                let row = AgentRow {
                    task,
                    y,
                    h,
                    detail,
                    meta,
                };
                y += h;
                row
            })
            .collect();
        let more = if hidden > 0 { AGENT_MORE_H } else { 0.0 };
        AgentLayout {
            rows,
            hidden,
            height: y as f64 + AGENT_PAD_H as f64 / 2.0 + more as f64,
        }
    }

    /// Visible agent tasks in row order (before the row cap).
    pub(super) fn agent_tasks(&self) -> Vec<&TaskView> {
        let mut v: Vec<&TaskView> = self
            .tasks
            .iter()
            .filter(|t| self.visible(t) && self.in_agents(t))
            .collect();
        v.sort_by(|a, b| Self::row_order(a, b));
        v
    }

    /// Whether the Agents pane (not the Tasks pane) is where `t` is listed.
    pub(super) fn in_agents(&self, t: &TaskView) -> bool {
        t.is_agent() && self.panes.agents
    }

    /// How many agents the Agents pane lists.
    pub fn agent_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| self.visible(t) && self.in_agents(t))
            .count()
    }

    /// The pane an agent's row lives in: Agents when it is switched on, else Tasks.
    pub fn agent_panel(&self) -> Panel {
        if self.panes.agents {
            Panel::Agents
        } else {
            Panel::Tasks
        }
    }
}

/// How long an agent turn ran, in words: "42 s", "8 min", "1 h 05 min".
pub fn ran_text(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
    }
}

/// The quiet line of a finished turn's card: "Ran 8 min · Edited 4 files · 84k context". A turn
/// under ten seconds says nothing about its length.
pub fn ran_note(ran: Option<Duration>, rest: Option<String>) -> Option<String> {
    let ran = ran
        .filter(|d| d.as_secs() >= 10)
        .map(|d| format!("Ran {}", ran_text(d)));
    match (ran, rest) {
        (Some(a), Some(b)) => Some(format!("{a} · {b}")),
        (a, b) => a.or(b),
    }
}

/// The label of a quiet agent: "quiet 6m", "quiet 2h". Short, so it fits where the time goes.
pub fn quiet_text(q: Duration) -> String {
    let m = q.as_secs() / 60;
    if m < 60 {
        format!("quiet {m}m")
    } else {
        format!("quiet {}h", m / 60)
    }
}

impl TaskView {
    /// How long a working agent has said nothing, once that is past `after` (zero = never).
    /// Only an agent with a known process: without one it would just be gone. It is a label,
    /// not an alarm: a long model call or a long test run is normal.
    pub fn quiet_for(&self, now: Instant, after: Duration) -> Option<Duration> {
        if after.is_zero() || !self.is_agent() || !self.running() || self.pid.is_none() {
            return None;
        }
        let q = now.saturating_duration_since(self.changed);
        (q >= after).then_some(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_timed_in_words() {
        assert_eq!(ran_text(Duration::from_secs(42)), "42 s");
        assert_eq!(ran_text(Duration::from_secs(60)), "1 min");
        assert_eq!(ran_text(Duration::from_secs(8 * 60 + 40)), "8 min");
        assert_eq!(ran_text(Duration::from_secs(3600 + 5 * 60)), "1 h 05 min");
    }

    #[test]
    fn the_card_note_names_the_run_only_when_it_was_long() {
        let rest = Some("Edited 4 files · 84k context".to_string());
        assert_eq!(
            ran_note(Some(Duration::from_secs(500)), rest.clone()).as_deref(),
            Some("Ran 8 min · Edited 4 files · 84k context")
        );
        assert_eq!(ran_note(Some(Duration::from_secs(3)), rest.clone()), rest);
        assert_eq!(
            ran_note(Some(Duration::from_secs(90)), None).as_deref(),
            Some("Ran 1 min")
        );
        assert_eq!(ran_note(None, None), None);
    }
}

impl Model {
    /// Rows waiting for your answer (agents asking). The compact pill says how many.
    pub fn waiting_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| self.visible(t) && t.needs_input())
            .count()
    }
}

/// What the compact pill says on the right of a row that is waiting for you, when more than one
/// row is: "2 waiting". None for a single wait (the row's own "waiting · 3m" stands).
pub fn waiting_text(waiting: usize) -> Option<String> {
    (waiting >= 2).then(|| format!("{waiting} waiting"))
}

#[cfg(test)]
mod waiting_tests {
    use super::*;

    #[test]
    fn two_waits_are_counted_on_the_pill_one_is_not() {
        assert_eq!(waiting_text(0), None);
        assert_eq!(waiting_text(1), None);
        assert_eq!(waiting_text(3).as_deref(), Some("3 waiting"));
    }
}
