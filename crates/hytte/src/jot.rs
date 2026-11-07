//! Notes and to-dos: one list, one file (`jot.json`).
//!
//! A note lives 24 hours unless it is kept; a to-do leaves 24 hours after it is ticked. Times
//! are wall-clock milliseconds (like `timer.rs`), so a restart or sleep loses nothing, and the
//! next deadline feeds `Model::expire`, so no timer of its own runs.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_ITEMS: usize = 200;
pub const MAX_CHARS: usize = 2000;
/// How long a temporary note, or a ticked to-do, stays.
pub const LIFE_MS: u64 = 24 * 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Note,
    Todo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jot {
    pub id: u64,
    pub kind: Kind,
    pub text: String,
    pub made_ms: u64,
    /// A temporary note's end (None = kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    /// When a to-do was ticked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_ms: Option<u64>,
}

impl Jot {
    /// When this item leaves, if it ever does.
    pub fn deadline(&self) -> Option<u64> {
        match self.kind {
            Kind::Note => self.expires_ms,
            Kind::Todo => self.done_ms.map(|d| d + LIFE_MS),
        }
    }

    /// Fraction of a temporary note's life that is left (1 fresh, 0 gone); None when kept.
    pub fn life_left(&self, now: u64) -> Option<f32> {
        let end = self.expires_ms?;
        Some((end.saturating_sub(now) as f32 / LIFE_MS as f32).clamp(0.0, 1.0))
    }
}

/// "3 h", "40 m": time left until `end`, rounded up so it never says 0.
pub fn left_text(end: u64, now: u64) -> String {
    let m = end.saturating_sub(now).div_ceil(60_000).max(1);
    if m >= 60 {
        format!("{} h", m.div_ceil(60))
    } else {
        format!("{m} m")
    }
}

#[derive(Debug, Default)]
pub struct Store {
    pub items: Vec<Jot>,
    next: u64,
}

#[derive(Serialize, Deserialize)]
struct File {
    next: u64,
    items: Vec<Jot>,
}

pub fn path() -> PathBuf {
    crate::config::data_dir().join("jot.json")
}

impl Store {
    /// Read the file. A damaged one is set aside as `jot.json.bad` and never overwritten.
    pub fn load(path: &Path, now: u64) -> Store {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Store::default();
        };
        match serde_json::from_str::<File>(&text) {
            Ok(f) => {
                let next = f
                    .items
                    .iter()
                    .map(|j| j.id + 1)
                    .max()
                    .unwrap_or(0)
                    .max(f.next);
                let mut s = Store {
                    items: f.items,
                    next,
                };
                s.expire(now);
                s
            }
            Err(e) => {
                crate::logging::line(&format!("jot.json is damaged ({e}); kept as jot.json.bad"));
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                Store::default()
            }
        }
    }

    pub fn save(&self, path: &Path) {
        let f = File {
            next: self.next,
            items: self.items.clone(),
        };
        if let Ok(text) = serde_json::to_string(&f) {
            if let Err(e) = crate::config::write_atomic(path, &text) {
                crate::logging::line(&format!("couldn't save notes: {e}"));
            }
        }
    }

    /// Add an item; `None` when the text is empty or the list is full.
    pub fn add(&mut self, kind: Kind, text: &str, keep: bool, now: u64) -> Option<u64> {
        let text: String = text.trim().chars().take(MAX_CHARS).collect();
        if text.is_empty() || self.items.len() >= MAX_ITEMS {
            return None;
        }
        let id = self.next;
        self.next += 1;
        self.items.push(Jot {
            id,
            kind,
            text,
            made_ms: now,
            expires_ms: (kind == Kind::Note && !keep).then_some(now + LIFE_MS),
            done_ms: None,
        });
        Some(id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Jot> {
        self.items.iter_mut().find(|j| j.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Jot> {
        self.items.iter().find(|j| j.id == id)
    }

    pub fn remove(&mut self, id: u64) -> Option<Jot> {
        let i = self.items.iter().position(|j| j.id == id)?;
        Some(self.items.remove(i))
    }

    /// Put a removed item back (Undo), keeping its place in time.
    pub fn restore(&mut self, j: Jot) {
        self.items.push(j);
    }

    pub fn edit(&mut self, id: u64, text: &str) {
        let text: String = text.trim().chars().take(MAX_CHARS).collect();
        if let Some(j) = self.get_mut(id) {
            if !text.is_empty() {
                j.text = text;
            }
        }
    }

    pub fn keep(&mut self, id: u64) {
        if let Some(j) = self.get_mut(id) {
            j.expires_ms = None;
        }
    }

    /// Tick or untick a to-do.
    pub fn toggle(&mut self, id: u64, now: u64) {
        if let Some(j) = self.get_mut(id).filter(|j| j.kind == Kind::Todo) {
            j.done_ms = if j.done_ms.is_some() { None } else { Some(now) };
        }
    }

    /// Drop what is due; true when anything went.
    pub fn expire(&mut self, now: u64) -> bool {
        let n = self.items.len();
        self.items.retain(|j| j.deadline().is_none_or(|d| d > now));
        self.items.len() != n
    }

    /// The soonest moment something leaves.
    pub fn next_deadline(&self) -> Option<u64> {
        self.items.iter().filter_map(Jot::deadline).min()
    }

    /// Notes, newest first.
    pub fn notes(&self) -> Vec<&Jot> {
        let mut v: Vec<&Jot> = self.items.iter().filter(|j| j.kind == Kind::Note).collect();
        v.sort_by_key(|j| std::cmp::Reverse((j.made_ms, j.id)));
        v
    }

    /// To-dos: open ones in the order they were added, ticked ones below them.
    pub fn todos(&self) -> Vec<&Jot> {
        let mut v: Vec<&Jot> = self.items.iter().filter(|j| j.kind == Kind::Todo).collect();
        v.sort_by_key(|j| (j.done_ms.is_some(), j.made_ms, j.id));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3_600_000;

    #[test]
    fn a_temporary_note_leaves_after_a_day_and_a_kept_one_stays() {
        let mut s = Store::default();
        let a = s.add(Kind::Note, "  wifi 4471 ", false, 1000).unwrap();
        let b = s.add(Kind::Note, "passport", true, 1000).unwrap();
        assert_eq!(s.get_mut(a).unwrap().text, "wifi 4471");
        assert_eq!(s.next_deadline(), Some(1000 + 24 * H));
        assert!(!s.expire(1000 + 24 * H - 1));
        assert!(s.expire(1000 + 24 * H));
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].id, b);
        assert_eq!(s.next_deadline(), None);
        s.keep(a); // already gone: nothing happens
    }

    #[test]
    fn keeping_a_note_cancels_its_deadline() {
        let mut s = Store::default();
        let a = s.add(Kind::Note, "x", false, 0).unwrap();
        s.keep(a);
        assert!(!s.expire(100 * H));
    }

    #[test]
    fn ticked_todos_sink_and_leave_a_day_after_the_tick() {
        let mut s = Store::default();
        let a = s.add(Kind::Todo, "one", false, 1).unwrap();
        let b = s.add(Kind::Todo, "two", false, 2).unwrap();
        s.toggle(a, 5_000);
        let order: Vec<u64> = s.todos().iter().map(|j| j.id).collect();
        assert_eq!(order, vec![b, a]);
        assert_eq!(s.next_deadline(), Some(5_000 + 24 * H));
        s.toggle(a, 6_000); // unticked: no deadline
        assert_eq!(s.next_deadline(), None);
        s.toggle(a, 7_000);
        assert!(s.expire(7_000 + 24 * H));
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn empty_text_and_a_full_list_are_refused() {
        let mut s = Store::default();
        assert!(s.add(Kind::Note, "   ", false, 0).is_none());
        for i in 0..MAX_ITEMS {
            assert!(s.add(Kind::Todo, &i.to_string(), false, 0).is_some());
        }
        assert!(s.add(Kind::Todo, "one too many", false, 0).is_none());
        let long = "é".repeat(MAX_CHARS + 50);
        s.items.clear();
        let id = s.add(Kind::Note, &long, false, 0).unwrap();
        assert_eq!(s.get_mut(id).unwrap().text.chars().count(), MAX_CHARS);
    }

    #[test]
    fn undo_puts_an_item_back() {
        let mut s = Store::default();
        let a = s.add(Kind::Note, "keep me", true, 10).unwrap();
        let j = s.remove(a).unwrap();
        assert!(s.items.is_empty());
        s.restore(j);
        assert_eq!(s.notes()[0].text, "keep me");
        s.edit(a, "  changed ");
        s.edit(a, "   "); // an empty edit is ignored
        assert_eq!(s.notes()[0].text, "changed");
    }

    #[test]
    fn items_survive_a_restart_and_expired_ones_do_not() {
        let dir = std::env::temp_dir().join(format!("hytte-jot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("jot.json");
        let mut s = Store::default();
        s.add(Kind::Note, "stays", true, 0);
        s.add(Kind::Note, "goes", false, 0);
        let t = s.add(Kind::Todo, "ticked", false, 0).unwrap();
        s.toggle(t, 0);
        s.save(&p);
        let back = Store::load(&p, 25 * H);
        assert_eq!(back.items.len(), 1);
        assert_eq!(back.items[0].text, "stays");
        let mut back = Store::load(&p, 0);
        assert_eq!(back.items.len(), 3);
        // ids keep counting after a reload
        let id = back.add(Kind::Note, "new", true, 0).unwrap();
        assert_eq!(back.items.iter().filter(|j| j.id == id).count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_set_aside_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("hytte-jot-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("jot.json");
        std::fs::write(&p, "{ not json").unwrap();
        let s = Store::load(&p, 0);
        assert!(s.items.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("jot.json.bad")).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn time_left_rounds_up() {
        assert_eq!(left_text(10 * 60_000, 0), "10 m");
        assert_eq!(left_text(1, 0), "1 m");
        assert_eq!(left_text(90 * 60_000, 0), "2 h");
        assert_eq!(left_text(0, 5), "1 m");
    }
}
