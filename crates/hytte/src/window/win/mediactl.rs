//! Media extras on the UI thread: the sleep timer's fade and stop, and lyrics lookups.

use super::*;

impl Ui {
    /// Start the fade when the sleep timer's last 20 seconds begin; drop the timer once it has
    /// run out. The fade itself (volume steps, the pause, giving the volume back) runs on its
    /// own thread.
    pub(super) fn sleep_tick(&mut self, now: Instant) {
        let app = self
            .model
            .media
            .as_ref()
            .map(|m| m.app.clone())
            .unwrap_or_default();
        let Some(s) = self.model.sleep.as_mut() else {
            return;
        };
        if s.fade_due(now) && !s.fading {
            s.fading = true;
            let secs = if s.fire {
                0
            } else {
                s.until.map_or(0, |u| {
                    u.saturating_duration_since(now)
                        .as_secs()
                        .min(crate::sleep::FADE.as_secs()) as u32
                })
            };
            crate::sleep::start_fade(app, secs);
        }
        if s.done(now) {
            self.model.sleep = None;
        }
    }

    /// Look the playing song's lyrics up when the lyrics page is open and they aren't known.
    pub(super) fn lyrics_tick(&mut self) {
        let Some((key, title, artist, album, dur)) = self.model.lyrics_request() else {
            return;
        };
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            let r = crate::lyrics::lookup(&title, &artist, &album, dur);
            push_event(&sh, UiEvent::Lyrics(key, r));
        });
    }
}
