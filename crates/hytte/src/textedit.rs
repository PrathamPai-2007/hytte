//! One line of editable text: caret, selection, word jumps. Pure logic, no Win32, so the fields
//! on the pill (notes, to-dos, the weather place) can be tested without a window.
//!
//! Positions are byte offsets that always sit on a character boundary and never inside a
//! combining mark, joiner or emoji modifier, so one arrow press or Backspace moves over a whole
//! visible character.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextEdit {
    text: String,
    caret: usize,
    anchor: usize,
    /// Most characters the field takes (0 = no limit).
    limit: usize,
}

/// Marks that attach to the character before them.
fn extends(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0x200D | 0xFE00..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextEdit {
    pub fn new(limit: usize) -> Self {
        TextEdit {
            limit,
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub fn with_text(text: &str, limit: usize) -> Self {
        let mut t = Self::new(limit);
        t.insert(text);
        t
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected range as (start, end) bytes, or `None` with nothing selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.caret != self.anchor)
            .then(|| (self.caret.min(self.anchor), self.caret.max(self.anchor)))
    }

    pub fn selected(&self) -> &str {
        self.selection().map_or("", |(a, b)| &self.text[a..b])
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
    }

    /// Replace everything and put the caret at the end.
    pub fn set(&mut self, text: &str) {
        self.clear();
        self.insert(text);
    }

    fn prev(&self, from: usize) -> usize {
        let mut i = from;
        loop {
            // Step over the character and any marks attached to it.
            while i > 0 {
                i -= 1;
                while !self.text.is_char_boundary(i) {
                    i -= 1;
                }
                if !self.text[i..].chars().next().is_some_and(extends) {
                    break;
                }
            }
            // A joiner glues the character before it to this one: keep going.
            if i > 0 && self.text[..i].ends_with('\u{200D}') {
                continue;
            }
            return i;
        }
    }

    fn next(&self, from: usize) -> usize {
        let mut i = from;
        let mut rest = self.text[from..].chars();
        let Some(first) = rest.next() else {
            return from;
        };
        i += first.len_utf8();
        let mut joined = false;
        for c in rest {
            if extends(c) {
                joined = c == '\u{200D}';
            } else if joined {
                joined = false; // the character a joiner pulls in
            } else {
                break;
            }
            i += c.len_utf8();
        }
        i
    }

    fn word_prev(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 {
            let p = self.prev(i);
            if self.text[p..].chars().next().is_some_and(is_word) {
                break;
            }
            i = p;
        }
        while i > 0 {
            let p = self.prev(i);
            if !self.text[p..].chars().next().is_some_and(is_word) {
                break;
            }
            i = p;
        }
        i
    }

    fn word_next(&self, from: usize) -> usize {
        let mut i = from;
        while i < self.text.len() {
            if self.text[i..].chars().next().is_some_and(is_word) {
                break;
            }
            i = self.next(i);
        }
        while i < self.text.len() {
            if !self.text[i..].chars().next().is_some_and(is_word) {
                break;
            }
            i = self.next(i);
        }
        i
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        self.caret = to;
        if !extend {
            self.anchor = to;
        }
    }

    pub fn left(&mut self, extend: bool, word: bool) {
        if !extend {
            if let Some((a, _)) = self.selection() {
                return self.move_to(a, false);
            }
        }
        let to = if word {
            self.word_prev(self.caret)
        } else {
            self.prev(self.caret)
        };
        self.move_to(to, extend);
    }

    pub fn right(&mut self, extend: bool, word: bool) {
        if !extend {
            if let Some((_, b)) = self.selection() {
                return self.move_to(b, false);
            }
        }
        let to = if word {
            self.word_next(self.caret)
        } else {
            self.next(self.caret)
        };
        self.move_to(to, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// Put the caret at a byte offset (a click), snapped back to a real boundary.
    pub fn set_caret(&mut self, byte: usize, extend: bool) {
        let mut b = byte.min(self.text.len());
        while !self.text.is_char_boundary(b) {
            b -= 1;
        }
        while b > 0 && self.text[b..].chars().next().is_some_and(extends) {
            b = self.prev(b);
        }
        self.move_to(b, extend);
    }

    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            return false;
        };
        self.text.replace_range(a..b, "");
        self.move_to(a, false);
        true
    }

    /// Type or paste. Line breaks and tabs become spaces, other control characters go, and the
    /// text is cut at the limit.
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        let mut room = if self.limit == 0 {
            usize::MAX
        } else {
            self.limit.saturating_sub(self.text.chars().count())
        };
        let mut add = String::new();
        for c in s.chars() {
            let c = if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            };
            if c.is_control() {
                continue;
            }
            if room == 0 && !extends(c) {
                break;
            }
            if !extends(c) {
                room -= 1;
            }
            add.push(c);
        }
        self.text.insert_str(self.caret, &add);
        self.move_to(self.caret + add.len(), false);
    }

    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() {
            return;
        }
        let to = if word {
            self.word_prev(self.caret)
        } else {
            self.prev(self.caret)
        };
        self.text.replace_range(to..self.caret, "");
        self.move_to(to, false);
    }

    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() {
            return;
        }
        let to = if word {
            self.word_next(self.caret)
        } else {
            self.next(self.caret)
        };
        self.text.replace_range(self.caret..to, "");
    }

    /// Cut: the selected text, removed.
    pub fn cut(&mut self) -> String {
        let s = self.selected().to_string();
        self.delete_selection();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> TextEdit {
        TextEdit::with_text(s, 0)
    }

    #[test]
    fn typing_and_deleting() {
        let mut e = t("");
        e.insert("héllo");
        assert_eq!((e.text(), e.caret()), ("héllo", 6));
        e.left(false, false);
        e.backspace(false);
        assert_eq!(e.text(), "hélo");
        e.delete(false);
        assert_eq!(e.text(), "hél");
        e.home(false);
        e.delete(false);
        assert_eq!(e.text(), "él");
    }

    #[test]
    fn selection_replaces_and_arrows_collapse_it() {
        let mut e = t("one two three");
        e.select_all();
        assert_eq!(e.selected(), "one two three");
        e.left(false, false);
        assert_eq!((e.caret(), e.selection()), (0, None));
        e.end(false);
        e.left(true, true);
        assert_eq!(e.selected(), "three");
        e.insert("4");
        assert_eq!(e.text(), "one two 4");
        e.select_all();
        assert_eq!(e.cut(), "one two 4");
        assert!(e.is_empty());
    }

    #[test]
    fn word_jumps_skip_spaces_and_punctuation() {
        let mut e = t("alpha, beta  gamma");
        e.left(false, true);
        assert_eq!(e.caret(), 13);
        e.left(false, true);
        assert_eq!(e.caret(), 7);
        e.home(false);
        e.right(false, true);
        assert_eq!(e.caret(), 5);
        e.backspace(true);
        assert_eq!(e.text(), ", beta  gamma");
    }

    #[test]
    fn a_visible_character_moves_as_one() {
        // e + combining acute, a thumbs-up with a skin tone, a family joined by joiners.
        let s = "e\u{301}\u{1F44D}\u{1F3FD}\u{1F468}\u{200D}\u{1F469}";
        let mut e = t(s);
        e.left(false, false);
        assert_eq!(e.text()[e.caret()..].chars().next(), Some('\u{1F468}'));
        e.left(false, false);
        assert_eq!(e.text()[e.caret()..].chars().next(), Some('\u{1F44D}'));
        e.left(false, false);
        assert_eq!(e.caret(), 0);
        e.right(false, false);
        assert_eq!(e.caret(), 3); // past e + accent
        e.end(false);
        e.backspace(false);
        assert_eq!(e.text(), "e\u{301}\u{1F44D}\u{1F3FD}");
    }

    #[test]
    fn paste_is_one_line_and_limited() {
        let mut e = TextEdit::new(8);
        e.insert("a\nb\r\nc\td\u{7}efghij");
        assert_eq!(e.text(), "a b  c d");
        e.insert("zzz");
        assert_eq!(e.text(), "a b  c d");
        let mut f = TextEdit::new(2);
        f.insert("e\u{301}x!");
        assert_eq!(f.text(), "e\u{301}x"); // the accent doesn't use up room
    }

    #[test]
    fn a_click_snaps_to_a_boundary() {
        let mut e = t("aé\u{301}b");
        e.set_caret(2, false); // inside é
        assert_eq!(e.caret(), 1);
        e.set_caret(4, false); // between é and its accent
        assert_eq!(e.caret(), 1);
        e.set_caret(99, true);
        assert_eq!(e.selected(), "é\u{301}b");
    }

    #[test]
    fn edges_do_nothing() {
        let mut e = t("");
        e.left(false, false);
        e.right(false, true);
        e.backspace(true);
        e.delete(false);
        assert_eq!(e.text(), "");
    }
}
