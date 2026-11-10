//! Pure UI model + animation state (no Win32), so scene selection, task
//! lifecycle and expiry are unit-testable. The renderer reads this; the
//! window layer feeds it events and clocks.

use crate::animation::{RectSpring, Spring};
use crate::shelf::ShelfItem;
use crate::tasks::{TaskState, TaskUpdate};
use hytte_proto::ports::PortInfo;
use hytte_proto::TaskEvent;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod actions;
mod agents;
mod anim;
mod cards;
mod media;
mod notes;
mod scene;
mod script;
mod shelf;
mod tasks;
#[cfg(test)]
mod tests;
mod timer;
mod weather;

pub use actions::*;
pub use agents::*;
pub use anim::*;
pub use cards::*;
pub use media::*;
pub use notes::*;
pub use scene::*;
pub use script::*;
pub use shelf::*;
pub use tasks::*;
// Private to this module tree: the weather glow helper `scene` uses.
use weather::*;

pub const SUCCESS_HOLD: Duration = Duration::from_secs(3);

pub const LOST_HOLD: Duration = Duration::from_secs(5);

/// A running download whose file hasn't changed for this long is dropped from the list (its
/// browser crashed or it was abandoned); it comes back if the file starts changing again.
pub const DOWNLOAD_STALE: Duration = Duration::from_secs(5 * 60);

/// No keyboard or mouse input for this long: the user is away, finished rows wait for them.
pub const AWAY_AFTER: Duration = Duration::from_secs(60);

/// While away with a finished row held, check this often whether the user is back.
pub const AWAY_POLL: Duration = Duration::from_secs(5);

pub const CHIP_HOLD: Duration = Duration::from_secs(10);

/// Width the red mute lock takes from the pill's right edge.
pub const LOCK_W: f32 = 26.0;

pub const KILL_CONFIRM: Duration = Duration::from_secs(3);

/// How long the timer-finished card (and its alarm) waits before folding away on its own.
pub const TIMER_DONE_HOLD: Duration = Duration::from_secs(120);

/// Keep-awake for a while: how long the first click on the tile holds the PC up.
pub const AWAKE_FOR: Duration = Duration::from_secs(3600);

/// How long a failed task's red pulse takes to fade to rest.
pub const FAIL_PULSE: Duration = Duration::from_secs(8);

/// Height of one dev-server row in the Tasks pane.
pub const SERVER_ROW: f32 = 36.0;

/// Height of the output ticker line under a running task in the expanded list.
pub const LINE_H: f32 = 14.0;

/// Height of the tab strip under a pane that has neighbours.
pub const TAB_H: f32 = 24.0;

/// One app's row in the per-app volume list.
pub const APP_ROW_H: f32 = 28.0;

/// Shelf tiles in view at once; the row scrolls past that.
pub const SHELF_VISIBLE: usize = 5;

/// How long the plug-in card stays after the charger is connected.
pub const CHARGE_HOLD: Duration = Duration::from_secs(3);

/// Events marshalled from worker threads to the UI thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Task(TaskUpdate),
    /// Place search results for the query typed when search number `0` was current.
    Places(u64, String, Result<Vec<crate::weather::Place>, String>),
    /// The same for the wider (OpenStreetMap) search the user asked for.
    WidePlaces(u64, String, Result<Vec<crate::weather::Place>, String>),
    /// A forecast arrived (or the fetch failed).
    Forecast(Result<crate::weather::Forecast, String>),
    /// Lyrics for the song with this key (or the lookup failed).
    Lyrics(String, Result<crate::lyrics::Lyrics, String>),
    Media(Option<MediaInfo>),
    MediaArt(Option<Arc<ArtBitmap>>),
    Privacy(bool, bool, Option<String>),
    MicMute(bool),
    /// The default speaker: (volume 0..1, muted); None without an output device.
    Speaker(Option<(f32, bool)>),
    Power(Option<crate::power::Battery>),
    DropDone(crate::drop::DropResult),
    /// Listening ports, and each port's process with its ancestors (parent first).
    Ports(Vec<PortInfo>, std::collections::HashMap<u32, Vec<u32>>),
    /// A process has been using a lot of CPU or memory.
    Hog(crate::hog::Alert),
    /// A browser download finished: its name and where it landed.
    DownloadDone {
        name: String,
        path: PathBuf,
    },
    /// The next calendar event (None = nothing upcoming).
    Calendar(Option<crate::calendar::NextEvent>),
    /// Take a task out of the list without a trace (a download that was cancelled).
    TaskGone(String),
    /// Dropped onto the notch while the shelf is the drop target.
    ShelfAdd(crate::drop::DropJob),
    /// Drop items whose file work (copies, snippets) finished on a worker thread.
    ShelfStaged(Vec<ShelfItem>),
    /// A Snipping Tool screenshot saved as a PNG: shelved, and tracked for its short stay.
    Screenshot(ShelfItem),
    Thumb(u64, Option<Arc<ArtBitmap>>),
    SetClipboard(String),
    DragEnter,
    DragLeave,
    /// `config.toml` was edited and parses.
    Config(Box<crate::config::Config>),
    /// What is wrong with `config.toml` (a mistake in it, or a setting that doesn't exist).
    ConfigProblem(String),
    /// An agent asks permission (`hytte agent ask`); send the answer on `reply`.
    Ask {
        id: u64,
        summary: String,
        reply: crossbeam_channel::Sender<bool>,
    },
    /// A script asked for a card or a shelf item (`hytte notify`, `hytte card`, `hytte shelf
    /// add`). A `card --wait` comes with where to send the pressed button's label.
    Cmd(hytte_proto::Cmd, Option<crossbeam_channel::Sender<String>>),
    /// A script pane's command finished: its name and how it went.
    Script(String, crate::scriptpane::Run),
    /// An `[[action]]` finished: its name and how it went.
    ActionDone(String, crate::scriptpane::Run),
    /// Files copied in Explorer, to go on the shelf by reference (`[shelf] shelve_copied`).
    ShelfCopied(Vec<PathBuf>),
    /// A Bluetooth device connected: its name and battery percentage when it has one.
    Bluetooth {
        name: String,
        battery: Option<u8>,
    },
    /// The pane packs on disk changed (or were read for the first time).
    Packs(Vec<crate::packs::Pack>),
    /// A newer version exists: its tag and the first line of its notes.
    UpdateOffer {
        tag: String,
        note: String,
    },
    /// A line of news for the user (an update check's result, an install error).
    Notice(String, Tone),
}

/// Keep the PC (and its screen) awake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Awake {
    #[default]
    Off,
    Until(Instant),
    On,
}

impl Awake {
    pub fn active(self, now: Instant) -> bool {
        match self {
            Awake::Off => false,
            Awake::Until(t) => now < t,
            Awake::On => true,
        }
    }
    /// The tile cycles off -> an hour -> until turned off -> off.
    pub fn next(self, now: Instant) -> Self {
        match self {
            Awake::Off => Awake::Until(now + AWAKE_FOR),
            Awake::Until(_) => Awake::On,
            Awake::On => Awake::Off,
        }
    }
}

#[derive(Default)]
pub struct Model {
    pub tasks: Vec<TaskView>,
    pub media: Option<MediaInfo>,
    pub media_stamp: Option<Instant>,
    pub art: Option<Arc<ArtBitmap>>,
    pub art_gen: u32,
    /// Dominant colour of the album art (see [`accent_of`]); tints the media glow.
    pub art_accent: Option<[f32; 3]>,
    /// `[general] adaptive_glow`: let the glow follow the album art.
    pub adaptive_glow: bool,
    /// `[general] finish_peek_secs`: how long an agent's reply card stays; zero = no card.
    pub finish_peek: Duration,
    /// `[general] glow_strength`: multiplies the border glow.
    pub glow_gain: f32,
    /// `[general] pill_opacity`: alpha of the pill body.
    pub body_alpha: f32,
    pub cam: bool,
    pub mic: bool,
    pub privacy_app: Option<String>,
    pub mic_muted: bool,
    /// The default speaker: (volume 0..1, muted).
    pub speaker: Option<(f32, bool)>,
    pub awake: Awake,
    /// None on machines without a battery.
    pub power: Option<crate::power::Battery>,
    /// When the charger was last plugged in or out; the card shows for [`CHARGE_HOLD`].
    pub charge_since: Option<Instant>,
    /// Which way the last change went: true plugged in, false unplugged.
    pub charge_in: bool,
    /// A user-initiated drop keeps the pill visible even while fullscreen suppresses it.
    pub force_until: Option<Instant>,
    pub drop_over: bool,
    pub chip: Option<Chip>,
    /// Script panes (`[[script_pane]]`), by slot.
    pub scripts: Vec<ScriptView>,
    /// Live cards (`hytte card`) waiting for the chip, oldest first.
    // ponytail: a chip set directly elsewhere (a download, a notice) replaces a card on it
    // without requeueing; a progress card comes back with its next update. Route every chip
    // through one setter if that ever loses a waited-on card in practice.
    pub cards: std::collections::VecDeque<Chip>,
    /// Numbers live cards sent without an id.
    pub card_seq: u64,
    /// A chip's Kill button was clicked once and waits for a second click until then.
    pub chip_armed: Option<Instant>,
    /// `[clipboard] history`: the text clips (newest first), whether it is on, and the first
    /// one in view. Memory only.
    pub clips: crate::clipboard::History,
    pub clips_on: bool,
    pub clip_off: usize,
    /// Names of the `[[action]]`s, so a card button `Label:name` can tell an action from text.
    pub action_names: Vec<String>,
    /// An action whose first click is in: it runs on the next one before the instant.
    pub armed_action: Option<(String, Instant)>,
    /// Actions running now (one at a time per name).
    pub running_actions: Vec<String>,
    /// Exe names (lower-cased) the resource-hog alert is muted for, for this session.
    pub hog_ignore: std::collections::HashSet<String>,
    /// The next calendar event and when its heads-up is due (None once shown or if none).
    pub calendar: Option<(crate::calendar::NextEvent, Instant)>,
    /// (subject, start) of the event whose heads-up was shown, so it isn't repeated.
    pub calendar_seen: Option<(String, i64)>,
    pub ports: Vec<PortInfo>,
    /// Each port's pid -> its ancestors, to find the terminal task that started it.
    pub port_parents: std::collections::HashMap<u32, Vec<u32>>,
    /// Port whose Kill button is armed ("Kill?") and until when.
    pub kill_armed: Option<(u16, Instant)>,
    /// The first task row shown: the list scrolls (mouse wheel) when more than fit.
    pub task_first: usize,
    /// `[general] pin_button`: the open pill shows a pin.
    pub pin_button: bool,
    /// The pill is pinned open (the pin was clicked); it ignores the collapse timer.
    pub pinned: bool,
    /// `[general] task_rows`: how many rows show before the list scrolls (0 = the default, 4).
    pub task_rows: usize,
    pub shelf: Vec<ShelfItem>,
    pub shelf_sel: Option<u64>,
    /// First shelf tile in view when there are more than `SHELF_VISIBLE`.
    pub shelf_off: usize,
    pub selected: Option<Panel>,
    pub peek_until: Option<Instant>,
    /// Last time the model was advanced; visibility rules compare against it.
    pub now: Option<Instant>,
    /// Lower-cased command names the daemon refuses to show (shell hooks).
    pub ignore: Vec<String>,
    pub timer: Option<crate::timer::Timer>,
    /// Right-click panel (set or inspect the timer).
    pub timer_panel: bool,
    /// Minutes the panel will start with (wheel / presets / typed digits).
    pub timer_min: u32,
    /// When the last minutes digit was typed: the next one within a moment extends it.
    pub timer_typed: Option<Instant>,
    pub timer_done: Option<(crate::timer::TimerKind, Instant)>,
    /// The timer that just finished (its length and when it ended), for the finished card.
    pub timer_last: Option<crate::timer::Timer>,
    /// Length of the break offered after a finished focus session.
    pub timer_break: u32,
    /// Focus sessions finished, for long-break spacing.
    pub focus_done: u32,
    /// `[timer] rounds`: how many dots a focus cycle has.
    pub rounds: u32,
    /// The per-app volume list under the volume bar is open, and what it shows.
    pub apps_open: bool,
    pub apps: Vec<crate::mic::AppVol>,
    /// No input for [`AWAY_AFTER`]: finished rows are held until the user is back.
    pub away: bool,
    /// The last input after an away spell; held rows count their hold time from here.
    pub back: Option<Instant>,
    /// `[panes]`: which of Notes, To-do and Weather show in the tab strip.
    pub panes: crate::config::Panes,
    /// Notes and to-dos; `jot_dirty` asks the window layer to save them.
    pub jot: crate::jot::Store,
    pub jot_dirty: bool,
    /// The text field that has keyboard focus, if any.
    pub typing: Option<Field>,
    pub note_edit: crate::textedit::TextEdit,
    pub todo_edit: crate::textedit::TextEdit,
    pub place_edit: crate::textedit::TextEdit,
    /// The Notes field's switch: keep the note for good (false = 24 h).
    pub note_keep: bool,
    /// An existing note or to-do being edited in the field (instead of adding a new one).
    pub jot_edit: Option<u64>,
    /// The last removed item, and when: Undo puts it back until it expires.
    pub jot_undo: Option<(crate::jot::Jot, Instant)>,
    /// First list row in view in Notes / To-do when there are more than `JOT_ROWS`.
    pub jot_first: usize,
    /// The chosen place, and what is known about its weather.
    pub weather_place: Option<crate::weather::Place>,
    pub forecast: Option<crate::weather::Forecast>,
    pub weather_busy: bool,
    /// The last forecast fetch failed.
    pub weather_err: bool,
    /// Don't fetch again before this (after a failure).
    pub weather_retry: Option<Instant>,
    /// The place chooser is showing (always, while no place is chosen).
    pub choosing: bool,
    pub places: Vec<crate::weather::Place>,
    pub place_sel: usize,
    pub places_busy: bool,
    pub places_err: bool,
    /// The query `places` answers.
    pub places_for: String,
    /// Open-Meteo found nothing for a while: the "search more widely" row is offered.
    pub wide_offer: bool,
    pub wide_busy: bool,
    pub wide_err: bool,
    /// The query the wider search answered (even with nothing).
    pub wide_for: String,
    /// `places` came from the wider search (its source is credited under the list).
    pub places_wide: bool,
    /// Wider-search answers this session, so the same query is never asked twice.
    pub wide_cache: Vec<(String, Vec<crate::weather::Place>)>,
    /// Show Fahrenheit.
    pub imperial: bool,
    /// Keyboard mode is on (the hotkey, or typing in a field): the open pill shows key hints.
    pub keyboard: bool,
    /// `[general] skin`: the pill's colours.
    pub skin: crate::skin::Skin,
    /// The sleep timer ("stop playing in 30 min"), and whether its chips are showing.
    pub sleep: Option<crate::sleep::Sleep>,
    pub sleep_open: bool,
    /// The lyrics page is showing in the Media pane; `lyrics_on` is `[media] lyrics`.
    pub lyrics_open: bool,
    pub lyrics_on: bool,
    /// `[media] lyrics_live` is off: timed lyrics show as plain words to scroll.
    pub lyrics_still: bool,
    /// The song the lyrics below are for (or being fetched for), and what is known.
    pub lyric_key: String,
    pub lyric_state: LyricState,
    /// The last few songs' lyrics, newest last. In memory only.
    pub lyric_cache: Vec<(String, crate::lyrics::Lyrics)>,
    /// First line in view of plain (untimed) lyrics.
    pub lyric_first: usize,
    /// Alerts held back during a Focus session, and what was said about them when it ended.
    pub held: Held,
    pub focus_summary: Option<String>,
    pub nudge: Option<Nudge>,
}

/// A break reminder on screen.
#[derive(Debug, Clone, Copy)]
pub struct Nudge {
    pub kind: crate::breaks::Kind,
    pub since: Instant,
}

impl Model {
    /// New battery reading. Switching between battery and AC starts the plug-in or unplug card.
    pub fn set_power(&mut self, p: Option<crate::power::Battery>, now: Instant) {
        if let (Some(old), Some(new)) = (self.power, p) {
            if old.plugged != new.plugged {
                self.charge_since = Some(now);
                self.charge_in = new.plugged;
            }
        }
        self.power = p;
    }

    pub fn peek(&mut self, panel: Panel, until: Instant) {
        // A pane the user switched off stays shut: it would open on an unrelated one.
        if !self.panels().contains(&panel) {
            return;
        }
        self.selected = Some(panel);
        self.peek_until = Some(until);
    }

    /// Feed the time since the last keyboard / mouse input (anywhere on the PC).
    pub fn set_idle(&mut self, idle: Duration, now: Instant) {
        let away = idle >= AWAY_AFTER;
        if self.away && !away {
            self.back = now.checked_sub(idle);
        }
        self.away = away;
    }

    /// Drop finished tasks / chips past their hold time. Returns the next
    /// instant something changes, so the caller can arm a single timer.
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.now = Some(now);
        // A row that finished while the user was away starts its hold when they come back.
        let (away, back) = (self.away, self.back);
        let from = move |c: Instant| back.map_or(c, |b| b.max(c));
        if self.shell_count() <= self.row_cap() {
            self.task_first = 0;
        }
        self.tasks.retain(|t| match t.event {
            TaskEvent::Done => away || now.duration_since(from(t.changed)) < SUCCESS_HOLD,
            TaskEvent::Lost => away || now.duration_since(from(t.changed)) < LOST_HOLD,
            _ if t.source == "download" && t.running() => {
                now.duration_since(t.changed) < DOWNLOAD_STALE
            }
            _ => true, // running, waiting and failed persist
        });
        if self
            .chip
            .as_ref()
            .is_some_and(|c| now.duration_since(c.since) >= c.hold)
        {
            self.chip = None;
        }
        if self.chip.is_none() || self.chip_armed.is_some_and(|t| now >= t) {
            self.chip_armed = None;
        }
        let waiting = self.expire_waiting(now);
        if self.peek_until.is_some_and(|p| now >= p) {
            self.peek_until = None;
        }
        if self.armed_action.as_ref().is_some_and(|(_, t)| now >= *t) {
            self.armed_action = None;
        }
        if self.kill_armed.is_some_and(|(_, t)| now >= t) {
            self.kill_armed = None;
        }
        if self.force_until.is_some_and(|f| now >= f) {
            self.force_until = None;
        }
        if self.charge_since.is_some_and(|s| now >= s + CHARGE_HOLD) {
            self.charge_since = None;
        }
        if !self.awake.active(now) {
            self.awake = Awake::Off;
        }
        if self
            .timer_done
            .is_some_and(|(_, s)| now.duration_since(s) >= TIMER_DONE_HOLD)
        {
            self.timer_done = None;
        }
        if self.timer_done.is_none() {
            self.focus_summary = None;
        }
        if self.nudge.is_some_and(|n| now >= n.since + n.kind.hold()) {
            self.nudge = None;
        }
        let wall = crate::timer::now_ms();
        if self.jot.expire(wall) {
            self.jot_dirty = true;
        }
        if self
            .jot_undo
            .as_ref()
            .is_some_and(|(_, t)| now.duration_since(*t) >= UNDO_HOLD)
        {
            self.jot_undo = None;
        }
        let mut next: Option<Instant> = None;
        let mut keep = |i: Instant| next = Some(next.map_or(i, |n| n.min(i)));
        if let Some(d) = self.jot.next_deadline() {
            keep(now + Duration::from_millis(d.saturating_sub(wall)));
        }
        if let Some((_, t)) = &self.jot_undo {
            keep(*t + UNDO_HOLD);
        }
        if let Some(n) = self.nudge {
            keep(n.since + n.kind.hold());
        }
        if let Some(s) = &self.sleep {
            for d in s.deadlines(now) {
                keep(d);
            }
        }
        for t in &self.tasks {
            match t.event {
                TaskEvent::Done | TaskEvent::Lost if away => keep(now + AWAY_POLL),
                TaskEvent::Done => keep(from(t.changed) + SUCCESS_HOLD),
                TaskEvent::Lost => keep(from(t.changed) + LOST_HOLD),
                _ if t.source == "download" && t.running() => keep(t.changed + DOWNLOAD_STALE),
                _ => {}
            }
            if t.visible_after > now {
                keep(t.visible_after);
            }
        }
        if let Some(c) = &self.chip {
            keep(c.since + c.hold);
        }
        if let Some(d) = waiting {
            keep(d);
        }
        if let Some((_, t)) = &self.armed_action {
            keep(*t);
        }
        if let Some(t) = self.chip_armed {
            keep(t);
        }
        if let Some((_, at)) = &self.calendar {
            keep(*at);
        }
        if let Some(p) = self.peek_until {
            keep(p);
        }
        if let Some((_, t)) = self.kill_armed {
            keep(t);
        }
        if let Some(f) = self.force_until {
            keep(f);
        }
        if let Some(s) = self.charge_since {
            keep(s + CHARGE_HOLD);
        }
        if let Some((_, s)) = self.timer_done {
            keep(s + TIMER_DONE_HOLD);
        }
        if let Awake::Until(t) = self.awake {
            keep(t);
        }
        next
    }

    /// A Focus session is running (paused or not): background alerts wait for its end.
    pub fn holding(&self) -> bool {
        self.timer
            .as_ref()
            .is_some_and(|t| t.kind == crate::timer::TimerKind::Focus)
    }

    /// Apply `[panes]`; a hidden pane that was selected falls back to Home (via `panel()`).
    pub fn apply_panes(&mut self, p: &crate::config::Panes) {
        self.panes = p.clone();
        if self.typing.is_some() && !self.panels().contains(&self.panel()) {
            self.typing = None;
        }
    }

    pub fn privacy_active(&self) -> bool {
        self.cam || self.mic
    }
}
