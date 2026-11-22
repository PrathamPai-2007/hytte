//! What stands between an agent's tool call and your machine: a rate limit and an audit log.
//! Used by the pill (for `action_run`) and by `hytte mcp` (for the pane tools).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Calls of one kind allowed in [`WINDOW`]. A runaway loop gets a tool error, not your machine.
pub const MAX_CALLS: usize = 10;
pub const WINDOW: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Limiter {
    calls: VecDeque<Instant>,
}

impl Limiter {
    /// Whether a call at `now` fits; a call that fits is counted.
    pub fn allow(&mut self, now: Instant) -> bool {
        while self
            .calls
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW)
        {
            self.calls.pop_front();
        }
        if self.calls.len() >= MAX_CALLS {
            return false;
        }
        self.calls.push_back(now);
        true
    }
}

/// One line for the log: control characters become spaces and each field is cut to 200
/// characters, so a payload can neither forge a log line nor fill the disk.
pub fn audit_line(unix_secs: u64, tool: &str, target: &str, result: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(200)
            .collect()
    };
    format!(
        "{unix_secs}\t{}\t{}\t{}\n",
        clean(tool),
        clean(target),
        clean(result)
    )
}

/// Append to `%APPDATA%\Hytte\mcp.log` (about 256 KB, then one old copy is kept).
pub fn audit(tool: &str, target: &str, result: &str) {
    let path = crate::config::data_dir().join("mcp.log");
    audit_to(&path, tool, target, result);
}

pub fn audit_to(path: &std::path::Path, tool: &str, target: &str, result: &str) {
    use std::io::Write;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(audit_line(secs, tool, target, result).as_bytes());
    }
}

// ---- "always allow this action": set from a terminal, not by an agent's tool call ----

/// `%APPDATA%\Hytte\mcp-grants\<action>.grant`, beside `pack-answers`, outside any pack.
pub fn grants_dir() -> std::path::PathBuf {
    crate::config::data_dir().join("mcp-grants")
}

/// What a grant is tied to (SHA-256): the action's name, its command and the folder it runs in. Edit the
/// command (or approve a pack that redefines the name) and the grant no longer matches, so the
/// pill asks again.
pub fn grant_hash(name: &str, command: &str, dir: Option<&std::path::Path>) -> String {
    // SHA-256 over length-prefixed fields: no value can be split differently to collide.
    let dir = dir.map(|d| d.display().to_string()).unwrap_or_default();
    let text = format!(
        "{}:{name}{}:{command}{}:{dir}",
        name.len(),
        command.len(),
        dir.len()
    );
    crate::registry::sha256_hex(text.as_bytes())
}

fn grant_file(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    hytte_proto::valid_action_id(name).then(|| dir.join(format!("{}.grant", name.to_lowercase())))
}

pub fn is_granted_in(dir: &std::path::Path, name: &str, hash: &str) -> bool {
    grant_file(dir, name)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .is_some_and(|t| t.trim() == hash)
}

pub fn grant_in(dir: &std::path::Path, name: &str, hash: &str) -> Result<(), String> {
    let f = grant_file(dir, name).ok_or("not an action name")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(f, hash).map_err(|e| e.to_string())
}

/// True when there was a grant to remove.
pub fn revoke_in(dir: &std::path::Path, name: &str) -> bool {
    grant_file(dir, name).is_some_and(|f| std::fs::remove_file(f).is_ok())
}

/// The granted action names (lower case).
pub fn granted_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".grant")
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_holds_only_for_the_command_that_was_granted() {
        let d = std::env::temp_dir().join(format!("hytte-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let h = grant_hash("deploy", "make deploy", None);
        assert!(!is_granted_in(&d, "deploy", &h));
        grant_in(&d, "Deploy", &h).unwrap();
        assert!(is_granted_in(&d, "deploy", &h) && is_granted_in(&d, "DEPLOY", &h));
        // The command was edited (or a pack redefined the name): no longer granted.
        assert!(!is_granted_in(
            &d,
            "deploy",
            &grant_hash("deploy", "make evil", None)
        ));
        // Fields can't be shifted across the separator to make two different actions equal.
        assert_ne!(grant_hash("a", "b\0c", None), grant_hash("a\0b", "c", None));
        assert_ne!(
            grant_hash("a", "c", None),
            grant_hash("a", "c", Some(std::path::Path::new("C:/pack")))
        );
        assert_eq!(granted_in(&d), ["deploy"]);
        assert!(revoke_in(&d, "deploy") && !revoke_in(&d, "deploy"));
        assert!(!is_granted_in(&d, "deploy", &h));
        // A name that could climb out of the folder is not an action name.
        assert!(grant_in(&d, "..", &h).is_err() && grant_in(&d, "a/b", &h).is_err());
        assert!(!revoke_in(&d, ".."));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_limiter_stops_at_ten_a_minute_and_forgets() {
        let mut l = Limiter::default();
        let t0 = Instant::now();
        for i in 0..MAX_CALLS {
            assert!(l.allow(t0 + Duration::from_secs(i as u64)), "call {i}");
        }
        assert!(!l.allow(t0 + Duration::from_secs(30)), "the eleventh");
        // The first call is a minute old: one slot is free again, then it is full.
        assert!(l.allow(t0 + Duration::from_secs(60)));
        assert!(!l.allow(t0 + Duration::from_secs(60)));
        assert!(l.allow(t0 + Duration::from_secs(200)), "all forgotten");
    }

    #[test]
    fn a_log_line_cannot_be_forged_or_grow_without_limit() {
        let l = audit_line(5, "tool", "a\nFAKE\tline\r", &"x".repeat(1000));
        assert_eq!(l.matches('\n').count(), 1, "one line");
        assert_eq!(l.split('\t').count(), 4, "exactly four fields: {l:?}");
        assert!(l.len() < 450);
    }

    #[test]
    fn audit_appends_and_rotates() {
        let dir = std::env::temp_dir().join(format!("hytte-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let f = dir.join("mcp.log");
        audit_to(&f, "action_run", "deploy", "allowed");
        audit_to(&f, "action_run", "deploy", "denied");
        let text = std::fs::read_to_string(&f).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().next().unwrap().ends_with("allowed"));
        std::fs::write(&f, vec![b'x'; 300 * 1024]).unwrap();
        audit_to(&f, "t", "x", "y");
        assert!(f.with_extension("log.old").exists());
        assert_eq!(std::fs::read_to_string(&f).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
