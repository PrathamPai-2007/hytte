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
