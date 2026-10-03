//! Named-mutex single instance.

pub struct Guard {
    #[cfg(windows)]
    _handle: windows::Win32::Foundation::HANDLE,
}

/// Returns Err when another instance holds the mutex.
pub fn acquire(name: &str) -> Result<Guard, ()> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::core::HSTRING;
        use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
        unsafe {
            let h: HANDLE = CreateMutexW(None, false, &HSTRING::from(format!("Local\\{name}")))
                .map_err(|_| ())?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(h);
                return Err(());
            }
            Ok(Guard { _handle: h })
        }
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        Ok(Guard {})
    }
}
