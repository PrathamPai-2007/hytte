//! `notch.exe` — tiny separate IPC client.
//! Fast shell startup: std + windows + serde_json only.
//! `notch run -- <cmd...>` still runs the command even if the daemon is absent.

mod agent;
mod hook;
mod portscmd;
mod progress;
// Shared with the daemon; its tray helpers go unused here.
#[path = "../setup.rs"]
#[allow(dead_code)]
mod setup;

use hytte_proto::{HytteMessage, TaskEvent, PIPE_NAME};
use std::collections::VecDeque;
use std::io::Write;
use std::process::Command;
use std::time::{Duration, Instant};

const TAIL_LINES: usize = 5;
/// At most this often a progress update goes to the pipe.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  notch run [--no-progress] -- <cmd...>  # run cmd, report to notch (with progress read from its output)");
    eprintln!("  notch set --progress <0-100> --label \"...\" [--task <id>]");
    eprintln!("  notch agent <start|needs-input|resume|done|fail|hooks> ...   # AI agent / long-task monitor");
    eprintln!("  notch init <pwsh|bash|zsh|nu>          # shell integration: auto-track commands over 3 s");
    eprintln!("  notch setup [--undo]                   # put notch on PATH + add the hook to PowerShell / Git Bash");
    eprintln!("  notch ports [--all]                    # list dev-server listeners");
    eprintln!("  notch kill :PORT [--force]             # stop whatever listens on PORT");
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

/// Best-effort pipe send. Returns false when daemon is absent.
fn send_msg(msg: &HytteMessage) -> bool {
    let line = match msg.to_line() {
        Ok(l) => l,
        Err(_) => return false,
    };
    send_line_best_effort(&line)
}

#[cfg(windows)]
fn send_line_best_effort(line: &str) -> bool {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_FLAG_WRITE_THROUGH to push the message out promptly.
    const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
    let mut f = match OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_WRITE_THROUGH)
        .open(PIPE_NAME)
    {
        Ok(f) => f,
        Err(_) => return false,
    };
    f.write_all(line.as_bytes()).is_ok()
}

#[cfg(not(windows))]
fn send_line_best_effort(_line: &str) -> bool {
    false
}

/// `hytte.exe` next to this exe, started detached; waits briefly for its pipe.
/// Only `notch run` does this (shell hooks must stay instant and never spawn GUIs).
#[cfg(windows)]
fn start_daemon() -> bool {
    use std::os::windows::process::CommandExt;
    let Some(exe) = std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("hytte.exe"))
        .filter(|p| p.exists())
    else {
        return false;
    };
    // DETACHED_PROCESS | CREATE_NO_WINDOW
    if Command::new(exe)
        .creation_flags(0x0800_0008)
        .spawn()
        .is_err()
    {
        return false;
    }
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if std::fs::OpenOptions::new()
            .write(true)
            .open(PIPE_NAME)
            .is_ok()
        {
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn start_daemon() -> bool {
    false
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
    let label = cmd_args.join(" ");
    let truncated: String = label.chars().take(128).collect();

    let tag = |mut m: HytteMessage| {
        m.pid = Some(std::process::id());
        m
    };
    let start = tag(HytteMessage::start(id.clone(), truncated.clone()));
    if !send_msg(&start) && start_daemon() {
        let _ = send_msg(&start);
    }

    let t0 = Instant::now();
    // Spawn child with inherited stdio so the terminal behaves normally,
    // while we capture nothing directly. For the stderr tail we re-run
    // capture only on failure via a second lightweight approach:
    // instead, run with piped stderr + passthrough via tee in-thread.
    // Simplest honest approach: inherit stdout, pipe stderr, forward it.
    let mut child = Command::new(&cmd_args[0])
        .args(&cmd_args[1..])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            let mut fail = tag(HytteMessage::start(id.clone(), truncated.clone()));
            fail.event = TaskEvent::Failed;
            fail.duration_ms = Some(t0.elapsed().as_millis() as u64);
            fail.stderr_tail = Some(format!("spawn failed: {e}"));
            let _ = send_msg(&fail);
            eprintln!("notch: spawn failed: {e}");
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
            let pct = scan.as_mut().and_then(|s| s.feed(chunk)).filter(|p| shown != Some(*p));
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
                daemon = send_msg(&m);
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
        eprintln!("notch: wait failed: {e}");
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

fn cmd_set(args: &[String]) {
    let mut progress: Option<u8> = None;
    let mut label = String::new();
    let mut task = task_id();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--progress" => {
                i += 1;
                if i >= args.len() {
                    usage();
                }
                progress = Some(args[i].parse().unwrap_or_else(|_| usage()));
            }
            "--label" => {
                i += 1;
                if i >= args.len() {
                    usage();
                }
                label = args[i].clone();
            }
            "--task" => {
                i += 1;
                if i >= args.len() {
                    usage();
                }
                task = args[i].clone();
            }
            _ => usage(),
        }
        i += 1;
    }
    let p = match progress {
        Some(p) if p <= 100 => p,
        _ => usage(),
    };
    let mut msg = HytteMessage::start(task, label);
    msg.event = TaskEvent::Progress;
    msg.progress = Some(p);
    if !send_msg(&msg) {
        eprintln!("notch: daemon not running (message dropped)");
    }
}

#[cfg(windows)]
fn cmd_setup(args: &[String]) {
    let undo = match args {
        [] => false,
        [a] if a == "--undo" => true,
        _ => usage(),
    };
    if !undo {
        match setup::ensure_on_path() {
            Ok(true) => println!("PATH: added the notch folder (new terminals will find `notch`)"),
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
    eprintln!("notch setup is Windows-only; see `notch init <shell>`");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    match args[0].as_str() {
        "run" => cmd_run(&args[1..]),
        "set" => cmd_set(&args[1..]),
        "agent" => agent::run(&args[1..], send_msg),
        "hook" => hook::run(&args[1..], send_msg),
        "init" => match args.get(1) {
            Some(sh) => hook::init(sh),
            None => hook::usage(),
        },
        "ports" => portscmd::list(&args[1..]),
        "kill" => portscmd::kill_cmd(&args[1..]),
        "setup" => cmd_setup(&args[1..]),
        _ => usage(),
    }
}
