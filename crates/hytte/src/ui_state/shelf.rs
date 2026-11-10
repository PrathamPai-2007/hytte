//! The Shelf pane: which tiles are in view.

use super::*;

/// Height of the clipboard block's header, of each clip row, and the padding under it.
pub const CLIP_HEAD_H: f32 = 22.0;
pub const CLIP_ROW_H: f32 = 24.0;
pub const CLIP_PAD_H: f32 = 8.0;

impl Model {
    /// `[clipboard] history` as the config has it now: off forgets every clip.
    pub fn apply_clipboard(&mut self, on: bool, ttl_secs: u64) {
        self.clips_on = on;
        self.clip_ttl = Duration::from_secs(ttl_secs);
        if !on {
            self.clear_clips();
        }
    }

    /// Forget every clip (the Clear link, lock, sleep, an emptied clipboard).
    pub fn clear_clips(&mut self) {
        self.clips.clear();
        self.clip_off = 0;
    }

    /// Forget clip `i`.
    pub fn remove_clip(&mut self, i: usize) {
        if i < self.clips.len() {
            self.clips.remove(i);
        }
        // Stay within what is left.
        self.scroll_clips(0);
    }

    /// A clip taken from the clipboard (after the rules said it may be kept).
    pub fn add_clip(&mut self, text: &str) {
        if self.clips_on {
            self.clips.add(text);
            self.clip_off = 0;
        }
    }

    /// Whether the Shelf pane has a clipboard block to show.
    pub fn clips_shown(&self) -> bool {
        self.clips_on && self.clips.len() > 0
    }

    /// Clips in view: where they start and how many.
    pub fn clip_view(&self) -> (usize, usize) {
        let n = self.clips.len();
        let off = self
            .clip_off
            .min(n.saturating_sub(crate::clipboard::CLIP_ROWS));
        (off, n.saturating_sub(off).min(crate::clipboard::CLIP_ROWS))
    }

    /// Scroll the clips by `by` rows, staying within them.
    pub fn scroll_clips(&mut self, by: i32) {
        let max = self.clips.len().saturating_sub(crate::clipboard::CLIP_ROWS) as i32;
        let (off, _) = self.clip_view();
        self.clip_off = (off as i32 + by).clamp(0, max) as usize;
    }

    /// Height of the clipboard block (0 without clips).
    pub fn clips_h(&self) -> f64 {
        if !self.clips_shown() {
            return 0.0;
        }
        (CLIP_HEAD_H + self.clip_view().1 as f32 * CLIP_ROW_H + CLIP_PAD_H) as f64
    }

    /// Height of the shelf's own part (tiles, hint or the selected item's chips), without the tab
    /// strip and the clips. Nothing when only clips are there.
    pub fn shelf_tiles_h(&self) -> f64 {
        if self.shelf.is_empty() {
            0.0
        } else if self.shelf_sel.is_some() {
            136.0 + 32.0 * (self.shelf_chip_lines() - 1) as f64
        } else {
            118.0
        }
    }

    /// How many lines the selected shelf item's action chips take (1 when none is selected).
    pub fn shelf_chip_lines(&self) -> usize {
        let Some(it) = self
            .shelf_sel
            .and_then(|id| self.shelf.iter().find(|i| i.id == id))
        else {
            return 1;
        };
        let chips = crate::transforms::chips_for(&it.path);
        let labels: Vec<&str> = chips.iter().map(|c| c.0).collect();
        crate::transforms::chip_lines(&labels, 380.0 - 8.0).len()
    }

    /// Panels that currently have something to show, in priority order.
    /// Left to right: shelf, music, the quick controls (home), then what comes and goes (dev
    /// work, a running timer), and the battery at the right end.
    /// The shelf tiles in view, and where they start (clamped, so removing items never
    /// leaves the row scrolled past its end).
    pub fn shelf_page(&self) -> (usize, &[ShelfItem]) {
        let off = self
            .shelf_off
            .min(self.shelf.len().saturating_sub(SHELF_VISIBLE));
        let end = (off + SHELF_VISIBLE).min(self.shelf.len());
        (off, &self.shelf[off..end])
    }

    /// Scroll the shelf row by `by` tiles, staying within it.
    pub fn scroll_shelf(&mut self, by: i32) {
        let (off, _) = self.shelf_page();
        let max = self.shelf.len().saturating_sub(SHELF_VISIBLE) as i32;
        self.shelf_off = (off as i32 + by).clamp(0, max) as usize;
    }
}
