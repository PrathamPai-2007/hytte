//! Typing in the pill's text fields (notes, to-dos, the weather place).
//!
//! The pill never takes focus by itself. Clicking a field (or the note hotkey) takes it, the
//! same way keyboard mode does: the window that had focus is remembered, and Esc or clicking
//! elsewhere gives it back. Characters arrive as `WM_CHAR`, editing keys as `WM_KEYDOWN`; the
//! text itself lives in `textedit::TextEdit`.

use super::*;
use crate::ui_state::Field;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};

/// Half a caret blink.
const CARET_MS: u32 = 530;

/// The pane a field lives in.
pub(super) fn field_panel(f: Field) -> Panel {
    match f {
        Field::Note => Panel::Notes,
        Field::Todo => Panel::Todo,
        Field::Place => Panel::Weather,
    }
}

impl Ui {
    /// Give `f` keyboard focus: take it from the foreground window (remembered, to give back).
    pub(super) fn begin_typing(&mut self, hwnd: HWND, f: Field) {
        self.model.typing = Some(f);
        if self.kb.is_none() {
            unsafe {
                self.kb = Some(GetForegroundWindow());
                self.kb_focus = None;
                self.hover = true;
                // Allowed: the user just clicked the pill or pressed its hotkey.
                let _ = SetForegroundWindow(hwnd);
            }
        }
        self.reset_caret(hwnd);
    }

    /// Show the caret now and restart its blink.
    pub(super) fn reset_caret(&mut self, hwnd: HWND) {
        self.caret_on = true;
        unsafe {
            SetTimer(Some(hwnd), T_CARET, CARET_MS, None);
        }
        self.kick();
    }

    /// `T_CARET` fired: blink, or settle when nothing is being typed (so frames stop).
    pub(super) fn caret_tick(&mut self, hwnd: HWND) {
        if self.model.typing.is_some() {
            self.caret_on = !self.caret_on;
            unsafe {
                SetTimer(Some(hwnd), T_CARET, CARET_MS, None);
            }
            self.kick();
        } else if !self.caret_on {
            self.caret_on = true;
        }
    }

    /// A click on a field: focus it and put the caret where the pointer is.
    pub(super) fn click_field(&mut self, hwnd: HWND, f: Field) {
        self.begin_typing(hwnd, f);
        let Some(rect) = self
            .hits
            .iter()
            .find(|h| h.action == Action::Field(f))
            .map(|h| h.rect)
        else {
            return;
        };
        let Some((mx, _)) = self.mouse_logical() else {
            return;
        };
        let ed = self.model.edit_ref(f);
        let byte = self
            .rend
            .field_byte_at(ed.text(), ed.caret(), rect.2 - rect.0, mx - rect.0);
        self.model.edit_mut(f).set_caret(byte, false);
    }

    /// The hotkey: open Notes with the cursor ready (or close it again).
    pub(super) fn toggle_note(&mut self, hwnd: HWND) {
        if self.model.typing == Some(Field::Note) {
            self.end_keyboard(true);
            return;
        }
        if !self.model.panels().contains(&Panel::Notes) {
            return;
        }
        self.select_panel(Panel::Notes);
        self.hover = true;
        self.begin_typing(hwnd, Field::Note);
        self.after_model_change(hwnd);
    }

    /// A character typed (UTF-16 unit: an emoji arrives as two).
    pub(super) fn type_char(&mut self, hwnd: HWND, unit: u16) {
        let c = match unit {
            0xD800..=0xDBFF => {
                self.high = Some(unit);
                return;
            }
            0xDC00..=0xDFFF => self
                .high
                .take()
                .and_then(|h| char::decode_utf16([h, unit]).next()?.ok()),
            u => {
                self.high = None;
                char::from_u32(u as u32)
            }
        };
        let Some(c) = c.filter(|c| !c.is_control()) else {
            return;
        };
        let Some(f) = self.model.typing else { return };
        self.model.edit_mut(f).insert(c.encode_utf8(&mut [0; 4]));
        self.edited(hwnd, f);
    }

    /// After the text changed: caret on, redraw, and let the field's owner react.
    fn edited(&mut self, hwnd: HWND, f: Field) {
        self.reset_caret(hwnd);
        if f == Field::Place {
            self.place_typed(hwnd);
        }
        self.after_model_change(hwnd);
    }

    /// An editing key while a field has focus.
    pub(super) fn typing_key(&mut self, hwnd: HWND, vk: u16) {
        let Some(f) = self.model.typing else { return };
        let (ctrl, shift) = unsafe {
            (
                GetKeyState(VK_CONTROL.0 as i32) < 0,
                GetKeyState(VK_SHIFT.0 as i32) < 0,
            )
        };
        let mut changed = true;
        match VIRTUAL_KEY(vk) {
            VK_ESCAPE => {
                self.end_keyboard(true);
                self.after_model_change(hwnd);
                return;
            }
            VK_RETURN => {
                self.enter_field(hwnd, f);
                return;
            }
            VK_UP | VK_DOWN if f == Field::Place => {
                self.model
                    .place_move(if VIRTUAL_KEY(vk) == VK_DOWN { 1 } else { -1 });
            }
            VK_LEFT => self.model.edit_mut(f).left(shift, ctrl),
            VK_RIGHT => self.model.edit_mut(f).right(shift, ctrl),
            VK_HOME => self.model.edit_mut(f).home(shift),
            VK_END => self.model.edit_mut(f).end(shift),
            VK_BACK => self.model.edit_mut(f).backspace(ctrl),
            VK_DELETE => self.model.edit_mut(f).delete(ctrl),
            VIRTUAL_KEY(0x41) if ctrl => self.model.edit_mut(f).select_all(),
            VIRTUAL_KEY(0x43) if ctrl => {
                let s = self.model.edit_ref(f).selected().to_string();
                if !s.is_empty() {
                    set_clipboard(hwnd, &s);
                }
            }
            VIRTUAL_KEY(0x58) if ctrl => {
                let s = self.model.edit_mut(f).cut();
                if !s.is_empty() {
                    set_clipboard(hwnd, &s);
                }
            }
            VIRTUAL_KEY(0x56) if ctrl => {
                if let Some(text) = clipboard_text(hwnd) {
                    self.model.edit_mut(f).insert(&text);
                }
            }
            _ => changed = false,
        }
        if changed {
            self.edited(hwnd, f);
        }
    }

    /// Enter in a field.
    fn enter_field(&mut self, hwnd: HWND, f: Field) {
        match f {
            Field::Note | Field::Todo => {
                self.model.submit_jot(f);
            }
            Field::Place => self.place_enter(hwnd),
        }
        self.reset_caret(hwnd);
        self.after_model_change(hwnd);
    }
}

/// Text on the clipboard, if any.
fn clipboard_text(hwnd: HWND) -> Option<String> {
    let bytes = unsafe { clipboard_bytes(hwnd, 13) }?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .take_while(|&u| u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}
