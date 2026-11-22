//! What an agent may read back about the pill (`hytte mcp`'s `pane_get`, `agent_status` and
//! `events`). Answers are one JSON string, built from the model on the UI thread and sent over
//! the pipe as a `Pressed` line. Nothing here changes anything.

use super::*;
use serde_json::{json, Value};
use std::collections::VecDeque;

/// Events kept for an agent to poll; older ones are forgotten.
const KEEP: usize = 100;
/// Text of a pane cut to this many characters in an answer.
const TEXT_CHARS: usize = 2000;

/// A small ring of things that happened, numbered so a caller can ask "since 17".
#[derive(Default)]
pub struct EventLog {
    next: u64,
    items: VecDeque<(u64, Value)>,
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

impl Model {
    /// Note that something happened (an action finished). `detail` is cut to one short line.
    pub fn log_event(&mut self, kind: &str, name: &str, detail: &str) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let log = &mut self.events;
        if log.next == 0 {
            // Seeded from the clock, so a restart does not hand out small numbers again: a
            // caller holding an old cursor still gets everything new.
            log.next = at * 1000;
        }
        log.next += 1;
        let one_line = detail.lines().next().unwrap_or("").trim();
        log.items.push_back((
            log.next,
            json!({"seq": log.next, "at": at, "type": kind, "name": cut(name, 64),
                   "detail": cut(one_line, 120)}),
        ));
        while log.items.len() > KEEP {
            log.items.pop_front();
        }
    }

    /// Events after `since` (0: all kept), and the number to pass next time.
    pub fn events_json(&self, since: u64) -> String {
        let items: Vec<&Value> = self
            .events
            .items
            .iter()
            .filter(|(n, _)| *n > since)
            .map(|(_, v)| v)
            .collect();
        json!({"events": items, "next": self.events.next}).to_string()
    }

    /// One pane as the user sees it right now, by name (any case), or None.
    pub fn pane_json(&self, name: &str) -> Option<String> {
        let s = self
            .scripts
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name.trim()))?;
        let rows: Vec<Value> = s
            .rows
            .iter()
            .map(|r| {
                json!({"text": cut(&r.text, 120), "value": cut(&r.value, 60),
                       "progress": r.progress, "action": r.action})
            })
            .collect();
        let buttons: Vec<Value> = s
            .buttons
            .iter()
            .map(|b| json!({"label": b.label, "action": b.action}))
            .collect();
        Some(
            json!({"name": s.name, "text": cut(&s.text, TEXT_CHARS), "rows": rows,
                   "buttons": buttons, "problem": s.problem, "busy": s.busy,
                   "age_secs": s.updated.map(|t| t.elapsed().as_secs())})
            .to_string(),
        )
    }

    /// The coding agents the pill knows: what each is doing and whether it waits for you.
    pub fn agents_json(&self) -> String {
        let agents: Vec<Value> = self
            .tasks
            .iter()
            .filter(|t| t.is_agent())
            .map(|t| {
                json!({"name": cut(&t.label, 64), "state": format!("{:?}", t.event),
                       "waiting_for_you": t.event == TaskEvent::NeedsInput,
                       "message": t.attention.as_deref().map(|m| cut(m, 200)),
                       "doing": t.line.as_deref().map(|m| cut(m, 120)),
                       "working_secs": t.started.elapsed().as_secs()})
            })
            .collect();
        json!({"agents": agents}).to_string()
    }

    /// The answer to a `pane_get` / `agent_status` / `events` command, as one JSON string.
    pub fn answer_query(&self, c: &hytte_proto::Cmd) -> String {
        match c.cmd.as_str() {
            "pane_get" => self
                .pane_json(c.id.as_deref().unwrap_or(""))
                .unwrap_or_else(|| json!({"error": "no such pane"}).to_string()),
            "agent_status" => self.agents_json(),
            "events" => self.events_json(
                c.text
                    .as_deref()
                    .and_then(|t| t.trim().parse().ok())
                    .unwrap_or(0),
            ),
            _ => json!({"error": "unknown query"}).to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScriptPane;
    use crate::tasks::{TaskState, TaskUpdate};

    fn model_with_pane() -> Model {
        let mut m = Model::default();
        m.apply_scripts(&[ScriptPane {
            name: "Deploys".into(),
            command: String::new(),
            every: 60,
            pack: None,
            dir: None,
        }]);
        m
    }

    #[test]
    fn a_pane_reads_back_as_the_user_sees_it() {
        let mut m = model_with_pane();
        m.script_text("deploys", "api ok\nweb ok", None, Instant::now());
        let v: Value = serde_json::from_str(&m.pane_json("DEPLOYS").unwrap()).unwrap();
        assert_eq!(v["name"], "Deploys");
        assert_eq!(v["text"], "api ok\nweb ok");
        m.script_text(
            "Deploys",
            r#"{"rows":[{"text":"api","value":"ok","action":"restart"}]}"#,
            None,
            Instant::now(),
        );
        let v: Value = serde_json::from_str(&m.pane_json("Deploys").unwrap()).unwrap();
        assert_eq!(v["rows"][0]["text"], "api");
        assert_eq!(v["text"], "");
        assert!(m.pane_json("nope").is_none());
        let q = hytte_proto::Cmd {
            cmd: "pane_get".into(),
            id: Some("nope".into()),
            ..Default::default()
        };
        assert!(m.answer_query(&q).contains("no such pane"));
    }

    #[test]
    fn events_are_numbered_polled_by_cursor_and_bounded() {
        let mut m = Model::default();
        assert_eq!(m.events_json(0), r#"{"events":[],"next":0}"#);
        m.log_event("action_done", "deploy", "finished\nsecond line is dropped");
        m.log_event("action_done", "restart", "exit 1");
        let all: Value = serde_json::from_str(&m.events_json(0)).unwrap();
        assert_eq!(all["events"].as_array().unwrap().len(), 2);
        assert_eq!(all["events"][0]["detail"], "finished");
        let b = all["events"][0]["seq"].as_u64().unwrap();
        assert!(b > 1_000_000_000_000, "seeded from the clock: {b}");
        assert_eq!(all["next"], b + 1);
        let after: Value = serde_json::from_str(&m.events_json(b)).unwrap();
        assert_eq!(after["events"].as_array().unwrap().len(), 1);
        assert_eq!(after["events"][0]["name"], "restart");
        assert!(m.events_json(b + 1).contains(r#""events":[]"#));
        for i in 0..250 {
            m.log_event("action_done", &format!("a{i}"), "x");
        }
        let all: Value = serde_json::from_str(&m.events_json(0)).unwrap();
        assert_eq!(all["events"].as_array().unwrap().len(), KEEP);
        assert_eq!(all["next"], b + 251, "numbers keep counting");
        // A long payload is cut, so an answer stays small.
        m.log_event("x", &"n".repeat(500), &"d".repeat(500));
        let last: Value = serde_json::from_str(&m.events_json(b + 251)).unwrap();
        assert!(last["events"][0].to_string().len() < 300);
    }

    #[test]
    fn agent_status_lists_only_agents_and_says_who_waits_for_you() {
        let mut m = Model::default();
        assert_eq!(m.agents_json(), r#"{"agents":[]}"#);
        let now = Instant::now();
        let task = |id: &str, label: &str, source: &str, ev, message: Option<&str>| {
            TaskUpdate::Upsert(TaskState {
                task_id: id.into(),
                label: label.into(),
                progress: None,
                event: ev,
                duration_ms: None,
                stderr_tail: None,
                source: source.into(),
                pid: None,
                delay_ms: 0,
                message: message.map(Into::into),
                exit_code: None,
                line: None,
                cwd: None,
                agent: Default::default(),
                updated: now,
            })
        };
        m.apply_task(
            task(
                "a1",
                "claude",
                "agent",
                TaskEvent::NeedsInput,
                Some("approve?"),
            ),
            now,
        );
        m.apply_task(
            task("c1", "cargo build", "run", TaskEvent::Start, None),
            now,
        );
        let v: Value = serde_json::from_str(&m.agents_json()).unwrap();
        assert_eq!(
            v["agents"].as_array().unwrap().len(),
            1,
            "the build is not an agent"
        );
        assert_eq!(v["agents"][0]["waiting_for_you"], true);
        assert_eq!(v["agents"][0]["message"], "approve?");
    }
}
