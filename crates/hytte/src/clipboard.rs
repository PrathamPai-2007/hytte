//! What Hytte does with the clipboard, decided in plain functions so the rules can be tested:
//!
//! * `[shelf] shelve_copied`: files copied (not cut) in Explorer go on the shelf, by reference.
//! * `[clipboard] history`: a short text history under the shelf tiles, in memory only.
//!
//! Both are off unless switched on, and both skip what Hytte itself put on the clipboard and
//! anything an app marked as private (password managers set `ExcludeClipboardContentFromMonitor
//! Processing`; Windows' own history honours `CanIncludeInClipboardHistory`).

use std::collections::VecDeque;
use std::path::PathBuf;

/// Clips kept, and the longest one (bytes). A longer text is skipped, not cut: a clip that
/// pasted back wrong would be worse than none.
pub const MAX_CLIPS: usize = 20;
pub const MAX_CLIP_BYTES: usize = 4096;
/// Clips shown at once in the Shelf pane.
pub const CLIP_ROWS: usize = 5;

const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

/// What was on the clipboard when it changed, as far as the rules care.
#[derive(Debug, Default, Clone)]
pub struct Formats {
    /// `CF_HDROP`: a list of files, as the shell writes it.
    pub hdrop: Option<Vec<u8>>,
    /// `Preferred DropEffect`: what the owner meant (copy, move, link).
    pub effect: Option<u32>,
    /// `ExcludeClipboardContentFromMonitorProcessing` or `Clipboard Viewer Ignore` is there.
    pub excluded: bool,
    /// `CanIncludeInClipboardHistory` is there and says 0.
    pub history_off: bool,
    /// The text, if there is some.
    pub text: Option<String>,
}

/// The files of a `CF_HDROP` (a `DROPFILES` header, then paths ending in a double null).
pub fn parse_hdrop(b: &[u8]) -> Vec<PathBuf> {
    let u32_at = |i: usize| {
        b.get(i..i + 4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    };
    let (Some(start), Some(wide)) = (u32_at(0), u32_at(16)) else {
        return vec![];
    };
    let Some(body) = b.get(start as usize..) else {
        return vec![];
    };
    let mut out = vec![];
    if wide != 0 {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        for p in units.split(|u| *u == 0).take_while(|p| !p.is_empty()) {
            out.push(PathBuf::from(String::from_utf16_lossy(p)));
        }
    } else {
        for p in body.split(|c| *c == 0).take_while(|p| !p.is_empty()) {
            out.push(PathBuf::from(String::from_utf8_lossy(p).into_owned()));
        }
    }
    out
}

/// The files to put on the shelf for this clipboard: those of a copy (not a cut, not a plain
/// "move"), when the owner didn't ask to be left alone. Without a `Preferred DropEffect` nothing
/// is taken: a file list that doesn't say what it is for isn't ours to guess.
pub fn files_to_shelve(f: &Formats) -> Vec<PathBuf> {
    let Some(effect) = f.effect else {
        return vec![];
    };
    if f.excluded || effect & DROPEFFECT_COPY == 0 || effect & DROPEFFECT_MOVE != 0 {
        return vec![];
    }
    f.hdrop.as_deref().map(parse_hdrop).unwrap_or_default()
}

/// The app's name as compared with the ignore list: lower case, without `.exe`.
fn stem(exe: &str) -> String {
    let e = exe.trim().to_ascii_lowercase();
    e.strip_suffix(".exe").unwrap_or(&e).to_string()
}

/// The text to remember for this clipboard, or None: nothing to keep when an app marked it
/// private, when it came from a program on the ignore list, when it is empty or past
/// [`MAX_CLIP_BYTES`], or when it looks like a secret (`secrets::mask` would change it).
pub fn clip_to_record(f: &Formats, owner_exe: Option<&str>, ignore: &[String]) -> Option<String> {
    if f.excluded || f.history_off {
        return None;
    }
    if let Some(exe) = owner_exe {
        if ignore.iter().any(|i| stem(i) == stem(exe)) {
            return None;
        }
    }
    let text = f.text.as_deref()?.trim();
    if text.is_empty() || text.len() > MAX_CLIP_BYTES || crate::secrets::mask(text) != text {
        return None;
    }
    Some(text.to_string())
}

/// The clipboard history: newest first, repeats move to the front, at most [`MAX_CLIPS`].
/// Never written anywhere.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct History(VecDeque<String>);

impl History {
    pub fn add(&mut self, text: &str) {
        self.0.retain(|t| t != text);
        self.0.push_front(text.to_string());
        self.0.truncate(MAX_CLIPS);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn get(&self, i: usize) -> Option<&str> {
        self.0.get(i).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrop(paths: &[&str], wide: bool) -> Vec<u8> {
        let mut b = vec![0u8; 20];
        b[0..4].copy_from_slice(&20u32.to_le_bytes());
        b[16..20].copy_from_slice(&(wide as u32).to_le_bytes());
        for p in paths {
            if wide {
                for u in p.encode_utf16().chain([0]) {
                    b.extend(u.to_le_bytes());
                }
            } else {
                b.extend(p.bytes());
                b.push(0);
            }
        }
        b.extend(if wide { vec![0, 0] } else { vec![0] });
        b
    }

    fn files(effect: Option<u32>) -> Formats {
        Formats {
            hdrop: Some(hdrop(&[r"C:\a\x.txt", r"C:\b\y z.png"], true)),
            effect,
            ..Default::default()
        }
    }

    /// Live check (`cargo test -- --ignored shell_clipboard`): a file list on the real clipboard
    /// with Explorer's "Preferred DropEffect" is read the way `on_clipboard` reads it, and a cut is
    /// left alone. (It overwrites your clipboard.)
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn reads_what_the_shell_writes_on_the_real_clipboard() {
        use windows::Win32::System::DataExchange::*;
        let file = std::env::temp_dir().join("hytte-clip-live.txt");
        std::fs::write(&file, "x").unwrap();
        let read = |effect: u32| {
            let ps = format!(
                "Add-Type -AssemblyName System.Windows.Forms;                  $o = New-Object Windows.Forms.DataObject;                  $l = New-Object Collections.Specialized.StringCollection; [void]$l.Add('{}');                  $o.SetFileDropList($l);                  $o.SetData('Preferred DropEffect', [IO.MemoryStream]::new([BitConverter]::GetBytes([uint32]{effect})));                  [Windows.Forms.Clipboard]::SetDataObject($o, $true)",
                file.display()
            );
            let st = std::process::Command::new("powershell")
                .args(["-STA", "-NoProfile", "-Command", &ps])
                .status()
                .unwrap();
            assert!(st.success());
            let mut f = Formats::default();
            unsafe {
                OpenClipboard(None).unwrap();
                let bytes = |fmt: u32| {
                    let h = GetClipboardData(fmt).ok()?;
                    let h = windows::Win32::Foundation::HGLOBAL(h.0);
                    let p = windows::Win32::System::Memory::GlobalLock(h) as *const u8;
                    let v = std::slice::from_raw_parts(
                        p,
                        windows::Win32::System::Memory::GlobalSize(h),
                    )
                    .to_vec();
                    let _ = windows::Win32::System::Memory::GlobalUnlock(h);
                    Some(v)
                };
                f.hdrop = bytes(15);
                let eff = RegisterClipboardFormatW(windows::core::w!("Preferred DropEffect"));
                f.effect =
                    bytes(eff).and_then(|b| Some(u32::from_le_bytes(b.get(..4)?.try_into().ok()?)));
                let _ = CloseClipboard();
            }
            f
        };
        let copy = read(5);
        assert_eq!(copy.effect, Some(5));
        assert_eq!(files_to_shelve(&copy), vec![file.clone()]);
        let cut = read(2);
        assert_eq!(cut.effect, Some(2));
        assert!(files_to_shelve(&cut).is_empty());
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn a_file_list_is_read_wide_or_narrow() {
        let want = vec![PathBuf::from(r"C:\a\x.txt"), PathBuf::from(r"C:\b\y z.png")];
        assert_eq!(
            parse_hdrop(&hdrop(&[r"C:\a\x.txt", r"C:\b\y z.png"], true)),
            want
        );
        assert_eq!(
            parse_hdrop(&hdrop(&[r"C:\a\x.txt", r"C:\b\y z.png"], false)),
            want
        );
        assert_eq!(
            parse_hdrop(&hdrop(&["C:\\日本\\ふ.txt"], true)),
            vec![PathBuf::from("C:\\日本\\ふ.txt")]
        );
        // Garbage and short buffers are nothing, not a panic.
        assert!(parse_hdrop(&[]).is_empty() && parse_hdrop(&[1, 2, 3]).is_empty());
        let mut bad = hdrop(&["C:\\a"], true);
        bad[0..4].copy_from_slice(&9999u32.to_le_bytes());
        assert!(parse_hdrop(&bad).is_empty());
    }

    #[test]
    fn a_copy_is_shelved_but_a_cut_is_not() {
        assert_eq!(files_to_shelve(&files(Some(1))).len(), 2, "copy");
        // Explorer's Ctrl+C says copy-or-link.
        assert_eq!(files_to_shelve(&files(Some(5))).len(), 2, "copy | link");
        assert!(files_to_shelve(&files(Some(2))).is_empty(), "cut");
        assert!(files_to_shelve(&files(Some(3))).is_empty(), "copy | move");
        assert!(files_to_shelve(&files(Some(4))).is_empty(), "link only");
        // No stated purpose: not guessed.
        assert!(files_to_shelve(&files(None)).is_empty());
        // Nothing but a file list counts.
        let text_only = Formats {
            effect: Some(1),
            ..Default::default()
        };
        assert!(files_to_shelve(&text_only).is_empty());
    }

    #[test]
    fn an_app_that_asks_to_be_left_alone_is_left_alone() {
        let mut f = files(Some(1));
        f.excluded = true;
        assert!(files_to_shelve(&f).is_empty());
    }

    fn text(t: &str) -> Formats {
        Formats {
            text: Some(t.into()),
            ..Default::default()
        }
    }

    #[test]
    fn text_is_recorded_unless_a_rule_says_no() {
        let none: Vec<String> = vec![];
        assert_eq!(
            clip_to_record(&text("  hello \n"), None, &none).as_deref(),
            Some("hello")
        );
        // Marked private (what password managers and browsers' password fields do).
        let mut f = text("hunter22");
        f.excluded = true;
        assert_eq!(clip_to_record(&f, None, &none), None);
        let mut f = text("hello");
        f.history_off = true;
        assert_eq!(clip_to_record(&f, None, &none), None);
        // From a program on the ignore list, whatever the case or the .exe.
        let ignore = vec!["KeePassXC".to_string(), "1password.exe".to_string()];
        assert_eq!(
            clip_to_record(&text("a b"), Some("keepassxc.exe"), &ignore),
            None
        );
        assert_eq!(
            clip_to_record(&text("a b"), Some("1Password.exe"), &ignore),
            None
        );
        assert_eq!(
            clip_to_record(&text("a b"), Some("notepad.exe"), &ignore).as_deref(),
            Some("a b")
        );
        // Empty, or no text at all.
        assert_eq!(clip_to_record(&text("   "), None, &none), None);
        assert_eq!(clip_to_record(&Formats::default(), None, &none), None);
        // The size limit skips rather than cuts.
        assert!(clip_to_record(&text(&"x ".repeat(MAX_CLIP_BYTES / 2)), None, &none).is_some());
        assert_eq!(
            clip_to_record(&text(&"x".repeat(MAX_CLIP_BYTES + 1)), None, &none),
            None
        );
    }

    #[test]
    fn a_clip_that_looks_like_a_secret_is_refused() {
        let none: Vec<String> = vec![];
        for s in [
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "AKIAIOSFODNN7EXAMPLE",
            "export TOKEN=sk-abcdefghijklmnopqrstuvwxyz0123456789",
        ] {
            let masked = crate::secrets::mask(s) != s;
            assert_eq!(
                clip_to_record(&text(s), None, &none).is_none(),
                masked,
                "{s}"
            );
        }
        // Whatever mask changes, the rule refuses (checked against a case mask certainly changes).
        let s = "Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345";
        assert!(
            crate::secrets::mask(s) != s,
            "the test needs a string mask changes"
        );
        assert_eq!(clip_to_record(&text(s), None, &none), None);
    }

    #[test]
    fn history_keeps_twenty_newest_first_and_forgets_on_clear() {
        let mut h = History::default();
        for i in 0..25 {
            h.add(&format!("clip {i}"));
        }
        assert_eq!(h.len(), MAX_CLIPS);
        assert_eq!(h.get(0), Some("clip 24"));
        assert_eq!(h.get(MAX_CLIPS - 1), Some("clip 5"));
        // A repeat moves to the front instead of twice in the list.
        h.add("clip 10");
        assert_eq!(h.get(0), Some("clip 10"));
        assert_eq!(
            (0..h.len())
                .filter(|i| h.get(*i) == Some("clip 10"))
                .count(),
            1
        );
        assert_eq!(h.len(), MAX_CLIPS);
        h.clear();
        assert_eq!((h.len(), h.get(0)), (0, None));
    }
}
