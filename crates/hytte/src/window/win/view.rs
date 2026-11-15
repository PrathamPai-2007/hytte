//! The pill window: where it sits, which monitor, what is under the pointer.

use super::*;

impl Ui {
    pub(super) fn kick(&mut self) {
        if !self.shared.animating.swap(true, Ordering::SeqCst) {
            self.last = Instant::now();
        }
        if let Some(t) = self.shared.ticker.get() {
            t.unpark();
        }
        post(WM_TICK);
    }

    pub(super) fn sentinel(&self) -> bool {
        self.fs_hidden && self.cfg.general.fullscreen != "hide"
    }

    pub(super) fn hidden(&self) -> bool {
        // A drag near the top edge or a fresh drop must stay reachable even over fullscreen.
        self.paused
            || self.gaming
            || (self.fs_hidden
                && self.cfg.general.fullscreen == "hide"
                && !self.armed
                && !self.model.forced())
    }

    pub(super) fn layout(&mut self) {
        self.model.keyboard = self.kb.is_some();
        self.measure_replies();
        let scene = self.model.scene(self.hover, self.sentinel());
        let size = self.model.size(scene);
        let glow = self.model.glow(scene);
        self.anim.set_scene(scene, size, glow);
        let bubble = self.model.bubble(scene);
        self.anim.set_bubble(bubble);
        self.anim
            .hover
            .set_target(if self.inside && scene != Scene::Sentinel {
                1.0
            } else {
                0.0
            });
        self.anim
            .vis
            .set_target(if self.hidden() { 0.0 } else { 1.0 });
    }

    /// Cursor in the coordinates of the last drawn frame (logical px).
    pub(super) fn mouse_logical(&self) -> Option<(f32, f32)> {
        let (sx, sy) = self.mouse?;
        Some((
            (sx - self.origin.0) as f32 / self.scale,
            (sy - self.origin.1) as f32 / self.scale,
        ))
    }

    pub(super) fn set_mouse_client(&mut self, lparam: LPARAM) {
        self.mouse = Some((self.origin.0 + lo(lparam), self.origin.1 + hi(lparam)));
    }

    /// The pointer is on the pill, or on the bubble beside it.
    pub(super) fn pill_contains(&self) -> bool {
        let Some((mx, my)) = self.mouse_logical() else {
            return false;
        };
        let pw = self.anim.rect.w.pos as f32;
        let ph = (self.anim.rect.h.pos as f32).max(6.0);
        let ox = (self.crop_w - pw) / 2.0;
        let on_pill = mx >= ox && mx <= ox + pw && my >= 0.0 && my <= ph;
        let on_zone = self.hover && self.spot_at().is_some();
        on_pill || on_zone || self.bubble_hit().is_some()
    }

    /// Which of the two kept spots the pointer is on (the top 32 px strip only).
    pub(super) fn spot_at(&self) -> Option<Spot> {
        let (mx, my) = self.mouse_logical()?;
        let z = self.zone.as_ref()?;
        let dx = mx - self.crop_w / 2.0;
        if !(0.0..=32.0).contains(&my) {
            return None;
        }
        if dx.abs() <= z.pill_w / 2.0 {
            Some(Spot::Pill)
        } else if dx >= z.dx && dx <= z.dx + z.w {
            Some(Spot::Bubble)
        } else {
            None
        }
    }

    /// Gliding along the top strip from the bubble to the pill (or back) shows that one's
    /// pane. Entering the strip from below, or the gap between, changes nothing.
    pub(super) fn follow_spot(&mut self) {
        if !self.hover {
            return;
        }
        let (mx, my) = match self.mouse_logical() {
            Some(m) => m,
            None => return,
        };
        let now = self.spot_at();
        let Some(z) = self.zone.as_mut() else { return };
        if my > 32.0 || mx < 0.0 {
            // Left the strip: the next spot entered is not a glide.
            z.at = None;
            return;
        }
        let Some(now) = now else { return };
        let (was, kind, base) = (z.at.replace(now), z.kind, z.base);
        match (was, now) {
            (Some(Spot::Bubble), Spot::Pill) => {
                self.model.timer_panel = false;
                self.model.selected = base;
                self.anim.nav = 0.0;
            }
            (Some(Spot::Pill), Spot::Bubble) if self.model.panels().contains(&kind.panel()) => {
                self.select_panel(kind.panel());
            }
            _ => {}
        }
    }

    /// The bubble (the second live activity beside a compact pill) under the pointer.
    pub(super) fn bubble_hit(&self) -> Option<Bubble> {
        let (mx, my) = self.mouse_logical()?;
        let kind = self.anim.bubble_kind?;
        let s = self.anim.bubble.pos as f32;
        // Only while it is out and still wanted (not while it slides back in).
        if s < 0.5
            || self
                .model
                .bubble(self.model.scene(self.hover, self.sentinel()))
                != Some(kind)
        {
            return None;
        }
        let pw = self.anim.rect.w.pos as f32;
        let ox = (self.crop_w - pw) / 2.0;
        let x = ox + kind.x(pw, s);
        let bh = (self.anim.rect.h.pos as f32).clamp(6.0, 32.0);
        (mx >= x && mx <= x + kind.width() && my >= 0.0 && my <= bh).then_some(kind)
    }

    pub(super) fn crop(&self) -> Crop {
        self.rend.plan(
            self.anim.rect.w.pos as f32,
            self.anim.rect.h.pos as f32,
            self.armed,
            self.anim.bubble_room(),
            self.glowing(),
        )
    }

    pub(super) fn glowing(&self) -> bool {
        self.anim.glow.pos.max(self.anim.glow.target) > 0.01
    }

    pub(super) fn origin_for(&self, crop: Crop) -> (i32, i32) {
        let mon_w = self.mon.2 - self.mon.0;
        (
            self.mon.0 + (mon_w - crop.w_px) / 2 + self.offset_px(),
            self.mon.1,
        )
    }

    /// The sideways shift of the pill from the middle of its monitor, in px, kept on the
    /// monitor. While dragging it is the live value; otherwise this monitor's saved one.
    pub(super) fn offset_px(&self) -> i32 {
        let frac = self
            .live
            .unwrap_or_else(|| self.positions.get(&self.mon_name));
        let mon_w = self.mon.2 - self.mon.0;
        crate::position::offset_px(
            frac,
            mon_w,
            self.pill_px(),
            self.bubble_extra_px(),
            self.margin_px(),
        )
    }

    /// How far the second activity's bubble sticks out to the right of the pill, in px (it
    /// slides out of the pill, so this grows with it). Nothing on the left, so the pill
    /// itself can still sit at the left edge.
    fn bubble_extra_px(&self) -> i32 {
        let Some(kind) = self.anim.bubble_kind else {
            return 0;
        };
        if self.model.placing {
            return 0;
        }
        let s = self.anim.bubble.pos as f32;
        (((kind.width() + 14.0) * s - 6.0).max(0.0) * self.scale).ceil() as i32
    }

    /// The pill's width on screen in px. The window is a wider, transparent canvas (glow,
    /// drop zone, bubble room) clipped to what shows, so the edge test uses this and
    /// [`Self::bubble_extra_px`], not the canvas.
    fn pill_px(&self) -> i32 {
        (self.anim.rect.w.pos as f32 * self.scale).ceil() as i32
    }

    fn margin_px(&self) -> i32 {
        (crate::position::MARGIN * self.scale) as i32
    }

    /// The pointer is on the grab strip: the top few px of the pill, over nothing clickable.
    pub(super) fn on_strip(&self) -> bool {
        if self.hidden() || self.model.typing.is_some() {
            return false;
        }
        let Some((mx, my)) = self.mouse_logical() else {
            return false;
        };
        let pw = self.anim.rect.w.pos as f32;
        let ph = self.anim.rect.h.pos as f32;
        let ox = (self.crop_w - pw) / 2.0;
        my >= 0.0
            && my <= crate::position::STRIP.min(ph.max(6.0))
            && mx >= ox
            && mx <= ox + pw
            && self.hit_at().is_none()
    }

    /// Button down on the strip: a drag may follow (it starts after a few px of movement).
    pub(super) fn place_press(&mut self) -> bool {
        let mut p = POINT::default();
        if self.place.is_some() || !self.on_strip() || unsafe { GetCursorPos(&mut p) }.is_err() {
            return false;
        }
        self.place = Some(crate::position::Drag::new(p.x, self.offset_px()));
        PLACING.store(true, Ordering::Relaxed);
        true
    }

    /// The pointer moved with the button held: start dragging past the threshold, then follow.
    pub(super) fn place_move(&mut self) {
        let Some(mut d) = self.place else { return };
        let mut p = POINT::default();
        if unsafe { GetCursorPos(&mut p) }.is_err() {
            return;
        }
        self.mouse = Some((p.x, p.y));
        if !d.active {
            let thr = (crate::position::THRESHOLD * self.scale) as i32;
            if !d.past_threshold(p.x, thr) {
                return;
            }
            d.active = true;
            // Drag mode shows instructions in place of the pane; a drop zone has no use now.
            self.model.placing = true;
            self.armed = false;
            ARMED.store(false, Ordering::Relaxed);
        }
        self.place = Some(d);
        let mon_w = self.mon.2 - self.mon.0;
        let narrowest = (crate::position::MIN_PILL * self.scale) as i32;
        let max = crate::position::max_shift(mon_w, narrowest, self.margin_px());
        self.live = Some(crate::position::offset_frac(d.offset_at(p.x, max), mon_w));
        self.layout();
        self.kick();
    }

    /// End a press or drag. `keep` stores where the pill was let go (a drag), else the old
    /// spot comes back. Returns whether a drag was on screen (so the release is not a click).
    pub(super) fn place_end(&mut self, keep: bool) -> bool {
        PLACING.store(false, Ordering::Relaxed);
        let Some(d) = self.place.take() else {
            return false;
        };
        if d.active {
            if let (true, Some(f)) = (keep, self.live) {
                self.positions.set(&self.mon_name, f);
                self.positions.save();
            }
            self.live = None;
            self.model.placing = false;
            self.layout();
            self.kick();
        }
        d.active
    }

    /// Put this monitor's pill back in the middle (a double-click on the strip).
    pub(super) fn place_reset(&mut self) {
        self.positions.set(&self.mon_name, 0.0);
        self.positions.save();
        self.live = None;
        self.kick();
    }

    /// Put every monitor's pill back in the middle (the tray's "Reset pill position").
    pub(super) fn place_reset_all(&mut self) {
        self.positions.clear();
        self.positions.save();
        self.live = None;
        self.kick();
    }

    /// While dragging: Esc, a pause, a game or a file drop over the pill ends it.
    pub(super) fn place_watch(&mut self) {
        if self.place.is_none() {
            return;
        }
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE};
        let esc = unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } as u16 & 0x8000 != 0;
        if esc || self.hidden() || self.model.drop_over {
            self.place_end(false);
            unsafe {
                let _ = ReleaseCapture();
            }
        }
    }

    /// The monitor the pill should hang from, per `[general] monitor`: "active" is the one
    /// holding the foreground window, "cursor" the one under the pointer, anything else
    /// the primary.
    pub(super) fn wanted_monitor(&self) -> HMONITOR {
        unsafe {
            let primary = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            match self.cfg.general.monitor.as_str() {
                "active" => {
                    let fg = GetForegroundWindow();
                    if fg.0.is_null() {
                        primary
                    } else {
                        MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
                    }
                }
                "cursor" => {
                    let mut p = POINT::default();
                    if GetCursorPos(&mut p).is_ok() {
                        MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
                    } else {
                        primary
                    }
                }
                n => match n.parse::<usize>() {
                    Ok(i) if i >= 1 => monitors().get(i - 1).copied().unwrap_or(primary),
                    _ => primary,
                },
            }
        }
    }

    /// "Next monitor": hang the pill from the next monitor (left to right, then round to the
    /// first), and keep that choice in `config.toml` so it survives a restart.
    pub(super) fn next_monitor(&mut self) {
        const INDEX: [&str; 8] = ["1", "2", "3", "4", "5", "6", "7", "8"];
        let all = monitors();
        if all.len() < 2 {
            return;
        }
        let here = all
            .iter()
            .position(|m| m.0 as isize == self.hmon)
            .unwrap_or(0);
        let next = (here + 1) % all.len().min(INDEX.len());
        self.cfg.general.monitor = INDEX[next].into();
        let _ = crate::config::set("general.monitor", crate::config::Val::S(INDEX[next]));
        self.refresh_monitor();
        self.kick();
    }

    /// Move the pill to another monitor when `[general] monitor` says it should follow.
    pub(super) fn follow_monitor(&mut self) {
        if matches!(self.cfg.general.monitor.as_str(), "active" | "cursor") {
            let before = (self.mon, self.scale);
            self.refresh_monitor();
            if before != (self.mon, self.scale) {
                self.kick();
            }
        }
    }

    pub(super) fn refresh_monitor(&mut self) {
        unsafe {
            let hmon = self.wanted_monitor();
            use windows::Win32::Graphics::Gdi::MONITORINFOEXW;
            let mut mie = MONITORINFOEXW::default();
            mie.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            self.hmon = hmon.0 as isize;
            if GetMonitorInfoW(hmon, &mut mie as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool()
            {
                let mi = mie.monitorInfo;
                let name = &mie.szDevice;
                let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                self.mon_name = String::from_utf16_lossy(&name[..len]);
                let r = mi.rcMonitor;
                // A taskbar docked at the top (any edge works since 24H2): hang below it.
                let top = mi.rcWork.top.max(r.top);
                self.mon = (r.left, top, r.right, r.bottom);
            }
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            self.scale = dx as f32 / 96.0 * self.cfg.general.pill_scale.clamp(0.7, 1.6);
        }
        self.rend.set_scale(self.scale);
        self.set_zone((self.mon.0 + self.mon.2) / 2);
    }

    /// Hot zone for drag arming (files dragged to the pill), in screen px, around `centre`.
    pub(super) fn set_zone(&self, centre: i32) {
        let half = (230.0 * self.scale) as i32;
        ZONE_L.store(centre - half, Ordering::Relaxed);
        ZONE_R.store(centre + half, Ordering::Relaxed);
        ZONE_B.store(self.mon.1 + (40.0 * self.scale) as i32, Ordering::Relaxed);
    }
}
