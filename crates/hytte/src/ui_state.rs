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
    /// A script asked for a card or a shelf item (`hytte notify`, `hytte shelf add`).
    Cmd(hytte_proto::Cmd),
    /// A newer version exists: its tag and the first line of its notes.
    UpdateOffer {
        tag: String,
        note: String,
    },
    /// A line of news for the user (an update check's result, an install error).
    Notice(String, Tone),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    /// Source app's AppUserModelId, used to bring it to the front.
    pub app: String,
    pub pos_ms: u64,
    pub dur_ms: u64,
}

/// The colour that best represents a picture: the average of its pixels weighted by how
/// saturated they are, brightened to a glow-worthy level. None for greyscale art.
pub fn accent_of(bgra: &[u8]) -> Option<[f32; 3]> {
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for px in bgra.as_chunks::<4>().0 {
        let a = px[3] as f32 / 255.0;
        if a < 0.5 {
            continue;
        }
        // Premultiplied -> straight, 0..1.
        let (b, g, r) = (
            px[0] as f32 / 255.0 / a,
            px[1] as f32 / 255.0 / a,
            px[2] as f32 / 255.0 / a,
        );
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        // Near-black and washed-out pixels carry no hue worth glowing.
        let w = sat * sat * max;
        sum[0] += r * w;
        sum[1] += g * w;
        sum[2] += b * w;
        weight += w;
    }
    if weight < 1.0 {
        return None;
    }
    let c = [sum[0] / weight, sum[1] / weight, sum[2] / weight];
    let top = c[0].max(c[1]).max(c[2]).max(0.01);
    Some([c[0] / top * 0.95, c[1] / top * 0.95, c[2] / top * 0.95])
}

/// Premultiplied BGRA, ready for Direct2D.
#[derive(Debug)]
pub struct ArtBitmap {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: String,
    pub label: String,
    pub event: TaskEvent,
    pub progress: Option<u8>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub stderr: Vec<String>,
    pub pid: Option<u32>,
    /// Agent message while `NeedsInput`.
    pub attention: Option<String>,
    /// The command's latest output line, shown under a running task while it changes.
    pub line: Option<String>,
    /// Who reported it ("run", "shell", "agent") and the folder it ran in: what Rerun needs.
    pub source: String,
    pub cwd: Option<String>,
    pub started: Instant,
    pub changed: Instant,
    /// Hidden until this instant (shell hooks: fast commands never show).
    pub visible_after: Instant,
}

impl TaskView {
    pub fn running(&self) -> bool {
        matches!(
            self.event,
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed
        )
    }
    /// A terminal command that can be run again in a new window (not an agent or download).
    pub fn rerunnable(&self) -> bool {
        matches!(self.source.as_str(), "run" | "shell")
            && self.cwd.is_some()
            && !self.label.ends_with('…') // cut off, so not the whole command
    }

    pub fn failed(&self) -> bool {
        self.event == TaskEvent::Failed
    }
    pub fn needs_input(&self) -> bool {
        self.event == TaskEvent::NeedsInput
    }
    /// A running task with output to show gets a ticker line in the expanded list.
    pub fn shows_line(&self) -> bool {
        self.running() && self.line.is_some()
    }
    /// Strength (1..0) of the red failure pulse: it fades out over [`FAIL_PULSE`]
    /// and then rests, so a failed task left on screen costs no frames.
    pub fn fail_pulse(&self, now: Instant) -> f32 {
        let since = now.saturating_duration_since(self.changed).as_secs_f32();
        (1.0 - since / FAIL_PULSE.as_secs_f32()).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Tasks,
    Shelf,
    Media,
    Timer,
    Power,
    Home,
    Notes,
    Todo,
    Weather,
}

impl Panel {
    pub fn name(self) -> &'static str {
        match self {
            Panel::Tasks => "Tasks",
            Panel::Shelf => "Shelf",
            Panel::Media => "Media",
            Panel::Timer => "Timer",
            Panel::Power => "Power",
            Panel::Home => "Home",
            Panel::Notes => "Notes",
            Panel::Todo => "To-do",
            Panel::Weather => "Weather",
        }
    }

    pub fn from_name(s: &str) -> Option<Panel> {
        [
            Panel::Tasks,
            Panel::Shelf,
            Panel::Media,
            Panel::Timer,
            Panel::Power,
            Panel::Home,
            Panel::Notes,
            Panel::Todo,
            Panel::Weather,
        ]
        .into_iter()
        .find(|p| p.name() == s.trim())
    }

    /// The panel an expanded scene shows; None for scenes that aren't panels.
    pub fn of(scene: Scene) -> Option<Panel> {
        Some(match scene {
            Scene::ExpTasks => Panel::Tasks,
            Scene::ExpShelf => Panel::Shelf,
            Scene::ExpMedia => Panel::Media,
            Scene::ExpTimer => Panel::Timer,
            Scene::ExpPower => Panel::Power,
            Scene::ExpHome => Panel::Home,
            Scene::ExpNotes => Panel::Notes,
            Scene::ExpTodo => Panel::Todo,
            Scene::ExpWeather => Panel::Weather,
            _ => return None,
        })
    }
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
}

#[derive(Debug, Clone)]
pub struct Chip {
    pub summary: String,
    pub open: Option<PathBuf>,
    pub copy: Option<String>,
    pub since: Instant,
    /// Buttons beyond Open / Copy / Dismiss, in order.
    pub extra: Vec<(String, ChipAction)>,
    /// How long it stays before folding away on its own.
    pub hold: Duration,
    pub tone: Tone,
}

impl Chip {
    /// Short enough for one line of the card, which then needs less height.
    pub fn one_line(&self) -> bool {
        self.summary.chars().count() < 44 && !self.summary.contains('\n')
    }

    pub fn new(summary: impl Into<String>, since: Instant) -> Self {
        Self {
            summary: summary.into(),
            open: None,
            copy: None,
            since,
            extra: vec![],
            hold: CHIP_HOLD,
            tone: Tone::Done,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Sentinel,
    Idle,
    CompactTask,
    CompactMedia,
    ExpTasks,
    ExpMedia,
    ExpShelf,
    ExpHome,
    ExpPower,
    /// A break reminder (eyes, stretch, water).
    ExpNudge,
    ExpNotes,
    ExpTodo,
    ExpWeather,
    ExpDrop,
    ExpChip,
    ExpTimer,
    ExpTimerDone,
    /// The charger was just plugged in.
    ExpCharge,
    /// Settings (from the gear on Home). Not a pane: no tab, not in the swipe rotation.
    ExpSettings,
}

/// A second, smaller pill that splits off beside a compact one when two live activities
/// compete for the same collapsed space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bubble {
    /// The countdown timer, as a clock.
    Timer,
    /// The music that is playing, as its album art.
    Media,
}

impl Bubble {
    pub fn width(self) -> f32 {
        match self {
            Bubble::Timer => 58.0,
            Bubble::Media => 34.0,
        }
    }

    /// The pane that opens when the pointer rests on this bubble.
    pub fn panel(self) -> Panel {
        match self {
            Bubble::Timer => Panel::Timer,
            Bubble::Media => Panel::Media,
        }
    }

    /// Left edge of the bubble in pill-local px, for a pill `pw` wide with the bubble `s` of
    /// the way out (0 behind the pill, 1 fully out). The renderer draws it here and the
    /// window hit-tests it here.
    pub fn x(self, pw: f32, s: f32) -> f32 {
        let bw = self.width();
        (pw - bw - 6.0) + (bw + 14.0) * s
    }
}

/// Logical px to the right of the pill that the bubble can occupy (gap + widest bubble).
pub const BUBBLE_ROOM: f32 = 72.0;

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
    /// A chip's Kill button was clicked once and waits for a second click until then.
    pub chip_armed: Option<Instant>,
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
    /// The settings pane is open, on this group (see `SETTINGS_GROUPS`).
    pub settings: Option<u8>,
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
    /// Show Fahrenheit.
    pub imperial: bool,
    /// `[general] skin`: the pill's colours.
    pub skin: crate::skin::Skin,
    /// The sleep timer ("stop playing in 30 min"), and whether its chips are showing.
    pub sleep: Option<crate::sleep::Sleep>,
    pub sleep_open: bool,
    /// The lyrics page is showing in the Media pane; `lyrics_on` is `[media] lyrics`.
    pub lyrics_open: bool,
    pub lyrics_on: bool,
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

/// How long to wait after a failed forecast fetch before trying again.
pub const WEATHER_RETRY: Duration = Duration::from_secs(60);

/// The glow that follows the sky: warm for a sunny day, indigo for a clear night, blue for rain.
fn sky_glow(h: crate::weather::Hour) -> ([f32; 3], f32) {
    use crate::weather::Sky;
    match (crate::weather::sky(h.code), h.day) {
        (Sky::Clear, true) => ([1.0, 0.80, 0.40], 0.30),
        (Sky::Clear, false) => ([0.55, 0.58, 1.0], 0.30),
        (Sky::Partly, true) => ([0.75, 0.85, 1.0], 0.22),
        (Sky::Partly, false) => ([0.55, 0.60, 0.90], 0.22),
        (Sky::Cloud, _) => ([0.70, 0.75, 0.85], 0.20),
        (Sky::Fog, _) => ([0.80, 0.80, 0.85], 0.18),
        (Sky::Rain, _) => ([0.35, 0.63, 1.0], 0.38),
        (Sky::Snow, _) => ([0.85, 0.92, 1.0], 0.35),
        (Sky::Thunder, _) => ([0.65, 0.54, 0.98], 0.45),
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

/// What the lyrics page knows about the playing song.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LyricState {
    #[default]
    Idle,
    Loading,
    Ready(crate::lyrics::Lyrics),
    Failed,
}

/// Songs whose lyrics are kept in memory.
const LYRIC_CACHE: usize = 20;

/// A break reminder on screen.
#[derive(Debug, Clone, Copy)]
pub struct Nudge {
    pub kind: crate::breaks::Kind,
    pub since: Instant,
}

/// A text field on the pill that can take keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Note,
    Todo,
    Place,
}

/// Rows of notes or to-dos shown before the list scrolls.
pub const JOT_ROWS: usize = 4;
/// How long Undo stays after a delete.
pub const UNDO_HOLD: Duration = Duration::from_secs(6);

/// The settings pane's groups, as its segmented header shows them.
pub const SETTINGS_GROUPS: [&str; 3] = ["Look", "Behaviour", "Features"];

/// First word of a command line, lower-cased, without path or `.exe`.
/// AI coding agents started from a shell. They report themselves through their hooks
/// (`hytte setup agents`), so the shell's "running for an hour" row would only duplicate them.
pub const AGENT_CLIS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "gemini",
    "agy",
    "cursor-agent",
];

pub fn command_name(label: &str) -> String {
    let first = label.split_whitespace().next().unwrap_or("");
    let base = first
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(first)
        .to_ascii_lowercase();
    base.strip_suffix(".exe").unwrap_or(&base).to_string()
}

/// Exit codes of a command the user stopped (Ctrl+C, closing the window), not one that broke:
/// SIGINT and SIGTERM as a shell reports them, and Windows' STATUS_CONTROL_C_EXIT.
fn is_interrupt(code: Option<i32>) -> bool {
    matches!(code, Some(130 | 143 | -1_073_741_510))
}

impl Model {
    pub fn apply_task(&mut self, u: TaskUpdate, now: Instant) {
        self.now = Some(now);
        match u {
            TaskUpdate::Upsert(s) => self.upsert(s, now),
            TaskUpdate::Gone(id) => self.tasks.retain(|t| !(t.id == id && t.failed())),
            TaskUpdate::Lost(id) => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    t.event = TaskEvent::Lost;
                    t.changed = now;
                }
            }
        }
    }

    fn upsert(&mut self, s: TaskState, now: Instant) {
        let shell = s.source == "shell";
        let terminal = matches!(s.event, TaskEvent::Done | TaskEvent::Failed);
        let name = command_name(&s.label);
        if shell && (self.ignore.contains(&name) || AGENT_CLIS.contains(&name.as_str())) {
            return;
        }
        // An agent's turn ended with a reply: its first line on a card for the "Open on
        // finish" time, Copy takes all of it. A pending ask or warning (amber) keeps the card.
        if s.source == "agent"
            && s.event == TaskEvent::Done
            && !self.finish_peek.is_zero()
            && !self.holding()
        {
            let msg = s.message.as_deref().map(str::trim).unwrap_or("");
            let first = msg
                .lines()
                .map(|l| {
                    l.trim()
                        .trim_start_matches(['#', '*', '-', ' '])
                        .trim_end_matches('*')
                })
                .find(|l| !l.is_empty());
            if let (Some(first), false) = (
                first,
                self.chip.as_ref().is_some_and(|c| c.tone == Tone::Warn),
            ) {
                let mut c = Chip::new(format!("{}: {first}", s.label), now);
                c.copy = Some(msg.to_string());
                c.hold = self.finish_peek;
                self.chip = Some(c);
            }
        }
        let existing = self.tasks.iter().position(|t| t.id == s.task_id);
        // A command the user stopped (Ctrl+C, window closed) is not an error: never shown.
        // A shell command that ended before the threshold is not shown either.
        if terminal {
            let hidden = shell
                && existing
                    .map(|i| now < self.tasks[i].visible_after)
                    .unwrap_or(true);
            if hidden || is_interrupt(s.exit_code) {
                if let Some(i) = existing {
                    self.tasks.remove(i);
                }
                return;
            }
        }
        let stderr: Vec<String> = s
            .stderr_tail
            .as_deref()
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        let attention = match s.event {
            TaskEvent::NeedsInput => Some(
                s.message
                    .clone()
                    .unwrap_or_else(|| "Needs your input".into()),
            ),
            _ => None,
        };
        if let Some(i) = existing {
            let t = &mut self.tasks[i];
            // A repeated "still waiting" keeps the wait's start, so its age shows.
            let same_wait = t.event == TaskEvent::NeedsInput && s.event == TaskEvent::NeedsInput;
            if !s.label.is_empty() {
                t.label = s.label;
            }
            t.event = s.event;
            t.progress = s.progress.or(t.progress);
            t.duration_ms = s.duration_ms;
            t.exit_code = s.exit_code;
            t.stderr = stderr;
            t.attention = attention;
            // A message without a line (progress, a retry) keeps the last one shown.
            t.line = s.line.or(t.line.take());
            t.pid = s.pid.or(t.pid);
            t.cwd = s.cwd.or(t.cwd.take());
            if !same_wait {
                t.changed = now;
            }
        } else {
            self.tasks.push(TaskView {
                id: s.task_id,
                label: s.label,
                event: s.event,
                progress: s.progress,
                duration_ms: s.duration_ms,
                exit_code: s.exit_code,
                stderr,
                pid: s.pid,
                attention,
                line: s.line,
                source: s.source,
                cwd: s.cwd,
                started: now,
                changed: now,
                visible_after: now + Duration::from_millis(s.delay_ms as u64),
            });
        }
    }

    pub fn apply_media(&mut self, m: Option<MediaInfo>, now: Instant) {
        if m.is_none() {
            self.art = None;
            self.art_accent = None;
            self.sleep_open = false;
            self.lyrics_open = false;
        }
        // A different song: its lyrics start over.
        let key = m.as_ref().map(|m| crate::lyrics::key(&m.title, &m.artist));
        if key.as_deref() != Some(self.lyric_key.as_str()) {
            self.lyric_key.clear();
            self.lyric_state = LyricState::Idle;
            self.lyric_first = 0;
        }
        self.media = m;
        self.media_stamp = Some(now);
        // An "end of track" sleep timer follows the song.
        if let Some(s) = self.sleep.as_mut() {
            s.retime(self.media.as_ref(), now, now);
        }
    }

    /// Set the sleep timer (None cancels it).
    pub fn set_sleep(&mut self, kind: Option<crate::sleep::Kind>, now: Instant) {
        self.sleep =
            kind.map(|k| crate::sleep::Sleep::new(k, self.media.as_ref(), self.media_stamp, now));
        self.sleep_open = false;
    }

    /// Lyrics to fetch now, as (key, title, artist, album, length ms): the page is open, lyrics
    /// are on, a song plays and nothing is known about it yet. Cached songs are answered here.
    pub fn lyrics_request(&mut self) -> Option<(String, String, String, String, u64)> {
        if !(self.lyrics_open && self.lyrics_on) {
            return None;
        }
        let m = self.media.as_ref().filter(|m| !m.title.is_empty())?;
        let key = crate::lyrics::key(&m.title, &m.artist);
        if key == self.lyric_key {
            return None;
        }
        self.lyric_key = key.clone();
        self.lyric_first = 0;
        if let Some((_, l)) = self.lyric_cache.iter().find(|(k, _)| *k == key) {
            self.lyric_state = LyricState::Ready(l.clone());
            return None;
        }
        self.lyric_state = LyricState::Loading;
        Some((
            key,
            m.title.clone(),
            m.artist.clone(),
            m.album.clone(),
            m.dur_ms,
        ))
    }

    /// A lookup finished. A late answer for an earlier song is kept for later, not shown.
    pub fn lyrics_result(&mut self, key: String, r: Result<crate::lyrics::Lyrics, String>) {
        match r {
            Ok(l) => {
                self.lyric_cache.retain(|(k, _)| *k != key);
                self.lyric_cache.push((key.clone(), l.clone()));
                if self.lyric_cache.len() > LYRIC_CACHE {
                    self.lyric_cache.remove(0);
                }
                if key == self.lyric_key {
                    self.lyric_state = LyricState::Ready(l);
                }
            }
            Err(_) if key == self.lyric_key => self.lyric_state = LyricState::Failed,
            Err(_) => {}
        }
    }

    /// Scroll plain lyrics by `by` lines; false when there is nothing to scroll (or at an end).
    pub fn scroll_lyrics(&mut self, by: i32) -> bool {
        let LyricState::Ready(crate::lyrics::Lyrics::Plain(lines)) = &self.lyric_state else {
            return false;
        };
        let max = lines.len().saturating_sub(3) as i32;
        let first = self.lyric_first.min(max as usize) as i32;
        let next = (first + by).clamp(0, max);
        self.lyric_first = next as usize;
        next != first
    }

    /// The Media pane's sleep chips and lyrics page close (the pill collapsed, another pane).
    pub fn close_media_extras(&mut self) {
        self.sleep_open = false;
        self.lyrics_open = false;
    }

    pub fn set_art(&mut self, a: Option<Arc<ArtBitmap>>) {
        self.art_accent = a.as_ref().and_then(|b| accent_of(&b.bgra));
        self.art = a;
        self.art_gen = self.art_gen.wrapping_add(1);
    }

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

    pub fn set_ports(
        &mut self,
        p: Vec<PortInfo>,
        parents: std::collections::HashMap<u32, Vec<u32>>,
    ) {
        self.ports = p;
        self.port_parents = parents;
        if let Some((port, _)) = self.kill_armed {
            if !self.ports.iter().any(|x| x.port == port) {
                self.kill_armed = None;
            }
        }
    }

    pub fn peek(&mut self, panel: Panel, until: Instant) {
        self.selected = Some(panel);
        self.peek_until = Some(until);
    }

    fn visible(&self, t: &TaskView) -> bool {
        self.now.is_none_or(|n| n >= t.visible_after)
    }

    /// Height the open per-app volume list adds to the Home pane (one row says "no apps").
    pub fn apps_h(&self) -> f64 {
        if self.apps_open {
            4.0 + APP_ROW_H as f64 * self.apps.len().max(1) as f64
        } else {
            0.0
        }
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
        if self.visible_count() <= self.row_cap() {
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
        if self.peek_until.is_some_and(|p| now >= p) {
            self.peek_until = None;
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

    /// Load the saved notes and give the text fields their limits.
    pub fn init_jot(&mut self, path: &std::path::Path) {
        self.jot = crate::jot::Store::load(path, crate::timer::now_ms());
        self.note_edit = crate::textedit::TextEdit::new(crate::jot::MAX_CHARS);
        self.todo_edit = crate::textedit::TextEdit::new(crate::jot::MAX_CHARS);
        self.place_edit = crate::textedit::TextEdit::new(80);
        self.note_keep = false;
    }

    pub fn edit_mut(&mut self, f: Field) -> &mut crate::textedit::TextEdit {
        match f {
            Field::Note => &mut self.note_edit,
            Field::Todo => &mut self.todo_edit,
            Field::Place => &mut self.place_edit,
        }
    }

    pub fn edit_ref(&self, f: Field) -> &crate::textedit::TextEdit {
        match f {
            Field::Note => &self.note_edit,
            Field::Todo => &self.todo_edit,
            Field::Place => &self.place_edit,
        }
    }

    /// Enter in a Notes or To-do field: add the item, or save the edit of an existing one.
    /// Returns true when something was stored. The field clears when it was.
    pub fn submit_jot(&mut self, f: Field) -> bool {
        use crate::jot::Kind;
        let kind = match f {
            Field::Note => Kind::Note,
            Field::Todo => Kind::Todo,
            Field::Place => return false,
        };
        let text = self.edit_ref(f).text().to_string();
        let now = crate::timer::now_ms();
        let ok = match self.jot_edit {
            Some(id) if !text.trim().is_empty() => {
                self.jot.edit(id, &text);
                self.jot_edit = None;
                true
            }
            Some(_) => false,
            None => self.jot.add(kind, &text, self.note_keep, now).is_some(),
        };
        if ok {
            self.edit_mut(f).clear();
            self.jot_first = 0;
            self.jot_dirty = true;
        }
        ok
    }

    /// Delete an item, keeping it for Undo.
    pub fn delete_jot(&mut self, id: u64, now: Instant) {
        if let Some(j) = self.jot.remove(id) {
            self.jot_undo = Some((j, now));
            self.jot_dirty = true;
            if self.jot_edit == Some(id) {
                self.jot_edit = None;
            }
        }
    }

    pub fn undo_jot(&mut self) {
        if let Some((j, _)) = self.jot_undo.take() {
            self.jot.restore(j);
            self.jot_dirty = true;
        }
    }

    /// Start editing an existing item in its pane's field.
    pub fn begin_edit(&mut self, id: u64) {
        let Some(j) = self.jot.get(id) else { return };
        let (f, text) = match j.kind {
            crate::jot::Kind::Note => (Field::Note, j.text.clone()),
            crate::jot::Kind::Todo => (Field::Todo, j.text.clone()),
        };
        self.jot_edit = Some(id);
        self.edit_mut(f).set(&text);
        self.typing = Some(f);
    }

    /// The Weather pane shows its place chooser (no place yet, or changing it).
    pub fn choosing_place(&self) -> bool {
        self.choosing || self.weather_place.is_none()
    }

    /// Height of the Weather pane without the tab strip.
    pub fn weather_h(&self) -> f64 {
        if self.choosing_place() {
            let rows = self.places.len().max(1) as f64;
            38.0 + 34.0 + 4.0 + rows * 28.0 + 8.0
        } else if self.forecast.is_some() {
            148.0
        } else {
            120.0
        }
    }

    /// Should the forecast be fetched now? A place is chosen, nothing is in flight, the data is
    /// old (or missing) and a recent failure isn't being waited out.
    pub fn want_forecast(&self, wall_ms: u64, now: Instant) -> bool {
        self.weather_place.is_some()
            && !self.choosing_place()
            && !self.weather_busy
            && self.weather_retry.is_none_or(|t| now >= t)
            && self.forecast.as_ref().is_none_or(|f| !f.fresh(wall_ms))
    }

    /// A forecast fetch finished. One for a place that was changed since is dropped.
    pub fn forecast_result(&mut self, r: Result<crate::weather::Forecast, String>, now: Instant) {
        self.weather_busy = false;
        match r {
            Ok(f) if self.weather_place.as_ref() == Some(&f.place) => {
                self.forecast = Some(f);
                self.weather_err = false;
                self.weather_retry = None;
            }
            Ok(_) => {}
            Err(_) => {
                self.weather_err = true;
                self.weather_retry = Some(now + WEATHER_RETRY);
            }
        }
    }

    /// Search results. Ones for text that is no longer in the field are dropped.
    pub fn places_result(&mut self, query: String, r: Result<Vec<crate::weather::Place>, String>) {
        if self.place_edit.text().trim() != query {
            return;
        }
        self.places_busy = false;
        self.places_for = query;
        match r {
            Ok(p) => {
                self.places = p;
                self.places_err = false;
                self.place_sel = 0;
            }
            Err(_) => {
                self.places.clear();
                self.places_err = true;
            }
        }
    }

    /// Move the highlighted suggestion.
    pub fn place_move(&mut self, by: i32) {
        if !self.places.is_empty() {
            self.place_sel =
                (self.place_sel as i32 + by).clamp(0, self.places.len() as i32 - 1) as usize;
        }
    }

    /// Choose suggestion `i`: the chooser closes and the forecast starts over.
    pub fn pick_place(&mut self, i: usize) -> Option<crate::weather::Place> {
        let p = self.places.get(i)?.clone();
        self.weather_place = Some(p.clone());
        self.forecast = None;
        self.weather_err = false;
        self.weather_retry = None;
        self.close_chooser();
        Some(p)
    }

    fn close_chooser(&mut self) {
        self.choosing = false;
        self.places.clear();
        self.places_busy = false;
        self.places_err = false;
        self.place_edit.clear();
    }

    /// Open the chooser to change the place.
    pub fn start_choosing(&mut self) {
        self.choosing = true;
        self.close_chooser_fields();
    }

    fn close_chooser_fields(&mut self) {
        self.places.clear();
        self.places_busy = false;
        self.places_err = false;
        self.place_edit.clear();
    }

    /// Typing ended (Esc, clicked away): a half-made edit of an existing item is dropped, a
    /// draft for a new one stays.
    pub fn end_typing(&mut self) {
        if self.jot_edit.take().is_some() {
            if let Some(f) = self.typing {
                self.edit_mut(f).clear();
            }
        }
        // Leaving the place field with a place already chosen means "keep it".
        if self.typing == Some(Field::Place) && self.weather_place.is_some() {
            self.close_chooser();
        }
        self.typing = None;
    }

    /// The items of Notes or To-do as they list (top to bottom).
    pub fn jot_list(&self, scene: Scene) -> Vec<&crate::jot::Jot> {
        if scene == Scene::ExpTodo {
            self.jot.todos()
        } else {
            self.jot.notes()
        }
    }

    /// Rows in view: (first, count), with the scroll clamped.
    pub fn jot_window(&self, scene: Scene) -> (usize, usize) {
        let n = self.jot_list(scene).len();
        let first = self.jot_first.min(n.saturating_sub(JOT_ROWS));
        (first, (n - first).min(JOT_ROWS))
    }

    /// Height of the Notes / To-do pane without the tab strip: field, rows (one line of
    /// invitation when empty) and a little room below.
    pub fn jot_h(&self, scene: Scene) -> f64 {
        let (_, shown) = self.jot_window(scene);
        let undo = self
            .jot_undo
            .as_ref()
            .is_some_and(|(j, _)| (j.kind == crate::jot::Kind::Todo) == (scene == Scene::ExpTodo));
        let rows = (shown + undo as usize).max(1);
        12.0 + 34.0 + 8.0 + rows as f64 * 30.0 + 8.0
    }

    /// Move a due timer to the finished card. Returns its kind when it just fired.
    pub fn finish_timer(&mut self, wall_ms: u64, now: Instant) -> Option<crate::timer::TimerKind> {
        if !self
            .timer
            .as_ref()
            .is_some_and(|t| t.remaining_ms(wall_ms) == 0)
        {
            return None;
        }
        let t = self.timer.take()?;
        if t.kind == crate::timer::TimerKind::Focus {
            self.focus_done += 1;
        }
        self.timer_done = Some((t.kind, now));
        self.timer_last = Some(t.clone());
        Some(t.kind)
    }

    /// A new "next event". The heads-up is due `lead` before it starts, or at once if that
    /// moment has passed; an event whose heads-up was already shown isn't armed again.
    pub fn set_calendar(
        &mut self,
        ev: Option<crate::calendar::NextEvent>,
        wall_ms: i64,
        lead_ms: i64,
        now: Instant,
    ) {
        self.calendar = match ev {
            Some(e) if self.calendar_seen.as_ref() != Some(&(e.subject.clone(), e.start_ms)) => {
                let wait = (e.start_ms - lead_ms - wall_ms).max(0) as u64;
                Some((e, now + Duration::from_millis(wait)))
            }
            _ => None,
        };
    }

    /// The event whose heads-up is due now, once; it is then remembered as seen.
    pub fn calendar_due(&mut self, now: Instant) -> Option<crate::calendar::NextEvent> {
        let due = self.calendar.as_ref().is_some_and(|(_, at)| now >= *at);
        if !due {
            return None;
        }
        let (e, _) = self.calendar.take()?;
        self.calendar_seen = Some((e.subject.clone(), e.start_ms));
        Some(e)
    }

    /// Forget a task quietly, whatever its state.
    pub fn remove_task(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id);
    }

    pub fn dismiss_task(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id || t.running());
    }

    pub fn dismiss_chip(&mut self) {
        self.chip = None;
    }

    pub fn privacy_active(&self) -> bool {
        self.cam || self.mic
    }

    /// Row order: waiting-on-you first, then failed, running, finished; newest first within each.
    fn row_order(a: &TaskView, b: &TaskView) -> std::cmp::Ordering {
        let rank = |t: &TaskView| match t.event {
            TaskEvent::NeedsInput => 0,
            TaskEvent::Failed => 1,
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => 2,
            _ => 3,
        };
        rank(a).cmp(&rank(b)).then(b.changed.cmp(&a.changed))
    }

    /// Visible tasks: waiting-on-you first, then failed, running, finished.
    /// Task rows that fit before the list scrolls.
    fn row_cap(&self) -> usize {
        if self.task_rows == 0 {
            4
        } else {
            self.task_rows.clamp(2, 8)
        }
    }

    pub fn rows(&self) -> Vec<&TaskView> {
        let mut v: Vec<&TaskView> = self.tasks.iter().filter(|t| self.visible(t)).collect();
        v.sort_by(|a, b| Self::row_order(a, b));
        let first = self.task_first.min(v.len().saturating_sub(self.row_cap()));
        v.drain(..first);
        v.truncate(self.row_cap());
        v
    }

    /// Rows scrolled off above and below the ones shown.
    pub fn rows_hidden(&self) -> (usize, usize) {
        let n = self.visible_count();
        let first = self.task_first.min(n.saturating_sub(self.row_cap()));
        (first, n.saturating_sub(first + self.row_cap()))
    }

    /// Wheel over the task list: `by` rows down (negative = up). False when the list is
    /// already at that end, so the wheel can page the panes instead.
    pub fn scroll_tasks(&mut self, by: i32) -> bool {
        let max = self.visible_count().saturating_sub(self.row_cap()) as i32;
        let first = self.task_first.min(max as usize) as i32;
        let next = (first + by).clamp(0, max);
        self.task_first = next as usize;
        next != first
    }

    /// Scroll the Notes or To-do list by `by` rows; false when it didn't move (at an end, or
    /// nothing to scroll), so the wheel pages instead.
    pub fn scroll_jot(&mut self, by: i32) -> bool {
        let scene = if self.panel() == Panel::Todo {
            Scene::ExpTodo
        } else {
            Scene::ExpNotes
        };
        let max = self.jot_list(scene).len().saturating_sub(JOT_ROWS) as i32;
        let first = self.jot_first.min(max as usize) as i32;
        let next = (first + by).clamp(0, max);
        self.jot_first = next as usize;
        next != first
    }

    /// The task whose process started this server (it, or one of its ancestors, is the task's).
    pub fn task_for_port(&self, p: &PortInfo) -> Option<&TaskView> {
        let chain = self.port_parents.get(&p.pid);
        self.tasks.iter().find(|t| {
            t.running()
                && t.pid
                    .is_some_and(|tp| tp == p.pid || chain.is_some_and(|c| c.contains(&tp)))
        })
    }

    /// The terminal command a server can be restarted from: the one that started it, when it
    /// is tracked and can be run again whole (not cut off, not an agent or download).
    pub fn restart_task(&self, p: &PortInfo) -> Option<&TaskView> {
        self.task_for_port(p).filter(|t| t.rerunnable())
    }

    /// How many lines the selected shelf item's action chips take (1 when none is selected).
    pub fn shelf_chip_lines(&self) -> usize {
        let Some(it) = self
            .shelf_sel
            .and_then(|id| self.shelf.iter().find(|i| i.id == id))
        else {
            return 1;
        };
        let chips = crate::transforms::chips_for(&it.path);
        let labels: Vec<&str> = chips.iter().map(|c| c.0).collect();
        crate::transforms::chip_lines(&labels, 380.0 - 8.0).len()
    }

    pub fn visible_count(&self) -> usize {
        self.tasks.iter().filter(|t| self.visible(t)).count()
    }

    /// Height of the Servers section under the tasks (header + rows), 0 without ports.
    pub fn servers_h(&self) -> f64 {
        match self.ports.len().min(5) {
            0 => 0.0,
            n => 26.0 + n as f64 * SERVER_ROW as f64,
        }
    }

    /// The task shown in the compact pill.
    pub fn primary(&self) -> Option<&TaskView> {
        // Same pick as `rows()[0]` (min_by keeps the first of equals, like the stable sort),
        // without allocating: this runs several times per frame.
        self.tasks
            .iter()
            .filter(|t| self.visible(t))
            .min_by(|a, b| Self::row_order(a, b))
    }

    /// The second activity to show as a bubble next to a compact scene, if any: the timer
    /// first, then playing music beside a task. Expanded scenes show these as panels instead.
    pub fn bubble(&self, scene: Scene) -> Option<Bubble> {
        match scene {
            Scene::CompactTask | Scene::CompactMedia if self.timer.is_some() => Some(Bubble::Timer),
            Scene::CompactTask if self.media.as_ref().is_some_and(|m| m.playing) => {
                Some(Bubble::Media)
            }
            _ => None,
        }
    }

    /// Panels that currently have something to show, in priority order.
    /// Left to right: shelf, music, the quick controls (home), then what comes and goes (dev
    /// work, a running timer), and the battery at the right end.
    /// The shelf tiles in view, and where they start (clamped, so removing items never
    /// leaves the row scrolled past its end).
    pub fn shelf_page(&self) -> (usize, &[ShelfItem]) {
        let off = self
            .shelf_off
            .min(self.shelf.len().saturating_sub(SHELF_VISIBLE));
        let end = (off + SHELF_VISIBLE).min(self.shelf.len());
        (off, &self.shelf[off..end])
    }

    /// Scroll the shelf row by `by` tiles, staying within it.
    pub fn scroll_shelf(&mut self, by: i32) {
        let (off, _) = self.shelf_page();
        let max = self.shelf.len().saturating_sub(SHELF_VISIBLE) as i32;
        self.shelf_off = (off as i32 + by).clamp(0, max) as usize;
    }

    pub fn panels(&self) -> Vec<Panel> {
        let mut v = vec![];
        if !self.shelf.is_empty() {
            v.push(Panel::Shelf);
        }
        if self.media.is_some() {
            v.push(Panel::Media);
        }
        // Quick controls (mic, sound, keep awake, timer) are always there.
        v.push(Panel::Home);
        if self.panes.notes {
            v.push(Panel::Notes);
        }
        if self.panes.todo {
            v.push(Panel::Todo);
        }
        if self.panes.weather {
            v.push(Panel::Weather);
        }
        // Terminal tasks and the dev servers they run share one pane.
        if self.visible_count() > 0 || !self.ports.is_empty() {
            v.push(Panel::Tasks);
        }
        if self.timer.is_some() {
            v.push(Panel::Timer);
        }
        if self.power.is_some() {
            v.push(Panel::Power);
        }
        v
    }

    /// Move the selection by `dir` panels (wraps); used by the mouse wheel.
    pub fn step_panel(&mut self, dir: i32) -> Panel {
        let ps = self.panels();
        let i = ps.iter().position(|p| *p == self.panel()).unwrap_or(0) as i32;
        let p = ps[(i + dir).rem_euclid(ps.len() as i32) as usize];
        self.selected = Some(p);
        p
    }

    /// Drag-over or a fresh drop: the user is interacting, so ignore fullscreen suppression.
    pub fn forced(&self) -> bool {
        // A finished timer stays in view over fullscreen for as long as its card (and alarm) is up.
        // So does an agent's Allow / Deny: over a maximised terminal it would otherwise
        // wait unseen behind the thin line.
        self.drop_over
            || self.timer_done.is_some()
            || self.chip.as_ref().is_some_and(|c| {
                c.extra
                    .iter()
                    .any(|(_, a)| matches!(a, ChipAction::Answer(..)))
            })
            || self.now.zip(self.force_until).is_some_and(|(n, f)| n < f)
    }

    pub fn panel(&self) -> Panel {
        let ps = self.panels();
        match self.selected {
            Some(p) if ps.contains(&p) => p,
            // Unpicked: what the collapsed pill shows (a task, music), else the quick controls.
            _ if self.visible_count() > 0 => Panel::Tasks,
            _ if self.media.as_ref().is_some_and(|m| m.playing) => Panel::Media,
            _ => Panel::Home,
        }
    }

    pub fn tabs(&self) -> bool {
        self.panels().len() > 1
    }

    pub fn scene(&self, hover: bool, suppressed: bool) -> Scene {
        if suppressed && !hover && !self.forced() {
            return Scene::Sentinel;
        }
        if self.drop_over {
            return Scene::ExpDrop;
        }
        if self.chip.is_some() {
            return Scene::ExpChip;
        }
        if self.timer_done.is_some() {
            return Scene::ExpTimerDone;
        }
        if self.timer_panel {
            return Scene::ExpTimer;
        }
        if self.settings.is_some() {
            return Scene::ExpSettings;
        }
        if self.nudge.is_some() {
            return Scene::ExpNudge;
        }
        let peeking = self.now.zip(self.peek_until).is_some_and(|(n, p)| n < p);
        if hover || peeking {
            return match self.panel() {
                Panel::Tasks => Scene::ExpTasks,
                Panel::Shelf => Scene::ExpShelf,
                Panel::Media => Scene::ExpMedia,
                Panel::Timer => Scene::ExpTimer,
                Panel::Power => Scene::ExpPower,
                Panel::Home => Scene::ExpHome,
                Panel::Notes => Scene::ExpNotes,
                Panel::Todo => Scene::ExpTodo,
                Panel::Weather => Scene::ExpWeather,
            };
        }
        if self
            .now
            .zip(self.charge_since)
            .is_some_and(|(n, s)| n < s + CHARGE_HOLD)
        {
            return Scene::ExpCharge;
        }
        if self.visible_count() > 0 {
            Scene::CompactTask
        } else if self.media.as_ref().is_some_and(|m| m.playing) {
            Scene::CompactMedia
        } else {
            Scene::Idle
        }
    }

    /// Target pill size in logical px.
    pub fn size(&self, scene: Scene) -> (f64, f64) {
        let tab = if self.tabs() { TAB_H as f64 } else { 0.0 };
        match scene {
            Scene::Sentinel => (96.0, 2.5),
            Scene::Idle => {
                let chips = (!self.ports.is_empty()) as u8 + (!self.shelf.is_empty()) as u8;
                let w = match (chips, self.privacy_active()) {
                    (0, false) => 120.0,
                    (0, true) => 150.0,
                    (n, p) => 64.0 + 58.0 * n as f64 + if p { 34.0 } else { 0.0 },
                };
                (w + if self.mic_muted { LOCK_W as f64 } else { 0.0 }, 24.0)
            }
            Scene::CompactTask => (292.0, 32.0),
            Scene::CompactMedia => (240.0, 32.0),
            Scene::ExpMedia => (380.0, 132.0 + tab),
            Scene::ExpHome => (380.0, 136.0 + tab + self.apps_h()),
            Scene::ExpPower => (380.0, 112.0 + tab),
            Scene::ExpNudge => (360.0, 56.0),
            Scene::ExpNotes | Scene::ExpTodo => (380.0, self.jot_h(scene) + tab),
            Scene::ExpWeather => (380.0, self.weather_h() + tab),
            Scene::ExpDrop => (380.0, 116.0),
            Scene::ExpChip if self.chip.as_ref().is_some_and(Chip::one_line) => (380.0, 80.0),
            Scene::ExpChip => (380.0, 96.0),
            Scene::ExpTimerDone if self.focus_summary.is_some() => (380.0, 134.0),
            Scene::ExpTimerDone => (380.0, 114.0),
            Scene::ExpCharge => (300.0, 52.0),
            Scene::ExpSettings => (380.0, 216.0),
            // Running: ring + controls. Not running: the setup card.
            Scene::ExpTimer if self.timer.is_some() => (380.0, 96.0 + tab),
            Scene::ExpTimer => (380.0, 152.0 + tab),
            Scene::ExpShelf if self.shelf_sel.is_some() => (
                380.0,
                136.0 + 32.0 * (self.shelf_chip_lines() - 1) as f64 + tab,
            ),
            Scene::ExpShelf => (380.0, 118.0 + tab),
            Scene::ExpTasks => {
                let rows = self.rows();
                let (up, down) = self.rows_hidden();
                let more = if up + down > 0 { 16.0 } else { 0.0 };
                let mut h = 22.0 + rows.len() as f64 * 34.0 + more + tab + self.servers_h();
                for t in &rows {
                    if t.needs_input() {
                        h += 20.0;
                    } else if t.shows_line() {
                        h += LINE_H as f64;
                    }
                }
                if let Some(f) = rows.iter().find(|t| t.failed()) {
                    h += f.stderr.len().min(4) as f64 * 15.0 + 34.0;
                }
                (
                    380.0,
                    h.min(290.0 + (self.row_cap() as f64 - 4.0).max(0.0) * 34.0),
                )
            }
        }
    }

    /// Glow colour (rgb 0..1) and strength for the scene.
    pub fn glow(&self, scene: Scene) -> ([f32; 3], f32) {
        let (c, k) = self.glow_base(scene);
        // A scene with no colour of its own takes the skin's.
        (if c == [1.0; 3] { self.skin.glow() } else { c }, k)
    }

    fn glow_base(&self, scene: Scene) -> ([f32; 3], f32) {
        const RED: [f32; 3] = [1.0, 0.35, 0.37];
        const GREEN: [f32; 3] = [0.20, 0.82, 0.48];
        const BLUE: [f32; 3] = [0.35, 0.63, 1.0];
        const PURPLE: [f32; 3] = [0.65, 0.54, 0.98];
        const AMBER: [f32; 3] = [1.0, 0.72, 0.20];
        if self.mic_muted
            && matches!(
                scene,
                Scene::Idle | Scene::CompactMedia | Scene::ExpHome | Scene::ExpMedia
            )
        {
            return (RED, 0.8);
        }
        match scene {
            Scene::Sentinel | Scene::Idle => ([1.0; 3], 0.0),
            Scene::ExpDrop => (PURPLE, 1.0),
            Scene::ExpTimer => (BLUE, 0.30),
            Scene::ExpTimerDone => (GREEN, 0.8),
            Scene::ExpCharge if !self.charge_in => (AMBER, 0.6),
            Scene::ExpCharge => (GREEN, 0.7),
            Scene::ExpChip | Scene::ExpShelf => (PURPLE, 0.5),
            Scene::CompactMedia | Scene::ExpMedia
                if self.adaptive_glow && self.art_accent.is_some() =>
            {
                (self.art_accent.unwrap_or([1.0; 3]), 0.40)
            }
            Scene::ExpPower => match self.power {
                Some(b) if b.plugged => (GREEN, 0.35),
                Some(b) if b.pct <= 20 => (RED, 0.5),
                _ => ([1.0; 3], 0.10),
            },
            Scene::ExpTasks if self.visible_count() == 0 => ([1.0; 3], 0.10),
            Scene::ExpNudge => match self.nudge.map(|n| n.kind) {
                Some(crate::breaks::Kind::Water) => ([0.45, 0.85, 1.0], 0.35),
                Some(crate::breaks::Kind::Stretch) => (GREEN, 0.30),
                _ => (BLUE, 0.35),
            },
            Scene::ExpWeather if self.forecast.is_some() && !self.choosing_place() => self
                .forecast
                .as_ref()
                .map_or(([1.0; 3], 0.10), |f| sky_glow(f.now)),
            Scene::CompactMedia
            | Scene::ExpMedia
            | Scene::ExpHome
            | Scene::ExpSettings
            | Scene::ExpNotes
            | Scene::ExpTodo
            | Scene::ExpWeather => ([1.0; 3], 0.10),
            Scene::CompactTask | Scene::ExpTasks => match self.primary().map(|t| t.event) {
                Some(TaskEvent::NeedsInput) => (AMBER, 1.0),
                Some(TaskEvent::Failed) => (RED, 0.9),
                Some(TaskEvent::Done) => (GREEN, 0.6),
                Some(TaskEvent::Lost) => ([0.7; 3], 0.3),
                _ => (BLUE, 0.45),
            },
        }
    }

    /// True while something needs a steady (ambient) frame clock.
    pub fn ambient(&self, scene: Scene, now: Instant) -> bool {
        // The fullscreen sentinel is a static bar: nothing in it moves.
        if scene == Scene::Sentinel {
            return false;
        }
        self.tasks.iter().any(|t| {
            self.visible(t)
                && (t.running() || t.needs_input() || (t.failed() && t.fail_pulse(now) > 0.0))
        })
            // Privacy dots only breathe while expanded; collapsed they are static
            // so an active camera/mic doesn't cost a 30 fps render loop.
            || (self.privacy_active() && !matches!(scene, Scene::Idle | Scene::CompactTask | Scene::CompactMedia | Scene::Sentinel))
            || self.drop_over
            || scene == Scene::ExpCharge
            // The eye break's ring counts its 20 seconds down.
            || (scene == Scene::ExpNudge
                && self.nudge.is_some_and(|n| n.kind == crate::breaks::Kind::Eyes))
            // The bell swings while the alarm rings.
            || scene == Scene::ExpTimerDone
            || (self.media.as_ref().is_some_and(|m| m.playing)
                && matches!(scene, Scene::CompactMedia | Scene::ExpMedia))
    }
}

/// Scalar animation state, advanced with real frame deltas.
pub struct Anim {
    pub rect: RectSpring,
    pub glow: Spring,
    pub glow_rgb: [f32; 3],
    pub content: Spring,
    pub hover: Spring,
    pub vis: Spring,
    pub t: f64,
    pub scene: Scene,
    /// The scene being left; its content fades out (`out`) under the new one.
    pub prev: Scene,
    pub out: Spring,
    pub reduce: bool,
    /// Which way the user is paging (-1 previous, +1 next), set just before a layout that
    /// changes the scene and consumed by it. 0 = not a user page change.
    pub nav: f32,
    /// 0..1 how far the bubble has split off (springs a little past 1).
    pub bubble: Spring,
    /// The bubble being shown, kept while it retracts so it can still be drawn.
    pub bubble_kind: Option<Bubble>,
    /// `nav` as it was when the current scene started: the content slides in from that side.
    pub slide: f32,
}

impl Anim {
    pub fn new(w: f64, h: f64) -> Self {
        let mut vis = Spring::unit(1.0, 1.0, 0.22);
        vis.snap();
        Self {
            rect: RectSpring::new(w, h),
            glow: Spring::unit(0.0, 1.0, 0.30),
            glow_rgb: [1.0; 3],
            content: Spring::unit(1.0, 1.0, 0.21),
            hover: Spring::unit(0.0, 1.0, 0.16),
            vis,
            t: 0.0,
            scene: Scene::Idle,
            prev: Scene::Idle,
            out: Spring::unit(0.0, 1.0, 0.17),
            reduce: false,
            nav: 0.0,
            slide: 0.0,
            bubble: Spring::unit(0.0, 0.72, 0.34),
            bubble_kind: None,
        }
    }

    /// A quick squash-and-settle of the pill: widens and flattens a little, then the
    /// springs carry it back. Used when a drop lands.
    pub fn squash(&mut self) {
        if !self.reduce {
            self.rect.w.vel += 160.0;
            self.rect.h.vel -= 110.0;
        }
    }

    /// Point the bubble spring at the wanted state (`Some` = split off).
    pub fn set_bubble(&mut self, kind: Option<Bubble>) {
        if kind.is_some() {
            self.bubble_kind = kind;
        }
        self.bubble.set_target(kind.is_some() as u8 as f64);
        if self.reduce {
            self.bubble.snap();
        }
    }

    /// Logical px of room the bubble needs beside the pill right now.
    pub fn bubble_room(&self) -> f32 {
        if self.bubble.pos > 0.001 {
            BUBBLE_ROOM
        } else {
            0.0
        }
    }

    pub fn set_scene(&mut self, scene: Scene, size: (f64, f64), glow: ([f32; 3], f32)) {
        if scene != self.scene {
            // Fade the old content out from wherever its own fade-in had got to,
            // instead of cutting it in one frame.
            self.prev = self.scene;
            self.out.pos = self.content.pos.clamp(0.0, 1.0);
            self.out.vel = 0.0;
            self.out.set_target(0.0);
            self.scene = scene;
            self.slide = std::mem::take(&mut self.nav);
            self.content.pos = 0.0;
            self.content.vel = 0.0;
        }
        self.content.set_target(1.0);
        self.rect.set_target(size.0, size.1);
        self.glow.set_target(glow.1 as f64);
        self.glow_rgb = glow.0;
        if self.reduce {
            self.rect.snap();
            self.glow.snap();
            self.content.snap();
            self.out.snap();
        }
    }

    /// Advance everything; true while any spring is still moving.
    pub fn step(&mut self, dt: f64) -> bool {
        self.t += dt;
        if self.reduce {
            self.hover.snap();
            self.vis.snap();
            self.bubble.snap();
            return false;
        }
        let a = self.rect.advance(dt);
        let b = self.glow.advance(dt);
        let c = self.content.advance(dt);
        let d = self.hover.advance(dt);
        let e = self.vis.advance(dt);
        let f = self.out.advance(dt);
        let g = self.bubble.advance(dt);
        a || b || c || d || e || f || g
    }
}

/// Turns sideways scroll (touchpad swipe, tilt wheel) into pane pages: one per swipe, as
/// soon as it has moved a little.
///
/// A fast touchpad swipe is noisy: its deltas alternate high and low while the finger moves,
/// a lone event can point the wrong way, and after the finger lifts it keeps sending decaying
/// deltas (inertia) for half a second. So the deltas are smoothed, a single opposite event
/// inside a stream is ignored, and after a page the stream only pages again once it has
/// clearly slowed down and then sped up (a new swipe in the old one's tail), or after a pause.
#[derive(Debug, Default)]
pub struct Swipe {
    /// Movement since the gesture started (or re-armed).
    acc: i32,
    /// When the last delta came, and which way the gesture goes (+1 / -1).
    last: Option<Instant>,
    dir: i32,
    /// Opposite deltas in a row.
    flip: u8,
    /// Smoothed |delta|.
    ema: f32,
    /// After a page: when, the fastest it has gone since, whether it has slowed down, and
    /// the slowest since then.
    paged: Option<Instant>,
    peak: f32,
    slowed: bool,
    trough: f32,
}

/// Movement before a gesture pages (a tilt-wheel notch is 120).
const SWIPE_STEP: i32 = 80;
/// A pause this long between deltas ends a gesture.
const SWIPE_GAP: Duration = Duration::from_millis(100);
/// No second page sooner than this after one, however the stream behaves.
const SWIPE_LOCK: Duration = Duration::from_millis(160);

impl Swipe {
    /// Feed one delta; `Some(+1 / -1)` when it pages (right = next).
    pub fn feed(&mut self, delta: i32, now: Instant) -> Option<i32> {
        if delta == 0 {
            return None;
        }
        let gap = self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) > SWIPE_GAP);
        self.last = Some(now);
        let sign = delta.signum();
        let mag = delta.unsigned_abs() as f32;
        if !gap && sign != self.dir {
            // One stray event the other way is noise; two in a row turn the gesture round.
            self.flip += 1;
            if self.flip < 2 {
                return None;
            }
        }
        if gap || sign != self.dir {
            *self = Swipe {
                last: self.last,
                dir: sign,
                ema: mag,
                ..Default::default()
            };
        }
        self.flip = 0;
        self.ema += 0.3 * (mag - self.ema);
        if let Some(at) = self.paged {
            self.peak = self.peak.max(self.ema);
            if self.ema < 0.45 * self.peak {
                self.slowed = true;
            }
            if self.slowed {
                self.trough = self.trough.min(self.ema);
            }
            let again = now.saturating_duration_since(at) >= SWIPE_LOCK
                && self.slowed
                && self.ema >= 1.8 * self.trough
                && mag >= 24.0;
            if !again {
                return None;
            }
            self.paged = None;
            self.acc = 0;
        }
        self.acc += delta;
        if self.acc.abs() < SWIPE_STEP {
            return None;
        }
        self.paged = Some(now);
        self.peak = self.ema;
        self.slowed = false;
        self.trough = f32::MAX;
        Some(self.dir)
    }
}

/// What to call a listening process on the Ports pane: Docker and WSL by their helper
/// processes (their listeners are containers and Linux servers), else the project folder of
/// the terminal command that started it when that is tracked, else the exe's name.
pub fn port_name(exe: &str, cwd: Option<&str>) -> String {
    let stem = exe.trim_end_matches(".exe");
    let low = stem.to_ascii_lowercase();
    if low.contains("docker") {
        return "Docker".into();
    }
    if low.starts_with("wsl") || low == "vmmem" {
        return "WSL".into();
    }
    let folder = cwd
        .and_then(|c| c.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next())
        .filter(|f| !f.is_empty() && !f.ends_with(':'));
    folder.unwrap_or(stem).to_string()
}

/// Clicks on Allow / Deny count only this long after an ask appears.
pub const ASK_CLICK_GUARD: Duration = Duration::from_millis(400);

/// Whether an Allow / Deny click may answer: not within [`ASK_CLICK_GUARD`] of the ask appearing.
pub fn ask_click_ready(shown: Option<Instant>, now: Instant) -> bool {
    shown.is_none_or(|s| now.saturating_duration_since(s) >= ASK_CLICK_GUARD)
}

#[cfg(test)]
mod tests {
    #[test]
    fn panel_names_round_trip() {
        for p in [
            Panel::Tasks,
            Panel::Shelf,
            Panel::Media,
            Panel::Timer,
            Panel::Power,
            Panel::Home,
        ] {
            assert_eq!(Panel::from_name(p.name()), Some(p));
            assert_eq!(
                Panel::from_name(&format!(
                    "{}
",
                    p.name()
                )),
                Some(p)
            );
        }
        assert_eq!(Panel::from_name("nope"), None);
        assert_eq!(Panel::from_name(""), None);
    }

    use super::*;

    #[test]
    fn listeners_are_named_by_project_docker_wsl_or_exe() {
        assert_eq!(
            port_name("node.exe", Some(r"C:\code\shop-front")),
            "shop-front"
        );
        assert_eq!(
            port_name("node.exe", Some(r"C:\code\shop-front\")),
            "shop-front"
        );
        assert_eq!(port_name("node.exe", None), "node");
        assert_eq!(port_name("node.exe", Some(r"C:\")), "node");
        assert_eq!(
            port_name("com.docker.backend.exe", Some(r"C:\code\x")),
            "Docker"
        );
        assert_eq!(port_name("wslrelay.exe", None), "WSL");
        assert_eq!(port_name("vmmem", None), "WSL");
    }

    #[test]
    fn answers_wait_out_the_click_guard() {
        let t = Instant::now();
        assert!(!ask_click_ready(Some(t), t + Duration::from_millis(100)));
        assert!(ask_click_ready(Some(t), t + Duration::from_millis(400)));
        assert!(ask_click_ready(None, t));
    }

    #[test]
    fn restart_is_offered_only_for_a_tracked_runnable_command() {
        let t0 = Instant::now();
        let mut m = Model::default();
        let port = PortInfo {
            port: 3000,
            pid: 500,
            exe: "node.exe".into(),
        };
        assert!(m.restart_task(&port).is_none(), "no tracked command");
        let up = |id: &str, source: &str| match st(id, TaskEvent::Start, t0) {
            TaskUpdate::Upsert(mut s) => {
                s.source = source.into();
                s.pid = Some(500);
                s.cwd = Some("C:/x".into());
                TaskUpdate::Upsert(s)
            }
            u => u,
        };
        m.apply_task(up("agent", "agent"), t0);
        assert!(m.restart_task(&port).is_none(), "an agent can't be rerun");
        m.tasks.clear();
        m.apply_task(up("dev", "run"), t0);
        assert_eq!(m.restart_task(&port).map(|t| t.id.as_str()), Some("dev"));
    }

    #[test]
    fn a_cut_command_is_not_rerunnable() {
        let now = Instant::now();
        let mut t = TaskView {
            id: "t".into(),
            label: "cargo build".into(),
            event: TaskEvent::Failed,
            progress: None,
            duration_ms: None,
            exit_code: Some(1),
            stderr: vec![],
            pid: None,
            attention: None,
            line: None,
            source: "run".into(),
            cwd: Some("C:/x".into()),
            started: now,
            changed: now,
            visible_after: now,
        };
        assert!(t.rerunnable());
        t.label.push('…');
        assert!(!t.rerunnable());
    }

    #[test]
    fn shelf_row_scrolls_within_its_items() {
        let item = |id| ShelfItem {
            id,
            path: "x".into(),
            name: "x".into(),
            is_dir: false,
            owned: false,
        };
        let mut m = Model {
            shelf: (1..=8).map(item).collect(),
            ..Default::default()
        };
        assert_eq!(m.shelf_page().1.len(), SHELF_VISIBLE);
        m.scroll_shelf(10);
        assert_eq!(m.shelf_page().0, 3, "stops with the last tile in view");
        m.scroll_shelf(-1);
        assert_eq!(m.shelf_page().1[0].id, 3);
        // Items removed from the end: the row slides back instead of showing a gap.
        m.shelf.truncate(6);
        assert_eq!(m.shelf_page(), (1, &m.shelf[1..6]));
        m.shelf.truncate(3);
        assert_eq!(m.shelf_page().0, 0);
        m.scroll_shelf(-5);
        assert_eq!(m.shelf_off, 0);
    }

    #[test]
    fn swipe_pages_once_per_gesture() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut s = Swipe::default();
        // A swipe: rising then decaying deltas every 8 ms, with a long inertia tail.
        let swipe = [10, 25, 30, 22, 14, 9, 6, 4, 3, 2, 2, 1, 1, 1];
        let mut pages = vec![];
        for (i, d) in swipe.iter().enumerate() {
            pages.extend(s.feed(*d, ms(i as u64 * 8)));
        }
        assert_eq!(pages, vec![1], "one page, early in the swipe");
        // A second swipe during that swipe's inertia still pages: its deltas jump up.
        let mut t = swipe.len() as u64 * 8;
        let mut second = vec![];
        for d in [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 12, 28, 34, 30, 15, 6] {
            second.extend(s.feed(d, ms(t)));
            t += 8;
        }
        assert_eq!(second, vec![1]);
        // After a pause, going left pages back, once it has moved far enough.
        assert_eq!(s.feed(-30, ms(t + 300)), None);
        assert_eq!(s.feed(-30, ms(t + 308)), None);
        assert_eq!(s.feed(-30, ms(t + 316)), Some(-1));
        // Tilt-wheel notches 300 ms apart: one page each.
        let mut s = Swipe::default();
        assert_eq!(s.feed(120, ms(0)), Some(1));
        assert_eq!(s.feed(120, ms(300)), Some(1));
        // Reversing mid-stream: one stray event is ignored, a second turns it round.
        assert_eq!(s.feed(-120, ms(320)), None);
        assert_eq!(s.feed(-120, ms(330)), Some(-1));
    }

    /// Recorded from a real touchpad: one fast swipe left (alternating deltas while the finger
    /// moves, a lone spike the wrong way, then the inertia tail) pages exactly once.
    #[test]
    fn noisy_fast_swipe_pages_once() {
        let t0 = Instant::now();
        let mut s = Swipe::default();
        let swipe = [
            -220, -251, -97, -227, -128, -146, -463, -174, -172, -571, -196, -205, -408, -199,
            -198, -115, 467, -943, -745, -686, -670, -697, -639, -664, -611, -811, -820, -452,
            -541, -524, -512, -525, -511, -549, -390, -435, -417, -648, -403, -563, -339, -344,
            -478, -285, -278, -247, -286, -278,
        ];
        let pages: Vec<i32> = swipe
            .iter()
            .enumerate()
            .filter_map(|(i, d)| s.feed(*d, t0 + Duration::from_millis(i as u64 * 4)))
            .collect();
        assert_eq!(pages, vec![-1]);
    }

    fn st(id: &str, ev: TaskEvent, now: Instant) -> TaskUpdate {
        TaskUpdate::Upsert(TaskState {
            task_id: id.into(),
            label: "build".into(),
            progress: None,
            event: ev,
            duration_ms: Some(10),
            stderr_tail: Some(
                "boom
bang"
                    .into(),
            ),
            source: "run".into(),
            pid: None,
            delay_ms: 0,
            message: None,
            exit_code: None,
            line: None,
            cwd: None,
            updated: now,
        })
    }

    fn shell(id: &str, label: &str, ev: TaskEvent, delay: u32, now: Instant) -> TaskUpdate {
        TaskUpdate::Upsert(TaskState {
            task_id: id.into(),
            label: label.into(),
            progress: None,
            event: ev,
            duration_ms: None,
            stderr_tail: None,
            source: "shell".into(),
            pid: Some(1),
            delay_ms: delay,
            message: None,
            exit_code: None,
            line: None,
            cwd: None,
            updated: now,
        })
    }

    #[test]
    fn success_expires_failure_persists() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Done, t0), t0);
        m.apply_task(st("b", TaskEvent::Failed, t0), t0);
        assert_eq!(m.tasks.len(), 2);
        m.expire(t0 + Duration::from_secs(4));
        assert_eq!(m.tasks.len(), 1);
        assert!(m.tasks[0].failed());
        assert_eq!(m.tasks[0].stderr, vec!["boom", "bang"]);
        m.dismiss_task("b");
        assert!(m.tasks.is_empty());
    }

    #[test]
    fn finished_rows_wait_for_an_away_user() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.set_idle(Duration::from_secs(90), t0);
        m.apply_task(st("a", TaskEvent::Start, t0), t0);
        m.apply_task(st("a", TaskEvent::Done, t0), t0);
        // Away: held well past SUCCESS_HOLD, and the deadline is a short poll.
        let t1 = t0 + Duration::from_secs(600);
        m.set_idle(Duration::from_secs(690), t1);
        assert_eq!(m.expire(t1), Some(t1 + AWAY_POLL));
        assert_eq!(m.tasks.len(), 1);
        // Back (input 1 s ago): the full hold runs from that input, then the row goes.
        let t2 = t1 + Duration::from_secs(5);
        m.set_idle(Duration::from_secs(1), t2);
        assert!(m.expire(t2).is_some());
        assert_eq!(m.tasks.len(), 1);
        m.expire(t2 + SUCCESS_HOLD);
        assert!(m.tasks.is_empty());
    }

    #[test]
    fn a_stalled_download_expires_and_a_live_one_does_not() {
        let t0 = Instant::now();
        let mut m = Model::default();
        let dl = |id: &str, at| match st(id, TaskEvent::Start, at) {
            TaskUpdate::Upsert(mut s) => {
                s.source = "download".into();
                TaskUpdate::Upsert(s)
            }
            u => u,
        };
        m.apply_task(dl("stalled", t0), t0);
        m.apply_task(dl("live", t0), t0);
        let late = t0 + DOWNLOAD_STALE - Duration::from_secs(1);
        m.apply_task(dl("live", late), late); // its file grew
        m.expire(late);
        assert_eq!(m.tasks.len(), 2, "nothing is stale yet");
        let later = t0 + DOWNLOAD_STALE + Duration::from_secs(1);
        m.expire(later);
        assert_eq!(m.tasks.len(), 1);
        assert_eq!(m.tasks[0].id, "live");
        // The stalled one starts changing again: it is back.
        m.apply_task(dl("stalled", later), later);
        assert_eq!(m.tasks.len(), 2);
    }

    #[test]
    fn a_repeated_wait_keeps_its_start() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(90);
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Start, t0), t0);
        m.apply_task(st("a", TaskEvent::NeedsInput, t0), t0);
        m.apply_task(st("a", TaskEvent::NeedsInput, t1), t1);
        assert_eq!(m.tasks[0].changed, t0);
        m.apply_task(st("a", TaskEvent::Resumed, t1), t1);
        assert_eq!(m.tasks[0].changed, t1);
    }

    fn song(title: &str, playing: bool, pos: u64) -> MediaInfo {
        MediaInfo {
            title: title.into(),
            artist: "Band".into(),
            album: "Album".into(),
            playing,
            app: "Spotify.exe".into(),
            pos_ms: pos,
            dur_ms: 200_000,
        }
    }

    #[test]
    fn lyrics_are_fetched_once_per_song_and_only_when_wanted() {
        use crate::lyrics::Lyrics;
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_media(Some(song("One", true, 0)), t0);
        assert_eq!(m.lyrics_request(), None, "page closed");
        m.lyrics_open = true;
        assert_eq!(m.lyrics_request(), None, "lyrics off");
        m.lyrics_on = true;
        let (key, title, artist, album, dur) = m.lyrics_request().expect("a lookup");
        assert_eq!(
            (title.as_str(), artist.as_str(), album.as_str(), dur),
            ("One", "Band", "Album", 200_000)
        );
        assert_eq!(m.lyric_state, LyricState::Loading);
        assert_eq!(m.lyrics_request(), None, "not twice");
        // Position updates for the same song change nothing.
        m.apply_media(Some(song("One", true, 5_000)), t0);
        assert_eq!(m.lyric_state, LyricState::Loading);
        m.lyrics_result(key.clone(), Ok(Lyrics::Instrumental));
        assert_eq!(m.lyric_state, LyricState::Ready(Lyrics::Instrumental));
        // The next song starts over, and the first one is remembered.
        m.apply_media(Some(song("Two", true, 0)), t0);
        assert_eq!(m.lyric_state, LyricState::Idle);
        let (key2, ..) = m.lyrics_request().expect("a lookup for the new song");
        // A late answer for the old song doesn't show on the new one.
        m.lyrics_result(key.clone(), Ok(Lyrics::NotFound));
        assert_eq!(m.lyric_state, LyricState::Loading);
        m.lyrics_result(key2, Err("offline".into()));
        assert_eq!(m.lyric_state, LyricState::Failed);
        // Back to song one: answered from memory, no lookup.
        m.apply_media(Some(song("One", true, 0)), t0);
        assert_eq!(m.lyrics_request(), None);
        assert_eq!(
            m.lyric_state,
            LyricState::Ready(Lyrics::NotFound),
            "the latest answer is kept"
        );
        // The player going away closes the extras.
        m.sleep_open = true;
        m.apply_media(None, t0);
        assert!(!m.lyrics_open && !m.sleep_open);
    }

    #[test]
    fn the_lyric_cache_keeps_the_last_twenty_songs() {
        use crate::lyrics::Lyrics;
        let mut m = Model::default();
        for i in 0..25 {
            m.lyrics_result(format!("k{i}"), Ok(Lyrics::Instrumental));
        }
        assert_eq!(m.lyric_cache.len(), 20);
        assert_eq!(m.lyric_cache[0].0, "k5");
    }

    #[test]
    fn plain_lyrics_scroll_three_lines_at_a_time_to_the_end() {
        use crate::lyrics::Lyrics;
        let mut m = Model::default();
        assert!(!m.scroll_lyrics(1), "nothing to scroll");
        m.lyric_state = LyricState::Ready(Lyrics::Plain((0..6).map(|i| format!("l{i}")).collect()));
        assert!(m.scroll_lyrics(1) && m.scroll_lyrics(1) && m.scroll_lyrics(1));
        assert_eq!(m.lyric_first, 3, "the last three lines");
        assert!(!m.scroll_lyrics(1), "at the end the wheel pages");
        assert!(m.scroll_lyrics(-1));
    }

    #[test]
    fn the_sleep_timer_follows_the_song_and_wakes_the_model() {
        use crate::sleep::Kind;
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_media(Some(song("One", true, 100_000)), t0);
        m.set_sleep(Some(Kind::EndOfTrack), t0);
        assert_eq!(
            m.sleep.as_ref().unwrap().until,
            Some(t0 + Duration::from_secs(100))
        );
        assert!(m.expire(t0).is_some(), "the model wakes for the fade");
        // A seek moves it; a new song fires it.
        m.apply_media(
            Some(song("One", true, 190_000)),
            t0 + Duration::from_secs(1),
        );
        assert_eq!(
            m.sleep.as_ref().unwrap().until,
            Some(t0 + Duration::from_secs(11))
        );
        m.apply_media(Some(song("Two", true, 0)), t0 + Duration::from_secs(2));
        assert!(m.sleep.as_ref().unwrap().fire);
        m.set_sleep(None, t0);
        assert!(m.sleep.is_none() && !m.sleep_open);
        assert_eq!(m.expire(t0), None, "nothing left to wake for");
        m.set_sleep(Some(Kind::Minutes(30)), t0);
        assert_eq!(
            m.expire(t0),
            Some(t0 + Duration::from_secs(60)),
            "the label ticks each minute"
        );
    }

    fn focus_timer() -> crate::timer::Timer {
        crate::timer::Timer::start(crate::timer::TimerKind::Focus, 25, crate::timer::now_ms())
    }

    #[test]
    fn only_a_focus_timer_holds_alerts() {
        use crate::timer::{Timer, TimerKind};
        let mut m = Model::default();
        assert!(!m.holding());
        let wall = crate::timer::now_ms();
        for (kind, holds) in [
            (TimerKind::Plain, false),
            (TimerKind::Break, false),
            (TimerKind::Focus, true),
        ] {
            m.timer = Some(Timer::start(kind, 5, wall));
            assert_eq!(m.holding(), holds, "{kind:?}");
        }
        // Paused is still a session.
        m.timer = Some(focus_timer());
        m.timer.as_mut().unwrap().paused_ms = Some(60_000);
        assert!(m.holding());
    }

    #[test]
    fn held_alerts_are_told_once_in_one_line() {
        let mut h = Held::default();
        assert_eq!(h.take_summary(), None);
        h.downloads = 2;
        h.failed = 1;
        assert_eq!(
            h.take_summary().as_deref(),
            Some("While you focused: 2 downloads, 1 task failed")
        );
        assert_eq!(h.take_summary(), None, "counts start over");
        h = Held {
            downloads: 1,
            done: 3,
            failed: 0,
            warnings: 1,
        };
        assert_eq!(
            h.take_summary().as_deref(),
            Some("While you focused: 1 download, 3 tasks finished, 1 warning")
        );
    }

    #[test]
    fn an_agent_reply_waits_out_a_focus_session() {
        let t0 = Instant::now();
        let mut m = Model {
            finish_peek: Duration::from_secs(5),
            ..Default::default()
        };
        let reply = || {
            let mut u = st("agent:Claude Code:1", TaskEvent::Done, t0);
            if let TaskUpdate::Upsert(s) = &mut u {
                s.source = "agent".into();
                s.label = "Claude Code".into();
                s.message = Some("All done.".into());
            }
            u
        };
        m.timer = Some(focus_timer());
        m.apply_task(reply(), t0);
        assert!(m.chip.is_none(), "no card during focus");
        m.timer = None;
        m.apply_task(reply(), t0);
        assert!(m.chip.is_some(), "and back to normal afterwards");
    }

    #[test]
    fn a_break_nudge_sits_below_cards_and_ends_by_itself() {
        use crate::breaks::Kind;
        let t0 = Instant::now();
        let mut m = Model::default();
        m.now = Some(t0);
        m.nudge = Some(Nudge {
            kind: Kind::Eyes,
            since: t0,
        });
        assert_eq!(m.scene(false, false), Scene::ExpNudge);
        assert_eq!(
            m.scene(true, false),
            Scene::ExpNudge,
            "hovering doesn't hide it"
        );
        // A card, the finished-timer card and the settings pane are never covered by it.
        m.settings = Some(0);
        assert_eq!(m.scene(false, false), Scene::ExpSettings);
        m.settings = None;
        m.timer_done = Some((crate::timer::TimerKind::Focus, t0));
        assert_eq!(m.scene(false, false), Scene::ExpTimerDone);
        m.timer_done = None;
        m.chip = Some(Chip::new("x", t0));
        assert_eq!(m.scene(false, false), Scene::ExpChip);
        m.chip = None;
        // The eye break animates its ring, and only it; neither outlives its hold.
        assert!(m.ambient(Scene::ExpNudge, t0));
        m.nudge = Some(Nudge {
            kind: Kind::Water,
            since: t0,
        });
        assert!(!m.ambient(Scene::ExpNudge, t0));
        assert_eq!(m.expire(t0), Some(t0 + Duration::from_secs(10)));
        assert!(m.nudge.is_some());
        m.expire(t0 + Duration::from_secs(10));
        assert!(m.nudge.is_none());
        assert_eq!(m.scene(false, false), Scene::Idle);
        m.nudge = Some(Nudge {
            kind: Kind::Eyes,
            since: t0,
        });
        m.expire(t0 + Duration::from_secs(19));
        assert!(m.nudge.is_some(), "twenty seconds for the eyes");
        m.expire(t0 + Duration::from_secs(20));
        assert!(m.nudge.is_none());
        assert!(!m.ambient(Scene::ExpNudge, t0), "frames stop with it");
    }

    #[test]
    fn the_focus_summary_goes_when_the_finished_card_does() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.timer_done = Some((crate::timer::TimerKind::Focus, t0));
        m.focus_summary = Some("While you focused: 1 download".into());
        assert_eq!(m.size(Scene::ExpTimerDone).1, 134.0, "room for the line");
        m.expire(t0);
        assert!(m.focus_summary.is_some());
        m.timer_done = None;
        m.expire(t0);
        assert!(m.focus_summary.is_none());
        assert_eq!(m.size(Scene::ExpTimerDone).1, 114.0);
    }

    #[test]
    fn an_agent_reply_comes_as_a_card_with_copy() {
        let t0 = Instant::now();
        let mut m = Model {
            finish_peek: Duration::from_secs(5),
            ..Default::default()
        };
        let done = |msg: Option<&str>| {
            let mut u = st("agent:Claude Code:1", TaskEvent::Done, t0);
            if let TaskUpdate::Upsert(s) = &mut u {
                s.source = "agent".into();
                s.label = "Claude Code".into();
                s.message = msg.map(Into::into);
            }
            u
        };
        m.apply_task(done(None), t0);
        assert!(m.chip.is_none());
        m.apply_task(done(Some("\n## **Fixed the bug.**\nDetails follow.")), t0);
        let c = m.chip.as_ref().unwrap();
        assert_eq!(c.summary, "Claude Code: Fixed the bug.");
        assert_eq!(c.hold, Duration::from_secs(5));
        assert_eq!(
            c.copy.as_deref(),
            Some("## **Fixed the bug.**\nDetails follow.")
        );
        // A pending ask is not pushed aside.
        let mut ask = Chip::new("Allow?", t0);
        ask.tone = Tone::Warn;
        m.chip = Some(ask);
        m.apply_task(done(Some("Other reply")), t0);
        assert_eq!(m.chip.as_ref().unwrap().summary, "Allow?");
        // "Open on finish" off: no card at all.
        m.chip = None;
        m.finish_peek = Duration::ZERO;
        m.apply_task(done(Some("Quiet reply")), t0);
        assert!(m.chip.is_none());
    }

    #[test]
    fn running_cannot_be_dismissed() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Start, t0), t0);
        m.dismiss_task("a");
        assert_eq!(m.tasks.len(), 1);
    }

    #[test]
    fn scene_priority() {
        let t0 = Instant::now();
        let mut m = Model::default();
        assert_eq!(m.scene(false, false), Scene::Idle);
        assert_eq!(m.scene(false, true), Scene::Sentinel);
        assert_eq!(m.scene(true, true), Scene::ExpHome);
        m.apply_task(st("a", TaskEvent::Start, t0), t0);
        assert_eq!(m.scene(false, false), Scene::CompactTask);
        assert_eq!(m.scene(true, false), Scene::ExpTasks);
        m.drop_over = true;
        assert_eq!(m.scene(false, false), Scene::ExpDrop);
    }

    #[test]
    fn leaving_a_scene_fades_its_content_out() {
        let mut a = Anim::new(120.0, 24.0);
        a.set_scene(Scene::ExpHome, (380.0, 86.0), ([1.0; 3], 0.1));
        while a.step(1.0 / 60.0) {}
        a.set_scene(Scene::Idle, (120.0, 24.0), ([1.0; 3], 0.0));
        assert_eq!(
            (a.prev, a.out.pos),
            (Scene::ExpHome, 1.0),
            "old content still fully shown"
        );
        a.step(0.05);
        assert!(a.out.pos > 0.0 && a.out.pos < 1.0, "fading, not cut");
        while a.step(1.0 / 60.0) {}
        assert_eq!(a.out.pos, 0.0);
    }

    #[test]
    fn reduced_motion_snaps() {
        let mut a = Anim::new(120.0, 24.0);
        a.reduce = true;
        a.set_scene(Scene::ExpHome, (320.0, 84.0), ([1.0; 3], 0.1));
        assert_eq!(a.rect.w.pos, 320.0);
        assert!(!a.step(0.016));
    }

    #[test]
    fn shell_task_hidden_until_threshold() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(shell("s", "cargo build", TaskEvent::Start, 3000, t0), t0);
        assert_eq!(m.scene(false, false), Scene::Idle, "not shown yet");
        let next = m.expire(t0 + Duration::from_millis(100)).unwrap();
        assert_eq!(
            next,
            t0 + Duration::from_millis(3000),
            "timer armed for the threshold"
        );
        m.expire(t0 + Duration::from_millis(3100));
        assert_eq!(m.scene(false, false), Scene::CompactTask);
    }

    #[test]
    fn fast_shell_command_never_shows() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(shell("s", "ls", TaskEvent::Start, 3000, t0), t0);
        let t1 = t0 + Duration::from_millis(500);
        m.apply_task(shell("s", "ls", TaskEvent::Done, 0, t1), t1);
        assert!(m.tasks.is_empty());
    }

    #[test]
    fn an_agent_ask_shows_over_fullscreen() {
        let t0 = Instant::now();
        let mut m = Model::default();
        assert_eq!(m.scene(false, true), Scene::Sentinel);
        let mut c = Chip::new("Claude Code: Running: cargo build", t0);
        c.extra = vec![("Allow".into(), ChipAction::Answer(1, true))];
        m.chip = Some(c);
        assert_ne!(m.scene(false, true), Scene::Sentinel);
    }

    #[test]
    fn agent_clis_are_left_to_their_hooks() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(shell("s", "claude --resume", TaskEvent::Start, 0, t0), t0);
        m.apply_task(
            shell("o", r"C:in\opencode.exe", TaskEvent::Start, 0, t0),
            t0,
        );
        assert!(m.tasks.is_empty());
    }

    #[test]
    fn ignored_shell_commands_are_dropped() {
        let t0 = Instant::now();
        let mut m = Model {
            ignore: vec!["vim".into()],
            ..Default::default()
        };
        m.apply_task(
            shell("s", r"C:\tools\Vim.exe file.txt", TaskEvent::Start, 0, t0),
            t0,
        );
        assert!(m.tasks.is_empty());
    }

    #[test]
    fn attention_outranks_and_peeks() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Failed, t0), t0);
        let mut u = match st("b", TaskEvent::NeedsInput, t0) {
            TaskUpdate::Upsert(s) => s,
            _ => unreachable!(),
        };
        u.message = Some("approve edit?".into());
        m.apply_task(TaskUpdate::Upsert(u), t0);
        assert_eq!(m.primary().unwrap().id, "b");
        assert!(m.rows()[0].needs_input());
        m.peek(Panel::Tasks, t0 + Duration::from_secs(6));
        assert_eq!(m.scene(false, false), Scene::ExpTasks);
        m.expire(t0 + Duration::from_secs(7));
        assert_eq!(m.scene(false, false), Scene::CompactTask);
        assert!(
            m.tasks.iter().any(|t| t.needs_input()),
            "attention persists after the peek"
        );
    }

    /// A model showing only the panes that exist without Notes, To-do and Weather.
    fn no_extra_panes() -> Model {
        let mut m = Model::default();
        m.panes = crate::config::Panes {
            notes: false,
            todo: false,
            weather: false,
        };
        m
    }

    #[test]
    fn notes_todo_and_weather_follow_home_and_can_be_switched_off() {
        let mut m = Model::default();
        assert_eq!(
            m.panels(),
            vec![Panel::Home, Panel::Notes, Panel::Todo, Panel::Weather]
        );
        assert!(m.tabs(), "the strip shows even with nothing else going on");
        m.panes.todo = false;
        m.selected = Some(Panel::Todo);
        assert_eq!(m.panel(), Panel::Home, "a hidden pane can't stay selected");
        assert_eq!(Panel::from_name("To-do"), Some(Panel::Todo));
        assert_eq!(Panel::of(Scene::ExpWeather), Some(Panel::Weather));
        // Scenes follow the panel, and the typing panes grow with their rows.
        m.selected = Some(Panel::Notes);
        assert_eq!(m.scene(true, false), Scene::ExpNotes);
        let empty = m.size(Scene::ExpNotes).1;
        let now = crate::timer::now_ms();
        for i in 0..3 {
            m.jot
                .add(crate::jot::Kind::Note, &format!("n{i}"), true, now);
        }
        assert_eq!(m.size(Scene::ExpNotes).1, empty + 60.0);
        for i in 3..9 {
            m.jot
                .add(crate::jot::Kind::Note, &format!("n{i}"), true, now);
        }
        assert_eq!(
            m.size(Scene::ExpNotes).1,
            empty + 90.0,
            "no taller than four rows"
        );
    }

    #[test]
    fn notes_are_added_edited_deleted_and_undone() {
        let mut m = Model::default();
        m.note_edit.set("  buy milk ");
        assert!(m.submit_jot(Field::Note));
        assert!(m.note_edit.is_empty(), "the field clears");
        assert!(m.jot_dirty);
        assert_eq!(m.jot.notes()[0].text, "buy milk");
        assert!(!m.submit_jot(Field::Note), "nothing to add");
        let id = m.jot.notes()[0].id;
        m.begin_edit(id);
        assert_eq!(m.typing, Some(Field::Note));
        assert_eq!(m.note_edit.text(), "buy milk");
        m.note_edit.set("buy oat milk");
        assert!(m.submit_jot(Field::Note));
        assert_eq!(m.jot.notes()[0].text, "buy oat milk");
        assert_eq!(m.jot.items.len(), 1, "an edit doesn't add");
        // Esc on an edit drops it and leaves the field empty.
        m.begin_edit(id);
        m.end_typing();
        assert!(m.note_edit.is_empty() && m.typing.is_none() && m.jot_edit.is_none());
        let t0 = Instant::now();
        m.delete_jot(id, t0);
        assert!(m.jot.items.is_empty());
        assert!(m.jot_undo.is_some());
        m.undo_jot();
        assert_eq!(m.jot.items.len(), 1);
        // Undo goes after a few seconds, with its own deadline in `expire`.
        m.delete_jot(id, t0);
        let next = m.expire(t0 + Duration::from_secs(1));
        assert_eq!(next, Some(t0 + UNDO_HOLD));
        m.expire(t0 + UNDO_HOLD);
        assert!(m.jot_undo.is_none());
    }

    #[test]
    fn a_draft_survives_leaving_the_field_and_wheel_scrolls_only_long_lists() {
        let mut m = Model::default();
        m.note_edit.set("half a thought");
        m.typing = Some(Field::Note);
        m.end_typing();
        assert_eq!(m.note_edit.text(), "half a thought", "the draft stays");
        m.selected = Some(Panel::Notes);
        let now = crate::timer::now_ms();
        for i in 0..4 {
            m.jot
                .add(crate::jot::Kind::Note, &format!("n{i}"), true, now + i);
        }
        assert!(!m.scroll_jot(1), "four rows fit: the wheel pages instead");
        m.jot.add(crate::jot::Kind::Note, "n4", true, now + 4);
        assert!(m.scroll_jot(1));
        assert!(!m.scroll_jot(1), "at the end it pages again");
        assert_eq!(m.jot_window(Scene::ExpNotes), (1, 4));
    }

    #[test]
    fn temporary_notes_set_the_next_deadline() {
        let mut m = Model::default();
        let t0 = Instant::now();
        assert_eq!(m.expire(t0), None);
        m.jot.add(
            crate::jot::Kind::Note,
            "soon",
            false,
            crate::timer::now_ms(),
        );
        let next = m.expire(t0).expect("a deadline for the 24 h note");
        let secs = next.duration_since(t0).as_secs();
        assert!((86_395..=86_400).contains(&secs), "{secs}");
    }

    #[test]
    fn panels_and_selection() {
        // (a running timer adds its own pane)
        let mut m = no_extra_panes();
        assert_eq!(m.panels(), vec![Panel::Home]);
        assert!(!m.tabs());
        m.ports = vec![PortInfo {
            port: 3000,
            pid: 9,
            exe: "node.exe".into(),
        }];
        m.shelf = vec![ShelfItem {
            id: 1,
            path: "a".into(),
            name: "a".into(),
            is_dir: false,
            owned: false,
        }];
        // Shelf and music to the left of home, dev work and the battery to its right; a
        // listening port puts the Tasks pane up even without a task.
        m.power = Some(crate::power::Battery {
            pct: 50,
            plugged: false,
            watts: None,
            mode: crate::power::Mode::Balanced,
            mins_left: None,
        });
        assert_eq!(
            m.panels(),
            vec![Panel::Shelf, Panel::Home, Panel::Tasks, Panel::Power]
        );
        assert!(m.tabs());
        assert_eq!(m.panel(), Panel::Home, "home is the default");
        m.selected = Some(Panel::Tasks);
        assert_eq!(m.panel(), Panel::Tasks);
        m.ports.clear();
        assert_eq!(m.panel(), Panel::Home, "stale selection falls back to home");
        // A running task in the collapsed pill opens on its own pane.
        m.selected = None;
        m.apply_task(st("t", TaskEvent::Start, Instant::now()), Instant::now());
        assert_eq!(m.panel(), Panel::Tasks);
    }

    #[test]
    fn wheel_steps_and_wraps() {
        let mut m = no_extra_panes();
        m.ports = vec![PortInfo {
            port: 3000,
            pid: 9,
            exe: "node.exe".into(),
        }];
        assert_eq!(m.panels(), vec![Panel::Home, Panel::Tasks]);
        assert_eq!(m.step_panel(1), Panel::Tasks);
        assert_eq!(m.step_panel(1), Panel::Home, "wraps forward");
        assert_eq!(m.step_panel(-1), Panel::Tasks, "wraps backward");
    }

    #[test]
    fn drop_breaks_through_fullscreen_suppression() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.now = Some(t0);
        assert_eq!(m.scene(false, true), Scene::Sentinel);
        m.drop_over = true;
        assert_eq!(
            m.scene(false, true),
            Scene::ExpDrop,
            "drag over the notch while fullscreen"
        );
        m.drop_over = false;
        m.shelf = vec![ShelfItem {
            id: 1,
            path: "a".into(),
            name: "a".into(),
            is_dir: false,
            owned: false,
        }];
        m.force_until = Some(t0 + Duration::from_secs(4));
        m.peek(Panel::Shelf, t0 + Duration::from_secs(4));
        assert_eq!(
            m.scene(false, true),
            Scene::ExpShelf,
            "feedback after the drop"
        );
        m.expire(t0 + Duration::from_secs(5));
        assert_eq!(m.scene(false, true), Scene::Sentinel);
    }

    #[test]
    fn timer_fires_once_and_card_expires() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.timer = Some(crate::timer::Timer::start(
            crate::timer::TimerKind::Focus,
            1,
            1_000,
        ));
        assert_eq!(m.finish_timer(30_000, t0), None, "not due yet");
        assert_eq!(
            m.finish_timer(61_000, t0),
            Some(crate::timer::TimerKind::Focus)
        );
        assert_eq!((m.focus_done, m.timer.is_none()), (1, true));
        assert_eq!(m.finish_timer(99_000, t0), None, "only fires once");
        assert_eq!(m.scene(false, false), Scene::ExpTimerDone);
        m.expire(t0 + TIMER_DONE_HOLD);
        assert_eq!(m.scene(false, false), Scene::Idle);
    }

    #[test]
    fn muted_mic_widens_idle_and_glows_red() {
        let mut m = Model::default();
        let w0 = m.size(Scene::Idle).0;
        m.mic_muted = true;
        assert_eq!(m.size(Scene::Idle).0, w0 + LOCK_W as f64);
        assert_eq!(m.glow(Scene::Idle).1, 0.8);
    }

    #[test]
    fn idle_frames_stop_for_settled_failures_and_sentinel() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Failed, t0), t0);
        assert!(m.ambient(Scene::CompactTask, t0), "fresh failure pulses");
        let later = t0 + FAIL_PULSE;
        assert!(!m.ambient(Scene::CompactTask, later), "then rests");
        assert_eq!(m.tasks[0].fail_pulse(later), 0.0);
        m.apply_task(st("b", TaskEvent::Start, t0), t0);
        assert!(
            m.ambient(Scene::CompactTask, later),
            "running work still animates"
        );
        assert!(
            !m.ambient(Scene::Sentinel, later),
            "the sentinel bar is static"
        );
    }

    #[test]
    fn primary_is_first_row() {
        let t0 = Instant::now();
        let mut m = Model::default();
        // Same rank and timestamp: the first inserted wins in both.
        m.apply_task(st("x", TaskEvent::Start, t0), t0);
        m.apply_task(st("y", TaskEvent::Start, t0), t0);
        assert_eq!(m.primary().unwrap().id, m.rows()[0].id);
        m.apply_task(st("z", TaskEvent::Failed, t0), t0 + Duration::from_secs(1));
        assert_eq!(m.primary().unwrap().id, "z");
        assert_eq!(m.primary().unwrap().id, m.rows()[0].id);
    }

    fn battery(plugged: bool) -> crate::power::Battery {
        crate::power::Battery {
            pct: 60,
            plugged,
            watts: None,
            mode: crate::power::Mode::Balanced,
            mins_left: None,
        }
    }

    #[test]
    fn unplugging_shows_the_card_the_other_way() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.set_power(Some(battery(true)), t0);
        m.set_power(Some(battery(false)), t0);
        assert_eq!(m.charge_since, Some(t0));
        assert!(!m.charge_in);
        m.expire(t0);
        assert_eq!(m.scene(false, false), Scene::ExpCharge);
        assert_eq!(m.expire(t0 + CHARGE_HOLD), None);
        // Plugging back in flips the direction.
        let t1 = t0 + CHARGE_HOLD;
        m.set_power(Some(battery(true)), t1);
        assert!(m.charge_in);
    }

    #[test]
    fn plugging_in_shows_the_charge_card_briefly() {
        let t0 = Instant::now();
        let mut m = Model::default();
        // The first reading, or staying on one source, never triggers it.
        m.set_power(Some(battery(false)), t0);
        m.set_power(Some(battery(false)), t0);
        assert_eq!(m.charge_since, None);

        m.set_power(Some(battery(true)), t0);
        assert_eq!(m.charge_since, Some(t0));
        m.expire(t0);
        assert_eq!(m.scene(false, false), Scene::ExpCharge);
        // Hovering still opens the real panel (home), and a charge card is not interactive state.
        assert_eq!(m.scene(true, false), Scene::ExpHome);
        assert!(m.ambient(Scene::ExpCharge, t0));
        // The single expiry timer is armed for exactly the end of the card.
        assert_eq!(m.expire(t0), Some(t0 + CHARGE_HOLD));
        assert_eq!(m.expire(t0 + CHARGE_HOLD), None);
        assert_eq!(m.scene(false, false), Scene::Idle);
    }

    #[test]
    fn paging_direction_belongs_to_one_scene_change() {
        let mut a = Anim::new(100.0, 24.0);
        let size = (380.0, 100.0);
        let glow = ([1.0; 3], 0.0);
        a.nav = 1.0;
        a.set_scene(Scene::ExpMedia, size, glow);
        assert_eq!((a.slide, a.nav), (1.0, 0.0));
        // Re-laying out the same scene keeps the side it slid in from.
        a.set_scene(Scene::ExpMedia, size, glow);
        assert_eq!(a.slide, 1.0);
        // A change nobody paged for (a peek) goes back to the plain drift.
        a.set_scene(Scene::ExpHome, size, glow);
        assert_eq!(a.slide, 0.0);
    }

    #[test]
    fn squash_adds_motion_unless_reduced() {
        let mut a = Anim::new(100.0, 24.0);
        a.squash();
        assert!(a.rect.w.vel > 0.0 && a.rect.h.vel < 0.0);
        let mut b = Anim::new(100.0, 24.0);
        b.reduce = true;
        b.squash();
        assert_eq!((b.rect.w.vel, b.rect.h.vel), (0.0, 0.0));
    }

    #[test]
    fn accent_follows_the_colour_and_ignores_grey() {
        // Premultiplied BGRA, fully opaque: a red picture and a grey one.
        let red: Vec<u8> = (0..16).flat_map(|_| [20u8, 30, 220, 255]).collect();
        let c = accent_of(&red).expect("red has a hue");
        assert!(c[0] > 0.9 && c[1] < 0.3 && c[2] < 0.3, "{c:?}");
        let grey: Vec<u8> = (0..16).flat_map(|_| [128u8, 128, 128, 255]).collect();
        assert_eq!(accent_of(&grey), None);
        let black: Vec<u8> = (0..16).flat_map(|_| [0u8, 0, 0, 255]).collect();
        assert_eq!(accent_of(&black), None);
    }

    #[test]
    fn media_glow_takes_the_art_colour_only_when_asked() {
        let mut m = Model::default();
        m.art_accent = Some([0.9, 0.2, 0.2]);
        assert_eq!(m.glow(Scene::ExpMedia).0, [1.0; 3]);
        m.adaptive_glow = true;
        assert_eq!(m.glow(Scene::ExpMedia), ([0.9, 0.2, 0.2], 0.40));
        // Other scenes keep their own colours.
        assert_eq!(m.glow(Scene::ExpHome).0, [1.0; 3]);
    }

    #[test]
    fn output_line_is_kept_between_updates_and_adds_a_row_line() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Start, t0), t0);
        let plain = m.size(Scene::ExpTasks).1;
        let with_line = |line: Option<&str>| {
            let mut u = st("a", TaskEvent::Progress, t0);
            if let TaskUpdate::Upsert(s) = &mut u {
                s.line = line.map(str::to_owned);
            }
            u
        };
        m.apply_task(with_line(Some("compiling foo")), t0);
        assert_eq!(m.tasks[0].line.as_deref(), Some("compiling foo"));
        assert_eq!(m.size(Scene::ExpTasks).1, plain + LINE_H as f64);
        // A progress update without a line doesn't blank the ticker.
        m.apply_task(with_line(None), t0);
        assert_eq!(m.tasks[0].line.as_deref(), Some("compiling foo"));
        // A finished task has no ticker.
        m.apply_task(st("a", TaskEvent::Done, t0), t0);
        assert!(!m.tasks[0].shows_line());
    }

    #[test]
    fn a_second_activity_splits_off_as_a_bubble() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Start, t0), t0);
        assert_eq!(m.bubble(Scene::CompactTask), None);

        // Music playing beside a task.
        m.media = Some(MediaInfo {
            album: String::new(),
            title: "t".into(),
            artist: "a".into(),
            playing: true,
            app: String::new(),
            pos_ms: 0,
            dur_ms: 0,
        });
        assert_eq!(m.bubble(Scene::CompactTask), Some(Bubble::Media));
        // The timer outranks the music, and also joins compact media.
        m.timer = Some(crate::timer::Timer::start(
            crate::timer::TimerKind::Plain,
            5,
            0,
        ));
        assert_eq!(m.bubble(Scene::CompactTask), Some(Bubble::Timer));
        assert_eq!(m.bubble(Scene::CompactMedia), Some(Bubble::Timer));
        // Expanded scenes show these as panels instead.
        assert_eq!(m.bubble(Scene::ExpTasks), None);
        assert_eq!(m.bubble(Scene::Idle), None);
    }

    #[test]
    fn a_bubble_opens_its_own_pane_and_sits_right_of_the_pill() {
        assert_eq!(Bubble::Timer.panel(), Panel::Timer);
        assert_eq!(Bubble::Media.panel(), Panel::Media);
        // Out (s = 1) it starts 8 px past the pill's right edge; at 0 it is tucked behind it.
        assert_eq!(Bubble::Timer.x(292.0, 1.0), 292.0 + 8.0);
        assert!(Bubble::Media.x(292.0, 0.0) < 292.0);
    }

    #[test]
    fn bubble_room_only_while_it_is_out() {
        let mut a = Anim::new(100.0, 24.0);
        assert_eq!(a.bubble_room(), 0.0);
        a.set_bubble(Some(Bubble::Timer));
        a.step(0.05);
        assert_eq!(a.bubble_room(), BUBBLE_ROOM);
        // The kind survives while it retracts.
        a.set_bubble(None);
        assert_eq!(a.bubble_kind, Some(Bubble::Timer));
    }

    #[test]
    fn a_chip_keeps_its_own_hold_time_and_the_kill_confirm_dies_with_it() {
        let t0 = Instant::now();
        let mut m = Model::default();
        let mut c = Chip::new("hog", t0);
        c.hold = Duration::from_secs(20);
        m.chip = Some(c);
        m.chip_armed = Some(t0 + KILL_CONFIRM);
        // The single deadline timer wakes for the confirm window first, then the chip's end.
        assert_eq!(m.expire(t0), Some(t0 + KILL_CONFIRM));
        assert!(m.chip.is_some() && m.chip_armed.is_some());
        assert_eq!(
            m.expire(t0 + KILL_CONFIRM),
            Some(t0 + Duration::from_secs(20))
        );
        assert!(m.chip.is_some() && m.chip_armed.is_none());
        // Past the default 10 s the long-hold chip is still there; at 20 s it goes.
        m.expire(t0 + Duration::from_secs(11));
        assert!(m.chip.is_some());
        m.expire(t0 + Duration::from_secs(20));
        assert!(m.chip.is_none());
        // Dismissing a chip also drops a pending Kill confirm.
        m.chip = Some(Chip::new("x", t0));
        m.chip_armed = Some(t0 + Duration::from_secs(1));
        m.dismiss_chip();
        m.expire(t0);
        assert!(m.chip_armed.is_none());
    }

    #[test]
    fn calendar_heads_up_comes_due_once() {
        let t0 = Instant::now();
        let ev = || crate::calendar::NextEvent {
            subject: "Standup".into(),
            start_ms: 1_000_000_000 + 30 * 60_000,
            url: None,
        };
        let wall = 1_000_000_000;
        let lead = 10 * 60_000;
        let mut m = Model::default();
        m.set_calendar(Some(ev()), wall, lead, t0);
        // Due 20 minutes from now, and the single deadline timer knows.
        assert_eq!(m.calendar_due(t0), None);
        assert_eq!(m.expire(t0), Some(t0 + Duration::from_secs(20 * 60)));
        assert_eq!(
            m.calendar_due(t0 + Duration::from_secs(20 * 60))
                .map(|e| e.subject),
            Some("Standup".into())
        );
        // Shown: the same event coming back from the worker doesn't re-arm it.
        m.set_calendar(Some(ev()), wall, lead, t0);
        assert!(m.calendar.is_none());
        // A different event does; and one already inside its lead time is due at once.
        let mut next = ev();
        next.subject = "Review".into();
        next.start_ms = wall + 4 * 60_000;
        m.set_calendar(Some(next), wall, lead, t0);
        assert!(m.calendar_due(t0).is_some());
        // Nothing upcoming clears it.
        m.set_calendar(None, wall, lead, t0);
        assert!(m.calendar.is_none());
    }
    #[test]
    fn a_stopped_command_is_not_an_error() {
        let t0 = Instant::now();
        for code in [130, 143, -1_073_741_510] {
            let mut m = Model::default();
            m.apply_task(shell("a", "dev", TaskEvent::Start, 0, t0), t0);
            let mut end = shell("a", "dev", TaskEvent::Failed, 0, t0);
            if let TaskUpdate::Upsert(s) = &mut end {
                s.exit_code = Some(code);
            }
            m.apply_task(end, t0 + Duration::from_secs(9));
            assert!(m.tasks.is_empty(), "exit {code} left a row");
        }
    }

    #[test]
    fn a_failed_row_goes_when_its_terminal_closes() {
        let t0 = Instant::now();
        let mut m = Model::default();
        m.apply_task(st("a", TaskEvent::Failed, t0), t0);
        m.apply_task(st("b", TaskEvent::Start, t0), t0);
        m.apply_task(TaskUpdate::Gone("a".into()), t0);
        m.apply_task(TaskUpdate::Gone("b".into()), t0);
        // Only a failed row is dropped; a running one is the registry's to mark Lost.
        assert_eq!(m.tasks.len(), 1);
        assert_eq!(m.tasks[0].id, "b");
    }

    #[test]
    fn the_task_list_scrolls_past_four_rows() {
        let t0 = Instant::now();
        let mut m = Model::default();
        for i in 0..7 {
            m.apply_task(st(&format!("t{i}"), TaskEvent::Start, t0), t0);
        }
        assert_eq!(m.rows().len(), 4);
        assert_eq!(m.rows_hidden(), (0, 3));
        assert!(m.scroll_tasks(10));
        assert!(!m.scroll_tasks(1), "at the end: the wheel pages instead");
        assert_eq!(m.rows_hidden(), (3, 0));
        assert_eq!(m.rows().len(), 4);
        m.scroll_tasks(-1);
        assert_eq!(m.rows_hidden(), (2, 1));
        // Rows leaving keeps the window valid.
        m.task_first = 9;
        assert_eq!(m.rows().len(), 4);
    }
}
