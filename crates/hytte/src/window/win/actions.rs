//! What a click does: actions, chip buttons, script commands, update and ask cards.

use super::*;

impl Ui {
    pub(super) fn run_action(&mut self, hwnd: HWND, a: Action) {
        match a {
            Action::DismissTask(id) => self.model.dismiss_task(&id),
            Action::DismissChip => {
                let was_ask = self.model.chip.as_ref().is_some_and(|c| {
                    c.extra
                        .iter()
                        .any(|(_, a)| matches!(a, ChipAction::Answer(..)))
                });
                self.model.dismiss_chip();
                if was_ask {
                    self.asks.pop_front();
                    self.show_ask(Instant::now());
                }
            }
            Action::Chip(a) => self.chip_action(a),
            Action::CopyText(s) => set_clipboard(hwnd, &s),
            Action::OpenPath(p) => {
                let _ = std::process::Command::new("explorer").arg(p).spawn();
            }
            Action::MediaPrev => crate::media::control(crate::media::Cmd::Prev),
            Action::MediaToggle => crate::media::control(crate::media::Cmd::Toggle),
            Action::MediaNext => crate::media::control(crate::media::Cmd::Next),
            Action::FocusTerminal(pid, id) => {
                if let Some(h) = crate::proc::terminal_window_for(pid) {
                    crate::proc::focus_window(h);
                    let _ = self.tabs.send(crate::tabs::Cmd::Select(id));
                }
            }
            Action::SelectPanel(p) => self.select_panel(p),
            Action::Field(f) => self.click_field(hwnd, f),
            Action::NoteKeep(k) => self.model.note_keep = k,
            Action::ToggleJot(id) => {
                self.model.jot.toggle(id, crate::timer::now_ms());
                self.model.jot_dirty = true;
            }
            Action::EditJot(id) => {
                self.model.begin_edit(id);
                if let Some(f) = self.model.typing {
                    self.begin_typing(hwnd, f);
                }
            }
            Action::KeepJot(id) => {
                self.model.jot.keep(id);
                self.model.jot_dirty = true;
            }
            Action::DeleteJot(id) => self.model.delete_jot(id, Instant::now()),
            Action::UndoJot => self.model.undo_jot(),
            Action::PickPlace(i) => self.pick_place(hwnd, i),
            Action::ChangePlace => self.change_place(hwnd),
            Action::SearchWide => self.search_wide(),
            Action::CancelPlace => {
                self.model.choosing = false;
                self.end_keyboard(true);
            }
            Action::RetryWeather => self.model.weather_retry = None,
            Action::ToggleMic => self.toggle_mic(),
            Action::ToggleSpeaker => {
                if let Some(s) = crate::mic::toggle_speaker() {
                    self.model.speaker = Some(s);
                }
            }
            // Mouse presses on the bar are handled as a drag (`scrub_to`); this is the keyboard.
            Action::Volume => {
                if let Some(s) = crate::mic::toggle_speaker() {
                    self.model.speaker = Some(s);
                }
            }
            Action::ToggleApps => {
                self.model.apps_open = !self.model.apps_open;
                if self.model.apps_open {
                    self.model.apps = crate::mic::apps();
                }
            }
            // Dragged with the mouse (`scrub_to`); from the keyboard: mute / unmute it.
            Action::AppVolume(exe) => {
                if let Some(a) = self.model.apps.iter().find(|a| a.exe == exe) {
                    let v = if a.muted || a.vol == 0.0 { 1.0 } else { 0.0 };
                    self.set_app_volume(&exe, v);
                }
            }
            // Mouse presses seek by dragging (`scrub_to`); from the keyboard: play / pause.
            Action::Seek => crate::media::control(crate::media::Cmd::Toggle),
            Action::CycleAwake => self.model.awake = self.model.awake.next(Instant::now()),
            Action::OpenTimer => {
                if self.model.timer.is_some() {
                    self.select_panel(Panel::Timer);
                } else {
                    self.model.timer_panel = true;
                }
            }
            Action::TimerNudge(d) => {
                self.model.timer_min = crate::timer::nudge(self.model.timer_min, d);
            }
            Action::TimerMinutes(m) => self.model.timer_min = m,
            Action::TimerStart(k) => self.start_timer(k, self.model.timer_min),
            Action::TimerBreak(m) => self.start_timer(crate::timer::TimerKind::Break, m),
            Action::TimerAdd(m) => {
                if let Some(t) = &mut self.model.timer {
                    t.add_minutes(m as u64);
                    crate::timer::save(Some(t));
                } else if self.model.timer_done.is_some() {
                    self.start_timer(crate::timer::TimerKind::Plain, m);
                }
            }
            Action::TimerPause => {
                if let Some(t) = &mut self.model.timer {
                    t.toggle_pause(crate::timer::now_ms());
                    crate::timer::save(Some(t));
                }
            }
            Action::TimerStop => {
                let was_focus = self.model.holding();
                self.model.timer = None;
                crate::timer::save(None);
                // Ended early: what was held back shows now, on an ordinary card.
                if let (true, Some(text)) = (was_focus, self.model.held.take_summary()) {
                    let mut c = Chip::new(text, Instant::now());
                    c.tone = crate::ui_state::Tone::Info;
                    self.model.chip = Some(c);
                }
            }
            Action::NudgeSnooze => {
                if let Some(n) = self.model.nudge.take() {
                    self.breaks.snooze(n.kind, Instant::now());
                }
            }
            Action::NudgeSkip => self.model.nudge = None,
            Action::SleepMenu => self.model.sleep_open = !self.model.sleep_open,
            Action::SleepSet(kind) => {
                crate::sleep::cancel();
                self.model.set_sleep(Some(kind), Instant::now());
            }
            Action::SleepCancel => {
                crate::sleep::cancel();
                self.model.set_sleep(None, Instant::now());
            }
            Action::LyricsOpen => self.model.lyrics_open = true,
            Action::LyricsClose => self.model.lyrics_open = false,
            Action::LyricsRetry => {
                self.model.lyric_key.clear();
                self.model.lyric_state = crate::ui_state::LyricState::Idle;
            }
            Action::TimerDismiss => self.model.timer_done = None,
            Action::TimerClose => self.model.timer_panel = false,
            Action::Settings(g) => self.model.settings = g,
            Action::Pin => {
                self.model.pinned = !self.model.pinned;
                // Unpinned with the pointer already away: let it close as usual.
                if !self.model.pinned && !self.inside {
                    unsafe {
                        SetTimer(
                            Some(hwnd),
                            T_COLLAPSE,
                            self.cfg.general.collapse_delay_ms.clamp(20, 5000),
                            None,
                        );
                    }
                }
            }
            // Dragged: handled by `scrub_to`.
            Action::Opacity
            | Action::Glow
            | Action::ClearSearch
            | Action::PaneMove(..)
            | Action::Jump(_)
            | Action::EditConfig(_) => {}
            Action::Set(key, v) => match crate::config::set(key, v) {
                Ok(Some(cfg)) => self.apply_config(cfg),
                Ok(None) => {
                    let mut c = Chip::new(
                        "config.toml has a mistake in it: fix it first",
                        Instant::now(),
                    );
                    c.tone = crate::ui_state::Tone::Warn;
                    c.open = Some(crate::config::config_path());
                    self.model.chip = Some(c);
                }
                Err(e) => crate::logging::line(&format!("settings: {e}")),
            },
            Action::OpenConfig => {
                self.model.settings = None;
                crate::settings_win::open(&self.cfg);
            }
            Action::Rerun(id) => self.rerun(&id),
            Action::ShelfScroll(by) => self.model.scroll_shelf(by),
            Action::SetPowerMode(m) => {
                crate::power::set_mode(m);
                self.model.power = crate::power::read();
            }
            Action::FocusMedia(app) => crate::proc::focus_app(&app),
            Action::OpenPort(port)
            | Action::KillPort(port, _)
            | Action::RestartPort(port, _, _)
                if !port_alive(&self.model, port) =>
            {
                // The process is gone since the last scan: drop it instead of erroring.
                self.model.ports.retain(|p| p.port != port);
                self.model.kill_armed = None;
                crate::ports::request_refresh();
            }
            Action::OpenPort(port) => unsafe {
                let url: Vec<u16> = format!("http://localhost:{port}\0")
                    .encode_utf16()
                    .collect();
                ShellExecuteW(
                    None,
                    windows::core::w!("open"),
                    windows::core::PCWSTR(url.as_ptr()),
                    None,
                    None,
                    SW_SHOWNORMAL,
                );
            },
            Action::RestartPort(port, pid, task_id) => {
                if self.model.kill_armed.is_some_and(|(p, _)| p == port) {
                    self.model.kill_armed = None;
                    let exe = self
                        .model
                        .ports
                        .iter()
                        .find(|p| p.port == port && p.pid == pid)
                        .map(|p| p.exe.clone())
                        .unwrap_or_default();
                    match hytte_proto::ports::kill(pid, &exe) {
                        // The same command, in its folder, in a new terminal.
                        Ok(()) => self.rerun(&task_id),
                        Err(e) => {
                            let msg = format!("Could not restart :{port} - {e}");
                            self.model.chip = Some(Chip::new(msg, Instant::now()));
                        }
                    }
                    crate::ports::request_refresh();
                } else {
                    self.model.kill_armed =
                        Some((port, Instant::now() + crate::ui_state::KILL_CONFIRM));
                }
            }
            Action::KillPort(port, pid) => {
                if self.model.kill_armed.is_some_and(|(p, _)| p == port) {
                    self.model.kill_armed = None;
                    let exe = self
                        .model
                        .ports
                        .iter()
                        .find(|p| p.port == port && p.pid == pid)
                        .map(|p| p.exe.clone())
                        .unwrap_or_default();
                    let msg = match hytte_proto::ports::kill(pid, &exe) {
                        Ok(()) => format!("Stopped :{port}"),
                        Err(e) => format!("Could not stop :{port} - {e}"),
                    };
                    self.model.chip = Some(Chip::new(msg, Instant::now()));
                    crate::ports::request_refresh();
                } else {
                    self.model.kill_armed =
                        Some((port, Instant::now() + crate::ui_state::KILL_CONFIRM));
                }
            }
            Action::ShelfTile(id) => {
                self.model.shelf_sel = if self.model.shelf_sel == Some(id) {
                    None
                } else {
                    Some(id)
                };
            }
            Action::RemoveShelf(id) => {
                crate::shelf::remove(&mut self.model.shelf, id);
                if self.model.shelf_sel == Some(id) {
                    self.model.shelf_sel = None;
                }
                self.save_shelf();
            }
            Action::ShelfOp(id, op) => {
                if let Some(it) = self.model.shelf.iter().find(|i| i.id == id) {
                    let job = DropJob {
                        paths: vec![it.path.clone()],
                        text: None,
                        op: Some(op),
                    };
                    self.model.chip = Some(Chip::new("Working…", Instant::now()));
                    let _ = self.shared.drop_tx.send(job);
                }
            }
        }
        self.after_model_change(hwnd);
    }

    /// One of a chip's extra buttons.
    pub(super) fn chip_action(&mut self, a: ChipAction) {
        match a {
            ChipAction::OpenUrl(url) => {
                open_url(&url);
                self.model.dismiss_chip();
            }
            ChipAction::Shelve(path) => {
                let job = DropJob {
                    paths: vec![path],
                    text: None,
                    op: None,
                };
                push_event(&self.shared, UiEvent::ShelfAdd(job));
                self.model.dismiss_chip();
            }
            ChipAction::KillPid(pid, exe) => {
                if self.model.chip_armed.take().is_some() {
                    let msg = match hytte_proto::ports::kill(pid, &exe) {
                        Ok(()) => format!("Stopped process {pid}"),
                        Err(e) => format!("Could not stop process {pid} - {e}"),
                    };
                    self.model.chip = Some(Chip::new(msg, Instant::now()));
                } else {
                    self.model.chip_armed = Some(Instant::now() + crate::ui_state::KILL_CONFIRM);
                }
            }
            ChipAction::IgnoreExe(exe) => {
                self.model.hog_ignore.insert(exe.to_ascii_lowercase());
                self.model.dismiss_chip();
            }
            ChipAction::InstallUpdate => {
                self.model.dismiss_chip();
                crate::update::install_pending();
            }
            ChipAction::UpdateLater(tag) => {
                crate::update::skip(&tag);
                self.model.dismiss_chip();
            }
            ChipAction::Answer(id, allow) => {
                // A click aimed at the previous chip must not answer the next ask.
                if !crate::ui_state::ask_click_ready(self.ask_shown, Instant::now()) {
                    return;
                }
                if let Some(i) = self.asks.iter().position(|a| a.0 == id) {
                    if let Some((.., reply)) = self.asks.remove(i) {
                        let _ = reply.send(allow);
                    }
                }
                self.model.dismiss_chip();
                self.show_ask(Instant::now());
            }
        }
    }

    /// A script's request (`hytte notify`, `hytte shelf add`). Paused or in a game nothing
    /// may pop up, and a card never covers another one.
    pub(super) fn run_cmd(&mut self, c: hytte_proto::Cmd, now: Instant) {
        if self.paused || self.gaming {
            return;
        }
        if c.cmd == "notify" {
            if self.model.chip.is_none() {
                self.model.chip = Some(Chip::new(c.text.unwrap_or_default(), now));
            }
            return;
        }
        let job = DropJob {
            paths: c.path.into_iter().map(Into::into).collect(),
            text: c.text,
            op: None,
        };
        push_event(&self.shared, UiEvent::ShelfAdd(job));
    }

    /// Put a waiting update offer on the chip, if nothing else is there and nothing is in
    /// the way (a game, or paused): it stays queued and shows when that ends.
    pub(super) fn show_update(&mut self, now: Instant) {
        if self.paused || self.gaming || self.model.chip.is_some() {
            return;
        }
        let Some((tag, note)) = self.update_offer.take() else {
            return;
        };
        let shown = tag.trim_start_matches('v');
        let mut text = format!("Hytte {shown} is available");
        if !note.is_empty() {
            text = format!("{text}\n{note}");
        }
        let mut c = Chip::new(text, now);
        c.tone = crate::ui_state::Tone::Info;
        c.hold = Duration::from_secs(45);
        c.extra = vec![
            ("Update".into(), ChipAction::InstallUpdate),
            ("Later".into(), ChipAction::UpdateLater(tag)),
        ];
        self.model.chip = Some(c);
    }

    /// Put the oldest live agent ask on the chip, "(1/N)" when more wait behind it. Asks
    /// past `ASK_WAIT_SECS` are dropped: their pipe already gave up and the agent asked in
    /// its terminal.
    pub(super) fn show_ask(&mut self, now: Instant) {
        let wait = Duration::from_secs(hytte_proto::ASK_WAIT_SECS);
        self.asks.retain(|a| now.duration_since(a.2) < wait);
        let Some((id, summary, at, _)) = self.asks.front() else {
            return;
        };
        let summary = match self.asks.len() {
            1 => summary.clone(),
            n => format!("(1/{n}) {summary}"),
        };
        let mut c = Chip::new(summary, *at);
        c.tone = crate::ui_state::Tone::Warn;
        c.hold = wait;
        c.extra = vec![
            ("Allow".into(), ChipAction::Answer(*id, true)),
            ("Deny".into(), ChipAction::Answer(*id, false)),
        ];
        self.model.chip = Some(c);
        self.model.chip_armed = None;
        self.ask_shown = Some(now);
    }
}
