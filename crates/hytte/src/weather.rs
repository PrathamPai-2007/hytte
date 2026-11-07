//! Weather for the Weather pane, from Open-Meteo (free for non-commercial use, no key; its data
//! is CC BY 4.0, so the pane credits it). Two calls: a place search (suggestions while typing)
//! and a forecast. Both run on worker threads and report with a `UiEvent`; nothing is fetched
//! unless the pane is open and its data is old.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Data older than this is fetched again when the pane opens.
pub const FRESH_MS: u64 = 30 * 60_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    /// "Vestland, Norway": what tells two places of one name apart.
    pub region: String,
    pub lat: f64,
    pub lon: f64,
}

fn place_file() -> PathBuf {
    crate::config::data_dir().join("place.json")
}

impl Place {
    pub fn load() -> Option<Place> {
        let text = std::fs::read_to_string(place_file()).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) {
        if let Ok(text) = serde_json::to_string(self) {
            let _ = crate::config::write_atomic(&place_file(), &text);
        }
    }
}

/// What the sky is doing, for the icon and the glow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sky {
    Clear,
    Partly,
    Cloud,
    Fog,
    Rain,
    Snow,
    Thunder,
}

/// WMO weather code → sky.
pub fn sky(code: u8) -> Sky {
    match code {
        0 | 1 => Sky::Clear,
        2 => Sky::Partly,
        45 | 48 => Sky::Fog,
        51..=67 | 80..=82 => Sky::Rain,
        71..=77 | 85 | 86 => Sky::Snow,
        95..=99 => Sky::Thunder,
        _ => Sky::Cloud, // 3 and anything unknown
    }
}

pub fn describe(code: u8, day: bool) -> &'static str {
    match code {
        0 if day => "Sunny",
        0 => "Clear",
        1 if day => "Mostly sunny",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51..=55 => "Drizzle",
        56 | 57 | 66 | 67 => "Freezing rain",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        71 => "Light snow",
        73 | 77 => "Snow",
        75 => "Heavy snow",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95..=99 => "Thunderstorm",
        _ => "Cloudy",
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hour {
    /// Hour of the day at the place, 0..24.
    pub hour: u8,
    /// Celsius.
    pub temp: f32,
    pub code: u8,
    pub day: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Forecast {
    pub place: Place,
    pub fetched_ms: u64,
    pub now: Hour,
    /// Today's high and low, Celsius.
    pub hi: f32,
    pub lo: f32,
    /// The next five hours.
    pub next: Vec<Hour>,
}

impl Forecast {
    pub fn fresh(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.fetched_ms) < FRESH_MS
    }
}

pub fn places_url(query: &str) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=4&language=en&format=json",
        crate::net::encode(query.trim())
    )
}

pub fn forecast_url(p: &Place) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=temperature_2m,weather_code,is_day\
         &hourly=temperature_2m,weather_code,is_day\
         &daily=temperature_2m_max,temperature_2m_min&forecast_days=2&timezone=auto",
        p.lat, p.lon
    )
}

/// Matches for the place search. No `results` key means no matches, not an error.
pub fn parse_places(body: &[u8]) -> Result<Vec<Place>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("unexpected reply: {e}"))?;
    let mut out: Vec<Place> = vec![];
    for r in v["results"].as_array().into_iter().flatten() {
        let (Some(name), Some(lat), Some(lon)) = (
            r["name"].as_str(),
            r["latitude"].as_f64(),
            r["longitude"].as_f64(),
        ) else {
            continue;
        };
        let mut parts: Vec<&str> = vec![];
        for key in ["admin1", "country"] {
            if let Some(s) = r[key].as_str().filter(|s| !s.is_empty() && *s != name) {
                if !parts.contains(&s) {
                    parts.push(s);
                }
            }
        }
        let p = Place {
            name: name.to_string(),
            region: parts.join(", "),
            lat,
            lon,
        };
        // The service can list the same town twice (two ids); show it once.
        if !out.iter().any(|q| q.name == p.name && q.region == p.region) {
            out.push(p);
        }
    }
    Ok(out)
}

fn hour_of(time: &str) -> Option<u8> {
    time.get(11..13)?.parse().ok()
}

/// The forecast reply: now, today's range and the next five hours at the place's own clock.
pub fn parse_forecast(place: &Place, body: &[u8], fetched_ms: u64) -> Result<Forecast, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("unexpected reply: {e}"))?;
    let cur = &v["current"];
    let (Some(ct), Some(t), Some(code)) = (
        cur["time"].as_str(),
        cur["temperature_2m"].as_f64(),
        cur["weather_code"].as_u64(),
    ) else {
        return Err("the forecast had no current weather".into());
    };
    let day_of = |x: &serde_json::Value| x.as_i64().unwrap_or(1) != 0;
    let now = Hour {
        hour: hour_of(ct).ok_or("bad time")?,
        temp: t as f32,
        code: code as u8,
        day: day_of(&cur["is_day"]),
    };
    let h = &v["hourly"];
    let times = h["time"].as_array().ok_or("no hourly data")?;
    let this_hour = ct.get(..13).map(|s| format!("{s}:00")).unwrap_or_default();
    let at = times
        .iter()
        .position(|x| x.as_str() == Some(this_hour.as_str()))
        .ok_or("the current hour isn't in the forecast")?;
    let mut next = vec![];
    for i in at + 1..(at + 6).min(times.len()) {
        let (Some(time), Some(temp), Some(code)) = (
            times[i].as_str(),
            h["temperature_2m"][i].as_f64(),
            h["weather_code"][i].as_u64(),
        ) else {
            break;
        };
        next.push(Hour {
            hour: hour_of(time).ok_or("bad time")?,
            temp: temp as f32,
            code: code as u8,
            day: day_of(&h["is_day"][i]),
        });
    }
    let d = &v["daily"];
    let (Some(hi), Some(lo)) = (
        d["temperature_2m_max"][0].as_f64(),
        d["temperature_2m_min"][0].as_f64(),
    ) else {
        return Err("the forecast had no daily range".into());
    };
    Ok(Forecast {
        place: place.clone(),
        fetched_ms,
        now,
        hi: hi as f32,
        lo: lo as f32,
        next,
    })
}

/// Search places on a worker thread.
pub fn fetch_places(query: &str) -> Result<Vec<Place>, String> {
    parse_places(&crate::net::fetch(&places_url(query), None, 10, 262_144)?)
}

/// Fetch the forecast on a worker thread.
pub fn fetch_forecast(p: &Place) -> Result<Forecast, String> {
    let body = crate::net::fetch(&forecast_url(p), None, 15, 1_048_576)?;
    parse_forecast(p, &body, crate::timer::now_ms())
}

/// Fahrenheit? `[weather] units` says; "auto" follows the Windows measurement setting.
pub fn imperial(units: &str) -> bool {
    match units {
        "imperial" => true,
        "metric" => false,
        _ => windows_imperial(),
    }
}

#[cfg(windows)]
fn windows_imperial() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_IMEASURE, LOCALE_RETURN_NUMBER};
    let mut buf = [0u16; 2];
    // LOCALE_NAME_USER_DEFAULT is a null name. With LOCALE_RETURN_NUMBER the two u16s are a DWORD.
    let n = unsafe {
        GetLocaleInfoEx(
            PCWSTR::null(),
            LOCALE_IMEASURE | LOCALE_RETURN_NUMBER,
            Some(&mut buf),
        )
    };
    n > 0 && buf[0] == 1
}

#[cfg(not(windows))]
fn windows_imperial() -> bool {
    false
}

/// "11°" or "52°": a whole number, never "-0".
pub fn temp_text(celsius: f32, imperial: bool) -> String {
    let t = if imperial {
        celsius * 9.0 / 5.0 + 32.0
    } else {
        celsius
    };
    let r = t.round() as i32;
    format!("{}°", if r == 0 { 0 } else { r })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEO: &str = r#"{"results":[
      {"id":3161732,"name":"Bergen","latitude":60.39299,"longitude":5.32415,"country":"Norway","admin1":"Vestland"},
      {"id":1,"name":"Bergen","latitude":60.39,"longitude":5.32,"country":"Norway","admin1":"Vestland"},
      {"id":2,"name":"Singapore","latitude":1.28,"longitude":103.85,"country":"Singapore","admin1":"Central Singapore"},
      {"id":3,"name":"Tromsø","latitude":69.65,"longitude":18.96,"country":"Norway"},
      {"id":4,"name":"nameless","longitude":1.0}
    ],"generationtime_ms":0.4}"#;

    #[test]
    fn places_show_their_region_and_never_twice() {
        let p = parse_places(GEO.as_bytes()).unwrap();
        let names: Vec<(&str, &str)> = p
            .iter()
            .map(|p| (p.name.as_str(), p.region.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Bergen", "Vestland, Norway"),
                ("Singapore", "Central Singapore"), // the country is the name: not repeated
                ("Tromsø", "Norway"),
            ]
        );
        assert_eq!(
            parse_places(br#"{"generationtime_ms":0.2}"#).unwrap(),
            vec![]
        );
        assert!(parse_places(b"<html>403</html>").is_err());
    }

    #[test]
    fn urls_encode_the_query() {
        assert!(places_url(" Tromsø ").contains("name=Troms%C3%B8&"));
        let p = Place {
            name: "x".into(),
            region: String::new(),
            lat: 60.3929,
            lon: -5.3,
        };
        let u = forecast_url(&p);
        assert!(u.contains("latitude=60.3929&longitude=-5.3000"));
        assert!(u.starts_with("https://"));
    }

    fn sample(hours: usize) -> String {
        let times: Vec<String> = (0..hours)
            .map(|i| format!("\"2026-11-06T{:02}:00\"", i % 24))
            .collect();
        let temps: Vec<String> = (0..hours).map(|i| format!("{}.5", i)).collect();
        let codes: Vec<String> = (0..hours).map(|i| (i % 4).to_string()).collect();
        let days: Vec<String> = (0..hours)
            .map(|i| ((6..18).contains(&(i % 24)) as u8).to_string())
            .collect();
        format!(
            r#"{{"current":{{"time":"2026-11-06T14:15","temperature_2m":11.4,"weather_code":61,"is_day":1}},
            "hourly":{{"time":[{}],"temperature_2m":[{}],"weather_code":[{}],"is_day":[{}]}},
            "daily":{{"temperature_2m_max":[14.2,13.0],"temperature_2m_min":[7.6,6.0]}}}}"#,
            times.join(","),
            temps.join(","),
            codes.join(","),
            days.join(",")
        )
    }

    fn here() -> Place {
        Place {
            name: "Bergen".into(),
            region: String::new(),
            lat: 0.0,
            lon: 0.0,
        }
    }

    #[test]
    fn the_forecast_starts_at_the_current_hour() {
        let f = parse_forecast(&here(), sample(24).as_bytes(), 1234).unwrap();
        assert_eq!((f.now.hour, f.now.code, f.now.day), (14, 61, true));
        assert_eq!((f.hi, f.lo), (14.2, 7.6));
        let hours: Vec<u8> = f.next.iter().map(|h| h.hour).collect();
        assert_eq!(hours, vec![15, 16, 17, 18, 19]);
        assert_eq!(f.next[0].temp, 15.5);
        assert!(
            f.next[2].day && !f.next[4].day,
            "17:00 is day, 19:00 is night"
        );
        assert!(f.fresh(1234 + FRESH_MS - 1) && !f.fresh(1234 + FRESH_MS));
    }

    #[test]
    fn a_short_forecast_gives_what_there_is_and_garbage_is_an_error() {
        let f = parse_forecast(&here(), sample(17).as_bytes(), 0).unwrap();
        assert_eq!(f.next.len(), 2, "hours 15 and 16 only");
        assert!(parse_forecast(&here(), sample(10).as_bytes(), 0).is_err());
        assert!(parse_forecast(&here(), b"{}", 0).is_err());
        assert!(parse_forecast(&here(), b"nope", 0).is_err());
    }

    #[test]
    fn every_weather_code_has_a_sky_and_a_description() {
        for code in 0..=99u8 {
            assert!(!describe(code, true).is_empty() && !describe(code, false).is_empty());
            let _ = sky(code);
        }
        assert_eq!(sky(0), Sky::Clear);
        assert_eq!(sky(2), Sky::Partly);
        assert_eq!(sky(3), Sky::Cloud);
        assert_eq!(sky(63), Sky::Rain);
        assert_eq!(sky(81), Sky::Rain);
        assert_eq!(sky(73), Sky::Snow);
        assert_eq!(sky(96), Sky::Thunder);
        assert_eq!(sky(48), Sky::Fog);
        assert_eq!(describe(0, true), "Sunny");
        assert_eq!(describe(0, false), "Clear");
    }

    #[test]
    fn temperatures_round_and_convert() {
        assert_eq!(temp_text(11.4, false), "11°");
        assert_eq!(temp_text(-0.3, false), "0°");
        assert_eq!(temp_text(-4.6, false), "-5°");
        assert_eq!(temp_text(0.0, true), "32°");
        assert_eq!(temp_text(21.0, true), "70°");
        assert!(imperial("imperial") && !imperial("metric"));
    }
}
