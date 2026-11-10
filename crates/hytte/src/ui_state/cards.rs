//! Chips and live cards: what a card is, its buttons, held alerts and the ask click guard.

use super::*;

/// What an extra chip button does when clicked.
#[derive(Debug, Clone, PartialEq)]
pub enum ChipAction {
    /// Open a link (a meeting, a page).
    OpenUrl(String),
    /// Put a file on the shelf.
    Shelve(PathBuf),
    /// Terminate a process; the first click arms it ("Kill?"), the second confirms.
    KillPid(u32, String),
    /// Stop alerting about a process (by exe name) for this session.
    IgnoreExe(String),
    /// Answer the pending agent ask with this id: allow (true) or deny.
    Answer(u64, bool),
    /// Download and install the update that is on offer.
    InstallUpdate,
    /// Not now: don't offer this version (tag) again.
    UpdateLater(String),
    /// A live card's button (card id, label): reported back to a waiting script, nothing run.
    Card(String, String),
    /// The welcome tour: show this card next (past the last one: the tour is done).
    Welcome(usize),
    /// The welcome tour: skip the rest.
    WelcomeSkip,
}

/// The look of a chip's icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// A finished job: purple tick.
    #[default]
    Done,
    /// Something wants attention: amber.
    Warn,
    /// For your information: blue.
    Info,
    /// Something failed: red (a live card's `error` icon).
    Error,
}

/// How long a progress card stays without an update.
pub const CARD_SILENCE: Duration = Duration::from_secs(30);

/// What makes a chip a live card (`hytte card`): its id, and the parts updates change.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub body: String,
    pub progress: Option<u8>,
}

/// How urgent a chip is. The chip slot shows one at a time; the rest wait in [`Model::cards`],
/// most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Prio {
    /// News and the result of something you did.
    #[default]
    Info,
    /// A download finished, a process is hogging the CPU, a heads-up.
    Alert,
    /// A script's live card (`hytte card`).
    Card,
    /// A new version is on offer.
    Update,
    /// An agent is waiting for your answer.
    Ask,
}

/// A chip that has waited this long for the slot (or its own hold, if longer) is dropped unseen.
pub const WAIT_STALE: Duration = Duration::from_secs(30);

/// At most this many chips wait for the slot; past it the least urgent, newest one is dropped.
pub const MAX_WAITING: usize = 8;

#[derive(Debug, Clone)]
pub struct Chip {
    /// What it is when the slot is busy: see [`Prio`].
    pub prio: Prio,
    /// A chip with the same key replaces this one (a second "download finished" for the same
    /// file) instead of queueing behind it.
    pub key: Option<String>,
    pub summary: String,
    /// A quiet line under the text ("Edited 4 files · 84k context"); the card grows to hold it.
    pub note: Option<String>,
    pub open: Option<PathBuf>,
    pub copy: Option<String>,
    pub since: Instant,
    /// Buttons beyond Open / Copy / Dismiss, in order.
    pub extra: Vec<(String, ChipAction)>,
    /// How long it stays before folding away on its own.
    pub hold: Duration,
    pub tone: Tone,
    /// Set when the chip is a script's live card.
    pub card: Option<Card>,
}

impl Chip {
    /// Short enough for one line of the card, which then needs less height.
    pub fn one_line(&self) -> bool {
        self.summary.chars().count() < 44 && !self.summary.contains('\n')
    }

    pub fn new(summary: impl Into<String>, since: Instant) -> Self {
        Self {
            prio: Prio::Info,
            key: None,
            summary: summary.into(),
            note: None,
            open: None,
            copy: None,
            since,
            extra: vec![],
            hold: CHIP_HOLD,
            tone: Tone::Done,
            card: None,
        }
    }

    /// The live card's id, if this chip is one.
    pub fn card_id(&self) -> Option<&str> {
        self.card.as_ref().map(|c| c.id.as_str())
    }

    /// Apply a `card` command to this card: only the fields it carries change, and the hold
    /// starts again (a progress card is "30 s of silence", not 30 s in all).
    fn apply_card(&mut self, c: &hytte_proto::Cmd, now: Instant) {
        let Some(k) = self.card.as_mut() else { return };
        if let Some(t) = &c.title {
            k.title = t.trim().to_string();
        }
        if let Some(t) = &c.text {
            k.body = t.trim().to_string();
        }
        if c.progress.is_some() {
            k.progress = c.progress;
        }
        if c.done && k.progress.is_some() {
            k.progress = Some(100);
        }
        self.summary = match (k.title.is_empty(), k.body.is_empty()) {
            (false, false) => format!("{}\n{}", k.title, k.body),
            (true, _) => k.body.clone(),
            (false, true) => k.title.clone(),
        };
        self.tone = match (c.icon.as_deref(), c.done) {
            (Some("ok"), _) => Tone::Done,
            (Some("warn"), _) => Tone::Warn,
            (Some("error"), _) => Tone::Error,
            (Some(_), _) => Tone::Info,
            (None, true) => Tone::Done,
            (None, false) => self.tone,
        };
        if !c.buttons.is_empty() || c.open.is_some() {
            let id = k.id.clone();
            self.extra.clear();
            self.open = None;
            match c.open.as_deref() {
                Some(o) if o.starts_with("http") => self
                    .extra
                    .push(("Open".into(), ChipAction::OpenUrl(o.into()))),
                Some(o) => self.open = Some(o.into()),
                None => {}
            }
            for b in &c.buttons {
                let b = b.trim().to_string();
                self.extra
                    .push((b.clone(), ChipAction::Card(id.clone(), b)));
            }
        }
        let running = self.card.as_ref().is_some_and(|k| k.progress.is_some()) && !c.done;
        self.hold = match c.ttl {
            Some(t) => Duration::from_secs(t as u64),
            None if running => CARD_SILENCE,
            None => Duration::from_secs(hytte_proto::CARD_TTL_SECS as u64),
        };
        self.since = now;
    }
}

/// Alerts held back while a Focus session runs, to be told as one line afterwards.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Held {
    pub downloads: u32,
    pub done: u32,
    pub failed: u32,
    pub warnings: u32,
}

impl Held {
    /// "While you focused: 2 downloads, 1 task failed", or None when nothing was held. Counts
    /// start over.
    pub fn take_summary(&mut self) -> Option<String> {
        let h = std::mem::take(self);
        let plural = |n: u32, one: &str, many: &str| {
            (n > 0).then(|| format!("{n} {}", if n == 1 { one } else { many }))
        };
        let parts: Vec<String> = [
            plural(h.downloads, "download", "downloads"),
            plural(h.done, "task finished", "tasks finished"),
            plural(h.failed, "task failed", "tasks failed"),
            plural(h.warnings, "warning", "warnings"),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| format!("While you focused: {}", parts.join(", ")))
    }
}

/// Clicks on Allow / Deny count only this long after an ask appears.
pub const ASK_CLICK_GUARD: Duration = Duration::from_millis(400);

/// Whether an Allow / Deny click may answer: not within [`ASK_CLICK_GUARD`] of the ask appearing.
pub fn ask_click_ready(shown: Option<Instant>, now: Instant) -> bool {
    shown.is_none_or(|s| now.saturating_duration_since(s) >= ASK_CLICK_GUARD)
}

impl Model {
    pub fn dismiss_chip(&mut self) {
        self.chip = None;
    }

    /// Where `c` goes among the waiting chips: after the last one at least as urgent.
    fn queue_at(&self, c: &Chip) -> usize {
        self.cards
            .iter()
            .position(|k| k.prio < c.prio)
            .unwrap_or(self.cards.len())
    }

    /// Put `c` in line, most urgent first, first come first served within a level.
    fn enqueue(&mut self, c: Chip) {
        let at = self.queue_at(&c);
        self.cards.insert(at, c);
        self.cards.truncate(MAX_WAITING);
    }

    /// Show `c` now if the slot is free (and nothing more urgent waits), else let it wait its
    /// turn. A chip with the same `key` as the one on screen or waiting replaces it. The caller
    /// shows waiting chips with [`Model::show_card`] when the way is clear.
    pub fn push_chip(&mut self, c: Chip, now: Instant) {
        if let Some(key) = &c.key {
            if let Some(shown) = self.chip.as_mut().filter(|k| k.key.as_ref() == Some(key)) {
                *shown = c;
                shown.since = now;
                return;
            }
            self.cards.retain(|k| k.key.as_ref() != Some(key));
        }
        let free = self.chip.is_none() && self.cards.front().is_none_or(|f| f.prio <= c.prio);
        if free {
            self.chip = Some(c);
            self.chip_armed = None;
        } else {
            self.enqueue(c);
        }
    }

    /// The result of something you just did: it takes the slot now, unless an update offer or
    /// an ask is on it. What it displaces waits at the front, except plain news.
    pub fn interrupt(&mut self, c: Chip, now: Instant) {
        if self.chip.as_ref().is_some_and(|k| k.prio >= Prio::Update) {
            return self.push_chip(c, now);
        }
        self.requeue_card();
        self.chip = Some(c);
        self.chip_armed = None;
    }

    /// A script's live card (`hytte card`): update the card with its id wherever it is (on the
    /// chip or queued), or queue a new one. `clear` takes it away. Returns the card's id.
    /// The caller shows queued cards with [`Model::show_card`] when nothing is in the way.
    pub fn card(&mut self, c: &hytte_proto::Cmd, now: Instant) -> String {
        let id = match &c.id {
            Some(id) => id.clone(),
            None => {
                self.card_seq += 1;
                format!("card:{}", self.card_seq)
            }
        };
        let on_chip = self.chip.as_ref().and_then(Chip::card_id) == Some(id.as_str());
        if c.clear {
            if on_chip {
                self.chip = None;
            }
            self.cards.retain(|k| k.card_id() != Some(id.as_str()));
            return id;
        }
        let slot = if on_chip {
            self.chip.as_mut()
        } else {
            self.cards
                .iter_mut()
                .find(|k| k.card_id() == Some(id.as_str()))
        };
        match slot {
            Some(k) => k.apply_card(c, now),
            None => {
                let mut k = Chip::new("", now);
                k.prio = Prio::Card;
                k.tone = Tone::Info;
                k.card = Some(Card {
                    id: id.clone(),
                    title: String::new(),
                    body: String::new(),
                    progress: None,
                });
                k.apply_card(c, now);
                self.enqueue(k);
            }
        }
        id
    }

    /// Put the next queued card on the chip if the chip is free. Its hold starts now.
    pub fn show_card(&mut self, now: Instant) {
        if self.chip.is_none() {
            if let Some(mut k) = self.cards.pop_front() {
                k.since = now;
                self.chip = Some(k);
                self.chip_armed = None;
            }
        }
    }

    /// Something more urgent takes the chip: what is on it goes back to the front of the queue
    /// instead of being lost (plain news is not worth showing twice).
    pub fn requeue_card(&mut self) {
        if self.chip.as_ref().is_some_and(|c| c.prio > Prio::Info) {
            if let Some(c) = self.chip.take() {
                self.cards.push_front(c);
            }
        }
    }

    /// Waiting chips that aren't live cards go stale: after [`WAIT_STALE`] (or their own hold,
    /// if longer) counted from when they arrived, nobody needs to hear about them any more.
    /// Returns the next such deadline.
    pub(super) fn expire_waiting(&mut self, now: Instant) -> Option<Instant> {
        let stale = |k: &Chip| k.since + k.hold.max(WAIT_STALE);
        self.cards.retain(|k| k.card.is_some() || now < stale(k));
        self.cards
            .iter()
            .filter(|k| k.card.is_none())
            .map(stale)
            .min()
    }

    /// Whether a live card with this id is on the chip or waiting for it.
    pub fn has_card(&self, id: &str) -> bool {
        self.chip.as_ref().and_then(Chip::card_id) == Some(id)
            || self.cards.iter().any(|k| k.card_id() == Some(id))
    }
}
