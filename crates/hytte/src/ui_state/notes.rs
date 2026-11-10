//! The Notes and To-do panes: the jot lists and their text fields.

use super::*;

/// A text field on the pill that can take keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Note,
    Todo,
    Place,
}

/// Rows of notes or to-dos shown before the list scrolls.
pub const JOT_ROWS: usize = 4;

/// How long Undo stays after a delete.
pub const UNDO_HOLD: Duration = Duration::from_secs(6);

impl Model {
    /// Load the saved notes and give the text fields their limits.
    pub fn init_jot(&mut self, path: &std::path::Path) {
        self.jot = crate::jot::Store::load(path, crate::timer::now_ms());
        self.note_edit = crate::textedit::TextEdit::new(crate::jot::MAX_CHARS);
        self.todo_edit = crate::textedit::TextEdit::new(crate::jot::MAX_CHARS);
        self.place_edit = crate::textedit::TextEdit::new(80);
        self.note_keep = false;
    }

    pub fn edit_mut(&mut self, f: Field) -> &mut crate::textedit::TextEdit {
        match f {
            Field::Note => &mut self.note_edit,
            Field::Todo => &mut self.todo_edit,
            Field::Place => &mut self.place_edit,
        }
    }

    pub fn edit_ref(&self, f: Field) -> &crate::textedit::TextEdit {
        match f {
            Field::Note => &self.note_edit,
            Field::Todo => &self.todo_edit,
            Field::Place => &self.place_edit,
        }
    }

    /// Enter in a Notes or To-do field: add the item, or save the edit of an existing one.
    /// Returns true when something was stored. The field clears when it was.
    pub fn submit_jot(&mut self, f: Field) -> bool {
        use crate::jot::Kind;
        let kind = match f {
            Field::Note => Kind::Note,
            Field::Todo => Kind::Todo,
            Field::Place => return false,
        };
        let text = self.edit_ref(f).text().to_string();
        let now = crate::timer::now_ms();
        let ok = match self.jot_edit {
            Some(id) if !text.trim().is_empty() => {
                self.jot_edit = None;
                // The item expired while it was being edited: keep the text as a new one.
                self.jot.edit(id, &text) || self.jot.add(kind, &text, self.note_keep, now).is_some()
            }
            Some(_) => false,
            None => self.jot.add(kind, &text, self.note_keep, now).is_some(),
        };
        if ok {
            self.edit_mut(f).clear();
            self.jot_first = 0;
            self.jot_dirty = true;
        }
        ok
    }

    /// Delete an item, keeping it for Undo.
    pub fn delete_jot(&mut self, id: u64, now: Instant) {
        if let Some(j) = self.jot.remove(id) {
            self.jot_undo = Some((j, now));
            self.jot_dirty = true;
            if self.jot_edit == Some(id) {
                self.jot_edit = None;
            }
        }
    }

    pub fn undo_jot(&mut self) {
        if let Some((j, _)) = self.jot_undo.take() {
            self.jot.restore(j);
            self.jot_dirty = true;
        }
    }

    /// Start editing an existing item in its pane's field.
    pub fn begin_edit(&mut self, id: u64) {
        let Some(j) = self.jot.get(id) else { return };
        let (f, text) = match j.kind {
            crate::jot::Kind::Note => (Field::Note, j.text.clone()),
            crate::jot::Kind::Todo => (Field::Todo, j.text.clone()),
        };
        self.jot_edit = Some(id);
        self.edit_mut(f).set(&text);
        self.typing = Some(f);
    }

    /// Typing ended (Esc, clicked away): a half-made edit of an existing item is dropped, a
    /// draft for a new one stays.
    pub fn end_typing(&mut self) {
        if self.jot_edit.take().is_some() {
            if let Some(f) = self.typing {
                self.edit_mut(f).clear();
            }
        }
        // Leaving the place field with a place already chosen means "keep it".
        if self.typing == Some(Field::Place) && self.weather_place.is_some() {
            self.close_chooser();
        }
        self.typing = None;
    }

    /// The items of Notes or To-do as they list (top to bottom).
    pub fn jot_list(&self, scene: Scene) -> Vec<&crate::jot::Jot> {
        if scene == Scene::ExpTodo {
            self.jot.todos()
        } else {
            self.jot.notes()
        }
    }

    /// Rows in view: (first, count), with the scroll clamped.
    pub fn jot_window(&self, scene: Scene) -> (usize, usize) {
        let n = self.jot_list(scene).len();
        let first = self.jot_first.min(n.saturating_sub(JOT_ROWS));
        (first, (n - first).min(JOT_ROWS))
    }

    /// Height of the Notes / To-do pane without the tab strip: field, rows (one line of
    /// invitation when empty) and a little room below.
    pub fn jot_h(&self, scene: Scene) -> f64 {
        let (_, shown) = self.jot_window(scene);
        let undo = self
            .jot_undo
            .as_ref()
            .is_some_and(|(j, _)| (j.kind == crate::jot::Kind::Todo) == (scene == Scene::ExpTodo));
        let rows = (shown + undo as usize).max(1);
        12.0 + 34.0 + 8.0 + rows as f64 * 30.0 + 8.0
    }

    /// Scroll the Notes or To-do list by `by` rows; false when it didn't move (at an end, or
    /// nothing to scroll), so the wheel pages instead.
    pub fn scroll_jot(&mut self, by: i32) -> bool {
        let scene = if self.panel() == Panel::Todo {
            Scene::ExpTodo
        } else {
            Scene::ExpNotes
        };
        let max = self.jot_list(scene).len().saturating_sub(JOT_ROWS) as i32;
        let first = self.jot_first.min(max as usize) as i32;
        let next = (first + by).clamp(0, max);
        self.jot_first = next as usize;
        next != first
    }
}
