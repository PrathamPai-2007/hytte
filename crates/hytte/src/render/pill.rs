//! The pill itself: its shape, glow and background, the bubble, and scene switching.

use super::*;

impl Renderer {
    // ------------------------------------------------------------------ shape

    pub(super) fn pill_geometry(
        &self,
        w: f32,
        h: f32,
        rb: f32,
        e: f32,
    ) -> Option<ID2D1PathGeometry> {
        let key = [w.to_bits(), h.to_bits()];
        if let Some((k, g)) = self.geom.borrow().as_ref() {
            if *k == key {
                return Some(g.clone());
            }
        }
        let g = self.build_pill_geometry(w, h, rb, e)?;
        *self.geom.borrow_mut() = Some((key, g.clone()));
        Some(g)
    }

    pub(super) fn build_pill_geometry(
        &self,
        w: f32,
        h: f32,
        rb: f32,
        e: f32,
    ) -> Option<ID2D1PathGeometry> {
        unsafe {
            let g = self.factory.CreatePathGeometry().ok()?;
            let s = g.Open().ok()?;
            let arc = |x: f32, y: f32, r: f32, cw: bool| D2D1_ARC_SEGMENT {
                point: v(x, y),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: if cw {
                    D2D1_SWEEP_DIRECTION_CLOCKWISE
                } else {
                    D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                },
                arcSize: D2D1_ARC_SIZE_SMALL,
            };
            s.BeginFigure(v(-e, 0.0), D2D1_FIGURE_BEGIN_FILLED);
            s.AddLine(v(w + e, 0.0));
            if e > 0.5 {
                s.AddArc(&arc(w, e, e, false));
            }
            s.AddLine(v(w, h - rb));
            s.AddArc(&arc(w - rb, h, rb, true));
            s.AddLine(v(rb, h));
            s.AddArc(&arc(0.0, h - rb, rb, true));
            s.AddLine(v(0.0, e));
            if e > 0.5 {
                s.AddArc(&arc(-e, 0.0, e, false));
            }
            s.EndFigure(D2D1_FIGURE_END_CLOSED);
            s.Close().ok()?;
            Some(g)
        }
    }

    pub(super) fn background(&self, fr: &Frame, w: f32, h: f32) {
        let an = fr.anim;
        let sentinel = an.scene == Scene::Sentinel;
        let rb = (h * 0.5).min(24.0);
        let e = if h > 18.0 { (h * 0.3).min(8.0) } else { 0.0 };
        if sentinel {
            self.fill_rr(0.0, 0.0, w, h, h / 2.0, color(self.fg(), 0.30));
            return;
        }
        let Some(g) = self.pill_geometry(w, h, rb, e) else {
            return;
        };
        unsafe {
            // Soft glow: stacked widening strokes under the body.
            let mut gl = an.glow.pos as f32 * fr.model.glow_gain;
            if fr.model.primary().is_some_and(|t| t.needs_input())
                && matches!(an.scene, Scene::CompactTask | Scene::ExpTasks)
            {
                gl *= 0.72 + 0.28 * (self.t.get() * 4.0).sin();
            }
            if gl > 0.01 {
                // Five strokes, each standing in for a pair of the nine it replaced: stroking
                // the silhouette is the costliest thing a frame does.
                for j in 1..=5 {
                    let k = |i: i32| (1.0 - i as f32 / 10.0).powi(2);
                    let a = gl * 0.075 * (k(2 * j - 1) + k(2 * j));
                    self.rt.DrawGeometry(
                        &g,
                        self.solid(color(an.glow_rgb, a)),
                        j as f32 * 4.4,
                        &self.round,
                    );
                }
            }
            // Body: near-black with a faint vertical gradient.
            match self.body_brush() {
                Some(b) => {
                    b.SetEndPoint(v(0.0, h.max(1.0)));
                    b.SetOpacity(fr.model.body_alpha);
                    self.rt.FillGeometry(&g, &b, None::<&ID2D1Brush>)
                }
                None => self.rt.FillGeometry(
                    &g,
                    self.solid(color(self.body_colors().0, fr.model.body_alpha)),
                    None::<&ID2D1Brush>,
                ),
            }
            // Tinted glass: the glow's colour pools up from the bottom of the body. It
            // follows the glow spring (and its pulse), so it never needs frames of its own.
            if gl > 0.01 {
                let c = an.glow_rgb;
                let k = if self.light.get() { 0.09 } else { 0.14 };
                if let Some(b) = self.grad(
                    Grad::Tint,
                    [c[0], c[1], c[2], k],
                    &[(0.0, c, 0.0), (1.0, c, k)],
                ) {
                    b.SetStartPoint(v(0.0, h * 0.2));
                    b.SetEndPoint(v(0.0, h));
                    b.SetOpacity(gl.min(1.0));
                    self.rt.FillGeometry(&g, &b, None::<&ID2D1Brush>);
                }
            }
            // Rim: barely there at the screen edge, catching the light round the bottom.
            let fg = self.skin.get().rim(self.fg(), self.light.get());
            let rim = self.grad(
                Grad::Rim,
                [fg[0], fg[1], fg[2], self.light.get() as u8 as f32],
                &[(0.0, fg, 0.03), (1.0, fg, 0.14)],
            );
            match rim {
                Some(b) => {
                    b.SetEndPoint(v(0.0, h.max(1.0)));
                    self.rt.DrawGeometry(&g, &b, 1.0, &self.round);
                }
                None => self
                    .rt
                    .DrawGeometry(&g, self.solid(color(fg, 0.07)), 1.0, &self.round),
            }
            let hv = an.hover.pos as f32;
            if hv > 0.01 {
                self.rt.FillGeometry(
                    &g,
                    self.solid(color(self.fg(), 0.025 * hv)),
                    None::<&ID2D1Brush>,
                );
            }
        }
    }

    // ----------------------------------------------------------------- scenes

    /// The small pill split off to the right of a compact one: it slides out from behind the
    /// main pill as the spring runs, and slides back in when the second activity ends.
    pub(super) fn bubble(&self, fr: &Frame, pw: f32, ph: f32) {
        let an = fr.anim;
        let s = an.bubble.pos as f32;
        let Some(kind) = an.bubble_kind.filter(|_| s > 0.01) else {
            return;
        };
        let bw = kind.width();
        let bh = ph.min(32.0);
        let x = kind.x(pw, s);
        let a = s.clamp(0.0, 1.0);
        let (_, bottom) = self.body_colors();
        // Hangs from the top edge like the pill: its top corners are above the screen.
        self.fill_rr(x, -bh / 2.0, bw, bh * 1.5, bh / 2.0, color(bottom, a));
        self.stroke_rr(
            x,
            -bh / 2.0,
            bw,
            bh * 1.5,
            bh / 2.0,
            color(self.fg(), 0.07 * a),
            1.0,
            false,
        );
        let cy = bh / 2.0;
        match kind {
            Bubble::Timer => {
                let Some(t) = fr.model.timer.as_ref() else {
                    return;
                };
                let wall = crate::timer::now_ms();
                let left = t.remaining_ms(wall);
                let col = if left < 60_000 { RED } else { BLUE };
                self.ca.set(a);
                let clock = if left >= 3_600_000 {
                    format!("{}:{:02}h", left / 3_600_000, (left / 60_000) % 60)
                } else {
                    fmt_clock(left)
                };
                self.text(
                    &clock,
                    &self.f.btn,
                    x,
                    cy - 8.0,
                    bw,
                    16.0,
                    self.cc(col, 1.0),
                );
            }
            Bubble::Media => {
                self.ca.set(a);
                self.art_tile(x + (bw - 20.0) / 2.0, cy - 10.0, 20.0, 10.0);
            }
        }
        self.ca.set(1.0);
    }

    pub(super) fn scene(&self, fr: &Frame, w: f32, h: f32) {
        self.bubble(fr, w, h);
        self.background(fr, w, h);
        let an = fr.anim;
        if an.scene == Scene::Sentinel {
            return;
        }
        if let Some(t) = fr
            .model
            .timer
            .as_ref()
            // Collapsed only: expanded, the timer pane and the Timer tile show it.
            .filter(|_| {
                matches!(
                    an.scene,
                    Scene::Idle | Scene::CompactTask | Scene::CompactMedia
                )
            })
        {
            let m = fr.model;
            let centre = an.scene == Scene::Idle && m.ports.is_empty() && m.shelf.is_empty();
            self.timer_filament(t, w, h, centre, if m.mic_muted { LOCK_W } else { 0.0 });
        }
        let m = fr.model;
        let lock = if m.mic_muted { LOCK_W } else { 0.0 };
        let pad_r = lock
            + if m.privacy_active() {
                12.0 + 14.0 * (m.cam as u8 + m.mic as u8) as f32
            } else {
                0.0
            };
        let fit = ((h - 14.0) / 10.0).clamp(0.0, 1.0);
        // Keyboard mode: the scene and its tabs sit above a line of key hints.
        let hint = if m.keyboard && crate::ui_state::hinted(an.scene) {
            crate::ui_state::KB_HINT_H
        } else {
            0.0
        };
        let full_h = h;
        let h = h - hint;
        self.morph.set(self.morph_for(fr));
        self.hero_right
            .set(w - 14.0 - pad_r - if m.visible_count() > 1 { 28.0 } else { 0.0 });
        // The scene being left fades out and drifts up, clipped to the pill as it
        // changes size. It is not interactive: no hover, and its hit areas are dropped.
        let o = an.out.pos.clamp(0.0, 1.0) as f32;
        if o > 0.01 && an.prev != an.scene && an.prev != Scene::Sentinel {
            self.ca.set(o * fit);
            let mouse = self.mouse.take();
            let n = self.hits.borrow().len();
            unsafe {
                let (dx, dy) = if an.slide != 0.0 {
                    (-an.slide * (1.0 - o) * 16.0, 0.0)
                } else {
                    (0.0, -(1.0 - o) * 4.0)
                };
                self.rt
                    .SetTransform(&mat(self.scale, self.ox.get() + dx, dy));
                self.rt
                    .PushAxisAlignedClip(&rect(0.0, 0.0, w, h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
            // dt = 0: progress smoothing already advances once this frame, in the live scene.
            let old = Frame { dt: 0.0, ..*fr };
            self.content(&old, an.prev, w, h, pad_r);
            unsafe { self.rt.PopAxisAlignedClip() };
            self.hits.borrow_mut().truncate(n);
            self.mouse.set(mouse);
        }
        // Content fades/slides in on scene change.
        let c = an.content.pos.clamp(0.0, 1.0) as f32;
        self.ca.set(c * fit);
        let (dx, dy) = if an.slide != 0.0 {
            (an.slide * (1.0 - c) * 16.0, 0.0)
        } else {
            (0.0, (1.0 - c) * 6.0)
        };
        unsafe {
            self.rt
                .SetTransform(&mat(self.scale, self.ox.get() + dx, dy))
        };
        self.focus_live.set(true);
        self.content(fr, an.scene, w, h, pad_r);
        self.focus_live.set(false);
        self.tab_strip(fr, w, h, fit);
        if hint > 0.0 {
            self.text(
                m.kb_hint(),
                &self.f.small_c,
                0.0,
                full_h - hint - 2.0,
                w,
                hint,
                self.cc(GRAY, 0.85 * c),
            );
        }
        self.morph.set(None);
        // The quick controls spell out the mic state (its tile), not the camera's: keep the dots.
        let home = an.scene == Scene::ExpHome;
        if m.privacy_active() {
            let cy = if h < 40.0 { h / 2.0 } else { 16.0 };
            self.privacy_dots(
                m,
                w - 14.0 - lock,
                cy,
                !matches!(
                    an.scene,
                    Scene::Idle | Scene::CompactTask | Scene::CompactMedia
                ),
            );
        }
        if m.mic_muted
            && !home
            && !matches!(
                an.scene,
                Scene::ExpDrop
                    | Scene::ExpChip
                    | Scene::ExpTimer
                    | Scene::ExpTimerDone
                    | Scene::ExpSettings
            )
        {
            let cy = if h < 40.0 { h / 2.0 } else { 16.0 };
            self.mute_badge(w - 14.0 - 6.0, cy);
        }
    }

    /// Body of one scene (and its tab strip), in pill-local coordinates.
    pub(super) fn content(&self, fr: &Frame, scene: Scene, w: f32, h: f32, pad_r: f32) {
        match scene {
            Scene::Sentinel => {}
            Scene::Idle => self.idle(fr, w, h),
            Scene::CompactTask => self.compact_task(fr, w, h, pad_r),
            Scene::CompactMedia => self.compact_media(fr, w, h, pad_r),
            Scene::ExpTasks => self.exp_tasks(fr, w, h),
            Scene::ExpMedia => self.exp_media(fr, w, h),
            Scene::ExpShelf => self.exp_shelf(fr, w, h),
            Scene::ExpHome => self.exp_home(fr, w, h),
            Scene::ExpPower => self.exp_power(fr, w, h),
            Scene::ExpNudge => self.exp_nudge(fr, w, h),
            Scene::ExpNotes => self.exp_notes(fr, w, h),
            Scene::ExpTodo => self.exp_todo(fr, w, h),
            Scene::ExpWeather => self.exp_weather(fr, w, h),
            Scene::ExpScript(i) => self.exp_script(fr, i, w, h),
            Scene::ExpDrop => self.exp_drop(w, h),
            Scene::ExpChip => self.exp_chip(fr, w, h),
            Scene::ExpTimer => self.exp_timer(fr, w, h),
            Scene::ExpTimerDone => self.exp_timer_done(fr, w, h),
            Scene::ExpCharge => self.exp_charge(fr, w, h),
            Scene::ExpSettings => self.exp_settings(fr, w, h),
        }
    }

    /// Progress of a compact <-> expanded transition of the same subject (media with media,
    /// the task list with the compact task), taken from the pill's own height spring so the
    /// shared elements move exactly as fast as the pill grows. None for any other change.
    pub(super) fn morph_for(&self, fr: &Frame) -> Option<f32> {
        let an = fr.anim;
        let exp = match (an.prev, an.scene) {
            (Scene::CompactMedia, Scene::ExpMedia) | (Scene::ExpMedia, Scene::CompactMedia) => {
                Scene::ExpMedia
            }
            (Scene::CompactTask, Scene::ExpTasks) | (Scene::ExpTasks, Scene::CompactTask) => {
                Scene::ExpTasks
            }
            _ => return None,
        };
        let he = fr.model.size(exp).1 as f32;
        Some(((an.rect.h.pos as f32 - 32.0) / (he - 32.0)).clamp(0.0, 1.0))
    }

    /// Where an element shared by a compact scene and its expanded twin goes. While the pill
    /// morphs it slides between `compact` and `exp` and is drawn fully opaque (so the
    /// cross-fading old and new scenes don't dim it); otherwise it is at `natural`. Returns
    /// the rect and what to restore with `hero_end`. The scenes' own slide-and-drift offsets
    /// are dropped too, or the outgoing and incoming copies would sit a few pixels apart.
    pub(super) fn hero(
        &self,
        compact: [f32; 4],
        exp: [f32; 4],
        natural: [f32; 4],
    ) -> ([f32; 4], Option<(f32, Matrix3x2)>) {
        match self.morph.get() {
            Some(p) => {
                let mut r = [0.0; 4];
                for i in 0..4 {
                    r[i] = compact[i] + (exp[i] - compact[i]) * p;
                }
                let mut m = Matrix3x2::default();
                // SAFETY: `m` is a valid out-struct for the call.
                unsafe {
                    self.rt.GetTransform(&mut m);
                    self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0));
                }
                (r, Some((self.ca.replace(1.0), m)))
            }
            None => (natural, None),
        }
    }

    /// Width of a task row's label in the expanded list.
    pub(super) fn exp_label_w(w: f32, t: &TaskView) -> f32 {
        w - 42.0 - 74.0 - if t.running() { 0.0 } else { 24.0 } - 22.0
    }

    /// The compact task label's rect and the same label's rect in the first expanded row.
    pub(super) fn label_ends(&self, w: f32, t: &TaskView) -> ([f32; 4], [f32; 4]) {
        (
            [38.0, 6.0, self.hero_right.get() - 62.0 - 46.0, 17.0],
            [42.0, 17.0, Self::exp_label_w(w, t), 20.0],
        )
    }

    pub(super) fn hero_end(&self, saved: Option<(f32, Matrix3x2)>) {
        if let Some((a, m)) = saved {
            self.ca.set(a);
            unsafe { self.rt.SetTransform(&m) };
        }
    }
}
