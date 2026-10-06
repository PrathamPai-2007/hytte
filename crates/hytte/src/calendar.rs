//! Next calendar event: a heads-up a few minutes before it starts, with a Join button when it
//! has a meeting link.
//!
//! Reads the Windows appointment store (the calendars of the Calendar / Outlook apps) read-only.
//! Nothing is polled: the worker sleeps until the store reports a change or until the next
//! event is due, whichever is first. Opt-in (`[calendar] enabled`), because event titles are
//! private and the pill is shown on screen.

use std::time::Duration;

/// 100 ns ticks between 1601-01-01 (WinRT's epoch) and 1970-01-01.
const EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// WinRT `DateTime` ticks to Unix milliseconds.
pub fn unix_ms(ticks: i64) -> i64 {
    (ticks - EPOCH_TICKS) / 10_000
}

/// An appointment as the worker reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub subject: String,
    pub location: String,
    pub details: String,
    pub online_link: String,
    pub start_ms: i64,
    pub all_day: bool,
    pub canceled: bool,
}

/// The one event the pill cares about.
#[derive(Debug, Clone, PartialEq)]
pub struct NextEvent {
    pub subject: String,
    pub start_ms: i64,
    pub url: Option<String>,
}

/// Hosts whose links join a meeting.
const MEETING_HOSTS: [&str; 6] = [
    "teams.microsoft.com",
    "teams.live.com",
    "meet.google.com",
    "zoom.us",
    "webex.com",
    "whereby.com",
];

/// The first meeting link in the given texts (the provider's own link field first), if any.
/// Only `https://` links are returned: the result is handed to the shell.
pub fn meeting_url(texts: &[&str]) -> Option<String> {
    for text in texts {
        for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '(' | ')')) {
            let Some(rest) = word.strip_prefix("https://") else { continue };
            let host = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = host.to_ascii_lowercase();
            if MEETING_HOSTS
                .iter()
                .any(|h| host == *h || host.ends_with(&format!(".{h}")))
            {
                return Some(word.trim_end_matches(['.', ',', ';']).to_string());
            }
        }
    }
    None
}

/// The earliest event that is still worth a heads-up: starts no more than `grace_ms` ago, isn't
/// all-day or cancelled, and has a title.
pub fn pick_next(appts: &[Raw], now_ms: i64, grace_ms: i64) -> Option<NextEvent> {
    appts
        .iter()
        .filter(|a| !a.all_day && !a.canceled && !a.subject.trim().is_empty())
        .filter(|a| a.start_ms + grace_ms >= now_ms)
        .min_by_key(|a| a.start_ms)
        .map(|a| NextEvent {
            subject: a.subject.trim().to_string(),
            start_ms: a.start_ms,
            url: meeting_url(&[&a.online_link, &a.location, &a.details]),
        })
}

/// How long the worker may sleep: until just after the event starts (so the next one is picked
/// up), capped so a changed day or clock is noticed.
pub fn sleep_for(next: Option<&NextEvent>, now_ms: i64) -> Duration {
    const MAX: Duration = Duration::from_secs(30 * 60);
    match next {
        Some(e) => {
            let ms = (e.start_ms - now_ms + 60_000).max(1_000) as u64;
            Duration::from_millis(ms).min(MAX)
        }
        None => MAX,
    }
}

/// "Standup starts in 9 min" / "Standup is starting" / "Standup started 1 min ago".
pub fn summary(subject: &str, start_ms: i64, now_ms: i64) -> String {
    let mins = (start_ms - now_ms + 30_000).div_euclid(60_000);
    match mins {
        m if m >= 2 => format!("{subject} starts in {m} min"),
        1 => format!("{subject} starts in 1 min"),
        0 => format!("{subject} is starting"),
        m => format!("{subject} started {} min ago", -m),
    }
}

#[cfg(windows)]
pub fn spawn_watcher(
    cfg: crate::config::Calendar,
    ui_tx: crossbeam_channel::Sender<crate::ui_state::UiEvent>,
) {
    if !cfg.enabled {
        return;
    }
    std::thread::spawn(move || {
        crate::proc::eco_thread();
        win::watch(ui_tx);
    });
}

#[cfg(not(windows))]
pub fn spawn_watcher(
    _cfg: crate::config::Calendar,
    _ui_tx: crossbeam_channel::Sender<crate::ui_state::UiEvent>,
) {
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::ui_state::UiEvent;
    use crate::winrt::block_on;
    use crossbeam_channel::{bounded, RecvTimeoutError, Sender};
    use windows::ApplicationModel::Appointments::{
        Appointment, AppointmentManager, AppointmentProperties, AppointmentStore,
        AppointmentStoreAccessType, FindAppointmentsOptions,
    };
    use windows::Foundation::{DateTime, TimeSpan, TypedEventHandler};
    use windows::core::HSTRING;

    const T: Duration = Duration::from_secs(10);
    /// Look this far ahead.
    const HORIZON_MS: i64 = 24 * 3600 * 1000;

    pub fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    }

    pub fn read(store: &AppointmentStore) -> Option<Vec<Raw>> {
        let options = FindAppointmentsOptions::new().ok()?;
        let props = options.FetchProperties().ok()?;
        for p in AppointmentProperties::DefaultProperties().ok()? {
            let _ = props.Append(&p);
        }
        // The property names are the store's own strings, not the plain words.
        for extra in [
            AppointmentProperties::Details(),
            AppointmentProperties::OnlineMeetingLink(),
            AppointmentProperties::IsCanceledMeeting(),
            AppointmentProperties::AllDay(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = props.Append(&extra);
        }
        options.SetMaxCount(30).ok()?;
        // Ask from a bit in the past so an event that just started is still seen.
        let from = DateTime {
            UniversalTime: (now_ms() - 5 * 60_000) * 10_000 + EPOCH_TICKS,
        };
        let range = TimeSpan {
            Duration: (HORIZON_MS + 5 * 60_000) * 10_000,
        };
        let found = block_on(store.FindAppointmentsAsyncWithOptions(from, range, &options).ok()?, T)?.ok()?;
        let mut out = vec![];
        for a in found {
            out.push(raw(&a));
        }
        Some(out)
    }

    fn raw(a: &Appointment) -> Raw {
        let s = |r: windows::core::Result<HSTRING>| r.map(|h| h.to_string()).unwrap_or_default();
        Raw {
            subject: s(a.Subject()),
            location: s(a.Location()),
            details: s(a.Details()),
            online_link: s(a.OnlineMeetingLink()),
            start_ms: a.StartTime().map_or(0, |d| unix_ms(d.UniversalTime)),
            all_day: a.AllDay().unwrap_or(false),
            canceled: a.IsCanceledMeeting().unwrap_or(false),
        }
    }

    pub fn watch(ui_tx: Sender<UiEvent>) {
        let Some(op) = AppointmentManager::RequestStoreAsync(AppointmentStoreAccessType::AllCalendarsReadOnly).ok() else {
            return;
        };
        let Some(Ok(store)) = block_on(op, T) else { return };
        // A change in any calendar wakes the worker; a full queue already means "re-read".
        let (tx, rx) = bounded::<()>(1);
        let _token = store.StoreChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.try_send(());
            Ok(())
        }));
        let mut last: Option<Option<NextEvent>> = None;
        loop {
            let appts = read(&store).unwrap_or_default();
            let now = now_ms();
            let next = pick_next(&appts, now, 2 * 60_000);
            if last.as_ref() != Some(&next) {
                last = Some(next.clone());
                let _ = ui_tx.send(UiEvent::Calendar(next.clone()));
            }
            match rx.recv_timeout(sleep_for(next.as_ref(), now)) {
                Err(RecvTimeoutError::Disconnected) => return,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appt(subject: &str, start_ms: i64) -> Raw {
        Raw {
            subject: subject.into(),
            location: String::new(),
            details: String::new(),
            online_link: String::new(),
            start_ms,
            all_day: false,
            canceled: false,
        }
    }

    #[test]
    fn winrt_ticks_to_unix_ms() {
        assert_eq!(unix_ms(EPOCH_TICKS), 0);
        assert_eq!(unix_ms(EPOCH_TICKS + 1_500 * 10_000), 1_500);
    }

    #[test]
    fn meeting_links() {
        let teams = "Join: <https://teams.microsoft.com/l/meetup-join/19%3a123?x=1>.";
        assert_eq!(
            meeting_url(&[teams]).as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/19%3a123?x=1")
        );
        assert_eq!(
            meeting_url(&["", "https://us02web.zoom.us/j/123456?pwd=abc, see you"]).as_deref(),
            Some("https://us02web.zoom.us/j/123456?pwd=abc")
        );
        assert_eq!(meeting_url(&["https://meet.google.com/abc-defg-hij"]).as_deref(), Some("https://meet.google.com/abc-defg-hij"));
        // The provider's link field wins over text later in the list.
        assert_eq!(
            meeting_url(&["https://zoom.us/j/1", "https://meet.google.com/x"]).as_deref(),
            Some("https://zoom.us/j/1")
        );
        // Not meeting hosts, not https, or a lookalike host.
        assert_eq!(meeting_url(&["https://example.com/zoom.us"]), None);
        assert_eq!(meeting_url(&["http://zoom.us/j/1"]), None);
        assert_eq!(meeting_url(&["https://notzoom.us/j/1"]), None);
        assert_eq!(meeting_url(&["javascript:alert(1)"]), None);
    }

    #[test]
    fn picks_the_earliest_worthwhile_event() {
        let now = 1_000_000_000;
        let mut allday = appt("Holiday", now + 1_000);
        allday.all_day = true;
        let mut gone = appt("Cancelled", now + 2_000);
        gone.canceled = true;
        let list = [
            appt("Later", now + 3_600_000),
            allday,
            gone,
            appt("Long over", now - 600_000),
            appt("Just started", now - 60_000),
            appt("  ", now + 500),
        ];
        let n = pick_next(&list, now, 120_000).unwrap();
        assert_eq!(n.subject, "Just started");
        // Past the grace period it moves on.
        let n = pick_next(&list, now + 90_000, 120_000);
        assert_eq!(n.unwrap().subject, "Later");
        assert_eq!(pick_next(&[], now, 120_000), None);
    }

    #[test]
    fn link_comes_from_any_field() {
        let mut a = appt("Standup", 5_000_000_000);
        a.details = "Dial in at https://webex.com/meet/me".into();
        assert_eq!(
            pick_next(&[a], 0, 0).unwrap().url.as_deref(),
            Some("https://webex.com/meet/me")
        );
    }

    #[test]
    fn sleeping_until_the_event_is_over_but_never_too_long() {
        let now = 0;
        let ev = |start| NextEvent { subject: "x".into(), start_ms: start, url: None };
        assert_eq!(sleep_for(Some(&ev(5 * 60_000)), now), Duration::from_secs(6 * 60));
        assert_eq!(sleep_for(Some(&ev(5 * 3_600_000)), now), Duration::from_secs(30 * 60));
        assert_eq!(sleep_for(None, now), Duration::from_secs(30 * 60));
        // An event already underway: re-read in a second, not never.
        assert!(sleep_for(Some(&ev(-3_600_000)), now) >= Duration::from_secs(1));
    }

    #[test]
    fn summaries() {
        let t = 10 * 60_000;
        assert_eq!(summary("Standup", t, 0), "Standup starts in 10 min");
        assert_eq!(summary("Standup", t, t - 60_000), "Standup starts in 1 min");
        assert_eq!(summary("Standup", t, t), "Standup is starting");
        assert_eq!(summary("Standup", t, t + 2 * 60_000), "Standup started 2 min ago");
    }

    /// Manual helpers for checking the worker against a real store: create a throw-away app
    /// calendar with an event a few minutes away and a Teams link, then delete it.
    /// `cargo test -p hytte calendar_manual -- --ignored --nocapture`
    #[cfg(windows)]
    mod manual {
        use super::*;
        use crate::winrt::block_on;
        use windows::ApplicationModel::Appointments::{
            Appointment, AppointmentManager, AppointmentStoreAccessType,
        };
        use windows::core::HSTRING;
        use windows::Foundation::{DateTime, TimeSpan};

        const NAME: &str = "HytteTestCalendar";
        const T: Duration = Duration::from_secs(10);

        #[test]
        #[ignore]
        fn calendar_manual_create() {
            let store = block_on(
                AppointmentManager::RequestStoreAsync(AppointmentStoreAccessType::AppCalendarsReadWrite).unwrap(),
                T,
            )
            .unwrap()
            .unwrap();
            let cal = block_on(store.CreateAppointmentCalendarAsync(&HSTRING::from(NAME)).unwrap(), T)
                .unwrap()
                .unwrap();
            let a = Appointment::new().unwrap();
            a.SetSubject(&HSTRING::from("Design review")).unwrap();
            a.SetDetails(&HSTRING::from("Join https://teams.microsoft.com/l/meetup-join/abc123")).unwrap();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64;
            a.SetStartTime(DateTime { UniversalTime: (now + 4 * 60_000) * 10_000 + EPOCH_TICKS }).unwrap();
            a.SetDuration(TimeSpan { Duration: 30 * 60 * 10_000_000 }).unwrap();
            block_on(cal.SaveAppointmentAsync(&a).unwrap(), T).unwrap().unwrap();
            println!("created a test event starting in 4 minutes");
        }

        #[test]
        #[ignore]
        fn calendar_manual_dump() {
            let store = block_on(
                AppointmentManager::RequestStoreAsync(AppointmentStoreAccessType::AllCalendarsReadOnly).unwrap(),
                T,
            )
            .unwrap()
            .unwrap();
            for r in super::super::win::read(&store).unwrap_or_default() {
                println!("{r:?}");
            }
        }

        #[test]
        #[ignore]
        fn calendar_manual_delete() {
            let store = block_on(
                AppointmentManager::RequestStoreAsync(AppointmentStoreAccessType::AllCalendarsReadWrite).unwrap(),
                T,
            )
            .unwrap()
            .unwrap();
            let cals = block_on(store.FindAppointmentCalendarsAsync().unwrap(), T).unwrap().unwrap();
            for c in cals {
                let name = c.DisplayName().map(|n| n.to_string()).unwrap_or_default();
                println!("calendar: {name:?}");
                if name.contains("HytteTest") {
                    block_on(c.DeleteAsync().unwrap(), T).unwrap().unwrap();
                    println!("deleted the test calendar");
                }
            }
        }
    }
}
