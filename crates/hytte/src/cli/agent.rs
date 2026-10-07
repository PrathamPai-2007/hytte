//! `notch agent ...` — lets unattended agents (Claude Code, Aider, eval
//! scripts) show up in the notch and ask for a human.

use hytte_proto::{Ask, HytteMessage, TaskEvent, PROTOCOL_VERSION};
use std::io::{IsTerminal, Read};

const HOOKS_CLAUDE: &str = r#"{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "notch agent resume --name \"Claude Code\"" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "notch agent needs-input --name \"Claude Code\"" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "notch agent done --name \"Claude Code\"" }] }]
  }
}"#;

/// The plain hooks, plus (`--tools`) one that reports every tool call so the pill can say what
/// the agent is doing (it runs `notch` before each call, a few milliseconds each), and
/// (`--approve`) one that puts Claude's permission prompts on the pill as Allow / Deny.
fn hooks_claude(tools: bool, approve: bool) -> serde_json::Value {
    let mut v: serde_json::Value = serde_json::from_str(HOOKS_CLAUDE).expect("valid hooks JSON");
    let hook = |c: &str| serde_json::json!([{ "matcher": "*", "hooks": [{ "type": "command", "command": c }] }]);
    if tools {
        v["hooks"]["PreToolUse"] = hook(r#"notch agent tool --name "Claude Code""#);
    }
    if approve {
        v["hooks"]["PermissionRequest"] = hook(r#"notch agent ask --name "Claude Code""#);
    }
    v
}

pub fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  notch agent start       [--name N] [--pid P]");
    eprintln!("  notch agent needs-input [--name N] [--pid P] [--message M]   (message may arrive as hook JSON on stdin)");
    eprintln!("  notch agent resume      [--name N] [--pid P]");
    eprintln!("  notch agent done|fail   [--name N] [--pid P]");
    eprintln!("  notch agent tool        [--name N] [--pid P] [--detail D]   (tool call may arrive as hook JSON on stdin)");
    eprintln!("  notch agent ask         [--name N] [--pid P]                 (permission request as hook JSON on stdin; answered on the pill)");
    eprintln!("  notch agent hooks claude [--tools] [--approve]                 # print Claude Code hooks config");
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

/// Claude Code `PermissionRequest` hook output for an answer given on the pill.
pub fn decision_json(allow: bool) -> serde_json::Value {
    let (behavior, message) = if allow {
        ("allow", "Allowed from Hytte")
    } else {
        ("deny", "Denied from Hytte")
    };
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": behavior, "message": message }
        }
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
        let flags = args.get(2..).unwrap_or_default();
        if args.get(1).map(String::as_str) != Some("claude")
            || flags.iter().any(|f| f != "--tools" && f != "--approve")
        {
            usage();
        }
        let has = |f: &str| flags.iter().any(|a| a == f);
        let v = hooks_claude(has("--tools"), has("--approve"));
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return;
    }
    let mut o = parse(&args[1..]);
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
    fn hook_flags_add_their_hooks_only() {
        let v = hooks_claude(true, false);
        assert!(v["hooks"]["PreToolUse"].is_array());
        assert!(v["hooks"]["Stop"].is_array());
        assert!(v["hooks"].get("PermissionRequest").is_none());
        let v = hooks_claude(false, true);
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            r#"notch agent ask --name "Claude Code""#
        );
        let plain = hooks_claude(false, false);
        assert!(plain["hooks"].get("PreToolUse").is_none());
        assert!(plain["hooks"].get("PermissionRequest").is_none());
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
        let v = decision_json(false);
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
    }
}
