//! `hytte <command>` — the CLI half of hytte.exe: a short-lived IPC client.
//! Runs before any daemon state exists (no log, no mutex, no config), so shell hooks stay fast.
//! `hytte run -- <cmd...>` still runs the command even if the daemon is absent.

mod agent;
mod doctor;
mod hook;
mod portscmd;
mod progress;

use crate::setup;
use hytte_proto::{HytteMessage, TaskEvent};
use std::collections::VecDeque;
use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

const TAIL_LINES: usize = 5;
/// At most this often a progress update goes to the pipe.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// The command list. `help` prints it to stdout and exits 0; a mistake prints it to stderr, exit 2.
const USAGE: &str = "\
usage:
  hytte                                  # start the pill (the tray app); no-op if it is running
  hytte run [--no-progress] -- <cmd...>  # run cmd, report it to the pill (progress read from its output)
  hytte set (--progress <0-100> | --done | --fail) --label \"...\" [--task <id>]
  hytte agent <start|needs-input|resume|tool|ask|done|fail|hooks> ...   # AI agent / long-task monitor
  hytte init <pwsh|bash|zsh|nu>          # shell integration: auto-track commands over 3 s
  hytte setup [--undo]                   # put hytte on PATH + add the hook to PowerShell / Git Bash
  hytte setup agents [--undo]            # connect Claude Code, Codex, OpenCode, Gemini, Antigravity, Cursor
  hytte notify \"text\"                    # show a card on the pill (for your own scripts)
  hytte card --title \"...\" [...]          # a live card: progress, buttons, --wait (hytte card --help)
  hytte pane set <name> [--json] [--text \"...\"]   # text for a [[script_pane]] (or piped in); --json: {\"rows\": [...]}
  hytte shelf add <file>... | (text on stdin)   # put files or text on the shelf
  hytte doctor                           # check the setup and say how to fix what's wrong
  hytte ports [--all]                    # list dev-server listeners
  hytte kill :PORT [--force]             # stop whatever listens on PORT
  hytte --version | help";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn task_id() -> String {
    format!("{}-{}", std::process::id(), chrono_stamp())
}

// Cheap timestamp without extra deps.
fn chrono_stamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Best-effort pipe send. Returns false when the message did not get through.
fn send_msg(msg: &HytteMessage) -> bool {
    try_send(msg).is_ok()
}

/// Send one message; the error says whether there is no daemon ([`is_absent`]) or just no luck.
fn try_send(msg: &HytteMessage) -> std::io::Result<()> {
    let line = msg
        .to_line()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    open_pipe(false)?.write_all(line.as_bytes())
}

/// No pipe at all, as opposed to a busy or refused one.
fn is_absent(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

/// Open the daemon's pipe. When every instance is busy for a moment (a burst of shell hooks)
/// retry for up to ~200 ms instead of dropping the message; a missing pipe fails at once.
#[cfg(windows)]
fn open_pipe(read: bool) -> std::io::Result<std::fs::File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    const ERROR_PIPE_BUSY: i32 = 231;
    // FILE_FLAG_WRITE_THROUGH pushes a message out promptly.
    const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
    let mut tries = 0;
    loop {
        match OpenOptions::new()
            .read(read)
            .write(true)
            .custom_flags(if read { 0 } else { FILE_FLAG_WRITE_THROUGH })
            .open(hytte_proto::pipe_name())
        {
            // Never talk to a pipe served by another user's process.
            Ok(f) if !server_is_ours(&f) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "the Hytte pipe is served by another user",
                ))
            }
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && tries < 10 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            r => return r,
        }
    }
}

#[cfg(windows)]
fn server_is_ours(f: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    hytte_proto::sys::pipe_server_is_current_user(windows::Win32::Foundation::HANDLE(
        f.as_raw_handle(),
    ))
}

#[cfg(not(windows))]
fn open_pipe(_read: bool) -> std::io::Result<std::fs::File> {
    Err(std::io::ErrorKind::NotFound.into())
}

/// Send an [`hytte_proto::Ask`] and wait for the human's answer. None = no daemon, no
/// answer in time, or dismissed: the caller falls back to asking in the terminal.
fn ask_pipe(ask: &hytte_proto::Ask) -> Option<bool> {
    let mut line = serde_json::to_string(ask).ok()?;
    line.push('\n');
    let reply = request_reply(&line, hytte_proto::ASK_WAIT_SECS + 2)?;
    serde_json::from_str::<hytte_proto::Answer>(reply.trim())
        .ok()
        .map(|a| a.allow)
}

/// Send one line and wait up to `secs` for the daemon's one-line reply. None = no daemon,
/// no reply in time, or a hang-up without one.
fn request_reply(line: &str, secs: u64) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let mut f = open_pipe(true).ok()?;
    f.write_all(line.as_bytes()).ok()?;
    // The read blocks until the daemon answers or hangs up; cap it from outside.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reply = String::new();
        if BufReader::new(f).read_line(&mut reply).is_ok() {
            let _ = tx.send(reply);
        }
    });
    rx.recv_timeout(Duration::from_secs(secs))
        .ok()
        .filter(|r| !r.trim().is_empty())
}

/// This same exe with no arguments (the pill), started detached; waits briefly for its pipe.
/// Only `hytte run` does this (shell hooks must stay instant and never spawn GUIs).
#[cfg(windows)]
fn start_daemon() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    if crate::spawn_detached(&exe).is_err() {
        return false;
    }
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if open_pipe(false).is_ok() {
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn start_daemon() -> bool {
    false
}

/// An argument as it must be typed in PowerShell to reach the program unchanged.
fn quote_arg(a: &str) -> String {
    let plain = !a.is_empty()
        && a.chars()
            .all(|c| c.is_alphanumeric() || r"_-./\:=+%".contains(c));
    if plain {
        a.into()
    } else {
        format!("'{}'", a.replace('\'', "''"))
    }
}

/// Fit a command line into the protocol's 1024-byte label. A cut one ends in `…`, which the
/// pill reads as "can't be run again".
pub(crate) fn fit_label(s: &str) -> String {
    if s.len() <= 1000 {
        return s.into();
    }
    let mut cut = 1000;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Send a script request (`notify`, `shelf_add`). False when it didn't get through.
fn send_cmd(c: &hytte_proto::Cmd) -> bool {
    c.to_line().is_ok_and(|l| {
        open_pipe(false)
            .and_then(|mut f| f.write_all(l.as_bytes()))
            .is_ok()
    })
}

/// How long `hytte card --wait` leaves its question up unless `--ttl` says otherwise.
const CARD_WAIT_TTL: u32 = 60;

/// Parse `hytte card` flags into a `card` command (validated), or say what is wrong.
fn build_card(args: &[String]) -> Result<hytte_proto::Cmd, String> {
    let mut c = hytte_proto::Cmd::card();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        match a.as_str() {
            "--id" => c.id = Some(val()?),
            "--title" => c.title = Some(val()?),
            "--body" => c.text = Some(val()?),
            "--icon" => c.icon = Some(val()?),
            "--open" => c.open = Some(val()?),
            "--button" => c.buttons.push(val()?),
            "--progress" => {
                c.progress = Some(
                    val()?
                        .parse()
                        .ok()
                        .filter(|p| *p <= 100)
                        .ok_or("--progress takes 0-100")?,
                )
            }
            "--ttl" => {
                c.ttl = Some(
                    val()?
                        .parse()
                        .ok()
                        .filter(|t| (1..=hytte_proto::CARD_MAX_TTL_SECS).contains(t))
                        .ok_or("--ttl takes 1-3600 seconds")?,
                )
            }
            "--done" => c.done = true,
            "--clear" => c.clear = true,
            "--wait" => c.wait = true,
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if c.wait && c.buttons.is_empty() {
        return Err("--wait needs at least one --button".into());
    }
    if c.wait && c.ttl.is_none() {
        c.ttl = Some(CARD_WAIT_TTL);
    }
    if let Some(i) = c.icon.as_deref() {
        if !hytte_proto::CARD_ICONS.contains(&i) {
            return Err(format!(
                "--icon is one of {}",
                hytte_proto::CARD_ICONS.join(", ")
            ));
        }
    }
    if c.validate() {
        Ok(c)
    } else {
        Err(
            "a card needs --title or --body (or --id to update one); --clear needs --id; \
             at most 2 buttons of 24 bytes; title 128, body 1024 bytes; \
             --open is an http(s) link or a full path"
                .into(),
        )
    }
}

/// `hytte card ...`: put a live card on the pill, update or clear it, or (`--wait`) ask a
/// question and print the pressed button. Exit 0 with the label; 1 when nothing was pressed or
/// the pill isn't running; 2 for a mistake in the arguments.
fn cmd_card(args: &[String]) {
    if matches!(
        args.first().map(String::as_str),
        Some("--help" | "-h") | None
    ) {
        println!("{CARD_USAGE}");
        if args.is_empty() {
            std::process::exit(2);
        }
        return;
    }
    let c = match build_card(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hytte card: {e}\n\n{CARD_USAGE}");
            std::process::exit(2);
        }
    };
    if !c.wait {
        if !send_cmd(&c) {
            eprintln!("hytte: daemon not running (card dropped)");
            std::process::exit(1);
        }
        return;
    }
    let Ok(line) = c.to_line() else {
        std::process::exit(2)
    };
    let secs = c.ttl.unwrap_or(CARD_WAIT_TTL) as u64 + hytte_proto::CARD_QUEUE_SECS + 10;
    let pressed = request_reply(&line, secs)
        .and_then(|r| serde_json::from_str::<hytte_proto::Pressed>(r.trim()).ok());
    match pressed {
        Some(p) => println!("{}", p.pressed),
        None => std::process::exit(1),
    }
}

const CARD_USAGE: &str = "\
usage: hytte card [options]
  --title \"...\"      bold first line (128 bytes)
  --body \"...\"       text under it (1024 bytes)
  --id <id>          send again with the same id to update the card in place
  --progress <0-100> a bar; the card stays until --done or 30 s without an update
  --done             the job finished (fills the bar, shows a tick)
  --icon <info|ok|warn|error>
  --button \"...\"     up to two; a press only reports back, it never runs anything
  --open <url|path>  adds an Open button
  --ttl <seconds>    how long it stays (default 10, with --wait 60, at most 3600)
  --wait             block until a button is pressed: prints its label, exit 0;
                     no press (or no pill) exits 1
  --clear            take the card with --id away
examples:
  hytte card --id deploy --title Deploy --body \"Uploading...\" --progress 40
  hytte card --id deploy --done --body Live
  hytte card --title \"Restart server?\" --button Restart --button Later --wait";

/// `hytte pane set <name> [--json] [--text "..."]`, or the text piped in: what a
/// `[[script_pane]]` shows until its next run or the next push. With `--json` the text must be
/// `{"rows": [{"text": "...", "value": "...", "progress": 40, "tone": "good", "action": "name"}]}`;
/// it is checked here so a typo is said to you, not dropped by the pill. Exit 1 when no pane has
/// that name or the pill isn't running.
fn cmd_pane(args: &[String]) {
    use std::io::{IsTerminal, Read};
    let (name, json, text) = match args {
        [set, name, rest @ ..] if set == "set" => {
            let mut json = false;
            let mut text = None;
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--json" => json = true,
                    "--text" => text = Some(it.next().unwrap_or_else(|| usage()).clone()),
                    _ => usage(),
                }
            }
            (name, json, text)
        }
        _ => usage(),
    };
    let text = match text {
        Some(t) => t,
        None => {
            let mut stdin = std::io::stdin();
            if stdin.is_terminal() {
                usage();
            }
            let mut t = String::new();
            let _ = stdin
                .by_ref()
                .take(hytte_proto::MAX_PANE_TEXT_BYTES as u64)
                .read_to_string(&mut t);
            t
        }
    };
    // Read only: unlike `config::load`, never writes a default file from the CLI.
    let text_cfg = std::fs::read_to_string(crate::config::config_path()).unwrap_or_default();
    let cfg = crate::config::parse(&text_cfg).unwrap_or_default();
    let panes = cfg.script_panes();
    if !panes
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(name.trim()))
    {
        let known: Vec<&str> = panes.iter().map(|p| p.name.as_str()).collect();
        eprintln!(
            "hytte pane: no [[script_pane]] called \"{name}\" in config.toml{}",
            if known.is_empty() {
                String::new()
            } else {
                format!(" (there is: {})", known.join(", "))
            }
        );
        std::process::exit(1);
    }
    let mut text = text;
    hytte_proto::cut(&mut text, hytte_proto::MAX_PANE_TEXT_BYTES);
    if json && crate::scriptpane::parse_rows(&text).is_none() {
        eprintln!(
            "hytte pane: --json wants {{\"rows\": [{{\"text\": \"...\", \"value\": \"...\"}}]}} (nothing sent)"
        );
        std::process::exit(1);
    }
    if !send_cmd(&hytte_proto::Cmd::pane_set(name.trim(), text)) {
        eprintln!("hytte: daemon not running (text dropped)");
        std::process::exit(1);
    }
}

fn cmd_notify(args: &[String]) {
    let mut text = args.join(" ");
    if text.trim().is_empty() {
        usage();
    }
    hytte_proto::cut(&mut text, 1024);
    if !send_cmd(&hytte_proto::Cmd::notify(text)) {
        eprintln!("hytte: daemon not running (message dropped)");
        std::process::exit(1);
    }
}

/// `hytte shelf add <file>...`, or text piped in: put them on the pill's shelf.
fn cmd_shelf(args: &[String]) {
    use std::io::{IsTerminal, Read};
    let [add, files @ ..] = args else { usage() };
    if add != "add" {
        usage();
    }
    let mut cmds = vec![];
    if files.is_empty() {
        let mut stdin = std::io::stdin();
        if stdin.is_terminal() {
            usage();
        }
        let mut text = String::new();
        let n = stdin
            .by_ref()
            .take(hytte_proto::MAX_SNIPPET_BYTES as u64 + 1)
            .read_to_string(&mut text)
            .unwrap_or(0);
        if n == 0 || text.trim().is_empty() {
            eprintln!("hytte: nothing to add (no input)");
            std::process::exit(2);
        }
        if text.len() > hytte_proto::MAX_SNIPPET_BYTES {
            eprintln!(
                "hytte: text is over {} KB; put it in a file and add the file",
                hytte_proto::MAX_SNIPPET_BYTES / 1024
            );
            std::process::exit(2);
        }
        cmds.push(hytte_proto::Cmd::shelve_text(text));
    }
    for f in files {
        // Absolute, and without the `\?\` prefix canonicalize adds.
        let path = std::fs::canonicalize(f).map(|p| p.display().to_string());
        let Ok(path) = path else {
            eprintln!("hytte: no such file: {f}");
            std::process::exit(2);
        };
        // canonicalize returns the `\\?\` form; the daemon wants a plain path.
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path).to_string();
        cmds.push(hytte_proto::Cmd::shelve_path(path));
    }
    for c in &cmds {
        if !send_cmd(c) {
            eprintln!("hytte: daemon not running (nothing added)");
            std::process::exit(1);
        }
    }
}

/// Rust's spawn only finds `.exe` on PATH, so `npm` (really `npm.cmd`) fails. For a bare name,
/// look through PATH with PATHEXT; the full path then goes to `Command`, which escapes
/// arguments for batch files itself. Anything else is passed through unchanged.
fn find_program(name: &str) -> std::path::PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    find_in(name, &path, &exts).unwrap_or_else(|| name.into())
}

fn find_in(name: &str, path: &std::ffi::OsStr, pathext: &str) -> Option<std::path::PathBuf> {
    if name.contains(['\\', '/', ':', '.']) {
        return None;
    }
    for dir in std::env::split_paths(path) {
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            let full = dir.join(format!("{name}{ext}"));
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

fn cmd_run(args: &[String]) {
    // Split at `--`.
    let dash = args.iter().position(|a| a == "--");
    let cmd_args: Vec<String> = match dash {
        Some(i) => args[i + 1..].to_vec(),
        None => args.to_vec(),
    };
    let mut track = true;
    for flag in dash.map_or(&[][..], |i| &args[..i]) {
        match flag.as_str() {
            "--no-progress" => track = false,
            _ => usage(),
        }
    }
    if cmd_args.is_empty() {
        usage();
    }
    let id = task_id();
    // Quoted the way PowerShell wants it, since Rerun runs the label through PowerShell.
    let label = cmd_args
        .iter()
        .map(|a| quote_arg(a))
        .collect::<Vec<_>>()
        .join(" ");
    let truncated = fit_label(&label);
    let cwd = std::env::current_dir()
        .ok()
        .map(|d| d.to_string_lossy().into_owned());

    let tag = |mut m: HytteMessage| {
        m.pid = Some(std::process::id());
        m.cwd = cwd.clone();
        m
    };
    let start = tag(HytteMessage::start(id.clone(), truncated.clone()));
    // Only a missing pipe starts a daemon; a busy one is a daemon that is just busy.
    if try_send(&start).is_err_and(|e| is_absent(&e)) && start_daemon() {
        let _ = send_msg(&start);
    }

    let t0 = Instant::now();
    // Spawn child with inherited stdio so the terminal behaves normally,
    // while we capture nothing directly. For the stderr tail we re-run
    // capture only on failure via a second lightweight approach:
    // instead, run with piped stderr + passthrough via tee in-thread.
    // Simplest honest approach: inherit stdout, pipe stderr, forward it.
    let mut child = Command::new(find_program(&cmd_args[0]))
        .args(&cmd_args[1..])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            let mut fail = tag(HytteMessage::start(id.clone(), truncated.clone()));
            fail.event = TaskEvent::Failed;
            fail.duration_ms = Some(t0.elapsed().as_millis() as u64);
            let why = if e.kind() == std::io::ErrorKind::InvalidInput {
                format!("{e} (an argument can't be passed safely to a .cmd/.bat file)")
            } else {
                e.to_string()
            };
            fail.stderr_tail = Some(format!("spawn failed: {why}"));
            let _ = send_msg(&fail);
            eprintln!("hytte: spawn failed: {why}");
            std::process::exit(127);
        });

    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL_LINES + 1);
    if let Some(mut stderr) = child.stderr.take() {
        use std::io::Read;
        // Raw bytes, not lines: `\r` progress bars reach the terminal as they're drawn, and
        // output that isn't UTF-8 is passed through instead of ending the copy.
        let mut out = std::io::stderr().lock();
        let mut scan = track.then(progress::Scanner::default);
        let mut line: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        let mut shown: Option<u8> = None;
        let mut last_line = String::new();
        let mut shown_at: Option<Instant> = None;
        let mut daemon = true;
        loop {
            let n = match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let chunk = &buf[..n];
            let _ = out.write_all(chunk);
            let _ = out.flush();
            for &b in chunk {
                if b == b'\n' {
                    push_tail(&mut tail, &line);
                    line.clear();
                } else if line.len() < 4096 {
                    line.push(b);
                }
            }
            let pct = scan
                .as_mut()
                .and_then(|s| s.feed(chunk))
                .filter(|p| shown != Some(*p));
            // The newest thing the command printed: the line being drawn, else the last whole one.
            let latest = progress::clean_line(if line.is_empty() {
                tail.back().map_or(&[][..], |l| l.as_bytes())
            } else {
                &line
            });
            let fresh = (!latest.is_empty() && last_line != latest).then_some(latest);
            let due = shown_at.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
            if daemon && due && (pct.is_some() || fresh.is_some()) {
                let mut m = tag(HytteMessage::start(id.clone(), truncated.clone()));
                m.event = TaskEvent::Progress;
                m.progress = pct;
                m.line = fresh.clone();
                daemon = !try_send(&m).is_err_and(|e| is_absent(&e));
                shown = pct.or(shown);
                if let Some(l) = fresh {
                    last_line = l;
                }
                shown_at = Some(Instant::now());
            }
        }
        if !line.is_empty() {
            push_tail(&mut tail, &line);
        }
    }
    let status = child.wait().unwrap_or_else(|e| {
        eprintln!("hytte: wait failed: {e}");
        std::process::exit(126);
    });
    let code = status.code().unwrap_or(-1);
    let dur = t0.elapsed().as_millis() as u64;
    let mut done = tag(HytteMessage::start(id.clone(), truncated));
    done.duration_ms = Some(dur);
    if status.success() {
        done.event = TaskEvent::Done;
        done.exit_code = Some(0);
    } else {
        done.event = TaskEvent::Failed;
        done.exit_code = Some(code);
        let tail_str: Vec<String> = tail.into_iter().collect();
        if !tail_str.is_empty() {
            done.stderr_tail = Some(tail_str.join("\n"));
        }
    }
    let _ = send_msg(&done);
    std::process::exit(code);
}

/// Keeps the last lines for a failure report, as the terminal showed them: only the text after
/// the final `\r` of a redrawn line.
fn push_tail(tail: &mut VecDeque<String>, line: &[u8]) {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let shown = line.rsplit(|&b| b == b'\r').next().unwrap_or(line);
    tail.push_back(String::from_utf8_lossy(shown).into_owned());
    while tail.len() > TAIL_LINES {
        tail.pop_front();
    }
}

/// The message for `hytte set`; None when the arguments don't make sense.
fn build_set(args: &[String]) -> Option<HytteMessage> {
    let mut progress: Option<u8> = None;
    let mut label = String::new();
    let mut task = task_id();
    let mut end: Option<TaskEvent> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--progress" => progress = Some(it.next()?.parse().ok()?),
            "--label" => label = it.next()?.clone(),
            "--task" => task = it.next()?.clone(),
            "--done" => end = Some(TaskEvent::Done),
            "--fail" => end = Some(TaskEvent::Failed),
            _ => return None,
        }
    }
    let mut msg = HytteMessage::start(task, label);
    match (end, progress) {
        // Finish the row: green for --done, red for --fail.
        (Some(ev), None) => {
            msg.event = ev;
            if ev == TaskEvent::Failed {
                msg.exit_code = Some(1);
            }
        }
        (None, Some(p)) if p <= 100 => {
            msg.event = TaskEvent::Progress;
            msg.progress = Some(p);
        }
        _ => return None,
    }
    Some(msg)
}

fn cmd_set(args: &[String]) {
    let Some(msg) = build_set(args) else { usage() };
    if !send_msg(&msg) {
        eprintln!("hytte: daemon not running (message dropped)");
    }
}

#[cfg(windows)]
fn cmd_setup(args: &[String]) {
    if args.first().is_some_and(|a| a == "agents") {
        let undo = match &args[1..] {
            [] => false,
            [a] if a == "--undo" => true,
            _ => usage(),
        };
        for line in crate::agents::connect(undo) {
            println!("{line}");
        }
        return;
    }
    let undo = match args {
        [] => false,
        [a] if a == "--undo" => true,
        _ => usage(),
    };
    if !undo {
        match setup::ensure_on_path() {
            Ok(true) => println!("PATH: added the Hytte folder (new terminals will find `hytte`)"),
            Ok(false) => {}
            Err(e) => println!("PATH: {e}"),
        }
    }
    for line in setup::setup_shells(undo) {
        println!("{line}");
    }
}

#[cfg(not(windows))]
fn cmd_setup(_args: &[String]) {
    eprintln!("hytte setup is Windows-only; see `hytte init <shell>`");
    std::process::exit(2);
}

/// `args` excludes the exe name and is never empty (no arguments starts the daemon instead).
pub fn run(args: &[String]) {
    match args[0].as_str() {
        "help" | "-h" | "--help" | "/?" => println!("{USAGE}"),
        "--version" | "-V" | "version" => println!("hytte {}", env!("CARGO_PKG_VERSION")),
        "run" => cmd_run(&args[1..]),
        "set" => cmd_set(&args[1..]),
        "agent" => agent::run(&args[1..], send_msg, ask_pipe),
        "hook" => hook::run(&args[1..], send_msg),
        "init" => match args.get(1) {
            Some(sh) => hook::init(sh),
            None => hook::usage(),
        },
        "ports" => portscmd::list(&args[1..]),
        "kill" => portscmd::kill_cmd(&args[1..]),
        "setup" => cmd_setup(&args[1..]),
        "doctor" => doctor::run(),
        "notify" => cmd_notify(&args[1..]),
        "card" => cmd_card(&args[1..]),
        "pane" => cmd_pane(&args[1..]),
        "shelf" => cmd_shelf(&args[1..]),
        #[cfg(debug_assertions)]
        "__update" => crate::update::debug_install(),
        _ => usage(),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_card, build_set, find_in, fit_label, quote_arg};
    use hytte_proto::TaskEvent;

    #[test]
    fn set_builds_progress_done_and_fail() {
        let a = |v: &[&str]| build_set(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let p = a(&["--progress", "40", "--label", "x", "--task", "t"]).unwrap();
        assert_eq!(
            (p.event, p.progress, p.task_id.as_str()),
            (TaskEvent::Progress, Some(40), "t")
        );
        let d = a(&["--done", "--task", "t"]).unwrap();
        assert_eq!((d.event, d.exit_code), (TaskEvent::Done, None));
        let f = a(&["--fail", "--task", "t"]).unwrap();
        assert_eq!((f.event, f.exit_code), (TaskEvent::Failed, Some(1)));
        assert!(a(&[]).is_none() && a(&["--progress", "101"]).is_none());
        assert!(a(&["--done", "--progress", "5"]).is_none() && a(&["--bogus"]).is_none());
    }

    #[test]
    fn card_flags_build_a_valid_card_or_say_why_not() {
        let a = |v: &[&str]| build_card(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let c = a(&["--id", "deploy", "--title", "Deploy", "--progress", "40"]).unwrap();
        assert_eq!((c.id.as_deref(), c.progress), (Some("deploy"), Some(40)));
        let w = a(&[
            "--title", "Restart?", "--button", "Restart", "--button", "Later", "--wait",
        ])
        .unwrap();
        assert_eq!(
            (w.buttons.len(), w.ttl, w.wait),
            (2, Some(60), true),
            "--wait waits a minute"
        );
        assert!(a(&["--id", "deploy", "--done"]).unwrap().done);
        assert!(a(&["--id", "x", "--clear"]).unwrap().clear);
        for bad in [
            &["--title", "t", "--wait"][..],
            &["--title", "t", "--progress", "101"],
            &["--title", "t", "--ttl", "0"],
            &["--title", "t", "--icon", "skull"],
            &["--title"],
            &["--clear"],
            &["--bogus"],
            &[],
            &[
                "--title", "t", "--button", "a", "--button", "b", "--button", "c",
            ],
        ] {
            assert!(a(bad).is_err(), "{bad:?}");
        }
        assert!(a(&["--title", "t", "--wait"])
            .unwrap_err()
            .contains("--button"));
    }

    #[test]
    fn quoting_keeps_arguments_whole() {
        assert_eq!(quote_arg("--release"), "--release");
        assert_eq!(quote_arg("fix the bug"), "'fix the bug'");
        assert_eq!(quote_arg("it's"), "'it''s'");
        assert_eq!(quote_arg(""), "''");
        assert_eq!(quote_arg("a;b"), "'a;b'");
    }

    #[test]
    fn a_cut_label_is_marked() {
        assert_eq!(fit_label("short"), "short");
        let cut = fit_label(&"あ".repeat(500));
        assert!(cut.ends_with('…') && cut.len() <= 1003);
    }

    #[test]
    fn finds_cmd_through_pathext_but_leaves_paths_alone() {
        let dir = std::env::temp_dir().join(format!("hytte-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fakenpm.cmd"), "").unwrap();
        let path = std::env::join_paths([&dir]).unwrap();
        // The extension keeps PATHEXT's case; Windows file names don't care.
        let found = find_in("fakenpm", &path, ".COM;.EXE;.BAT;.CMD").unwrap();
        assert!(found
            .to_string_lossy()
            .to_lowercase()
            .ends_with("fakenpm.cmd"));
        assert_eq!(find_in("missing", &path, ".CMD"), None);
        assert_eq!(find_in("fakenpm.cmd", &path, ".CMD"), None);
        assert_eq!(find_in(r"C:\tools\x", &path, ".CMD"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
