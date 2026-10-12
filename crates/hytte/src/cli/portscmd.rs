//! `hytte ports` and `hytte kill :PORT` — talk to the OS directly, no daemon needed.

use hytte_proto::ports::{current, kill};
use std::io::Write;

pub fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  hytte ports [--all]                 # list dev-server listeners");
    eprintln!("  hytte kill :PORT [--force]          # stop the process listening on PORT");
    std::process::exit(2);
}

pub fn list(args: &[String]) {
    let all = args.iter().any(|a| a == "--all");
    let ports = current(hytte_proto::ports::DEFAULT_WATCH, all);
    if ports.is_empty() {
        println!(
            "no dev listeners found{}",
            if all { "" } else { " (try --all)" }
        );
    }
    for p in ports {
        println!(":{:<6} {:<24} pid {}", p.port, p.exe, p.pid);
    }
}

pub fn kill_cmd(args: &[String]) {
    let force = args.iter().any(|a| a == "--force");
    let Some(spec) = args.iter().find(|a| !a.starts_with("--")) else {
        usage()
    };
    let port: u16 = spec
        .trim_start_matches(':')
        .parse()
        .unwrap_or_else(|_| usage());
    let Some(p) = current(&[], true).into_iter().find(|p| p.port == port) else {
        eprintln!("hytte: nothing is listening on :{port}");
        std::process::exit(1);
    };
    if !force {
        print!(
            "Stop {} (pid {}) listening on :{port}? [y/N] ",
            p.exe, p.pid
        );
        let _ = std::io::stdout().flush();
        let mut a = String::new();
        let _ = std::io::stdin().read_line(&mut a);
        if !a.trim().eq_ignore_ascii_case("y") {
            println!("cancelled");
            return;
        }
    }
    match kill(p.pid) {
        Ok(()) => println!("stopped {} on :{port}", p.exe),
        Err(e) => {
            eprintln!("hytte: {e}");
            std::process::exit(1);
        }
    }
}
