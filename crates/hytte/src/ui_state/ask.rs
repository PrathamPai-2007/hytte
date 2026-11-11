//! The Ask pane (one question, one streamed answer from the user's own Ollama) and the AI card
//! (Explain / Summarise / Shorten on a clip, a shelf file or a failed command).
//!
//! Pure state: the window layer runs the requests on worker threads and feeds the pieces back as
//! [`UiEvent`]s. The wrapped height of the text comes from the renderer (it needs DirectWrite) and
//! is stored in [`Reply::text_h`] by the window layer before the pill's size is worked out.

use super::*;
use crate::ollama::{PullBar, Question, Status};

/// Height of the question field.
pub const ASK_FIELD_H: f32 = 34.0;
/// The answer box: least and most px it takes in the Ask pane (it scrolls past the most).
pub const ASK_BOX_MIN: f32 = 40.0;
pub const ASK_BOX_MAX: f32 = 150.0;
/// The line under the answer: the model, and Copy / Clear.
pub const ASK_FOOT_H: f32 = 18.0;
/// The Ask pane's room for a message and its button when Ollama isn't ready.
pub const ASK_NOTICE_H: f32 = 74.0;
/// Space left and right of the answer text.
pub const REPLY_PAD: f32 = 16.0;
/// Width the answer text wraps to (the pill is 380 wide; a thin scroll bar sits at the right).
pub const REPLY_W: f32 = 380.0 - 2.0 * REPLY_PAD - 6.0;
/// The AI card: the most its text takes before it scrolls inside the card, and the room around.
pub const AI_CARD_CHROME: f32 = 86.0;
pub const AI_BOX_MAX: f32 = 290.0 - AI_CARD_CHROME;

/// How long a finished AI card stays before folding away.
pub const AI_HOLD: Duration = Duration::from_secs(60);
/// Bytes of a clip, file or error output sent with an action.
pub const AI_INPUT_BYTES: usize = 6000;
/// Longest answer kept, in bytes.
const REPLY_MAX: usize = 12_000;

/// Brief, plain answers: the pill is small and a 1–2B model rambles.
const SYSTEM: &str = "You answer inside a small desktop widget. Reply in plain text only: no \
markdown, no headings, no bullet symbols, no code fences. Be brief and concrete. If you are \
not sure, say so.";

/// What a one-click action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiKind {
    Explain,
    Summarise,
    Shorten,
}

impl AiKind {
    pub fn label(self) -> &'static str {
        match self {
            AiKind::Explain => "Explain",
            AiKind::Summarise => "Summarise",
            AiKind::Shorten => "Shorten",
        }
    }

    fn instruction(self) -> &'static str {
        match self {
            AiKind::Explain => "Explain what this is and what it means, in 2 to 4 short sentences.",
            AiKind::Summarise => "Summarise this in 2 to 4 short sentences.",
            AiKind::Shorten => {
                "Rewrite this shorter and clearer, keeping its meaning. Reply with only the rewritten text."
            }
        }
    }

    /// Most tokens the answer may take.
    fn max_tokens(self) -> u32 {
        match self {
            AiKind::Shorten => 300,
            _ => 220,
        }
    }
}

/// Where an action's text comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum AiSource {
    /// A clipboard clip, by its place in the list.
    Clip(usize),
    /// A shelf item, by id: the window layer reads the start of its file.
    File(u64),
    /// A failed task, by id: its command, exit code and the end of its output.
    Failed(String),
}

/// An answer on its way: the text so far, what the user sees of it, and its scroll.
#[derive(Debug, Clone, Default)]
pub struct Reply {
    /// What the model said.
    pub text: String,
    /// The same without markdown marks: what is drawn, measured and copied.
    pub shown: String,
    pub busy: bool,
    /// What went wrong (Ollama's own words, or "Ollama isn't running").
    pub error: Option<String>,
    /// px scrolled, and whether the view sticks to the newest line.
    pub scroll: f32,
    pub follow: bool,
    /// Height of `shown` wrapped to the box's width, from the renderer.
    pub text_h: f32,
    /// Bytes of `shown` that `text_h` was measured for.
    pub measured: usize,
}

impl Reply {
    fn begin() -> Self {
        Self {
            busy: true,
            follow: true,
            ..Self::default()
        }
    }

    pub fn push(&mut self, piece: &str) {
        if self.text.len() < REPLY_MAX {
            self.text.push_str(piece);
            self.shown = plain(&self.text);
        }
    }

    /// Farthest the text can scroll inside a box `box_h` tall.
    pub fn max_scroll(&self, box_h: f32) -> f32 {
        (self.text_h - box_h).max(0.0)
    }

    /// Keep the scroll inside the text (and at the end while it follows the answer).
    pub fn settle(&mut self, box_h: f32) {
        let max = self.max_scroll(box_h);
        self.scroll = if self.follow {
            max
        } else {
            self.scroll.clamp(0.0, max)
        };
    }

    /// The wheel: `dy` px (positive = further down). Reaching the end follows the answer again.
    pub fn scroll_by(&mut self, dy: f32, box_h: f32) {
        let max = self.max_scroll(box_h);
        self.scroll = (self.scroll + dy).clamp(0.0, max);
        self.follow = self.scroll >= max - 1.0;
    }

    /// Record the wrapped height measured for the text now. While the answer is still coming
    /// the height only grows, so a box never shrinks under the reader's eyes.
    pub fn measured_h(&mut self, h: f32) {
        self.text_h = if self.busy { h.max(self.text_h) } else { h };
        self.measured = self.shown.len();
    }

    /// The renderer must measure again.
    pub fn stale(&self) -> bool {
        self.measured != self.shown.len()
    }
}

/// Text as plain lines: markdown marks and a thinking block removed, bullets as "•".
pub fn plain(s: &str) -> String {
    // A model that thinks anyway: drop what is inside <think>…</think> (or all of it, while open).
    let mut text = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("<think>") {
        text.push_str(&rest[..i]);
        match rest[i..].find("</think>") {
            Some(j) => rest = &rest[i + j + "</think>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    text.push_str(rest);
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.trim_start().lines().enumerate() {
        if line.trim_start().starts_with("```") {
            continue;
        }
        if n > 0 && !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        let t = line.trim_start();
        let t = t.trim_start_matches('#').trim_start();
        let (lead, t) = match t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            Some(x) => ("• ", x),
            None => ("", t),
        };
        out.push_str(lead);
        out.push_str(&t.replace("**", "").replace("__", "").replace('`', ""));
    }
    out.trim_end().to_string()
}

/// `text` cut to at most `max` bytes on a character boundary; whether it was cut.
pub fn cut(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut n = max;
    while !text.is_char_boundary(n) {
        n -= 1;
    }
    (&text[..n], true)
}

/// The Ask pane's state besides its field.
#[derive(Debug, Clone, Default)]
pub struct AskView {
    pub status: Status,
    pub reply: Reply,
    /// Ollama's download of the model, while one runs.
    pub pull: Option<PullBar>,
    pub pull_err: Option<String>,
}

/// The card an action's answer shows in.
#[derive(Debug, Clone)]
pub struct AiCard {
    pub title: String,
    pub reply: Reply,
    /// When the answer finished (the card folds away [`AI_HOLD`] later).
    pub done_at: Option<Instant>,
}

/// A question to run: where the answer goes, which request it is, and what to ask.
#[derive(Debug, Clone, PartialEq)]
pub struct AiJob {
    pub to_card: bool,
    pub seq: u64,
    pub host: String,
    pub question: Question,
}

impl Model {
    /// `[ai]` as the config has it now. Switching off forgets the answers.
    pub fn apply_ai(&mut self, ai: &crate::config::Ai) {
        let was = self.ai.enabled;
        self.ai = ai.clone();
        if was && !ai.enabled {
            self.ask = AskView::default();
            self.ai_card = None;
            if self.typing == Some(Field::Ask) {
                self.typing = None;
            }
        }
        // A different address or model: look again when the pane opens.
        self.ask.status = Status::Unknown;
    }

    /// The Ask pane is in the tab strip.
    pub fn ask_shown(&self) -> bool {
        self.ai.enabled && self.panes.ask
    }

    /// The box under the field: its height for what it holds.
    pub fn ask_box_h(&self) -> f32 {
        if !matches!(self.ask.status, Status::Ready) {
            return ASK_NOTICE_H;
        }
        let r = &self.ask.reply;
        if r.shown.is_empty() && r.error.is_none() {
            ASK_BOX_MIN
        } else {
            // A line of room under the text; an error takes at least its two lines.
            (r.text_h + 4.0).clamp(ASK_BOX_MIN, ASK_BOX_MAX)
        }
    }

    /// Height of the Ask pane without the tab strip.
    pub fn ask_h(&self) -> f64 {
        (12.0 + ASK_FIELD_H + 8.0 + self.ask_box_h() + 6.0 + ASK_FOOT_H + 8.0) as f64
    }

    /// Height of the text box of the AI card.
    pub fn ai_box_h(&self) -> f32 {
        self.ai_card
            .as_ref()
            .map_or(0.0, |c| (c.reply.text_h + 4.0).clamp(20.0, AI_BOX_MAX))
    }

    /// Height of the AI card.
    pub fn ai_card_h(&self) -> f64 {
        (AI_CARD_CHROME + self.ai_box_h()) as f64
    }

    /// Start asking what is in the field. None when there is nothing to ask, or Ollama isn't
    /// ready. The field keeps its text, so a question can be changed and asked again.
    pub fn begin_ask(&mut self) -> Option<AiJob> {
        let q = self.ask_edit.text().trim().to_string();
        if q.is_empty() || self.ask.reply.busy || self.ask.status != Status::Ready {
            return None;
        }
        self.ai_seq += 1;
        self.ask.reply = Reply::begin();
        Some(self.job(false, q, AiKind::Explain.max_tokens() + 180))
    }

    fn job(&self, to_card: bool, prompt: String, max_tokens: u32) -> AiJob {
        AiJob {
            to_card,
            seq: self.ai_seq,
            host: self.ai.host.clone(),
            question: Question {
                model: self.ai.model.clone(),
                system: SYSTEM.into(),
                prompt,
                max_tokens,
                keep_alive: self.ai.keep_alive.clone(),
            },
        }
    }

    /// Start an action on `text`, answered in the card. The text is cut to [`AI_INPUT_BYTES`].
    pub fn begin_ai(&mut self, kind: AiKind, what: &str, text: &str) -> AiJob {
        let (body, was_cut) = cut(text, AI_INPUT_BYTES);
        self.ai_seq += 1;
        let title = format!(
            "{} {what}{}",
            kind.label(),
            if was_cut { " (start only)" } else { "" }
        );
        self.ai_card = Some(AiCard {
            title,
            reply: Reply::begin(),
            done_at: None,
        });
        let prompt = format!("{}\n\n{body}", kind.instruction());
        self.job(true, prompt, kind.max_tokens())
    }

    /// An action on a failed command: its line, how it ended and what it printed last.
    pub fn begin_ai_failed(&mut self, command: &str, code: Option<i32>, output: &str) -> AiJob {
        let code = code.map_or(String::new(), |c| format!("Exit code: {c}\n"));
        let text = format!("Command: {command}\n{code}Output:\n{output}");
        let (body, was_cut) = cut(&text, AI_INPUT_BYTES);
        self.ai_seq += 1;
        self.ai_card = Some(AiCard {
            title: format!(
                "Why it failed{}",
                if was_cut { " (start only)" } else { "" }
            ),
            reply: Reply::begin(),
            done_at: None,
        });
        let prompt = format!(
            "This command failed. Say in 2 to 4 short sentences what most likely went wrong and what to try next.\n\n{body}"
        );
        self.job(true, prompt, 260)
    }

    /// A piece of an answer arrived. One for a request that was replaced since is dropped.
    pub fn ai_text(&mut self, seq: u64, to_card: bool, piece: &str) {
        if seq != self.ai_seq {
            return;
        }
        match (to_card, self.ai_card.as_mut()) {
            (true, Some(c)) => c.reply.push(piece),
            (false, _) => self.ask.reply.push(piece),
            _ => {}
        }
    }

    /// An answer finished (or failed, or was stopped).
    pub fn ai_done(&mut self, seq: u64, to_card: bool, r: Result<(), String>, now: Instant) {
        if seq != self.ai_seq {
            return;
        }
        let reply = match (to_card, self.ai_card.as_mut()) {
            (true, Some(c)) => {
                c.done_at = Some(now);
                &mut c.reply
            }
            (false, _) => &mut self.ask.reply,
            _ => return,
        };
        reply.busy = false;
        reply.follow = reply.follow || reply.shown.is_empty();
        if let Err(e) = r {
            reply.error = Some(e);
        }
    }

    /// Stop the answer that is coming (the pane's Stop, or the card's).
    pub fn stop_ai(&mut self, now: Instant) {
        let seq = self.ai_seq;
        if self.ask.reply.busy {
            self.ai_done(seq, false, Ok(()), now);
        }
        if self.ai_card.as_ref().is_some_and(|c| c.reply.busy) {
            self.ai_done(seq, true, Ok(()), now);
        }
    }

    /// Forget the pane's question and answer.
    pub fn clear_ask(&mut self) {
        self.ai_seq += 1;
        self.ask.reply = Reply::default();
        self.ask_edit.clear();
    }

    /// Close the card.
    pub fn dismiss_ai(&mut self) {
        self.ai_seq += 1;
        self.ai_card = None;
    }

    /// The card folds away a while after its answer is complete.
    pub fn ai_card_due(&self) -> Option<Instant> {
        self.ai_card
            .as_ref()
            .and_then(|c| c.done_at.map(|d| d + AI_HOLD))
    }

    /// The text field is the Ask pane's; the model, to show under the answer.
    pub fn ask_model(&self) -> &str {
        &self.ai.model
    }

    /// Ollama was looked at.
    pub fn ollama_status(&mut self, s: Status) {
        self.ask.status = s;
    }

    /// Start the model download.
    pub fn begin_pull(&mut self) {
        self.ask.pull = Some(PullBar::default());
        self.ask.pull_err = None;
    }

    pub fn pull_progress(&mut self, p: &crate::ollama::Pull) {
        if let Some(b) = self.ask.pull.as_mut() {
            b.update(p);
        }
    }

    /// The download ended: on success Ollama is looked at again; an error is kept to show.
    pub fn pull_done(&mut self, r: Result<(), String>) {
        self.ask.pull = None;
        match r {
            Ok(()) => self.ask.status = Status::Unknown,
            Err(e) => self.ask.pull_err = Some(e),
        }
    }

    /// The text of clip `i`, for an action on it.
    pub fn clip_text(&self, i: usize) -> Option<String> {
        self.clips.get(i).map(str::to_string)
    }

    /// A failed task's command, exit code and output tail, for Explain; None without output.
    pub fn failed_text(&self, id: &str) -> Option<(String, Option<i32>, String)> {
        let t = self.tasks.iter().find(|t| t.id == id)?;
        let out = t.stderr.join("\n");
        (!out.trim().is_empty()).then(|| (t.label.clone(), t.exit_code, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_marks_are_dropped() {
        assert_eq!(plain("**Bold** and `code`"), "Bold and code");
        assert_eq!(plain("## Title\n- one\n* two"), "Title\n• one\n• two");
        assert_eq!(plain("```rust\nlet x = 1;\n```"), "let x = 1;");
        assert_eq!(plain("  leading"), "leading");
        assert_eq!(plain(""), "");
    }

    #[test]
    fn a_thinking_block_is_hidden_even_while_open() {
        assert_eq!(plain("<think>hmm</think>The answer."), "The answer.");
        assert_eq!(plain("<think>still thinking"), "");
        assert_eq!(plain("A<think>x</think>B<think>y</think>C"), "ABC");
    }

    #[test]
    fn cutting_never_splits_a_character() {
        assert_eq!(cut("hello", 10), ("hello", false));
        assert_eq!(cut("hello", 3), ("hel", true));
        // "é" is two bytes: cutting inside it backs up.
        assert_eq!(cut("aé", 2), ("a", true));
        assert_eq!(cut("日本語", 4), ("日", true));
    }

    #[test]
    fn the_box_only_grows_while_the_answer_comes_and_scrolls_past_the_cap() {
        let mut r = Reply::begin();
        r.push("one line");
        r.measured_h(40.0);
        r.push(" more");
        r.measured_h(20.0); // a shorter measure while busy: kept at the larger
        assert_eq!(r.text_h, 40.0);
        r.busy = false;
        r.measured_h(20.0);
        assert_eq!(r.text_h, 20.0);
        // Scrolling: the text is 400 high in a 150 box.
        r.text_h = 400.0;
        r.follow = true;
        r.settle(150.0);
        assert_eq!(r.scroll, 250.0);
        r.scroll_by(-100.0, 150.0);
        assert_eq!((r.scroll, r.follow), (150.0, false));
        r.scroll_by(500.0, 150.0);
        assert_eq!((r.scroll, r.follow), (250.0, true));
        r.scroll_by(-999.0, 150.0);
        assert_eq!(r.scroll, 0.0);
    }

    fn ready() -> Model {
        let mut m = Model::default();
        m.ai.enabled = true;
        m.ai.model = "qwen3:1.7b".into();
        m.ai.host = "127.0.0.1:11434".into();
        m.ai.keep_alive = "2m".into();
        m.ask.status = Status::Ready;
        m
    }

    #[test]
    fn asking_needs_a_question_and_a_ready_ollama() {
        let mut m = ready();
        assert!(m.begin_ask().is_none());
        m.ask_edit.set("why is the sky blue");
        m.ask.status = Status::NotRunning;
        assert!(m.begin_ask().is_none());
        m.ask.status = Status::Ready;
        let job = m.begin_ask().unwrap();
        assert!(!job.to_card && job.seq == 1);
        assert_eq!(job.question.prompt, "why is the sky blue");
        assert_eq!(job.question.model, "qwen3:1.7b");
        assert!(m.ask.reply.busy);
        // Busy: a second press does nothing.
        assert!(m.begin_ask().is_none());
    }

    #[test]
    fn pieces_build_the_answer_and_a_replaced_request_is_ignored() {
        let mut m = ready();
        m.ask_edit.set("q");
        let job = m.begin_ask().unwrap();
        m.ai_text(job.seq, false, "Hel");
        m.ai_text(job.seq, false, "lo **you**");
        assert_eq!(m.ask.reply.shown, "Hello you");
        m.ai_done(job.seq, false, Ok(()), Instant::now());
        assert!(!m.ask.reply.busy && m.ask.reply.error.is_none());
        // A stale piece (another request started) is dropped.
        m.clear_ask();
        m.ai_text(job.seq, false, "late");
        assert!(m.ask.reply.shown.is_empty());
    }

    #[test]
    fn an_error_is_kept_with_what_came_before() {
        let mut m = ready();
        m.ask_edit.set("q");
        let job = m.begin_ask().unwrap();
        m.ai_text(job.seq, false, "partial");
        m.ai_done(
            job.seq,
            false,
            Err("model 'x' not found".into()),
            Instant::now(),
        );
        assert_eq!(m.ask.reply.error.as_deref(), Some("model 'x' not found"));
        assert_eq!(m.ask.reply.shown, "partial");
    }

    #[test]
    fn an_action_makes_a_card_with_a_cut_note_and_it_folds_away_later() {
        let mut m = ready();
        let big = "x".repeat(AI_INPUT_BYTES + 100);
        let job = m.begin_ai(AiKind::Summarise, "clip", &big);
        assert!(job.to_card);
        assert!(m.ai_card.as_ref().unwrap().title.contains("start only"));
        assert!(job.question.prompt.len() < AI_INPUT_BYTES + 200);
        assert!(job.question.prompt.starts_with("Summarise this"));
        assert!(m.ai_card_due().is_none(), "busy: no deadline yet");
        let now = Instant::now();
        m.ai_text(job.seq, true, "Short.");
        m.ai_done(job.seq, true, Ok(()), now);
        assert_eq!(m.ai_card_due(), Some(now + AI_HOLD));
        m.dismiss_ai();
        assert!(m.ai_card.is_none());
    }

    #[test]
    fn a_new_action_replaces_a_running_one() {
        let mut m = ready();
        let a = m.begin_ai(AiKind::Explain, "clip", "one");
        let b = m.begin_ai(AiKind::Shorten, "clip", "two");
        m.ai_text(a.seq, true, "stale");
        assert!(m.ai_card.as_ref().unwrap().reply.shown.is_empty());
        m.ai_text(b.seq, true, "fresh");
        assert_eq!(m.ai_card.as_ref().unwrap().reply.shown, "fresh");
        assert!(b.question.prompt.starts_with("Rewrite this shorter"));
    }

    #[test]
    fn turning_ai_off_forgets_everything() {
        let mut m = ready();
        m.ask_edit.set("q");
        let job = m.begin_ask().unwrap();
        m.ai_text(job.seq, false, "secret answer");
        m.begin_ai(AiKind::Explain, "clip", "text");
        let mut off = m.ai.clone();
        off.enabled = false;
        m.apply_ai(&off);
        assert!(m.ask.reply.text.is_empty());
        assert!(m.ai_card.is_none());
    }

    #[test]
    fn the_pane_height_follows_the_state() {
        let mut m = ready();
        let empty = m.ask_h();
        m.ask.reply.push("a long answer");
        m.ask.reply.text_h = 100.0;
        assert!(m.ask_h() > empty);
        m.ask.reply.text_h = 5000.0;
        assert_eq!(m.ask_box_h(), ASK_BOX_MAX);
        m.ask.status = Status::NotRunning;
        assert_eq!(m.ask_box_h(), ASK_NOTICE_H);
    }

    #[test]
    fn a_failed_command_needs_output_to_explain() {
        let m = ready();
        assert!(m.failed_text("nope").is_none());
    }
}

#[cfg(test)]
mod scene_tests {
    use super::*;

    fn on() -> Model {
        let mut m = Model::default();
        m.panes = crate::config::Panes::default();
        m.apply_ai(&crate::config::Ai {
            enabled: true,
            ..Default::default()
        });
        m
    }

    #[test]
    fn the_pane_shows_only_while_ai_and_its_switch_are_on() {
        let mut m = on();
        assert!(m.panels().contains(&Panel::Ask));
        m.panes.ask = false;
        assert!(!m.panels().contains(&Panel::Ask));
        m.panes.ask = true;
        let off = crate::config::Ai::default();
        m.apply_ai(&off);
        assert!(!m.panels().contains(&Panel::Ask), "off by default");
        assert_eq!(Panel::from_key("ask"), Some(Panel::Ask));
        assert_eq!(Panel::from_name("Ask"), Some(Panel::Ask));
    }

    #[test]
    fn the_card_waits_below_an_agents_question_and_above_the_rest() {
        let mut m = on();
        m.now = Some(Instant::now());
        m.ask.status = Status::Ready;
        let job = m.begin_ai(AiKind::Explain, "this clip", "text");
        assert_eq!(m.scene(false, false), Scene::ExpAi);
        // An agent's question takes the slot (a chip) and the card comes back after.
        m.chip = Some(Chip::new("Claude wants to run rm", Instant::now()));
        assert_eq!(m.scene(false, false), Scene::ExpChip);
        m.chip = None;
        assert_eq!(m.scene(false, false), Scene::ExpAi);
        let _ = job;
        // With ai switched off the card never shows.
        let mut off = m.ai.clone();
        off.enabled = false;
        m.apply_ai(&off);
        assert_ne!(m.scene(false, false), Scene::ExpAi);
    }

    #[test]
    fn the_card_grows_with_its_answer_up_to_the_screen() {
        let mut m = on();
        let job = m.begin_ai(AiKind::Summarise, "this clip", "text");
        m.ai_text(job.seq, true, "x");
        let c = m.ai_card.as_mut().unwrap();
        c.reply.text_h = 40.0;
        let small = m.size(Scene::ExpAi).1;
        m.ai_card.as_mut().unwrap().reply.text_h = 120.0;
        let bigger = m.size(Scene::ExpAi).1;
        assert!(bigger > small);
        m.ai_card.as_mut().unwrap().reply.text_h = 9000.0;
        // Never taller than the pill's canvas allows.
        assert!(m.size(Scene::ExpAi).1 <= 290.0);
    }

    #[test]
    fn a_finished_card_folds_away_on_its_own() {
        let mut m = on();
        let job = m.begin_ai(AiKind::Explain, "this clip", "text");
        let t0 = Instant::now();
        m.ai_done(job.seq, true, Ok(()), t0);
        assert!(m.expire(t0 + Duration::from_secs(5)).is_some());
        assert!(m.ai_card.is_some());
        m.expire(t0 + AI_HOLD + Duration::from_secs(1));
        assert!(m.ai_card.is_none());
    }

    #[test]
    fn a_clip_opens_its_chips_and_they_close_when_the_list_changes() {
        let mut m = on();
        m.apply_clipboard(true, 0);
        m.add_clip("one");
        m.add_clip("two");
        let closed = m.clips_h();
        m.toggle_clip_ai(1);
        assert_eq!(m.clip_ai_row(), Some(1));
        assert!(m.clips_h() > closed);
        m.toggle_clip_ai(1);
        assert_eq!(m.clip_ai_row(), None);
        m.toggle_clip_ai(0);
        m.add_clip("three");
        assert_eq!(m.clip_ai_row(), None, "indexes moved: closed");
        // Off: no chips even if one was open.
        m.toggle_clip_ai(0);
        let off = crate::config::Ai::default();
        m.apply_ai(&off);
        assert_eq!(m.clip_ai_row(), None);
        assert_eq!(
            m.clips_h(),
            (CLIP_HEAD_H + 3.0 * CLIP_ROW_H + CLIP_PAD_H) as f64
        );
    }

    #[test]
    fn shelf_files_get_ask_chips_only_for_text_and_only_when_on() {
        use std::path::Path;
        let mut m = on();
        let labels = |m: &Model, p: &str| {
            m.chips_of(Path::new(p))
                .iter()
                .map(|c| c.0)
                .collect::<Vec<_>>()
        };
        assert!(labels(&m, "notes.md").contains(&"Summarise"));
        assert!(labels(&m, "build.log").contains(&"Explain"));
        assert!(!labels(&m, "photo.png").contains(&"Explain"));
        assert!(!labels(&m, "movie.mkv").contains(&"Summarise"));
        let off = crate::config::Ai::default();
        m.apply_ai(&off);
        assert!(!labels(&m, "notes.md").contains(&"Summarise"));
    }
}
