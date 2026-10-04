//! Privacy dots via ConsentStore registry.
//! One mechanism for camera + mic: RegNotifyChangeKeyValue on
//! HKCU\...\CapabilityAccessManager\ConsentStore\{webcam,microphone}.
//! "In use" = LastUsedTimeStart != 0 and LastUsedTimeStop == 0, for packaged
//! apps (direct subkeys) and NonPackaged apps (one level deeper).

use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;

const BASE: &str =
    "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\";

pub fn spawn_watcher(ui_tx: Sender<UiEvent>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        watch_loop(ui_tx);
        #[cfg(not(windows))]
        {
            let _ = ui_tx;
        }
    });
}

/// Registry subkey -> user-facing app name.
/// `Microsoft.WindowsCamera_8wekyb3d8bbwe` -> `WindowsCamera`,
/// `C:#Program Files#Zoom#bin#Zoom.exe` -> `Zoom`.
pub fn friendly_name(key: &str) -> String {
    if key.contains('#') {
        let file = key.rsplit('#').next().unwrap_or(key);
        return file
            .strip_suffix(".exe")
            .or_else(|| file.strip_suffix(".EXE"))
            .unwrap_or(file)
            .to_string();
    }
    let pkg = key.split('_').next().unwrap_or(key);
    pkg.rsplit('.').next().unwrap_or(pkg).to_string()
}

#[cfg(windows)]
fn watch_loop(ui_tx: Sender<UiEvent>) {
    use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE};
    use windows::Win32::System::Registry::*;
    use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
    let mut events: Vec<HANDLE> = vec![];
    let mut keys: Vec<HKEY> = vec![];
    unsafe {
        for sub in ["webcam", "microphone"] {
            let w: Vec<u16> = format!("{BASE}{sub}").encode_utf16().chain([0]).collect();
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(w.as_ptr()),
                None,
                KEY_NOTIFY,
                &mut hkey,
            ) == ERROR_SUCCESS
            {
                if let Ok(ev) = CreateEventW(None, false, false, None) {
                    let _ = RegNotifyChangeKeyValue(
                        hkey,
                        true,
                        REG_NOTIFY_CHANGE_LAST_SET,
                        Some(ev),
                        true,
                    );
                    events.push(ev);
                    keys.push(hkey);
                }
            }
        }
    }
    let mut last = None;
    push_state(&ui_tx, &mut last);
    loop {
        if events.is_empty() {
            // ponytail: ConsentStore missing -> slow fallback poll, upgrade if ever seen in the wild
            std::thread::sleep(std::time::Duration::from_secs(10));
            push_state(&ui_tx, &mut last);
            continue;
        }
        unsafe {
            // 10 s timeout is a safety net for missed notifications, not a poll.
            let rc = WaitForMultipleObjects(&events, false, 10_000);
            let i = rc.0 as usize;
            if i < events.len() {
                // Re-arm only the key that fired.
                let _ = RegNotifyChangeKeyValue(
                    keys[i],
                    true,
                    REG_NOTIFY_CHANGE_LAST_SET,
                    Some(events[i]),
                    true,
                );
            }
        }
        push_state(&ui_tx, &mut last);
    }
}

#[cfg(windows)]
fn push_state(ui_tx: &Sender<UiEvent>, last: &mut Option<(bool, bool, Option<String>)>) {
    let cam = in_use("webcam");
    let mic = in_use("microphone");
    let state = (cam.is_some(), mic.is_some(), cam.or(mic));
    if last.as_ref() != Some(&state) {
        *last = Some(state.clone());
        let _ = ui_tx.send(UiEvent::Privacy(state.0, state.1, state.2));
    }
}

/// Some(app) when a device is currently in use.
#[cfg(windows)]
fn in_use(device: &str) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;
    unsafe fn open(parent: HKEY, path: &str) -> Option<HKEY> {
        let w: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let mut h = HKEY::default();
        (unsafe {
            RegOpenKeyExW(
                parent,
                windows::core::PCWSTR(w.as_ptr()),
                None,
                KEY_READ,
                &mut h,
            )
        } == ERROR_SUCCESS)
            .then_some(h)
    }
    unsafe fn children(h: HKEY) -> Vec<String> {
        let mut out = vec![];
        let mut i = 0;
        loop {
            let mut buf = vec![0u16; 512];
            let mut len = buf.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    h,
                    i,
                    Some(windows::core::PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if rc != ERROR_SUCCESS || i > 512 {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
            i += 1;
        }
        out
    }
    unsafe {
        let root = open(HKEY_CURRENT_USER, &format!("{BASE}{device}"))?;
        let mut found = None;
        'outer: for name in children(root) {
            let Some(k) = open(root, &name) else { continue };
            if active(k) {
                found = Some(friendly_name(&name));
            } else if name == "NonPackaged" {
                for np in children(k) {
                    if let Some(c) = open(k, &np) {
                        let a = active(c);
                        let _ = RegCloseKey(c);
                        if a {
                            found = Some(friendly_name(&np));
                            let _ = RegCloseKey(k);
                            break 'outer;
                        }
                    }
                }
            }
            let _ = RegCloseKey(k);
            if found.is_some() {
                break;
            }
        }
        let _ = RegCloseKey(root);
        found
    }
}

#[cfg(windows)]
fn active(k: windows::Win32::System::Registry::HKEY) -> bool {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::*;
    fn q(k: HKEY, name: &str) -> Option<u64> {
        let n: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut data = [0u8; 8];
        let mut len = 8u32;
        let rc = unsafe {
            RegQueryValueExW(
                k,
                windows::core::PCWSTR(n.as_ptr()),
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut len),
            )
        };
        (rc == ERROR_SUCCESS && len == 8).then(|| u64::from_le_bytes(data))
    }
    q(k, "LastUsedTimeStart").is_some_and(|s| s != 0) && q(k, "LastUsedTimeStop") == Some(0)
}

#[cfg(test)]
mod tests {
    use super::friendly_name;

    #[test]
    fn names() {
        assert_eq!(
            friendly_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"),
            "WindowsCamera"
        );
        assert_eq!(friendly_name("C:#Program Files#Zoom#bin#Zoom.exe"), "Zoom");
        assert_eq!(friendly_name("chrome"), "chrome");
    }
}
