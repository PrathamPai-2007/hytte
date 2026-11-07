//! Lyrics for the song that is playing, from lrclib.net (free, no key). Off until the user turns
//! it on, since every lookup sends the song title and artist to that service. Lookups happen only
//! while the lyrics page is open. Results stay in memory (the last few songs) and are never
//! written to disk.

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub at_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lyrics {
    /// Timed lines, in order.
    Synced(Vec<Line>),
    /// Just the words.
    Plain(Vec<String>),
    Instrumental,
    NotFound,
}

/// Parse an LRC body: `[mm:ss.xx] text`, several stamps allowed on one line. Tags such as
/// `[ar: Someone]` are skipped. Lines come back in time order.
pub fn parse_lrc(body: &str) -> Vec<Line> {
    let mut out = vec![];
    for raw in body.lines() {
        let mut rest = raw.trim();
        let mut stamps = vec![];
        while let Some(r) = rest.strip_prefix('[') {
            let Some((tag, after)) = r.split_once(']') else {
                break;
            };
            match stamp_ms(tag) {
                Some(ms) => stamps.push(ms),
                None => break,
            }
            rest = after;
        }
        let text = rest.trim();
        for at_ms in stamps {
            out.push(Line {
                at_ms,
                text: text.to_string(),
            });
        }
    }
    out.sort_by_key(|l| l.at_ms);
    out
}

/// "01:23.45" → 83450. Anything else (a tag like "ar:Someone") is None.
fn stamp_ms(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let (sec, frac) = s.split_once(['.', ':']).unwrap_or((s, "0"));
    let sec: u64 = sec.trim().parse().ok()?;
    if sec >= 60 || frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // ".5" is half a second, ".05" is 50 ms, ".005" is 5 ms.
    let ms = frac.chars().take(3).collect::<String>();
    let ms: u64 = format!("{ms:0<3}").parse().ok()?;
    Some(m * 60_000 + sec * 1000 + ms)
}

/// The line being sung at `pos_ms`: the last one that has started. None before the first.
pub fn current_line(lines: &[Line], pos_ms: u64) -> Option<usize> {
    lines.iter().rposition(|l| l.at_ms <= pos_ms)
}

/// Timed lyrics are trusted when the entry's length is this close to the song's.
const SYNC_OK_MS: f64 = 3_000.0;
/// Beyond this the entry is a different recording: its words may still fit, its timing won't.
const NEAR_MS: f64 = 10_000.0;

/// One answer from lrclib (`/api/get` is one object, `/api/search` a list of them), judged
/// against the song's length (0 = unknown): within a few seconds its timing is right; a little
/// further off only the words are used; further still it isn't this recording. Returns the
/// lyrics and how far off the length was.
fn from_entry(v: &serde_json::Value, dur_ms: u64) -> Option<(Lyrics, f64)> {
    let diff = match (dur_ms, v["duration"].as_f64()) {
        (0, _) | (_, None) => 0.0,
        (d, Some(secs)) => (secs * 1000.0 - d as f64).abs(),
    };
    if diff > NEAR_MS {
        return None;
    }
    if v["instrumental"].as_bool() == Some(true) {
        return Some((Lyrics::Instrumental, diff));
    }
    let synced = v["syncedLyrics"]
        .as_str()
        .map(parse_lrc)
        .filter(|l| !l.is_empty());
    if let Some(lines) = synced.clone().filter(|_| diff <= SYNC_OK_MS) {
        return Some((Lyrics::Synced(lines), diff));
    }
    if let Some(p) = v["plainLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
        return Some((
            Lyrics::Plain(p.lines().map(|l| l.trim().to_string()).collect()),
            diff,
        ));
    }
    // Only timed words, and the timing can't be trusted: show them untimed.
    synced.map(|l| {
        let words = l.into_iter().map(|l| l.text).collect();
        (Lyrics::Plain(words), diff)
    })
}

/// The reply to `/api/get`.
pub fn from_get(body: &[u8], dur_ms: u64) -> Result<Lyrics, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("unexpected reply: {e}"))?;
    Ok(from_entry(&v, dur_ms).map_or(Lyrics::NotFound, |(l, _)| l))
}

/// The reply to `/api/search`: timed lyrics of the closest length if any entry has them, else
/// the closest of the rest.
pub fn from_search(body: &[u8], dur_ms: u64) -> Result<Lyrics, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("unexpected reply: {e}"))?;
    let mut found: Vec<(Lyrics, f64)> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| from_entry(e, dur_ms))
        .collect();
    // Stable: equally close entries keep the service's order.
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    Ok(found
        .iter()
        .find(|(l, _)| matches!(l, Lyrics::Synced(_)))
        .or_else(|| found.first())
        .map_or(Lyrics::NotFound, |(l, _)| l.clone()))
}

/// A key that says which song this is.
pub fn key(title: &str, artist: &str) -> String {
    format!("{title}\n{artist}")
}

fn get_url(title: &str, artist: &str, album: &str, dur_ms: u64) -> String {
    let mut u = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}",
        crate::net::encode(title),
        crate::net::encode(artist)
    );
    if !album.is_empty() {
        u += &format!("&album_name={}", crate::net::encode(album));
    }
    if dur_ms > 0 {
        u += &format!("&duration={}", dur_ms / 1000);
    }
    u
}

fn search_url(title: &str, artist: &str) -> String {
    format!(
        "https://lrclib.net/api/search?track_name={}&artist_name={}",
        crate::net::encode(title),
        crate::net::encode(artist)
    )
}

/// Look a song up (on a worker thread): the exact match, then title and artist alone (the
/// player's album and length often differ from the database's), then a search. "Not found" is
/// an answer, an unreachable service is an error.
pub fn lookup(title: &str, artist: &str, album: &str, dur_ms: u64) -> Result<Lyrics, String> {
    let fetch = |u: &str| crate::net::fetch(u, None, 10, 2 * 1024 * 1024);
    let mut urls = vec![get_url(title, artist, album, dur_ms)];
    if !album.is_empty() || dur_ms > 0 {
        urls.push(get_url(title, artist, "", 0));
    }
    for u in urls {
        match fetch(&u) {
            Ok(body) => match from_get(&body, dur_ms)? {
                Lyrics::NotFound => {}
                found => return Ok(found),
            },
            // curl -f turns an HTTP error into a failure: 404 is the service saying "no such song".
            Err(e) if e.contains("404") => {}
            Err(e) => return Err(e),
        }
    }
    from_search(&fetch(&search_url(title, artist))?, dur_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = "[ar: Nobody]\n[ti: Sample]\n[00:05.06] First made-up line\n[00:12.5]Second line\n[01:02.005] Third line\n[00:30.00][01:30.00] Chorus line\n[01:45.00]";

    #[test]
    fn timed_lines_parse_in_order_with_repeats_and_no_tags() {
        let l = parse_lrc(SONG);
        let at: Vec<u64> = l.iter().map(|l| l.at_ms).collect();
        assert_eq!(at, vec![5060, 12500, 30000, 62005, 90000, 105000]);
        assert_eq!(l[0].text, "First made-up line");
        assert_eq!(l[2].text, "Chorus line");
        assert_eq!(l[4].text, "Chorus line", "two stamps, two lines");
        assert_eq!(l[5].text, "", "an empty line marks a pause");
        assert!(parse_lrc("no stamps here\n[bad:tag]").is_empty());
        assert_eq!(stamp_ms("00:01.5"), Some(1500));
        assert_eq!(stamp_ms("00:61.0"), None);
    }

    #[test]
    fn the_current_line_is_the_last_one_started() {
        let l = parse_lrc(SONG);
        assert_eq!(current_line(&l, 0), None, "before the first line");
        assert_eq!(current_line(&l, 5060), Some(0));
        assert_eq!(current_line(&l, 29_999), Some(1));
        assert_eq!(current_line(&l, 30_000), Some(2));
        assert_eq!(current_line(&l, 10 * 60_000), Some(5));
        assert_eq!(current_line(&[], 5), None);
    }

    #[test]
    fn replies_become_timed_plain_instrumental_or_nothing() {
        let timed =
            br#"{"syncedLyrics":"[00:01.00] one\n[00:02.00] two","plainLyrics":"one\ntwo"}"#;
        assert!(matches!(from_get(timed, 0), Ok(Lyrics::Synced(l)) if l.len() == 2));
        let plain = br#"{"syncedLyrics":null,"plainLyrics":"one\n two \nthree"}"#;
        assert_eq!(
            from_get(plain, 0),
            Ok(Lyrics::Plain(vec![
                "one".into(),
                "two".into(),
                "three".into()
            ]))
        );
        assert_eq!(
            from_get(
                br#"{"instrumental":true,"plainLyrics":null,"syncedLyrics":null}"#,
                0
            ),
            Ok(Lyrics::Instrumental)
        );
        assert_eq!(
            from_get(br#"{"plainLyrics":"  ","syncedLyrics":""}"#, 0),
            Ok(Lyrics::NotFound)
        );
        assert!(from_get(b"<html>", 0).is_err());
    }

    #[test]
    fn a_search_takes_a_song_of_about_the_right_length_and_prefers_timed_words() {
        let body = br#"[
          {"duration": 300.0, "plainLyrics": "wrong length", "syncedLyrics": "[00:01.00] wrong"},
          {"duration": 241.0, "plainLyrics": "right\nlength", "syncedLyrics": null},
          {"duration": 243.5, "plainLyrics": "also right", "syncedLyrics": "[00:02.00] timed"}
        ]"#;
        assert!(
            matches!(from_search(body, 241_000), Ok(Lyrics::Synced(l)) if l[0].text == "timed")
        );
        assert_eq!(from_search(body, 100_000), Ok(Lyrics::NotFound));
        // No length known: anything goes, timed first.
        assert!(matches!(from_search(body, 0), Ok(Lyrics::Synced(l)) if l[0].text == "wrong"));
        assert_eq!(from_search(b"[]", 0), Ok(Lyrics::NotFound));
    }

    #[test]
    fn a_different_master_keeps_its_words_but_not_its_timing() {
        let entry = |dur: f64| {
            format!(
                r#"{{"duration": {dur}, "plainLyrics": "one\ntwo", "syncedLyrics": "[00:01.00] one\n[00:02.00] two"}}"#
            )
        };
        let get = |dur: f64, song_ms: u64| from_get(entry(dur).as_bytes(), song_ms).unwrap();
        // Within 3 s: timed. 4 s off: the words, untimed. 11 s off: another recording.
        assert!(matches!(get(246.0, 243_000), Lyrics::Synced(_)));
        assert_eq!(
            get(247.0, 243_000),
            Lyrics::Plain(vec!["one".into(), "two".into()])
        );
        assert_eq!(get(254.0, 243_000), Lyrics::NotFound);
        // Only timed lyrics on a loose match: shown as plain words.
        let only_timed =
            br#"{"duration": 247.0, "syncedLyrics": "[00:01.00] one\n[00:02.00] two"}"#;
        assert_eq!(
            from_get(only_timed, 243_000),
            Ok(Lyrics::Plain(vec!["one".into(), "two".into()]))
        );
        // A search takes the closest timed entry, else the closest words.
        let list = format!("[{}, {}]", entry(250.0), entry(244.0));
        assert!(matches!(
            from_search(list.as_bytes(), 243_000),
            Ok(Lyrics::Synced(_))
        ));
        let loose = format!("[{}, {}]", entry(249.0), entry(247.0));
        assert!(matches!(
            from_search(loose.as_bytes(), 243_000),
            Ok(Lyrics::Plain(_))
        ));
    }

    #[test]
    fn urls_encode_the_song_and_skip_what_is_unknown() {
        assert_eq!(
            get_url("Café del Mar", "Energy 52", "", 0),
            "https://lrclib.net/api/get?track_name=Caf%C3%A9%20del%20Mar&artist_name=Energy%2052"
        );
        assert!(get_url("A", "B", "C D", 241_900).ends_with("&album_name=C%20D&duration=241"));
        assert_ne!(key("A", "B"), key("B", "A"));
    }
}
