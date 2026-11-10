//! Frames: relayout after a change, drawing, rebuilding the GPU renderer, hits.

use super::*;

impl Ui {
    pub(super) fn after_model_change(&mut self, hwnd: HWND) {
        let now = Instant::now();
        let wall = crate::timer::now_ms();
        if let Some(kind) = self.model.finish_timer(wall, now) {
            if kind == crate::timer::TimerKind::Focus {
                self.model.focus_summary = self.model.held.take_summary();
            }
            let t = &self.cfg.timer;
            self.model.timer_break = crate::timer::break_minutes(
                self.model.focus_done,
                t.rounds,
                t.break_min,
                t.long_break_min,
            );
            self.model.timer_panel = false;
            crate::timer::save(None);
        }
        // 1 Hz redraw for the fuse, only while a timer runs; no timer = no wake-ups.
        match &self.model.timer {
            Some(t) if t.paused_ms.is_none() => unsafe {
                // A Focus line on a bare pill moves a pixel every few seconds: no need to redraw
                // it every one. (A bubble clock or an open pane still wants the second.)
                let quiet = t.kind == crate::timer::TimerKind::Focus
                    && matches!(self.anim.scene, Scene::Idle | Scene::Sentinel);
                let step = if quiet {
                    (t.total_ms / 100).clamp(1000, 30_000)
                } else {
                    1000
                };
                SetTimer(
                    Some(hwnd),
                    T_TIMER,
                    t.remaining_ms(wall).min(step) as u32 + 15,
                    None,
                );
            },
            _ => unsafe {
                let _ = KillTimer(Some(hwnd), T_TIMER);
            },
        }
        self.model.set_idle(input_idle(), now);
        let mut next = self.model.expire(now);
        self.focus_dnd();
        // Break reminders: only when nothing is in the way; a due one waits for the way to clear.
        let clear = !self.model.holding()
            && !self.quiet()
            && !self.hidden()
            && !(self.model.cam || self.model.mic)
            && self.model.chip.is_none()
            && self.model.timer_done.is_none()
            && self.model.nudge.is_none()
            && !self.model.drop_over
            // The pill is in use right now: the nudge waits.
            && self.model.typing.is_none()
            && self.kb.is_none()
            && !self.hover;
        if let Some(kind) = self.breaks.tick(now, input_idle(), clear) {
            self.model.nudge = Some(crate::ui_state::Nudge { kind, since: now });
            next = self.model.expire(now);
        }
        let idle = input_idle();
        for d in [
            self.breaks.next_deadline(now),
            self.breaks.away_deadline(now, idle),
        ]
        .into_iter()
        .flatten()
        {
            next = Some(next.map_or(d, |n| n.min(d)));
        }
        // A field only keeps focus while its pane is the one showing.
        if let Some(f) = self.model.typing {
            if self.model.panel() != typing::field_panel(f) {
                self.end_keyboard(false);
            }
        }
        if self.model.jot_dirty {
            self.model.jot_dirty = false;
            self.model.jot.save(&crate::jot::path());
        }
        if self.model.chip.is_none() && !self.asks.is_empty() {
            // The front ask timed out (its agent asked in the terminal): show the next one.
            self.show_ask(now);
            next = self.model.expire(now);
        }
        if self.model.chip.is_none() && self.update_offer.is_some() {
            self.show_update(now);
            next = self.model.expire(now);
        }
        if self.model.chip.is_none() && !self.model.cards.is_empty() && !self.paused && !self.gaming
        {
            self.model.show_card(now);
            next = self.model.expire(now);
        }
        // The welcome tour: on the next hover, when nothing else wants the pill.
        if self.welcome.is_some()
            && self.hover
            && self.model.chip.is_none()
            && self.model.cards.is_empty()
            && self.asks.is_empty()
            && self.update_offer.is_none()
            && !self.model.holding()
            && !self.quiet()
            && !self.hidden()
            && !(self.model.cam || self.model.mic)
            && self.model.typing.is_none()
            && self.model.timer_done.is_none()
            && self.model.nudge.is_none()
        {
            self.show_welcome(now);
            next = self.model.expire(now);
        }
        // A card that expired or was dismissed answers its waiting script with a hang-up.
        let model = &self.model;
        self.card_waits.retain(|(id, _)| model.has_card(id));
        match next {
            Some(next) => unsafe {
                let ms = next.saturating_duration_since(now).as_millis() as u32 + 20;
                SetTimer(Some(hwnd), T_EXPIRE, ms.max(20), None);
            },
            None => unsafe {
                let _ = KillTimer(Some(hwnd), T_EXPIRE);
            },
        }
        self.sync_ring();
        // Keep awake: held by this thread, released by Windows if Hytte exits.
        let awake = self.model.awake.active(now);
        if awake != self.awake_on {
            self.awake_on = awake;
            use windows::Win32::System::Power::{
                SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
            };
            let flags = if awake {
                ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
            } else {
                ES_CONTINUOUS
            };
            unsafe { SetThreadExecutionState(flags) };
        }
        // A calendar heads-up that has come due becomes a card (never over another one).
        // Switched off in settings: a due heads-up is dropped, not saved for later.
        let calendar = self.cfg.calendar.enabled;
        if let Some(ev) = self.model.calendar_due(now).filter(|_| calendar) {
            let text =
                crate::calendar::summary(&ev.subject, ev.start_ms, crate::timer::now_ms() as i64);
            let mut c = Chip::new(text, now);
            c.prio = crate::ui_state::Prio::Alert;
            c.key = Some("calendar".into());
            c.tone = crate::ui_state::Tone::Info;
            c.hold = Duration::from_secs(60);
            if let Some(url) = ev.url {
                c.extra = vec![("Join".into(), ChipAction::OpenUrl(url))];
            }
            self.model.push_chip(c, now);
        }
        self.request_thumbs();
        self.layout();
        // `nav` only applies to the layout it was set for.
        self.anim.nav = 0.0;
        self.kick();
        self.weather_refresh();
        self.script_refresh(hwnd);
        self.sleep_tick(now);
        self.lyrics_tick();
    }

    pub(super) fn frame(&mut self, hwnd: HWND) {
        let now = Instant::now();
        let gap = now.saturating_duration_since(self.last);
        // While springs move the ticker paces on vblank: use DWM's own clock so dt is a
        // whole number of refresh periods instead of scheduler-jittered wall time.
        let vb = if self.shared.fast.load(Ordering::Relaxed) {
            VBLANK.load(Ordering::Relaxed)
        } else {
            0
        };
        let dt = vblank_dt(self.prev_vblank, vb)
            .map_or(gap.as_secs_f32(), |d| d as f32)
            .min(0.05);
        self.prev_vblank = vb;
        self.last = now;
        self.layout();
        let speed = match self.cfg.general.animation.as_str() {
            "calm" => 0.7,
            "snappy" => 1.4,
            _ => 1.0,
        };
        let mut moving = self.anim.step((dt * speed) as f64);
        // The idle pill fades towards `idle_opacity` and back when it is pointed at.
        let idle = self.model.scene(self.hover, self.sentinel()) == Scene::Idle
            && self.kb.is_none()
            && !self.inside;
        let dim_to = if idle {
            self.cfg.general.idle_opacity.clamp(0.2, 1.0)
        } else {
            1.0
        };
        self.dim += (dim_to - self.dim) * (1.0 - (-dt * 6.0).exp());
        if (dim_to - self.dim).abs() < 0.004 {
            self.dim = dim_to;
        } else {
            moving = true;
        }

        // Visibility: fade out then hide; show before fading in.
        let want_hidden = self.hidden();
        if !want_hidden && !self.shown {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            self.shown = true;
        }
        let faded_out = want_hidden && self.anim.vis.pos < 0.01;
        if faded_out && self.shown {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            self.shown = false;
        }
        if self.shown {
            let crop = self.crop();
            let origin = self.origin_for(crop);
            self.origin = origin;
            self.crop_w = crop.w_log;
            let fr = Frame {
                model: &self.model,
                anim: &self.anim,
                now,
                dt,
                mouse: self.mouse_logical(),
                pressed: self.pressed,
                light: self.light,
                accent: accent_for(self.accent, self.light),
                focus: self.kb.and(self.kb_focus),
                armed: self.armed,
                ringing: self.ringing,
                caret_on: self.caret_on,
            };
            self.rend.draw(&fr, crop, &mut self.hits);
            let drawn = now.elapsed().as_micros() as u32;
            let a = (self.anim.vis.pos.clamp(0.0, 1.0) * self.dim as f64 * 255.0).round() as u8;
            let heading = self
                .rend
                .plan(
                    self.anim.rect.w.target as f32,
                    self.anim.rect.h.target as f32,
                    self.armed,
                    self.anim.bubble_room(),
                    self.glowing(),
                )
                .clip;
            self.rend
                .present(hwnd, crop, origin.0, origin.1, a, heading);
            if self.rend.lost() && self.gpu_retry == 0 {
                // Driver reset or adapter removed: rebuild the GPU pipeline and redraw.
                crate::logging::line("gpu renderer: device lost, rebuilding");
                self.rebuild_gpu(hwnd);
            }
            // Skip the first frame after idle: its gap is the idle time, not a frame.
            if moving && gap.as_millis() < 100 {
                self.perf.record(
                    gap.as_micros() as u32,
                    drawn,
                    now.elapsed().as_micros() as u32,
                );
            }
        }
        // A hovered label scrolling (`Renderer::label`) runs only while it is hovered.
        let ambient = !self.anim.reduce
            && self.shown
            && (self.model.ambient(self.anim.scene, now) || self.rend.wants_frames());
        let sh = &self.shared;
        sh.fast.store(moving, Ordering::Relaxed);
        sh.animating.store(moving || ambient, Ordering::SeqCst);
        if !(moving || ambient) {
            // Settled: give the pages back once it has stayed that way.
            unsafe {
                SetTimer(Some(hwnd), T_TRIM, TRIM_MS, None);
            }
        }
    }

    /// Rebuild a lost GPU pipeline. On failure (driver mid-update, adapter gone) retry on
    /// `T_GPU` with backoff instead of hammering the driver every frame.
    pub(super) fn rebuild_gpu(&mut self, hwnd: HWND) {
        match self.rend.rebuild_gpu(hwnd) {
            Ok(()) => {
                self.gpu_retry = 0;
                self.thumb_req.clear();
                self.kick();
            }
            Err(e) => {
                self.gpu_retry = (self.gpu_retry * 2).clamp(1_000, 30_000);
                crate::logging::line(&format!(
                    "gpu renderer: rebuild failed: {e}; retry in {} ms",
                    self.gpu_retry
                ));
                unsafe {
                    SetTimer(Some(hwnd), T_GPU, self.gpu_retry, None);
                }
            }
        }
    }

    pub(super) fn hit_at(&self) -> Option<&Hit> {
        let (mx, my) = self.mouse_logical()?;
        self.hits
            .iter()
            .rev()
            .find(|h| mx >= h.rect.0 && mx <= h.rect.2 && my >= h.rect.1 && my <= h.rect.3)
    }
}
