//! Brushes, shapes, text, buttons and icons: the drawing primitives the scenes are made of.

use super::*;

impl Renderer {
    // ---------------------------------------------------------------- helpers

    /// A vertical gradient brush from cache `slot`, rebuilt only when `key` changes. Its end
    /// points and opacity are the caller's to set per frame.
    pub(super) fn grad(
        &self,
        slot: Grad,
        key: [f32; 4],
        stops: &[(f32, [f32; 3], f32)],
    ) -> Option<ID2D1LinearGradientBrush> {
        let mut g = self.grads.borrow_mut();
        let cell = &mut g[slot as usize];
        if let Some((k, b)) = cell.as_ref() {
            if *k == key {
                return Some(b.clone());
            }
        }
        let stops: Vec<_> = stops
            .iter()
            .map(|&(position, c, a)| D2D1_GRADIENT_STOP {
                position,
                color: color(c, a),
            })
            .collect();
        let b = self.linear_brush(&stops)?;
        *cell = Some((key, b.clone()));
        Some(b)
    }

    /// Pill body gradient.
    pub(super) fn body_brush(&self) -> Option<ID2D1LinearGradientBrush> {
        let (top, bottom) = self.body_colors();
        let key = [
            self.light.get() as u8 as f32,
            self.skin.get() as u8 as f32,
            0.0,
            0.0,
        ];
        self.grad(Grad::Body, key, &[(0.0, top, 1.0), (1.0, bottom, 1.0)])
    }

    /// Text and line colour: white on the dark pill, near-black on the light one.
    pub(super) fn fg(&self) -> [f32; 3] {
        if self.light.get() {
            [0.07, 0.07, 0.09]
        } else {
            WHITE
        }
    }

    /// The opposite of `fg`: ink for things drawn on top of an `fg`-coloured shape.
    pub(super) fn ink(&self) -> [f32; 3] {
        if self.light.get() {
            [0.97, 0.97, 0.98]
        } else {
            [0.04, 0.04, 0.05]
        }
    }

    /// Pill body gradient (top, bottom).
    pub(super) fn body_colors(&self) -> ([f32; 3], [f32; 3]) {
        self.skin.get().body(self.light.get())
    }

    /// Shimmer for indeterminate progress; the content fade is applied through the brush
    /// opacity and the position through its end points.
    pub(super) fn shimmer_brush(&self, col: [f32; 3]) -> Option<ID2D1LinearGradientBrush> {
        let key = [col[0], col[1], col[2], 0.0];
        self.grad(
            Grad::Shimmer,
            key,
            &[(0.0, col, 0.0), (0.5, col, 1.0), (1.0, col, 0.0)],
        )
    }

    pub(super) fn linear_brush(
        &self,
        stops: &[D2D1_GRADIENT_STOP],
    ) -> Option<ID2D1LinearGradientBrush> {
        unsafe {
            let c = self
                .rt
                .CreateGradientStopCollection(stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .ok()?;
            self.rt
                .CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: v(0.0, 0.0),
                        endPoint: v(0.0, 1.0),
                    },
                    None,
                    &c,
                )
                .ok()
        }
    }

    pub(super) fn solid(&self, c: D2D1_COLOR_F) -> &ID2D1SolidColorBrush {
        unsafe { self.br.SetColor(&c) };
        &self.br
    }

    /// Colour with the content fade applied.
    /// On the light pill, saturated accents are drawn darker so their text stays readable;
    /// greys and the fg / ink colours are left alone.
    pub(super) fn cc(&self, c: [f32; 3], a: f32) -> D2D1_COLOR_F {
        let hi = c.iter().fold(0.0f32, |m, &x| m.max(x));
        let lo = c.iter().fold(1.0f32, |m, &x| m.min(x));
        let c = if self.light.get() && hi - lo > 0.25 {
            c.map(|x| x * 0.78)
        } else {
            c
        };
        color(c, a * self.ca.get())
    }

    pub(super) fn fill_rr(&self, x: f32, y: f32, w: f32, h: f32, r: f32, c: D2D1_COLOR_F) {
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(x, y, w, h),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.FillRoundedRectangle(&rr, self.solid(c)) };
    }

    pub(super) fn stroke_rr(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        c: D2D1_COLOR_F,
        width: f32,
        dashed: bool,
    ) {
        let rr = D2D1_ROUNDED_RECT {
            rect: rect(x, y, w, h),
            radiusX: r,
            radiusY: r,
        };
        unsafe {
            self.rt.DrawRoundedRectangle(
                &rr,
                self.solid(c),
                width,
                if dashed { &self.dashed } else { &self.round },
            )
        };
    }

    pub(super) fn circle(&self, cx: f32, cy: f32, r: f32, c: D2D1_COLOR_F) {
        let e = D2D1_ELLIPSE {
            point: v(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.FillEllipse(&e, self.solid(c)) };
    }

    /// A Segoe Fluent Icons glyph centred on (cx, cy) in a square box of side `s`.
    pub(super) fn glyph(&self, cp: char, cx: f32, cy: f32, s: f32, c: D2D1_COLOR_F) {
        let mut b = [0u8; 4];
        self.text(
            cp.encode_utf8(&mut b),
            &self.f.icon,
            cx - s / 2.0,
            cy - s / 2.0,
            s,
            s,
            c,
        );
    }

    pub(super) fn ring(&self, cx: f32, cy: f32, r: f32, c: D2D1_COLOR_F, width: f32) {
        let e = D2D1_ELLIPSE {
            point: v(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.DrawEllipse(&e, self.solid(c), width, &self.round) };
    }

    pub(super) fn line(&self, a: (f32, f32), b: (f32, f32), c: D2D1_COLOR_F, width: f32) {
        unsafe {
            self.rt
                .DrawLine(v(a.0, a.1), v(b.0, b.1), self.solid(c), width, &self.round)
        };
    }

    pub(super) fn poly(&self, pts: &[(f32, f32)], c: D2D1_COLOR_F) {
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(s) = g.Open() else { return };
            s.BeginFigure(v(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_FILLED);
            for p in &pts[1..] {
                s.AddLine(v(p.0, p.1));
            }
            s.EndFigure(D2D1_FIGURE_END_CLOSED);
            let _ = s.Close();
            self.rt.FillGeometry(&g, self.solid(c), None::<&ID2D1Brush>);
        }
    }

    pub(super) fn text(
        &self,
        s: &str,
        f: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: D2D1_COLOR_F,
    ) {
        if w <= 1.0 {
            return;
        }
        let mut wide = self.wide.borrow_mut();
        wide.clear();
        wide.extend(s.encode_utf16());
        unsafe {
            self.rt.DrawText(
                &wide,
                f,
                &rect(x, y, w, h),
                self.solid(c),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    /// Text that doesn't fit fades out at the end; while `hov`, it instead scrolls to show
    /// the rest: a pause, a glide to the end, a pause, then back to the start.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn label(
        &self,
        s: &str,
        f: &IDWriteTextFormat,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        c: D2D1_COLOR_F,
        hov: bool,
    ) {
        let layout = (|| {
            let wide: Vec<u16> = s.encode_utf16().collect();
            let l = unsafe { self.dw.CreateTextLayout(&wide, f, 4096.0, h).ok()? };
            let mut m = DWRITE_TEXT_METRICS::default();
            unsafe { l.GetMetrics(&mut m).ok()? };
            (m.widthIncludingTrailingWhitespace > w)
                .then_some((l, m.widthIncludingTrailingWhitespace))
        })();
        let Some((l, tw)) = layout else {
            return self.text(s, f, x, y, w, h, c);
        };
        if !hov {
            // Too long and not hovered: it fades out at the end rather than being cut.
            const SLICES: usize = 14;
            let fw = (w * 0.4).min(26.0);
            let sw = fw / SLICES as f32;
            unsafe {
                let _ = l.SetMaxWidth(tw + 1.0);
                let _ = l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
                self.rt
                    .PushAxisAlignedClip(&rect(x, y, w - fw, h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                self.rt
                    .DrawTextLayout(v(x, y), &l, self.solid(c), D2D1_DRAW_TEXT_OPTIONS_NONE);
                self.rt.PopAxisAlignedClip();
                for i in 0..SLICES {
                    let k = 1.0 - (i as f32 + 0.5) / SLICES as f32;
                    let sx = x + w - fw + i as f32 * sw;
                    self.rt.PushAxisAlignedClip(
                        &rect(sx, y, sw + 0.3, h),
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                    let mut cc = c;
                    cc.a *= k * k;
                    self.rt.DrawTextLayout(
                        v(x, y),
                        &l,
                        self.solid(cc),
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                    self.rt.PopAxisAlignedClip();
                }
            }
            return;
        }
        // Restart from the beginning whenever a different label is hovered.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut hs);
            hs.finish()
        };
        let t = self.t.get();
        let t0 = match self.marquee.get() {
            Some((k, t0)) if k == key => t0,
            _ => {
                self.marquee.set(Some((key, t)));
                t
            }
        };
        let travel = tw - w + 8.0;
        let glide = travel / 38.0;
        let (rest, cycle) = (1.1, 2.2 + glide);
        let ph = (t - t0) % (cycle + 0.9);
        let k = ((ph - rest) / glide).clamp(0.0, 1.0);
        let ease = k * k * (3.0 - 2.0 * k);
        self.scrolling.set(true);
        unsafe {
            let _ = l.SetMaxWidth(tw + 1.0);
            let _ = l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            self.rt
                .PushAxisAlignedClip(&rect(x, y, w, h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            self.rt.DrawTextLayout(
                v(x - travel * ease, y),
                &l,
                self.solid(c),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
            self.rt.PopAxisAlignedClip();
        }
    }

    /// Register a clickable region (pill-relative); true when hovered.
    pub(super) fn hit(&self, x: f32, y: f32, w: f32, h: f32, action: Action) -> bool {
        let (ox, oy) = (self.ox.get(), self.oy.get());
        let index = self.hits.borrow().len();
        self.hits.borrow_mut().push(Hit {
            rect: (x + ox, y + oy, x + ox + w, y + oy + h),
            action,
        });
        if self.focus_live.get() && self.focus.get() == Some(index) {
            self.stroke_rr(
                x - 2.0,
                y - 2.0,
                w + 4.0,
                h + 4.0,
                (h / 2.0).min(10.0) + 2.0,
                color(self.accent.get(), 0.95),
                1.6,
                false,
            );
        }
        self.over(x, y, w, h)
    }

    /// The pointer is over this pill-relative box (nothing is registered).
    pub(super) fn over(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let (ox, oy) = (self.ox.get(), self.oy.get());
        self.mouse.get().is_some_and(|(mx, my)| {
            mx >= x + ox && mx <= x + ox + w && my >= y + oy && my <= y + oy + h
        })
    }

    /// Register a pressable region: (hovered, held down, the box shrunk 6 % while held).
    pub(super) fn press(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        action: Action,
    ) -> (bool, bool, [f32; 4]) {
        let hov = self.hit(x, y, w, h, action);
        let down = hov && self.pressed.get();
        let k = if down { 0.03 } else { 0.0 };
        (
            hov,
            down,
            [
                x + w * k,
                y + h * k,
                w * (1.0 - 2.0 * k),
                h * (1.0 - 2.0 * k),
            ],
        )
    }

    pub(super) fn button(
        &self,
        label: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        accent: [f32; 3],
        action: Action,
    ) {
        // Pressed in: a touch smaller and brighter while the button is held.
        let (hov, down, [x, y, w, h]) = self.press(x, y, w, h, action);
        let a = if down {
            0.36
        } else if hov {
            0.26
        } else {
            0.13
        };
        self.fill_rr(x, y, w, h, h / 2.0, self.cc(accent, a));
        self.text(
            label,
            &self.f.btn,
            x,
            y,
            w,
            h,
            self.cc(self.fg(), if hov { 1.0 } else { 0.86 }),
        );
    }

    /// A filled call-to-action: the accent itself with dark ink on it.
    pub(super) fn primary(
        &self,
        label: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        accent: [f32; 3],
        action: Action,
    ) {
        let (hov, _, [x, y, w, h]) = self.press(x, y, w, h, action);
        self.fill_rr(
            x,
            y,
            w,
            h,
            h / 2.0,
            self.cc(accent, if hov { 1.0 } else { 0.86 }),
        );
        self.text(
            label,
            &self.f.btn,
            x,
            y,
            w,
            h,
            self.cc([0.03, 0.03, 0.05], 0.92),
        );
    }

    /// A round icon button; `draw` paints the icon centred on (cx, cy) in the given colour.
    pub(super) fn round_btn(
        &self,
        cx: f32,
        cy: f32,
        r: f32,
        accent: [f32; 3],
        action: Action,
        draw: impl Fn(f32, f32, D2D1_COLOR_F),
    ) {
        let hov = self.hit(cx - r, cy - r, r * 2.0, r * 2.0, action);
        let down = hov && self.pressed.get();
        let rr = if down { r * 0.94 } else { r };
        let a = if down {
            0.34
        } else if hov {
            0.24
        } else {
            0.12
        };
        self.circle(cx, cy, rr, self.cc(accent, a));
        draw(cx, cy, self.cc(self.fg(), if hov { 1.0 } else { 0.88 }));
    }

    /// A small Segoe Fluent Icons glyph (tab strip) centred on (cx, cy).
    pub(super) fn glyph_s(&self, cp: char, cx: f32, cy: f32, c: D2D1_COLOR_F) {
        let mut b = [0u8; 4];
        let f = &self.f.icon_s;
        self.text(cp.encode_utf8(&mut b), f, cx - 9.0, cy - 9.0, 18.0, 18.0, c);
    }

    /// A larger Segoe Fluent Icons glyph centred on (cx, cy).
    pub(super) fn glyph_l(&self, cp: char, cx: f32, cy: f32, c: D2D1_COLOR_F) {
        let mut b = [0u8; 4];
        self.text(
            cp.encode_utf8(&mut b),
            &self.f.icon_l,
            cx - 16.0,
            cy - 16.0,
            32.0,
            32.0,
            c,
        );
    }

    /// Clockwise arc from 12 o'clock covering `frac` of the circle.
    pub(super) fn arc(&self, cx: f32, cy: f32, r: f32, frac: f32, c: D2D1_COLOR_F, width: f32) {
        if frac <= 0.002 {
            return;
        }
        if frac >= 0.998 {
            self.ring(cx, cy, r, c, width);
            return;
        }
        let a = frac * std::f32::consts::TAU;
        unsafe {
            let Ok(g) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(sk) = g.Open() else { return };
            sk.BeginFigure(v(cx, cy - r), D2D1_FIGURE_BEGIN_HOLLOW);
            sk.AddArc(&D2D1_ARC_SEGMENT {
                point: v(cx + r * a.sin(), cy - r * a.cos()),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sk.EndFigure(D2D1_FIGURE_END_OPEN);
            let _ = sk.Close();
            self.rt.DrawGeometry(&g, self.solid(c), width, &self.round);
        }
    }

    /// Two rounded bars.
    pub(super) fn pause_icon(&self, cx: f32, cy: f32, s: f32, c: D2D1_COLOR_F) {
        let bw = s * 0.3;
        self.fill_rr(cx - s * 0.38, cy - s / 2.0, bw, s, bw / 2.5, c);
        self.fill_rr(cx + s * 0.08, cy - s / 2.0, bw, s, bw / 2.5, c);
    }

    /// A play triangle, nudged right so it looks centred.
    pub(super) fn play_icon(&self, cx: f32, cy: f32, s: f32, c: D2D1_COLOR_F) {
        let h = s / 2.0;
        self.poly(
            &[(cx - h * 0.7, cy - h), (cx - h * 0.7, cy + h), (cx + h, cy)],
            c,
        );
    }

    /// Previous (dir -1) or next (dir +1) track: a triangle against a bar.
    pub(super) fn skip_icon(&self, cx: f32, cy: f32, dir: f32, c: D2D1_COLOR_F) {
        let h = 6.5;
        self.poly(
            &[
                (cx - 5.0 * dir, cy - h),
                (cx - 5.0 * dir, cy + h),
                (cx + 4.0 * dir, cy),
            ],
            c,
        );
        let bx = if dir > 0.0 { cx + 4.5 } else { cx - 7.0 };
        self.fill_rr(bx, cy - h, 2.5, h * 2.0, 1.2, c);
    }

    /// A lightning bolt, 16 px tall at `k` = 1.
    pub(super) fn bolt(&self, cx: f32, cy: f32, k: f32, c: D2D1_COLOR_F) {
        let pts = [
            (1.5, -8.0),
            (-4.5, 1.0),
            (-0.5, 1.0),
            (-1.5, 8.0),
            (4.5, -1.0),
            (0.5, -1.0),
        ]
        .map(|(x, y)| (cx + x * k, cy + y * k));
        self.poly(&pts, c);
    }

    /// An x of half-size `d` centred on (cx, cy).
    pub(super) fn cross(&self, cx: f32, cy: f32, d: f32, c: D2D1_COLOR_F, width: f32) {
        self.line((cx - d, cy - d), (cx + d, cy + d), c, width);
        self.line((cx - d, cy + d), (cx + d, cy - d), c, width);
    }

    /// Rotate what is drawn next by `deg` around (cx, cy); returns the transform to restore.
    pub(super) fn rotate(&self, deg: f32, cx: f32, cy: f32) -> Matrix3x2 {
        let mut cur = Matrix3x2::default();
        unsafe { self.rt.GetTransform(&mut cur) };
        let (s, c) = deg.to_radians().sin_cos();
        let r = Matrix3x2 {
            M11: c,
            M12: s,
            M21: -s,
            M22: c,
            M31: cx - c * cx + s * cy,
            M32: cy - s * cx - c * cy,
        };
        let m = Matrix3x2 {
            M11: r.M11 * cur.M11 + r.M12 * cur.M21,
            M12: r.M11 * cur.M12 + r.M12 * cur.M22,
            M21: r.M21 * cur.M11 + r.M22 * cur.M21,
            M22: r.M21 * cur.M12 + r.M22 * cur.M22,
            M31: r.M31 * cur.M11 + r.M32 * cur.M21 + cur.M31,
            M32: r.M31 * cur.M12 + r.M32 * cur.M22 + cur.M32,
        };
        unsafe { self.rt.SetTransform(&m) };
        cur
    }

    /// A quick-control tile: icon, name and state. `on` tints it with that accent.
    pub(super) fn tile(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        icon: char,
        label: &str,
        sub: &str,
        on: Option<[f32; 3]>,
        action: Action,
    ) {
        let (hov, _, [x, y, w, h]) = self.press(x, y, w, h, action);
        match on {
            Some(c) => {
                self.fill_rr(x, y, w, h, 14.0, self.cc(c, if hov { 0.30 } else { 0.22 }));
                self.stroke_rr(
                    x + 0.5,
                    y + 0.5,
                    w - 1.0,
                    h - 1.0,
                    13.5,
                    self.cc(c, 0.35),
                    1.0,
                    false,
                );
            }
            None => self.fill_rr(
                x,
                y,
                w,
                h,
                14.0,
                self.cc(self.fg(), if hov { 0.11 } else { 0.06 }),
            ),
        }
        let ic = on.unwrap_or(self.fg());
        self.glyph_l(icon, x + w / 2.0, y + 20.0, self.cc(ic, 0.95));
        self.text(
            label,
            &self.f.btn,
            x,
            y + 33.0,
            w,
            14.0,
            self.cc(self.fg(), 0.95),
        );
        let sc = if on.is_some() { ic } else { GRAY };
        self.text(
            sub,
            &self.f.small_c,
            x,
            y + 46.0,
            w,
            13.0,
            self.cc(sc, 0.95),
        );
    }
}
