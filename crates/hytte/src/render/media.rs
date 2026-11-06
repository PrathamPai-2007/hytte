//! The Music pane.

use super::*;

impl Renderer {
    pub(super) fn exp_media(&self, fr: &Frame, w: f32, h: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        let (a, saved) = self.hero(
            [8.0, 6.0, 20.0, 5.0],
            [16.0, 16.0, 72.0, 12.0],
            [16.0, 16.0, 72.0, 12.0],
        );
        self.art_tile(a[0], a[1], a[2], a[3]);
        self.hero_end(saved);
        let tx = 102.0;
        let right = w - 16.0;
        // Art + title open the playing app.
        if !md.app.is_empty() {
            let hov = self.hit(16.0, 12.0, 72.0, 76.0, Action::FocusMedia(md.app.clone()))
                | self.hit(
                    tx,
                    12.0,
                    right - tx - 34.0,
                    48.0,
                    Action::FocusMedia(md.app.clone()),
                );
            if hov {
                self.fill_rr(16.0, 16.0, 72.0, 72.0, 12.0, self.cc(self.fg(), 0.10));
            }
        }
        let hov = self.over(tx, 12.0, right - tx - 34.0, 48.0);
        self.label(
            &md.title,
            &self.f.big,
            tx,
            14.0,
            right - tx - 34.0,
            22.0,
            self.cc(self.fg(), 0.97),
            hov,
        );
        self.label(
            &md.artist,
            &self.f.body,
            tx,
            37.0,
            right - tx - 34.0,
            18.0,
            self.cc(GRAY, 1.0),
            hov,
        );
        self.eq_bars(right - 18.0, 18.0, 14.0, md.playing, GREEN);
        // Timeline with live extrapolation while playing.
        let mut pos = md.pos_ms;
        if md.playing {
            if let Some(s) = fr.model.media_stamp {
                pos += fr.now.saturating_duration_since(s).as_millis() as u64;
            }
        }
        if md.dur_ms > 0 {
            pos = pos.min(md.dur_ms);
        }
        let frac = if md.dur_ms > 0 {
            pos as f32 / md.dur_ms as f32
        } else {
            0.0
        };
        let bx = tx;
        let bw = right - tx;
        // The album's own colour when adaptive glow is on, plain ink otherwise.
        let acc = fr
            .model
            .art_accent
            .filter(|_| fr.model.adaptive_glow && !self.light.get())
            .unwrap_or(self.fg());
        // Seekable: a press anywhere on it (a little taller than the bar) jumps there.
        let hov = md.dur_ms > 0 && self.hit(bx - 4.0, 58.0, bw + 8.0, 19.0, Action::Seek);
        let (bh, by) = if hov { (7.0, 64.0) } else { (5.0, 65.0) };
        self.fill_rr(bx, by, bw, bh, bh / 2.0, self.cc(self.fg(), 0.12));
        if md.dur_ms > 0 {
            let fw = (bw * frac).max(bh);
            self.fill_rr(bx, by, fw, bh, bh / 2.0, self.cc(acc, 0.92));
            if md.playing || hov {
                let r = if hov { 6.5 } else { 5.0 };
                self.circle(
                    bx + fw - bh / 2.0,
                    by + bh / 2.0,
                    r,
                    self.cc(self.fg(), 1.0),
                );
            }
        }
        self.text(
            &fmt_clock(pos),
            &self.f.small,
            bx,
            73.0,
            50.0,
            14.0,
            self.cc(GRAY, 1.0),
        );
        self.text(
            &fmt_clock(md.dur_ms),
            &self.f.small_r,
            right - 50.0,
            73.0,
            50.0,
            14.0,
            self.cc(GRAY, 1.0),
        );
        // Transport controls, clear of the tab dots.
        let cx = tx + bw / 2.0;
        let tab = if fr.model.tabs() { TAB_H } else { 0.0 };
        let cy = h - tab - 26.0;
        for (dir, action) in [(-1.0, Action::MediaPrev), (1.0, Action::MediaNext)] {
            let x = cx + dir * 46.0;
            let hov = self.hit(x - 16.0, cy - 16.0, 32.0, 32.0, action);
            if hov {
                self.circle(x, cy, 16.0, self.cc(self.fg(), 0.09));
            }
            self.skip_icon(x, cy, dir, self.cc(self.fg(), if hov { 1.0 } else { 0.8 }));
        }
        let hov = self.hit(cx - 18.0, cy - 18.0, 36.0, 36.0, Action::MediaToggle);
        let down = hov && self.pressed.get();
        let r = if down {
            16.0
        } else if hov {
            18.0
        } else {
            17.0
        };
        self.circle(cx, cy, r, self.cc(self.fg(), 0.95));
        let dark = self.cc(self.ink(), 1.0);
        if md.playing {
            self.pause_icon(cx, cy, 13.0, dark);
        } else {
            self.play_icon(cx + 1.0, cy, 14.0, dark);
        }
    }
}
