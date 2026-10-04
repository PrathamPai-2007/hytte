//! `notch agent ...` — lets unattended agents (Claude Code, Aider, eval
//! scripts) show up in the notch and ask for a human.

use hytte_proto::{HytteMessage, TaskEvent};
use std::io::{IsTerminal, Read};

const HOOKS_CLAUDE: &str = r#"{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "notch agent resume --name \"Claude Code\"" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "notch agent needs-input --name \"Claude Code\"" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "notch agent done --name \"Claude Code\"" }] }]
  }
}"#;

pub fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  notch agent start       [--name N] [--pid P]");
    eprintln!("  notch agent needs-input [--name N] [--pid P] [--message M]   (message may arrive as hook JSON on stdin)");
    eprintln!("  notch agent resume      [--name N] [--pid P]");
    eprintln!("  notch agent done|fail   [--name N] [--pid P]");
    eprintln!("  notch agent hooks claude                                       # print Claude Code hooks config");
    std::process::exit(2);
}

pub struct Opts {
    name: String,
    pid: u32,
    message: Option<String>,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        name: "Agent".into(),
        pid: hytte_proto::sys::owner_pid(std::process::id()),
        message: None,
    };
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned();
        match (args[i].as_str(), v) {
            ("--name", Some(v)) => o.name = v,
            ("--pid", Some(v)) => o.pid = v.parse().unwrap_or_else(|_| usage()),
            ("--message", Some(v)) => o.message = Some(v),
            _ => usage(),
        }
        i += 2;
    }
    o
}

/// Agent hooks (e.g. Claude Code's `Notification`) pipe a JSON payload to stdin.
fn message_from_stdin() -> Option<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buf = String::new();
    stdin.by_ref().take(64 * 1024).read_to_string(&mut buf).ok()?;
    let v: serde_json::Value = serde_json::from_str(buf.trim()).ok()?;
    v.get("message").and_then(|m| m.as_str()).map(|m| m.chars().take(300).collect())
}

pub fn build(event: TaskEvent, o: &Opts) -> HytteMessage {
    let mut m = HytteMessage::start(format!("agent:{}:{}", o.name, o.pid), o.name.clone());
    m.event = event;
    m.source = Some("agent".into());
    m.pid = Some(o.pid);
    m.message = o.message.clone();
    m
}

pub fn run(args: &[String], send: impl Fn(&HytteMessage) -> bool) {
    let Some(sub) = args.first() else { usage() };
    if sub == "hooks" {
        match args.get(1).map(String::as_str) {
            Some("claude") => println!("{HOOKS_CLAUDE}"),
            _ => usage(),
        }
        return;
    }
    let mut o = parse(&args[1..]);
    let event = match sub.as_str() {
        "start" => TaskEvent::Start,
        "needs-input" => {
            if o.message.is_none() {
                o.message = message_from_stdin();
            }
            TaskEvent::NeedsInput
        }
        "resume" => TaskEvent::Resumed,
        "done" => TaskEvent::Done,
        "fail" => TaskEvent::Failed,
        _ => usage(),
    };
    if !send(&build(event, &o)) {
        eprintln!("notch: daemon not running (message dropped)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_agent_message() {
        let o = Opts { name: "Claude Code".into(), pid: 7, message: Some("approve?".into()) };
        let m = build(TaskEvent::NeedsInput, &o);
        assert_eq!(m.task_id, "agent:Claude Code:7");
        assert_eq!((m.source.as_deref(), m.pid), (Some("agent"), Some(7)));
        assert!(m.validate());
    }

    #[test]
    fn hooks_json_is_valid() {
        let v: serde_json::Value = serde_json::from_str(HOOKS_CLAUDE).unwrap();
        assert!(v["hooks"]["Notification"].is_array());
    }
}
