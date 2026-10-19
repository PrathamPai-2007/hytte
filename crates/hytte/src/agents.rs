//! Connects AI coding agents to the pill: hooks in Claude Code, Codex, Gemini CLI, Antigravity
//! and Cursor, and a plugin in OpenCode. Every entry runs this exe by its full path, so it works
//! without PATH. Idempotent: Hytte's own entries (a command that runs `hytte… agent`) are replaced
//! and the rest of each file is kept as it was; the first change to a file leaves a
//! `.hytte-backup` copy beside it. Used by the tray item and `hytte setup agents [--undo]`.
#![cfg_attr(not(windows), allow(dead_code))]

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const PLUGIN: &str = include_str!("agent_plugin.js");

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Claude,
    Codex,
    Gemini,
    Antigravity,
    Cursor,
    OpenCode,
}

struct Target {
    name: &'static str,
    path: PathBuf,
    kind: Kind,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// The agents installed for this user, found by their config folders.
fn targets() -> Vec<Target> {
    let Some(h) = home() else {
        return vec![];
    };
    let env_dir = |var: &str, default: PathBuf| {
        std::env::var_os(var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or(default)
    };
    let claude = env_dir("CLAUDE_CONFIG_DIR", h.join(".claude"));
    let codex = env_dir("CODEX_HOME", h.join(".codex"));
    let opencode = h.join(".config").join("opencode");
    let gemini = h.join(".gemini");
    let cursor = h.join(".cursor");
    let cursor_app =
        std::env::var_os("APPDATA").is_some_and(|a| Path::new(&a).join("Cursor").is_dir());
    [
        (
            claude.is_dir(),
            "Claude Code",
            claude.join("settings.json"),
            Kind::Claude,
        ),
        (
            codex.is_dir(),
            "Codex",
            codex.join("hooks.json"),
            Kind::Codex,
        ),
        (
            opencode.is_dir(),
            "OpenCode",
            opencode.join("plugins").join("hytte.js"),
            Kind::OpenCode,
        ),
        (
            gemini.join("settings.json").is_file(),
            "Gemini CLI",
            gemini.join("settings.json"),
            Kind::Gemini,
        ),
        // The CLI (`agy`), the IDE and Antigravity 2.0 share this file.
        (
            gemini.join("antigravity-cli").is_dir() || gemini.join("config").is_dir(),
            "Antigravity",
            gemini.join("config").join("hooks.json"),
            Kind::Antigravity,
        ),
        (
            cursor.is_dir() || cursor_app,
            "Cursor",
            cursor.join("hooks.json"),
            Kind::Cursor,
        ),
    ]
    .into_iter()
    .filter(|t| t.0)
    .map(|(_, name, path, kind)| Target { name, path, kind })
    .collect()
}

/// This exe as the start of a hook command. Forward slashes work in cmd, PowerShell and Git Bash
/// alike, which is why the path is only quoted when it has to be.
pub fn exe_cmd(exe: &Path) -> String {
    let s = exe.display().to_string().replace('\\', "/");
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s
    }
}

/// A hook command Hytte wrote (or one a user pasted from `hytte agent hooks`).
fn ours(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    c.contains("hytte") && c.contains(" agent ")
}

fn command(exe: &str, sub: &str, name: &str) -> String {
    let name = if name.contains(' ') {
        format!("\"{name}\"")
    } else {
        name.to_string()
    };
    format!("{exe} agent {sub} --name {name}")
}

/// Claude Code's hooks (Codex reads the same format, without `Notification`): what it is doing,
/// when it waits for you, and its permission prompts answered on the pill. With no answer there,
/// the agent asks in the terminal as usual.
pub fn claude_hooks(exe: &str, name: &str) -> Value {
    let c = |sub: &str| json!({ "type": "command", "command": command(exe, sub, name) });
    let mut ask = c("ask");
    // Above the pill's own wait, so the agent never gives up on the hook first.
    ask["timeout"] = json!(hytte_proto::ASK_WAIT_SECS + 30);
    let mut v = json!({
        "UserPromptSubmit": [{ "hooks": [c("resume")] }],
        "PreToolUse": [{ "matcher": "*", "hooks": [c("tool")] }],
        "PermissionRequest": [{ "matcher": "*", "hooks": [ask] }],
        "Stop": [{ "hooks": [c("done")] }]
    });
    if name == "Claude Code" {
        v["Notification"] = json!([{ "matcher": "permission_prompt|elicitation_dialog", "hooks": [c("needs-input")] }]);
    }
    v
}

/// The hooks for the agents that read a hook's stdout as JSON: `--ack` prints `{}`.
fn json_hooks(kind: Kind, exe: &str) -> Value {
    let c = |sub: &str, name: &str| command(exe, &format!("{sub} --ack"), name);
    match kind {
        Kind::Gemini => {
            let h = |sub: &str| json!([{ "hooks": [{ "type": "command", "command": c(sub, "Gemini") }] }]);
            json!({ "BeforeAgent": h("resume"), "Notification": h("needs-input"), "AfterAgent": h("done") })
        }
        // Only hooks whose output is informational: Cursor blocks the action on any reply its
        // gating hooks don't expect, so prompts stay in Cursor.
        Kind::Cursor => {
            let h = |sub: &str| json!([{ "command": c(sub, "Cursor") }]);
            json!({ "postToolUse": h("resume"), "stop": h("done") })
        }
        // Antigravity's named hook: a model call means it is working, Stop that the turn ended.
        _ => {
            let h = |sub: &str| json!([{ "type": "command", "command": c(sub, "Antigravity"), "timeout": 5 }]);
            json!({ "PreInvocation": h("resume"), "Stop": h("done") })
        }
    }
}

/// `hooks` (event -> matcher groups or handlers) without Hytte's handlers; whatever is left empty goes.
fn strip(hooks: &mut Map<String, Value>) {
    let is_ours = |h: &Value| h.get("command").and_then(Value::as_str).is_some_and(ours);
    for groups in hooks.values_mut() {
        let Some(gs) = groups.as_array_mut() else {
            continue;
        };
        gs.retain_mut(|g| {
            if is_ours(g) {
                return false;
            }
            let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let before = hs.len();
            hs.retain(|h| !is_ours(h));
            hs.len() == before || !hs.is_empty()
        });
    }
    hooks.retain(|_, g| !g.as_array().is_some_and(Vec::is_empty));
}

/// The new file text for `kind`, or `None` when nothing changes. An empty string means delete it.
fn next(kind: Kind, text: Option<&str>, exe: &str, undo: bool) -> Result<Option<String>, String> {
    if kind == Kind::OpenCode {
        let want = PLUGIN.replace("__HYTTE_EXE__", &exe.replace('"', ""));
        return Ok(match (text, undo) {
            (Some(_), true) => Some(String::new()),
            (None, true) => None,
            (Some(t), false) if t == want => None,
            _ => Some(want),
        });
    }
    let mut v: Value = match text.map(str::trim) {
        None | Some("") => json!({}),
        Some(t) => serde_json::from_str(t)
            .map_err(|_| "isn't plain JSON, so it was left alone".to_string())?,
    };
    let Some(root) = v.as_object_mut() else {
        return Err("isn't a JSON object, so it was left alone".into());
    };
    let before = root.clone();
    if kind == Kind::Antigravity {
        root.remove("hytte");
        if !undo {
            root.insert("hytte".into(), json_hooks(kind, exe));
        }
    } else {
        if kind == Kind::Cursor && !undo && !root.contains_key("version") {
            root.insert("version".into(), json!(1));
        }
        // Edited where it is, so the file keeps its order.
        let slot = root.entry("hooks").or_insert_with(|| json!({}));
        if !slot.is_object() {
            *slot = json!({});
        }
        let Some(hooks) = slot.as_object_mut() else {
            unreachable!()
        };
        strip(hooks);
        if !undo {
            let add = match kind {
                Kind::Claude => claude_hooks(exe, "Claude Code"),
                Kind::Codex => claude_hooks(exe, "Codex"),
                _ => json_hooks(kind, exe),
            };
            for (event, groups) in add.as_object().into_iter().flatten() {
                let list = hooks.entry(event.clone()).or_insert_with(|| json!([]));
                if let (Some(l), Some(g)) = (list.as_array_mut(), groups.as_array()) {
                    l.extend(g.iter().cloned());
                }
            }
        }
        if hooks.is_empty() {
            root.shift_remove("hooks");
        }
    }
    if *root == before {
        return Ok(None);
    }
    let bare = root.is_empty()
        || (kind == Kind::Cursor && root.len() == 1 && root.contains_key("version"));
    if undo && bare && text.is_some() && kind != Kind::Claude && kind != Kind::Gemini {
        // A hooks file Hytte created and nothing else uses.
        return Ok(Some(String::new()));
    }
    let mut s = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    s.push('\n');
    Ok(Some(s))
}

/// The exe paths Hytte's entries in `text` point at: each `….exe` on a line that mentions Hytte,
/// back to the quote that opens it.
// ponytail: a text scan, not a parse; enough to tell whether the recorded exe still exists.
fn our_exes(text: &str) -> Vec<String> {
    let mut out = vec![];
    for l in text
        .lines()
        .filter(|l| l.to_ascii_lowercase().contains("hytte"))
    {
        let low = l.to_ascii_lowercase();
        let mut from = 0;
        while let Some(i) = low[from..].find(".exe") {
            let end = from + i + 4;
            let start = l[..end].rfind('"').map_or(0, |q| q + 1);
            out.push(l[start..end].trim().to_string());
            from = end;
        }
    }
    out
}

/// Apply one target; returns a line for the user.
fn apply(t: &Target, exe: &str, undo: bool) -> String {
    let shown = t.path.display();
    let text = match std::fs::read_to_string(&t.path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return format!("{}: couldn't read {shown}: {e}", t.name),
    };
    let new = match next(t.kind, text.as_deref(), exe, undo) {
        Ok(Some(n)) => n,
        Ok(None) if undo => return format!("{}: not connected", t.name),
        Ok(None) => return format!("{}: already connected", t.name),
        Err(e) => return format!("{}: {shown} {e}", t.name),
    };
    let r = if new.is_empty() {
        std::fs::remove_file(&t.path)
    } else {
        // Only the user's own file, as it was before Hytte first touched it.
        if let (Some(old), false) = (&text, t.kind == Kind::OpenCode) {
            let mut b = t.path.as_os_str().to_owned();
            b.push(".hytte-backup");
            if !undo && our_exes(old).is_empty() && !Path::new(&b).exists() {
                let _ = std::fs::write(&b, old);
            }
        }
        crate::setup::write_file(&t.path, new.as_bytes())
    };
    match r {
        Ok(()) if undo => format!("{}: disconnected", t.name),
        Ok(()) if t.kind == Kind::Codex => {
            format!(
                "{}: connected ({shown}); approve the hooks once with /hooks in Codex",
                t.name
            )
        }
        Ok(()) => format!("{}: connected ({shown})", t.name),
        Err(e) => format!("{}: couldn't write {shown}: {e}", t.name),
    }
}

fn this_exe() -> Option<String> {
    std::env::current_exe().ok().map(|p| exe_cmd(&p))
}

/// Connect (or with `undo`, disconnect) every installed agent; one line per agent.
pub fn connect(undo: bool) -> Vec<String> {
    let Some(exe) = this_exe() else {
        return vec!["couldn't find hytte.exe's own path".into()];
    };
    let ts = targets();
    if ts.is_empty() {
        return vec!["No supported agent found (Claude Code, Codex, OpenCode, Gemini CLI, Antigravity, Cursor).".into()];
    }
    let mut out: Vec<String> = ts.iter().map(|t| apply(t, &exe, undo)).collect();
    if !undo {
        out.push("Restart agents that are already running to pick it up.".into());
    }
    out
}

/// Whether any agent is connected.
pub fn is_connected() -> bool {
    targets()
        .iter()
        .any(|t| std::fs::read_to_string(&t.path).is_ok_and(|s| !our_exes(&s).is_empty()))
}

/// At startup: entries that point at an exe that is gone (Hytte was moved) are pointed at this
/// one. Entries that still work are never touched, so a dev build can't take them over.
pub fn repair() -> Vec<String> {
    let Some(exe) = this_exe() else {
        return vec![];
    };
    targets()
        .iter()
        .filter(|t| {
            std::fs::read_to_string(&t.path).is_ok_and(|s| {
                let exes = our_exes(&s);
                !exes.is_empty() && exes.iter().all(|e| !Path::new(e).is_file())
            })
        })
        .map(|t| apply(t, &exe, false))
        .collect()
}

/// The tray item: connect, or offer to disconnect when already connected.
#[cfg(windows)]
pub fn tray() {
    std::thread::spawn(|| {
        let lines = if is_connected() {
            if !crate::setup::message(
                "AI agents are connected to Hytte.\n\nDisconnect them?",
                true,
            ) {
                return;
            }
            connect(true)
        } else {
            connect(false)
        };
        crate::setup::message(&lines.join("\n"), false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "C:/Apps/hytte.exe";

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn claude_merge_keeps_other_hooks_and_is_idempotent() {
        let mine = r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"notify-send done"}]}],
            "Notification":[{"hooks":[{"type":"command","command":"hytte agent needs-input --name \"Claude Code\""}]}]}}"#;
        let a = next(Kind::Claude, Some(mine), EXE, false).unwrap().unwrap();
        let h = &parse(&a)["hooks"];
        // The user's own Stop hook stays, ours goes beside it; the old pasted one is replaced.
        assert_eq!(h["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(h["Stop"][0]["hooks"][0]["command"], "notify-send done");
        assert_eq!(h["Notification"].as_array().unwrap().len(), 1);
        assert_eq!(
            h["PermissionRequest"][0]["hooks"][0]["command"],
            r#"C:/Apps/hytte.exe agent ask --name "Claude Code""#
        );
        assert!(
            h["PermissionRequest"][0]["hooks"][0]["timeout"]
                .as_u64()
                .unwrap()
                > hytte_proto::ASK_WAIT_SECS
        );
        assert!(
            a.find("\"model\"").unwrap() < a.find("\"hooks\"").unwrap(),
            "key order kept"
        );
        let first = r#"{"hooks":{},"model":"opus"}"#;
        let f = next(Kind::Claude, Some(first), EXE, false)
            .unwrap()
            .unwrap();
        assert!(
            f.find("\"hooks\"").unwrap() < f.find("\"model\"").unwrap(),
            "hooks stay in place"
        );
        assert_eq!(next(Kind::Claude, Some(&a), EXE, false).unwrap(), None);
        // Undo leaves exactly the user's hooks.
        let u = next(Kind::Claude, Some(&a), EXE, true).unwrap().unwrap();
        let h = &parse(&u)["hooks"];
        assert_eq!(h.as_object().unwrap().len(), 1);
        assert_eq!(h["Stop"][0]["hooks"][0]["command"], "notify-send done");
        assert_eq!(next(Kind::Claude, Some(&u), EXE, true).unwrap(), None);
    }

    #[test]
    fn a_new_exe_path_replaces_the_old_entries() {
        let a = next(Kind::Gemini, None, "C:/Old/hytte.exe", false)
            .unwrap()
            .unwrap();
        let b = next(Kind::Gemini, Some(&a), EXE, false).unwrap().unwrap();
        assert!(!b.contains("C:/Old") && b.contains(EXE));
        assert_eq!(
            parse(&b)["hooks"]["AfterAgent"].as_array().unwrap().len(),
            1
        );
        assert_eq!(our_exes(&a), vec!["C:/Old/hytte.exe"; 3]);
    }

    #[test]
    fn codex_and_cursor_files_come_and_go() {
        let c = next(Kind::Codex, None, EXE, false).unwrap().unwrap();
        let v = parse(&c);
        assert!(v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--name Codex"));
        assert!(v["hooks"].get("Notification").is_none());
        assert_eq!(
            next(Kind::Codex, Some(&c), EXE, true).unwrap().as_deref(),
            Some("")
        );
        let k = next(Kind::Cursor, None, EXE, false).unwrap().unwrap();
        let v = parse(&k);
        assert_eq!(v["version"], 1);
        assert_eq!(
            v["hooks"]["stop"][0]["command"],
            "C:/Apps/hytte.exe agent done --ack --name Cursor"
        );
        // The user's own Cursor hook keeps the file alive after undo.
        let mine = r#"{"version":1,"hooks":{"stop":[{"command":"./audit.sh"}]}}"#;
        let k = next(Kind::Cursor, Some(mine), EXE, false).unwrap().unwrap();
        let u = next(Kind::Cursor, Some(&k), EXE, true).unwrap().unwrap();
        assert_eq!(parse(&u), parse(mine));
    }

    #[test]
    fn antigravity_gets_one_named_hook() {
        let mine = r#"{"lint":{"PostToolUse":[]}}"#;
        let a = next(Kind::Antigravity, Some(mine), EXE, false)
            .unwrap()
            .unwrap();
        let v = parse(&a);
        assert!(v["lint"].is_object());
        assert!(v["hytte"]["Stop"][0]["command"]
            .as_str()
            .unwrap()
            .contains("done --ack"));
        let u = next(Kind::Antigravity, Some(&a), EXE, true)
            .unwrap()
            .unwrap();
        assert_eq!(parse(&u), parse(mine));
    }

    #[test]
    fn opencode_plugin_is_written_then_removed() {
        let a = next(Kind::OpenCode, None, "\"C:/My Apps/hytte.exe\"", false)
            .unwrap()
            .unwrap();
        assert!(a.contains(r#"const HYTTE = "C:/My Apps/hytte.exe";"#));
        assert_eq!(our_exes(&a), vec!["C:/My Apps/hytte.exe"]);
        assert_eq!(
            next(Kind::OpenCode, Some(&a), "\"C:/My Apps/hytte.exe\"", false).unwrap(),
            None
        );
        assert_eq!(
            next(Kind::OpenCode, Some(&a), EXE, true)
                .unwrap()
                .as_deref(),
            Some("")
        );
    }

    #[test]
    fn quoted_paths_are_found_in_json() {
        let a = next(Kind::Claude, None, "\"C:/My Apps/hytte.exe\"", false)
            .unwrap()
            .unwrap();
        assert!(our_exes(&a).iter().all(|e| e == "C:/My Apps/hytte.exe"));
    }

    #[test]
    fn broken_files_are_left_alone() {
        assert!(next(Kind::Claude, Some("{ // comment\n}"), EXE, false).is_err());
        assert!(next(Kind::Claude, Some("[]"), EXE, false).is_err());
    }

    #[test]
    fn paths_are_quoted_only_with_spaces() {
        assert_eq!(
            exe_cmd(Path::new(r"C:\Users\me\hytte.exe")),
            "C:/Users/me/hytte.exe"
        );
        assert_eq!(
            exe_cmd(Path::new(r"C:\My Apps\hytte.exe")),
            "\"C:/My Apps/hytte.exe\""
        );
    }
}
