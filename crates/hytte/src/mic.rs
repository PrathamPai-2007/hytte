//! Global microphone mute (every active capture endpoint) and the default speaker's volume
//! and mute, via IAudioEndpointVolume.
//! Changes made elsewhere (hardware key, the volume flyout, Settings) arrive as endpoint-volume
//! callbacks, so the watcher sleeps until Windows says something changed; changing them
//! from Hytte updates the UI immediately. It reuses one device enumerator and the
//! activated endpoints, re-enumerating (and re-subscribing) only when Windows
//! reports a device was added, removed, changed state or became the default.

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

/// The default speaker: (volume 0..1, muted), or None without an output device.
pub fn speaker() -> Option<(f32, bool)> {
    #[cfg(windows)]
    {
        imp::speaker_state(&imp::speaker_endpoint()?)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Set the default speaker's volume (0..1); turning it up also unmutes. Returns the new state.
pub fn set_volume(v: f32) -> Option<(f32, bool)> {
    #[cfg(windows)]
    {
        let e = imp::speaker_endpoint()?;
        unsafe {
            let _ = e.SetMasterVolumeLevelScalar(v.clamp(0.0, 1.0), std::ptr::null());
            if v > 0.0 {
                let _ = e.SetMute(false, std::ptr::null());
            }
        }
        imp::speaker_state(&e)
    }
    #[cfg(not(windows))]
    {
        let _ = v;
        None
    }
}

/// Mute or unmute the default speaker. Returns the new state.
pub fn toggle_speaker() -> Option<(f32, bool)> {
    #[cfg(windows)]
    {
        let e = imp::speaker_endpoint()?;
        let (_, muted) = imp::speaker_state(&e)?;
        unsafe {
            let _ = e.SetMute(!muted, std::ptr::null());
        }
        imp::speaker_state(&e)
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
        eCapture, eConsole, eRender, EDataFlow, ERole, IMMDeviceEnumerator, IMMNotificationClient,
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

    fn speaker_of(en: &IMMDeviceEnumerator) -> Option<IAudioEndpointVolume> {
        unsafe {
            en.GetDefaultAudioEndpoint(eRender, eConsole)
                .ok()?
                .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                .ok()
        }
    }

    pub fn speaker_endpoint() -> Option<IAudioEndpointVolume> {
        speaker_of(&enumerator()?)
    }

    pub fn speaker_state(e: &IAudioEndpointVolume) -> Option<(f32, bool)> {
        unsafe {
            let v = e.GetMasterVolumeLevelScalar().ok()?;
            let m = e.GetMute().ok()?.as_bool();
            Some(((v * 100.0).round() / 100.0, m))
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
        fn OnNotify(
            &self,
            _data: *mut AUDIO_VOLUME_NOTIFICATION_DATA,
        ) -> windows::core::Result<()> {
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
            flow: EDataFlow,
            role: ERole,
            _id: &PCWSTR,
        ) -> windows::core::Result<()> {
            // A new default speaker (headphones plugged in): follow it.
            if flow == eRender && role == eConsole {
                let _ = self.0.try_send(Wake::Devices);
            }
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
        let mut spk: Option<Subscribed> = None;
        let mut dirty = true;
        let mut last = None;
        let mut last_spk = None;
        loop {
            // Without device notifications we re-enumerate every wake, as before.
            if dirty || !notified {
                drop(subs.take()); // unsubscribe the old set first
                drop(spk.take());
                let (eps, speaker) = match &en {
                    Some(en) => (endpoints_of(en), speaker_of(en)),
                    None => (endpoints(), speaker_endpoint()),
                };
                subs = Some(Subscribed::new(eps, &tx));
                spk = Some(Subscribed::new(speaker.into_iter().collect(), &tx));
                dirty = false;
            }
            let m = subs.as_ref().is_some_and(Subscribed::muted);
            if last != Some(m) {
                last = Some(m);
                let _ = ui_tx.send(UiEvent::MicMute(m));
            }
            let s = spk
                .as_ref()
                .and_then(|s| s.eps.first())
                .and_then(|(e, _)| speaker_state(e));
            if last_spk != Some(s) {
                last_spk = Some(s);
                let _ = ui_tx.send(UiEvent::Speaker(s));
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
