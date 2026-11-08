//! Script panes (`[[script_pane]]`): run a pane's command hidden, with a time limit, and turn
//! what it prints (or what `hytte pane set` sends) into a few clean lines for the pill.

use std::time::{Duration, Instant};

/// Lines a script pane shows.
pub const MAX_LINES: usize = 6;
/// Bytes a script pane keeps.
pub const MAX_BYTES: usize = 2048;
/// A command still running after this is stopped, with everything it started.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// What one run of a pane's command left: its cleaned output and how it ended
/// (`None`: stopped at the time limit).
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub code: Option<i32>,
}

/// The pane's command is due: never run, or last run `every` ago.
pub fn due(last: Option<Instant>, every: Duration, now: Instant) -> bool {
    last.is_none_or(|l| now.saturating_duration_since(l) >= every)
}

/// Text from a script made safe and small for the pill: terminal colour codes and other
/// control characters removed, a line rewritten with `\r` (a progress bar) keeps its last
/// state, blank lines at either end dropped, then at most [`MAX_LINES`] lines and
/// [`MAX_BYTES`] bytes.
pub fn clean(s: &str) -> String {
    let mut plain = String::with_capacity(s.len().min(MAX_BYTES * 4));
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.next() {
                // CSI: parameters up to a final byte in @..~.
                Some('[') => {
                    for d in it.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                // OSC (titles, links): up to BEL or ESC \.
                Some(']') => {
                    while let Some(d) = it.next() {
                        if d == '\x07' || (d == '\x1b' && it.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\r' => plain.push(c),
            '\t' => plain.push(' '),
            c if c.is_control() => {}
            c => plain.push(c),
        }
    }
    let lines: Vec<&str> = plain
        .split('\n')
        .map(|l| l.rsplit('\r').find(|p| !p.trim().is_empty()).unwrap_or(""))
        .map(str::trim_end)
        .collect();
    let first = lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(lines.len());
    let last = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(first, |i| i + 1);
    let mut out = lines[first..last]
        .iter()
        .take(MAX_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    hytte_proto::cut(&mut out, MAX_BYTES);
    out
}

/// Run `command` with `cmd /c`, hidden, in the user's home folder, for at most `limit`.
/// The process and everything it starts live in a Job object, so a hung script and its
/// children are all stopped at the limit. Shows stdout, or stderr when stdout is empty.
#[cfg(windows)]
pub fn run(command: &str, limit: Duration) -> Run {
    use std::io::Read;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::*;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let home = std::env::var_os("USERPROFILE").unwrap_or_else(|| ".".into());
    let child = Command::new("cmd")
        .raw_arg(format!("/d /s /c \"{command}\""))
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return Run {
                text: clean(&format!("Couldn't start it: {e}")),
                code: Some(-1),
            }
        }
    };
    // ponytail: the child is put in the job just after it starts, so a grandchild it spawns
    // in that first instant escapes the time limit. Start it suspended (CreateProcessW) if
    // that ever matters.
    // SAFETY: plain Win32 calls on a handle we own; the job is closed below on every path.
    let job = unsafe {
        let job = CreateJobObjectW(None, None).ok();
        if let Some(j) = job {
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let _ = SetInformationJobObject(
                j,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            let _ = AssignProcessToJobObject(j, HANDLE(child.as_raw_handle()));
        }
        job
    };
    // Read both pipes on their own threads so a chatty script can't fill one and stall.
    let grab = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(r) = r {
                let _ = r.take(64 * 1024).read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = grab(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let err = grab(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + limit;
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                // SAFETY: the job handle is valid until closed below.
                unsafe {
                    if let Some(j) = job {
                        let _ = TerminateJobObject(j, 1);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    if let Some(j) = job {
        // SAFETY: closed once; KILL_ON_JOB_CLOSE ends anything still left in it.
        unsafe {
            let _ = CloseHandle(j);
        }
    }
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    let pick = if String::from_utf8_lossy(&out).trim().is_empty() {
        err
    } else {
        out
    };
    Run {
        text: clean(&String::from_utf8_lossy(&pick)),
        code,
    }
}

#[cfg(not(windows))]
pub fn run(_command: &str, _limit: Duration) -> Run {
    Run {
        text: String::new(),
        code: Some(-1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_cleaned_for_the_pill() {
        assert_eq!(clean("\x1b[32mok\x1b[0m  \n"), "ok");
        assert_eq!(clean("\n\n  3 users\n\n"), "  3 users");
        assert_eq!(clean("a\tb"), "a b");
        assert_eq!(clean("\x1b]0;title\x07shown"), "shown");
        assert_eq!(clean("\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\"), "link");
        assert_eq!(clean("10%\r50%\r100%\ndone"), "100%\ndone");
        assert_eq!(clean("win\r\nline\r\n"), "win\nline");
        assert_eq!(clean("bell\x07 null\0"), "bell null");
        let many: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        assert_eq!(clean(&many).lines().count(), MAX_LINES);
        let huge = "あ".repeat(2000);
        let c = clean(&huge);
        assert!(c.len() <= MAX_BYTES && c.is_char_boundary(c.len()));
        assert_eq!(clean(""), "");
    }

    #[test]
    fn a_pane_is_due_once_its_interval_passed() {
        let t0 = Instant::now();
        let every = Duration::from_secs(30);
        assert!(due(None, every, t0));
        assert!(!due(Some(t0), every, t0 + Duration::from_secs(29)));
        assert!(due(Some(t0), every, t0 + every));
    }

    #[cfg(windows)]
    #[test]
    fn commands_run_hidden_and_are_stopped_at_the_limit() {
        let ok = run("echo hello", TIMEOUT);
        assert_eq!((ok.text.as_str(), ok.code), ("hello", Some(0)));
        let bad = run("echo nope 1>&2 & exit /b 3", TIMEOUT);
        assert_eq!((bad.text.as_str(), bad.code), ("nope", Some(3)));
        // ping keeps going for ~30 s; the limit stops it and its cmd parent.
        let t0 = Instant::now();
        let slow = run("ping -n 30 127.0.0.1 >nul", Duration::from_millis(800));
        assert_eq!(slow.code, None);
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
    }
}
