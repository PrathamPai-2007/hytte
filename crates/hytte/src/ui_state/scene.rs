//! Which pane or card the pill shows, how big it is and how it glows: `Scene`, `Panel`, `Bubble` and
//! the scene-selection part of `Model`.

use super::*;

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
    /// One of your `[[script_pane]]`s, by its place in the config (0..4).
    Script(u8),
}

/// Script pane keys in `[panes] order`, by slot (the pane's place among `[[script_pane]]`s).
pub const SCRIPT_KEYS: [&str; crate::config::MAX_SCRIPT_PANES] =
    ["script1", "script2", "script3", "script4"];

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
            // The tab shows the pane's own name ([`Model::panel_label`]).
            Panel::Script(_) => "Script",
        }
    }

    /// Every pane, in the order the tab strip has them until the user moves them.
    pub const ALL: [Panel; 13] = [
        Panel::Shelf,
        Panel::Media,
        Panel::Home,
        Panel::Notes,
        Panel::Todo,
        Panel::Weather,
        Panel::Tasks,
        Panel::Timer,
        Panel::Power,
        Panel::Script(0),
        Panel::Script(1),
        Panel::Script(2),
        Panel::Script(3),
    ];

    /// The pane's name in `[panes]` (the switch `panes.<key>` and the `order` list).
    pub fn key(self) -> &'static str {
        match self {
            Panel::Tasks => "tasks",
            Panel::Shelf => "shelf",
            Panel::Media => "media",
            Panel::Timer => "timer",
            Panel::Power => "power",
            Panel::Home => "home",
            Panel::Notes => "notes",
            Panel::Todo => "todo",
            Panel::Weather => "weather",
            Panel::Script(i) => SCRIPT_KEYS[(i as usize).min(SCRIPT_KEYS.len() - 1)],
        }
    }

    pub fn from_key(s: &str) -> Option<Panel> {
        Panel::ALL.into_iter().find(|p| p.key() == s.trim())
    }

    /// All panes in the user's order: those named in `order` first (as written, unknown names
    /// and repeats ignored), then the rest in their usual order.
    pub fn arranged(order: &str) -> Vec<Panel> {
        let mut v: Vec<Panel> = vec![];
        for p in order.split(',').filter_map(Panel::from_key) {
            if !v.contains(&p) {
                v.push(p);
            }
        }
        let rest: Vec<Panel> = Panel::ALL.into_iter().filter(|p| !v.contains(p)).collect();
        v.extend(rest);
        v
    }

    /// From a pane's name ("To-do") or, for script panes, its key ("script2").
    pub fn from_name(s: &str) -> Option<Panel> {
        Panel::ALL
            .into_iter()
            .find(|p| !matches!(p, Panel::Script(_)) && p.name() == s.trim())
            .or_else(|| Panel::from_key(s).filter(|p| matches!(p, Panel::Script(_))))
    }

    /// What the pane is saved as between runs: its name, or a script pane's key.
    pub fn saved_name(self) -> &'static str {
        match self {
            Panel::Script(_) => self.key(),
            _ => self.name(),
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
            Scene::ExpNotes => Panel::Notes,
            Scene::ExpTodo => Panel::Todo,
            Scene::ExpWeather => Panel::Weather,
            Scene::ExpScript(i) => Panel::Script(i),
            _ => return None,
        })
    }
}

/// Height of the key hint line at the bottom of the open pill in keyboard mode.
pub const KB_HINT_H: f32 = 20.0;

/// Scenes that show the key hints in keyboard mode: the open pill, not the bare or compact
/// one, nor the brief cards that take no keys.
pub fn hinted(scene: Scene) -> bool {
    !matches!(
        scene,
        Scene::Sentinel
            | Scene::Idle
            | Scene::CompactTask
            | Scene::CompactMedia
            | Scene::ExpCharge
            | Scene::ExpDrop
    )
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
    /// A script pane, by slot.
    ExpScript(u8),
    ExpDrop,
    ExpChip,
    ExpTimer,
    ExpTimerDone,
    /// The charger was just plugged in.
    ExpCharge,
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

/// The settings pane's groups, as its segmented header shows them.
/// `order` with the pane `key` moved `dir` places (-1 earlier, 1 later) among all nine; the
/// whole new order, ready to write back.
pub fn move_pane(order: &str, key: &str, dir: i8) -> String {
    let mut v = Panel::arranged(order);
    if let Some(i) = Panel::from_key(key).and_then(|p| v.iter().position(|q| *q == p)) {
        let j = (i as i32 + dir as i32).clamp(0, v.len() as i32 - 1) as usize;
        v.swap(i, j);
    }
    v.iter().map(|p| p.key()).collect::<Vec<_>>().join(",")
}

impl Model {
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

    pub fn panels(&self) -> Vec<Panel> {
        let mut v = vec![];
        if self.panes.shelf && !self.shelf.is_empty() {
            v.push(Panel::Shelf);
        }
        if self.panes.media && self.media.is_some() {
            v.push(Panel::Media);
        }
        // The quick controls are the home pane: they cannot be switched off.
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
        if self.panes.tasks && (self.visible_count() > 0 || !self.ports.is_empty()) {
            v.push(Panel::Tasks);
        }
        if self.panes.timer && self.timer.is_some() {
            v.push(Panel::Timer);
        }
        if self.panes.power && self.power.is_some() {
            v.push(Panel::Power);
        }
        v.extend(
            (0..self.scripts.len())
                .filter(|i| self.panes.script_on(*i))
                .map(|i| Panel::Script(i as u8)),
        );
        let order = Panel::arranged(&self.panes.order);
        v.sort_by_key(|p| order.iter().position(|q| q == p));
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
            // Unpicked: what the collapsed pill shows (a task, music), else the quick controls,
            // else the first pane left.
            _ => {
                let want = if self.visible_count() > 0 {
                    Panel::Tasks
                } else if self.media.as_ref().is_some_and(|m| m.playing) {
                    Panel::Media
                } else {
                    Panel::Home
                };
                if ps.contains(&want) {
                    want
                } else {
                    ps[0]
                }
            }
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
                Panel::Script(i) => Scene::ExpScript(i),
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
    /// The pill's size for `scene`, with room for the key hints in keyboard mode.
    pub fn size(&self, scene: Scene) -> (f64, f64) {
        let (w, h) = self.scene_size(scene);
        if self.keyboard && hinted(scene) {
            (w, h + KB_HINT_H as f64)
        } else {
            (w, h)
        }
    }

    /// The key hint line for keyboard mode. A card takes no arrows (they move the pane under
    /// it), so its line leaves them out.
    pub fn kb_hint(&self) -> &'static str {
        if self.typing.is_some() {
            "Enter confirm  ·  Esc stop typing"
        } else if self.chip.is_some() {
            "Tab buttons  ·  Enter press  ·  Esc close"
        } else {
            "← → panes  ·  Tab buttons  ·  Enter press  ·  Esc close"
        }
    }

    pub(super) fn scene_size(&self, scene: Scene) -> (f64, f64) {
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
            Scene::ExpScript(i) => (380.0, self.script_h(i) + tab),
            Scene::ExpDrop => (380.0, 116.0),
            Scene::ExpChip if self.chip.as_ref().is_some_and(Chip::one_line) => (380.0, 80.0),
            Scene::ExpChip => (380.0, 96.0),
            Scene::ExpTimerDone if self.focus_summary.is_some() => (380.0, 134.0),
            Scene::ExpTimerDone => (380.0, 114.0),
            Scene::ExpCharge => (300.0, 52.0),
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

    pub(super) fn glow_base(&self, scene: Scene) -> ([f32; 3], f32) {
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
            | Scene::ExpNotes
            | Scene::ExpTodo
            | Scene::ExpWeather
            | Scene::ExpScript(_) => ([1.0; 3], 0.10),
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
