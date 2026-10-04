//! Global microphone mute: every active capture endpoint, via IAudioEndpointVolume.
//! The state is polled (2 s) so changes made elsewhere (hardware key, Settings)
//! show up too; toggling from Hytte updates the UI immediately. The poll reuses
//! one device enumerator and the activated endpoints, re-enumerating only when
//! Windows reports a capture device was added, removed or changed state.

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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::{implement, GUID, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eCapture, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
        IMMNotificationClient_Impl, DEVICE_STATE, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    const MM_DEVICE_ENUMERATOR: GUID = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);

    fn enumerator() -> Option<IMMDeviceEnumerator> {
        unsafe { CoCreateInstance(&MM_DEVICE_ENUMERATOR, None, CLSCTX_ALL).ok() }
    }

    fn endpoints() -> Vec<IAudioEndpointVolume> {
        enumerator().map(|en| endpoints_of(&en)).unwrap_or_default()
    }

    fn endpoints_of(en: &IMMDeviceEnumerator) -> Vec<IAudioEndpointVolume> {
        unsafe {
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

    /// Flags the cached endpoint list stale when the set of devices changes.
    #[implement(IMMNotificationClient)]
    struct DeviceChanges(Arc<AtomicBool>);

    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for DeviceChanges_Impl {
        fn OnDeviceStateChanged(
            &self,
            _id: &PCWSTR,
            _state: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            self.0.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
            self.0.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
            self.0.store(true, Ordering::Relaxed);
            Ok(())
        }
        fn OnDefaultDeviceChanged(
            &self,
            _flow: EDataFlow,
            _role: ERole,
            _id: &PCWSTR,
        ) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnPropertyValueChanged(
            &self,
            _id: &PCWSTR,
            _key: &PROPERTYKEY,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    pub fn watch(ui_tx: Sender<UiEvent>) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        // Without device notifications we re-enumerate every poll, as before.
        let dirty = Arc::new(AtomicBool::new(true));
        let en = enumerator();
        let client: IMMNotificationClient = DeviceChanges(dirty.clone()).into();
        let notified = en
            .as_ref()
            .is_some_and(|en| unsafe { en.RegisterEndpointNotificationCallback(&client) }.is_ok());
        let mut eps = vec![];
        let mut last = None;
        loop {
            if !notified || dirty.swap(false, Ordering::Relaxed) {
                eps = match &en {
                    Some(en) => endpoints_of(en),
                    None => endpoints(),
                };
            }
            let m = muted(&eps);
            if last != Some(m) {
                last = Some(m);
                let _ = ui_tx.send(UiEvent::MicMute(m));
            }
            // ponytail: 2 s poll; IAudioEndpointVolume callbacks if this ever shows in profiles
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }
}
