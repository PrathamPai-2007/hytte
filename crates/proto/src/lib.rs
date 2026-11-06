//! Shared IPC protocol for `\\.\pipe\hytte`.
//! Newline-delimited JSON, every message carries `v` for versioning.

use serde::{Deserialize, Serialize};

pub mod ports;
pub mod sys;

/// Protocol version. Bump on breaking change.
pub const PROTOCOL_VERSION: u32 = 1;
/// The pipe name prefix; the user's SID follows, so each Windows user has their own pipe.
pub const PIPE_PREFIX: &str = r"\\.\pipe\hytte";

/// This user's pipe: `\\.\pipe\hytte-<SID>`. Another account's Hytte uses another name, and a
/// stranger who creates the plain name can't intercept anything.
pub fn pipe_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| match sys::user_sid() {
        Some(sid) => format!("{PIPE_PREFIX}-{sid}"),
        None => PIPE_PREFIX.to_string(),
    })
}
/// Max single message bytes — pipe input is untrusted.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskEvent {
    Start,
    Progress,
    Done,
    Failed,
    Lost,
    /// An agent/long task is blocked waiting for a human.
    NeedsInput,
    /// A previously blocked agent continues running.
    Resumed,
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
    /// Who sent this: "run", "shell" or "agent". Additive; absent = "run".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Owning process (the terminal's shell/agent), used for click-to-focus and liveness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Daemon shows the task only after this many ms (shell hooks use 3000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u32>,
    /// Free text, e.g. why an agent needs input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The latest line of the command's output, for the live ticker. Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
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
            source: None,
            pid: None,
            delay_ms: None,
            message: None,
            cwd: None,
            line: None,
        }
    }

    /// Trim every text field to its limit, on a character boundary.
    fn clamp(&mut self) {
        cut(&mut self.label, 1024);
        for (f, max) in [
            (&mut self.stderr_tail, 8192),
            (&mut self.message, 1024),
            (&mut self.cwd, 1024),
            (&mut self.source, 16),
            (&mut self.line, 512),
        ] {
            if let Some(s) = f {
                cut(s, max);
            }
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
        if self.message.as_ref().is_some_and(|m| m.len() > 1024)
            || self.cwd.as_ref().is_some_and(|c| c.len() > 1024)
            || self.source.as_ref().is_some_and(|c| c.len() > 16)
            || self.line.as_ref().is_some_and(|l| l.len() > 512)
        {
            return false;
        }
        true
    }

    /// Encode as a single NDJSON line. Over-long text is trimmed to the limits [`validate`]
    /// enforces, so a long reply or error tail still arrives instead of being rejected.
    pub fn to_line(&self) -> anyhow_string::Result<String> {
        let mut m = self.clone();
        m.clamp();
        serde_json::to_string(&m)
            .map(|mut s| {
                s.push('\n');
                s
            })
            .map_err(|e| anyhow_string::Error(e.to_string()))
    }
}

/// Truncate to at most `max` bytes without splitting a character.
fn cut(s: &mut String, max: usize) {
    if s.len() > max {
        let mut n = max;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
}

/// How long the daemon waits for the human to answer an [`Ask`]. Claude Code shows its own
/// permission prompt only after the hook returns, so this is also how long the terminal waits.
pub const ASK_WAIT_SECS: u64 = 30;

/// A yes/no question for the human (`hytte agent ask`). The daemon answers on the same
/// connection with one [`Answer`] line, or closes it unanswered (timeout, dismissed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ask {
    pub v: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// What is being asked, shown verbatim on the pill.
    pub text: String,
}

impl Ask {
    pub fn validate(&self) -> bool {
        self.v == PROTOCOL_VERSION
            && !self.name.is_empty()
            && self.name.len() <= 128
            && !self.text.is_empty()
            && self.text.len() <= 1024
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Answer {
    pub allow: bool,
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
    fn old_messages_still_parse() {
        let old = r#"{"v":1,"task_id":"t","event":"Done","label":"x","duration_ms":5}"#;
        let m: HytteMessage = serde_json::from_str(old).unwrap();
        assert!(m.validate() && m.pid.is_none() && m.source.is_none());
        let new = r#"{"v":1,"task_id":"a","event":"NeedsInput","message":"approve?","pid":42,"source":"agent"}"#;
        let m: HytteMessage = serde_json::from_str(new).unwrap();
        assert_eq!((m.event, m.pid), (TaskEvent::NeedsInput, Some(42)));
    }

    #[test]
    fn ask_is_not_a_task_message() {
        let a = Ask {
            v: PROTOCOL_VERSION,
            name: "Claude Code".into(),
            pid: Some(7),
            text: "Running: rm -rf build".into(),
        };
        assert!(a.validate());
        let line = serde_json::to_string(&a).unwrap();
        // The pipe tries HytteMessage first; an Ask must never parse as one.
        assert!(serde_json::from_str::<HytteMessage>(&line).is_err());
        let back: Ask = serde_json::from_str(&line).unwrap();
        assert_eq!(back.pid, Some(7));
        assert!(!Ask {
            text: String::new(),
            ..a
        }
        .validate());
    }

    #[test]
    fn to_line_trims_long_text_on_a_char_boundary() {
        let mut m = HytteMessage::start("t", "あ".repeat(600)); // 1800 bytes
        m.message = Some("x".repeat(4000));
        m.stderr_tail = Some("e".repeat(20_000));
        m.line = Some("l".repeat(900));
        let back: HytteMessage = serde_json::from_str(m.to_line().unwrap().trim()).unwrap();
        assert!(back.validate());
        assert_eq!(back.label.len(), 1023); // 341 whole characters
        assert_eq!(back.message.unwrap().len(), 1024);
        assert_eq!(back.stderr_tail.unwrap().len(), 8192);
    }

    #[test]
    fn the_pipe_is_per_user() {
        let name = pipe_name();
        assert!(name.starts_with(PIPE_PREFIX));
        #[cfg(windows)]
        assert!(name.contains("-S-1-"), "{name}");
    }

    #[test]
    fn rejects_bad_progress() {
        let mut m = HytteMessage::start("t", "x");
        m.progress = Some(101);
        assert!(!m.validate());
    }
}
