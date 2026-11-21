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

#[cfg(test)]
mod tests {
    use super::*;

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
