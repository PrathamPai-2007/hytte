//! The pill itself: its shape, glow and background, the bubble, and scene switching.

use super::*;

/// Room around the pill in the cached background, for the glow (as in `Renderer::plan`).
const BG_MARGIN: f32 = 28.0;

/// What the background is painted from, besides the theme and skin.
struct BgPaint<'a> {
    g: &'a ID2D1PathGeometry,
    h: f32,
    /// Glow strength (the spring times the setting), before any breathing.
    gl: f32,
    rgb: [f32; 3],
    body_alpha: f32,
    hover: f32,
}

/// The background of a steady pill, kept as pictures (see `Renderer::background`).
#[derive(Default)]
pub(super) struct BgCache {
    /// What the last frame's background depended on.
    last: Option<[u32; 13]>,
    layers: Option<BgLayers>,
}

struct BgLayers {
    glow: Option<ID2D1Bitmap>,
    body: Option<ID2D1Bitmap>,
    /// Size in device pixels.
    w: f32,
    h: f32,
}

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

    /// The pill's background: glow, body, tint, rim and hover sheen.
    ///
    /// It is most of what a frame costs (stroking and filling the silhouette on the CPU
    /// renderer), and in a steady pill it is the same picture every frame while a spinner or
    /// the equaliser moves in front of it. So once it has been the same for two frames it is
    /// painted into two bitmaps (the glow, and everything else) and copied from them until
    /// something it depends on changes. While the pill resizes or the glow fades it is painted
    /// directly, as before.
    pub(super) fn background(&self, fr: &Frame, w: f32, h: f32) {
        let an = fr.anim;
        if an.scene == Scene::Sentinel {
            self.fill_rr(0.0, 0.0, w, h, h / 2.0, color(self.fg(), 0.30));
            return;
        }
        let rb = (h * 0.5).min(24.0);
        let e = if h > 18.0 { (h * 0.3).min(8.0) } else { 0.0 };
        let Some(g) = self.pill_geometry(w, h, rb, e) else {
            return;
        };
        // The glow's strength, and the breathing of a waiting agent's. The breath only moves
        // the glow outside the pill: the tint inside holds, so the cached body stays valid
        // through the commonest long wait.
        let gl = an.glow.pos as f32 * fr.model.glow_gain;
        let breath = if fr.model.primary().is_some_and(|t| t.needs_input())
            && matches!(an.scene, Scene::CompactTask | Scene::ExpTasks)
        {
            0.72 + 0.28 * (self.t.get() * 4.0).sin()
        } else {
            1.0
        };
        let p = BgPaint {
            g: &g,
            h,
            gl,
            rgb: an.glow_rgb,
            body_alpha: fr.model.body_alpha,
            hover: an.hover.pos as f32,
        };
        // Where the bitmaps sit: whole device pixels, with the pill's sub-pixel phase kept
        // inside them, so a copy lands exactly where the direct painting would.
        let dev_x = (self.ox.get() - BG_MARGIN) * self.scale;
        let (ix, frac) = (dev_x.floor(), dev_x - dev_x.floor());
        let key = [
            w.to_bits(),
            h.to_bits(),
            self.scale.to_bits(),
            frac.to_bits(),
            gl.to_bits(),
            p.rgb[0].to_bits(),
            p.rgb[1].to_bits(),
            p.rgb[2].to_bits(),
            p.body_alpha.to_bits(),
            self.light.get() as u32,
            self.skin.get() as u32,
            self.skin.get().cache_key().to_bits(),
            p.hover.to_bits(),
        ];
        let mut c = self.bg.borrow_mut();
        let steady = c.last == Some(key);
        c.last = Some(key);
        if !steady {
            c.layers = None;
        } else if c.layers.is_none() {
            c.layers = self.bg_layers(&p, w, frac);
        }
        match &c.layers {
            Some(l) => unsafe {
                let mut old = Matrix3x2::default();
                self.rt.GetTransform(&mut old);
                self.rt.SetTransform(&mat(1.0, 0.0, 0.0));
                let dest = rect(ix, 0.0, l.w, l.h);
                for (bmp, opacity) in [(&l.glow, breath), (&l.body, 1.0)] {
                    if let Some(b) = bmp {
                        self.rt.DrawBitmap(
                            b,
                            Some(&dest),
                            opacity,
                            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                            None,
                        );
                    }
                }
                self.rt.SetTransform(&old);
            },
            None => {
                drop(c);
                self.paint_glow(&self.rt, &p, breath);
                self.paint_body(&self.rt, &p);
            }
        }
    }

    /// The two bitmaps of a steady background, or None if they can't be made (it is then
    /// painted directly).
    fn bg_layers(&self, p: &BgPaint, w: f32, frac: f32) -> Option<BgLayers> {
        let s = self.scale;
        let (pw, ph) = (
            ((w + 2.0 * BG_MARGIN) * s).ceil() + 1.0,
            ((p.h + BG_MARGIN) * s).ceil(),
        );
        let layer = |paint: &dyn Fn(&ID2D1RenderTarget)| -> Option<ID2D1Bitmap> {
            // SAFETY: plain Direct2D calls on a target made here; the size is a valid struct.
            unsafe {
                let size = D2D_SIZE_U {
                    width: pw as u32,
                    height: ph as u32,
                };
                let t = self
                    .rt
                    .CreateCompatibleRenderTarget(
                        None,
                        Some(&size),
                        None,
                        D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE,
                    )
                    .ok()?;
                let rt: ID2D1RenderTarget = t.cast().ok()?;
                rt.BeginDraw();
                rt.Clear(Some(&color(WHITE, 0.0)));
                rt.SetTransform(&Matrix3x2 {
                    M11: s,
                    M12: 0.0,
                    M21: 0.0,
                    M22: s,
                    M31: BG_MARGIN * s + frac,
                    M32: 0.0,
                });
                paint(&rt);
                rt.EndDraw(None, None).ok()?;
                t.GetBitmap().ok()
            }
        };
        let glow = if p.gl > 0.01 {
            Some(layer(&|rt| self.paint_glow(rt, p, 1.0))?)
        } else {
            None
        };
        let body = Some(layer(&|rt| self.paint_body(rt, p))?);
        Some(BgLayers {
            glow,
            body,
            w: pw,
            h: ph,
        })
    }

    /// Soft glow: stacked widening strokes under the body, at `breath` of their strength.
    fn paint_glow(&self, rt: &ID2D1RenderTarget, p: &BgPaint, breath: f32) {
        let gl = p.gl * breath;
        if gl <= 0.01 {
            return;
        }
        // Five strokes, each standing in for a pair of the nine it replaced: stroking
        // the silhouette is the costliest thing a frame does.
        for j in 1..=5 {
            let k = |i: i32| (1.0 - i as f32 / 10.0).powi(2);
            let a = gl * 0.075 * (k(2 * j - 1) + k(2 * j));
            // SAFETY: a valid geometry, brush and stroke style.
            unsafe {
                rt.DrawGeometry(
                    p.g,
                    self.solid(color(p.rgb, a)),
                    j as f32 * 4.4,
                    &self.round,
                );
            }
        }
    }

    /// Body, tint, rim and hover sheen.
    fn paint_body(&self, rt: &ID2D1RenderTarget, p: &BgPaint) {
        let (g, h, gl) = (p.g, p.h, p.gl);
        // SAFETY: valid geometry and brushes throughout.
        unsafe {
            // Body: near-black with a faint vertical gradient.
            match self.body_brush() {
                Some(b) => {
                    b.SetEndPoint(v(0.0, h.max(1.0)));
                    b.SetOpacity(p.body_alpha);
                    rt.FillGeometry(g, &b, None::<&ID2D1Brush>)
                }
                None => rt.FillGeometry(
                    g,
                    self.solid(color(self.body_colors().0, p.body_alpha)),
                    None::<&ID2D1Brush>,
                ),
            }
            // Tinted glass: the glow's colour pools up from the bottom of the body. It
            // follows the glow spring, so it never needs frames of its own.
            if gl > 0.01 {
                let c = p.rgb;
                let k = if self.light.get() { 0.09 } else { 0.14 };
                if let Some(b) = self.grad(
                    Grad::Tint,
                    [c[0], c[1], c[2], k],
                    &[(0.0, c, 0.0), (1.0, c, k)],
                ) {
                    b.SetStartPoint(v(0.0, h * 0.2));
                    b.SetEndPoint(v(0.0, h));
                    b.SetOpacity(gl.min(1.0));
                    rt.FillGeometry(g, &b, None::<&ID2D1Brush>);
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
                    rt.DrawGeometry(g, &b, 1.0, &self.round);
                }
                None => rt.DrawGeometry(g, self.solid(color(fg, 0.07)), 1.0, &self.round),
            }
            if p.hover > 0.01 {
                rt.FillGeometry(
                    g,
                    self.solid(color(self.fg(), 0.025 * p.hover)),
                    None::<&ID2D1Brush>,
                );
            }
        }
    }

    /// Give the background bitmaps back (the pill has been still for a while, or the device
    /// that made them is gone).
    pub fn drop_background(&self) {
        *self.bg.borrow_mut() = BgCache::default();
    }

    /// Whether the last frame took its background from the bitmaps.
    #[cfg(test)]
    pub(super) fn background_cached(&self) -> bool {
        self.bg.borrow().layers.is_some()
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
                self.digits(
                    &clock,
                    &self.f.btn,
                    x,
                    cy - 8.0,
                    bw,
                    16.0,
                    self.cc(col, 1.0),
                    None,
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
        self.pad_r.set(pad_r);
        self.hero_right
            .set(w - 14.0 - pad_r - if m.visible_count() > 1 { 28.0 } else { 0.0 });
        // Every scene still visible, back to front (`Anim::shown`). The ones being left are
        // clipped to the pill as it changes size and are not interactive: no hover, and their
        // hit areas are dropped.
        let mut c = 1.0;
        for s in an.shown().iter() {
            if s.live {
                c = s.alpha;
                self.ca.set(s.alpha * fit);
                unsafe {
                    self.rt
                        .SetTransform(&mat(self.scale, self.ox.get() + s.dx, s.dy))
                };
                if s.alpha < 1.0 {
                    self.stagger.set(Some((s.alpha, fit, s.dx, s.dy)));
                }
                self.steady.set(s.alpha >= 1.0);
                self.focus_live.set(true);
                self.content(fr, s.scene, w, h, pad_r);
                self.focus_live.set(false);
                self.stagger.set(None);
                self.steady.set(false);
                continue;
            }
            self.ca.set(s.alpha * fit);
            let mouse = self.mouse.take();
            let n = self.hits.borrow().len();
            unsafe {
                self.rt
                    .SetTransform(&mat(self.scale, self.ox.get() + s.dx, s.dy));
                self.rt
                    .PushAxisAlignedClip(&rect(0.0, 0.0, w, h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
            // dt = 0: progress smoothing already advances once this frame, in the live scene.
            let old = Frame { dt: 0.0, ..*fr };
            self.content(&old, s.scene, w, h, pad_r);
            unsafe { self.rt.PopAxisAlignedClip() };
            self.hits.borrow_mut().truncate(n);
            self.mouse.set(mouse);
        }
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
                Scene::ExpDrop | Scene::ExpChip | Scene::ExpTimer | Scene::ExpTimerDone
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
            Scene::ExpAgents => self.exp_agents(fr, w, h),
            Scene::ExpNudge => self.exp_nudge(fr, w, h),
            Scene::ExpNotes => self.exp_notes(fr, w, h),
            Scene::ExpTodo => self.exp_todo(fr, w, h),
            Scene::ExpWeather => self.exp_weather(fr, w, h),
            Scene::ExpAsk => self.exp_ask(fr, w, h),
            Scene::ExpAi => self.exp_ai(fr, w, h),
            Scene::ExpScript(i) => self.exp_script(fr, i, w, h),
            Scene::ExpDrop => self.exp_drop(w, h),
            Scene::ExpChip => self.exp_chip(fr, w, h),
            Scene::ExpTimer => self.exp_timer(fr, w, h),
            Scene::ExpTimerDone => self.exp_timer_done(fr, w, h),
            Scene::ExpCharge => self.exp_charge(fr, w, h),
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

    /// The equaliser's spot in compact media (pill height 32) and in the media pane.
    pub(super) fn eq_ends(&self, w: f32) -> ([f32; 4], [f32; 4]) {
        let right_c = w - 14.0 - self.pad_r.get();
        (
            [right_c - 16.0, 16.0 - 7.0, 14.0, 0.0],
            [w - 16.0 - 18.0, 18.0, 14.0, 0.0],
        )
    }

    /// The task's right-hand value (percent, time) in the compact pill and in the first row of
    /// the list: right-aligned, so what matters is where each box ends.
    pub(super) fn value_ends(&self, w: f32, t: &TaskView) -> ([f32; 4], [f32; 4]) {
        let close_w = if t.running() { 0.0 } else { 24.0 };
        let right_c = self.hero_right.get();
        (
            [right_c - 62.0, 16.0 - 10.0, 62.0, 17.0],
            [w - 16.0 - close_w - 74.0, 10.0 + 8.0, 74.0, 18.0],
        )
    }

    pub(super) fn hero_end(&self, saved: Option<(f32, Matrix3x2)>) {
        if let Some((a, m)) = saved {
            self.ca.set(a);
            unsafe { self.rt.SetTransform(&m) };
        }
    }
}
