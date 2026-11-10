//! The Weather pane: forecast, place search and the place chooser.

use super::*;

/// How long to wait after a failed forecast fetch before trying again.
pub const WEATHER_RETRY: Duration = Duration::from_secs(60);

/// The glow that follows the sky: warm for a sunny day, indigo for a clear night, blue for rain.
pub(super) fn sky_glow(h: crate::weather::Hour) -> ([f32; 3], f32) {
    use crate::weather::Sky;
    match (crate::weather::sky(h.code), h.day) {
        (Sky::Clear, true) => ([1.0, 0.80, 0.40], 0.30),
        (Sky::Clear, false) => ([0.55, 0.58, 1.0], 0.30),
        (Sky::Partly, true) => ([0.75, 0.85, 1.0], 0.22),
        (Sky::Partly, false) => ([0.55, 0.60, 0.90], 0.22),
        (Sky::Cloud, _) => ([0.70, 0.75, 0.85], 0.20),
        (Sky::Fog, _) => ([0.80, 0.80, 0.85], 0.18),
        (Sky::Rain, _) => ([0.35, 0.63, 1.0], 0.38),
        (Sky::Snow, _) => ([0.85, 0.92, 1.0], 0.35),
        (Sky::Thunder, _) => ([0.65, 0.54, 0.98], 0.45),
    }
}

impl Model {
    /// The Weather pane shows its place chooser (no place yet, or changing it).
    pub fn choosing_place(&self) -> bool {
        self.choosing || self.weather_place.is_none()
    }

    /// Height of the Weather pane without the tab strip.
    pub fn weather_h(&self) -> f64 {
        if self.choosing_place() {
            let mut rows = self.places.len().max(1) as f64;
            if self.places.is_empty() && self.wide_offer {
                rows += 1.0; // the "search more widely" row
            }
            // A line crediting OpenStreetMap under its results.
            let credit = if self.places_wide { 16.0 } else { 0.0 };
            38.0 + 34.0 + 4.0 + rows * 28.0 + 8.0 + credit
        } else if self.forecast.is_some() {
            148.0
        } else {
            120.0
        }
    }

    /// Should the forecast be fetched now? A place is chosen, nothing is in flight, the data is
    /// old (or missing) and a recent failure isn't being waited out.
    pub fn want_forecast(&self, wall_ms: u64, now: Instant) -> bool {
        self.weather_place.is_some()
            && !self.choosing_place()
            && !self.weather_busy
            && self.weather_retry.is_none_or(|t| now >= t)
            && self.forecast.as_ref().is_none_or(|f| !f.fresh(wall_ms))
    }

    /// A forecast fetch finished. One for a place that was changed since is dropped.
    pub fn forecast_result(&mut self, r: Result<crate::weather::Forecast, String>, now: Instant) {
        self.weather_busy = false;
        match r {
            Ok(f) if self.weather_place.as_ref() == Some(&f.place) => {
                self.forecast = Some(f);
                self.weather_err = false;
                self.weather_retry = None;
            }
            Ok(_) => {}
            Err(_) => {
                self.weather_err = true;
                self.weather_retry = Some(now + WEATHER_RETRY);
            }
        }
    }

    /// Search results. Ones for text that is no longer in the field are dropped.
    pub fn places_result(&mut self, query: String, r: Result<Vec<crate::weather::Place>, String>) {
        if self.place_edit.text().trim() != query {
            return;
        }
        self.places_busy = false;
        self.places_for = query;
        self.reset_wide();
        match r {
            Ok(p) => {
                self.places = p;
                self.places_err = false;
                self.place_sel = 0;
            }
            Err(_) => {
                self.places.clear();
                self.places_err = true;
            }
        }
    }

    /// Forget the wider search (the query changed, or a new Open-Meteo answer came).
    pub fn reset_wide(&mut self) {
        self.wide_offer = false;
        self.wide_busy = false;
        self.wide_err = false;
        self.wide_for.clear();
        self.places_wide = false;
    }

    /// Two seconds of "nothing found" have passed: offer the wider search. Only when the
    /// answer on screen is Open-Meteo's empty one for what is still in the field.
    pub fn offer_wide(&mut self) -> bool {
        let q = self.place_edit.text().trim();
        let due = self.choosing_place()
            && self.places.is_empty()
            && !self.places_busy
            && !self.places_err
            && !self.wide_busy
            && q.chars().count() >= 2
            && self.places_for == q;
        self.wide_offer = due;
        due
    }

    /// The user asked for the wider search. A repeat of an earlier query is answered from the
    /// session cache; otherwise the query to send.
    pub fn start_wide(&mut self) -> Option<String> {
        if !self.wide_offer || self.wide_busy {
            return None;
        }
        let q = self.place_edit.text().trim().to_string();
        let key = q.to_lowercase();
        self.wide_err = false;
        if let Some((_, p)) = self.wide_cache.iter().find(|(k, _)| *k == key) {
            self.places = p.clone();
            self.wide_for = q;
            self.places_wide = true;
            self.wide_offer = false;
            self.place_sel = 0;
            return None;
        }
        self.wide_busy = true;
        Some(q)
    }

    /// The wider search answered. One for text no longer in the field is dropped.
    pub fn wide_result(&mut self, query: String, r: Result<Vec<crate::weather::Place>, String>) {
        if self.place_edit.text().trim() != query {
            return;
        }
        self.wide_busy = false;
        match r {
            Ok(p) => {
                if self.wide_cache.len() >= 8 {
                    self.wide_cache.remove(0);
                }
                self.wide_cache.push((query.to_lowercase(), p.clone()));
                self.wide_for = query;
                self.places_wide = !p.is_empty();
                self.wide_offer = false;
                self.places = p;
                self.place_sel = 0;
            }
            Err(_) => self.wide_err = true,
        }
    }

    /// Move the highlighted suggestion.
    pub fn place_move(&mut self, by: i32) {
        if !self.places.is_empty() {
            self.place_sel =
                (self.place_sel as i32 + by).clamp(0, self.places.len() as i32 - 1) as usize;
        }
    }

    /// Choose suggestion `i`: the chooser closes and the forecast starts over.
    pub fn pick_place(&mut self, i: usize) -> Option<crate::weather::Place> {
        let p = self.places.get(i)?.clone();
        self.weather_place = Some(p.clone());
        self.forecast = None;
        self.weather_err = false;
        self.weather_retry = None;
        self.close_chooser();
        Some(p)
    }

    pub(super) fn close_chooser(&mut self) {
        self.choosing = false;
        self.reset_wide();
        self.places.clear();
        self.places_busy = false;
        self.places_err = false;
        self.place_edit.clear();
    }

    /// Open the chooser to change the place.
    pub fn start_choosing(&mut self) {
        self.choosing = true;
        self.close_chooser_fields();
    }

    pub(super) fn close_chooser_fields(&mut self) {
        self.reset_wide();
        self.places.clear();
        self.places_busy = false;
        self.places_err = false;
        self.place_edit.clear();
    }
}
