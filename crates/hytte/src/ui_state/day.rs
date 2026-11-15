//! Today's agent time: how long agents worked, how long they waited for you, how many turns
//! finished. In memory only (never written to disk); it starts over at local midnight and on
//! every restart.

use super::*;

/// Height of the "Today" line under the agent rows.
pub const AGENT_DAY_H: f32 = 20.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Working,
    Waiting,
}

#[derive(Debug, Default)]
pub struct DayStats {
    /// Local date as yyyymmdd; a change starts the totals over.
    day: u32,
    working: Duration,
    waiting: Duration,
    runs: u32,
    /// Each live agent turn's phase and when it began, so time counts as it passes.
    live: std::collections::HashMap<String, (Phase, Instant)>,
}

impl DayStats {
    /// One agent report: `event` for turn `id` at `now`, on local date `day`.
    pub fn note(&mut self, id: &str, event: TaskEvent, now: Instant, day: u32) {
        if day != self.day {
            // A new day: what ran before midnight stays in the past; running turns carry on.
            self.day = day;
            self.working = Duration::ZERO;
            self.waiting = Duration::ZERO;
            self.runs = 0;
            for (_, since) in self.live.values_mut() {
                *since = now;
            }
        }
        let next = match event {
            TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed => Some(Phase::Working),
            TaskEvent::NeedsInput => Some(Phase::Waiting),
            _ => None,
        };
        let cur = self.live.get(id).copied();
        if cur.map(|c| c.0) == next && next.is_some() {
            return;
        }
        if let Some((p, since)) = cur {
            let spent = now.saturating_duration_since(since);
            match p {
                Phase::Working => self.working += spent,
                Phase::Waiting => self.waiting += spent,
            }
        }
        self.runs += (event == TaskEvent::Done) as u32;
        match next {
            Some(p) => {
                self.live.insert(id.to_string(), (p, now));
            }
            None => {
                self.live.remove(id);
            }
        }
    }

    /// (working, waiting, finished turns) including the phases still running.
    pub fn totals(&self, now: Instant) -> (Duration, Duration, u32) {
        let (mut w, mut q) = (self.working, self.waiting);
        for (p, since) in self.live.values() {
            let spent = now.saturating_duration_since(*since);
            match p {
                Phase::Working => w += spent,
                Phase::Waiting => q += spent,
            }
        }
        (w, q, self.runs)
    }
}

/// "Today  2 h 10 min working · 14 min waiting for you · 9 turns", only the parts with
/// something in them; None when nothing has happened today. Under a minute rounds away.
pub fn day_text(t: (Duration, Duration, u32)) -> Option<String> {
    let span = |d: Duration| ran_text(Duration::from_secs(d.as_secs() / 60 * 60));
    let parts: Vec<String> = [
        (t.0.as_secs() >= 60).then(|| format!("{} working", span(t.0))),
        (t.1.as_secs() >= 60).then(|| format!("{} waiting for you", span(t.1))),
        (t.2 > 0).then(|| format!("{} turn{}", t.2, if t.2 == 1 { "" } else { "s" })),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| format!("Today  {}", parts.join(" · ")))
}

impl Model {
    /// The Agents pane's "Today" line.
    pub fn day_line(&self) -> Option<String> {
        day_text(self.day.totals(self.now?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_and_waiting_time_add_up_and_turns_are_counted() {
        let t0 = Instant::now();
        let at = |m: u64| t0 + Duration::from_secs(m * 60);
        let mut d = DayStats::default();
        d.note("a", TaskEvent::Start, at(0), 1);
        d.note("a", TaskEvent::Progress, at(2), 1); // still working: no new phase
        d.note("a", TaskEvent::NeedsInput, at(10), 1);
        d.note("a", TaskEvent::Resumed, at(14), 1);
        // Mid-phase counts as it passes.
        let (w, q, r) = d.totals(at(16));
        assert_eq!((w.as_secs() / 60, q.as_secs() / 60, r), (12, 4, 0));
        d.note("a", TaskEvent::Done, at(20), 1);
        let (w, q, r) = d.totals(at(99));
        assert_eq!(
            (w.as_secs() / 60, q.as_secs() / 60, r),
            (16, 4, 1),
            "a finished turn stops counting"
        );
        // A failure ends the turn without counting it as finished.
        d.note("b", TaskEvent::Start, at(30), 1);
        d.note("b", TaskEvent::Failed, at(31), 1);
        assert_eq!(d.totals(at(99)).2, 1);
    }

    #[test]
    fn a_new_day_starts_over_but_a_running_turn_carries_on() {
        let t0 = Instant::now();
        let at = |m: u64| t0 + Duration::from_secs(m * 60);
        let mut d = DayStats::default();
        d.note("a", TaskEvent::Start, at(0), 1);
        d.note("b", TaskEvent::Start, at(0), 1);
        d.note("b", TaskEvent::Done, at(5), 1);
        d.note("a", TaskEvent::Progress, at(60), 2);
        let (w, _, r) = d.totals(at(70));
        assert_eq!(
            (w.as_secs() / 60, r),
            (10, 0),
            "only the time since midnight"
        );
    }

    #[test]
    fn the_line_names_only_what_there_is() {
        let m = |s: u64| Duration::from_secs(s * 60);
        assert_eq!(day_text((m(0), m(0), 0)), None);
        assert_eq!(day_text((Duration::from_secs(50), m(0), 0)), None);
        assert_eq!(
            day_text((m(130), m(14), 9)).as_deref(),
            Some("Today  2 h 10 min working · 14 min waiting for you · 9 turns")
        );
        assert_eq!(day_text((m(0), m(0), 1)).as_deref(), Some("Today  1 turn"));
    }
}
