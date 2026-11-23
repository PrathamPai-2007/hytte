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
            Action::CopyClip(i) => {
                if let Some(t) = self.model.clips.get(i).map(str::to_owned) {
                    // Hytte's own write: on_clipboard skips it, so it is not recorded again.
                    set_clipboard(hwnd, &t);
                    self.anim.squash();
                }
            }
            Action::ClipScroll(by) => self.model.scroll_clips(by),
            Action::DeleteClip(i) => self.model.remove_clip(i),
            Action::ClearClips => self.model.clear_clips(),
            Action::AskSend => self.ask_send(),
            Action::AskStop => self.ask_stop(),
            Action::AskClear => self.ask_clear(),
            Action::AskRetry => self.model.ollama_status(crate::ollama::Status::Unknown),
            Action::AskStart => self.ask_start(),
            Action::AskPull => self.ask_pull(),
            Action::AskWarm => self.ask_warm(),
            Action::AskUnload => self.ask_unload(),
            Action::AskRunAnyway => self.ask_run_anyway(),
            Action::AskCancelTight => self.ask_cancel_tight(),
            Action::AskPullStop => self.ask_pull_stop(),
            Action::Ai(kind, src) => self.ai_action(kind, src),
            Action::AiDismiss => self.ai_dismiss(),
            Action::ClipAi(i) => self.model.toggle_clip_ai(i),
            Action::Named(name, arg) => {
                self.run_named(&name, arg.as_deref());
            }
            Action::ScriptRefresh(slot) => {
                if let Some(s) = self.model.scripts.get_mut(slot as usize) {
                    if !s.busy {
                        s.ran = None;
                    }
                }
                self.script_refresh(hwnd);
            }
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
                    self.model.interrupt(c, Instant::now());
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
            | Action::Hue
            | Action::Vivid
            | Action::SkinColor(_)
            | Action::Hotkey(_)
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
                    self.model.interrupt(c, Instant::now());
                }
                Err(e) => crate::logging::line(&format!("settings: {e}")),
            },
            Action::OpenConfig => crate::settings_win::open(&self.cfg),
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
            Action::ShelfOp(id, crate::transforms::Conv::Ai(kind)) => {
                self.ai_action(kind, crate::ui_state::AiSource::File(id));
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
            ChipAction::Welcome(step) => {
                self.model.dismiss_chip();
                match self.welcome.as_mut() {
                    Some(w) if step < crate::welcome::STEPS => {
                        w.1 = step;
                        self.show_welcome(Instant::now());
                    }
                    _ => {
                        self.welcome = None;
                        crate::welcome::finish();
                    }
                }
            }
            ChipAction::WelcomeSkip => {
                self.model.dismiss_chip();
                self.welcome = None;
                crate::welcome::finish();
            }
            ChipAction::PackAnswer(folder, hash, run) => {
                // The file may have changed since the card went up: an answer is for what was shown.
                if let Some(p) = self
                    .packs
                    .iter()
                    .find(|p| p.folder == folder && p.hash == hash)
                {
                    crate::packs::answer(&p.folder, &hash, run);
                    self.packs = crate::packs::scan();
                    self.refresh_panes();
                }
                self.model.dismiss_chip();
            }
            ChipAction::Action(name) => {
                if self.run_named(&name, None) {
                    self.model.dismiss_chip();
                }
            }
            ChipAction::Card(id, label) => {
                if let Some(i) = self.card_waits.iter().position(|w| w.0 == id) {
                    let _ = self.card_waits.remove(i).1.send(label);
                }
                self.model.dismiss_chip();
            }
        }
    }

    /// What the panes and actions are now: `config.toml`'s, then those of approved packs.
    pub(super) fn refresh_panes(&mut self) {
        let (panes, actions) = crate::packs::effective(
            self.cfg.script_panes(),
            self.cfg.named_actions(),
            &self.packs,
        );
        self.model.apply_scripts(&panes);
        self.model.apply_actions(&actions);
        self.actions = actions;
        // An open Settings window lists the panes by name: a deleted pack or `[[script_pane]]`
        // has to leave it too.
        let names: Vec<String> = panes.into_iter().map(|p| p.name).collect();
        crate::render::set_script_names(names.clone());
        self.carry_slot_settings(names);
        crate::settings_win::refresh(&self.cfg);
    }

    /// The panes' names in the slots changed: move each pane's switch and place with it
    /// (see `packs::remap_slots`). The write comes back through the config watcher, by which
    /// time the names match and nothing more happens.
    fn carry_slot_settings(&mut self, names: Vec<String>) {
        if !self.packs_seen || self.slot_names.as_ref() == Some(&names) {
            return;
        }
        if let Some(old) = self.slot_names.replace(names.clone()) {
            let p = &self.cfg.panes;
            let on = std::array::from_fn(|i| p.script_on(i));
            let (next, order) = crate::packs::remap_slots(&old, &names, on, &p.order);
            use crate::config::{set, set_text, Val};
            for i in 0..crate::config::MAX_SCRIPT_PANES {
                if next[i] != on[i] {
                    let _ = set(crate::ui_state::SCRIPT_SWITCH_KEYS[i], Val::B(next[i]));
                }
            }
            if order != p.order {
                let _ = set_text("panes.order", &order);
            }
        }
        crate::packs::save_slot_names(&names);
    }

    /// The action called `name` (any case), from `config.toml` or an approved pack.
    pub(super) fn named_action(&self, name: &str) -> Option<crate::config::NamedAction> {
        self.actions
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name.trim()))
            .cloned()
    }

    /// A card for each new or changed pack that has not been answered (with its command shown),
    /// or whose `pane.toml` has a mistake. Each is asked once a session.
    pub(super) fn ask_packs(&mut self, now: Instant) {
        use crate::packs::State;
        // What is asked about: the folder, the `pane.toml` hash and the version `hytte pack`
        // installed (an update can change a script and leave `pane.toml` alone).
        let key_of = |p: &crate::packs::Pack| {
            format!(
                "{}:{}:{}",
                p.folder,
                p.hash,
                crate::registry::version_of(&p.path)
            )
        };
        // A pack that is no longer waiting (answered, removed) is forgotten, so if it comes
        // back waiting (an answer dropped by `hytte pack update`, or made again) it asks again.
        let waiting: Vec<String> = self
            .packs
            .iter()
            .filter(|p| p.state == State::Pending)
            .map(key_of)
            .collect();
        self.packs_asked.retain(|k| waiting.contains(k));
        for p in self.packs.clone() {
            let key = key_of(&p);
            if self.packs_asked.contains(&key) {
                continue;
            }
            let mut c = match &p.state {
                State::Pending => {
                    let mut c = Chip::new(format!("Run the pane \"{}\"?", p.name), now);
                    let shown = crate::ui_state::pack_run_note(&p.command, p.actions.len());
                    c.note = Some(shown);
                    c.extra = vec![
                        (
                            "Run".into(),
                            ChipAction::PackAnswer(p.folder.clone(), p.hash.clone(), true),
                        ),
                        (
                            "Ignore".into(),
                            ChipAction::PackAnswer(p.folder.clone(), p.hash.clone(), false),
                        ),
                    ];
                    c.open = Some(p.path.clone());
                    c
                }
                State::Bad(why) => {
                    let mut c = Chip::new(format!("Pane pack \"{}\": {why}", p.folder), now);
                    c.open = Some(p.path.join("pane.toml"));
                    c
                }
                _ => continue,
            };
            self.packs_asked.push(key.clone());
            c.tone = crate::ui_state::Tone::Warn;
            c.prio = crate::ui_state::Prio::Alert;
            c.hold = Duration::from_secs(60);
            c.key = Some(format!("pack:{}", p.folder));
            self.model.push_chip(c, now);
        }
    }

    /// Run the `[[action]]` called `name`: the first click of one that needs confirming only
    /// arms it. The command runs on a worker (hidden, in a Job, stopped at its timeout) and its
    /// result comes back as `UiEvent::ActionDone`. True when it started.
    pub(super) fn run_named(&mut self, name: &str, arg: Option<&str>) -> bool {
        self.run_named_with(name, arg, false)
    }

    /// `approved`: a human already said yes (an agent's `action_run`), so the action's own
    /// `confirm` arming is skipped.
    pub(super) fn run_named_with(&mut self, name: &str, arg: Option<&str>, approved: bool) -> bool {
        use crate::ui_state::Press;
        let now = Instant::now();
        let Some(a) = self.named_action(name) else {
            // A name config.toml doesn't have (a script asked for it, or it was removed).
            let mut c = Chip::new(format!("No action called \"{name}\" in config.toml"), now);
            c.tone = crate::ui_state::Tone::Warn;
            self.model.interrupt(c, now);
            return false;
        };
        // Rows of one action with different args are separate presses (armed, running).
        let key = crate::ui_state::action_key(&a.name, arg);
        match self.model.press_action(&key, a.confirm && !approved, now) {
            Press::Arm | Press::Busy => false,
            Press::Run => {
                let sh = self.shared.clone();
                let arg = arg.map(str::to_owned);
                std::thread::spawn(move || {
                    crate::proc::eco_thread();
                    let run = crate::scriptpane::run_with(
                        &a.command,
                        std::time::Duration::from_secs(a.timeout),
                        a.dir.as_deref(),
                        arg.as_deref(),
                    );
                    push_event(&sh, UiEvent::ActionDone(key, run));
                });
                true
            }
        }
    }

    /// A script's request (`hytte notify`, `hytte card`, `hytte shelf add`). Paused or in a
    /// game nothing may pop up (a waiting `card` gets a hang-up: no press), and a notify card
    /// never covers another one. Live cards queue for the chip (`after_model_change`).
    pub(super) fn run_cmd(
        &mut self,
        c: hytte_proto::Cmd,
        reply: Option<Sender<String>>,
        now: Instant,
    ) {
        if self.paused || self.gaming {
            return;
        }
        // Never run from here: the pipe asks the human first and sends `ActionApproved`.
        if c.cmd == "action_run" {
            return;
        }
        if c.cmd == "pane_set" {
            // A name no [[script_pane]] has is ignored (the CLI already said so).
            let (name, text) = (c.id.unwrap_or_default(), c.text.unwrap_or_default());
            self.model.script_text(&name, &text, None, now);
            return;
        }
        if matches!(c.cmd.as_str(), "note_add" | "todo_add") {
            // The same list and the same 24-hour rule as typing into the pane. A full list
            // drops the item (the request is one-way, so the sender is not told).
            let kind = if c.cmd == "note_add" {
                crate::jot::Kind::Note
            } else {
                crate::jot::Kind::Todo
            };
            let text = c.text.unwrap_or_default();
            if self
                .model
                .jot
                .add(kind, &text, c.keep, crate::timer::now_ms())
                .is_some()
            {
                self.model.jot_dirty = true;
            }
            return;
        }
        if c.cmd == "card" {
            let id = self.model.card(&c, now);
            if let Some(r) = reply {
                self.card_waits.retain(|w| w.0 != id);
                self.card_waits.push((id, r));
            }
            return;
        }
        if c.cmd == "notify" {
            self.model
                .push_chip(Chip::new(c.text.unwrap_or_default(), now), now);
            return;
        }
        let job = DropJob {
            paths: c.path.into_iter().map(Into::into).collect(),
            text: c.text,
            op: None,
        };
        push_event(&self.shared, UiEvent::ShelfAdd(job));
    }

    /// Put the welcome tour's current card on the chip. Unanswered, it folds away after a
    /// while and comes back on the next hover; only Done or Skip end the tour.
    pub(super) fn show_welcome(&mut self, now: Instant) {
        if let Some((tour, step)) = self.welcome {
            let hotkey = &self.cfg.general.hotkey;
            self.model.chip = Some(crate::welcome::chip(tour, step, hotkey, now));
        }
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
        c.prio = crate::ui_state::Prio::Update;
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
        c.prio = crate::ui_state::Prio::Ask;
        self.model.requeue_card();
        c.extra = vec![
            ("Allow".into(), ChipAction::Answer(*id, true)),
            ("Deny".into(), ChipAction::Answer(*id, false)),
        ];
        self.model.chip = Some(c);
        self.model.chip_armed = None;
        self.ask_shown = Some(now);
    }
}
