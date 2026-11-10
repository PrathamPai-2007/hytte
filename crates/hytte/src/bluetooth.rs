//! Bluetooth connect card (`[moments] bluetooth`): a card naming the earbuds, mouse or
//! keyboard that just connected, with its battery when the device reports one.
//!
//! Event-driven: two WinRT `DeviceWatcher`s (classic and low-energy devices, filtered to those
//! *connected*) call back when a device connects; between connections the thread only waits.
//! The devices already connected when Hytte starts are the watcher's first batch, which is
//! skipped (`EnumerationCompleted` marks its end), so starting Hytte does not announce them.

use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A device that drops and comes back within this is one connection, not two cards.
pub const FLAP: Duration = Duration::from_secs(15);

/// The property Windows keeps a Bluetooth device's battery percentage in.
pub const BATTERY_KEY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";

static ENABLED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static UI: OnceLock<Sender<UiEvent>> = OnceLock::new();

/// `[moments] bluetooth`, live. The watcher thread starts the first time it is on and then
/// just stops reporting while it is off. `ui` is given once, at start.
pub fn configure(enabled: bool, ui: Option<Sender<UiEvent>>) {
    if let Some(ui) = ui {
        let _ = UI.set(ui);
    }
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled && !STARTED.swap(true, Ordering::AcqRel) {
        if let Some(ui) = UI.get().cloned() {
            std::thread::spawn(move || {
                #[cfg(windows)]
                {
                    crate::proc::eco_thread();
                    imp::run(ui);
                }
                #[cfg(not(windows))]
                let _ = ui;
            });
        }
    }
}

/// A device by name, or a plain word when it has none.
pub fn clean_name(name: &str) -> String {
    let n: String = name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect();
    if n.trim().is_empty() {
        "Bluetooth device".to_string()
    } else {
        n.trim().to_string()
    }
}

/// A battery reading, when it is a percentage.
pub fn battery_percent(v: i64) -> Option<u8> {
    (0..=100).contains(&v).then_some(v as u8)
}

/// When each device was last heard of, to tell a new connection from a flap.
#[derive(Default)]
pub struct Seen(HashMap<String, Instant>);

impl Seen {
    /// Whether a connection of `id` at `now` is news: the first time, or after [`FLAP`] of
    /// quiet. Every report counts as activity, so a device that keeps flapping stays quiet.
    pub fn is_new(&mut self, id: &str, now: Instant) -> bool {
        let new = self
            .0
            .get(id)
            .is_none_or(|t| now.saturating_duration_since(*t) >= FLAP);
        self.0.insert(id.to_string(), now);
        new
    }
}

/// The card's text: the device, then its battery when known.
pub fn card_text(name: &str, battery: Option<u8>) -> (String, Option<String>) {
    (
        format!("{} connected", clean_name(name)),
        battery.map(|b| format!("Battery {b}%")),
    )
}

/// What the watcher found out, for the UI thread.
pub fn announce(name: &str, battery: Option<u8>) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(ui) = UI.get() {
        let _ = ui.send(UiEvent::Bluetooth {
            name: clean_name(name),
            battery,
        });
    }
}

/// Seen devices, shared by both watchers' callbacks.
static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

fn is_new(id: &str) -> bool {
    SEEN.lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Seen::default)
        .is_new(id, Instant::now())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::Interface;
    use windows::core::HSTRING;
    use windows::Devices::Bluetooth::{
        BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
    };
    use windows::Devices::Enumeration::{DeviceInformation, DeviceWatcher};
    use windows::Foundation::{IReference, TypedEventHandler};
    use windows_collections::IIterable;

    /// The device's battery from its extra properties, if Windows has one.
    fn battery_of(info: &DeviceInformation) -> Option<u8> {
        let v = info
            .Properties()
            .ok()?
            .Lookup(&HSTRING::from(BATTERY_KEY))
            .ok()?;
        v.cast::<IReference<u8>>()
            .ok()
            .and_then(|r| r.Value().ok())
            .map(i64::from)
            .or_else(|| {
                v.cast::<IReference<i32>>()
                    .ok()
                    .and_then(|r| r.Value().ok())
                    .map(i64::from)
            })
            .or_else(|| {
                v.cast::<IReference<u32>>()
                    .ok()
                    .and_then(|r| r.Value().ok())
                    .map(i64::from)
            })
            .and_then(battery_percent)
    }

    /// A watcher for connected devices of the kind `selector` describes. Devices found before
    /// the first batch is complete were connected already and stay quiet.
    fn watch(selector: HSTRING) -> Option<DeviceWatcher> {
        let props = IIterable::<HSTRING>::from(vec![HSTRING::from(BATTERY_KEY)]);
        let w = DeviceInformation::CreateWatcherAqsFilterAndAdditionalProperties(&selector, &props)
            .ok()?;
        let ready = std::sync::Arc::new(AtomicBool::new(false));
        let r1 = ready.clone();
        let _ = w.EnumerationCompleted(&TypedEventHandler::<
            DeviceWatcher,
            windows::core::IInspectable,
        >::new(move |_, _| {
            r1.store(true, Ordering::Release);
            Ok(())
        }));
        let _ = w.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
            move |_, info| {
                if let (true, Some(info)) = (ready.load(Ordering::Acquire), info.as_ref()) {
                    let id = info.Id().map(|i| i.to_string()).unwrap_or_default();
                    if is_new(&id) {
                        let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
                        announce(&name, battery_of(info));
                    }
                } else if let Some(info) = info.as_ref() {
                    // Part of the first batch: remember it so its quick reconnect is a flap.
                    let _ = is_new(&info.Id().map(|i| i.to_string()).unwrap_or_default());
                }
                Ok(())
            },
        ));
        w.Start().ok()?;
        Some(w)
    }

    pub fn run(_ui: Sender<UiEvent>) {
        let classic = BluetoothDevice::GetDeviceSelectorFromConnectionStatus(
            BluetoothConnectionStatus::Connected,
        )
        .ok()
        .and_then(watch);
        let le = BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(
            BluetoothConnectionStatus::Connected,
        )
        .ok()
        .and_then(watch);
        if classic.is_none() && le.is_none() {
            crate::logging::line("bluetooth: no watcher could start");
            return;
        }
        // The callbacks do the work on WinRT's threads; this thread only keeps the watchers
        // alive. No wake-ups.
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_never_empty() {
        assert_eq!(clean_name("  WH-1000XM5 "), "WH-1000XM5");
        assert_eq!(clean_name(""), "Bluetooth device");
        assert_eq!(clean_name("\u{0}\n "), "Bluetooth device");
        assert_eq!(clean_name(&"x".repeat(100)).chars().count(), 40);
    }

    #[test]
    fn a_battery_is_a_percentage_or_nothing() {
        assert_eq!(battery_percent(82), Some(82));
        assert_eq!(battery_percent(0), Some(0));
        assert_eq!(battery_percent(100), Some(100));
        assert_eq!(battery_percent(-1), None);
        assert_eq!(battery_percent(255), None);
    }

    #[test]
    fn a_flapping_connection_is_one_card() {
        let t0 = Instant::now();
        let mut s = Seen::default();
        assert!(s.is_new("buds", t0));
        // It drops and comes back a few seconds later: the same connection.
        assert!(!s.is_new("buds", t0 + Duration::from_secs(5)));
        assert!(!s.is_new("buds", t0 + Duration::from_secs(14)));
        // Still flapping 14 s after the last report: quiet. Another device is its own.
        assert!(s.is_new("mouse", t0 + Duration::from_secs(14)));
        // After a quiet spell it is news again.
        assert!(s.is_new("buds", t0 + Duration::from_secs(14) + FLAP));
    }

    /// Live check (`cargo test -- --ignored bluetooth`): the watcher starts, finishes its first
    /// batch and lists the devices connected right now with their battery.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn watcher_lists_the_connected_devices() {
        use windows::core::{IInspectable, HSTRING};
        use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice};
        use windows::Devices::Enumeration::{DeviceInformation, DeviceWatcher};
        use windows::Foundation::TypedEventHandler;
        let sel = BluetoothDevice::GetDeviceSelectorFromConnectionStatus(
            BluetoothConnectionStatus::Connected,
        )
        .unwrap();
        let props =
            windows_collections::IIterable::<HSTRING>::from(vec![HSTRING::from(BATTERY_KEY)]);
        let w =
            DeviceInformation::CreateWatcherAqsFilterAndAdditionalProperties(&sel, &props).unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let t1 = tx.clone();
        w.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(
            move |_, i| {
                if let Some(i) = i.as_ref() {
                    let name = i.Name().map(|n| n.to_string()).unwrap_or_default();
                    let _ = t1.send(format!("device: {name}"));
                }
                Ok(())
            },
        ))
        .unwrap();
        w.EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(
            move |_, _| {
                let _ = tx.send("done".into());
                Ok(())
            },
        ))
        .unwrap();
        w.Start().unwrap();
        let mut seen = vec![];
        while let Ok(m) = rx.recv_timeout(Duration::from_secs(10)) {
            println!("{m}");
            if m == "done" {
                break;
            }
            seen.push(m);
        }
        let _ = w.Stop();
        println!("{} connected device(s)", seen.len());
    }

    #[test]
    fn the_card_names_the_device_and_the_battery_when_known() {
        assert_eq!(
            card_text("Pixel Buds", Some(82)),
            ("Pixel Buds connected".into(), Some("Battery 82%".into()))
        );
        assert_eq!(
            card_text("", None),
            ("Bluetooth device connected".into(), None)
        );
    }
}
