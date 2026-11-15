//! Connects AI coding agents to the pill: hooks in Claude Code, Codex, Gemini CLI, Antigravity
//! Cursor and Copilot CLI, and a plugin in OpenCode. Every entry runs this exe by its full path, so it works
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
    /// GitHub Copilot CLI: a hooks file of its own, like OpenCode's plugin.
    Copilot,
}

impl Kind {
    /// A file only Hytte writes (made whole, deleted on undo) rather than merged into.
    fn owned(self) -> bool {
        matches!(self, Kind::OpenCode | Kind::Copilot)
    }
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
    home().map(|h| targets_in(h, true)).unwrap_or_default()
}

/// The agents under home folder `h`. `native` is this Windows user's own home: only there do
/// the agents' environment variables and the Cursor app folder apply (a WSL distro has its own).
fn targets_in(h: PathBuf, native: bool) -> Vec<Target> {
    let env_dir = |var: &str, default: PathBuf| {
        std::env::var_os(var)
            .filter(|v| native && !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or(default)
    };
    let claude = env_dir("CLAUDE_CONFIG_DIR", h.join(".claude"));
    let codex = env_dir("CODEX_HOME", h.join(".codex"));
    let opencode = h.join(".config").join("opencode");
    let gemini = h.join(".gemini");
    let cursor = h.join(".cursor");
    let copilot = env_dir("COPILOT_HOME", h.join(".copilot"));
    let cursor_app = native
        && std::env::var_os("APPDATA").is_some_and(|a| Path::new(&a).join("Cursor").is_dir());
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
            copilot.is_dir(),
            "Copilot CLI",
            copilot.join("hooks").join("hytte.json"),
            Kind::Copilot,
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

/// Copilot CLI's hooks file. Only informational hooks (a prompt, a tool ran, the turn ended), so
/// its own prompts stay in Copilot. `command` is for bash, `powershell` for the Windows shell.
fn copilot_file(exe: &str) -> String {
    let h = |sub: &str| {
        let cmd = command(exe, &format!("{sub} --ack"), "Copilot");
        let exe_q = if exe.starts_with('"') {
            exe.to_string()
        } else {
            format!("\"{exe}\"")
        };
        let ps = format!("& {}", cmd.replacen(exe, &exe_q, 1));
        json!([{ "type": "command", "command": cmd, "powershell": ps, "timeoutSec": 5 }])
    };
    let v = json!({ "version": 1, "hooks": {
        "userPromptSubmitted": h("resume"),
        "postToolUse": h("resume"),
        "agentStop": h("done")
    }});
    let mut s = serde_json::to_string_pretty(&v).unwrap_or_default();
    s.push('\n');
    s
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
    if kind.owned() {
        let want = if kind == Kind::Copilot {
            copilot_file(exe)
        } else {
            PLUGIN.replace("__HYTTE_EXE__", &exe.replace('"', ""))
        };
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
        if let (Some(old), false) = (&text, t.kind.owned()) {
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
    crate::setup::stable_exe().map(|p| exe_cmd(&p))
}

/// Connect (or with `undo`, disconnect) every installed agent; one line per agent.
pub fn connect(undo: bool) -> Vec<String> {
    let Some(exe) = this_exe() else {
        return vec!["couldn't find hytte.exe's own path".into()];
    };
    let ts = targets();
    if ts.is_empty() {
        return vec!["No supported agent found (Claude Code, Codex, OpenCode, Gemini CLI, Antigravity, Copilot CLI, Cursor).".into()];
    }
    let mut out: Vec<String> = ts.iter().map(|t| apply(t, &exe, undo)).collect();
    if !undo {
        out.push("Restart agents that are already running to pick it up.".into());
    }
    out
}

/// `C:\\Apps\\hytte.exe` as WSL sees it: `/mnt/c/Apps/hytte.exe`, quoted when it has a space.
/// None for a path that isn't on a drive letter.
fn wsl_exe(exe: &Path) -> Option<String> {
    let s = exe.display().to_string().replace('\\', "/");
    let (drive, rest) = s.split_once(":/")?;
    (drive.len() == 1).then(|| {
        exe_cmd(Path::new(&format!(
            "/mnt/{}/{rest}",
            drive.to_ascii_lowercase()
        )))
    })
}

/// The installed WSL distros (not Docker Desktop's two helpers). `wsl -l -q` prints UTF-16.
#[cfg(windows)]
fn wsl_distros() -> Vec<String> {
    let Ok(o) = std::process::Command::new("wsl.exe")
        .args(["-l", "-q"])
        .output()
    else {
        return vec![];
    };
    let u: Vec<u16> = o
        .stdout
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&u)
        .lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}').to_string())
        .filter(|l| !l.is_empty() && !l.starts_with("docker-desktop"))
        .collect()
}

/// A distro's home folder as a Windows path (`\\wsl.localhost\Ubuntu\home\me`). Starts the distro.
#[cfg(windows)]
fn wsl_home(distro: &str) -> Option<PathBuf> {
    let o = std::process::Command::new("wsl.exe")
        .args(["-d", distro, "-e", "sh", "-c", "echo $HOME"])
        .output()
        .ok()?;
    let home = String::from_utf8_lossy(&o.stdout).trim().to_string();
    home.starts_with('/').then(|| {
        PathBuf::from(format!(
            r"\\wsl.localhost\{distro}{}",
            home.replace('/', "\\")
        ))
    })
}

/// `hytte setup agents --wsl [--undo]`: the same hooks in each WSL distro's agent config, run
/// as this Windows exe through WSL's interop (`/mnt/c/...`). A hook there reports a Windows
/// process (the session's `wsl.exe`), so liveness and click-to-focus work as for any terminal.
#[cfg(windows)]
pub fn connect_wsl(undo: bool) -> Vec<String> {
    let Some(exe) = crate::setup::stable_exe().as_deref().and_then(wsl_exe) else {
        return vec!["WSL: hytte.exe must be on a drive letter (C:\\...) for WSL to run it".into()];
    };
    let distros = wsl_distros();
    if distros.is_empty() {
        return vec!["WSL: no distro installed".into()];
    }
    let mut out = vec![];
    for d in distros {
        let Some(h) = wsl_home(&d) else {
            out.push(format!("{d}: couldn't reach its home folder"));
            continue;
        };
        let ts = targets_in(h, false);
        if ts.is_empty() {
            out.push(format!("{d}: no supported agent found"));
        }
        out.extend(ts.iter().map(|t| format!("{d} / {}", apply(t, &exe, undo))));
    }
    if !undo {
        out.push("Restart agents that are already running to pick it up.".into());
    }
    out
}

/// How an installed agent's config stands, for `hytte doctor`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Connected,
    NotConnected,
    /// Our hooks are there, but every one points at a hytte.exe that no longer exists.
    Stale,
    /// Its settings file isn't plain JSON (comments, a trailing comma), so Hytte leaves it alone.
    Unreadable,
}

/// An agent's state from its settings file's text (`None`: no file).
fn state_of(kind: Kind, text: Option<&str>) -> State {
    let text = text.unwrap_or("");
    let exes = our_exes(text);
    let unreadable = kind != Kind::OpenCode
        && kind != Kind::Copilot
        && !text.trim().is_empty()
        && serde_json::from_str::<Value>(text.trim()).is_err();
    if exes.is_empty() && unreadable {
        State::Unreadable
    } else if exes.is_empty() {
        State::NotConnected
    } else if exes.iter().all(|e| !Path::new(e).is_file()) {
        State::Stale
    } else {
        State::Connected
    }
}

/// Whether the `hytte` MCP server (`hytte setup mcp`) is registered with an agent.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mcp {
    Registered,
    /// Registered, but its command is a hytte.exe that no longer exists.
    Stale,
    NotRegistered,
}

/// The command a `hytte` MCP server entry runs, from Claude Code's `.claude.json` (user scope)
/// or Codex's `config.toml`. None when there is no such entry.
fn mcp_command(claude: bool, text: &str) -> Option<String> {
    if claude {
        let v: Value = serde_json::from_str(text).ok()?;
        v["mcpServers"]["hytte"]["command"]
            .as_str()
            .map(String::from)
    } else {
        let v: toml::Value = toml::from_str(text).ok()?;
        v.get("mcp_servers")?
            .get("hytte")?
            .get("command")?
            .as_str()
            .map(String::from)
    }
}

fn mcp_state(claude: bool, text: &str) -> Mcp {
    match mcp_command(claude, text) {
        None => Mcp::NotRegistered,
        Some(c) if Path::new(&c).is_file() => Mcp::Registered,
        Some(_) => Mcp::Stale,
    }
}

/// The MCP registration of each installed agent that can have one (Claude Code, Codex).
pub fn mcp_status() -> Vec<(&'static str, Mcp)> {
    let Some(h) = home() else {
        return vec![];
    };
    let dir = |var: &str, default: PathBuf| {
        std::env::var_os(var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or(default)
    };
    let claude = dir("CLAUDE_CONFIG_DIR", h.join(".claude"));
    // Claude Code keeps user-scope servers in `.claude.json`: in the config dir when that is
    // set, else beside it in the home folder.
    let claude_json = if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty()) {
        claude.join(".claude.json")
    } else {
        h.join(".claude.json")
    };
    let codex = dir("CODEX_HOME", h.join(".codex"));
    let read = |p: PathBuf| std::fs::read_to_string(p).unwrap_or_default();
    let mut v = vec![];
    if claude.is_dir() {
        v.push(("Claude Code", mcp_state(true, &read(claude_json))));
    }
    if codex.is_dir() {
        v.push(("Codex", mcp_state(false, &read(codex.join("config.toml")))));
    }
    v
}

/// Each installed agent and its state.
pub fn status() -> Vec<(&'static str, State)> {
    targets()
        .iter()
        .map(|t| {
            let text = std::fs::read_to_string(&t.path).ok();
            (t.name, state_of(t.kind, text.as_deref()))
        })
        .collect()
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
    fn copilot_hooks_file_is_written_whole_and_removed() {
        let exe = "\"C:/My Apps/hytte.exe\"";
        let a = next(Kind::Copilot, None, exe, false).unwrap().unwrap();
        let v = parse(&a);
        assert_eq!(v["version"], 1);
        let e = &v["hooks"]["agentStop"][0];
        assert!(e["command"]
            .as_str()
            .unwrap()
            .ends_with("done --ack --name Copilot"));
        // PowerShell needs the call operator before a quoted path.
        assert!(e["powershell"]
            .as_str()
            .unwrap()
            .starts_with("& \"C:/My Apps/hytte.exe\" agent"));
        assert!(
            v["hooks"]["userPromptSubmitted"].is_array() && v["hooks"]["postToolUse"].is_array()
        );
        assert!(our_exes(&a).iter().any(|p| p == "C:/My Apps/hytte.exe"));
        assert_eq!(next(Kind::Copilot, Some(&a), exe, false).unwrap(), None);
        assert_eq!(
            next(Kind::Copilot, Some(&a), exe, true).unwrap().as_deref(),
            Some("")
        );
    }

    #[test]
    fn a_windows_exe_is_reached_through_mnt() {
        assert_eq!(
            wsl_exe(Path::new(r"C:\Users\me\hytte.exe")).as_deref(),
            Some("/mnt/c/Users/me/hytte.exe")
        );
        assert_eq!(
            wsl_exe(Path::new(r"D:\My Apps\hytte.exe")).as_deref(),
            Some("\"/mnt/d/My Apps/hytte.exe\"")
        );
        assert_eq!(wsl_exe(Path::new(r"\\server\share\hytte.exe")), None);
    }

    #[test]
    fn mcp_registration_is_read_from_each_agents_file() {
        let me = std::env::current_exe()
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        let claude = format!(r#"{{"mcpServers":{{"hytte":{{"command":"{me}","args":["mcp"]}}}}}}"#);
        assert_eq!(mcp_state(true, &claude), Mcp::Registered);
        assert_eq!(
            mcp_state(true, r#"{"mcpServers":{"other":{}}}"#),
            Mcp::NotRegistered
        );
        assert_eq!(mcp_state(true, ""), Mcp::NotRegistered);
        assert_eq!(
            mcp_state(
                true,
                r#"{"mcpServers":{"hytte":{"command":"C:/gone/hytte.exe"}}}"#
            ),
            Mcp::Stale
        );
        let codex = format!("[mcp_servers.hytte]\ncommand = '{me}'\nargs = [\"mcp\"]\n");
        assert_eq!(mcp_state(false, &codex), Mcp::Registered);
        assert_eq!(mcp_state(false, "model = \"x\"\n"), Mcp::NotRegistered);
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

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn a_settings_file_that_isnt_plain_json_is_unreadable_not_just_unconnected() {
        let commented = "{\n  // my hooks\n  \"hooks\": {}\n}";
        assert_eq!(state_of(Kind::Gemini, Some(commented)), State::Unreadable);
        assert_eq!(state_of(Kind::Claude, Some("{ nope")), State::Unreadable);
        // Missing, empty or plain: simply not connected. OpenCode's plugin is JavaScript.
        assert_eq!(state_of(Kind::Claude, None), State::NotConnected);
        assert_eq!(state_of(Kind::Claude, Some("  ")), State::NotConnected);
        assert_eq!(state_of(Kind::Claude, Some("{}")), State::NotConnected);
        assert_eq!(
            state_of(Kind::OpenCode, Some("export const X = 1; // js")),
            State::NotConnected
        );
        // Our hooks pointing at an exe that is gone.
        let ours = r#"{"hooks":{"Stop":[{"hooks":[{"command":"Z:/gone/hytte.exe agent done --name X"}]}]}}"#;
        assert_eq!(state_of(Kind::Claude, Some(ours)), State::Stale);
    }
}
