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
pub const CHIP_HOLD: Duration = Duration::from_secs(10);
/// Width the red mute lock takes from the pill's right edge.
pub const LOCK_W: f32 = 26.0;
pub const KILL_CONFIRM: Duration = Duration::from_secs(3);
/// How long the timer-finished card waits before folding away on its own.
pub const TIMER_DONE_HOLD: Duration = Duration::from_secs(30);
/// How long a failed task's red pulse takes to fade to rest.
pub const FAIL_PULSE: Duration = Duration::from_secs(8);

/// Events marshalled from worker threads to the UI thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Task(TaskUpdate),
    Media(Option<MediaInfo>),
    MediaArt(Option<Arc<ArtBitmap>>),
    Privacy(bool, bool, Option<String>),
    MicMute(bool),
    Power(Option<crate::power::Battery>),
    DropDone(crate::drop::DropResult),
    Ports(Vec<PortInfo>),
    /// Dropped onto the notch while the shelf is the drop target.
    ShelfAdd(crate::drop::DropJob),
    Thumb(u64, Option<Arc<ArtBitmap>>),
    SetClipboard(String),
    DragEnter,
    DragLeave,
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
    pub fn failed(&self) -> bool {
        self.event == TaskEvent::Failed
    }
    pub fn needs_input(&self) -> bool {
        self.event == TaskEvent::NeedsInput
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
    Ports,
    Timer,
    Home,
}

#[derive(Debug, Clone)]
pub struct Chip {
    pub summary: String,
    pub open: Option<PathBuf>,
    pub copy: Option<String>,
    pub since: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Sentinel,
    Idle,
    CompactTask,
    CompactMedia,
    ExpTasks,
    ExpMedia,
    ExpPorts,
    ExpShelf,
    ExpHome,
    ExpDrop,
    ExpChip,
    ExpTimer,
    ExpTimerDone,
}

#[derive(Default)]
pub struct Model {
    pub tasks: Vec<TaskView>,
    pub media: Option<MediaInfo>,
    pub media_stamp: Option<Instant>,
    pub art: Option<Arc<ArtBitmap>>,
    pub art_gen: u32,
    pub cam: bool,
    pub mic: bool,
    pub privacy_app: Option<String>,
    pub mic_muted: bool,
    /// None on machines without a battery.
    pub power: Option<crate::power::Battery>,
    /// A user-initiated drop keeps the pill visible even while fullscreen suppresses it.
    pub force_until: Option<Instant>,
    pub drop_over: bool,
    pub chip: Option<Chip>,
    pub ports: Vec<PortInfo>,
    /// Port whose Kill button is armed ("Kill?") and until when.
    pub kill_armed: Option<(u16, Instant)>,
    pub shelf: Vec<ShelfItem>,
    pub shelf_sel: Option<u64>,
    pub selected: Option<Panel>,
    pub peek_until: Option<Instant>,
    /// Last time the model was advanced; visibility rules compare against it.
    pub now: Option<Instant>,
    /// Lower-cased command names the daemon refuses to show (shell hooks).
    pub ignore: Vec<String>,
    pub timer: Option<crate::timer::Timer>,
    /// Right-click panel (set or inspect the timer).
    pub timer_panel: bool,
    /// Minutes the panel will start with (wheel / presets).
    pub timer_min: u32,
    pub timer_done: Option<(crate::timer::TimerKind, Instant)>,
    /// Length of the break offered after a finished focus session.
    pub timer_break: u32,
    /// Focus sessions finished, for long-break spacing.
    pub focus_done: u32,
}

/// First word of a command line, lower-cased, without path or `.exe`.
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
        if shell && self.ignore.contains(&command_name(&s.label)) {
            return;
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
            if !s.label.is_empty() {
                t.label = s.label;
            }
            t.event = s.event;
            t.progress = s.progress.or(t.progress);
            t.duration_ms = s.duration_ms;
            t.exit_code = s.exit_code;
            t.stderr = stderr;
            t.attention = attention;
            t.pid = s.pid.or(t.pid);
            t.changed = now;
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
                started: now,
                changed: now,
                visible_after: now + Duration::from_millis(s.delay_ms as u64),
            });
        }
    }

    pub fn apply_media(&mut self, m: Option<MediaInfo>, now: Instant) {
        if m.is_none() {
            self.art = None;
        }
        self.media = m;
        self.media_stamp = Some(now);
    }

    pub fn set_art(&mut self, a: Option<Arc<ArtBitmap>>) {
        self.art = a;
        self.art_gen = self.art_gen.wrapping_add(1);
    }

    pub fn set_ports(&mut self, p: Vec<PortInfo>) {
        self.ports = p;
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

    /// Drop finished tasks / chips past their hold time. Returns the next
    /// instant something changes, so the caller can arm a single timer.
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.now = Some(now);
        self.tasks.retain(|t| match t.event {
            TaskEvent::Done => now.duration_since(t.changed) < SUCCESS_HOLD,
            TaskEvent::Lost => now.duration_since(t.changed) < LOST_HOLD,
            _ => true, // running, waiting and failed persist
        });
        if self
            .chip
            .as_ref()
            .is_some_and(|c| now.duration_since(c.since) >= CHIP_HOLD)
        {
            self.chip = None;
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
                TaskEvent::Done => keep(t.changed + SUCCESS_HOLD),
                TaskEvent::Lost => keep(t.changed + LOST_HOLD),
                _ => {}
            }
            if t.visible_after > now {
                keep(t.visible_after);
            }
        }
        if let Some(c) = &self.chip {
            keep(c.since + CHIP_HOLD);
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
        if let Some((_, s)) = self.timer_done {
            keep(s + TIMER_DONE_HOLD);
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
        Some(t.kind)
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

    pub fn visible_count(&self) -> usize {
        self.tasks.iter().filter(|t| self.visible(t)).count()
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

    /// Panels that currently have something to show, in priority order.
    pub fn panels(&self) -> Vec<Panel> {
        let mut v = vec![];
        if self.visible_count() > 0 {
            v.push(Panel::Tasks);
        }
        if self.timer.is_some() {
            v.push(Panel::Timer);
        }
        if !self.shelf.is_empty() {
            v.push(Panel::Shelf);
        }
        if self.media.is_some() {
            v.push(Panel::Media);
        }
        if !self.ports.is_empty() {
            v.push(Panel::Ports);
        }
        // System card (mic, battery) is always reachable.
        v.push(Panel::Home);
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
        self.drop_over || self.now.zip(self.force_until).is_some_and(|(n, f)| n < f)
    }

    pub fn panel(&self) -> Panel {
        let ps = self.panels();
        match self.selected {
            Some(p) if ps.contains(&p) => p,
            _ => ps[0],
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
        let peeking = self.now.zip(self.peek_until).is_some_and(|(n, p)| n < p);
        if hover || peeking {
            return match self.panel() {
                Panel::Tasks => Scene::ExpTasks,
                Panel::Shelf => Scene::ExpShelf,
                Panel::Media => Scene::ExpMedia,
                Panel::Ports => Scene::ExpPorts,
                Panel::Timer => Scene::ExpTimer,
                Panel::Home => Scene::ExpHome,
            };
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
        let tab = if self.tabs() { 14.0 } else { 0.0 };
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
            Scene::ExpMedia => (380.0, 128.0 + tab),
            Scene::ExpHome => (380.0, if self.power.is_some() { 132.0 } else { 86.0 } + tab),
            Scene::ExpDrop => (380.0, 116.0),
            Scene::ExpChip | Scene::ExpTimerDone => (380.0, 96.0),
            Scene::ExpTimer => (380.0, 92.0 + tab),
            Scene::ExpPorts => (380.0, 20.0 + self.ports.len().min(5) as f64 * 34.0 + tab),
            Scene::ExpShelf => (380.0, 118.0 + tab),
            Scene::ExpTasks => {
                let rows = self.rows();
                let mut h = 22.0 + rows.len() as f64 * 34.0 + tab;
                for t in &rows {
                    if t.needs_input() {
                        h += 20.0;
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
                Scene::Idle
                    | Scene::CompactMedia
                    | Scene::ExpHome
                    | Scene::ExpMedia
                    | Scene::ExpPorts
            )
        {
            return (RED, 0.8);
        }
        match scene {
            Scene::Sentinel | Scene::Idle => ([1.0; 3], 0.0),
            Scene::ExpDrop => (PURPLE, 1.0),
            Scene::ExpTimer => (BLUE, 0.30),
            Scene::ExpTimerDone => (GREEN, 0.8),
            Scene::ExpChip | Scene::ExpShelf => (PURPLE, 0.5),
            Scene::CompactMedia | Scene::ExpMedia | Scene::ExpHome | Scene::ExpPorts => {
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
    pub reduce: bool,
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
            reduce: false,
        }
    }

    pub fn set_scene(&mut self, scene: Scene, size: (f64, f64), glow: ([f32; 3], f32)) {
        if scene != self.scene {
            self.scene = scene;
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
        }
    }

    /// Advance everything; true while any spring is still moving.
    pub fn step(&mut self, dt: f64) -> bool {
        self.t += dt;
        if self.reduce {
            self.hover.snap();
            self.vis.snap();
            return false;
        }
        let a = self.rect.advance(dt);
        let b = self.glow.advance(dt);
        let c = self.content.advance(dt);
        let d = self.hover.advance(dt);
        let e = self.vis.advance(dt);
        a || b || c || d || e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(m.panels(), vec![Panel::Shelf, Panel::Ports, Panel::Home]);
        assert!(m.tabs());
        assert_eq!(m.panel(), Panel::Shelf);
        m.selected = Some(Panel::Ports);
        assert_eq!(m.panel(), Panel::Ports);
        m.ports.clear();
        assert_eq!(m.panel(), Panel::Shelf, "stale selection falls back");
    }

    #[test]
    fn wheel_steps_and_wraps() {
        let mut m = Model::default();
        m.ports = vec![PortInfo {
            port: 3000,
            pid: 9,
            exe: "node.exe".into(),
        }];
        assert_eq!(m.panels(), vec![Panel::Ports, Panel::Home]);
        assert_eq!(m.step_panel(1), Panel::Home);
        assert_eq!(m.step_panel(1), Panel::Ports, "wraps forward");
        assert_eq!(m.step_panel(-1), Panel::Home, "wraps backward");
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
}
