//! `hytte doctor`: checks that everything Hytte needs is in place and says how to fix what
//! isn't. Exit code 0 when nothing failed (warnings are suggestions), 1 otherwise.

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug)]
pub struct Line {
    pub level: Level,
    pub what: String,
    /// How to fix it; only on a warning or a failure.
    pub fix: Option<String>,
}

impl Line {
    fn ok(what: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            what: what.into(),
            fix: None,
        }
    }
    fn warn(what: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            level: Level::Warn,
            what: what.into(),
            fix: Some(fix.into()),
        }
    }
    fn fail(what: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            level: Level::Fail,
            what: what.into(),
            fix: Some(fix.into()),
        }
    }
}

/// The report as text, one block per check.
pub fn render(lines: &[Line]) -> String {
    let mut out = String::from("Hytte doctor\n");
    for l in lines {
        let tag = match l.level {
            Level::Ok => "ok  ",
            Level::Warn => "warn",
            Level::Fail => "FAIL",
        };
        out.push_str(&format!("  {tag}  {}\n", l.what));
        if let Some(fix) = &l.fix {
            for (i, f) in fix.lines().enumerate() {
                out.push_str(&format!(
                    "        {}{f}\n",
                    if i == 0 { "fix: " } else { "     " }
                ));
            }
        }
    }
    let failed = lines.iter().filter(|l| l.level == Level::Fail).count();
    let warned = lines.iter().filter(|l| l.level == Level::Warn).count();
    out.push_str(&match (failed, warned) {
        (0, 0) => "\nAll good.\n".to_string(),
        (0, w) => format!("\nNo problems; {w} suggestion(s) above.\n"),
        (f, _) => format!("\n{f} problem(s) found.\n"),
    });
    out
}

/// 1 when any check failed.
pub fn exit_code(lines: &[Line]) -> i32 {
    i32::from(lines.iter().any(|l| l.level == Level::Fail))
}

/// An agent's hook state as `agents::status` reports it.
pub fn agent_line(name: &str, state: crate::agents::State) -> Line {
    use crate::agents::State::*;
    match state {
        Connected => Line::ok(format!("{name}: connected")),
        NotConnected => Line::warn(
            format!("{name}: installed but not connected"),
            "run `hytte setup agents`",
        ),
        Stale => Line::fail(
            format!("{name}: its hooks point at a hytte.exe that is gone"),
            "run `hytte setup agents` (Hytte also repairs this when it starts)",
        ),
    }
}

/// The daemon: is something answering on this user's pipe, and is it ours.
fn daemon_line() -> Line {
    match super::open_pipe(false) {
        Ok(_) => Line::ok("the pill is running"),
        Err(e) if super::is_absent(&e) => Line::fail(
            "the pill isn't running",
            "start it by running `hytte` (it also starts with Windows if autostart is on)",
        ),
        Err(e) => Line::fail(
            format!("can't use Hytte's pipe: {e}"),
            "another user's program may hold it; close Hytte there, or restart this one",
        ),
    }
}

pub fn checks() -> Vec<Line> {
    let mut v = vec![];
    let exe: Option<PathBuf> = std::env::current_exe().ok();
    v.push(Line::ok(format!(
        "hytte {} at {}",
        env!("CARGO_PKG_VERSION"),
        exe.as_ref()
            .map_or("(unknown path)".into(), |p| p.display().to_string())
    )));
    v.push(daemon_line());

    // PATH: shell hooks and scripts call `hytte` by name.
    let on_path = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("hytte.exe").is_file()));
    v.push(if on_path {
        Line::ok("`hytte` is on PATH")
    } else {
        Line::fail(
            "`hytte` isn't on PATH (in this terminal)",
            "run `hytte setup`, then open a new terminal",
        )
    });

    #[cfg(windows)]
    {
        let shells = crate::setup::shell_status();
        for (name, set_up, path) in &shells {
            v.push(if *set_up {
                Line::ok(format!("{name}: command tracking is set up"))
            } else {
                Line::warn(
                    format!("{name}: command tracking isn't set up ({})", path.display()),
                    "run `hytte setup`, then open a new terminal",
                )
            });
        }
        let ps_hooked = shells
            .iter()
            .any(|(n, s, _)| *s && *n == "Windows PowerShell");
        if let Some(policy) = crate::setup::ps51_blocked().filter(|_| ps_hooked) {
            v.push(Line::fail(
                format!("Windows PowerShell won't load profiles (policy: {policy})"),
                "Set-ExecutionPolicy -Scope CurrentUser RemoteSigned",
            ));
        }
    }

    let agents = crate::agents::status();
    if agents.is_empty() {
        v.push(Line::ok(
            "no supported coding agent found (nothing to connect)",
        ));
    }
    for (name, state) in agents {
        v.push(agent_line(name, state));
    }

    let cfg = crate::config::config_path();
    match std::fs::read_to_string(&cfg) {
        Ok(text) => match crate::config::problem(&text, true) {
            Some(p) => v.push(Line::fail(
                format!("config.toml: {p}"),
                format!("edit {}", cfg.display()),
            )),
            None => v.push(Line::ok("config.toml is fine")),
        },
        Err(_) => v.push(Line::ok("no config.toml (defaults in use)")),
    }
    v
}

pub fn run() {
    let lines = checks();
    print!("{}", render(&lines));
    std::process::exit(exit_code(&lines));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::State;

    #[test]
    fn failures_say_how_to_fix_and_set_the_exit_code() {
        let lines = vec![
            Line::ok("fine"),
            Line::warn("meh", "do this"),
            Line::fail("broken", "do that\nthen this"),
        ];
        let text = render(&lines);
        assert!(text.contains("  ok    fine"), "{text}");
        assert!(text.contains("  warn  meh\n        fix: do this"), "{text}");
        assert!(
            text.contains("  FAIL  broken\n        fix: do that\n             then this"),
            "{text}"
        );
        assert!(text.contains("1 problem(s) found."));
        assert_eq!(exit_code(&lines), 1);
        // Warnings alone are not a failure.
        assert_eq!(exit_code(&lines[..2]), 0);
        assert!(render(&lines[..1]).contains("All good."));
    }

    #[test]
    fn agent_states_map_to_levels() {
        assert_eq!(agent_line("Claude Code", State::Connected).level, Level::Ok);
        assert_eq!(agent_line("Codex", State::NotConnected).level, Level::Warn);
        let stale = agent_line("Cursor", State::Stale);
        assert_eq!(stale.level, Level::Fail);
        assert!(stale.fix.unwrap().contains("hytte setup agents"));
    }
}
