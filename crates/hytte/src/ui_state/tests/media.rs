use super::*;

#[test]
fn lyrics_are_not_looked_up_for_a_video_app() {
    let t0 = Instant::now();
    let mut m = Model::default();
    let mut s = song("A film", true, 0);
    s.app = "Microsoft.ZuneVideo_8wekyb3d8bbwe!Microsoft.ZuneVideo".into();
    m.apply_media(Some(s), t0);
    m.lyrics_open = true;
    m.lyrics_on = true;
    assert_eq!(m.lyrics_request(), None, "nothing is sent");
    assert_eq!(m.lyric_state, LyricState::Skipped);
    assert_eq!(m.lyric_rows(), None);
}

#[test]
fn lyrics_are_fetched_once_per_song_and_only_when_wanted() {
    use crate::lyrics::Lyrics;
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_media(Some(song("One", true, 0)), t0);
    assert_eq!(m.lyrics_request(), None, "page closed");
    m.lyrics_open = true;
    assert_eq!(m.lyrics_request(), None, "lyrics off");
    m.lyrics_on = true;
    let (key, title, artist, album, dur) = m.lyrics_request().expect("a lookup");
    assert_eq!(
        (title.as_str(), artist.as_str(), album.as_str(), dur),
        ("One", "Band", "Album", 200_000)
    );
    assert_eq!(m.lyric_state, LyricState::Loading);
    assert_eq!(m.lyrics_request(), None, "not twice");
    // Position updates for the same song change nothing.
    m.apply_media(Some(song("One", true, 5_000)), t0);
    assert_eq!(m.lyric_state, LyricState::Loading);
    m.lyrics_result(key.clone(), Ok(Lyrics::Instrumental));
    assert_eq!(m.lyric_state, LyricState::Ready(Lyrics::Instrumental));
    // The next song starts over, and the first one is remembered.
    m.apply_media(Some(song("Two", true, 0)), t0);
    assert_eq!(m.lyric_state, LyricState::Idle);
    let (key2, ..) = m.lyrics_request().expect("a lookup for the new song");
    // A late answer for the old song doesn't show on the new one.
    m.lyrics_result(key.clone(), Ok(Lyrics::NotFound));
    assert_eq!(m.lyric_state, LyricState::Loading);
    m.lyrics_result(key2, Err("offline".into()));
    assert_eq!(m.lyric_state, LyricState::Failed);
    // Back to song one: answered from memory, no lookup.
    m.apply_media(Some(song("One", true, 0)), t0);
    assert_eq!(m.lyrics_request(), None);
    assert_eq!(
        m.lyric_state,
        LyricState::Ready(Lyrics::NotFound),
        "the latest answer is kept"
    );
    // The player going away closes the extras.
    m.sleep_open = true;
    m.apply_media(None, t0);
    assert!(!m.lyrics_open && !m.sleep_open);
}

#[test]
fn the_lyric_cache_keeps_the_last_twenty_songs() {
    use crate::lyrics::Lyrics;
    let mut m = Model::default();
    for i in 0..25 {
        m.lyrics_result(format!("k{i}"), Ok(Lyrics::Instrumental));
    }
    assert_eq!(m.lyric_cache.len(), 20);
    assert_eq!(m.lyric_cache[0].0, "k5");
}

#[test]
fn plain_lyrics_scroll_three_lines_at_a_time_to_the_end() {
    use crate::lyrics::Lyrics;
    let mut m = Model::default();
    assert!(!m.scroll_lyrics(1), "nothing to scroll");
    m.lyric_state = LyricState::Ready(Lyrics::Plain((0..6).map(|i| format!("l{i}")).collect()));
    assert!(m.scroll_lyrics(1) && m.scroll_lyrics(1) && m.scroll_lyrics(1));
    assert_eq!(m.lyric_first, 3, "the last three lines");
    assert!(!m.scroll_lyrics(1), "at the end the wheel pages");
    assert!(m.scroll_lyrics(-1));
}

#[test]
fn the_sleep_timer_follows_the_song_and_wakes_the_model() {
    use crate::sleep::Kind;
    let t0 = Instant::now();
    let mut m = Model::default();
    m.apply_media(Some(song("One", true, 100_000)), t0);
    m.set_sleep(Some(Kind::EndOfTrack), t0);
    assert_eq!(
        m.sleep.as_ref().unwrap().until,
        Some(t0 + Duration::from_secs(100))
    );
    assert!(m.expire(t0).is_some(), "the model wakes for the fade");
    // A seek moves it; a new song fires it.
    m.apply_media(
        Some(song("One", true, 190_000)),
        t0 + Duration::from_secs(1),
    );
    assert_eq!(
        m.sleep.as_ref().unwrap().until,
        Some(t0 + Duration::from_secs(11))
    );
    m.apply_media(Some(song("Two", true, 0)), t0 + Duration::from_secs(2));
    assert!(m.sleep.as_ref().unwrap().fire);
    m.set_sleep(None, t0);
    assert!(m.sleep.is_none() && !m.sleep_open);
    assert_eq!(m.expire(t0), None, "nothing left to wake for");
    m.set_sleep(Some(Kind::Minutes(30)), t0);
    assert_eq!(
        m.expire(t0),
        Some(t0 + Duration::from_secs(60)),
        "the label ticks each minute"
    );
}
