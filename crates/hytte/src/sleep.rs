//! "Stop playing in 30 minutes": a sleep timer for the Media pane.
//!
//! In the last 20 seconds the player's own volume fades down, then it is paused, then its volume
//! is put back so the next song isn't silent. A manual pause doesn't cancel it; closing Hytte
//! does (a timer isn't kept across restarts). If Hytte dies mid-fade, `recover` restores the
//! volume at the next start.

use crate::ui_state::MediaInfo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The fade before the stop.
pub const FADE: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Minutes(u32),
    EndOfTrack,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sleep {
    pub kind: Kind,
    /// When playback stops. `None` for "end of track" while the song's length is unknown.
    pub until: Option<Instant>,
    /// The song this was set for (see `lyrics::key`).
    pub track: String,
    /// The fade has begun.
    pub fading: bool,
    /// Stop now: the song changed under an "end of track" timer.
    pub fire: bool,
}

impl Sleep {
    pub fn new(
        kind: Kind,
        media: Option<&MediaInfo>,
        stamp: Option<Instant>,
        now: Instant,
    ) -> Self {
        let mut s = Sleep {
            kind,
            until: None,
            track: media.map_or(String::new(), |m| crate::lyrics::key(&m.title, &m.artist)),
            fading: false,
            fire: false,
        };
        match kind {
            Kind::Minutes(m) => s.until = Some(now + Duration::from_secs(m as u64 * 60)),
            Kind::EndOfTrack => {
                s.retime(media, stamp.unwrap_or(now), now);
            }
        }
        s
    }

    /// Keep an "end of track" timer in step with the player: it follows seeks and pauses, and a
    /// new song means the old one ended (some players report no length, so this is the only
    /// signal there). Returns true when it is time to stop.
    pub fn retime(&mut self, media: Option<&MediaInfo>, stamp: Instant, now: Instant) -> bool {
        if self.kind != Kind::EndOfTrack {
            return false;
        }
        let Some(m) = media else {
            self.fire = true;
            return true;
        };
        if crate::lyrics::key(&m.title, &m.artist) != self.track {
            self.fire = true;
            return true;
        }
        self.until = (m.playing && m.dur_ms > 0).then(|| {
            let played = m.pos_ms + now.saturating_duration_since(stamp).as_millis() as u64;
            now + Duration::from_millis(m.dur_ms.saturating_sub(played))
        });
        false
    }

    /// What the Media pane says beside the moon: "28m", "45s", "End".
    pub fn label(&self, now: Instant) -> String {
        match (self.kind, self.until) {
            (Kind::EndOfTrack, _) => "End".into(),
            (_, Some(u)) => {
                let s = u.saturating_duration_since(now).as_secs_f32().ceil() as u64;
                if s >= 60 {
                    format!("{}m", s.div_ceil(60))
                } else {
                    format!("{s}s")
                }
            }
            _ => String::new(),
        }
    }

    /// Time to start fading (or the song is changing and there is no time left).
    pub fn fade_due(&self, now: Instant) -> bool {
        self.fire || self.until.is_some_and(|u| now + FADE >= u)
    }

    pub fn done(&self, now: Instant) -> bool {
        self.fire || self.until.is_some_and(|u| now >= u)
    }

    /// Moments the model should wake for: the fade start, the stop, and the next minute of the
    /// label (so "28m" ticks down).
    pub fn deadlines(&self, now: Instant) -> Vec<Instant> {
        let Some(u) = self.until else { return vec![] };
        let mut v = vec![u];
        if let Some(f) = u.checked_sub(FADE).filter(|f| *f > now) {
            v.push(f);
        }
        v.push(now + Duration::from_secs(60).min(u.saturating_duration_since(now)));
        v
    }
}

/// Which audio session is the player? `app` is the media session's app id (an exe name, or
/// something like `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`); `sessions` are
/// (exe, playing). A session whose exe name (without `.exe`) appears in the id, playing first.
pub fn pick_exe(app: &str, sessions: &[(String, bool)]) -> Option<String> {
    let app = app.to_ascii_lowercase();
    let hit = |(exe, _): &&(String, bool)| {
        let stem = exe
            .trim_end_matches(".exe")
            .trim_end_matches(".EXE")
            .to_ascii_lowercase();
        stem.len() >= 3 && app.contains(&stem)
    };
    let mut found: Vec<&(String, bool)> = sessions.iter().filter(hit).collect();
    found.sort_by_key(|(_, playing)| !playing);
    found.first().map(|(e, _)| e.clone())
}

/// The volume `elapsed` seconds into a fade of `total` seconds that starts at `orig`.
pub fn fade_volume(orig: f32, elapsed: f32, total: f32) -> f32 {
    if total <= 0.0 {
        return 0.0;
    }
    (orig * (1.0 - elapsed / total)).clamp(0.0, orig)
}

/// Bumped by every `start_fade` and `cancel`: a fade is cancelled once the counter has moved on
/// from the value it started with, so a cancel can't be undone by the next fade.
static GEN: AtomicU64 = AtomicU64::new(0);

/// A fade in progress stops and gives the volume back (the timer was cancelled).
pub fn cancel() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// The original volume a running (or crashed) fade saved for `exe`.
fn saved_volume(exe: &str) -> Option<f32> {
    let text = std::fs::read_to_string(marker()).ok()?;
    let (e, v) = text.split_once('\n')?;
    e.trim()
        .eq_ignore_ascii_case(exe)
        .then(|| v.trim().parse().ok())?
}

fn marker() -> std::path::PathBuf {
    crate::config::data_dir().join("sleep-restore.txt")
}

/// Fade the player over `secs` seconds, pause it, and put its volume back. `app` is the media
/// session's app id; the audio session is found on the thread. A player that can't be matched
/// is only paused. `secs` of 0 pauses at once.
pub fn start_fade(app: String, secs: u32) {
    let mine = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let cancelled = move || GEN.load(Ordering::SeqCst) != mine;
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            crate::proc::eco_thread();
            // The audio session calls below need COM on this thread.
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                );
            }
        }
        let sessions = crate::mic::apps();
        let list: Vec<(String, bool)> =
            sessions.iter().map(|a| (a.exe.clone(), a.active)).collect();
        let exe = pick_exe(&app, &list);
        // A fade cancelled a moment ago may not have given the volume back yet: trust the saved one.
        // ponytail: its restore may still delete this fade's marker; crash recovery is lost then.
        let orig = exe
            .as_deref()
            .and_then(|e| saved_volume(e).or_else(|| crate::mic::app_volume(e)));
        let target = exe.clone().zip(orig);
        let restore = || {
            if let Some((e, v)) = &target {
                crate::mic::set_app_volume_quiet(e, *v);
                let _ = std::fs::remove_file(marker());
            }
        };
        if let Some((e, v)) = &target {
            let _ = std::fs::write(marker(), format!("{e}\n{v}"));
        }
        for i in 1..=secs {
            std::thread::sleep(Duration::from_secs(1));
            if cancelled() {
                restore();
                return;
            }
            if let Some((e, v)) = &target {
                crate::mic::set_app_volume_quiet(e, fade_volume(*v, i as f32, secs as f32));
            }
        }
        if cancelled() {
            restore();
            return;
        }
        crate::media::control(crate::media::Cmd::Pause);
        std::thread::sleep(Duration::from_millis(900));
        restore();
    });
}

/// At startup: if Hytte died mid-fade, give the player its volume back.
pub fn recover() {
    let Ok(text) = std::fs::read_to_string(marker()) else {
        return;
    };
    // The main thread has no COM yet at startup; the audio session calls need it.
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }
    let restored = text.split_once('\n').is_some_and(|(exe, vol)| {
        vol.trim()
            .parse::<f32>()
            .is_ok_and(|v| crate::mic::set_app_volume_quiet(exe.trim(), v))
    });
    // ponytail: no session yet (player not running) keeps the marker for the next start.
    if restored || text.split_once('\n').is_none() {
        let _ = std::fs::remove_file(marker());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(title: &str, playing: bool, pos: u64, dur: u64) -> MediaInfo {
        MediaInfo {
            title: title.into(),
            artist: "A".into(),
            album: String::new(),
            playing,
            app: "Spotify.exe".into(),
            pos_ms: pos,
            dur_ms: dur,
        }
    }

    const S: Duration = Duration::from_secs(1);

    fn com() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
    }

    #[test]
    fn minutes_count_down_fade_and_stop() {
        let t0 = Instant::now();
        let s = Sleep::new(Kind::Minutes(30), None, None, t0);
        assert_eq!(s.until, Some(t0 + 1800 * S));
        assert_eq!(s.label(t0), "30m");
        assert_eq!(s.label(t0 + 29 * 60 * S), "1m", "a full minute left");
        assert_eq!(s.label(t0 + 29 * 60 * S + 30 * S), "30s");
        assert_eq!(s.label(t0 + 28 * 60 * S + 30 * S), "2m", "rounds up");
        assert_eq!(s.label(t0 + 1800 * S - 45 * S), "45s");
        assert!(!s.fade_due(t0 + 1779 * S));
        assert!(s.fade_due(t0 + 1780 * S));
        assert!(!s.done(t0 + 1799 * S) && s.done(t0 + 1800 * S));
    }

    #[test]
    fn its_deadlines_include_the_fade_the_stop_and_the_next_label_tick() {
        let t0 = Instant::now();
        let s = Sleep::new(Kind::Minutes(30), None, None, t0);
        let d = s.deadlines(t0);
        assert!(d.contains(&(t0 + 1800 * S)) && d.contains(&(t0 + 1780 * S)));
        assert!(d.contains(&(t0 + 60 * S)));
        // In the fade already: just the stop (and the label, no later than the stop).
        let late = s.deadlines(t0 + 1790 * S);
        assert!(!late.contains(&(t0 + 1780 * S)));
        assert_eq!(late.iter().max(), Some(&(t0 + 1800 * S)));
        let none = Sleep::new(Kind::EndOfTrack, None, None, t0);
        assert!(none.deadlines(t0).is_empty());
    }

    #[test]
    fn end_of_track_follows_the_song_and_the_player() {
        let t0 = Instant::now();
        let m = song("One", true, 100_000, 180_000);
        let mut s = Sleep::new(Kind::EndOfTrack, Some(&m), Some(t0), t0);
        assert_eq!(s.until, Some(t0 + 80 * S));
        // Five seconds later (no new report): 75 s to go.
        assert!(!s.retime(Some(&m), t0, t0 + 5 * S));
        assert_eq!(s.until, Some(t0 + 80 * S));
        // A seek towards the end moves it.
        let seeked = song("One", true, 170_000, 180_000);
        s.retime(Some(&seeked), t0 + 6 * S, t0 + 6 * S);
        assert_eq!(s.until, Some(t0 + 16 * S));
        // Paused: no end in sight, but the timer stays.
        let paused = song("One", false, 170_000, 180_000);
        assert!(!s.retime(Some(&paused), t0, t0));
        assert_eq!(s.until, None);
        assert!(!s.fade_due(t0 + 1000 * S));
        // A new song: the old one ended.
        assert!(s.retime(Some(&song("Two", true, 0, 200_000)), t0, t0));
        assert!(s.fire && s.fade_due(t0) && s.done(t0));
        // No length reported: waits for the song to change.
        let nolen = song("One", true, 0, 0);
        let mut s = Sleep::new(Kind::EndOfTrack, Some(&nolen), Some(t0), t0);
        assert_eq!(s.until, None);
        assert!(s.retime(None, t0, t0), "the player went away");
        assert_eq!(s.label(t0), "End");
    }

    #[test]
    fn minutes_ignore_the_song() {
        let t0 = Instant::now();
        let mut s = Sleep::new(
            Kind::Minutes(15),
            Some(&song("One", true, 0, 100)),
            Some(t0),
            t0,
        );
        assert!(!s.retime(Some(&song("Two", true, 0, 100)), t0, t0));
        assert_eq!(s.until, Some(t0 + 900 * S));
    }

    #[test]
    fn the_player_is_found_among_the_audio_sessions() {
        let sessions = vec![
            ("chrome.exe".to_string(), false),
            ("Spotify.exe".to_string(), true),
            ("msedge.exe".to_string(), true),
            ("ab.exe".to_string(), true),
        ];
        let pick = |app: &str| pick_exe(app, &sessions);
        assert_eq!(pick("Spotify.exe").as_deref(), Some("Spotify.exe"));
        assert_eq!(
            pick("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify").as_deref(),
            Some("Spotify.exe")
        );
        assert_eq!(pick("Chrome").as_deref(), Some("chrome.exe"));
        assert_eq!(pick("MSEdge").as_deref(), Some("msedge.exe"));
        assert_eq!(pick("vlc.exe"), None, "unknown: pause without a fade");
        assert_eq!(
            pick("abc"),
            None,
            "a two-letter exe name is too vague to match"
        );
        assert_eq!(pick(""), None);
        // Two sessions of one app family: the one that is playing.
        let two = vec![
            ("chrome.exe".to_string(), false),
            ("chrome.exe".to_string(), true),
        ];
        assert_eq!(pick_exe("Chrome", &two).as_deref(), Some("chrome.exe"));
    }

    /// Manual: fades the real player named by HYTTE_SLEEP_APP (its media app id) for 4 seconds,
    /// pauses it, and prints its volume before and after (it must come back).
    /// `cargo test -p hytte sleep::tests::manual_fade -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn manual_fade() {
        let app = std::env::var("HYTTE_SLEEP_APP").unwrap();
        com();
        let vol = |app: &str| {
            let list = crate::mic::apps();
            let pairs: Vec<(String, bool)> =
                list.iter().map(|a| (a.exe.clone(), a.active)).collect();
            let exe = pick_exe(app, &pairs).unwrap_or_else(|| app.to_string());
            crate::mic::app_volume(&exe).map(|v| (exe, v))
        };
        let before = vol(&app);
        println!("before: {before:?}");
        start_fade(app.clone(), 4);
        std::thread::sleep(Duration::from_millis(2500));
        println!("mid-fade: {:?}", vol(&app));
        std::thread::sleep(Duration::from_secs(5));
        let after = vol(&app);
        println!("after: {after:?}");
        assert_eq!(before.map(|b| b.1), after.map(|a| a.1), "volume comes back");
    }

    /// Manual: prints the audio sessions Hytte can see.
    #[test]
    #[ignore]
    fn manual_sessions() {
        com();
        for a in crate::mic::apps() {
            println!(
                "session: {} ({}) vol={} active={}",
                a.exe, a.name, a.vol, a.active
            );
        }
    }

    #[test]
    fn the_fade_is_a_straight_line_down() {
        assert_eq!(fade_volume(0.8, 0.0, 20.0), 0.8);
        assert!((fade_volume(0.8, 10.0, 20.0) - 0.4).abs() < 1e-6);
        assert_eq!(fade_volume(0.8, 20.0, 20.0), 0.0);
        assert_eq!(fade_volume(0.8, 25.0, 20.0), 0.0);
        assert_eq!(fade_volume(0.5, 3.0, 0.0), 0.0);
    }
}
