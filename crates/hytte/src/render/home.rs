//! The Quick controls (Home) and Power panes.

use super::*;

impl Renderer {
    /// Quick controls: microphone, sound, keep awake and the timer as tiles, the speaker
    /// volume as a bar under them.
    pub(super) fn exp_home(&self, fr: &Frame, w: f32, _h: f32) {
        use crate::ui_state::Awake;
        let m = fr.model;
        let now = fr.now;
        let (gap, y, th) = (8.0, 12.0, 62.0);
        let tw = (w - 32.0 - gap * 3.0) / 4.0;
        let x = |i: f32| 16.0 + i * (tw + gap);
        let (mic_icon, mic_sub, mic_on) = if m.mic_muted {
            ('\u{F781}', "Muted", Some(RED))
        } else if m.mic {
            ('\u{E720}', "In use", Some(CAM))
        } else {
            ('\u{E720}', "On", None)
        };
        self.tile(
            x(0.0),
            y,
            tw,
            th,
            mic_icon,
            "Mic",
            mic_sub,
            mic_on,
            Action::ToggleMic,
        );
        let (snd_icon, snd_sub, snd_on) = match m.speaker {
            Some((_, true)) => ('\u{E74F}', "Muted".to_string(), Some(AMBER)),
            Some((v, false)) => (volume_glyph(v), format!("{}%", (v * 100.0).round()), None),
            None => ('\u{E74F}', "No output".to_string(), None),
        };
        self.tile(
            x(1.0),
            y,
            tw,
            th,
            snd_icon,
            "Sound",
            &snd_sub,
            snd_on,
            Action::ToggleSpeaker,
        );
        let awake_sub = match m.awake {
            Awake::Off => "Off".to_string(),
            Awake::On => "On".to_string(),
            Awake::Until(t) => {
                let mins = t.saturating_duration_since(now).as_secs().div_ceil(60);
                format!("{mins} min")
            }
        };
        let awake_on = m.awake.active(now).then_some(ORANGE);
        self.tile(
            x(2.0),
            y,
            tw,
            th,
            '\u{E890}',
            "Awake",
            &awake_sub,
            awake_on,
            Action::CycleAwake,
        );
        let wall = crate::timer::now_ms();
        let (timer_sub, timer_on) = match &m.timer {
            Some(t) => (crate::timer::fmt(t.remaining_ms(wall)), Some(BLUE)),
            None => ("Start".to_string(), None),
        };
        self.tile(
            x(3.0),
            y,
            tw,
            th,
            '\u{E916}',
            "Timer",
            &timer_sub,
            timer_on,
            Action::OpenTimer,
        );

        // Volume bar: click to set, scroll to nudge. The per-app chevron and the settings
        // gear sit at its end.
        let (bx, by, bw, bh) = (16.0, y + th + 12.0, w - 32.0 - 76.0, 32.0);
        let chev = if m.apps_open { '\u{E70E}' } else { '\u{E70D}' };
        self.round_btn(
            w - 68.0,
            by + bh / 2.0,
            16.0,
            self.fg(),
            Action::ToggleApps,
            |cx, cy, c| self.glyph(chev, cx, cy, 14.0, c),
        );
        if m.apps_open {
            self.app_rows(m, by + bh + 8.0, w);
        }
        self.round_btn(
            w - 32.0,
            by + bh / 2.0,
            16.0,
            self.fg(),
            Action::Settings(Some(0)),
            |cx, cy, c| self.glyph('\u{E713}', cx, cy, 16.0, c),
        );
        let hov = self.hit(bx, by, bw, bh, Action::Volume);
        self.fill_rr(
            bx,
            by,
            bw,
            bh,
            bh / 2.0,
            self.cc(self.fg(), if hov { 0.10 } else { 0.06 }),
        );
        match m.speaker {
            Some((vol, muted)) => {
                let fw = (bw * vol).max(bh);
                let (fill, a) = if muted {
                    (GRAY, 0.30)
                } else {
                    (self.fg(), 0.9)
                };
                self.fill_rr(bx, by, fw, bh, bh / 2.0, self.cc(fill, a));
                let icon = if muted { '\u{E74F}' } else { volume_glyph(vol) };
                let on_fill = if muted { self.fg() } else { self.ink() };
                self.glyph(icon, bx + 17.0, by + bh / 2.0, 20.0, self.cc(on_fill, 0.9));
                // Hovered: the exact level at the end of the bar (inside the fill when it
                // reaches that far, so it takes the fill's ink).
                if hov {
                    let tx = bx + bw - 54.0;
                    let ink = if fw > bw - 14.0 && !muted {
                        self.ink()
                    } else {
                        self.fg()
                    };
                    let pct = format!("{}%", (vol * 100.0).round());
                    self.text(&pct, &self.f.small_r, tx, by, 42.0, bh, self.cc(ink, 0.8));
                }
            }
            None => {
                self.text(
                    "No output device",
                    &self.f.small_c,
                    bx,
                    by,
                    bw,
                    bh,
                    self.cc(GRAY, 0.9),
                );
            }
        }
    }

    /// The per-app volume list: a name and a thin bar per app, or a note when none plays.
    pub(super) fn app_rows(&self, m: &crate::ui_state::Model, y: f32, w: f32) {
        use crate::ui_state::APP_ROW_H;
        let fg = self.fg();
        if m.apps.is_empty() {
            let note = "No apps are using the speaker";
            self.text(
                note,
                &self.f.small_c,
                16.0,
                y,
                w - 32.0,
                APP_ROW_H,
                self.cc(GRAY, 0.9),
            );
            return;
        }
        let (bx, bw, bh) = (120.0, w - 136.0, 18.0);
        for (i, a) in m.apps.iter().enumerate() {
            let ry = y + i as f32 * APP_ROW_H;
            let label = a.name.clone();
            let alpha = if a.active { 0.9 } else { 0.6 };
            self.text(
                &label,
                &self.f.small_l,
                20.0,
                ry,
                bx - 28.0,
                APP_ROW_H,
                self.cc(fg, alpha),
            );
            let by = ry + (APP_ROW_H - bh) / 2.0;
            // The hit box is the full row height, so a thin bar is still easy to grab.
            let hov = self.hit(bx, ry, bw, APP_ROW_H, Action::AppVolume(a.exe.clone()));
            self.fill_rr(
                bx,
                by,
                bw,
                bh,
                bh / 2.0,
                self.cc(fg, if hov { 0.10 } else { 0.06 }),
            );
            let (fill, fa) = if a.muted { (GRAY, 0.30) } else { (fg, 0.8) };
            self.fill_rr(
                bx,
                by,
                (bw * a.vol).max(bh),
                bh,
                bh / 2.0,
                self.cc(fill, fa),
            );
            if hov {
                let pct = format!("{}%", (a.vol * 100.0).round());
                let ink = if a.vol > 0.85 && !a.muted {
                    self.ink()
                } else {
                    fg
                };
                self.text(
                    &pct,
                    &self.f.small_r,
                    bx + bw - 50.0,
                    by,
                    42.0,
                    bh,
                    self.cc(ink, 0.8),
                );
            }
        }
    }

    /// Battery and power mode.
    pub(super) fn exp_power(&self, fr: &Frame, w: f32, _h: f32) {
        let Some(b) = fr.model.power else { return };
        let low = b.pct <= 20 && !b.plugged;
        let col = if low {
            RED
        } else if b.plugged {
            GREEN
        } else {
            self.fg()
        };
        // A big battery: outline, nub, charge, and a bolt while plugged in.
        let (ix, iy, iw, ih) = (22.0, 22.0, 40.0, 21.0);
        self.stroke_rr(ix, iy, iw, ih, 6.0, self.cc(self.fg(), 0.55), 1.6, false);
        self.fill_rr(
            ix + iw + 2.0,
            iy + 6.5,
            3.0,
            ih - 13.0,
            1.4,
            self.cc(self.fg(), 0.55),
        );
        let fw = ((iw - 6.0) * b.pct as f32 / 100.0).max(3.0);
        self.fill_rr(ix + 3.0, iy + 3.0, fw, ih - 6.0, 3.5, self.cc(col, 0.95));
        if b.plugged {
            let (cx, cy) = (ix + iw / 2.0, iy + ih / 2.0);
            self.bolt(cx, cy, 1.0, self.cc(self.ink(), 0.9));
        }
        let pct = format!("{}%", b.pct);
        self.text(
            &pct,
            &self.f.hero,
            80.0,
            10.0,
            120.0,
            34.0,
            self.cc(self.fg(), 0.97),
        );
        let sub = match b.left() {
            Some(l) => format!("{}  ·  {l}", capitalise(&b.flow())),
            None => capitalise(&b.flow()),
        };
        let sc = if low { RED } else { GRAY };
        self.text(
            &sub,
            &self.f.small,
            81.0,
            41.0,
            w - 97.0,
            16.0,
            self.cc(sc, 1.0),
        );

        // Power mode: one segmented control.
        let (sx, sy, sw, sh) = (16.0, 68.0, w - 32.0, 32.0);
        self.fill_rr(sx, sy, sw, sh, sh / 2.0, self.cc(self.fg(), 0.06));
        let seg = sw / 3.0;
        let modes = [
            (Mode::Saver, "Saver", GREEN),
            (Mode::Balanced, "Balanced", BLUE),
            (Mode::Performance, "Performance", ORANGE),
        ];
        for (i, (mode, label, accent)) in modes.into_iter().enumerate() {
            let x = sx + i as f32 * seg;
            let on = b.mode == mode;
            let hov = self.hit(x, sy, seg, sh, Action::SetPowerMode(mode));
            let (ix, iy, iw, ih) = (x + 3.0, sy + 3.0, seg - 6.0, sh - 6.0);
            if on {
                self.fill_rr(ix, iy, iw, ih, ih / 2.0, self.cc(accent, 0.26));
                self.stroke_rr(
                    ix + 0.5,
                    iy + 0.5,
                    iw - 1.0,
                    ih - 1.0,
                    ih / 2.0,
                    self.cc(accent, 0.6),
                    1.0,
                    false,
                );
            } else if hov {
                self.fill_rr(ix, iy, iw, ih, ih / 2.0, self.cc(self.fg(), 0.07));
            }
            let dot_x = x + seg / 2.0 - text_w(label) / 2.0 - 5.0;
            self.circle(
                dot_x,
                sy + sh / 2.0,
                3.0,
                self.cc(accent, if on { 1.0 } else { 0.55 }),
            );
            self.text(
                label,
                &self.f.btn,
                x + 6.0,
                sy,
                seg,
                sh,
                self.cc(self.fg(), if on { 1.0 } else { 0.7 }),
            );
        }
    }
}
