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
    /// An agent's context size in tokens (its last turn's input, from the transcript). Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    /// A file the agent is about to change (Edit, Write ...): the pill counts the distinct ones.
    /// Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The agent is launching a subagent: the pill counts launches. Additive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub subagent: bool,
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
            context: None,
            file: None,
            subagent: false,
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
            (&mut self.file, 1024),
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
            || self.file.as_ref().is_some_and(|f| f.len() > 1024)
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
pub fn cut(s: &mut String, max: usize) {
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

/// A request from a script (`hytte notify`, `hytte card`, `hytte shelf add`): show a card, or
/// put a file or text on the shelf. Never parses as a [`HytteMessage`] (no `task_id`) or an
/// [`Ask`] (no `name`), so an older daemon ignores it. The fields after `path` belong to `card`
/// and are all optional; a daemon that predates cards drops a `card` line as an unknown command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cmd {
    pub v: u32,
    /// `"notify"`, `"card"` or `"shelf_add"`.
    pub cmd: String,
    /// The card text (`notify`, a `card`'s body) or the snippet to shelve (`shelf_add`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// An absolute path to shelve (`shelf_add`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A card sent again with the same id updates it in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 0-100: a bar under the text. The card then stays until `done` or 30 s of silence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    /// One of [`CARD_ICONS`]; absent = info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Up to two labels. A press is only reported back (`wait`); it never runs anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buttons: Vec<String>,
    /// An http(s) URL or an absolute path, offered as **Open**.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
    /// Seconds the card stays (default [`CARD_TTL_SECS`], at most [`CARD_MAX_TTL_SECS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u32>,
    /// The job behind a progress card finished.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// Take the card with this id away.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear: bool,
    /// Keep the connection open and answer with one [`Pressed`] line (or hang up: no press).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wait: bool,
}

/// The icons a card may ask for.
pub const CARD_ICONS: [&str; 4] = ["info", "ok", "warn", "error"];
/// How long a card stays unless it says otherwise.
pub const CARD_TTL_SECS: u32 = 10;
pub const CARD_MAX_TTL_SECS: u32 = 3600;
/// How long a waited-on card may sit in the queue behind other cards before the wait gives up.
/// The daemon hangs up as soon as the card expires or is dismissed; this only caps a lost one.
// ponytail: a fixed allowance, not "until shown"; tie the wait to the card's display if queues
// of long cards ever make five minutes too short.
pub const CARD_QUEUE_SECS: u64 = 300;

/// The daemon's answer to a `card` sent with `wait`: which button was pressed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pressed {
    pub pressed: String,
}

/// Longest snippet `shelf_add` carries.
pub const MAX_SNIPPET_BYTES: usize = 16 * 1024;
/// Longest text `pane_set` carries (the pane keeps 2 KB of it).
pub const MAX_PANE_TEXT_BYTES: usize = 4 * 1024;

impl Cmd {
    pub fn notify(text: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            cmd: "notify".into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn shelve_path(path: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            cmd: "shelf_add".into(),
            path: Some(path.into()),
            ..Default::default()
        }
    }

    pub fn shelve_text(text: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            cmd: "shelf_add".into(),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    /// An empty `card`; fill in what you need.
    pub fn card() -> Self {
        Self {
            v: PROTOCOL_VERSION,
            cmd: "card".into(),
            ..Default::default()
        }
    }

    /// Text for the script pane called `name` (`hytte pane set`). An empty text empties it.
    pub fn pane_set(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            cmd: "pane_set".into(),
            id: Some(name.into()),
            text: Some(text.into()),
            ..Default::default()
        }
    }

    /// Untrusted input: the version, a known command, and sane lengths.
    pub fn validate(&self) -> bool {
        if self.v != PROTOCOL_VERSION {
            return false;
        }
        let text = self.text.as_deref().map(str::len);
        let path = self.path.as_deref().map(str::len);
        let card_only = self.title.is_some()
            || self.progress.is_some()
            || self.icon.is_some()
            || !self.buttons.is_empty()
            || self.open.is_some()
            || self.ttl.is_some()
            || self.done
            || self.clear
            || self.wait;
        if (card_only && self.cmd != "card")
            || (self.id.is_some() && !matches!(self.cmd.as_str(), "card" | "pane_set"))
        {
            return false;
        }
        match self.cmd.as_str() {
            "card" => self.card_ok(),
            // The pane's name rides in `id`.
            "pane_set" => {
                self.path.is_none()
                    && self
                        .id
                        .as_ref()
                        .is_some_and(|n| !n.trim().is_empty() && n.len() <= 64)
                    && text.is_some_and(|n| n <= MAX_PANE_TEXT_BYTES)
            }
            "notify" => self.path.is_none() && text.is_some_and(|n| n > 0 && n <= 1024),
            "shelf_add" => match (text, path) {
                (Some(n), None) => n > 0 && n <= MAX_SNIPPET_BYTES,
                (None, Some(n)) => n > 0 && n <= 1024 && self.path_is_absolute(),
                _ => false,
            },
            _ => false,
        }
    }

    fn card_ok(&self) -> bool {
        let fits = |s: &Option<String>, max: usize| s.as_ref().is_none_or(|s| s.len() <= max);
        let filled = |s: &Option<String>| s.as_ref().is_some_and(|s| !s.trim().is_empty());
        let id_ok = self
            .id
            .as_ref()
            .is_none_or(|i| !i.is_empty() && i.len() <= 128);
        let open_ok = self.open.as_ref().is_none_or(|o| {
            o.len() <= 1024
                && (o.starts_with("https://")
                    || o.starts_with("http://")
                    || std::path::Path::new(o).is_absolute())
        });
        // Clearing names the card; anything else must say something or update one by id.
        let shape = if self.clear {
            self.id.is_some()
        } else {
            self.id.is_some() || filled(&self.title) || filled(&self.text)
        };
        self.path.is_none()
            && shape
            && id_ok
            && open_ok
            && fits(&self.title, 128)
            && fits(&self.text, 1024)
            && self.progress.is_none_or(|p| p <= 100)
            && self.icon.as_deref().is_none_or(|i| CARD_ICONS.contains(&i))
            && self.buttons.len() <= 2
            && self
                .buttons
                .iter()
                .all(|b| !b.trim().is_empty() && b.len() <= 24)
            && self
                .ttl
                .is_none_or(|t| (1..=CARD_MAX_TTL_SECS).contains(&t))
            && (!self.wait || !self.buttons.is_empty())
    }

    fn path_is_absolute(&self) -> bool {
        self.path
            .as_deref()
            .is_some_and(|p| std::path::Path::new(p).is_absolute())
    }

    /// Encode as a single NDJSON line.
    pub fn to_line(&self) -> anyhow_string::Result<String> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
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
        assert_eq!(
            (m.context, m.file.as_deref(), m.subagent),
            (None, None, false)
        );
    }

    #[test]
    fn agent_details_are_optional_and_unknown_fields_are_ignored() {
        let mut m = HytteMessage::start("agent:x:1", "Claude Code");
        m.event = TaskEvent::Progress;
        m.context = Some(84_000);
        m.file = Some(r"C:\src\main.rs".into());
        m.subagent = true;
        let line = m.to_line().unwrap();
        let back: HytteMessage = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(back.context, Some(84_000));
        assert_eq!(back.file.as_deref(), Some(r"C:\src\main.rs"));
        assert!(back.subagent && back.validate());
        // Nothing extra is written for a plain message, so an older daemon sees what it knew.
        let plain = HytteMessage::start("t", "x").to_line().unwrap();
        assert!(
            !plain.contains("context") && !plain.contains("file") && !plain.contains("subagent")
        );
        // And a future field an old daemon doesn't know is no reason to reject the message.
        let future = r#"{"v":1,"task_id":"t","event":"Start","label":"x","from_the_future":[1,2]}"#;
        assert!(serde_json::from_str::<HytteMessage>(future)
            .unwrap()
            .validate());
        let long = HytteMessage {
            file: Some("f".repeat(2000)),
            ..HytteMessage::start("t", "x")
        };
        assert!(!long.validate());
        assert!(HytteMessage::validate(
            &serde_json::from_str(&long.to_line().unwrap()).unwrap()
        ));
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
    fn commands_are_validated_and_never_mistaken_for_other_messages() {
        let n = Cmd::notify("build finished");
        assert!(n.validate());
        let line = n.to_line().unwrap();
        assert!(serde_json::from_str::<HytteMessage>(line.trim()).is_err());
        assert!(serde_json::from_str::<Ask>(line.trim()).is_err());
        assert!(!Cmd::notify("").validate());
        assert!(!Cmd::notify("x".repeat(2000)).validate());
        assert!(Cmd::shelve_text("hello").validate());
        assert!(!Cmd::shelve_text("x".repeat(MAX_SNIPPET_BYTES + 1)).validate());
        assert!(!Cmd::shelve_path("relative/file.txt").validate());
        #[cfg(windows)]
        assert!(Cmd::shelve_path(r"C:\Users\me\a.txt").validate());
        let mut both = Cmd::shelve_text("t");
        both.path = Some("C:/x".into());
        assert!(!both.validate());
        let mut other = Cmd::notify("x");
        other.cmd = "format_disk".into();
        assert!(!other.validate());
        // Neither of the old messages parses as a command.
        let ask = serde_json::to_string(&Ask {
            v: PROTOCOL_VERSION,
            name: "a".into(),
            pid: None,
            text: "t".into(),
        })
        .unwrap();
        assert!(serde_json::from_str::<Cmd>(&ask).is_err());
        assert!(
            serde_json::from_str::<Cmd>(&HytteMessage::start("t", "x").to_line().unwrap()).is_err()
        );
    }

    #[test]
    fn cards_are_validated() {
        let mut c = Cmd::card();
        assert!(!c.validate(), "an empty card says nothing");
        c.title = Some("Deploy".into());
        c.progress = Some(40);
        assert!(c.validate());
        let line = c.to_line().unwrap();
        assert!(serde_json::from_str::<HytteMessage>(line.trim()).is_err());
        assert!(serde_json::from_str::<Ask>(line.trim()).is_err());
        let back: Cmd = serde_json::from_str(line.trim()).unwrap();
        assert_eq!((back.progress, back.done), (Some(40), false));
        // An update needs only its id.
        let upd = Cmd {
            id: Some("deploy".into()),
            done: true,
            ..Cmd::card()
        };
        assert!(upd.validate());
        let bad = |f: fn(&mut Cmd)| {
            let mut c = Cmd {
                title: Some("t".into()),
                ..Cmd::card()
            };
            f(&mut c);
            !c.validate()
        };
        assert!(bad(|c| c.progress = Some(101)));
        assert!(bad(|c| c.icon = Some("skull".into())));
        assert!(bad(|c| c.buttons = vec!["a".into(), "b".into(), "c".into()]));
        assert!(bad(|c| c.buttons = vec!["x".repeat(25)]));
        assert!(bad(|c| c.buttons = vec![" ".into()]));
        assert!(bad(|c| c.open = Some("relative/file".into())));
        assert!(bad(|c| c.open = Some("javascript:alert(1)".into())));
        assert!(bad(|c| c.ttl = Some(0)));
        assert!(bad(|c| c.ttl = Some(CARD_MAX_TTL_SECS + 1)));
        assert!(bad(|c| c.title = Some("x".repeat(129))));
        assert!(bad(|c| c.id = Some(String::new())));
        assert!(bad(|c| c.wait = true), "waiting needs a button to press");
        assert!(bad(|c| c.clear = true), "clearing needs an id");
        assert!(bad(|c| c.path = Some("C:/x".into())));
        let mut ok = Cmd {
            title: Some("t".into()),
            buttons: vec!["Restart".into(), "Later".into()],
            wait: true,
            open: Some("https://example.com".into()),
            ..Cmd::card()
        };
        assert!(ok.validate());
        ok.icon = Some("error".into());
        assert!(ok.validate());
        // Card fields on another command are refused, not silently ignored.
        let mut n = Cmd::notify("x");
        n.progress = Some(5);
        assert!(!n.validate());
    }

    #[test]
    fn old_commands_parse_with_the_card_fields_absent() {
        let old = r#"{"v":1,"cmd":"notify","text":"hi"}"#;
        let c: Cmd = serde_json::from_str(old).unwrap();
        assert!(c.validate() && c.id.is_none() && c.buttons.is_empty() && !c.wait);
        // A notify line written now is exactly what 1.6 wrote.
        assert_eq!(Cmd::notify("hi").to_line().unwrap().trim(), old);
    }

    #[test]
    fn pane_text_is_validated() {
        let p = Cmd::pane_set("Server", "3 users online");
        assert!(p.validate());
        assert!(
            Cmd::pane_set("Server", "").validate(),
            "empty text empties the pane"
        );
        assert!(!Cmd::pane_set(" ", "x").validate());
        assert!(!Cmd::pane_set("x".repeat(65), "x").validate());
        assert!(!Cmd::pane_set("S", "x".repeat(MAX_PANE_TEXT_BYTES + 1)).validate());
        let mut b = Cmd::pane_set("S", "x");
        b.progress = Some(3);
        assert!(!b.validate(), "no card fields on a pane");
        let mut n = Cmd::notify("x");
        n.id = Some("a".into());
        assert!(!n.validate(), "ids belong to cards and panes");
        let line = p.to_line().unwrap();
        assert!(serde_json::from_str::<HytteMessage>(line.trim()).is_err());
    }

    #[test]
    fn rejects_bad_progress() {
        let mut m = HytteMessage::start("t", "x");
        m.progress = Some(101);
        assert!(!m.validate());
    }
}
