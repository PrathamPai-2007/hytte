//! Keyboard mode, hover, Rerun, hotkeys.

use super::*;

impl Ui {
    /// Open the pill for keyboard use (or close it again): take focus, hold it open.
    pub(super) fn toggle_keyboard(&mut self, hwnd: HWND) {
        if self.kb.is_some() {
            self.end_keyboard(true);
            return;
        }
        unsafe {
            let prev = GetForegroundWindow();
            self.kb = Some(prev);
            self.kb_focus = None;
            self.hover = true;
            let _ = SetForegroundWindow(hwnd);
        }
        crate::ports::request_refresh();
        self.after_model_change(hwnd);
    }

    /// The pointer has rested on the pill long enough: open it.
    pub(super) fn open_hover(&mut self) {
        // Resting on the bubble opens its own pane, not the main pill's. Either way the two
        // spots are remembered while the pane is up (see `BubbleZone`).
        self.zone = None;
        let shown = self.model.bubble(self.model.scene(false, self.sentinel()));
        let pref = self.bubble_pref.take();
        if let Some(kind) = shown.filter(|k| self.anim.bubble_kind == Some(*k)) {
            let pw = self.anim.rect.w.pos as f32;
            let mut zone = BubbleZone {
                kind,
                dx: kind.x(pw, self.anim.bubble.pos as f32) - pw / 2.0,
                w: kind.width(),
                pill_w: pw,
                base: self.model.selected,
                at: Some(Spot::Pill),
            };
            if pref == Some(kind) && self.model.panels().contains(&kind.panel()) {
                zone.at = Some(Spot::Bubble);
                self.select_panel(kind.panel());
            }
            self.zone = Some(zone);
        }
        self.hover = true;
        // Opening the pill: make sure the port list and charge rate are not stale.
        crate::ports::request_refresh();
        if self.model.panel() == Panel::Power && self.model.power.is_some() {
            self.model.power = crate::power::read().or(self.model.power);
        }
        self.layout();
        self.kick();
    }

    /// Leave keyboard mode; `restore` hands focus back to the window that had it.
    pub(super) fn end_keyboard(&mut self, restore: bool) {
        let Some(prev) = self.kb.take() else { return };
        self.kb_focus = None;
        self.hover = self.inside;
        if restore && !prev.0.is_null() {
            unsafe {
                let _ = SetForegroundWindow(prev);
            }
        }
        self.layout();
        self.kick();
    }

    /// A key pressed while the pill has focus.
    pub(super) fn key(&mut self, hwnd: HWND, vk: u16) {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        if self.kb.is_none() {
            return;
        }
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        match VIRTUAL_KEY(vk) {
            VK_ESCAPE => self.end_keyboard(true),
            VK_LEFT | VK_RIGHT => {
                let dir = if VIRTUAL_KEY(vk) == VK_RIGHT { 1 } else { -1 };
                let p = self.model.step_panel(dir);
                self.select_panel(p);
                self.anim.nav = dir as f32;
                self.kb_focus = None;
                self.after_model_change(hwnd);
            }
            // On the timer setup card, digits type the minutes and Backspace erases one.
            VIRTUAL_KEY(k @ (0x30..=0x39 | 0x60..=0x69)) if self.setting_timer() => {
                let now = Instant::now();
                let fresh = self
                    .model
                    .timer_typed
                    .is_none_or(|t| now.saturating_duration_since(t) > TYPE_GAP);
                let d = (k % 0x30) as u32;
                self.model.timer_min = crate::timer::typed(self.model.timer_min, d, fresh);
                self.model.timer_typed = Some(now);
                self.after_model_change(hwnd);
            }
            VK_BACK if self.setting_timer() => {
                self.model.timer_min = (self.model.timer_min / 10).max(1);
                self.model.timer_typed = Some(Instant::now());
                self.after_model_change(hwnd);
            }
            // 1-9: jump straight to that pane (left to right, as the tab strip shows them).
            VIRTUAL_KEY(k @ 0x31..=0x39) => {
                let ps = self.model.panels();
                let Some(&p) = ps.get((k - 0x31) as usize) else {
                    return;
                };
                self.select_panel(p);
                self.kb_focus = None;
                self.after_model_change(hwnd);
            }
            VK_TAB => {
                let n = self.hits.len();
                if n > 0 {
                    self.kb_focus = Some(match (self.kb_focus, shift) {
                        (None, false) => 0,
                        (None, true) => n - 1,
                        (Some(i), false) => (i + 1) % n,
                        (Some(i), true) => (i + n - 1) % n,
                    });
                }
                self.kick();
            }
            VK_RETURN | VK_SPACE => {
                if let Some(a) = self
                    .kb_focus
                    .and_then(|i| self.hits.get(i))
                    .map(|h| h.action.clone())
                {
                    self.run_action(hwnd, a);
                }
            }
            _ => {}
        }
    }

    /// Run a failed command again in a new terminal, and clear its row: the new run
    /// reports itself (a `hytte run` one, or a shell command once it passes the threshold).
    pub(super) fn rerun(&mut self, id: &str) {
        let Some(t) = self
            .model
            .tasks
            .iter()
            .find(|t| t.id == id && t.rerunnable())
        else {
            return;
        };
        let shell = t.pid.and_then(hytte_proto::ports::exe_path);
        let hytte = std::env::current_exe().unwrap_or_default();
        let plan = crate::rerun::plan(
            &t.source,
            &t.label,
            shell.as_deref(),
            &hytte.to_string_lossy(),
        );
        let cwd = t.cwd.clone().unwrap_or_default();
        match crate::rerun::start(&plan, &cwd) {
            Ok(()) => self.model.remove_task(id),
            Err(e) => {
                crate::logging::line(&format!("rerun failed: {e}"));
                let mut c = Chip::new(format!("Couldn't rerun it: {e}"), Instant::now());
                c.tone = crate::ui_state::Tone::Warn;
                self.model.chip = Some(c);
            }
        }
    }

    /// The timer setup card is open (no timer running): digits type its minutes.
    pub(super) fn setting_timer(&self) -> bool {
        self.model.timer_panel && self.model.timer.is_none()
    }

    /// (Re)register the global hotkey from the config.
    pub(super) fn register_hotkey(&self, hwnd: HWND) {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
        };
        for (id, spec) in [
            (HOTKEY_ID, &self.cfg.general.hotkey),
            (MONITOR_HOTKEY_ID, &self.cfg.general.monitor_hotkey),
        ] {
            unsafe {
                let _ = UnregisterHotKey(Some(hwnd), id);
                // A game may want the same chord: give it back while one is in front.
                if spec.is_empty() || self.gaming {
                    continue;
                }
                match crate::config::parse_hotkey(spec) {
                    Some((mods, vk)) => {
                        if RegisterHotKey(
                            Some(hwnd),
                            id,
                            HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT,
                            vk,
                        )
                        .is_err()
                        {
                            crate::logging::line(&format!(
                                "hotkey {spec}: already taken by another app"
                            ));
                        }
                    }
                    None => crate::logging::line(&format!("hotkey {spec}: not understood")),
                }
            }
        }
    }
}
