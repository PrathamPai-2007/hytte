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
    Thumb(u64, Option<Arc<ArtBitmap>>),
    SetClipboard(String),
    DragEnter,
    DragLeave,
    /// `config.toml` was edited and parses.
    Config(Box<crate::config::Config>),
    /// An agent asks permission (`hytte agent ask`); send the answer on `reply`.
    Ask {
        id: u64,
        summary: String,
        reply: crossbeam_channel::Sender<bool>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
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
        matches!(self.source.as_str(), "run" | "shell") && self.cwd.is_some()
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
        }
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
    KillPid(u32),
    /// Stop alerting about a process (by exe name) for this session.
    IgnoreExe(String),
    /// Answer the pending agent ask with this id: allow (true) or deny.
    Answer(u64, bool),
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
}

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

impl Model {
    pub fn apply_task(&mut self, u: TaskUpdate, now: Instant) {
        self.now = Some(now);
        match u {
            TaskUpdate::Upsert(s) => self.upsert(s, now),
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
        if s.source == "agent" && s.event == TaskEvent::Done && !self.finish_peek.is_zero() {
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
        // Shell command that ended before the threshold, or was Ctrl-C'd: never shown.
        if shell && terminal {
            let hidden = existing
                .map(|i| now < self.tasks[i].visible_after)
                .unwrap_or(true);
            if hidden || s.exit_code == Some(130) {
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
        }
        self.media = m;
        self.media_stamp = Some(now);
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
        self.tasks.retain(|t| match t.event {
            TaskEvent::Done => away || now.duration_since(from(t.changed)) < SUCCESS_HOLD,
            TaskEvent::Lost => away || now.duration_since(from(t.changed)) < LOST_HOLD,
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
        let mut next: Option<Instant> = None;
        let mut keep = |i: Instant| next = Some(next.map_or(i, |n| n.min(i)));
        for t in &self.tasks {
            match t.event {
                TaskEvent::Done | TaskEvent::Lost if away => keep(now + AWAY_POLL),
                TaskEvent::Done => keep(from(t.changed) + SUCCESS_HOLD),
                TaskEvent::Lost => keep(from(t.changed) + LOST_HOLD),
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
    pub fn rows(&self) -> Vec<&TaskView> {
        let mut v: Vec<&TaskView> = self.tasks.iter().filter(|t| self.visible(t)).collect();
        v.sort_by(|a, b| Self::row_order(a, b));
        v.truncate(4);
        v
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
        let peeking = self.now.zip(self.peek_until).is_some_and(|(n, p)| n < p);
        if hover || peeking {
            return match self.panel() {
                Panel::Tasks => Scene::ExpTasks,
                Panel::Shelf => Scene::ExpShelf,
                Panel::Media => Scene::ExpMedia,
                Panel::Timer => Scene::ExpTimer,
                Panel::Power => Scene::ExpPower,
                Panel::Home => Scene::ExpHome,
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
            Scene::ExpDrop => (380.0, 116.0),
            Scene::ExpChip if self.chip.as_ref().is_some_and(Chip::one_line) => (380.0, 80.0),
            Scene::ExpChip => (380.0, 96.0),
            Scene::ExpTimerDone => (380.0, 114.0),
            Scene::ExpCharge => (300.0, 52.0),
            Scene::ExpSettings => (380.0, 216.0),
            // Running: ring + controls. Not running: the setup card.
            Scene::ExpTimer if self.timer.is_some() => (380.0, 96.0 + tab),
            Scene::ExpTimer => (380.0, 152.0 + tab),
            Scene::ExpShelf if self.shelf_sel.is_some() => (380.0, 136.0 + tab),
            Scene::ExpShelf => (380.0, 118.0 + tab),
            Scene::ExpTasks => {
                let rows = self.rows();
                let mut h = 22.0 + rows.len() as f64 * 34.0 + tab + self.servers_h();
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
                (380.0, h.min(290.0))
            }
        }
    }

    /// Glow colour (rgb 0..1) and strength for the scene.
    pub fn glow(&self, scene: Scene) -> ([f32; 3], f32) {
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
            Scene::CompactMedia | Scene::ExpMedia | Scene::ExpHome | Scene::ExpSettings => {
                ([1.0; 3], 0.10)
            }
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
            content: Spring::unit(1.0, 1.0, 0.18),
            hover: Spring::unit(0.0, 1.0, 0.16),
            vis,
            t: 0.0,
            scene: Scene::Idle,
            prev: Scene::Idle,
            out: Spring::unit(0.0, 1.0, 0.16),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn panels_and_selection() {
        // (a running timer adds its own pane)
        let mut m = Model::default();
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
        let mut m = Model::default();
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
}
