//! Global microphone mute: every active capture endpoint, via IAudioEndpointVolume.
//! Changes made elsewhere (hardware key, Settings) arrive as endpoint-volume
//! callbacks, so the watcher sleeps until Windows says something changed; toggling
//! from Hytte updates the UI immediately. It reuses one device enumerator and the
//! activated endpoints, re-enumerating (and re-subscribing) only when Windows
//! reports a capture device was added, removed or changed state.

use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;

pub fn spawn_watcher(ui_tx: Sender<UiEvent>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        crate::proc::eco_thread();
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
    use crossbeam_channel::RecvTimeoutError;
    use std::time::Duration;
    use windows::core::{implement, GUID, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::Endpoints::{
        IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
    };
    use windows::Win32::Media::Audio::{
        eCapture, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
        IMMNotificationClient_Impl, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE,
        DEVICE_STATE_ACTIVE,
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

    /// What woke the watcher: the set of devices changed, or a mute flag did.
    #[derive(Clone, Copy)]
    enum Wake {
        Devices,
        Mute,
    }

    /// Tells the watcher the cached endpoint list is stale when the set of devices changes.
    #[implement(IMMNotificationClient)]
    struct DeviceChanges(Sender<Wake>);

    /// Tells the watcher an endpoint's mute (or volume) changed. Runs on a system thread.
    #[implement(IAudioEndpointVolumeCallback)]
    struct MuteChanges(Sender<Wake>);

    #[allow(non_snake_case)]
    impl IAudioEndpointVolumeCallback_Impl for MuteChanges_Impl {
        fn OnNotify(&self, _data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
            // A full queue already means "recheck": dropping this one loses nothing.
            let _ = self.0.try_send(Wake::Mute);
            Ok(())
        }
    }

    #[allow(non_snake_case)]
    impl IMMNotificationClient_Impl for DeviceChanges_Impl {
        fn OnDeviceStateChanged(
            &self,
            _id: &PCWSTR,
            _state: DEVICE_STATE,
        ) -> windows::core::Result<()> {
            let _ = self.0.try_send(Wake::Devices);
            Ok(())
        }
        fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
            let _ = self.0.try_send(Wake::Devices);
            Ok(())
        }
        fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
            let _ = self.0.try_send(Wake::Devices);
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

    /// Endpoints with a mute-change subscription each; unsubscribed when dropped.
    struct Subscribed {
        eps: Vec<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)>,
    }

    impl Subscribed {
        fn new(eps: Vec<IAudioEndpointVolume>, tx: &Sender<Wake>) -> Self {
            let eps = eps
                .into_iter()
                .map(|e| {
                    let cb: IAudioEndpointVolumeCallback = MuteChanges(tx.clone()).into();
                    unsafe {
                        let _ = e.RegisterControlChangeNotify(&cb);
                    }
                    (e, cb)
                })
                .collect();
            Self { eps }
        }
        fn muted(&self) -> bool {
            !self.eps.is_empty()
                && self
                    .eps
                    .iter()
                    .all(|(e, _)| unsafe { e.GetMute().map(|b| b.as_bool()).unwrap_or(false) })
        }
    }

    impl Drop for Subscribed {
        fn drop(&mut self) {
            for (e, cb) in &self.eps {
                unsafe {
                    let _ = e.UnregisterControlChangeNotify(cb);
                }
            }
        }
    }

    pub fn watch(ui_tx: Sender<UiEvent>) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let (tx, rx) = crossbeam_channel::bounded::<Wake>(8);
        let en = enumerator();
        let client: IMMNotificationClient = DeviceChanges(tx.clone()).into();
        let notified = en
            .as_ref()
            .is_some_and(|en| unsafe { en.RegisterEndpointNotificationCallback(&client) }.is_ok());
        let mut subs: Option<Subscribed> = None;
        let mut dirty = true;
        let mut last = None;
        loop {
            // Without device notifications we re-enumerate every wake, as before.
            if dirty || !notified {
                drop(subs.take()); // unsubscribe the old set first
                let eps = match &en {
                    Some(en) => endpoints_of(en),
                    None => endpoints(),
                };
                subs = Some(Subscribed::new(eps, &tx));
                dirty = false;
            }
            let m = subs.as_ref().is_some_and(Subscribed::muted);
            if last != Some(m) {
                last = Some(m);
                let _ = ui_tx.send(UiEvent::MicMute(m));
            }
            // Mute changes and device changes wake us. The timeout is only a safety net
            // (a missed callback); with no device notifications it is the old 2 s poll.
            let wait = Duration::from_secs(if notified { 60 } else { 2 });
            match rx.recv_timeout(wait) {
                Ok(Wake::Devices) => dirty = true,
                Ok(Wake::Mute) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Fold a burst (one toggle notifies every endpoint) into one recheck.
            while let Ok(w) = rx.try_recv() {
                dirty |= matches!(w, Wake::Devices);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Manual: toggles the real microphone mute and restores it. Run with
    /// `cargo test -p hytte mic -- --ignored`. Passes only if the endpoint callback (not
    /// the 60 s safety timeout) wakes the watcher.
    #[test]
    #[ignore]
    fn mute_change_wakes_the_watcher() {
        let (tx, rx) = crossbeam_channel::unbounded();
        spawn_watcher(tx);
        let first = loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(UiEvent::MicMute(m)) => break m,
                Ok(_) => {}
                Err(_) => panic!("no initial MicMute"),
            }
        };
        let flipped = toggle().expect("a capture device");
        assert_ne!(flipped, first);
        let seen = loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(UiEvent::MicMute(m)) => break m,
                Ok(_) => {}
                Err(_) => {
                    toggle();
                    panic!("watcher did not report the change within 5 s");
                }
            }
        };
        toggle();
        assert_eq!(seen, flipped);
    }
}
