//! File logger + panic hook. Off at idle: no spam.

use std::path::PathBuf;

pub fn log_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Hytte").join("hytte.log")
}

pub fn init() {
    let path = log_path();
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::panic::set_hook(Box::new(move |info| {
        let msg = format!("panic: {info}\n");
        eprintln!("{msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())
        {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
    }));
}

/// Local "HH:MM:SS", so log lines can be lined up with what the user was doing.
fn stamp() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::SYSTEMTIME;
        use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
        let unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let ft = unix_ms * 10_000 + 116_444_736_000_000_000; // 100 ns ticks since 1601
        let ft = windows::Win32::Foundation::FILETIME {
            dwLowDateTime: ft as u32,
            dwHighDateTime: (ft >> 32) as u32,
        };
        let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
        unsafe {
            if FileTimeToSystemTime(&ft, &mut utc).is_ok()
                && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
            {
                return format!(
                    "{:02}:{:02}:{:02}",
                    local.wHour, local.wMinute, local.wSecond
                );
            }
        }
    }
    String::from("--:--:--")
}

/// Append one line to the log (best effort).
pub fn line(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        use std::io::Write;
        let _ = writeln!(f, "{} {msg}", stamp());
    }
}

/// Logs why the pill backs off, once per change of `state`, so a stuck "thin line" or a
/// missed game can be diagnosed.
pub fn note_fullscreen(state: u8, detail: &str) {
    use std::sync::atomic::{AtomicU8, Ordering};
    static LAST: AtomicU8 = AtomicU8::new(0);
    if LAST.swap(state, Ordering::Relaxed) != state {
        line(&format!("fullscreen: {detail}"));
    }
}
