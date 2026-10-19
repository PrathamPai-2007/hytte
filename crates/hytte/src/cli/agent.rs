//! `hytte agent ...` — lets unattended agents (Claude Code, Aider, eval
//! scripts) show up in the notch and ask for a human.

use hytte_proto::{Ask, HytteMessage, TaskEvent, PROTOCOL_VERSION};
use std::io::{IsTerminal, Read};

pub fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  hytte agent start       [--name N] [--pid P]");
    eprintln!("  hytte agent needs-input [--name N] [--pid P] [--message M]   (message may arrive as hook JSON on stdin)");
    eprintln!("  hytte agent resume      [--name N] [--pid P]");
    eprintln!("  hytte agent done|fail   [--name N] [--pid P]");
    eprintln!("  hytte agent tool        [--name N] [--pid P] [--detail D]   (tool call may arrive as hook JSON on stdin)");
    eprintln!("  hytte agent ask         [--name N] [--pid P]                 (permission request as hook JSON on stdin; answered on the pill)");
    eprintln!("  hytte agent hooks claude                                        # print the Claude Code hooks (`hytte setup agents` installs them)");
    eprintln!("  --ack prints {{}} after any of these, for agents that read hook output as JSON");
    std::process::exit(2);
}

pub struct Opts {
    name: String,
    pid: u32,
    message: Option<String>,
    /// What the agent is doing right now (`hytte agent tool`).
    detail: Option<String>,
    /// Print `{}` when done: Gemini, Antigravity and Cursor read a hook's stdout as JSON.
    ack: bool,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        name: "Agent".into(),
        pid: hytte_proto::sys::owner_pid(std::process::id()),
        message: None,
        detail: None,
        ack: false,
    };
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--ack" {
            o.ack = true;
            i += 1;
            continue;
        }
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

fn payload_from_stdin() -> Option<serde_json::Value> {
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
    serde_json::from_str(buf.trim()).ok()
}

fn tool_from_stdin() -> Option<String> {
    describe_tool(&payload_from_stdin()?)
}

/// What a permission request asks, for the pill. A command the pill can't show in full is
/// flagged at the *start* (the chip clips its end), so nobody approves text they never saw.
pub fn ask_text(payload: &serde_json::Value) -> Option<String> {
    let tool = payload.get("tool_name")?.as_str()?;
    let text = describe_tool(payload).unwrap_or_else(|| format!("Using {tool}"));
    let cmd = payload
        .get("tool_input")
        .and_then(|i| i.get("command"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim();
    let lines = cmd.lines().count();
    let flag = if lines > 1 {
        format!("({lines} lines, see terminal) ")
    } else if cmd.chars().count() > 60 {
        "(long, see terminal) ".to_string()
    } else {
        String::new()
    };
    Some(format!("{flag}{text}"))
}

/// `PermissionRequest` hook output (Claude Code, Codex) for an answer given on the pill.
/// `message` is only read with a deny: it tells the agent why.
pub fn decision_json(allow: bool) -> serde_json::Value {
    let decision = if allow {
        serde_json::json!({ "behavior": "allow" })
    } else {
        serde_json::json!({ "behavior": "deny", "message": "Denied from Hytte" })
    };
    serde_json::json!({
        "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
    })
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

pub fn run(
    args: &[String],
    send: impl Fn(&HytteMessage) -> bool,
    ask: impl Fn(&Ask) -> Option<bool>,
) {
    let Some(sub) = args.first() else { usage() };
    if sub == "hooks" {
        // `--tools` / `--approve` were opt-ins before 1.1; both are always in now.
        let flags = args.get(2..).unwrap_or_default();
        if args.get(1).map(String::as_str) != Some("claude")
            || flags.iter().any(|f| f != "--tools" && f != "--approve")
        {
            usage();
        }
        let exe = std::env::current_exe()
            .map(|p| crate::agents::exe_cmd(&p))
            .unwrap_or_else(|_| "hytte".into());
        let v = serde_json::json!({ "hooks": crate::agents::claude_hooks(&exe, "Claude Code") });
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return;
    }
    let o = parse(&args[1..]);
    let ack = o.ack;
    report(sub, o, send, ask);
    if ack {
        println!("{{}}");
    }
}

fn report(
    sub: &str,
    mut o: Opts,
    send: impl Fn(&HytteMessage) -> bool,
    ask: impl Fn(&Ask) -> Option<bool>,
) {
    if sub == "ask" {
        // No answer (no daemon, timeout, dismissed): print nothing, so Claude asks in the terminal.
        let Some(text) = payload_from_stdin().as_ref().and_then(ask_text) else {
            return;
        };
        let q = Ask {
            v: PROTOCOL_VERSION,
            name: o.name.clone(),
            pid: Some(o.pid),
            text: text.chars().take(300).collect(),
        };
        if let Some(allow) = ask(&q) {
            println!("{}", decision_json(allow));
        }
        return;
    }
    let event = match sub {
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
        eprintln!("hytte: daemon not running (message dropped)");
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
            ack: false,
        };
        let m = build(TaskEvent::NeedsInput, &o);
        assert_eq!(m.task_id, "agent:Claude Code:7");
        assert_eq!((m.source.as_deref(), m.pid), (Some("agent"), Some(7)));
        assert!(m.validate());
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
            ack: false,
        };
        let m = build(TaskEvent::Progress, &o);
        assert_eq!(m.line.as_deref(), Some("Editing render.rs"));
        assert_eq!(m.task_id, "agent:Claude Code:7");
        assert!(m.validate());
    }

    #[test]
    fn ack_is_a_flag_without_a_value() {
        let o = parse(&["--ack".into(), "--name".into(), "Gemini".into()]);
        assert!(o.ack);
        assert_eq!(o.name, "Gemini");
    }

    #[test]
    fn ask_text_never_hides_part_of_a_command() {
        let ask = |v: serde_json::Value| ask_text(&v).unwrap();
        assert_eq!(
            ask(serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "cargo build"}})),
            "Running: cargo build"
        );
        let multi = ask(
            serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "echo hi\nrm -rf /"}}),
        );
        assert!(multi.starts_with("(2 lines, see terminal) "), "{multi}");
        let long = ask(
            serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "x".repeat(200)}}),
        );
        assert!(long.starts_with("(long, see terminal) "), "{long}");
        // A tool we can't describe still names itself.
        assert_eq!(
            ask(serde_json::json!({"tool_name": "Edit", "tool_input": {}})),
            "Using Edit"
        );
        assert!(ask_text(&serde_json::json!({})).is_none());
    }

    #[test]
    fn decision_json_matches_claude_schema() {
        let v = decision_json(true);
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert!(v["hookSpecificOutput"]["decision"].get("message").is_none());
        let v = decision_json(false);
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(v["hookSpecificOutput"]["decision"]["message"].is_string());
    }
}
