//! Named actions (`[[action]]`): what a click may run, the confirm step and the result card.

use super::*;

/// `"Restart:restart-api"` is a button labelled `Restart` that runs the action `restart-api`,
/// when that is one of `actions` (any case). Anything else, `"Open: docs"` or `"Later"`, is just
/// a button.
pub fn split_action(button: &str, actions: &[String]) -> Option<(String, String)> {
    let (label, name) = button.rsplit_once(':')?;
    let (label, name) = (label.trim(), name.trim());
    let known = actions.iter().find(|a| a.eq_ignore_ascii_case(name))?;
    (!label.is_empty()).then(|| (label.to_string(), known.clone()))
}

/// What pressing an action's button leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// It asks to be confirmed: the button now says "Sure?" for a few seconds.
    Arm,
    /// Run it now.
    Run,
    /// It is still running from the last press.
    Busy,
}

/// The card for a finished action: done, failed with its exit code, or stopped at the limit,
/// with the first line it printed.
pub fn action_card(name: &str, run: &crate::scriptpane::Run, timeout_s: u64, now: Instant) -> Chip {
    let first = run.text.lines().next().unwrap_or("").trim();
    let (head, tone) = match run.code {
        Some(0) => (format!("{name} finished"), Tone::Done),
        Some(n) => (format!("{name} failed (exit {n})"), Tone::Warn),
        None => (format!("{name} stopped after {timeout_s} s"), Tone::Warn),
    };
    let mut c = Chip::new(head, now);
    c.tone = tone;
    c.key = Some(format!("action:{name}"));
    if !first.is_empty() {
        c.note = Some(first.chars().take(80).collect());
    }
    c
}

impl Model {
    /// The names of the `[[action]]`s, for telling card buttons apart.
    pub fn apply_actions(&mut self, list: &[crate::config::NamedAction]) {
        self.action_names = list.iter().map(|a| a.name.clone()).collect();
    }

    /// Whether `name` has had its first click and is waiting for the second.
    pub fn action_armed(&self, name: &str) -> bool {
        self.armed_action
            .as_ref()
            .is_some_and(|(n, t)| n == name && self.now.is_none_or(|now| now < *t))
    }

    /// A click on an action. One that needs confirming arms on the first click and runs on
    /// the second within [`KILL_CONFIRM`]; one already running ignores clicks.
    pub fn press_action(&mut self, name: &str, confirm: bool, now: Instant) -> Press {
        if self.running_actions.iter().any(|n| n == name) {
            return Press::Busy;
        }
        let armed = self
            .armed_action
            .as_ref()
            .is_some_and(|(n, t)| n == name && now < *t);
        if confirm && !armed {
            self.armed_action = Some((name.to_string(), now + KILL_CONFIRM));
            return Press::Arm;
        }
        self.armed_action = None;
        self.running_actions.push(name.to_string());
        Press::Run
    }

    /// An action ended: it may be pressed again.
    pub fn action_finished(&mut self, name: &str) {
        self.running_actions.retain(|n| n != name);
    }
}
