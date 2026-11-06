//! The panel tab strip and the Shelf pane.

use super::*;

impl Renderer {
    // ------------------------------------------------------- tabs / ports / shelf

    /// The pane switcher under a pane with neighbours: one icon per pane, the current one a
    /// pill with its name. Drawn once, as chrome that stays put while the pages slide. While
    /// paging, the old tab shrinks as the new one grows (the content spring drives both), so
    /// the highlight travels without any state of its own.
    pub(super) fn tab_strip(&self, fr: &Frame, w: f32, h: f32, fit: f32) {
        let (m, an) = (fr.model, fr.anim);
        if !m.tabs() {
            return;
        }
        let live = Panel::of(an.scene).is_some();
        let gone = an.prev != an.scene && Panel::of(an.prev).is_some();
        let (c, o) = (
            an.content.pos.clamp(0.0, 1.0) as f32,
            an.out.pos.clamp(0.0, 1.0) as f32,
        );
        // Fade with the pane when the strip arrives or leaves; stay solid between two panes.
        let a = match (live, gone) {
            (true, true) => 1.0,
            (true, false) => c,
            (false, true) => o,
            (false, false) => return,
        };
        // Leaving (collapsing to a compact scene): visible, but not clickable.
        let (mouse, n) = (self.mouse.get(), self.hits.borrow().len());
        if !live {
            self.mouse.set(None);
        }
        unsafe { self.rt.SetTransform(&mat(self.scale, self.ox.get(), 0.0)) };
        self.ca.set(a * fit);
        let ps = m.panels();
        // The pane on screen, not the selection: the timer setup card is no pane at all.
        let on = |s: Scene| Panel::of(s).filter(|p| ps.contains(p));
        let cur = on(an.scene);
        let prev = on(an.prev).filter(|_| live && gone);
        let weight = |p: Panel| match prev {
            Some(q) if Some(p) == cur && Some(q) != cur => c,
            Some(q) if p == q && Some(q) != cur => 1.0 - c,
            _ => (Some(p) == cur) as u8 as f32,
        };
        let icon = 26.0;
        let label = |p: Panel| text_w(p.name()) + 10.0;
        let total: f32 = ps.iter().map(|&p| icon + label(p) * weight(p)).sum();
        let mut x = (w - total) / 2.0;
        let (cy, th) = (h - TAB_H / 2.0 - 2.0, 20.0);
        let fg = self.fg();
        let mut tip = None;
        for &p in &ps {
            let k = weight(p);
            let tw = icon + label(p) * k;
            let hov = self.hit(x, cy - th / 2.0, tw, th, Action::SelectPanel(p));
            if k > 0.01 {
                let acc = self.accent.get();
                self.fill_rr(x, cy - th / 2.0, tw, th, th / 2.0, self.cc(acc, 0.24 * k));
            } else if hov {
                self.fill_rr(x, cy - th / 2.0, tw, th, th / 2.0, self.cc(fg, 0.06));
                tip = Some((p, x + tw / 2.0));
            }
            let ia = 0.42 + 0.53 * k.max(hov as u8 as f32 * 0.6);
            self.glyph_s(panel_glyph(p), x + icon / 2.0, cy, self.cc(fg, ia));
            if k > 0.01 {
                unsafe {
                    self.rt.PushAxisAlignedClip(
                        &rect(x, cy - th / 2.0, tw - 6.0, th),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                }
                self.text(
                    p.name(),
                    &self.f.small_l,
                    x + icon - 3.0,
                    cy - 8.0,
                    label(p) + 4.0,
                    16.0,
                    self.cc(fg, 0.95 * k),
                );
                unsafe { self.rt.PopAxisAlignedClip() };
            }
            x += tw;
        }
        if m.pin_button {
            let (px, on) = (w - 26.0, m.pinned);
            let hov = self.hit(px - 12.0, cy - 12.0, 24.0, 24.0, Action::Pin);
            if on {
                self.fill_rr(
                    px - 11.0,
                    cy - 11.0,
                    22.0,
                    22.0,
                    11.0,
                    self.cc(self.accent.get(), 0.24),
                );
            } else if hov {
                self.fill_rr(px - 11.0, cy - 11.0, 22.0, 22.0, 11.0, self.cc(fg, 0.06));
            }
            let (glyph, a) = if on {
                ('\u{E840}', 0.95)
            } else {
                ('\u{E718}', if hov { 0.8 } else { 0.42 })
            };
            self.glyph_s(glyph, px, cy, self.cc(fg, a));
        }
        // The hovered tab's name, in a small label just above it.
        if let Some((p, tx)) = tip {
            let lw = text_w(p.name()) + 16.0;
            let (lx, ly) = (
                (tx - lw / 2.0).clamp(6.0, w - lw - 6.0),
                cy - th / 2.0 - 24.0,
            );
            let (_, bottom) = self.body_colors();
            self.fill_rr(lx, ly, lw, 20.0, 10.0, self.cc(bottom, 0.96));
            self.stroke_rr(lx, ly, lw, 20.0, 10.0, self.cc(fg, 0.14), 1.0, false);
            self.text(p.name(), &self.f.btn, lx, ly, lw, 20.0, self.cc(fg, 0.95));
        }
        if !live {
            self.hits.borrow_mut().truncate(n);
            self.mouse.set(mouse);
        }
    }

    pub(super) fn exp_shelf(&self, fr: &Frame, w: f32, _h: f32) {
        let m = fr.model;
        let thumbs = self.thumbs.borrow();
        // The row of tiles is centred (70 px a tile, 60 of it the tile itself).
        let (off, page) = m.shelf_page();
        let x0 = (w - (page.len() as f32 * 70.0 - 10.0)) / 2.0;
        // More than fit: chevrons in the margins scroll the row (so does the wheel).
        if m.shelf.len() > page.len() {
            let more = [
                (off > 0, -1, '\u{E76B}', 2.0),
                (off + page.len() < m.shelf.len(), 1, '\u{E76C}', w - 18.0),
            ];
            for (on, by, icon, x) in more {
                if on {
                    let hov = self.hit(x, 14.0, 16.0, 60.0, Action::ShelfScroll(by));
                    let a = if hov { 0.95 } else { 0.5 };
                    self.glyph_s(icon, x + 8.0, 44.0, self.cc(self.fg(), a));
                }
            }
        }
        for (i, it) in page.iter().enumerate() {
            let (x, y, s) = (x0 + i as f32 * 70.0, 14.0, 60.0);
            let sel = m.shelf_sel == Some(it.id);
            let hov = self.hit(x, y, s, s, Action::ShelfTile(it.id));
            self.fill_rr(
                x,
                y,
                s,
                s,
                11.0,
                self.cc(
                    if sel { PURPLE } else { self.fg() },
                    if sel {
                        0.22
                    } else if hov {
                        0.11
                    } else {
                        0.07
                    },
                ),
            );
            if sel {
                self.stroke_rr(
                    x + 0.5,
                    y + 0.5,
                    s - 1.0,
                    s - 1.0,
                    11.0,
                    self.cc(PURPLE, 0.8),
                    1.2,
                    false,
                );
            }
            if let Some(b) = thumbs.get(&it.id) {
                self.paint_bitmap(b, x + 6.0, y + 6.0, 48.0, 7.0);
            } else if it.is_dir {
                self.fill_rr(x + 14.0, y + 20.0, 32.0, 22.0, 4.0, self.cc(AMBER, 0.85));
                self.fill_rr(x + 14.0, y + 16.0, 14.0, 8.0, 3.0, self.cc(AMBER, 0.85));
            } else {
                self.fill_rr(
                    x + 18.0,
                    y + 12.0,
                    24.0,
                    32.0,
                    4.0,
                    self.cc(self.fg(), 0.82),
                );
                for k in 0..3 {
                    self.fill_rr(
                        x + 22.0,
                        y + 19.0 + k as f32 * 7.0,
                        16.0,
                        2.0,
                        1.0,
                        self.cc([0.2; 3], 0.8),
                    );
                }
            }
            self.label(
                &it.name,
                &self.f.small_c,
                x - 2.0,
                y + s + 3.0,
                s + 4.0,
                14.0,
                self.cc(self.fg(), if sel || hov { 0.95 } else { 0.7 }),
                hov,
            );
            if hov || sel {
                let h2 = self.hit(
                    x + s - 14.0,
                    y - 5.0,
                    18.0,
                    18.0,
                    Action::RemoveShelf(it.id),
                );
                self.circle(
                    x + s - 5.0,
                    y + 4.0,
                    7.5,
                    self.cc(self.body_colors().1, 1.0),
                );
                self.ring(x + s - 5.0, y + 4.0, 7.5, self.cc(self.fg(), 0.12), 1.0);
                let a = if h2 { 1.0 } else { 0.7 };
                self.cross(x + s - 5.0, y + 4.0, 3.0, self.cc(self.fg(), a), 1.4);
            }
        }
        if let Some(it) = m
            .shelf_sel
            .and_then(|id| m.shelf.iter().find(|i| i.id == id))
        {
            let chips = crate::transforms::chips_for(&it.path);
            let labels: Vec<&str> = chips.iter().map(|c| c.0).collect();
            // Chip width from the label length; each line centred, a new line when full.
            for (row, line) in crate::transforms::chip_lines(&labels, w - 8.0)
                .iter()
                .enumerate()
            {
                let used: f32 = line
                    .iter()
                    .map(|&i| crate::transforms::chip_width(labels[i]))
                    .sum::<f32>()
                    + 6.0 * (line.len() - 1) as f32;
                let mut x = (w - used) / 2.0;
                for &i in line {
                    let (label, op) = &chips[i];
                    let cw = crate::transforms::chip_width(label);
                    self.button(
                        label,
                        x,
                        98.0 + row as f32 * 32.0,
                        cw,
                        26.0,
                        PURPLE,
                        Action::ShelfOp(it.id, *op),
                    );
                    x += cw + 6.0;
                }
            }
        } else {
            self.text(
                "Drag a tile out to drop it anywhere",
                &self.f.small_c,
                0.0,
                96.0,
                w,
                16.0,
                self.cc(GRAY, 0.7),
            );
        }
    }
}
