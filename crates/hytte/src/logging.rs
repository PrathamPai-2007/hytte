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

/// Append one line to the log (best effort).
pub fn line(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log_path()) {
        use std::io::Write;
        let _ = writeln!(f, "{msg}");
    }
}
