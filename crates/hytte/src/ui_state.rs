//! Pure UI model + animation state (no Win32), so scene selection, task
//! lifecycle and expiry are unit-testable. The renderer reads this; the
//! window layer feeds it events and clocks.

use crate::animation::{RectSpring, Spring};
use crate::tasks::{TaskState, TaskUpdate};
use hytte_proto::TaskEvent;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SUCCESS_HOLD: Duration = Duration::from_secs(3);
pub const LOST_HOLD: Duration = Duration::from_secs(5);
pub const CHIP_HOLD: Duration = Duration::from_secs(10);

/// Events marshalled from worker threads to the UI thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Task(TaskUpdate),
    Media(Option<MediaInfo>),
    MediaArt(Option<Arc<ArtBitmap>>),
    Privacy(bool, bool, Option<String>),
    DropDone(crate::drop::DropResult),
    SetClipboard(String),
    DragEnter,
    DragLeave,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub playing: bool,
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
    pub stderr: Vec<String>,
    pub started: Instant,
    pub changed: Instant,
}

impl TaskView {
    pub fn running(&self) -> bool {
        matches!(self.event, TaskEvent::Start | TaskEvent::Progress)
    }
    pub fn failed(&self) -> bool {
        self.event == TaskEvent::Failed
    }
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
    ExpHome,
    ExpDrop,
    ExpChip,
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
    pub drop_over: bool,
    pub chip: Option<Chip>,
}

impl Model {
    pub fn apply_task(&mut self, u: TaskUpdate, now: Instant) {
        match u {
            TaskUpdate::Upsert(s) => self.upsert(s, now),
            TaskUpdate::Lost(id) => {
                if let Some(t) = self.tasks.iter_mut().find(|t| t.id == id) {
                    t.event = TaskEvent::Lost;
                    t.changed = now;
                }
            }
            TaskUpdate::Prune => {}
        }
    }

    fn upsert(&mut self, s: TaskState, now: Instant) {
        let stderr: Vec<String> = s
            .stderr_tail
            .as_deref()
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == s.task_id) {
            if !s.label.is_empty() {
                t.label = s.label;
            }
            t.event = s.event;
            t.progress = s.progress.or(t.progress);
            t.duration_ms = s.duration_ms;
            t.stderr = stderr;
            t.changed = now;
        } else {
            self.tasks.push(TaskView {
                id: s.task_id,
                label: s.label,
                event: s.event,
                progress: s.progress,
                duration_ms: s.duration_ms,
                stderr,
                started: now,
                changed: now,
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

    /// Drop finished tasks / chips past their hold time. Returns the next
    /// instant something expires, so the caller can arm a single timer.
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.tasks.retain(|t| match t.event {
            TaskEvent::Done => now.duration_since(t.changed) < SUCCESS_HOLD,
            TaskEvent::Lost => now.duration_since(t.changed) < LOST_HOLD,
            _ => true, // running + failed persist
        });
        if let Some(c) = &self.chip {
            if now.duration_since(c.since) >= CHIP_HOLD {
                self.chip = None;
            }
        }
        let mut next: Option<Instant> = None;
        let mut keep = |i: Instant| next = Some(next.map_or(i, |n| n.min(i)));
        for t in &self.tasks {
            match t.event {
                TaskEvent::Done => keep(t.changed + SUCCESS_HOLD),
                TaskEvent::Lost => keep(t.changed + LOST_HOLD),
                _ => {}
            }
        }
        if let Some(c) = &self.chip {
            keep(c.since + CHIP_HOLD);
        }
        next
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

    /// Rows for the expanded task list: failed, running, then finished.
    pub fn rows(&self) -> Vec<&TaskView> {
        let mut v: Vec<&TaskView> = self.tasks.iter().collect();
        let rank = |t: &TaskView| match t.event {
            TaskEvent::Failed => 0,
            TaskEvent::Start | TaskEvent::Progress => 1,
            _ => 2,
        };
        v.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.changed.cmp(&a.changed)));
        v.truncate(4);
        v
    }

    /// The task shown in the compact pill.
    pub fn primary(&self) -> Option<&TaskView> {
        self.rows().into_iter().next()
    }

    pub fn scene(&self, hover: bool, suppressed: bool) -> Scene {
        if suppressed && !hover {
            return Scene::Sentinel;
        }
        if self.drop_over {
            return Scene::ExpDrop;
        }
        if self.chip.is_some() {
            return Scene::ExpChip;
        }
        if hover {
            if !self.tasks.is_empty() {
                Scene::ExpTasks
            } else if self.media.is_some() {
                Scene::ExpMedia
            } else {
                Scene::ExpHome
            }
        } else if !self.tasks.is_empty() {
            Scene::CompactTask
        } else if self.media.as_ref().is_some_and(|m| m.playing) {
            Scene::CompactMedia
        } else {
            Scene::Idle
        }
    }

    /// Target pill size in logical px.
    pub fn size(&self, scene: Scene) -> (f64, f64) {
        match scene {
            Scene::Sentinel => (96.0, 2.5),
            Scene::Idle => (if self.privacy_active() { 150.0 } else { 120.0 }, 24.0),
            Scene::CompactTask => (292.0, 32.0),
            Scene::CompactMedia => (240.0, 32.0),
            Scene::ExpMedia => (380.0, 128.0),
            Scene::ExpHome => (320.0, 84.0),
            Scene::ExpDrop => (380.0, 116.0),
            Scene::ExpChip => (380.0, 96.0),
            Scene::ExpTasks => {
                let rows = self.rows();
                let mut h = 22.0 + rows.len() as f64 * 34.0;
                if let Some(f) = rows.iter().find(|t| t.failed()) {
                    h += f.stderr.len().min(4) as f64 * 15.0 + 34.0;
                }
                (380.0, h.min(280.0))
            }
        }
    }

    /// Glow colour (rgb 0..1) and strength for the scene.
    pub fn glow(&self, scene: Scene) -> ([f32; 3], f32) {
        const RED: [f32; 3] = [1.0, 0.35, 0.37];
        const GREEN: [f32; 3] = [0.20, 0.82, 0.48];
        const BLUE: [f32; 3] = [0.35, 0.63, 1.0];
        const PURPLE: [f32; 3] = [0.65, 0.54, 0.98];
        match scene {
            Scene::Sentinel | Scene::Idle => ([1.0; 3], 0.0),
            Scene::ExpDrop => (PURPLE, 1.0),
            Scene::ExpChip => (PURPLE, 0.6),
            Scene::CompactMedia | Scene::ExpMedia | Scene::ExpHome => ([1.0; 3], 0.10),
            Scene::CompactTask | Scene::ExpTasks => match self.primary().map(|t| t.event) {
                Some(TaskEvent::Failed) => (RED, 0.9),
                Some(TaskEvent::Done) => (GREEN, 0.6),
                Some(TaskEvent::Lost) => ([0.7; 3], 0.3),
                _ => (BLUE, 0.45),
            },
        }
    }

    /// True while something needs a steady (ambient) frame clock.
    pub fn ambient(&self, scene: Scene) -> bool {
        self.tasks.iter().any(|t| t.running() || t.failed())
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
            stderr_tail: Some("boom\nbang".into()),
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
}
