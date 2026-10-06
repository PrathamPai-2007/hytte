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

/// The same hooks plus one that reports every tool call, so the pill can say what the agent is
/// doing right now. It runs `notch` before each tool call, which costs a few milliseconds each.
const HOOKS_CLAUDE_TOOLS: &str = r#"{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "notch agent resume --name \"Claude Code\"" }] }],
    "PreToolUse":       [{ "matcher": "*", "hooks": [{ "type": "command", "command": "notch agent tool --name \"Claude Code\"" }] }],
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
    eprintln!("  notch agent tool        [--name N] [--pid P] [--detail D]   (tool call may arrive as hook JSON on stdin)");
    eprintln!("  notch agent hooks claude [--tools]                             # print Claude Code hooks config");
    std::process::exit(2);
}

pub struct Opts {
    name: String,
    pid: u32,
    message: Option<String>,
    /// What the agent is doing right now (`notch agent tool`).
    detail: Option<String>,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        name: "Agent".into(),
        pid: hytte_proto::sys::owner_pid(std::process::id()),
        message: None,
        detail: None,
    };
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned();
        match (args[i].as_str(), v) {
            ("--name", Some(v)) => o.name = v,
            ("--pid", Some(v)) => o.pid = v.parse().unwrap_or_else(|_| usage()),
            ("--message", Some(v)) => o.message = Some(v),
            ("--detail", Some(v)) => o.detail = Some(v),
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
    stdin
        .by_ref()
        .take(64 * 1024)
        .read_to_string(&mut buf)
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(buf.trim()).ok()?;
    v.get("message")
        .and_then(|m| m.as_str())
        .map(|m| m.chars().take(300).collect())
}

fn tool_from_stdin() -> Option<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buf = String::new();
    stdin
        .by_ref()
        .take(64 * 1024)
        .read_to_string(&mut buf)
        .ok()?;
    describe_tool(&serde_json::from_str(buf.trim()).ok()?)
}

/// A short phrase for a tool call, from a Claude Code `PreToolUse` payload
/// (`{"tool_name": "Edit", "tool_input": {...}}`): "Editing render.rs", "Running: cargo build".
pub fn describe_tool(payload: &serde_json::Value) -> Option<String> {
    let tool = payload.get("tool_name")?.as_str()?;
    let input = payload.get("tool_input");
    let field = |k: &str| input.and_then(|i| i.get(k)).and_then(|v| v.as_str());
    let file = || {
        field("file_path")
            .or_else(|| field("notebook_path"))
            .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p).to_string())
    };
    let text = match tool {
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("Editing {}", file()?),
        "Write" => format!("Writing {}", file()?),
        "Read" => format!("Reading {}", file()?),
        "Bash" => format!("Running: {}", field("command")?.lines().next()?.trim()),
        "Grep" => format!("Searching: {}", field("pattern")?),
        "Glob" => format!("Finding {}", field("pattern")?),
        "WebFetch" => format!("Fetching {}", field("url")?),
        "WebSearch" => "Searching the web".to_string(),
        "Task" | "Agent" => format!("Subagent: {}", field("description").unwrap_or("working")),
        // MCP tools are named mcp__server__tool.
        other => format!("Using {}", other.rsplit("__").next().unwrap_or(other)),
    };
    let text: String = text.chars().take(120).collect();
    (!text.trim().is_empty()).then_some(text)
}

pub fn build(event: TaskEvent, o: &Opts) -> HytteMessage {
    let mut m = HytteMessage::start(format!("agent:{}:{}", o.name, o.pid), o.name.clone());
    m.event = event;
    m.source = Some("agent".into());
    m.pid = Some(o.pid);
    m.message = o.message.clone();
    m.line = o.detail.clone();
    m
}

pub fn run(args: &[String], send: impl Fn(&HytteMessage) -> bool) {
    let Some(sub) = args.first() else { usage() };
    if sub == "hooks" {
        match (
            args.get(1).map(String::as_str),
            args.get(2).map(String::as_str),
        ) {
            (Some("claude"), None) => println!("{HOOKS_CLAUDE}"),
            (Some("claude"), Some("--tools")) => println!("{HOOKS_CLAUDE_TOOLS}"),
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
        // A tool call is progress of an agent that is working: no detail, nothing to say.
        "tool" => {
            if o.detail.is_none() {
                o.detail = tool_from_stdin();
            }
            if o.detail.is_none() {
                return;
            }
            TaskEvent::Progress
        }
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
        let o = Opts {
            name: "Claude Code".into(),
            pid: 7,
            message: Some("approve?".into()),
            detail: None,
        };
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

    #[test]
    fn tool_calls_become_short_phrases() {
        let say = |v: serde_json::Value| describe_tool(&v);
        assert_eq!(
            say(
                serde_json::json!({"tool_name": "Edit", "tool_input": {"file_path": r"C:\src\render.rs"}})
            ),
            Some("Editing render.rs".into())
        );
        assert_eq!(
            say(
                serde_json::json!({"tool_name": "Read", "tool_input": {"file_path": "/home/me/lib.rs"}})
            ),
            Some("Reading lib.rs".into())
        );
        assert_eq!(
            say(
                serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "cargo build --release\ncargo test"}})
            ),
            Some("Running: cargo build --release".into())
        );
        assert_eq!(
            say(serde_json::json!({"tool_name": "Grep", "tool_input": {"pattern": "fn main"}})),
            Some("Searching: fn main".into())
        );
        assert_eq!(
            say(serde_json::json!({"tool_name": "mcp__github__create_issue", "tool_input": {}})),
            Some("Using create_issue".into())
        );
        // A tool we describe by its input, but the input is missing: say nothing rather than guess.
        assert_eq!(
            say(serde_json::json!({"tool_name": "Edit", "tool_input": {}})),
            None
        );
        assert_eq!(say(serde_json::json!({"nothing": 1})), None);
        // Long commands are capped.
        let long = "x".repeat(400);
        let c =
            say(serde_json::json!({"tool_name": "Bash", "tool_input": {"command": long}})).unwrap();
        assert_eq!(c.chars().count(), 120);
    }

    #[test]
    fn tool_messages_carry_the_detail_as_a_running_update() {
        let o = Opts {
            name: "Claude Code".into(),
            pid: 7,
            message: None,
            detail: Some("Editing render.rs".into()),
        };
        let m = build(TaskEvent::Progress, &o);
        assert_eq!(m.line.as_deref(), Some("Editing render.rs"));
        assert_eq!(m.task_id, "agent:Claude Code:7");
        assert!(m.validate());
    }

    #[test]
    fn tools_hooks_json_is_valid_and_adds_pre_tool_use() {
        let v: serde_json::Value = serde_json::from_str(HOOKS_CLAUDE_TOOLS).unwrap();
        assert!(v["hooks"]["PreToolUse"].is_array());
        assert!(v["hooks"]["Stop"].is_array());
        // The plain block stays free of the per-tool hook.
        let plain: serde_json::Value = serde_json::from_str(HOOKS_CLAUDE).unwrap();
        assert!(plain["hooks"].get("PreToolUse").is_none());
    }
}
