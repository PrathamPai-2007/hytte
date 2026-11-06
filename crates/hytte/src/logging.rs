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
    // Keep one old log: the file never grows past about 1 MB per run.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let msg = format!("panic in thread '{name}': {info}\n");
        eprintln!("{msg}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())
        {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
        // The UI thread is `main`: without it there is no pill, so start a fresh one. A worker
        // thread's panic only ends that worker (it is in the log by name).
        if name == "main" {
            relaunch();
        }
    }));
}

/// Relaunches allowed in any 10 minutes: a crash on every start must not loop forever.
const RELAUNCHES: usize = 3;
const WINDOW_SECS: u64 = 600;

/// Whether another relaunch fits, given the unix times (seconds) of the recent ones.
fn may_relaunch(recent: &[u64], now: u64) -> bool {
    recent
        .iter()
        .filter(|&&t| now.saturating_sub(t) < WINDOW_SECS)
        .count()
        < RELAUNCHES
}

fn relaunch() {
    let file = log_path().with_file_name("relaunches.txt");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let recent: Vec<u64> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .filter(|&t| now.saturating_sub(t) < WINDOW_SECS)
        .collect();
    if !may_relaunch(&recent, now) {
        line("crashed again and again: not relaunching (start Hytte by hand)");
        return;
    }
    let mut all = recent;
    all.push(now);
    let text: String = all.iter().map(|t| format!("{t}\n")).collect();
    let _ = std::fs::write(&file, text);
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        match crate::spawn_detached(&exe) {
            Ok(()) => line("crashed: started a new Hytte"),
            Err(e) => line(&format!("crashed, and couldn't relaunch: {e}")),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_three_relaunches_in_ten_minutes() {
        assert!(may_relaunch(&[], 1000));
        assert!(may_relaunch(&[900, 950], 1000));
        assert!(!may_relaunch(&[900, 950, 990], 1000));
        // Old ones no longer count.
        assert!(may_relaunch(&[100, 200, 300], 1000));
    }
}
