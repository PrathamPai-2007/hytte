//! The Notes and To-do panes, and the one-line text field they share with Weather.

use super::*;
use crate::jot::{Jot, Kind};
use crate::ui_state::{Field, JOT_ROWS};

/// Left edge of a field's text, from the field's own left edge.
const FIELD_PAD: f32 = 12.0;
/// Height of a list row.
pub(super) const ROW_H: f32 = 30.0;

fn utf16_of(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())].encode_utf16().count() as u32
}

fn byte_of(text: &str, utf16: u32) -> usize {
    let mut n = 0u32;
    for (i, c) in text.char_indices() {
        if n >= utf16 {
            return i;
        }
        n += c.len_utf16() as u32;
    }
    text.len()
}

impl Renderer {
    fn layout_of(&self, text: &str) -> Option<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.dw
                .CreateTextLayout(&wide, &self.f.body, 4000.0, 20.0)
                .ok()
        }
    }

    /// Width of `text` in the body font, in logical px.
    pub(super) fn text_width(&self, text: &str) -> f32 {
        self.caret_x(text, text.len())
    }

    /// x of a caret at `byte`, measured from the text's left edge.
    pub(super) fn caret_x(&self, text: &str, byte: usize) -> f32 {
        let Some(l) = self.layout_of(text) else {
            return text.chars().count() as f32 * 6.5;
        };
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = l.HitTestTextPosition(utf16_of(text, byte), false, &mut x, &mut y, &mut m);
        }
        x
    }

    /// How far a field's text is scrolled so the caret stays in view.
    fn field_scroll(&self, text: &str, caret: usize, inner_w: f32) -> f32 {
        (self.caret_x(text, caret) - (inner_w - 6.0)).max(0.0)
    }

    /// The byte under a click at `x` (from the field's left edge), for placing the caret.
    pub fn field_byte_at(&self, text: &str, caret: usize, hit_w: f32, x: f32) -> usize {
        let inner_w = hit_w - FIELD_PAD - 8.0;
        let scroll = self.field_scroll(text, caret, inner_w);
        let Some(l) = self.layout_of(text) else {
            return text.len();
        };
        let (mut trailing, mut inside) = (windows::core::BOOL(0), windows::core::BOOL(0));
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = l.HitTestPoint(
                x - FIELD_PAD + scroll,
                5.0,
                &mut trailing,
                &mut inside,
                &mut m,
            );
        }
        let pos = m.textPosition + trailing.as_bool() as u32 * m.length.max(1);
        byte_of(text, pos)
    }

    /// A one-line text field: hint when empty, selection, caret, sideways scroll. `right` keeps
    /// room at its right for something drawn over it (the 24 h / Keep switch).
    pub(super) fn text_field(
        &self,
        fr: &Frame,
        f: Field,
        (x, y, w, h): (f32, f32, f32, f32),
        hint: &str,
        right: f32,
    ) {
        let m = fr.model;
        let ed = m.edit_ref(f);
        let focused = m.typing == Some(f);
        let hov = self.hit(x, y, w - right, h, Action::Field(f));
        let fg = self.fg();
        self.fill_rr(
            x,
            y,
            w,
            h,
            12.0,
            self.cc(fg, if hov || focused { 0.10 } else { 0.07 }),
        );
        if focused {
            self.stroke_rr(
                x,
                y,
                w,
                h,
                12.0,
                self.cc(self.accent.get(), 0.85),
                1.3,
                false,
            );
        }
        let inner_x = x + FIELD_PAD;
        let inner_w = w - FIELD_PAD - right - 8.0;
        unsafe {
            self.rt.PushAxisAlignedClip(
                &rect(inner_x, y, inner_w, h),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
        }
        if ed.is_empty() {
            self.text(
                hint,
                &self.f.body,
                inner_x,
                y,
                inner_w,
                h,
                self.cc(GRAY, 0.8),
            );
        } else {
            let scroll = self.field_scroll(ed.text(), ed.caret(), inner_w);
            if let Some((a, b)) = ed.selection().filter(|_| focused) {
                let (xa, xb) = (self.caret_x(ed.text(), a), self.caret_x(ed.text(), b));
                self.fill_rr(
                    inner_x + xa - scroll,
                    y + 7.0,
                    xb - xa,
                    h - 14.0,
                    3.0,
                    self.cc(self.accent.get(), 0.35),
                );
            }
            self.text(
                ed.text(),
                &self.f.body,
                inner_x - scroll,
                y,
                4000.0,
                h,
                self.cc(fg, 0.97),
            );
        }
        if focused && fr.caret_on {
            let scroll = if ed.is_empty() {
                0.0
            } else {
                self.field_scroll(ed.text(), ed.caret(), inner_w)
            };
            let cx = inner_x + self.caret_x(ed.text(), ed.caret()) - scroll;
            self.line(
                (cx, y + 8.0),
                (cx, y + h - 8.0),
                self.cc(self.accent.get(), 1.0),
                1.4,
            );
        }
        unsafe { self.rt.PopAxisAlignedClip() };
    }

    pub(super) fn exp_notes(&self, fr: &Frame, w: f32, h: f32) {
        self.jot_pane(fr, w, h, Scene::ExpNotes);
    }

    pub(super) fn exp_todo(&self, fr: &Frame, w: f32, h: f32) {
        self.jot_pane(fr, w, h, Scene::ExpTodo);
    }

    fn jot_pane(&self, fr: &Frame, w: f32, _h: f32, scene: Scene) {
        let m = fr.model;
        let notes = scene == Scene::ExpNotes;
        let fg = self.fg();
        let right = if notes { 88.0 } else { 0.0 };
        self.text_field(
            fr,
            if notes { Field::Note } else { Field::Todo },
            (16.0, 12.0, w - 32.0, 34.0),
            if notes { "Type a note" } else { "Add a to-do" },
            right,
        );
        if notes {
            // 24 h / Keep: what happens to the note when Enter saves it.
            let (sx, sy, sw, sh) = (w - 16.0 - 82.0, 12.0 + 7.0, 78.0, 20.0);
            self.fill_rr(sx, sy, sw, sh, sh / 2.0, self.cc(fg, 0.08));
            for (i, (label, keep)) in [("24 h", false), ("Keep", true)].into_iter().enumerate() {
                let bx = sx + 2.0 + i as f32 * 37.0;
                let on = m.note_keep == keep;
                let hov = self.hit(bx, sy + 2.0, 37.0, sh - 4.0, Action::NoteKeep(keep));
                if on {
                    self.fill_rr(
                        bx,
                        sy + 2.0,
                        37.0,
                        sh - 4.0,
                        (sh - 4.0) / 2.0,
                        self.cc(fg, 0.16),
                    );
                }
                self.text(
                    label,
                    &self.f.btn,
                    bx,
                    sy + 2.0,
                    37.0,
                    sh - 4.0,
                    self.cc(
                        fg,
                        if on {
                            1.0
                        } else if hov {
                            0.8
                        } else {
                            0.5
                        },
                    ),
                );
            }
        }
        let list = m.jot_list(scene);
        let (first, shown) = m.jot_window(scene);
        let wall = crate::timer::now_ms();
        let mut y = 54.0;
        let undo = m
            .jot_undo
            .as_ref()
            .filter(|(j, _)| (j.kind == Kind::Todo) == !notes);
        if list.is_empty() && undo.is_none() {
            self.text(
                if notes {
                    "Notes you type here wait for you."
                } else {
                    "Nothing to do. Add something above."
                },
                &self.f.body,
                16.0,
                y,
                w - 32.0,
                ROW_H,
                self.cc(GRAY, 0.9),
            );
        }
        for j in list.iter().skip(first).take(shown) {
            self.jot_row(w, y, j, wall);
            y += ROW_H;
        }
        if undo.is_some() {
            self.text(
                "Deleted",
                &self.f.body,
                16.0,
                y,
                200.0,
                ROW_H,
                self.cc(GRAY, 1.0),
            );
            self.button(
                "Undo",
                w - 16.0 - 56.0,
                y + 4.0,
                56.0,
                ROW_H - 8.0,
                self.accent.get(),
                Action::UndoJot,
            );
        }
        // A slim scroll mark when there are more rows than fit.
        if list.len() > JOT_ROWS {
            let track = JOT_ROWS as f32 * ROW_H - 8.0;
            let th = (track * JOT_ROWS as f32 / list.len() as f32).max(14.0);
            let ty = 58.0 + (track - th) * first as f32 / (list.len() - JOT_ROWS) as f32;
            self.fill_rr(w - 8.0, ty, 2.5, th, 1.2, self.cc(fg, 0.22));
        }
    }

    fn jot_row(&self, w: f32, y: f32, j: &Jot, wall: u64) {
        let fg = self.fg();
        let hov = self.over(0.0, y, w, ROW_H);
        let cy = y + ROW_H / 2.0;
        if hov {
            self.fill_rr(8.0, y + 1.0, w - 16.0, ROW_H - 2.0, 9.0, self.cc(fg, 0.05));
        }
        let done = j.done_ms.is_some();
        match j.kind {
            Kind::Todo => {
                let hb = self.hit(12.0, y, 26.0, ROW_H, Action::ToggleJot(j.id));
                if done {
                    self.fill_rr(18.0, cy - 7.5, 15.0, 15.0, 5.0, self.cc(GREEN, 1.0));
                    let ink = self.cc([0.03, 0.08, 0.05], 1.0);
                    self.line((21.0, cy), (24.0, cy + 3.0), ink, 1.8);
                    self.line((24.0, cy + 3.0), (30.0, cy - 3.5), ink, 1.8);
                } else {
                    self.stroke_rr(
                        18.5,
                        cy - 7.0,
                        14.0,
                        14.0,
                        4.5,
                        self.cc(fg, if hb { 0.75 } else { 0.4 }),
                        1.4,
                        false,
                    );
                }
            }
            Kind::Note => match j.life_left(wall) {
                // A ring that drains over the note's 24 hours.
                Some(left) => {
                    self.ring(25.5, cy, 6.5, self.cc(fg, 0.12), 2.0);
                    self.arc(25.5, cy, 6.5, left, self.cc(BLUE, 0.95), 2.0);
                }
                None => self.glyph_s('\u{E718}', 25.5, cy, self.cc(fg, 0.45)),
            },
        }
        let right = if hov { 76.0 } else { 40.0 };
        let (tx, tw) = (42.0, w - 42.0 - 16.0 - right);
        self.hit(tx, y, tw, ROW_H, Action::EditJot(j.id));
        let col = if done {
            self.cc(GRAY, 0.85)
        } else {
            self.cc(fg, 0.95)
        };
        self.text(&j.text, &self.f.body, tx, y, tw, ROW_H, col);
        if done {
            let sw = self.text_width(&j.text).min(tw);
            self.line((tx, cy + 0.5), (tx + sw, cy + 0.5), self.cc(GRAY, 0.8), 1.2);
        }
        if hov {
            // Small round buttons: copy, keep (a temporary note), delete.
            let mut bx = w - 16.0 - 12.0;
            let mut icon = |glyph: char, action: Action| {
                let hb = self.hit(bx - 12.0, cy - 12.0, 24.0, 24.0, action);
                if hb {
                    self.circle(bx, cy, 12.0, self.cc(fg, 0.12));
                }
                self.glyph_s(glyph, bx, cy, self.cc(fg, if hb { 1.0 } else { 0.62 }));
                bx -= 26.0;
            };
            icon('\u{E74D}', Action::DeleteJot(j.id));
            if j.kind == Kind::Note && j.expires_ms.is_some() {
                icon('\u{E718}', Action::KeepJot(j.id));
            }
            icon('\u{E8C8}', Action::CopyText(j.text.clone()));
        } else if let Some(end) = j.expires_ms.filter(|_| j.kind == Kind::Note) {
            self.text(
                &crate::jot::left_text(end, wall),
                &self.f.small_r,
                w - 16.0 - 36.0,
                y,
                36.0,
                ROW_H,
                self.cc(GRAY, 0.9),
            );
        }
    }

    /// Placeholder until the forecast is wired up.
    pub(super) fn exp_weather(&self, _fr: &Frame, w: f32, _h: f32) {
        self.text(
            "Weather",
            &self.f.center,
            0.0,
            12.0,
            w,
            34.0,
            self.cc(self.fg(), 0.9),
        );
    }
}
