//! The Tasks pane: task rows, their order and expiry, and the dev-server rows.

use super::*;

/// What an agent has done this turn, from its hooks: it starts over with each prompt.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentStats {
    /// Context size in tokens, as of its last report.
    pub context: Option<u64>,
    /// The distinct files it changed (at most [`MAX_FILES`] are remembered).
    pub files: Vec<String>,
    /// Subagents it has launched.
    pub subagents: u32,
}

/// Distinct files remembered per agent turn.
pub const MAX_FILES: usize = 500;

impl AgentStats {
    /// Fold one hook report in.
    fn take(&mut self, i: crate::tasks::AgentInfo) {
        if i.context.is_some() {
            self.context = i.context;
        }
        if let Some(f) = i.file {
            if self.files.len() < MAX_FILES && !self.files.contains(&f) {
                self.files.push(f);
            }
        }
        self.subagents += i.subagent as u32;
    }
}

#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: String,
    pub label: String,
    pub event: TaskEvent,
    pub progress: Option<u8>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub stderr: Vec<String>,
    pub pid: Option<u32>,
    /// Agent message while `NeedsInput`.
    pub attention: Option<String>,
    /// The command's latest output line, shown under a running task while it changes.
    pub line: Option<String>,
    /// Who reported it ("run", "shell", "agent") and the folder it ran in: what Rerun needs.
    pub source: String,
    pub cwd: Option<String>,
    /// An agent's files, subagents and context (empty for everything else).
    pub agent: AgentStats,
    pub started: Instant,
    pub changed: Instant,
    /// Hidden until this instant (shell hooks: fast commands never show).
    pub visible_after: Instant,
}

impl TaskView {
    /// An AI coding agent's row (reported by its hooks), as opposed to a command.
    pub fn is_agent(&self) -> bool {
        self.source == "agent"
    }

    pub fn running(&self) -> bool {
        matches!(
            self.event,
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed
        )
    }
    /// A terminal command that can be run again in a new window (not an agent or download).
    pub fn rerunnable(&self) -> bool {
        matches!(self.source.as_str(), "run" | "shell")
            && self.cwd.is_some()
            && !self.label.ends_with('…') // cut off, so not the whole command
    }

    pub fn failed(&self) -> bool {
        self.event == TaskEvent::Failed
    }
    pub fn needs_input(&self) -> bool {
        self.event == TaskEvent::NeedsInput
    }
    /// A running task with output to show gets a ticker line in the expanded list.
    pub fn shows_line(&self) -> bool {
        self.running() && self.line.is_some()
    }
    /// Strength (1..0) of the red failure pulse: it fades out over [`FAIL_PULSE`]
    /// and then rests, so a failed task left on screen costs no frames.
    pub fn fail_pulse(&self, now: Instant) -> f32 {
        let since = now.saturating_duration_since(self.changed).as_secs_f32();
        (1.0 - since / FAIL_PULSE.as_secs_f32()).max(0.0)
    }
}

/// First word of a command line, lower-cased, without path or `.exe`.
/// AI coding agents started from a shell. They report themselves through their hooks
/// (`hytte setup agents`), so the shell's "running for an hour" row would only duplicate them.
pub const AGENT_CLIS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "gemini",
    "agy",
    "cursor-agent",
];

pub fn command_name(label: &str) -> String {
    let first = label.split_whitespace().next().unwrap_or("");
    let base = first
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(first)
        .to_ascii_lowercase();
    base.strip_suffix(".exe").unwrap_or(&base).to_string()
}

/// Exit codes of a command the user stopped (Ctrl+C, closing the window), not one that broke:
/// SIGINT and SIGTERM as a shell reports them, and Windows' STATUS_CONTROL_C_EXIT.
pub(super) fn is_interrupt(code: Option<i32>) -> bool {
    matches!(code, Some(130 | 143 | -1_073_741_510))
}

/// What to call a listening process on the Ports pane: Docker and WSL by their helper
/// processes (their listeners are containers and Linux servers), else the project folder of
/// the terminal command that started it when that is tracked, else the exe's name.
pub fn port_name(exe: &str, cwd: Option<&str>) -> String {
    let stem = exe.trim_end_matches(".exe");
    let low = stem.to_ascii_lowercase();
    if low.contains("docker") {
        return "Docker".into();
    }
    if low.starts_with("wsl") || low == "vmmem" {
        return "WSL".into();
    }
    let folder = cwd
        .and_then(|c| c.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next())
        .filter(|f| !f.is_empty() && !f.ends_with(':'));
    folder.unwrap_or(stem).to_string()
}

impl Model {
    pub fn apply_task(&mut self, u: TaskUpdate, now: Instant) {
        self.now = Some(now);
        match u {
            TaskUpdate::Upsert(s) => self.upsert(s, now),
            TaskUpdate::Gone(id) => self.tasks.retain(|t| !(t.id == id && t.failed())),
            TaskUpdate::Lost(id) => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    t.event = TaskEvent::Lost;
                    t.changed = now;
                }
            }
        }
    }

    pub(super) fn upsert(&mut self, s: TaskState, now: Instant) {
        let shell = s.source == "shell";
        let terminal = matches!(s.event, TaskEvent::Done | TaskEvent::Failed);
        let name = command_name(&s.label);
        if shell && (self.ignore.contains(&name) || AGENT_CLIS.contains(&name.as_str())) {
            return;
        }
        let existing = self.tasks.iter().position(|t| t.id == s.task_id);
        // What an agent did this turn: a new prompt (or a row that had finished) starts it over.
        let mut agent = existing
            .map(|i| self.tasks[i].agent.clone())
            .unwrap_or_default();
        let fresh = s.event == TaskEvent::Start
            || existing.is_some_and(|i| {
                matches!(
                    self.tasks[i].event,
                    TaskEvent::Done | TaskEvent::Failed | TaskEvent::Lost
                )
            });
        if fresh {
            agent = AgentStats::default();
        }
        // How long this agent turn ran (for the finished card and row).
        let ran = (s.source == "agent" && s.event == TaskEvent::Done)
            .then(|| existing.map(|i| now.saturating_duration_since(self.tasks[i].started)))
            .flatten();
        agent.take(s.agent.clone());
        // An agent's turn ended with a reply: its first line on a card for the "Open on
        // finish" time, Copy takes all of it. A pending ask or warning (amber) keeps the card.
        if s.source == "agent"
            && s.event == TaskEvent::Done
            && !self.finish_peek.is_zero()
            && !self.holding()
        {
            let msg = s.message.as_deref().map(str::trim).unwrap_or("");
            let first = msg
                .lines()
                .map(|l| {
                    l.trim()
                        .trim_start_matches(['#', '*', '-', ' '])
                        .trim_end_matches('*')
                })
                .find(|l| !l.is_empty());
            if let (Some(first), false) = (
                first,
                self.chip.as_ref().is_some_and(|c| c.tone == Tone::Warn),
            ) {
                let mut c = Chip::new(format!("{}: {first}", s.label), now);
                c.note = ran_note(ran, agent.line());
                c.copy = Some(msg.to_string());
                c.hold = self.finish_peek;
                c.prio = Prio::Alert;
                c.key = Some(format!("agent-done:{}", s.task_id));
                self.push_chip(c, now);
            }
        }
        // A command the user stopped (Ctrl+C, window closed) is not an error: never shown.
        // A shell command that ended before the threshold is not shown either.
        if terminal {
            let hidden = shell
                && existing
                    .map(|i| now < self.tasks[i].visible_after)
                    .unwrap_or(true);
            if hidden || is_interrupt(s.exit_code) {
                if let Some(i) = existing {
                    self.tasks.remove(i);
                }
                return;
            }
        }
        let stderr: Vec<String> = s
            .stderr_tail
            .as_deref()
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        let attention = match s.event {
            TaskEvent::NeedsInput => Some(
                s.message
                    .clone()
                    .unwrap_or_else(|| "Needs your input".into()),
            ),
            _ => None,
        };
        if let Some(i) = existing {
            let t = &mut self.tasks[i];
            // A repeated "still waiting" keeps the wait's start, so its age shows.
            let same_wait = t.event == TaskEvent::NeedsInput && s.event == TaskEvent::NeedsInput;
            if !s.label.is_empty() {
                t.label = s.label;
            }
            // A new turn of an agent whose row was still there (it finished, or you were away):
            // its clock starts over. A prompt that follows a question keeps counting.
            if fresh && s.source == "agent" && s.event != TaskEvent::Done {
                t.started = now;
            }
            t.event = s.event;
            t.progress = s.progress.or(t.progress);
            // An agent's hook doesn't time its turn: Hytte does.
            t.duration_ms = s.duration_ms.or(ran.map(|d| d.as_millis() as u64));
            t.exit_code = s.exit_code;
            t.stderr = stderr;
            t.attention = attention;
            // A message without a line (progress, a retry) keeps the last one shown.
            t.line = s.line.or(t.line.take());
            t.pid = s.pid.or(t.pid);
            t.cwd = s.cwd.or(t.cwd.take());
            t.agent = agent;
            if !same_wait {
                t.changed = now;
            }
        } else {
            self.tasks.push(TaskView {
                id: s.task_id,
                label: s.label,
                event: s.event,
                progress: s.progress,
                duration_ms: s.duration_ms,
                exit_code: s.exit_code,
                stderr,
                pid: s.pid,
                attention,
                line: s.line,
                source: s.source,
                cwd: s.cwd,
                agent,
                started: now,
                changed: now,
                visible_after: now + Duration::from_millis(s.delay_ms as u64),
            });
        }
    }

    pub fn set_ports(
        &mut self,
        p: Vec<PortInfo>,
        parents: std::collections::HashMap<u32, Vec<u32>>,
    ) {
        self.ports = p;
        self.port_parents = parents;
        if let Some((port, _)) = self.kill_armed {
            if !self.ports.iter().any(|x| x.port == port) {
                self.kill_armed = None;
            }
        }
    }

    pub(super) fn visible(&self, t: &TaskView) -> bool {
        self.now.is_none_or(|n| n >= t.visible_after)
    }

    /// Height the open per-app volume list adds to the Home pane (one row says "no apps").
    pub fn apps_h(&self) -> f64 {
        if self.apps_open {
            4.0 + APP_ROW_H as f64 * self.apps.len().max(1) as f64
        } else {
            0.0
        }
    }

    /// Forget a task quietly, whatever its state.
    pub fn remove_task(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id);
    }

    pub fn dismiss_task(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id || t.running());
    }

    /// Row order: waiting-on-you first, then failed, running, finished; newest first within each.
    pub(super) fn row_order(a: &TaskView, b: &TaskView) -> std::cmp::Ordering {
        let rank = |t: &TaskView| match t.event {
            TaskEvent::NeedsInput => 0,
            TaskEvent::Failed => 1,
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => 2,
            _ => 3,
        };
        rank(a).cmp(&rank(b)).then(b.changed.cmp(&a.changed))
    }

    /// Visible tasks: waiting-on-you first, then failed, running, finished.
    /// Task rows that fit before the list scrolls.
    pub(super) fn row_cap(&self) -> usize {
        if self.task_rows == 0 {
            4
        } else {
            self.task_rows.clamp(2, 8)
        }
    }

    /// The Tasks pane's rows: commands and downloads (agents too, while the Agents pane is off).
    pub fn rows(&self) -> Vec<&TaskView> {
        let mut v: Vec<&TaskView> = self
            .tasks
            .iter()
            .filter(|t| self.visible(t) && !self.in_agents(t))
            .collect();
        v.sort_by(|a, b| Self::row_order(a, b));
        let first = self.task_first.min(v.len().saturating_sub(self.row_cap()));
        v.drain(..first);
        v.truncate(self.row_cap());
        v
    }

    /// Rows scrolled off above and below the ones shown.
    pub fn rows_hidden(&self) -> (usize, usize) {
        let n = self.shell_count();
        let first = self.task_first.min(n.saturating_sub(self.row_cap()));
        (first, n.saturating_sub(first + self.row_cap()))
    }

    /// Wheel over the task list: `by` rows down (negative = up). False when the list is
    /// already at that end, so the wheel can page the panes instead.
    pub fn scroll_tasks(&mut self, by: i32) -> bool {
        let max = self.shell_count().saturating_sub(self.row_cap()) as i32;
        let first = self.task_first.min(max as usize) as i32;
        let next = (first + by).clamp(0, max);
        self.task_first = next as usize;
        next != first
    }

    /// The task whose process started this server (it, or one of its ancestors, is the task's).
    pub fn task_for_port(&self, p: &PortInfo) -> Option<&TaskView> {
        let chain = self.port_parents.get(&p.pid);
        self.tasks.iter().find(|t| {
            t.running()
                && t.pid
                    .is_some_and(|tp| tp == p.pid || chain.is_some_and(|c| c.contains(&tp)))
        })
    }

    /// The terminal command a server can be restarted from: the one that started it, when it
    /// is tracked and can be run again whole (not cut off, not an agent or download).
    pub fn restart_task(&self, p: &PortInfo) -> Option<&TaskView> {
        self.task_for_port(p).filter(|t| t.rerunnable())
    }

    /// Every row the pill has to show, in either pane (the compact pill and its count use this).
    pub fn visible_count(&self) -> usize {
        self.tasks.iter().filter(|t| self.visible(t)).count()
    }

    /// The rows the Tasks pane lists.
    pub fn shell_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| self.visible(t) && !self.in_agents(t))
            .count()
    }

    /// Height of the Servers section under the tasks (header + rows), 0 without ports.
    pub fn servers_h(&self) -> f64 {
        match self.ports.len().min(5) {
            0 => 0.0,
            n => 26.0 + n as f64 * SERVER_ROW as f64,
        }
    }

    /// The task shown in the compact pill.
    pub fn primary(&self) -> Option<&TaskView> {
        // Same pick as `rows()[0]` (min_by keeps the first of equals, like the stable sort),
        // without allocating: this runs several times per frame.
        self.tasks
            .iter()
            .filter(|t| self.visible(t))
            .min_by(|a, b| Self::row_order(a, b))
    }
}
