//! Draining `UiEvent`s into the model, and applying a changed config.

use super::forecast::WIDE_OFFER_MS;
use super::*;

impl Ui {
    pub(super) fn handle_events(&mut self, hwnd: HWND) {
        let now = Instant::now();
        let events: Vec<UiEvent> = std::mem::take(&mut *self.shared.pending.lock().unwrap());
        for e in events {
            match e {
                UiEvent::Task(mut u) => {
                    // The pane a row lives in is known only once it is in the model.
                    let mut peek_for_input = None;
                    let agent_row =
                        matches!(&u, crate::tasks::TaskUpdate::Upsert(s) if s.source == "agent");
                    if let crate::tasks::TaskUpdate::Upsert(s) = &mut u {
                        // Downloads switched off in settings: its watcher may still run.
                        if s.source == "download" && !self.cfg.downloads.enabled {
                            continue;
                        }
                        if s.source == "shell"
                            && s.delay_ms == 0
                            && s.event == hytte_proto::TaskEvent::Start
                        {
                            s.delay_ms = self.cfg.shell.threshold_ms;
                        }
                        if s.event == hytte_proto::TaskEvent::NeedsInput {
                            let until = now + Duration::from_secs(self.cfg.agent.peek_secs);
                            peek_for_input = Some(until);
                            // An agent waiting on you shows even over a maximised terminal.
                            self.model.force_until = Some(until);
                            if self.cfg.agent.sound && !self.quiet() {
                                unsafe {
                                    let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
                                        MB_ICONASTERISK,
                                    );
                                }
                            }
                        }
                    }
                    // First sight of a task with an owner: remember which terminal tab it ran in.
                    if let crate::tasks::TaskUpdate::Upsert(s) = &u {
                        if let (Some(pid), false) =
                            (s.pid, self.model.tasks.iter().any(|t| t.id == s.task_id))
                        {
                            let _ = self
                                .tabs
                                .send(crate::tabs::Cmd::Snap(s.task_id.clone(), pid));
                        }
                    }
                    let finished = match &u {
                        crate::tasks::TaskUpdate::Upsert(s)
                            if matches!(
                                s.event,
                                hytte_proto::TaskEvent::Done | hytte_proto::TaskEvent::Failed
                            ) =>
                        {
                            Some((s.task_id.clone(), s.event == hytte_proto::TaskEvent::Failed))
                        }
                        _ => None,
                    };
                    self.model.apply_task(u, now);
                    let pane = if agent_row {
                        self.model.agent_panel()
                    } else {
                        Panel::Tasks
                    };
                    if let Some(until) = peek_for_input {
                        self.model.peek(pane, until);
                    }
                    // Open the pill on the result, unless the task was hidden (a short
                    // shell command, or Ctrl-C) and so never made it into the list.
                    let secs = self.cfg.general.finish_peek_secs;
                    let shown = finished
                        .as_ref()
                        .is_some_and(|(id, _)| self.model.tasks.iter().any(|t| &t.id == id));
                    if let (true, Some((_, failed))) = (shown, &finished) {
                        if self.model.holding() {
                            // A Focus session: counted, told when it ends.
                            if *failed {
                                self.model.held.failed += 1;
                            } else {
                                self.model.held.done += 1;
                            }
                        } else if secs > 0 {
                            self.model.peek(pane, now + Duration::from_secs(secs));
                        }
                    }
                }
                UiEvent::Ports(p, parents) => self.model.set_ports(p, parents),
                UiEvent::TaskGone(id) => self.model.remove_task(&id),
                UiEvent::Calendar(ev) => self.model.set_calendar(
                    ev,
                    crate::timer::now_ms() as i64,
                    self.cfg.calendar.lead_min as i64 * 60_000,
                    now,
                ),
                UiEvent::DownloadDone { .. } if self.model.holding() => {
                    if self.cfg.downloads.enabled {
                        self.model.held.downloads += 1;
                    }
                }
                UiEvent::DownloadDone { name, path } => {
                    if self.cfg.downloads.enabled {
                        let mut c = Chip::new(format!("{name} finished downloading"), now);
                        c.prio = crate::ui_state::Prio::Alert;
                        c.key = Some(format!("download:{}", path.display()));
                        c.open = Some(path.clone());
                        c.extra = vec![("Shelve".into(), ChipAction::Shelve(path))];
                        self.model.push_chip(c, now);
                    }
                }
                UiEvent::Hog(_) if self.model.holding() => self.model.held.warnings += 1,
                UiEvent::Hog(a) => {
                    // Never over another card (it waits its turn), and never for a process the
                    // user muted.
                    let muted = self.model.hog_ignore.contains(&a.exe.to_ascii_lowercase());
                    if !muted {
                        let mut c = Chip::new(a.summary(), now);
                        c.prio = crate::ui_state::Prio::Alert;
                        c.key = Some(format!("hog:{}", a.exe.to_ascii_lowercase()));
                        c.tone = crate::ui_state::Tone::Warn;
                        c.hold = Duration::from_secs(20);
                        c.extra = vec![
                            ("Kill".into(), ChipAction::KillPid(a.pid, a.exe.clone())),
                            ("Ignore".into(), ChipAction::IgnoreExe(a.exe)),
                        ];
                        self.model.push_chip(c, now);
                    }
                }
                UiEvent::ShelfAdd(job) => {
                    // Copying (shelf.mode = "copy") and writing text snippets can take
                    // a while: do the file work on a worker so the pill keeps animating.
                    let cfg = self.cfg.shelf.clone();
                    let existing = crate::shelf::paths(&self.model.shelf);
                    let room = crate::shelf::room(&cfg, &self.model.shelf);
                    let sh = self.shared.clone();
                    std::thread::spawn(move || {
                        let staged = crate::shelf::stage(
                            &cfg,
                            &job.paths,
                            job.text.as_deref(),
                            &existing,
                            room,
                        );
                        if !staged.is_empty() {
                            push_event(&sh, UiEvent::ShelfStaged(staged));
                        }
                    });
                }
                UiEvent::ShelfStaged(staged) => {
                    if crate::shelf::take_too_big() {
                        let mb = crate::shelf::COPY_LIMIT / (1024 * 1024);
                        let mut c =
                            Chip::new(format!("Too big to copy to the shelf (limit {mb} MB)"), now);
                        c.tone = crate::ui_state::Tone::Warn;
                        self.model.interrupt(c, now);
                    }
                    self.add_staged(staged, now);
                }
                UiEvent::Screenshot(it) => {
                    let path = it.path.clone();
                    if self.add_staged(vec![it], now) > 0 {
                        self.track_shot(hwnd, path, now);
                    }
                }
                UiEvent::Thumb(id, bmp) => {
                    if let Some(b) = bmp {
                        self.rend.set_thumb(id, &b);
                    }
                }
                // Mid-seek, a late position update would yank the knob back.
                UiEvent::Media(m) if !self.scrubbing(&Action::Seek) => {
                    self.model.apply_media(m, now)
                }
                UiEvent::Media(_) => {}
                UiEvent::MediaArt(a) => self.model.set_art(a),
                UiEvent::MicMute(m) => self.model.mic_muted = m,
                // Mid-drag, a late notification for an earlier level would pull the bar back.
                UiEvent::Speaker(s) if !self.scrubbing(&Action::Volume) => self.model.speaker = s,
                UiEvent::Speaker(_) => {}
                UiEvent::Power(p) => self.model.set_power(p, now),
                UiEvent::Privacy(c, m, app) => {
                    // A brief card when something starts (not when it stops), unless a Focus
                    // session or quiet hours want silence.
                    let text = crate::ui_state::privacy_start(
                        (self.model.cam, self.model.mic),
                        (c, m),
                        app.as_deref(),
                    );
                    self.model.cam = c;
                    self.model.mic = m;
                    self.model.privacy_app = app;
                    if let Some(text) = text.filter(|_| {
                        self.cfg.moments.privacy && !self.model.holding() && !self.quiet()
                    }) {
                        let mut chip = Chip::new(text, now);
                        chip.tone = crate::ui_state::Tone::Info;
                        chip.hold = Duration::from_secs(3);
                        chip.prio = crate::ui_state::Prio::Alert;
                        chip.key = Some("privacy".into());
                        self.model.push_chip(chip, now);
                    }
                }
                UiEvent::ShelfCopied(paths) => {
                    // By reference whatever `[shelf] mode` says: a copy of everything you
                    // press Ctrl+C on would fill the disk.
                    let mut cfg = self.cfg.shelf.clone();
                    cfg.mode = "reference".into();
                    let existing = crate::shelf::paths(&self.model.shelf);
                    let room = crate::shelf::room(&cfg, &self.model.shelf);
                    let sh = self.shared.clone();
                    std::thread::spawn(move || {
                        let staged = crate::shelf::stage(&cfg, &paths, None, &existing, room);
                        if !staged.is_empty() {
                            push_event(&sh, UiEvent::ShelfStaged(staged));
                        }
                    });
                }
                UiEvent::Bluetooth { name, battery } => {
                    if !self.model.holding() && !self.quiet() {
                        let (text, note) = crate::bluetooth::card_text(&name, battery);
                        let mut chip = Chip::new(text, now);
                        chip.note = note;
                        chip.tone = crate::ui_state::Tone::Info;
                        chip.hold = Duration::from_secs(5);
                        chip.prio = crate::ui_state::Prio::Alert;
                        chip.key = Some(format!("bluetooth:{name}"));
                        self.model.push_chip(chip, now);
                    }
                }
                UiEvent::DropDone(r) => {
                    self.anim.squash();
                    self.model.interrupt(
                        Chip {
                            open: r.open,
                            copy: r.copy,
                            ..Chip::new(r.summary, now)
                        },
                        now,
                    );
                }
                UiEvent::SetClipboard(s) => set_clipboard(hwnd, &s),
                UiEvent::DragEnter => self.model.drop_over = true,
                UiEvent::DragLeave => self.model.drop_over = false,
                UiEvent::Config(cfg) => self.apply_config(*cfg),
                UiEvent::ConfigProblem(msg) => {
                    let mut c = Chip::new(msg, now);
                    c.tone = crate::ui_state::Tone::Warn;
                    c.hold = Duration::from_secs(12);
                    c.open = Some(crate::config::config_path());
                    self.model.interrupt(c, now);
                }
                UiEvent::Cmd(c, reply) => self.run_cmd(c, reply, now),
                UiEvent::UpdateOffer { tag, note } => {
                    self.update_offer = Some((tag, note));
                    self.show_update(now);
                }
                UiEvent::Forecast(r) => self.model.forecast_result(r, now),
                UiEvent::AiText(seq, card, t) => self.model.ai_text(seq, card, &t),
                UiEvent::AiDone(seq, card, r) => self.model.ai_done(seq, card, r, now),
                UiEvent::Ollama(s) => self.model.ollama_status(s),
                UiEvent::Pull(p) => self.model.pull_progress(&p),
                UiEvent::PullDone(r) => self.model.pull_done(r),
                UiEvent::Warm(r) => self.model.warm_done(r),
                UiEvent::Script(name, run) => self.model.script_done(&name, run, now),
                UiEvent::Packs(packs) => {
                    self.packs = packs;
                    self.refresh_panes();
                    self.ask_packs(now);
                }
                UiEvent::ActionDone(name, run) => {
                    self.model.action_finished(&name);
                    let timeout = self
                        .named_action(&name)
                        .map_or(crate::config::ACTION_TIMEOUT, |a| a.timeout);
                    let c = crate::ui_state::action_card(&name, &run, timeout, now);
                    self.model.interrupt(c, now);
                }
                UiEvent::Lyrics(key, r) => self.model.lyrics_result(key, r),
                UiEvent::Places(seq, q, r) if seq == self.place_seq => {
                    self.model.places_result(q, r);
                    // Nothing found: after two quiet seconds, offer the wider search.
                    if self.model.places.is_empty() && !self.model.places_err {
                        unsafe {
                            SetTimer(Some(hwnd), T_WIDE, WIDE_OFFER_MS, None);
                        }
                    }
                }
                UiEvent::Places(..) => {}
                UiEvent::WidePlaces(seq, q, r) if seq == self.place_seq => {
                    self.model.wide_result(q, r)
                }
                UiEvent::WidePlaces(..) => {}
                UiEvent::Notice(text, tone) => {
                    let mut c = Chip::new(text, now);
                    c.tone = tone;
                    self.model.interrupt(c, now);
                }
                // Nowhere to show it: dropping `reply` sends the agent to its terminal prompt now.
                UiEvent::Ask { .. } if self.paused || self.gaming => {}
                UiEvent::Ask { id, summary, reply } => {
                    // Urgent: it takes the chip over; a second agent's ask queues behind.
                    self.asks.push_back((id, summary, now, reply));
                    self.show_ask(now);
                    if self.cfg.agent.sound && !self.quiet() {
                        unsafe {
                            let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(
                                MB_ICONASTERISK,
                            );
                        }
                    }
                }
            }
        }
        self.after_model_change(hwnd);
    }

    /// A live edit of `config.toml`. Most settings are read where they're used, so only
    /// state derived from the config needs refreshing. Lowering `[shelf] max_items` keeps
    /// the tiles already there and only limits new ones.
    pub(super) fn apply_config(&mut self, new: Config) {
        let hotkey_changed = new.general.hotkey != self.cfg.general.hotkey
            || new.general.monitor_hotkey != self.cfg.general.monitor_hotkey
            || new.general.note_hotkey != self.cfg.general.note_hotkey
            || new.ai.hotkey != self.cfg.ai.hotkey;
        if new.general.hide_from_capture != self.cfg.general.hide_from_capture {
            set_capture_hidden(
                HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _),
                new.general.hide_from_capture,
            );
        }
        if new.general.autostart != self.cfg.general.autostart {
            crate::config::ensure_autostart(new.general.autostart);
        }
        if new.ports != self.cfg.ports {
            crate::ports::reconfigure(new.ports.clone());
        }
        if new.hog != self.cfg.hog {
            crate::ports::reconfigure_hog(new.hog.clone());
        }
        if new.downloads.enabled {
            crate::downloads::start(new.downloads.clone());
        } else {
            self.model.tasks.retain(|t| t.source != "download");
        }
        if new.calendar.enabled {
            crate::calendar::start();
        }
        crate::update::ENABLED.store(new.general.check_updates, Ordering::Relaxed);
        self.model.ignore = ignore_list(&new);
        self.light = theme_is_light(&new.general.theme);
        self.model.adaptive_glow = new.general.adaptive_glow;
        self.model.finish_peek = Duration::from_secs(new.general.finish_peek_secs);
        self.model.glow_gain = new.general.glow_strength.clamp(0.0, 3.0);
        self.model.body_alpha = new.general.pill_opacity.clamp(0.4, 1.0);
        self.model.task_rows = new.general.task_rows;
        self.model.agent_quiet = Duration::from_secs(new.agent.quiet_after_secs);
        self.model.pin_button = new.general.pin_button;
        if !new.general.pin_button {
            self.model.pinned = false;
        }
        self.model.rounds = new.timer.rounds;
        self.model.apply_ai(&new.ai);
        self.model.apply_panes(&new.panes);
        self.model.skin = crate::skin::Skin::from_name(&new.general.skin);
        self.model.lyrics_on = new.media.lyrics;
        self.model.lyrics_still = !new.media.lyrics_live;
        self.breaks.configure(break_minutes(&new), Instant::now());
        self.model.imperial = crate::weather::imperial(&new.weather.units);
        let monitor_changed = new.general.monitor != self.cfg.general.monitor
            || new.general.pill_scale != self.cfg.general.pill_scale;
        let general = new.general != self.cfg.general;
        crate::settings_win::refresh(&new);
        crate::bluetooth::configure(new.moments.bluetooth, None);
        self.model
            .apply_clipboard(new.clipboard.history, new.clipboard.expire_secs);
        self.cfg = new;
        self.refresh_panes();
        if hotkey_changed {
            self.register_hotkey(HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _));
        }
        if monitor_changed {
            self.refresh_monitor();
        }
        if general {
            self.evaluate_fullscreen(HWND(HWND_ADDR.load(Ordering::Relaxed) as *mut _));
        }
    }
}
