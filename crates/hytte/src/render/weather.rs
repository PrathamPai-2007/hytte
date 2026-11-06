//! The Weather pane: a place chooser with suggestions, then now plus the next five hours.

use super::*;
use crate::ui_state::Field;
use crate::weather::{describe, sky, temp_text, Hour, Sky};

const SUG_H: f32 = 28.0;

impl Renderer {
    pub(super) fn exp_weather(&self, fr: &Frame, w: f32, h: f32) {
        let m = fr.model;
        if m.choosing_place() {
            self.place_chooser(fr, w);
        } else if let Some(f) = &m.forecast {
            self.forecast_view(fr, f, w, h);
        } else {
            let place = m.weather_place.as_ref().map_or("", |p| p.name.as_str());
            let (a, b) = if m.weather_err {
                ("Can't reach the weather service.", "Check your connection.")
            } else {
                ("Checking the weather…", place)
            };
            self.text(
                a,
                &self.f.center,
                0.0,
                34.0,
                w,
                22.0,
                self.cc(self.fg(), 0.92),
            );
            self.text(b, &self.f.small_c, 0.0, 58.0, w, 16.0, self.cc(GRAY, 1.0));
            if m.weather_err {
                self.button(
                    "Try again",
                    w / 2.0 - 42.0,
                    84.0,
                    84.0,
                    26.0,
                    self.accent.get(),
                    Action::RetryWeather,
                );
            }
        }
    }

    fn place_chooser(&self, fr: &Frame, w: f32) {
        let m = fr.model;
        let fg = self.fg();
        self.text(
            "Where should Hytte check the weather?",
            &self.f.title,
            16.0,
            10.0,
            w - 32.0 - 60.0,
            22.0,
            self.cc(fg, 0.97),
        );
        if m.weather_place.is_some() {
            self.button(
                "Cancel",
                w - 16.0 - 54.0,
                12.0,
                54.0,
                20.0,
                self.accent.get(),
                Action::CancelPlace,
            );
        }
        self.text_field(
            fr,
            Field::Place,
            (16.0, 38.0, w - 32.0, 34.0),
            "Search for a town or city",
            0.0,
        );
        let mut y = 76.0;
        if m.places.is_empty() {
            let q = m.place_edit.text().trim();
            let msg = if m.places_busy {
                "Searching…"
            } else if m.places_err {
                "Can't reach the weather service."
            } else if q.chars().count() >= 2 && m.places_for == q {
                "No places match."
            } else {
                "Type at least two letters."
            };
            self.text(
                msg,
                &self.f.body,
                20.0,
                y,
                w - 40.0,
                SUG_H,
                self.cc(GRAY, 0.9),
            );
            return;
        }
        for (i, p) in m.places.iter().enumerate() {
            let hov = self.hit(12.0, y, w - 24.0, SUG_H, Action::PickPlace(i));
            if hov || i == m.place_sel {
                self.fill_rr(
                    12.0,
                    y,
                    w - 24.0,
                    SUG_H,
                    9.0,
                    self.cc(fg, if hov { 0.10 } else { 0.07 }),
                );
            }
            let nw = self.text_width(&p.name).min(w * 0.5);
            self.text(
                &p.name,
                &self.f.body,
                22.0,
                y,
                nw + 6.0,
                SUG_H,
                self.cc(fg, 0.97),
            );
            let rx = 22.0 + nw + 14.0;
            self.text(
                &p.region,
                &self.f.small,
                rx,
                y,
                w - 22.0 - rx,
                SUG_H,
                self.cc(GRAY, 0.95),
            );
            y += SUG_H;
        }
    }

    fn forecast_view(&self, fr: &Frame, f: &crate::weather::Forecast, w: f32, h: f32) {
        let m = fr.model;
        let (fg, imp) = (self.fg(), m.imperial);
        let wall = crate::timer::now_ms();
        // The pane proper ends where the tab strip begins.
        let bottom = h - TAB_H;
        self.text(
            &temp_text(f.now.temp, imp),
            &self.f.hero,
            16.0,
            6.0,
            84.0,
            48.0,
            self.cc(fg, 0.98),
        );
        self.text(
            describe(f.now.code, f.now.day),
            &self.f.title,
            100.0,
            12.0,
            w - 100.0 - 72.0,
            20.0,
            self.cc(fg, 0.97),
        );
        let line = format!(
            "{} · {} / {}",
            f.place.name,
            temp_text(f.hi, imp),
            temp_text(f.lo, imp)
        );
        let hov = self.hit(100.0, 32.0, w - 100.0 - 72.0, 20.0, Action::ChangePlace);
        self.text(
            &line,
            &self.f.body,
            100.0,
            32.0,
            w - 100.0 - 72.0,
            20.0,
            self.cc(if hov { fg } else { GRAY }, if hov { 0.95 } else { 1.0 }),
        );
        self.sky_icon(f.now, w - 16.0 - 24.0, 30.0, 40.0, 1.0);
        // Now and the next five hours.
        let hours: Vec<Hour> = std::iter::once(f.now)
            .chain(f.next.iter().copied())
            .collect();
        let step = (w - 32.0) / 6.0;
        for (i, hr) in hours.iter().enumerate() {
            let cx = 16.0 + step * (i as f32 + 0.5);
            let label = if i == 0 {
                "Now".to_string()
            } else {
                format!("{:02}", hr.hour)
            };
            self.text(
                &label,
                &self.f.small_c,
                cx - step / 2.0,
                62.0,
                step,
                14.0,
                self.cc(GRAY, 1.0),
            );
            self.sky_icon(*hr, cx, 90.0, 18.0, 0.9);
            self.text(
                &temp_text(hr.temp, imp),
                &self.f.btn,
                cx - step / 2.0,
                102.0,
                step,
                16.0,
                self.cc(fg, 0.95),
            );
        }
        // Where the data is from (its licence asks for that), and how old it is when stale.
        let stale = if f.fresh(wall) {
            String::new()
        } else if m.weather_err {
            format!(
                "Can't refresh · updated {}",
                crate::timer::clock(f.fetched_ms)
            )
        } else {
            format!("Updated {}", crate::timer::clock(f.fetched_ms))
        };
        if !stale.is_empty() {
            self.text(
                &stale,
                &self.f.small,
                16.0,
                bottom - 16.0,
                200.0,
                12.0,
                self.cc(AMBER, 0.85),
            );
        }
        self.text(
            "Weather data by Open-Meteo.com",
            &self.f.small_r,
            w - 16.0 - 190.0,
            bottom - 16.0,
            190.0,
            12.0,
            self.cc(GRAY, 0.55),
        );
    }

    /// A small drawn weather icon centred on (cx, cy), about `s` px across.
    pub(super) fn sky_icon(&self, hr: Hour, cx: f32, cy: f32, s: f32, a: f32) {
        let kind = sky(hr.code);
        // Pale on the dark pill, deeper on the light one so they still read.
        let light = self.light.get();
        let cloud_col = self.cc(
            if light {
                [0.52, 0.60, 0.74]
            } else {
                [0.80, 0.86, 0.95]
            },
            0.92 * a,
        );
        let sun = self.cc(
            if light {
                [0.93, 0.62, 0.05]
            } else {
                [1.0, 0.80, 0.30]
            },
            a,
        );
        let drop = self.cc(BLUE, a);
        let moon = |x: f32, y: f32, size: f32| {
            let c = if light {
                [0.36, 0.40, 0.72]
            } else {
                [0.78, 0.82, 1.0]
            };
            self.glyph('\u{E708}', x, y, size, self.cc(c, a));
        };
        let sun_at = |x: f32, y: f32, r: f32| {
            self.circle(x, y, r, sun);
            for k in 0..8 {
                let ang = k as f32 * std::f32::consts::FRAC_PI_4;
                let (c, sn) = (ang.cos(), ang.sin());
                self.line(
                    (x + c * r * 1.55, y + sn * r * 1.55),
                    (x + c * r * 2.0, y + sn * r * 2.0),
                    sun,
                    (s / 14.0).max(1.2),
                );
            }
        };
        let cloud = |x: f32, y: f32, sc: f32| {
            let u = s * sc;
            self.fill_rr(x - 0.40 * u, y, 0.80 * u, 0.26 * u, 0.13 * u, cloud_col);
            self.circle(x - 0.16 * u, y, 0.17 * u, cloud_col);
            self.circle(x + 0.08 * u, y - 0.08 * u, 0.24 * u, cloud_col);
        };
        match kind {
            Sky::Clear if hr.day => sun_at(cx, cy, s * 0.20),
            Sky::Clear => moon(cx, cy, s * 0.9),
            Sky::Partly => {
                if hr.day {
                    sun_at(cx - s * 0.16, cy - s * 0.12, s * 0.14);
                } else {
                    moon(cx - s * 0.18, cy - s * 0.12, s * 0.62);
                }
                cloud(cx + s * 0.05, cy + s * 0.08, 0.95);
            }
            Sky::Cloud => cloud(cx, cy, 1.05),
            Sky::Fog => {
                for (i, len) in [0.8f32, 0.6, 0.72].into_iter().enumerate() {
                    let y = cy - s * 0.2 + i as f32 * s * 0.22;
                    self.line(
                        (cx - len * s * 0.5, y),
                        (cx + len * s * 0.5, y),
                        cloud_col,
                        (s / 12.0).max(1.4),
                    );
                }
            }
            Sky::Rain => {
                cloud(cx, cy - s * 0.12, 1.0);
                for dx in [-0.24f32, 0.0, 0.24] {
                    let x = cx + dx * s;
                    self.line(
                        (x + s * 0.04, cy + s * 0.22),
                        (x - s * 0.04, cy + s * 0.42),
                        drop,
                        (s / 13.0).max(1.3),
                    );
                }
            }
            Sky::Snow => {
                cloud(cx, cy - s * 0.12, 1.0);
                for dx in [-0.22f32, 0.0, 0.22] {
                    self.circle(cx + dx * s, cy + s * 0.34, (s / 17.0).max(1.2), cloud_col);
                }
            }
            Sky::Thunder => {
                cloud(cx, cy - s * 0.14, 1.0);
                self.bolt(cx, cy + s * 0.26, s / 26.0, sun);
            }
        }
    }
}
