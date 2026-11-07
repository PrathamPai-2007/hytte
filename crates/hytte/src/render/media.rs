//! The Music pane: now playing, transport, the sleep timer and the lyrics page.

use super::*;
use crate::lyrics::{current_line, Lyrics};
use crate::sleep::Kind as SleepKind;
use crate::ui_state::{LyricState, MediaInfo};

/// How long the lyrics take to glide to the next line once it starts, in ms.
const LINE_SLIDE_MS: f32 = 520.0;
/// Distance between lyric lines, and the height where the sung one sits.
const LINE_PITCH: f32 = 26.0;
const LINE_MID: f32 = 57.0;

impl Renderer {
    /// The play position now: the player's last report, carried forward while it plays.
    fn media_pos(&self, fr: &Frame, md: &MediaInfo) -> u64 {
        let mut pos = md.pos_ms;
        if md.playing {
            if let Some(s) = fr.model.media_stamp {
                pos += fr.now.saturating_duration_since(s).as_millis() as u64;
            }
        }
        if md.dur_ms > 0 {
            pos = pos.min(md.dur_ms);
        }
        pos
    }

    pub(super) fn exp_media(&self, fr: &Frame, w: f32, h: f32) {
        let Some(md) = fr.model.media.as_ref() else {
            return;
        };
        if fr.model.sleep_open {
            return self.sleep_menu(fr, w);
        }
        if fr.model.lyrics_open {
            return self.lyrics_page(fr, md, w, h);
        }
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
        let pos = self.media_pos(fr, md);
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
        let tab = if fr.model.tabs() { TAB_H } else { 0.0 };
        let cy = h - tab - 26.0;
        let cx = tx + bw / 2.0;
        self.transport(fr, md, cx, cy, 1.0);
        // Lyrics at the left of the controls, the sleep timer at the right.
        self.button(
            "Lyrics",
            tx,
            cy - 12.0,
            54.0,
            24.0,
            self.fg(),
            Action::LyricsOpen,
        );
        self.sleep_button(fr, right, cy);
    }

    /// Previous, play / pause, next, centred on (cx, cy); `k` scales the whole set.
    fn transport(&self, fr: &Frame, md: &MediaInfo, cx: f32, cy: f32, k: f32) {
        let _ = fr;
        for (dir, action) in [(-1.0, Action::MediaPrev), (1.0, Action::MediaNext)] {
            let x = cx + dir * 46.0 * k;
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
        } * k;
        self.circle(cx, cy, r, self.cc(self.fg(), 0.95));
        let dark = self.cc(self.ink(), 1.0);
        if md.playing {
            self.pause_icon(cx, cy, 13.0 * k, dark);
        } else {
            self.play_icon(cx + k, cy, 14.0 * k, dark);
        }
    }

    /// The moon at the right of the transport row, with the time left beside it when set.
    fn sleep_button(&self, fr: &Frame, right: f32, cy: f32) {
        let m = fr.model;
        let label = m.sleep.as_ref().map(|s| s.label(fr.now));
        let wl = if label.is_some() { 36.0 } else { 0.0 };
        let hov = self.hit(
            right - 30.0 - wl,
            cy - 14.0,
            30.0 + wl,
            28.0,
            Action::SleepMenu,
        );
        let on = label.is_some();
        let tone = [0.78, 0.74, 1.0];
        if on || hov {
            self.fill_rr(
                right - 30.0 - wl,
                cy - 13.0,
                30.0 + wl,
                26.0,
                13.0,
                self.cc(tone, if on { 0.20 } else { 0.10 }),
            );
        }
        self.glyph(
            '\u{E708}',
            right - 15.0,
            cy,
            15.0,
            self.cc(
                if on { tone } else { self.fg() },
                if on || hov { 1.0 } else { 0.62 },
            ),
        );
        if let Some(l) = label {
            self.text(
                &l,
                &self.f.btn,
                right - 32.0 - 30.0,
                cy - 8.0,
                32.0,
                16.0,
                self.cc(tone, 1.0),
            );
        }
    }

    /// "Stop playing in": chips for 15, 30, 45, 60 minutes and the end of the song.
    fn sleep_menu(&self, fr: &Frame, w: f32) {
        let m = fr.model;
        let fg = self.fg();
        let tone = [0.78, 0.74, 1.0];
        self.glyph('\u{E708}', 28.0, 26.0, 16.0, self.cc(tone, 1.0));
        self.text(
            "Stop playing in",
            &self.f.title,
            46.0,
            16.0,
            w - 100.0,
            20.0,
            self.cc(fg, 0.97),
        );
        let hov = self.hit(w - 40.0, 10.0, 28.0, 28.0, Action::SleepMenu);
        if hov {
            self.circle(w - 26.0, 24.0, 13.0, self.cc(fg, 0.10));
        }
        self.cross(
            w - 26.0,
            24.0,
            4.5,
            self.cc(fg, if hov { 1.0 } else { 0.6 }),
            1.6,
        );
        let current = m.sleep.as_ref().map(|s| s.kind);
        let mut x = 16.0;
        let chips: [(String, SleepKind, f32); 5] = [
            ("15".into(), SleepKind::Minutes(15), 50.0),
            ("30".into(), SleepKind::Minutes(30), 50.0),
            ("45".into(), SleepKind::Minutes(45), 50.0),
            ("60".into(), SleepKind::Minutes(60), 50.0),
            ("End of track".into(), SleepKind::EndOfTrack, 100.0),
        ];
        for (label, kind, cw) in chips {
            let on = current == Some(kind);
            let hov = self.hit(x, 52.0, cw, 30.0, Action::SleepSet(kind));
            let fill = if on {
                self.cc(self.accent.get(), 0.9)
            } else {
                self.cc(fg, if hov { 0.16 } else { 0.10 })
            };
            self.fill_rr(x, 52.0, cw, 30.0, 15.0, fill);
            let ink = if on {
                self.cc(self.ink(), 1.0)
            } else {
                self.cc(fg, 0.95)
            };
            self.text(&label, &self.f.btn, x, 52.0, cw, 30.0, ink);
            x += cw + 8.0;
        }
        if let Some(s) = &m.sleep {
            let msg = match s.kind {
                SleepKind::EndOfTrack => "Stops when this song ends".to_string(),
                _ => format!("Stops in {}", s.label(fr.now)),
            };
            self.text(
                &msg,
                &self.f.small,
                18.0,
                94.0,
                w - 130.0,
                22.0,
                self.cc(GRAY, 1.0),
            );
            self.button(
                "Cancel timer",
                w - 16.0 - 96.0,
                94.0,
                96.0,
                22.0,
                fg,
                Action::SleepCancel,
            );
        } else {
            self.text(
                "Volume fades out, then it pauses.",
                &self.f.small,
                18.0,
                94.0,
                w - 36.0,
                22.0,
                self.cc(GRAY, 0.9),
            );
        }
    }

    /// "Live": follow the song line by line. A tick box; greyed and dead when this song has no
    /// timed lyrics.
    fn live_toggle(&self, x: f32, cy: f32, enabled: bool, on: bool) {
        let (fg, acc) = (self.fg(), self.accent.get());
        let (bw, bh) = (62.0, 24.0);
        let y = cy - bh / 2.0;
        let hov = enabled && self.hit(x, y, bw, bh, Action::Set("media.lyrics_live", Val::B(!on)));
        if hov {
            self.fill_rr(x, y, bw, bh, 12.0, self.cc(fg, 0.08));
        }
        let a = if enabled { 1.0 } else { 0.35 };
        let (bx, by, s) = (x + 8.0, cy - 7.0, 14.0);
        if enabled && on {
            self.fill_rr(bx, by, s, s, 4.0, self.cc(acc, 0.95));
            let ink = self.cc(self.ink(), 0.95);
            self.line((bx + 3.5, by + 7.5), (bx + 6.0, by + 10.0), ink, 1.8);
            self.line((bx + 6.0, by + 10.0), (bx + 10.8, by + 4.5), ink, 1.8);
        } else {
            self.stroke_rr(bx, by, s, s, 4.0, self.cc(fg, 0.55 * a), 1.3, false);
        }
        self.text(
            "Live",
            &self.f.btn,
            x + 28.0,
            y,
            bw - 28.0,
            bh,
            self.cc(fg, (if hov { 1.0 } else { 0.86 }) * a),
        );
    }

    /// Three lines of lyrics, the one being sung in the middle, over small controls.
    fn lyrics_page(&self, fr: &Frame, md: &MediaInfo, w: f32, h: f32) {
        let m = fr.model;
        let fg = self.fg();
        let hov = self.hit(10.0, 4.0, 30.0, 28.0, Action::LyricsClose);
        if hov {
            self.circle(24.0, 18.0, 13.0, self.cc(fg, 0.10));
        }
        self.glyph(
            '\u{E72B}',
            24.0,
            18.0,
            13.0,
            self.cc(fg, if hov { 1.0 } else { 0.7 }),
        );
        let title = if md.artist.is_empty() {
            md.title.clone()
        } else {
            format!("{} · {}", md.title, md.artist)
        };
        self.text(
            &title,
            &self.f.small_c,
            48.0,
            8.0,
            w - 96.0,
            20.0,
            self.cc(GRAY, 1.0),
        );
        let tab = if m.tabs() { TAB_H } else { 0.0 };
        let cy = h - tab - 20.0;
        self.transport(fr, md, w / 2.0, cy, 0.8);
        self.sleep_button(fr, w - 16.0, cy);
        // Only timed lyrics can follow the song: for any other song the box is disabled.
        let timed = m.lyrics_on && matches!(&m.lyric_state, LyricState::Ready(Lyrics::Synced(_)));
        self.live_toggle(16.0, cy, timed, !m.lyrics_still);
        let note = |text: &str, sub: &str| {
            self.text(text, &self.f.big_c, 0.0, 28.0, w, 24.0, self.cc(fg, 0.9));
            if !sub.is_empty() {
                self.text(
                    sub,
                    &self.f.small_c,
                    24.0,
                    52.0,
                    w - 48.0,
                    18.0,
                    self.cc(GRAY, 1.0),
                );
            }
        };
        if !m.lyrics_on {
            note(
                "Lyrics are off",
                "Looking them up sends the song and artist to lrclib.net.",
            );
            self.button(
                "Turn on",
                w / 2.0 - 40.0,
                74.0,
                80.0,
                22.0,
                self.accent.get(),
                Action::Set("media.lyrics", Val::B(true)),
            );
            return;
        }
        match &m.lyric_state {
            LyricState::Idle | LyricState::Loading => note("Searching…", ""),
            LyricState::Failed => {
                note("Can't reach the lyrics service", "");
                self.button(
                    "Try again",
                    w / 2.0 - 40.0,
                    60.0,
                    80.0,
                    22.0,
                    self.accent.get(),
                    Action::LyricsRetry,
                );
            }
            LyricState::Skipped => note(
                "Not for this app",
                "Lyrics aren't looked up for video and photo players.",
            ),
            LyricState::Ready(Lyrics::NotFound) => note("No lyrics found", ""),
            LyricState::Ready(Lyrics::Instrumental) => note("Instrumental", ""),
            LyricState::Ready(Lyrics::Synced(lines)) if !m.lyrics_still => {
                let pos = self.media_pos(fr, md);
                // Where the list is scrolled to, in lines: it glides from the line before to
                // the one being sung just after that one starts. Before the first line it
                // waits one line short, so the opening line rises into place.
                let at = match current_line(lines, pos) {
                    Some(i) => {
                        let t = ((pos - lines[i].at_ms) as f32 / LINE_SLIDE_MS).clamp(0.0, 1.0);
                        i as f32 - 1.0 + t * t * (3.0 - 2.0 * t)
                    }
                    None => -1.0,
                };
                let first = (at.floor() as i32 - 1).max(0);
                for k in first..=(at.floor() as i32 + 3) {
                    let Some(l) = lines.get(k as usize) else {
                        break;
                    };
                    let d = k as f32 - at;
                    // 1 on the line being sung, 0 a whole line away.
                    let e = (1.0 - d.abs()).clamp(0.0, 1.0);
                    let e = e * e * (3.0 - 2.0 * e);
                    // Lines further than a line and a half melt away before the edges.
                    let reach = (1.0 - (d.abs() - 1.0).max(0.0) / 0.6).clamp(0.0, 1.0);
                    let text = if l.text.is_empty() {
                        "♪"
                    } else {
                        l.text.as_str()
                    };
                    let cy = LINE_MID + d * LINE_PITCH;
                    let (small, big) = (0.55 * (1.0 - e) * reach, e);
                    if small > 0.02 {
                        self.text(
                            text,
                            &self.f.center,
                            24.0,
                            cy - 8.0,
                            w - 48.0,
                            16.0,
                            self.cc(GRAY, small),
                        );
                    }
                    if big > 0.02 {
                        self.text(
                            text,
                            &self.f.lyric,
                            24.0,
                            cy - 20.0,
                            w - 48.0,
                            40.0,
                            self.cc(fg, big),
                        );
                    }
                }
            }
            LyricState::Ready(Lyrics::Plain(_) | Lyrics::Synced(_)) => {
                let words: Vec<&str> = match &m.lyric_state {
                    LyricState::Ready(Lyrics::Plain(l)) => l.iter().map(String::as_str).collect(),
                    LyricState::Ready(Lyrics::Synced(l)) => {
                        l.iter().map(|l| l.text.as_str()).collect()
                    }
                    _ => vec![],
                };
                let first = m.lyric_first.min(words.len().saturating_sub(1));
                for (i, y) in [(0usize, 28.0f32), (1, 48.0), (2, 68.0)] {
                    let t = words.get(first + i).copied().unwrap_or("");
                    let a = if i == 0 { 0.95 } else { 0.7 };
                    self.text(t, &self.f.center, 24.0, y, w - 48.0, 20.0, self.cc(fg, a));
                }
            }
        }
    }
}
