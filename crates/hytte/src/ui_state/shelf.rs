//! The Shelf pane: which tiles are in view.

use super::*;

impl Model {
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
