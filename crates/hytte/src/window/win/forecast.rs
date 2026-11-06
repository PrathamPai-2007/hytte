//! Weather on the UI thread: place search while typing, picking a place, and fetching the
//! forecast when the pane is open and its data is old. The network calls run on worker threads.

use super::*;
use crate::ui_state::Field;

/// A pause after the last keystroke before the place search runs.
const SEARCH_DELAY_MS: u32 = 250;

impl Ui {
    /// Fetch the forecast if the Weather pane is showing and its data is old (or missing).
    /// Cheap to call: it does nothing when there is nothing to do.
    pub(super) fn weather_refresh(&mut self) {
        if self.anim.scene != Scene::ExpWeather {
            return;
        }
        let (now, wall) = (Instant::now(), crate::timer::now_ms());
        if !self.model.want_forecast(wall, now) {
            return;
        }
        let Some(place) = self.model.weather_place.clone() else {
            return;
        };
        self.model.weather_busy = true;
        let sh = self.shared.clone();
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            push_event(
                &sh,
                UiEvent::Forecast(crate::weather::fetch_forecast(&place)),
            );
        });
    }

    /// The place field changed: search once typing pauses.
    pub(super) fn place_typed(&mut self, hwnd: HWND) {
        self.place_seq += 1;
        self.model.place_sel = 0;
        unsafe {
            SetTimer(Some(hwnd), T_PLACE, SEARCH_DELAY_MS, None);
        }
    }

    /// `T_PLACE` fired: look the typed place up.
    pub(super) fn place_search(&mut self) {
        let q = self.model.place_edit.text().trim().to_string();
        if q.chars().count() < 2 {
            self.model.places.clear();
            self.model.places_busy = false;
            self.model.places_err = false;
            self.layout();
            self.kick();
            return;
        }
        self.model.places_busy = true;
        let (seq, sh) = (self.place_seq, self.shared.clone());
        std::thread::spawn(move || {
            crate::proc::eco_thread();
            let r = crate::weather::fetch_places(&q);
            push_event(&sh, UiEvent::Places(seq, q, r));
        });
        self.layout();
        self.kick();
    }

    /// Enter in the place field: take the highlighted suggestion.
    pub(super) fn place_enter(&mut self, hwnd: HWND) {
        self.pick_place(hwnd, self.model.place_sel);
    }

    pub(super) fn pick_place(&mut self, hwnd: HWND, i: usize) {
        if let Some(p) = self.model.pick_place(i) {
            p.save();
            self.end_keyboard(true);
            self.after_model_change(hwnd);
        }
    }

    /// Click on the place name: choose another one.
    pub(super) fn change_place(&mut self, hwnd: HWND) {
        self.model.start_choosing();
        self.begin_typing(hwnd, Field::Place);
    }
}
