//! Finds progress in a command's stderr for `notch run`: OSC 9;4 sequences (the ones Windows
//! Terminal turns into a tab progress bar) and plain `NN%` text, as printed by git, pip, curl,
//! ffmpeg and many others.

/// Longest segment (text since the last `\r` / `\n`) kept for percentage matching.
const SEG_CAP: usize = 512;
/// Longest OSC body accepted; real 9;4 sequences are a few bytes.
const OSC_CAP: usize = 32;

#[derive(Default)]
pub struct Scanner {
    seg: Vec<u8>,
    /// Inside `ESC ]`: the body so far.
    osc: Option<Vec<u8>>,
    /// The previous byte was ESC.
    esc: bool,
}

impl Scanner {
    /// Feeds raw output; returns the newest percentage this chunk reported, if any.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<u8> {
        let mut found = None;
        for &b in bytes {
            if let Some(body) = &mut self.osc {
                let done = b == 0x07 || (self.esc && b == b'\\');
                if done {
                    found = osc_progress(body).or(found);
                    self.osc = None;
                    self.esc = false;
                } else if b == 0x1b {
                    self.esc = true;
                } else if self.esc || body.len() >= OSC_CAP {
                    // Malformed or not ours: drop it.
                    self.osc = None;
                    self.esc = false;
                } else {
                    body.push(b);
                }
                continue;
            }
            if self.esc {
                self.esc = false;
                if b == b']' {
                    self.osc = Some(Vec::new());
                    continue;
                }
            }
            match b {
                0x1b => self.esc = true,
                b'\r' | b'\n' => self.seg.clear(),
                _ => {
                    if b == b'%' {
                        found = percent_before(&self.seg).or(found);
                    }
                    if self.seg.len() < SEG_CAP {
                        self.seg.push(b);
                    }
                }
            }
        }
        found
    }
}

/// `9;4;<state>;<pct>`: state 1 is "normal progress"; clear, error, indeterminate and paused
/// don't carry a usable value.
fn osc_progress(body: &[u8]) -> Option<u8> {
    let s = std::str::from_utf8(body).ok()?;
    let mut parts = s.split(';');
    if (parts.next()?, parts.next()?, parts.next()?) != ("9", "4", "1") {
        return None;
    }
    let p: u32 = parts.next()?.trim().parse().ok()?;
    Some(p.min(100) as u8)
}

/// The number right before a `%`: 1–3 digits with an optional fraction, at most 100.
fn percent_before(seg: &[u8]) -> Option<u8> {
    let start = seg
        .iter()
        .rposition(|c| !(c.is_ascii_digit() || *c == b'.'))
        .map_or(0, |i| i + 1);
    let run = std::str::from_utf8(&seg[start..]).ok()?;
    let (int, frac) = run.split_once('.').unwrap_or((run, ""));
    let ok = (1..=3).contains(&int.len()) && frac.bytes().all(|c| c.is_ascii_digit());
    let p: u32 = int.parse().ok().filter(|_| ok)?;
    (p <= 100).then_some(p as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_9_4_with_bel_and_st() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"\x1b]9;4;1;42\x07"), Some(42));
        assert_eq!(s.feed(b"\x1b]9;4;1;77\x1b\\"), Some(77));
        // Clear, error and indeterminate carry no value.
        assert_eq!(
            s.feed(b"\x1b]9;4;0;0\x07\x1b]9;4;2;50\x07\x1b]9;4;3;0\x07"),
            None
        );
        // Other OSCs (window title) are ignored, and their digits don't leak into the text.
        assert_eq!(s.feed(b"\x1b]0;build 99%\x07"), None);
    }

    #[test]
    fn sequences_split_across_reads() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"\x1b]9;4"), None);
        assert_eq!(s.feed(b";1;6"), None);
        assert_eq!(s.feed(b"5\x1b"), None);
        assert_eq!(s.feed(b"\\"), Some(65));
        assert_eq!(s.feed(b"Receiving objects:  1"), None);
        assert_eq!(s.feed(b"2% (3/25)"), Some(12));
    }

    #[test]
    fn carriage_return_bars_report_the_latest_value() {
        let mut s = Scanner::default();
        let out = b"Receiving objects:  10% (1/10)\rReceiving objects:  40% (4/10)\r";
        assert_eq!(s.feed(out), Some(40));
        assert_eq!(s.feed(b"[####      ] 45.7%"), Some(45));
        assert_eq!(s.feed(b"\x1b[32m 88%\x1b[0m"), Some(88));
    }

    #[test]
    fn implausible_numbers_are_ignored() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"1000%\n"), None);
        assert_eq!(s.feed(b"101%\n"), None);
        assert_eq!(s.feed(b"%\n"), None);
        assert_eq!(s.feed(b"1.2.3%\n"), None);
        assert_eq!(s.feed(b"\x1b]9;4;1;250\x07"), Some(100));
        // A percentage in an ordinary log line still counts (`--no-progress` opts out).
        assert_eq!(s.feed(b"coverage: 73% of lines\n"), Some(73));
    }

    #[test]
    fn long_lines_stay_bounded() {
        let mut s = Scanner::default();
        let big = vec![b'x'; 10_000];
        assert_eq!(s.feed(&big), None);
        assert!(s.seg.len() <= SEG_CAP);
        let mut osc = b"\x1b]".to_vec();
        osc.extend(std::iter::repeat_n(b'9', 1000));
        assert_eq!(s.feed(&osc), None);
        assert!(s.osc.is_none());
    }
}
