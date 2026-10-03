//! `notch hook ...` and `notch init <shell>` — zero-wrapper tracking.
//! Shell integrations send `start` when a command begins and `end` when the
//! prompt returns; the daemon only shows commands that outlive its threshold.

use hytte_proto::{HytteMessage, TaskEvent};

const PWSH: &str = include_str!("../shell/init.ps1");
const BASH: &str = include_str!("../shell/init.bash");
const ZSH: &str = include_str!("../shell/init.zsh");
const NU: &str = include_str!("../shell/init.nu");

pub fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  notch init <pwsh|bash|zsh|nu>          # print the shell integration script");
    eprintln!("  notch hook start --id I --cmd C --pid P [--cwd D]");
    eprintln!("  notch hook end   --id I --code N [--duration-ms MS]");
    std::process::exit(2);
}

pub fn init(shell: &str) {
    let s = match shell {
        "pwsh" | "powershell" => PWSH,
        "bash" => BASH,
        "zsh" => ZSH,
        "nu" | "nushell" => NU,
        _ => usage(),
    };
    print!("{s}");
}

#[derive(Default)]
struct Opts {
    id: String,
    cmd: String,
    pid: Option<u32>,
    cwd: Option<String>,
    code: Option<i32>,
    duration_ms: Option<u64>,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts::default();
    let mut i = 0;
    while i + 1 < args.len() {
        let v = args[i + 1].clone();
        match args[i].as_str() {
            "--id" => o.id = v,
            "--cmd" => o.cmd = v.chars().take(512).collect(),
            "--pid" => o.pid = v.parse().ok(),
            "--cwd" => o.cwd = Some(v),
            "--code" => o.code = v.parse().ok(),
            "--duration-ms" => o.duration_ms = v.parse().ok(),
            _ => usage(),
        }
        i += 2;
    }
    if o.id.is_empty() {
        usage();
    }
    o
}

fn build(start: bool, o: &Opts) -> HytteMessage {
    let mut m = HytteMessage::start(o.id.clone(), o.cmd.clone());
    m.source = Some("shell".into());
    m.pid = o.pid;
    m.cwd = o.cwd.clone();
    if !start {
        let code = o.code.unwrap_or(0);
        m.event = if code == 0 { TaskEvent::Done } else { TaskEvent::Failed };
        m.exit_code = Some(code);
        m.duration_ms = o.duration_ms;
    }
    m
}

pub fn run(args: &[String], send: impl Fn(&HytteMessage) -> bool) {
    let start = match args.first().map(String::as_str) {
        Some("start") => true,
        Some("end") => false,
        _ => usage(),
    };
    // Hooks must never slow or break a prompt: stay silent if the daemon is down.
    let _ = send(&build(start, &parse(&args[1..])));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_maps_exit_code() {
        let o = Opts { id: "1".into(), code: Some(3), duration_ms: Some(4000), ..Default::default() };
        let m = build(false, &o);
        assert_eq!((m.event, m.exit_code), (TaskEvent::Failed, Some(3)));
        assert_eq!(build(true, &o).source.as_deref(), Some("shell"));
    }

    #[test]
    fn scripts_are_embedded() {
        for s in [PWSH, BASH, ZSH, NU] {
            assert!(s.contains("notch"));
        }
    }
}
