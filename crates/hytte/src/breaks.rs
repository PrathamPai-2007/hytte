//! Break reminders: eyes (20-20-20), stretch, water. Pure scheduling, no Win32: the window layer
//! says whether the moment is clear (not in Focus, a meeting, a game, quiet hours...) and shows
//! the nudge.
//!
//! Each kind has its own interval. A nudge that comes due while the way is blocked is not lost:
//! it waits and shows once, as soon as the way clears. Five minutes away from the PC counts as
//! a break and restarts every clock. Two nudges never come back to back.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Eyes,
    Stretch,
    Water,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Eyes, Kind::Stretch, Kind::Water];

    fn index(self) -> usize {
        self as usize
    }

    /// The headline of the card.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Eyes => "Look 20 feet away",
            Kind::Stretch => "Time to stretch",
            Kind::Water => "Time for some water",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Kind::Eyes => "Rest your eyes for 20 seconds",
            Kind::Stretch => "Stand up and shake it out",
            Kind::Water => "Take a few sips",
        }
    }

    /// How long its card stays (the eye break counts 20 seconds down).
    pub fn hold(self) -> Duration {
        match self {
            Kind::Eyes => Duration::from_secs(20),
            _ => Duration::from_secs(10),
        }
    }
}

/// Idle this long counts as a break from the screen.
pub const AWAY: Duration = Duration::from_secs(5 * 60);
/// Between one nudge showing and the next.
pub const GAP: Duration = Duration::from_secs(2 * 60);
/// Snooze length.
pub const SNOOZE: Duration = Duration::from_secs(10 * 60);
/// How often a blocked nudge looks again whether the way is clear.
pub const RECHECK: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub struct Breaks {
    /// Interval in minutes per kind (0 = off).
    minutes: [u32; 3],
    due: [Option<Instant>; 3],
    last_shown: Option<Instant>,
}

impl Breaks {
    /// Apply the settings. A kind whose interval didn't change keeps its clock; a newly switched
    /// on one starts from now; one switched off stops.
    pub fn configure(&mut self, minutes: [u32; 3], now: Instant) {
        for (i, &m) in minutes.iter().enumerate() {
            if m == 0 {
                self.due[i] = None;
            } else if m != self.minutes[i] || self.due[i].is_none() {
                self.due[i] = Some(now + Duration::from_secs(m as u64 * 60));
            }
        }
        self.minutes = minutes;
    }

    fn restart(&mut self, i: usize, now: Instant) {
        self.due[i] =
            (self.minutes[i] > 0).then(|| now + Duration::from_secs(self.minutes[i] as u64 * 60));
    }

    /// Is a nudge due now? `idle` is the time since the last input; `clear` says nothing is in
    /// the way. Returns the kind to show (its clock restarts) or None.
    pub fn tick(&mut self, now: Instant, idle: Duration, clear: bool) -> Option<Kind> {
        if idle >= AWAY {
            // A break from the screen: every clock starts over, nothing is owed.
            for i in 0..3 {
                self.restart(i, now);
            }
            return None;
        }
        if !clear || self.last_shown.is_some_and(|t| now < t + GAP) {
            return None;
        }
        let i = (0..3)
            .filter(|&i| self.due[i].is_some_and(|d| d <= now))
            .min_by_key(|&i| self.due[i])?;
        self.restart(i, now);
        self.last_shown = Some(now);
        Some(Kind::ALL[i])
    }

    /// Put a kind off for a while (Snooze).
    pub fn snooze(&mut self, k: Kind, now: Instant) {
        if self.minutes[k.index()] > 0 {
            self.due[k.index()] = Some(now + SNOOZE);
        }
    }

    /// While a clock runs and the user is not yet away: when five minutes without input would be
    /// reached, so the break gets noticed even if nothing else wakes us.
    pub fn away_deadline(&self, now: Instant, idle: Duration) -> Option<Instant> {
        (idle < AWAY && self.due.iter().any(Option::is_some)).then(|| now + (AWAY - idle))
    }

    /// When to look again: the next due time, or a minute from now while a due one is blocked.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let first = self.due.iter().flatten().min().copied()?;
        let gap_end = self.last_shown.map(|t| t + GAP);
        let soonest = gap_end.map_or(first, |g| first.max(g));
        // Due already (blocked, since `tick` would have shown it): ask again soon.
        Some(if soonest <= now {
            now + RECHECK
        } else {
            soonest
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Duration = Duration::from_secs(60);

    fn all(eyes: u32, stretch: u32, water: u32, t0: Instant) -> Breaks {
        let mut b = Breaks::default();
        b.configure([eyes, stretch, water], t0);
        b
    }

    #[test]
    fn a_wake_is_set_for_when_the_user_counts_as_away() {
        let t0 = Instant::now();
        let b = all(20, 0, 0, t0);
        assert_eq!(b.away_deadline(t0, 2 * MIN), Some(t0 + 3 * MIN));
        assert_eq!(b.away_deadline(t0, AWAY), None);
        assert_eq!(all(0, 0, 0, t0).away_deadline(t0, MIN), None);
    }

    #[test]
    fn nothing_is_due_when_everything_is_off() {
        let t0 = Instant::now();
        let mut b = all(0, 0, 0, t0);
        assert_eq!(b.tick(t0 + 500 * MIN, Duration::ZERO, true), None);
        assert_eq!(b.next_deadline(t0), None);
    }

    #[test]
    fn each_kind_comes_due_on_its_own_interval_and_restarts() {
        let t0 = Instant::now();
        let mut b = all(20, 60, 0, t0);
        assert_eq!(b.tick(t0 + 19 * MIN, Duration::ZERO, true), None);
        assert_eq!(b.next_deadline(t0), Some(t0 + 20 * MIN));
        assert_eq!(
            b.tick(t0 + 20 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        // Not twice, and the gap keeps the next one back.
        assert_eq!(b.tick(t0 + 20 * MIN, Duration::ZERO, true), None);
        assert_eq!(
            b.tick(t0 + 40 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        // Both due at minute 60: the first kind wins the tie, the other follows after the gap.
        assert_eq!(
            b.tick(t0 + 60 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        assert_eq!(b.tick(t0 + 61 * MIN, Duration::ZERO, true), None);
        assert_eq!(
            b.tick(t0 + 62 * MIN, Duration::ZERO, true),
            Some(Kind::Stretch)
        );
    }

    #[test]
    fn a_blocked_nudge_waits_and_shows_once_when_the_way_clears() {
        let t0 = Instant::now();
        let mut b = all(20, 0, 0, t0);
        // Focus (or a meeting) for an hour: nothing shows, the owed nudge is one, not three.
        assert_eq!(b.tick(t0 + 60 * MIN, Duration::ZERO, false), None);
        assert_eq!(b.next_deadline(t0 + 60 * MIN), Some(t0 + 61 * MIN));
        assert_eq!(
            b.tick(t0 + 61 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        assert_eq!(b.tick(t0 + 62 * MIN, Duration::ZERO, true), None);
        assert_eq!(b.next_deadline(t0 + 61 * MIN), Some(t0 + 81 * MIN));
    }

    #[test]
    fn two_nudges_due_together_come_two_minutes_apart_oldest_first() {
        let t0 = Instant::now();
        let mut b = all(30, 30, 45, t0);
        assert_eq!(
            b.tick(t0 + 30 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        assert_eq!(b.tick(t0 + 31 * MIN, Duration::ZERO, true), None);
        assert_eq!(b.next_deadline(t0 + 31 * MIN), Some(t0 + 32 * MIN));
        assert_eq!(
            b.tick(t0 + 32 * MIN, Duration::ZERO, true),
            Some(Kind::Stretch)
        );
        assert_eq!(
            b.tick(t0 + 45 * MIN, Duration::ZERO, true),
            Some(Kind::Water)
        );
    }

    #[test]
    fn five_minutes_away_restarts_every_clock() {
        let t0 = Instant::now();
        let mut b = all(20, 60, 90, t0);
        // At the PC for 15 minutes, then gone for 6 and back at minute 21: nothing is due.
        assert_eq!(b.tick(t0 + 15 * MIN, 5 * MIN, true), None);
        assert_eq!(b.tick(t0 + 21 * MIN, Duration::ZERO, true), None);
        assert_eq!(
            b.next_deadline(t0 + 21 * MIN),
            Some(t0 + 35 * MIN),
            "20 min after the away check"
        );
        // A short idle spell is not a break.
        let mut c = all(20, 0, 0, t0);
        assert_eq!(c.tick(t0 + 20 * MIN, 4 * MIN, true), Some(Kind::Eyes));
    }

    #[test]
    fn snooze_puts_one_kind_off_ten_minutes() {
        let t0 = Instant::now();
        let mut b = all(20, 0, 0, t0);
        assert_eq!(
            b.tick(t0 + 20 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        b.snooze(Kind::Eyes, t0 + 21 * MIN);
        assert_eq!(b.next_deadline(t0 + 21 * MIN), Some(t0 + 31 * MIN));
        b.snooze(Kind::Water, t0); // off: stays off
        assert_eq!(
            b.tick(t0 + 31 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
    }

    #[test]
    fn changing_settings_keeps_unchanged_clocks() {
        let t0 = Instant::now();
        let mut b = all(20, 60, 0, t0);
        b.configure([20, 0, 45], t0 + 10 * MIN);
        assert_eq!(
            b.next_deadline(t0),
            Some(t0 + 20 * MIN),
            "eyes kept its clock"
        );
        assert_eq!(
            b.tick(t0 + 55 * MIN, Duration::ZERO, true),
            Some(Kind::Eyes)
        );
        assert_eq!(
            b.tick(t0 + 57 * MIN, Duration::ZERO, true),
            Some(Kind::Water),
            "water started at minute 10"
        );
        assert_eq!(
            b.tick(t0 + 61 * MIN, Duration::ZERO, true),
            None,
            "stretch is off"
        );
    }

    #[test]
    fn eye_breaks_count_twenty_seconds_the_others_ten() {
        assert_eq!(Kind::Eyes.hold(), Duration::from_secs(20));
        assert_eq!(Kind::Water.hold(), Duration::from_secs(10));
        assert_eq!(Kind::Stretch.hold(), Duration::from_secs(10));
    }
}
