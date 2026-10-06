//! Laptop battery + Windows power mode.
//! Percent / AC from GetSystemPowerStatus, charge rate from the battery
//! device's IOCTL_BATTERY_QUERY_STATUS, power mode via the (undocumented but
//! stable) powrprof overlay-scheme exports, resolved at runtime.
//! The worker sleeps until Windows reports a change (`poke`): the window forwards
//! power-setting broadcasts, and the effective-power-mode callback is registered here.

use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Saver,
    Balanced,
    Performance,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub pct: u8,
    pub plugged: bool,
    /// Signed watts: positive charging, negative discharging; None when the driver stays silent.
    pub watts: Option<f32>,
    pub mode: Mode,
}

impl Battery {
    pub fn status(&self) -> String {
        let flow = match self.watts {
            Some(w) if w >= 0.5 => format!("charging {w:.1} W"),
            Some(w) if w <= -0.5 => format!("using {:.1} W", -w),
            _ if self.plugged && self.pct >= 100 => "full".into(),
            _ if self.plugged => "plugged in".into(),
            _ => "on battery".into(),
        };
        format!("{}%  ·  {flow}", self.pct)
    }
}

/// Wakes the worker; set once by `spawn_watcher`.
static WAKE: std::sync::OnceLock<Sender<()>> = std::sync::OnceLock::new();

/// Tell the worker something about power changed (AC/DC, charge %, power mode, resume).
/// Cheap and safe from any thread, including a system callback.
pub fn poke() {
    if let Some(w) = WAKE.get() {
        let _ = w.try_send(());
    }
}

pub fn spawn_watcher(ui_tx: Sender<UiEvent>) {
    let (tx, rx) = crossbeam_channel::bounded::<()>(1);
    let _ = WAKE.set(tx);
    std::thread::spawn(move || {
        #[cfg(windows)]
        crate::proc::eco_thread();
        #[cfg(windows)]
        {
            imp::watch_power_mode();
            let mut last = None;
            loop {
                let b = imp::read();
                if last != Some(b) {
                    last = Some(b);
                    let _ = ui_tx.send(UiEvent::Power(b));
                }
                // Only a safety net against a missed broadcast; changes arrive via `poke`.
                if let Err(crossbeam_channel::RecvTimeoutError::Disconnected) =
                    rx.recv_timeout(std::time::Duration::from_secs(300))
                {
                    return;
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (ui_tx, rx);
        }
    });
}

/// None on machines without a battery.
pub fn read() -> Option<Battery> {
    #[cfg(windows)]
    {
        imp::read()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn set_mode(m: Mode) {
    #[cfg(windows)]
    imp::set_mode(m);
    #[cfg(not(windows))]
    let _ = m;
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::{s, w, GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::Win32::System::Power::*;
    use windows::Win32::System::IO::DeviceIoControl;

    const SAVER: GUID = GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);
    const BALANCED: GUID = GUID::zeroed();
    const PERFORMANCE: GUID = GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238);

    type GetOverlay = unsafe extern "system" fn(*mut GUID) -> u32;
    type SetOverlay = unsafe extern "system" fn(GUID) -> u32;

    /// powrprof's overlay-scheme exports, resolved once (the DLL stays loaded).
    struct Overlay {
        get: Option<GetOverlay>,
        set: Option<SetOverlay>,
    }

    fn overlay() -> &'static Overlay {
        static O: std::sync::OnceLock<Overlay> = std::sync::OnceLock::new();
        O.get_or_init(|| unsafe {
            let Ok(lib) = LoadLibraryW(w!("powrprof.dll")) else {
                return Overlay {
                    get: None,
                    set: None,
                };
            };
            Overlay {
                get: GetProcAddress(lib, s!("PowerGetActualOverlayScheme"))
                    .map(|f| std::mem::transmute::<_, GetOverlay>(f)),
                set: GetProcAddress(lib, s!("PowerSetActiveOverlayScheme"))
                    .map(|f| std::mem::transmute::<_, SetOverlay>(f)),
            }
        })
    }

    fn mode() -> Mode {
        unsafe {
            let Some(f) = overlay().get else {
                return Mode::Balanced;
            };
            let mut g = GUID::zeroed();
            if f(&mut g) != 0 {
                return Mode::Balanced;
            }
            if g == SAVER {
                Mode::Saver
            } else if g == PERFORMANCE {
                Mode::Performance
            } else {
                Mode::Balanced
            }
        }
    }

    /// Ask Windows to call `poke` whenever the effective power mode changes.
    pub fn watch_power_mode() {
        unsafe extern "system" fn on_mode(
            _mode: EFFECTIVE_POWER_MODE,
            _ctx: *const core::ffi::c_void,
        ) {
            poke();
        }
        let mut handle = std::ptr::null_mut();
        // SAFETY: the callback is a plain fn that outlives the process; the handle is kept
        // registered for the whole run, so it is deliberately never unregistered.
        unsafe {
            let _ = PowerRegisterForEffectivePowerModeNotifications(
                EFFECTIVE_POWER_MODE_V2,
                Some(on_mode),
                None,
                &mut handle,
            );
        }
    }

    pub fn set_mode(m: Mode) {
        unsafe {
            let Some(f) = overlay().set else {
                return;
            };
            f(match m {
                Mode::Saver => SAVER,
                Mode::Balanced => BALANCED,
                Mode::Performance => PERFORMANCE,
            });
        }
    }

    pub fn read() -> Option<Battery> {
        let mut st = SYSTEM_POWER_STATUS::default();
        unsafe { GetSystemPowerStatus(&mut st).ok()? };
        // 128 = no battery, 255 = unknown.
        if st.BatteryFlag & 128 != 0 || st.BatteryFlag == 255 || st.BatteryLifePercent > 100 {
            return None;
        }
        let plugged = st.ACLineStatus == 1;
        let watts = rate_mw().map(|mw| mw as f32 / 1000.0);
        Some(Battery {
            pct: st.BatteryLifePercent,
            plugged,
            watts,
            mode: mode(),
        })
    }

    /// Device path of the first battery, found once and reused; a failed query forgets
    /// it so the next read looks the device up again (battery swapped, driver reloaded).
    static BATTERY_PATH: std::sync::Mutex<Option<Vec<u16>>> = std::sync::Mutex::new(None);

    /// Signed charge rate in mW from the first battery device.
    fn rate_mw() -> Option<i32> {
        let mut cached = BATTERY_PATH.lock().unwrap_or_else(|e| e.into_inner());
        if cached.is_none() {
            *cached = battery_path();
        }
        match query_rate(cached.as_ref()?) {
            Some(rate) => rate,
            None => {
                *cached = None;
                None
            }
        }
    }

    /// NUL-terminated device interface path of the first battery.
    fn battery_path() -> Option<Vec<u16>> {
        unsafe {
            let set = SetupDiGetClassDevsW(
                Some(&GUID_DEVICE_BATTERY),
                PCWSTR::null(),
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .ok()?;
            let r = (|| {
                let mut ifd = SP_DEVICE_INTERFACE_DATA {
                    cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                    ..Default::default()
                };
                SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVICE_BATTERY, 0, &mut ifd).ok()?;
                let mut need = 0u32;
                let _ = SetupDiGetDeviceInterfaceDetailW(set, &ifd, None, 0, Some(&mut need), None);
                if need < 8 {
                    return None;
                }
                // u64 backing keeps the detail header aligned.
                let mut buf = vec![0u64; (need as usize).div_ceil(8)];
                let det = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                (*det).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                SetupDiGetDeviceInterfaceDetailW(set, &ifd, Some(det), need, None, None).ok()?;
                let p = (&raw const (*det).DevicePath) as *const u16;
                let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                let mut path = std::slice::from_raw_parts(p, len).to_vec();
                path.push(0);
                Some(path)
            })();
            let _ = SetupDiDestroyDeviceInfoList(set);
            r
        }
    }

    /// IOCTL_BATTERY_QUERY_STATUS on an already-located battery device.
    /// Outer None: the device could not be queried. Inner None: the driver reports no rate.
    fn query_rate(path: &[u16]) -> Option<Option<i32>> {
        unsafe {
            let h = CreateFileW(
                PCWSTR(path.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .ok()?;
            let out = (|| {
                let (mut tag, mut n) = (0u32, 0u32);
                let wait = 0u32;
                DeviceIoControl(
                    h,
                    IOCTL_BATTERY_QUERY_TAG,
                    Some(&wait as *const _ as *const _),
                    4,
                    Some(&mut tag as *mut _ as *mut _),
                    4,
                    Some(&mut n),
                    None,
                )
                .ok()?;
                if tag == 0 {
                    return None;
                }
                let q = BATTERY_WAIT_STATUS {
                    BatteryTag: tag,
                    ..Default::default()
                };
                let mut s = BATTERY_STATUS::default();
                DeviceIoControl(
                    h,
                    IOCTL_BATTERY_QUERY_STATUS,
                    Some(&q as *const _ as *const _),
                    std::mem::size_of::<BATTERY_WAIT_STATUS>() as u32,
                    Some(&mut s as *mut _ as *mut _),
                    std::mem::size_of::<BATTERY_STATUS>() as u32,
                    Some(&mut n),
                    None,
                )
                .ok()?;
                Some((s.Rate as u32 != BATTERY_UNKNOWN_RATE).then_some(s.Rate))
            })();
            let _ = CloseHandle(h);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_is_safe_on_any_machine() {
        // Desktop: None. Laptop: a sane percentage.
        let b = read();
        println!("battery: {b:?}");
        assert!(b.is_none_or(|b| b.pct <= 100));
    }

    #[test]
    fn status_text() {
        let b = |pct, plugged, watts| Battery {
            pct,
            plugged,
            watts,
            mode: Mode::Balanced,
        };
        assert_eq!(b(78, true, Some(24.46)).status(), "78%  ·  charging 24.5 W");
        assert_eq!(b(40, false, Some(-8.2)).status(), "40%  ·  using 8.2 W");
        assert_eq!(b(100, true, Some(0.0)).status(), "100%  ·  full");
        assert_eq!(b(55, false, None).status(), "55%  ·  on battery");
    }
}
