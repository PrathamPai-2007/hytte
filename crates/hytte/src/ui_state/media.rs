//! The Media pane: the song, its art, lyrics and the sleep timer.

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    /// Source app's AppUserModelId, used to bring it to the front.
    pub app: String,
    pub pos_ms: u64,
    pub dur_ms: u64,
}

/// The colour that best represents a picture: the average of its pixels weighted by how
/// saturated they are, brightened to a glow-worthy level. None for greyscale art.
pub fn accent_of(bgra: &[u8]) -> Option<[f32; 3]> {
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for px in bgra.as_chunks::<4>().0 {
        let a = px[3] as f32 / 255.0;
        if a < 0.5 {
            continue;
        }
        // Premultiplied -> straight, 0..1.
        let (b, g, r) = (
            px[0] as f32 / 255.0 / a,
            px[1] as f32 / 255.0 / a,
            px[2] as f32 / 255.0 / a,
        );
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        // Near-black and washed-out pixels carry no hue worth glowing.
        let w = sat * sat * max;
        sum[0] += r * w;
        sum[1] += g * w;
        sum[2] += b * w;
        weight += w;
    }
    if weight < 1.0 {
        return None;
    }
    let c = [sum[0] / weight, sum[1] / weight, sum[2] / weight];
    let top = c[0].max(c[1]).max(c[2]).max(0.01);
    Some([c[0] / top * 0.95, c[1] / top * 0.95, c[2] / top * 0.95])
}

/// Premultiplied BGRA, ready for Direct2D.
#[derive(Debug)]
pub struct ArtBitmap {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

/// What the lyrics page knows about the playing song.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum LyricState {
    #[default]
    Idle,
    Loading,
    Ready(crate::lyrics::Lyrics),
    Failed,
    /// The player is a video or photo app: nothing is looked up.
    Skipped,
}

/// Songs whose lyrics are kept in memory.
pub(super) const LYRIC_CACHE: usize = 20;

impl Model {
    pub fn apply_media(&mut self, m: Option<MediaInfo>, now: Instant) {
        if m.is_none() {
            self.art = None;
            self.art_accent = None;
            self.sleep_open = false;
            self.lyrics_open = false;
        }
        // A different song: its lyrics start over.
        let key = m.as_ref().map(|m| crate::lyrics::key(&m.title, &m.artist));
        if key.as_deref() != Some(self.lyric_key.as_str()) {
            self.lyric_key.clear();
            self.lyric_state = LyricState::Idle;
            self.lyric_first = 0;
        }
        self.media = m;
        self.media_stamp = Some(now);
        // An "end of track" sleep timer follows the song.
        if let Some(s) = self.sleep.as_mut() {
            s.retime(self.media.as_ref(), now, now);
        }
    }

    /// Set the sleep timer (None cancels it).
    pub fn set_sleep(&mut self, kind: Option<crate::sleep::Kind>, now: Instant) {
        self.sleep =
            kind.map(|k| crate::sleep::Sleep::new(k, self.media.as_ref(), self.media_stamp, now));
        self.sleep_open = false;
    }

    /// Lyrics to fetch now, as (key, title, artist, album, length ms): the page is open, lyrics
    /// are on, a song plays and nothing is known about it yet. Cached songs are answered here.
    pub fn lyrics_request(&mut self) -> Option<(String, String, String, String, u64)> {
        if !(self.lyrics_open && self.lyrics_on) {
            return None;
        }
        let m = self.media.as_ref().filter(|m| !m.title.is_empty())?;
        let key = crate::lyrics::key(&m.title, &m.artist);
        if key == self.lyric_key {
            return None;
        }
        self.lyric_key = key.clone();
        self.lyric_first = 0;
        if crate::lyrics::skips_app(&m.app) {
            self.lyric_state = LyricState::Skipped;
            return None;
        }
        if let Some((_, l)) = self.lyric_cache.iter().find(|(k, _)| *k == key) {
            self.lyric_state = LyricState::Ready(l.clone());
            return None;
        }
        self.lyric_state = LyricState::Loading;
        Some((
            key,
            m.title.clone(),
            m.artist.clone(),
            m.album.clone(),
            m.dur_ms,
        ))
    }

    /// A lookup finished. A late answer for an earlier song is kept for later, not shown.
    pub fn lyrics_result(&mut self, key: String, r: Result<crate::lyrics::Lyrics, String>) {
        match r {
            Ok(l) => {
                self.lyric_cache.retain(|(k, _)| *k != key);
                self.lyric_cache.push((key.clone(), l.clone()));
                if self.lyric_cache.len() > LYRIC_CACHE {
                    self.lyric_cache.remove(0);
                }
                if key == self.lyric_key {
                    self.lyric_state = LyricState::Ready(l);
                }
            }
            Err(_) if key == self.lyric_key => self.lyric_state = LyricState::Failed,
            Err(_) => {}
        }
    }

    /// How many lines the lyrics page lists for scrolling: plain words, or timed lines shown
    /// without following the song. None while the page follows the song or has no words.
    pub fn lyric_rows(&self) -> Option<usize> {
        match &self.lyric_state {
            LyricState::Ready(crate::lyrics::Lyrics::Plain(l)) => Some(l.len()),
            LyricState::Ready(crate::lyrics::Lyrics::Synced(l)) if self.lyrics_still => {
                Some(l.len())
            }
            _ => None,
        }
    }

    /// Scroll plain lyrics by `by` lines; false when there is nothing to scroll (or at an end).
    pub fn scroll_lyrics(&mut self, by: i32) -> bool {
        let Some(n) = self.lyric_rows() else {
            return false;
        };
        let max = n.saturating_sub(3) as i32;
        let first = self.lyric_first.min(max as usize) as i32;
        let next = (first + by).clamp(0, max);
        self.lyric_first = next as usize;
        next != first
    }

    /// The Media pane's sleep chips and lyrics page close (the pill collapsed, another pane).
    pub fn close_media_extras(&mut self) {
        self.sleep_open = false;
        self.lyrics_open = false;
    }

    pub fn set_art(&mut self, a: Option<Arc<ArtBitmap>>) {
        self.art_accent = a.as_ref().and_then(|b| accent_of(&b.bgra));
        self.art = a;
        self.art_gen = self.art_gen.wrapping_add(1);
    }
}
