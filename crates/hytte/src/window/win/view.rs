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
        (self.mon.0 + (mon_w - crop.w_px) / 2, self.mon.1)
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
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            self.hmon = hmon.0 as isize;
            if GetMonitorInfoW(hmon, &mut mi).as_bool() {
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
        // Hot zone for drag arming, in screen px.
        let half = (230.0 * self.scale) as i32;
        let mid = (self.mon.0 + self.mon.2) / 2;
        ZONE_L.store(mid - half, Ordering::Relaxed);
        ZONE_R.store(mid + half, Ordering::Relaxed);
        ZONE_B.store(self.mon.1 + (40.0 * self.scale) as i32, Ordering::Relaxed);
    }
}
