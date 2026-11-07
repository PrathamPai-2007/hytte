//! Windows Do Not Disturb, for Focus sessions.
//!
//! Microsoft's `FocusSessionManager.TryStartFocusSession` needs a Limited Access token, so this
//! uses the setting the Quick Settings tile writes: `NOC_GLOBAL_SETTING_TOASTS_ENABLED` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings`. `0` means toasts are
//! off (Do Not Disturb on); absent or `1` means on. It is undocumented and could change.
//!
//! Tested 2026-11-07 on Windows 11 26H2: the value can be written, read back and restored. A
//! screenshot test could not show whether a toast banner is then suppressed (the control run,
//! with the switch off, showed no banner either), so the visible effect is confirmed by hand.
//!
//! `enter` remembers what the value was in `dnd-restore.txt`, so if Hytte dies mid-session the
//! next start puts it back (`recover`).

use std::path::PathBuf;

const SUB: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings";
const NAME: &str = "NOC_GLOBAL_SETTING_TOASTS_ENABLED";

/// What the toasts value held: `None` when it was absent.
pub type Raw = Option<u32>;

/// True when `raw` means Do Not Disturb is on.
pub fn is_on(raw: Raw) -> bool {
    raw == Some(0)
}

fn marker() -> PathBuf {
    crate::config::data_dir().join("dnd-restore.txt")
}

fn encode(raw: Raw) -> String {
    raw.map_or("absent".into(), |v| v.to_string())
}

fn decode(s: &str) -> Option<Raw> {
    match s.trim() {
        "absent" => Some(None),
        n => n.parse().ok().map(Some),
    }
}

#[cfg(windows)]
mod reg {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn read() -> Raw {
        let (sub, name) = (wide(SUB), wide(NAME));
        let mut v = 0u32;
        let mut len = 4u32;
        unsafe {
            let r = RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut v as *mut u32 as *mut _),
                Some(&mut len),
            );
            (r == ERROR_SUCCESS).then_some(v)
        }
    }

    pub fn write(raw: Raw) -> bool {
        let (sub, name) = (wide(SUB), wide(NAME));
        let mut key = HKEY::default();
        unsafe {
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                KEY_SET_VALUE,
                &mut key,
            ) != ERROR_SUCCESS
            {
                return false;
            }
            let ok = match raw {
                Some(v) => {
                    RegSetValueExW(
                        key,
                        PCWSTR(name.as_ptr()),
                        None,
                        REG_DWORD,
                        Some(&v.to_le_bytes()),
                    ) == ERROR_SUCCESS
                }
                None => {
                    let r = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
                    r == ERROR_SUCCESS || r.0 == 2 // already absent
                }
            };
            let _ = RegCloseKey(key);
            ok
        }
    }
}

#[cfg(not(windows))]
mod reg {
    use super::Raw;
    pub fn read() -> Raw {
        None
    }
    pub fn write(_: Raw) -> bool {
        false
    }
}

pub fn read() -> Raw {
    reg::read()
}

/// Turn Do Not Disturb on. Returns what to hand to `leave` afterwards, or `None` when the write
/// did not take (the caller then just mutes Hytte's own alerts). Already on: nothing changes and
/// nothing needs restoring (`Some(Some(0))` would re-write the same value, harmless).
pub fn enter() -> Option<Raw> {
    let prev = read();
    if is_on(prev) {
        return Some(prev);
    }
    let _ = std::fs::write(marker(), encode(prev));
    if reg::write(Some(0)) && is_on(read()) {
        return Some(prev);
    }
    // It did not take: put back what was there and forget the marker.
    reg::write(prev);
    let _ = std::fs::remove_file(marker());
    None
}

/// Put the value back as `enter` found it.
pub fn leave(prev: Raw) {
    if reg::write(prev) {
        let _ = std::fs::remove_file(marker());
    }
}

/// At startup: undo a Do Not Disturb that a crashed session left on.
pub fn recover() {
    if let Some(prev) = std::fs::read_to_string(marker())
        .ok()
        .and_then(|s| decode(&s))
    {
        leave(prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_zero_is_do_not_disturb() {
        assert!(is_on(Some(0)));
        assert!(!is_on(Some(1)));
        assert!(!is_on(None));
    }

    #[test]
    fn marker_text_round_trips() {
        for raw in [None, Some(0), Some(1)] {
            assert_eq!(decode(&encode(raw)), Some(raw));
        }
        assert_eq!(decode("garbage"), None);
    }

    /// Manual: writes the real setting, reads it back and restores it.
    /// `cargo test -p hytte dnd -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_switch_round_trips() {
        let before = read();
        let prev = enter().expect("write did not take");
        assert!(is_on(read()), "read-back after enter");
        leave(prev);
        assert_eq!(read(), before, "restored");
        assert!(!marker().exists());
        println!("ok: before = {before:?}");
    }
}
