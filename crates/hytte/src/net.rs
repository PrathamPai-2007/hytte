//! Fetching over HTTPS with Windows' own `curl.exe` (System32, by absolute path), so no HTTP
//! stack is linked into Hytte. Used by the updater, the weather and the lyrics. Always run on a
//! worker thread: it blocks until the reply (or the timeout).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Percent-encode a query value (UTF-8 bytes; only unreserved characters stay as they are).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// GET `url`. `out` = save the body to this file, else return it. `secs` and `max_bytes` bound
/// the transfer. HTTPS only (including after a redirect) in release builds; debug builds also
/// take `file://` so an update can be tried against a local fake release.
pub fn fetch(url: &str, out: Option<&Path>, secs: u32, max_bytes: u64) -> Result<Vec<u8>, String> {
    let exe = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\curl.exe");
    let mut cmd = Command::new(exe);
    cmd.args([
        "-fsSL",
        "--max-time",
        &secs.to_string(),
        "--max-filesize",
        &max_bytes.to_string(),
    ]);
    if !cfg!(debug_assertions) {
        cmd.args(["--proto", "=https", "--proto-redir", "=https"]);
    }
    cmd.arg("-H")
        .arg(format!("User-Agent: hytte/{}", env!("CARGO_PKG_VERSION")))
        .arg(url);
    if let Some(p) = out {
        cmd.arg("-o").arg(p);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let o = cmd
        .output()
        .map_err(|e| format!("couldn't run curl.exe: {e}"))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(format!("download failed: {}", err.trim()));
    }
    Ok(o.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_are_percent_encoded_as_utf8() {
        assert_eq!(encode("Bergen"), "Bergen");
        assert_eq!(encode("Tromsø"), "Troms%C3%B8");
        assert_eq!(encode("São Paulo"), "S%C3%A3o%20Paulo");
        assert_eq!(encode("a&b=c?d#e"), "a%26b%3Dc%3Fd%23e");
        assert_eq!(encode("東京"), "%E6%9D%B1%E4%BA%AC");
        assert_eq!(encode(""), "");
    }
}
