//! Global microphone mute: every active capture endpoint, via IAudioEndpointVolume.
//! The state is polled (2 s) so changes made elsewhere (hardware key, Settings)
//! show up too; toggling from Hytte updates the UI immediately.

use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;

pub fn spawn_watcher(ui_tx: Sender<UiEvent>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        imp::watch(ui_tx);
        #[cfg(not(windows))]
        {
            let _ = ui_tx;
        }
    });
}

/// Mute every capture device, or unmute them all when they are already muted.
/// Returns the new state, or None when there is no microphone.
pub fn toggle() -> Option<bool> {
    #[cfg(windows)]
    {
        imp::toggle()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::GUID;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eCapture, IMMDeviceEnumerator, DEVICE_STATE_ACTIVE};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    const MM_DEVICE_ENUMERATOR: GUID = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);

    fn endpoints() -> Vec<IAudioEndpointVolume> {
        unsafe {
            let Ok(en) =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MM_DEVICE_ENUMERATOR, None, CLSCTX_ALL)
            else {
                return vec![];
            };
            let Ok(col) = en.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else {
                return vec![];
            };
            let n = col.GetCount().unwrap_or(0);
            (0..n)
                .filter_map(|i| {
                    col.Item(i)
                        .ok()?
                        .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                        .ok()
                })
                .collect()
        }
    }

    fn muted(eps: &[IAudioEndpointVolume]) -> bool {
        !eps.is_empty()
            && eps
                .iter()
                .all(|e| unsafe { e.GetMute().map(|b| b.as_bool()).unwrap_or(false) })
    }

    pub fn toggle() -> Option<bool> {
        let eps = endpoints();
        if eps.is_empty() {
            return None;
        }
        let want = !muted(&eps);
        for e in &eps {
            unsafe {
                let _ = e.SetMute(want, std::ptr::null());
            }
        }
        Some(muted(&eps))
    }

    pub fn watch(ui_tx: Sender<UiEvent>) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut last = None;
        loop {
            let m = muted(&endpoints());
            if last != Some(m) {
                last = Some(m);
                let _ = ui_tx.send(UiEvent::MicMute(m));
            }
            // ponytail: 2 s poll; IAudioEndpointVolume callbacks if this ever shows in profiles
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
}
