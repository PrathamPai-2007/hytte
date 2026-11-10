//! The welcome tour: three cards on the pill, shown once, to a fresh install only. Updating
//! users get nothing. `%APPDATA%\Hytte\welcome` remembers that the tour was finished or skipped.

use std::sync::atomic::{AtomicBool, Ordering};

/// What the marker file holds.
const SEEN: &str = "seen";

/// No `config.toml` (or an empty one) when Hytte started: a fresh install.
static FRESH: AtomicBool = AtomicBool::new(false);

/// Note at start, before the config is first written, whether this is a fresh install.
pub fn note_start(config_text: Option<&str>) {
    FRESH.store(
        config_text.is_none_or(|t| t.trim().is_empty()),
        Ordering::Relaxed,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tour {
    /// A fresh install: what Hytte does.
    Intro,
}

/// Which tour to show: only a fresh install that hasn't seen it (`seen` is the marker's text).
pub fn which(fresh: bool, seen: Option<&str>) -> Option<Tour> {
    (fresh && seen.is_none()).then_some(Tour::Intro)
}

fn marker() -> std::path::PathBuf {
    crate::config::data_dir().join("welcome")
}

/// The tour waiting to be shown at this start, if any.
pub fn pending() -> Option<Tour> {
    let seen = std::fs::read_to_string(marker()).ok();
    which(FRESH.load(Ordering::Relaxed), seen.as_deref())
}

/// The tour was finished or skipped: don't show it again.
pub fn finish() {
    let _ = std::fs::write(marker(), SEEN);
}

/// Cards in a tour.
pub const STEPS: usize = 3;

/// The words of card `step` (0-based): title and one line. `hotkey`: the keyboard-mode chord,
/// mentioned where it helps when one is set. Each line fits the card without being cut.
pub fn card(tour: Tour, step: usize, hotkey: &str) -> (String, String) {
    let n = format!("{} of {STEPS}", step + 1);
    let (title, body): (&str, String) = match (tour, step) {
        (Tour::Intro, 0) => (
            "Welcome to Hytte",
            "A small pill at the top of your screen.".into(),
        ),
        (Tour::Intro, 1) => (
            "What it does",
            "Keeps your tasks, music, timers and files close.".into(),
        ),
        (Tour::Intro, _) => (
            "Open it",
            match hotkey.trim() {
                "" => "Point at the pill. Right-click it for a timer.".into(),
                k => format!("Point at the pill, or press {k}."),
            },
        ),
    };
    (format!("{title}  ·  {n}"), body)
}

/// Card `step` as a chip: the words, **Next** and **Skip** (the last card: **Done**). It
/// stays two minutes; unanswered, it comes back on the next hover.
pub fn chip(
    tour: Tour,
    step: usize,
    hotkey: &str,
    now: std::time::Instant,
) -> crate::ui_state::Chip {
    use crate::ui_state::{Chip, ChipAction, Tone};
    let (title, body) = card(tour, step, hotkey);
    let mut c = Chip::new(format!("{title}\n{body}"), now);
    c.tone = Tone::Info;
    c.hold = std::time::Duration::from_secs(120);
    let last = step + 1 == STEPS;
    c.extra = vec![(
        if last { "Done" } else { "Next" }.into(),
        ChipAction::Welcome(step + 1),
    )];
    if !last {
        c.extra.push(("Skip".into(), ChipAction::WelcomeSkip));
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fresh_install_sees_the_tour_once() {
        assert_eq!(which(true, None), Some(Tour::Intro));
        assert_eq!(which(true, Some("seen")), None);
        assert_eq!(which(false, None), None, "an update shows nothing");
        assert_eq!(which(false, Some("1.7")), None);
    }

    #[test]
    fn a_fresh_install_is_one_without_settings() {
        note_start(None);
        assert!(FRESH.load(Ordering::Relaxed));
        note_start(Some("  \n"));
        assert!(FRESH.load(Ordering::Relaxed));
        note_start(Some("[general]\nskin = \"moss\"\n"));
        assert!(!FRESH.load(Ordering::Relaxed));
    }

    #[test]
    fn every_card_has_words_that_fit() {
        for tour in [Tour::Intro] {
            for step in 0..STEPS {
                for key in ["", "Ctrl+Alt+F9"] {
                    let (t, b) = card(tour, step, key);
                    assert!(t.ends_with(&format!("{} of {STEPS}", step + 1)));
                    // About what one line of the card holds before it is cut with "…".
                    assert!(b.chars().count() <= 54, "{tour:?} {step}: {b}");
                    assert!(!b.is_empty());
                }
            }
        }
        assert!(card(Tour::Intro, 2, "Win+Alt+H").1.contains("Win+Alt+H"));
    }

    #[test]
    fn cards_step_on_and_the_last_one_is_done() {
        use crate::ui_state::ChipAction;
        let now = std::time::Instant::now();
        let first = chip(Tour::Intro, 0, "", now);
        let acts: Vec<_> = first.extra.iter().map(|e| (e.0.as_str(), &e.1)).collect();
        assert_eq!(
            acts,
            [
                ("Next", &ChipAction::Welcome(1)),
                ("Skip", &ChipAction::WelcomeSkip)
            ]
        );
        let last = chip(Tour::Intro, STEPS - 1, "", now);
        assert_eq!(last.extra.len(), 1);
        assert_eq!(last.extra[0], ("Done".into(), ChipAction::Welcome(STEPS)));
    }
}
