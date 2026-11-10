//! The Timer pane and the calendar heads-up.

use super::*;

impl Model {
    /// Move a due timer to the finished card. Returns its kind when it just fired.
    pub fn finish_timer(&mut self, wall_ms: u64, now: Instant) -> Option<crate::timer::TimerKind> {
        if !self
            .timer
            .as_ref()
            .is_some_and(|t| t.remaining_ms(wall_ms) == 0)
        {
            return None;
        }
        let t = self.timer.take()?;
        if t.kind == crate::timer::TimerKind::Focus {
            self.focus_done += 1;
        }
        self.timer_done = Some((t.kind, now));
        self.timer_last = Some(t.clone());
        Some(t.kind)
    }

    /// A new "next event". The heads-up is due `lead` before it starts, or at once if that
    /// moment has passed; an event whose heads-up was already shown isn't armed again.
    pub fn set_calendar(
        &mut self,
        ev: Option<crate::calendar::NextEvent>,
        wall_ms: i64,
        lead_ms: i64,
        now: Instant,
    ) {
        self.calendar = match ev {
            Some(e) if self.calendar_seen.as_ref() != Some(&(e.subject.clone(), e.start_ms)) => {
                let wait = (e.start_ms - lead_ms - wall_ms).max(0) as u64;
                Some((e, now + Duration::from_millis(wait)))
            }
            _ => None,
        };
    }

    /// The event whose heads-up is due now, once; it is then remembered as seen.
    pub fn calendar_due(&mut self, now: Instant) -> Option<crate::calendar::NextEvent> {
        let due = self.calendar.as_ref().is_some_and(|(_, at)| now >= *at);
        if !due {
            return None;
        }
        let (e, _) = self.calendar.take()?;
        self.calendar_seen = Some((e.subject.clone(), e.start_ms));
        Some(e)
    }
}
