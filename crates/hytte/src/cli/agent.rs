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
    /// A file the tool call changes (from the hook JSON).
    file: Option<String>,
    /// The tool call launches a subagent.
    subagent: bool,
    /// Context size in tokens (from the Claude Code transcript).
    context: Option<u64>,
    /// The project folder: the hook's `cwd`, else where this hook was started (a hook runs in the
    /// agent's project).
    cwd: Option<String>,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        name: "Agent".into(),
        pid: hytte_proto::sys::owner_pid(std::process::id()),
        message: None,
        detail: None,
        ack: false,
        file: None,
        subagent: false,
        context: None,
        cwd: std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string()),
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

/// The file a `PreToolUse` payload is about to change (Edit, MultiEdit, Write, NotebookEdit), as
/// the full path the tool was given.
pub fn touched_file(payload: &serde_json::Value) -> Option<String> {
    if !matches!(
        payload.get("tool_name")?.as_str()?,
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit"
    ) {
        return None;
    }
    let input = payload.get("tool_input")?;
    ["file_path", "notebook_path"]
        .iter()
        .find_map(|k| input.get(k)?.as_str())
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

/// Whether a `PreToolUse` payload launches a subagent (Claude Code's `Task` / `Agent` tool).
pub fn launches_subagent(payload: &serde_json::Value) -> bool {
    matches!(
        payload.get("tool_name").and_then(|t| t.as_str()),
        Some("Task" | "Agent")
    )
}

/// The last `bytes` of a file as text (the start may cut a line; callers skip what doesn't parse).
fn read_tail(path: &std::path::Path, bytes: u64) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(bytes))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Tokens in the context of a Claude Code transcript's last assistant turn: its input, counting
/// what came from the prompt cache. Looks at the last 64 KB first, then the last 512 KB (a turn
/// with a huge tool result can push the previous one out of the first).
pub fn context_in_transcript(path: &std::path::Path) -> Option<u64> {
    let usage = |text: &str| {
        text.lines().rev().find_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            if v.get("type")?.as_str()? != "assistant" {
                return None;
            }
            let u = v.get("message")?.get("usage")?;
            let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
            let total =
                n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens");
            (total > 0).then_some(total)
        })
    };
    read_tail(path, 64 * 1024)
        .and_then(|t| usage(&t))
        .or_else(|| read_tail(path, 512 * 1024).and_then(|t| usage(&t)))
}

/// The project folder a hook payload names (Claude Code puts `cwd` in every one).
fn cwd_of(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("cwd")?
        .as_str()
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
}

fn context_of(payload: &serde_json::Value) -> Option<u64> {
    context_in_transcript(std::path::Path::new(
        payload.get("transcript_path")?.as_str()?,
    ))
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

/// The text of an `AskUserQuestion` request, if that is what the payload is.
fn question_text(payload: &serde_json::Value) -> Option<String> {
    if payload.get("tool_name")?.as_str()? != "AskUserQuestion" {
        return None;
    }
    let q = payload
        .get("tool_input")?
        .get("questions")?
        .get(0)?
        .get("question")?
        .as_str()?;
    Some(q.chars().take(300).collect())
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
            .map(|p| {
                // Folder and name: "src/render.rs", so two files with one name can be told apart.
                let mut parts = p.rsplit(['\\', '/']).filter(|s| !s.is_empty());
                let name = parts.next().unwrap_or(p);
                parts
                    .next()
                    .map_or(name.into(), |dir| format!("{dir}/{name}"))
            })
    };
    let text = match tool {
        "Edit" | "MultiEdit" | "NotebookEdit" => format!("Editing {}", file()?),
        "Write" => format!("Writing {}", file()?),
        "Read" => format!("Reading {}", file()?),
        "Bash" => {
            // The first line with something on it: a command can start with blank lines.
            let first = field("command")?
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())?;
            format!("Running: {first}")
        }
        "Grep" => format!("Searching: {}", field("pattern")?),
        "Glob" => format!("Finding {}", field("pattern")?),
        "WebFetch" => format!("Fetching {}", field("url")?),
        "WebSearch" => "Searching the web".to_string(),
        "Task" | "Agent" => format!("Subagent: {}", field("description").unwrap_or("working")),
        // MCP tools are named mcp__server__tool; show who runs it and a short view of the input.
        other => match other.strip_prefix("mcp__").and_then(|r| r.split_once("__")) {
            Some((server, tool)) => {
                let args = input
                    .filter(|i| i.as_object().is_some_and(|o| !o.is_empty()))
                    .map(|i| format!(" {}", i.to_string().chars().take(60).collect::<String>()))
                    .unwrap_or_default();
                format!("Using {server}: {tool}{args}")
            }
            None => format!("Using {other}"),
        },
    };
    let text: String = text.chars().take(120).collect();
    (!text.trim().is_empty()).then_some(text)
}

/// The agent's last reply, from a turn-end hook payload: Codex / Claude Code
/// `last_assistant_message`, Gemini `prompt_response`, else the tail of the Claude Code
/// transcript. At most 4000 characters (the pill shows its first line; Copy takes it all).
pub fn final_answer(payload: &serde_json::Value) -> Option<String> {
    let text = ["last_assistant_message", "prompt_response"]
        .iter()
        .find_map(|k| payload.get(k)?.as_str().map(str::to_owned))
        .or_else(|| {
            let p = payload.get("transcript_path")?.as_str()?;
            last_reply_in_transcript(std::path::Path::new(p))
        })?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(4000).collect())
}

/// The text of the last assistant entry in a Claude Code JSONL transcript (its last 512 KB).
fn last_reply_in_transcript(path: &std::path::Path) -> Option<String> {
    read_tail(path, 512 * 1024)?
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find_map(|v| {
            if v.get("type")?.as_str()? != "assistant" {
                return None;
            }
            let text: Vec<&str> = v
                .get("message")?
                .get("content")?
                .as_array()?
                .iter()
                .filter(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|c| c.get("text")?.as_str())
                .collect();
            (!text.is_empty()).then(|| text.join("\n"))
        })
}

pub fn build(event: TaskEvent, o: &Opts) -> HytteMessage {
    let mut m = HytteMessage::start(format!("agent:{}:{}", o.name, o.pid), o.name.clone());
    m.event = event;
    m.source = Some("agent".into());
    m.pid = Some(o.pid);
    m.message = o.message.clone();
    m.line = o.detail.clone();
    m.cwd = o.cwd.clone();
    m.file = o.file.clone();
    m.subagent = o.subagent;
    m.context = o.context;
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
        let payload = payload_from_stdin();
        // A question for the human is not a yes/no: Allow would answer nothing. Show it as
        // waiting; clicking the row goes to the terminal, where it is answered.
        if let Some(q) = payload.as_ref().and_then(question_text) {
            o.message = Some(q);
            let _ = send(&build(TaskEvent::NeedsInput, &o));
            return;
        }
        // No answer (no daemon, timeout, dismissed): print nothing, so Claude asks in the terminal.
        let Some(text) = payload.as_ref().and_then(ask_text) else {
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
            if let Some(p) = payload_from_stdin() {
                if o.detail.is_none() {
                    o.detail = describe_tool(&p);
                }
                o.cwd = cwd_of(&p).or(o.cwd.take());
                o.file = touched_file(&p);
                o.subagent = launches_subagent(&p);
                o.context = context_of(&p);
            }
            if o.detail.is_none() {
                return;
            }
            TaskEvent::Progress
        }
        "done" => {
            let payload = payload_from_stdin();
            if o.message.is_none() {
                o.message = payload.as_ref().and_then(final_answer);
            }
            o.context = payload.as_ref().and_then(context_of);
            o.cwd = payload.as_ref().and_then(cwd_of).or(o.cwd.take());
            TaskEvent::Done
        }
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
    fn final_answer_from_payload_or_transcript() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert_eq!(
            final_answer(&v(
                r#"{"last_assistant_message":"  All done.\nTests pass. "}"#
            )),
            Some("All done.\nTests pass.".into())
        );
        assert_eq!(
            final_answer(&v(r#"{"prompt_response":"Fixed it"}"#)),
            Some("Fixed it".into())
        );
        let f = std::env::temp_dir().join("hytte-transcript.jsonl");
        std::fs::write(
            &f,
            concat!(
                r#"{"type":"user","message":{"content":"hi"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"old"}]}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"},{"type":"text","text":"Build is green."}]}}"#,
                "\n",
                r#"{"type":"system","subtype":"stop"}"#,
                "\n"
            ),
        )
        .unwrap();
        let p = serde_json::json!({ "transcript_path": f });
        assert_eq!(final_answer(&p), Some("Build is green.".into()));
        assert_eq!(final_answer(&v(r#"{"stop_hook_active":false}"#)), None);
        assert_eq!(
            final_answer(&v(r#"{"last_assistant_message":"   "}"#)),
            None
        );
    }

    #[test]
    fn builds_agent_message() {
        let o = Opts {
            name: "Claude Code".into(),
            pid: 7,
            message: Some("approve?".into()),
            detail: None,
            ack: false,
            file: None,
            subagent: false,
            context: None,
            cwd: None,
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
            Some("Editing src/render.rs".into())
        );
        assert_eq!(
            say(
                serde_json::json!({"tool_name": "Read", "tool_input": {"file_path": "/home/me/lib.rs"}})
            ),
            Some("Reading me/lib.rs".into())
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
            Some("Using github: create_issue".into())
        );
        assert_eq!(
            say(
                serde_json::json!({"tool_name": "mcp__github__create_issue", "tool_input": {"title": "x"}})
            ),
            Some(r#"Using github: create_issue {"title":"x"}"#.into())
        );
        // Leading blank lines don't hide the command.
        assert_eq!(
            say(
                serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "\n\n  git push\n"}})
            ),
            Some("Running: git push".into())
        );
        assert_eq!(
            question_text(
                &serde_json::json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": [{"question": "Which one?"}]}})
            ),
            Some("Which one?".into())
        );
        assert_eq!(
            question_text(&serde_json::json!({"tool_name": "Bash"})),
            None
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
            file: None,
            subagent: false,
            context: None,
            cwd: None,
        };
        let m = build(TaskEvent::Progress, &o);
        assert_eq!(m.line.as_deref(), Some("Editing render.rs"));
        assert_eq!(m.task_id, "agent:Claude Code:7");
        assert!(m.validate());
    }

    #[test]
    fn files_and_subagents_come_from_the_tool_payload() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        for tool in ["Edit", "MultiEdit", "Write"] {
            let p =
                serde_json::json!({"tool_name": tool, "tool_input": {"file_path": r"C:\src\a.rs"}});
            assert_eq!(touched_file(&p).as_deref(), Some(r"C:\src\a.rs"), "{tool}");
        }
        let nb = v(r#"{"tool_name":"NotebookEdit","tool_input":{"notebook_path":"n.ipynb"}}"#);
        assert_eq!(touched_file(&nb).as_deref(), Some("n.ipynb"));
        // Reading or running things changes nothing the pill should count.
        for p in [
            r#"{"tool_name":"Read","tool_input":{"file_path":"a.rs"}}"#,
            r#"{"tool_name":"Bash","tool_input":{"command":"ls"}}"#,
            r#"{"tool_name":"Edit","tool_input":{}}"#,
            r#"{"tool_name":"Write","tool_input":{"file_path":""}}"#,
            r#"{"nothing":1}"#,
        ] {
            assert_eq!(touched_file(&v(p)), None, "{p}");
        }
        assert!(launches_subagent(&v(r#"{"tool_name":"Task"}"#)));
        assert!(launches_subagent(&v(r#"{"tool_name":"Agent"}"#)));
        assert!(!launches_subagent(&v(r#"{"tool_name":"Bash"}"#)));
        // And they ride the message.
        assert_eq!(
            cwd_of(&serde_json::json!({"cwd": r"C:\src\hytte"})).as_deref(),
            Some(r"C:\src\hytte")
        );
        assert_eq!(cwd_of(&serde_json::json!({"cwd": ""})), None);
        let o = Opts {
            file: Some("a.rs".into()),
            subagent: true,
            context: Some(1234),
            ..parse(&[])
        };
        let m = build(TaskEvent::Progress, &o);
        assert_eq!(
            (m.file.as_deref(), m.subagent, m.context),
            (Some("a.rs"), true, Some(1234))
        );
        assert!(m.validate());
    }

    #[test]
    fn context_is_the_last_assistant_turns_input() {
        let f = std::env::temp_dir().join("hytte-context.jsonl");
        let line = |input: u64, create: u64, read: u64| {
            format!(
                r#"{{"type":"assistant","message":{{"usage":{{"input_tokens":{input},"cache_creation_input_tokens":{create},"cache_read_input_tokens":{read},"output_tokens":999}}}}}}"#
            )
        };
        std::fs::write(
            &f,
            [
                line(5, 100, 1000),
                r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
                line(6, 1_200, 82_000),
                r#"{"type":"system","subtype":"stop"}"#.to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        assert_eq!(context_in_transcript(&f), Some(83_206));
        // A transcript with no usage (or none at all) says nothing.
        std::fs::write(&f, r#"{"type":"assistant","message":{"content":[]}}"#).unwrap();
        assert_eq!(context_in_transcript(&f), None);
        assert_eq!(
            context_in_transcript(&std::env::temp_dir().join("no-such-file")),
            None
        );
        // A big turn after the usage line (a huge tool result) pushes it past the first 64 KB.
        let big = format!(
            r#"{{"type":"user","message":{{"content":"{}"}}}}"#,
            "x".repeat(100_000)
        );
        std::fs::write(&f, [line(1, 2, 3), big].join("\n")).unwrap();
        assert_eq!(context_in_transcript(&f), Some(6));
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
