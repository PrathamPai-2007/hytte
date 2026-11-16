//! `hytte mcp`: a Model Context Protocol server on stdio, so an agent can put a card on the
//! pill, ask you a question with buttons, or add a note. Every tool is one of the commands the
//! CLI already has (`hytte_proto::Cmd`); nothing here reaches the daemon any other way.
//! Newline-delimited JSON-RPC 2.0, stdout carries protocol only. Requests are answered one at a
//! time, so a `card` that waits holds up the next call (an agent waits for its own answer anyway).

use hytte_proto::Cmd;
use serde_json::{json, Value};

/// What the tools need from the outside: sending to the pill, and waiting for a press.
pub(super) trait Backend {
    /// False when the pill isn't running.
    fn send(&self, c: &Cmd) -> bool;
    /// Send a waited-on card; the pressed button's label, or None (no pill, no press).
    fn ask(&self, c: &Cmd) -> Option<String>;
    /// The pane names: `[[script_pane]]`s in config.toml and approved packs.
    fn panes(&self) -> Vec<String>;
}

const LATEST: &str = "2025-06-18";

pub(super) fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    json!([
        {"name": "notify",
         "description": "Show a short message on the user's screen (the Hytte pill) for a few seconds. Use for FYIs, not questions.",
         "inputSchema": {"type": "object", "required": ["text"],
            "properties": {"text": s("The message, up to 1024 bytes")}}},
        {"name": "card",
         "description": "A card on the pill. With wait=true and 1-2 buttons it asks the user and returns the label of the button they press (or says there was no answer). Without wait it just shows, and a later call with the same id updates it (progress, done, clear).",
         "inputSchema": {"type": "object", "properties": {
            "title": s("Bold first line, up to 128 bytes"),
            "body": s("Text under it, up to 1024 bytes"),
            "id": s("Send again with the same id to update the card in place"),
            "icon": {"type": "string", "enum": hytte_proto::CARD_ICONS},
            "buttons": {"type": "array", "items": {"type": "string"}, "maxItems": 2,
                "description": "Up to two labels of 24 bytes. A press is reported back; only a Label:name button runs a named [[action]]"},
            "wait": {"type": "boolean", "description": "Block until a button is pressed (needs buttons)"},
            "ttl": {"type": "integer", "minimum": 1, "maximum": 3600,
                "description": "Seconds the card stays; with wait the default is 60"},
            "progress": {"type": "integer", "minimum": 0, "maximum": 100},
            "done": {"type": "boolean", "description": "The job behind a progress card finished"},
            "clear": {"type": "boolean", "description": "Take the card with this id away"},
            "open": s("An http(s) link or an absolute path, offered as an Open button")}}},
        {"name": "note_add",
         "description": "Add a note to the user's Notes pane (it leaves after 24 hours unless keep=true).",
         "inputSchema": {"type": "object", "required": ["text"], "properties": {
            "text": s("Up to 2000 characters"),
            "keep": {"type": "boolean", "description": "Keep it past 24 hours"}}}},
        {"name": "todo_add",
         "description": "Add an item to the user's To-do pane.",
         "inputSchema": {"type": "object", "required": ["text"],
            "properties": {"text": s("Up to 2000 characters")}}},
        {"name": "pane_set",
         "description": "Set the text of one of the user's [[script_pane]]s (a pane name from their config.toml, or an approved pane pack).",
         "inputSchema": {"type": "object", "required": ["name", "text"], "properties": {
            "name": s("The pane's name"),
            "text": s("Plain text, or JSON {\"rows\": [...]}, up to 4096 bytes")}}}
    ])
}

fn text_of(a: &Value, k: &str) -> Result<Option<String>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{k} must be a string")),
    }
}

fn flag(a: &Value, k: &str) -> Result<bool, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{k} must be true or false")),
    }
}

fn number(a: &Value, k: &str, max: u64) -> Result<Option<u64>, String> {
    match a.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= max)
            .map(Some)
            .ok_or_else(|| format!("{k} must be a whole number up to {max}")),
    }
}

fn card(a: &Value) -> Result<Cmd, String> {
    let mut c = Cmd::card();
    c.title = text_of(a, "title")?;
    c.text = text_of(a, "body")?;
    c.id = text_of(a, "id")?;
    c.icon = text_of(a, "icon")?;
    c.open = text_of(a, "open")?;
    c.progress = number(a, "progress", 100)?.map(|n| n as u8);
    c.ttl = number(a, "ttl", hytte_proto::CARD_MAX_TTL_SECS as u64)?.map(|n| n as u32);
    c.done = flag(a, "done")?;
    c.clear = flag(a, "clear")?;
    c.wait = flag(a, "wait")?;
    match a.get("buttons") {
        None | Some(Value::Null) => {}
        Some(Value::Array(b)) => {
            for x in b {
                c.buttons
                    .push(x.as_str().ok_or("buttons must be strings")?.to_string());
            }
        }
        Some(_) => return Err("buttons must be a list of strings".into()),
    }
    if c.wait && c.ttl.is_none() {
        c.ttl = Some(super::CARD_WAIT_TTL);
    }
    if c.validate() {
        Ok(c)
    } else {
        Err("not a valid card: it needs a title or body (or an id to update one); clear needs an id; wait needs a button; at most 2 buttons of 24 bytes; title 128, body 1024 bytes; open is an http(s) link or a full path".into())
    }
}

fn jot_arg(a: &Value) -> Result<String, String> {
    let t = text_of(a, "text")?.unwrap_or_default();
    // Same trim and 2000-character cut as `hytte note`.
    super::jot_text(&[t], None).ok_or_else(|| "text is empty".into())
}

/// Run one tool. Ok is the text for the model; Err is a tool error (the model sees it and can
/// recover), not a protocol error.
fn call(name: &str, a: &Value, io: &dyn Backend) -> Result<String, String> {
    let sent = |c: Cmd, what: &str| {
        if io.send(&c) {
            Ok(what.to_string())
        } else {
            Err("Hytte isn't running, so nothing was shown".to_string())
        }
    };
    match name {
        "notify" => {
            let mut t = text_of(a, "text")?.unwrap_or_default();
            if t.trim().is_empty() {
                return Err("text is empty".into());
            }
            hytte_proto::cut(&mut t, 1024);
            sent(Cmd::notify(t), "shown")
        }
        "card" => {
            let c = card(a)?;
            if !c.wait {
                return sent(c, "shown");
            }
            Ok(io.ask(&c).unwrap_or_else(|| {
                "no answer: nothing was pressed (or Hytte isn't running)".into()
            }))
        }
        "note_add" => sent(Cmd::note_add(jot_arg(a)?, flag(a, "keep")?), "added"),
        "todo_add" => sent(Cmd::todo_add(jot_arg(a)?), "added"),
        "pane_set" => {
            let n = text_of(a, "name")?.unwrap_or_default();
            let mut t = text_of(a, "text")?.unwrap_or_default();
            let panes = io.panes();
            if !panes.iter().any(|p| p.eq_ignore_ascii_case(n.trim())) {
                return Err(format!(
                    "no pane called \"{n}\" ([[script_pane]] or an approved pack){}",
                    if panes.is_empty() {
                        String::new()
                    } else {
                        format!(" (there is: {})", panes.join(", "))
                    }
                ));
            }
            hytte_proto::cut(&mut t, hytte_proto::MAX_PANE_TEXT_BYTES);
            sent(Cmd::pane_set(n.trim(), t), "set")
        }
        _ => Err(format!("unknown tool {name}")),
    }
}

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

/// One JSON-RPC message in, the response out (None for a notification).
pub(super) fn handle(req: &Value, io: &dyn Backend) -> Option<Value> {
    let id = req.get("id")?;
    let params = req.get("params").unwrap_or(&Value::Null);
    Some(match req.get("method").and_then(Value::as_str) {
        Some("initialize") => {
            let v = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(LATEST);
            reply(
                id,
                json!({"protocolVersion": v, "capabilities": {"tools": {}},
                    "serverInfo": {"name": "hytte", "version": env!("CARGO_PKG_VERSION")}}),
            )
        }
        Some("ping") => reply(id, json!({})),
        Some("tools/list") => reply(id, json!({"tools": tools()})),
        Some("tools/call") => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").unwrap_or(&Value::Null);
            let (text, is_error) = match call(name, args, io) {
                Ok(t) => (t, false),
                Err(e) => (e, true),
            };
            reply(
                id,
                json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
            )
        }
        _ => error(id, -32601, "method not found"),
    })
}

/// Read requests from stdin until it closes.
pub(super) fn run(io: &dyn Backend) {
    use std::io::{BufRead, Write};
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        // PowerShell 5 pipes a byte-order mark ahead of the first line.
        let line = line.trim_start_matches('\u{feff}').to_string();
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(&req, io),
            Err(_) => Some(error(&Value::Null, -32700, "parse error")),
        };
        if let Some(r) = resp {
            if writeln!(out, "{r}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        sent: RefCell<Vec<Cmd>>,
        up: bool,
        press: Option<String>,
    }
    impl Backend for Fake {
        fn send(&self, c: &Cmd) -> bool {
            self.sent.borrow_mut().push(c.clone());
            self.up
        }
        fn ask(&self, c: &Cmd) -> Option<String> {
            self.sent.borrow_mut().push(c.clone());
            self.press.clone()
        }
        fn panes(&self) -> Vec<String> {
            vec!["Build".into()]
        }
    }
    fn rpc(io: &Fake, m: &str, p: Value) -> Value {
        handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": m, "params": p}),
            io,
        )
        .unwrap()
    }
    fn tool(io: &Fake, name: &str, args: Value) -> (String, bool) {
        let r = rpc(io, "tools/call", json!({"name": name, "arguments": args}));
        let c = &r["result"];
        (
            c["content"][0]["text"].as_str().unwrap().to_string(),
            c["isError"].as_bool().unwrap(),
        )
    }

    #[test]
    fn handshake_lists_the_five_tools_and_ignores_notifications() {
        let io = Fake::default();
        let r = rpc(&io, "initialize", json!({"protocolVersion": "2024-11-05"}));
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(r["result"]["serverInfo"]["name"], "hytte");
        let names: Vec<_> = rpc(&io, "tools/list", json!({}))["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            ["notify", "card", "note_add", "todo_add", "pane_set"]
        );
        assert!(handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &io
        )
        .is_none());
        assert_eq!(rpc(&io, "nope", json!({}))["error"]["code"], -32601);
    }

    #[test]
    fn a_card_that_waits_returns_the_pressed_label() {
        let io = Fake {
            up: true,
            press: Some("Restart".into()),
            ..Default::default()
        };
        let a = json!({"title": "Restart?", "buttons": ["Restart", "Later"], "wait": true});
        assert_eq!(tool(&io, "card", a.clone()), ("Restart".into(), false));
        assert_eq!(
            io.sent.borrow()[0].ttl,
            Some(60),
            "a wait gets the default minute"
        );
        // Nobody pressed (or no pill): a plain answer the agent can act on, not a hang.
        let none = Fake::default();
        let (t, err) = tool(&none, "card", a);
        assert!(t.starts_with("no answer") && !err);
    }

    #[test]
    fn bad_input_is_a_tool_error_and_nothing_is_sent() {
        let io = Fake {
            up: true,
            ..Default::default()
        };
        for (n, a) in [
            ("card", json!({})),
            ("card", json!({"title": "t", "wait": true})),
            ("card", json!({"title": "t", "buttons": ["a", "b", "c"]})),
            ("card", json!({"title": "t", "progress": 101})),
            ("card", json!({"title": 5})),
            ("notify", json!({"text": "  "})),
            ("note_add", json!({"text": ""})),
            ("pane_set", json!({"name": "Nope", "text": "x"})),
            ("nope", json!({})),
        ] {
            let (_, err) = tool(&io, n, a.clone());
            assert!(err, "{n} {a}");
        }
        assert!(io.sent.borrow().is_empty());
    }

    #[test]
    fn notes_todos_and_panes_reach_the_pill() {
        let io = Fake {
            up: true,
            ..Default::default()
        };
        assert!(!tool(&io, "note_add", json!({"text": " hi ", "keep": true})).1);
        assert!(!tool(&io, "todo_add", json!({"text": "ship"})).1);
        assert!(!tool(&io, "pane_set", json!({"name": "build", "text": "ok"})).1);
        assert!(!tool(&io, "notify", json!({"text": "done"})).1);
        let s = io.sent.borrow();
        assert_eq!(
            (s[0].cmd.as_str(), s[0].keep, s[0].text.as_deref()),
            ("note_add", true, Some("hi"))
        );
        assert_eq!(s[1].cmd, "todo_add");
        assert_eq!(
            (s[2].cmd.as_str(), s[2].id.as_deref()),
            ("pane_set", Some("build"))
        );
        assert!(s.iter().all(Cmd::validate));
        // No pill: an error the model can read.
        let down = Fake::default();
        assert!(tool(&down, "notify", json!({"text": "x"})).1);
    }
}
