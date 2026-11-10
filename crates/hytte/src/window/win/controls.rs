//! Clipboard, timer, panels, scrubbing, volume, wheel, fullscreen and game mode.

use super::*;

impl Ui {
    /// The clipboard changed: a screenshot from the Snipping Tool (Win+Shift+S) goes on
    /// the shelf as a PNG. Other apps' images are left alone, and nothing is shelved over
    /// a game or while paused (the shelf would pop open).
    pub(super) fn on_clipboard(&mut self, hwnd: HWND) {
        const CF_DIB: u32 = 8;
        if !self.cfg.shelf.screenshots || self.gaming || self.paused {
            return;
        }
        let mut pid = 0u32;
        unsafe {
            let Ok(owner) = GetClipboardOwner() else {
                return;
            };
            GetWindowThreadProcessId(owner, Some(&mut pid));
        }
        let snip = crate::proc::exe_name(pid).is_some_and(|e| {
            let e = e.to_ascii_lowercase();
            e == "snippingtool.exe" || e == "screenclippinghost.exe"
        });
        if !snip || unsafe { IsClipboardFormatAvailable(CF_DIB) }.is_err() {
            return;
        }
        let Some(dib) = (unsafe { clipboard_bytes(hwnd, CF_DIB) }) else {
            return;
        };
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            dib.hash(&mut h);
            h.finish()
        };
        self.snip_seq = unsafe { GetClipboardSequenceNumber() };
        // The same picture again (the Snipping Tool touches the clipboard more than once):
        // not a new screenshot, but still the one on the clipboard, so a paste still counts.
        if std::mem::replace(&mut self.last_snip, hash) == hash {
            return;
        }
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            if let Some(it) = crate::shelf::stage_dib(&dib) {
                push_event(&sh, UiEvent::Screenshot(it));
            }
        });
    }

    pub(super) fn start_timer(&mut self, kind: crate::timer::TimerKind, minutes: u32) {
        let t = crate::timer::Timer::start(kind, minutes, crate::timer::now_ms());
        crate::timer::save(Some(&t));
        self.model.timer = Some(t);
        self.model.timer_done = None;
        self.model.timer_panel = false;
    }

    pub(super) fn select_panel(&mut self, p: Panel) {
        // Page direction for the slide: towards a later panel is "next".
        let ps = self.model.panels();
        let at = |q: Panel| ps.iter().position(|x| *x == q);
        self.anim.nav = match (at(self.model.panel()), at(p)) {
            (Some(a), Some(b)) if b > a => 1.0,
            (Some(a), Some(b)) if b < a => -1.0,
            _ => 0.0,
        };
        self.model.timer_panel = false;
        self.model.close_media_extras();
        self.model.selected = Some(p);
        match p {
            Panel::Tasks => crate::ports::request_refresh(),
            Panel::Power => self.model.power = crate::power::read().or(self.model.power),
            Panel::Home if self.model.apps_open => self.model.apps = crate::mic::apps(),
            _ => {}
        }
    }

    /// Set the volume from where the pointer is on the bar being dragged.
    pub(super) fn scrubbing(&self, what: &Action) -> bool {
        self.scrub.as_ref().is_some_and(|(a, _, _)| a == what)
    }

    pub(super) fn scrub_to(&mut self) {
        let (Some((what, l, r)), Some((mx, _))) = (self.scrub.clone(), self.mouse_logical()) else {
            return;
        };
        if what == Action::Seek {
            // The bar sits 4 px inside its hit box. Shown at once; sought on release.
            let f = ((mx - l - 4.0) / (r - l - 8.0)).clamp(0.0, 1.0) as f64;
            if let Some(md) = self.model.media.as_mut() {
                md.pos_ms = (md.dur_ms as f64 * f) as u64;
                self.model.media_stamp = Some(Instant::now());
            }
            self.kick();
            return;
        }
        let f = ((mx - l - 6.0) / (r - l - 12.0)).clamp(0.0, 1.0);
        let f = (f * 100.0).round() / 100.0;
        if what == Action::Opacity {
            // Live on the pill; written to config.toml on release.
            let v = 0.4 + 0.6 * f;
            self.cfg.general.pill_opacity = v;
            self.model.body_alpha = v;
            self.kick();
            return;
        }
        if let Action::AppVolume(exe) = &what {
            self.set_app_volume(exe, f);
            self.kick();
            return;
        }
        // Shown at once; Windows is only told when the level actually changed.
        let muted = self.model.speaker.is_some_and(|(_, m)| m) && f == 0.0;
        if self.model.speaker.map(|(v, _)| v) != Some(f) {
            self.model.speaker = Some((f, muted));
            let _ = crate::mic::set_volume(f);
        }
        self.kick();
    }

    /// Set one app's volume and show it at once (the list isn't re-read while dragging).
    pub(super) fn set_app_volume(&mut self, exe: &str, v: f32) {
        let v = (v.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        crate::mic::set_app_volume(exe, v);
        if let Some(a) = self.model.apps.iter_mut().find(|a| a.exe == exe) {
            a.vol = v;
            a.muted &= v == 0.0;
        }
    }

    pub(super) fn toggle_mic(&mut self) {
        if let Some(m) = crate::mic::toggle() {
            self.model.mic_muted = m;
        }
    }

    /// Vertical wheel: the volume, the shelf row or the timer minutes under the pointer;
    /// anywhere else a notched mouse wheel pages the panes, one per notch (down = next).
    /// A precision touchpad's vertical scroll (deltas that aren't whole notches) doesn't
    /// page: it swipes sideways for that (`hwheel`).
    pub(super) fn wheel(&mut self, delta: i32) {
        // ponytail: notch size tells mouse from touchpad; free-spin mice count as touchpads
        let notched = delta != 0 && delta % 120 == 0;
        // Lyrics move a line per notch, but a touchpad needs a long stretch per line (it sends
        // a stream of small deltas, and its coasting would otherwise fling the page).
        if self.hover
            && self.model.panel() == Panel::Media
            && self.model.lyrics_open
            && self.model.lyric_rows().is_some()
        {
            if self.lyric_wheel.signum() == -delta.signum() {
                self.lyric_wheel = 0; // the other way: start over
            }
            self.lyric_wheel += delta;
            let need = if notched { 120 } else { 600 };
            if self.lyric_wheel.abs() >= need {
                let dir = -self.lyric_wheel.signum();
                self.lyric_wheel = 0;
                self.model.scroll_lyrics(dir);
            }
            return;
        }
        self.wheel += delta;
        let steps = self.wheel / 120;
        if steps == 0 {
            return;
        }
        self.wheel -= steps * 120;
        // Over the volume bar or the sound tile, the wheel is the volume (2% a notch, as
        // in the Windows flyout).
        // Over the shelf row, the wheel scrolls it (down = further along).
        let over_shelf = self
            .hit_at()
            .is_some_and(|h| matches!(h.action, Action::ShelfTile(_) | Action::ShelfScroll(_)));
        if over_shelf {
            self.model.scroll_shelf(-steps);
            return;
        }
        let over_app = self.hit_at().and_then(|h| match &h.action {
            Action::AppVolume(exe) => Some(exe.clone()),
            _ => None,
        });
        if let Some(exe) = over_app {
            if let Some(a) = self.model.apps.iter().find(|a| a.exe == exe) {
                let v = a.vol + steps as f32 * 0.02;
                self.set_app_volume(&exe, v);
            }
            return;
        }
        let over_volume = self
            .hit_at()
            .is_some_and(|h| matches!(h.action, Action::Volume | Action::ToggleSpeaker));
        if over_volume {
            if let Some((v, _)) = self.model.speaker {
                if let Some(s) = crate::mic::set_volume(v + steps as f32 * 0.02) {
                    self.model.speaker = Some(s);
                }
            }
            return;
        }
        if self.hover
            && self.model.panel() == Panel::Tasks
            && self.model.scroll_tasks(-steps.signum())
        {
            return;
        }
        if self.hover
            && self.model.panel() == Panel::Media
            && self.model.lyrics_open
            && self.model.scroll_lyrics(-steps.signum())
        {
            return;
        }
        if self.hover
            && matches!(self.model.panel(), Panel::Notes | Panel::Todo)
            && self.model.scroll_jot(-steps.signum())
        {
            return;
        }
        if self.model.timer_panel && self.model.timer.is_none() {
            self.model.timer_min = crate::timer::nudge(self.model.timer_min, steps);
            return;
        }
        if notched {
            self.page(-steps.signum());
        }
    }

    /// Sideways scroll pages the panes: right = next, one pane per swipe (`Swipe`).
    pub(super) fn hwheel(&mut self, delta: i32) {
        if let Some(dir) = self.swipe.feed(delta, Instant::now()) {
            self.page(dir);
        }
    }

    /// One pane along: 1 = next, -1 = previous.
    pub(super) fn page(&mut self, dir: i32) {
        let now = Instant::now();
        // Collapsed pills peek open so the change is visible.
        let p = self.model.step_panel(dir);
        self.select_panel(p);
        // `step_panel` already moved the selection, so give the slide its direction here.
        self.anim.nav = dir as f32;
        if !self.hover {
            self.model.peek(p, now + Duration::from_secs(3));
        }
    }

    /// The alarm rings as long as the finished card is up (dismissed, or its hold ran out),
    /// but not over a game: the card is hidden there, so nothing could stop it.
    /// A Focus session turns Windows Do Not Disturb on and puts it back afterwards (a crashed
    /// session is undone at the next start, see `dnd::recover`). If Windows refuses, Focus still
    /// holds Hytte's own alerts; the failure is logged once.
    pub(super) fn focus_dnd(&mut self) {
        let want = self.model.holding() && self.cfg.timer.focus_dnd;
        if want && self.dnd.is_none() && !self.dnd_tried {
            self.dnd_tried = true;
            self.dnd = crate::dnd::enter();
            if self.dnd.is_none() {
                crate::logging::line("focus: couldn't turn Do Not Disturb on");
            }
        } else if !want {
            self.dnd_release();
            self.dnd_tried = false;
        }
    }

    /// Put Do Not Disturb back as it was.
    pub(super) fn dnd_release(&mut self) {
        if let Some(prev) = self.dnd.take() {
            crate::dnd::leave(prev);
        }
    }

    pub(super) fn sync_ring(&mut self) {
        let ring = self.cfg.timer.sound
            && self.model.timer_done.is_some()
            && !self.gaming
            && !self.quiet();
        if ring != self.ringing {
            self.ringing = ring;
            crate::timer::ring(ring);
        }
    }

    /// `[general] remember_panel`: write the panel the pill is on, to open on it next launch.
    pub(super) fn remember_panel(&self) {
        let g = &self.cfg.general;
        if g.remember_panel && g.start_panel == "last" {
            match self.model.selected {
                Some(p) => {
                    let _ = std::fs::write(panel_file(), p.saved_name());
                }
                None => {
                    let _ = std::fs::remove_file(panel_file());
                }
            }
        }
    }

    pub(super) fn evaluate_fullscreen(&mut self, hwnd: HWND) {
        use crate::fullscreen::Suppress;
        // Typing (or keyboard mode) puts Hytte itself in front. That says nothing about the
        // app underneath, and reading it as "no fullscreen app" would clear `hover` and
        // collapse the pill under the user's click. Decide again once focus is handed back.
        if self.kb.is_some() || self.model.typing.is_some() {
            return;
        }
        // None: an overlay (Game Bar, GPU / OEM tools) is in front; keep the current state.
        let Some(s) = crate::fullscreen::evaluate(&self.cfg.general, self.hmon) else {
            return;
        };
        let gaming = s == Suppress::Game;
        let hide = s != Suppress::Show;
        let changed = gaming != self.gaming;
        let entering = changed && gaming;
        if changed {
            self.set_gaming(hwnd, gaming);
        }
        // Game -> maximised keeps fs_hidden true but must still bring the window back.
        if hide != self.fs_hidden || changed {
            self.fs_hidden = hide;
            if !hide {
                self.hover = false;
            } else {
                self.model.pinned = false;
            }
            self.layout();
            if entering {
                // Gone at once: no fade frames drawn over the game.
                self.anim.vis.snap();
            }
            self.kick();
        }
    }

    /// Step completely aside for a game, or come back. Nothing of Hytte should cost the
    /// game a frame or a millisecond of input latency: no window over it (an overlay
    /// forces DWM composition), no system-wide mouse / location hooks, no global hotkey,
    /// background CPU / IO / memory priority, and the pollers pause (`GAMING`).
    pub(super) fn set_gaming(&mut self, hwnd: HWND, on: bool) {
        use windows::Win32::System::Threading::{
            SetPriorityClass, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
            PROCESS_MODE_BACKGROUND_BEGIN, PROCESS_MODE_BACKGROUND_END,
        };
        self.gaming = on;
        if on {
            // Pending agent asks go back to their terminals rather than wait unseen.
            self.asks.clear();
        } else {
            self.show_update(Instant::now());
        }
        crate::fullscreen::GAMING.store(on, Ordering::Relaxed);
        input_hooks(!on);
        self.register_hotkey(hwnd);
        self.armed = false;
        ARMED.store(false, Ordering::Relaxed);
        // Background mode lowers I/O and memory priority; the idle class lowers CPU priority
        // (background mode alone left the thread priorities unchanged in testing).
        let (mode, class) = if on {
            (PROCESS_MODE_BACKGROUND_BEGIN, IDLE_PRIORITY_CLASS)
        } else {
            (PROCESS_MODE_BACKGROUND_END, NORMAL_PRIORITY_CLASS)
        };
        for m in [mode, class] {
            if let Err(e) = unsafe { SetPriorityClass(GetCurrentProcess(), m) } {
                crate::logging::line(&format!("game mode: priority {:#x}: {e}", m.0));
            }
        }
        if !on {
            crate::ports::request_refresh();
        }
        self.sync_ring();
        crate::logging::line(if on {
            "game mode on: hidden, input hooks off, background priority"
        } else {
            "game mode off"
        });
    }
}
