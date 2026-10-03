//! Shared IPC protocol for `\\.\pipe\hytte`.
//! Newline-delimited JSON, every message carries `v` for versioning.

use serde::{Deserialize, Serialize};

/// Protocol version. Bump on breaking change.
pub const PROTOCOL_VERSION: u32 = 1;
/// Canonical pipe name.
pub const PIPE_NAME: &str = r"\\.\pipe\hytte";
/// Max single message bytes — pipe input is untrusted.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskEvent {
    Start,
    Progress,
    Done,
    Failed,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HytteMessage {
    /// Protocol version, always [`PROTOCOL_VERSION`].
    pub v: u32,
    pub task_id: String,
    pub event: TaskEvent,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_tail: Option<String>,
}

impl HytteMessage {
    pub fn start(task_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            task_id: task_id.into(),
            event: TaskEvent::Start,
            label: label.into(),
            progress: None,
            exit_code: None,
            duration_ms: None,
            stderr_tail: None,
        }
    }

    /// Validate untrusted input: version, lengths, UTF-8 already guaranteed by String.
    pub fn validate(&self) -> bool {
        if self.v != PROTOCOL_VERSION {
            return false;
        }
        if self.task_id.is_empty() || self.task_id.len() > 128 {
            return false;
        }
        if self.label.len() > 1024 {
            return false;
        }
        if let Some(p) = self.progress {
            if p > 100 {
                return false;
            }
        }
        if let Some(s) = &self.stderr_tail {
            if s.len() > 8192 {
                return false;
            }
        }
        true
    }

    /// Encode as a single NDJSON line.
    pub fn to_line(&self) -> anyhow_string::Result<String> {
        serde_json::to_string(self)
            .map(|mut s| {
                s.push('\n');
                s
            })
            .map_err(|e| anyhow_string::Error(e.to_string()))
    }
}

/// Minimal error carrier so `proto` stays dependency-light (no `anyhow`).
pub mod anyhow_string {
    #[derive(Debug)]
    pub struct Error(pub String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl std::error::Error for Error {}
    pub type Result<T> = std::result::Result<T, Error>;
    impl From<serde_json::Error> for Error {
        fn from(e: serde_json::Error) -> Self {
            Self(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_validate() {
        let m = HytteMessage::start("abc", "cargo build");
        assert!(m.validate());
        let line = m.to_line().unwrap();
        assert!(line.ends_with('\n'));
        let back: HytteMessage = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(back.task_id, "abc");
    }

    #[test]
    fn rejects_bad_progress() {
        let mut m = HytteMessage::start("t", "x");
        m.progress = Some(101);
        assert!(!m.validate());
    }
}
