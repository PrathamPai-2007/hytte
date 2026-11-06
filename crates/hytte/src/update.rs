//! Self-update from GitHub Releases.
//!
//! A background check (60 s after start, then daily; `[general] check_updates`) and the tray's
//! "Check for updates…" ask GitHub for the latest release. If it is newer, a message box asks;
//! on Yes the release's `hytte.exe` asset is downloaded next to this exe, checked (`--version`
//! must name the release), swapped in by renaming (a running exe can be renamed, not
//! overwritten), started, and this process quits. The leftover `hytte.exe.old` is removed on the
//! next start. Declining remembers that version (`update-skip`) so the daily check stays quiet.
//!
//! Downloads use Windows' own curl.exe (System32, by absolute path), so no HTTP stack is linked,
//! over HTTPS only. The release also carries `hytte.exe.minisig`, a minisign signature that is
//! checked against the public key built into this exe before the download is ever run.
//! Debug builds (testing) read `HYTTE_UPDATE_API` / `HYTTE_UPDATE_PUBKEY` and accept `file://`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const API: &str = "https://api.github.com/repos/PrathamPai-2007/hytte/releases/latest";
const ASSET: &str = "hytte.exe";
const SIG_ASSET: &str = "hytte.exe.minisig";
/// The release signing key (its secret half is only in the release workflow's secrets). Losing
/// that secret means a new key here and one manual update for everyone.
const PUBKEY: &str = "RWRhYIiFU0f4WrWEqRoBmoQycFtz0quv7vxbJLiej9Q6XuKDS/hCx/ZP";
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// One check at a time: a manual check while the daily one's prompt is open does nothing.
static BUSY: AtomicBool = AtomicBool::new(false);

/// "v2.0.1" / "2.0.1" / "2.0.1-rc1" → (2, 0, 1). Anything else is not a version.
fn parse(tag: &str) -> Option<(u32, u32, u32)> {
    let core = tag.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u32>().ok());
    let v = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(v)
}

#[derive(Debug, PartialEq)]
struct Release {
    tag: String,
    url: String,
    sig_url: String,
    /// The first line of the release notes ("what changed"), empty when there are none.
    note: String,
}

/// The first real line of a release body: not a heading or the "Full Changelog" footer, with
/// its list bullet and GitHub's " by @user in <link>" tail removed, and short enough for a box.
fn first_note(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("**"))
        .unwrap_or("");
    let line = line.trim_start_matches(['*', '-', ' ']);
    let line = line.split(" by @").next().unwrap_or(line).trim();
    if line.chars().count() > 140 {
        format!("{}…", line.chars().take(139).collect::<String>().trim_end())
    } else {
        line.to_string()
    }
}

/// The release's tag and its `hytte.exe` download, from the GitHub API's JSON.
fn release_from_json(body: &[u8]) -> Result<Release, String> {
    let v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("unexpected reply from GitHub: {e}"))?;
    let tag = v["tag_name"].as_str().ok_or("the release has no tag")?;
    let asset = |name: &str| {
        v["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["name"] == name)
            .and_then(|a| a["browser_download_url"].as_str())
    };
    let url = asset(ASSET).ok_or_else(|| format!("release {tag} has no {ASSET} to download"))?;
    let sig_url = asset(SIG_ASSET)
        .ok_or_else(|| format!("release {tag} has no {SIG_ASSET}, so it can't be verified"))?;
    Ok(Release {
        tag: tag.into(),
        url: url.into(),
        sig_url: sig_url.into(),
        note: first_note(v["body"].as_str().unwrap_or("")),
    })
}

/// Runs System32's curl.exe without a window. `out` = save to this file, else return the body.
fn curl(url: &str, out: Option<&Path>) -> Result<Vec<u8>, String> {
    let exe = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join(r"System32\curl.exe");
    let mut cmd = Command::new(exe);
    cmd.args(["-fsSL", "--max-time", "120", "--max-filesize", "67108864"]);
    if !cfg!(debug_assertions) {
        // HTTPS only, including after a redirect.
        cmd.args(["--proto", "=https", "--proto-redir", "=https"]);
    }
    cmd.arg("-H")
        .arg(format!("User-Agent: hytte/{CURRENT}"))
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

fn latest() -> Result<Release, String> {
    // Only a debug build lets the environment point updates somewhere else.
    let api = if cfg!(debug_assertions) {
        std::env::var("HYTTE_UPDATE_API").unwrap_or_else(|_| API.into())
    } else {
        API.into()
    };
    release_from_json(&curl(&api, None)?)
}

/// What `exe --version` prints, e.g. "hytte 2.0.1". A download that hangs is killed after 10 s.
fn version_of(exe: &Path) -> Option<String> {
    use std::process::Stdio;
    let mut cmd = Command::new(exe);
    cmd.arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().ok()?;
    let t0 = std::time::Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if t0.elapsed() > Duration::from_secs(10) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string())
}

/// Checks `data` against a minisign signature (the `.minisig` file's text) and public key.
fn verify(data: &[u8], sig: &str, key: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key).map_err(|e| format!("bad key: {e}"))?;
    let sig = minisign_verify::Signature::decode(sig).map_err(|e| format!("bad signature: {e}"))?;
    key.verify(data, &sig, false)
        .map_err(|e| format!("signature check failed: {e}"))
}

/// The public key to trust: the built-in one (debug builds may override it, for tests).
fn trusted_key() -> String {
    if cfg!(debug_assertions) {
        if let Ok(k) = std::env::var("HYTTE_UPDATE_PUBKEY") {
            return k;
        }
    }
    PUBKEY.into()
}

/// Downloads and swaps in the new exe; returns the path to start.
fn install(r: &Release) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no install folder")?;
    let new = dir.join("hytte.exe.new");
    let old = dir.join("hytte.exe.old");
    let _ = std::fs::remove_file(&new);
    if let Err(e) = std::fs::write(&new, b"") {
        return Err(format!(
            "can't write to {} ({e}). Download the new version from GitHub and replace hytte.exe by hand.",
            dir.display()
        ));
    }
    if let Err(e) = curl(&r.url, Some(&new)) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    // Signed by us, or it never runs: this is what makes a hijacked download harmless.
    let sig = match curl(&r.sig_url, None) {
        Ok(b) => String::from_utf8(b).unwrap_or_default(),
        Err(e) => {
            let _ = std::fs::remove_file(&new);
            return Err(e);
        }
    };
    let bytes = std::fs::read(&new).map_err(|e| e.to_string())?;
    if let Err(e) = verify(&bytes, &sig, &trusted_key()) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("{e}; nothing was changed"));
    }
    // The file must be a working hytte of exactly the advertised version.
    let want = format!("hytte {}", r.tag.trim_start_matches('v'));
    let got = version_of(&new).unwrap_or_else(|| "a file that doesn't run".into());
    if got != want {
        let _ = std::fs::remove_file(&new);
        return Err(format!(
            "the download doesn't look right (expected \"{want}\", got \"{got}\"); nothing was changed"
        ));
    }
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("couldn't move the current exe aside: {e}"))?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("couldn't put the new exe in place: {e}"));
    }
    Ok(exe)
}

fn skip_file() -> PathBuf {
    crate::config::config_path().with_file_name("update-skip")
}

/// Removes the exe an update left behind (it was still running when it was renamed).
pub fn cleanup() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        let _ = std::fs::remove_file(dir.join("hytte.exe.old"));
    }
}

fn message(text: &str, ask: bool) -> bool {
    crate::logging::line(&format!("update: {}", text.replace('\n', " ")));
    #[cfg(windows)]
    return crate::setup::message(text, ask);
    #[cfg(not(windows))]
    {
        let _ = ask;
        false
    }
}

/// `manual`: from the tray, so also report "up to date" and errors, and ignore a skipped version.
fn check(manual: bool) {
    if BUSY.swap(true, Ordering::AcqRel) {
        return;
    }
    run_check(manual);
    BUSY.store(false, Ordering::Release);
}

fn run_check(manual: bool) {
    let r = match latest() {
        Ok(r) => r,
        Err(e) => {
            if manual {
                message(&format!("Couldn't check for updates: {e}"), false);
            } else {
                crate::logging::line(&format!("update: check failed: {e}"));
            }
            return;
        }
    };
    let (Some(new), Some(cur)) = (parse(&r.tag), parse(CURRENT)) else {
        return;
    };
    if new <= cur {
        if manual {
            message(&format!("You're up to date (Hytte {CURRENT})."), false);
        }
        return;
    }
    let skip = skip_file();
    if !manual && std::fs::read_to_string(&skip).is_ok_and(|s| s.trim() == r.tag) {
        return;
    }
    let shown = r.tag.trim_start_matches('v');
    let what = if r.note.is_empty() {
        String::new()
    } else {
        format!("New in {shown}: {}\n\n", r.note)
    };
    let q = format!(
        "Hytte {shown} is available (you have {CURRENT}).\n\n{what}Update now? Hytte will restart in the same folder."
    );
    if !message(&q, true) {
        let _ = std::fs::write(&skip, &r.tag);
        return;
    }
    match install(&r) {
        Ok(exe) => {
            crate::logging::line(&format!("update: installed {}, restarting", r.tag));
            #[cfg(windows)]
            if let Err(e) = crate::spawn_detached(&exe) {
                message(
                    &format!("Updated to {shown}, but couldn't restart ({e}). Start Hytte again."),
                    false,
                );
            }
            crate::window::quit();
        }
        Err(e) => {
            message(&format!("The update didn't install: {e}"), false);
        }
    }
}

/// The daily background check.
/// `[general] check_updates`, live: the daily check skips while it's off.
pub static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn spawn_checker(enabled: bool) {
    ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        loop {
            if ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
                check(false);
            }
            std::thread::sleep(Duration::from_secs(24 * 60 * 60));
        }
    });
}

/// The tray item.
pub fn check_now() {
    std::thread::spawn(|| check(true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse("v2.0.0"), Some((2, 0, 0)));
        assert_eq!(parse("2.0.10-rc1"), Some((2, 0, 10)));
        assert!(parse("v2.0.10") > parse("v2.0.9"));
        assert!(parse("v10.0.0") > parse("v9.9.9"));
        for bad in ["", "v", "latest", "1.2", "1.2.3.4", "v1.x.0"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert!(parse(CURRENT).is_some());
    }

    #[test]
    fn the_first_note_is_the_first_real_line() {
        let body = "## What's Changed\n* Soften the timer chime by @someone in https://x/pull/9\n* Another\n\n**Full Changelog**: https://x";
        assert_eq!(first_note(body), "Soften the timer chime");
        assert_eq!(first_note(""), "");
        assert_eq!(first_note("## Only a heading\n\n**Full Changelog**: x"), "");
        let long = "a".repeat(300);
        assert_eq!(first_note(&long).chars().count(), 140);
    }

    #[test]
    fn release_json_finds_the_exe_asset() {
        let body = br#"{"tag_name":"v2.1.0","assets":[
            {"name":"hytte-2.1.0-x64.zip","browser_download_url":"https://x/zip"},
            {"name":"hytte.exe","browser_download_url":"https://x/exe"},
            {"name":"hytte.exe.minisig","browser_download_url":"https://x/sig"}]}"#;
        assert_eq!(
            release_from_json(body),
            Ok(Release {
                tag: "v2.1.0".into(),
                url: "https://x/exe".into(),
                sig_url: "https://x/sig".into(),
                note: String::new(),
            })
        );
        // Pre-1.0 releases only had the zip.
        let old = br#"{"tag_name":"v1.3.0","assets":[{"name":"hytte-1.3.0-x64.zip","browser_download_url":"u"}]}"#;
        assert!(release_from_json(old).unwrap_err().contains("no hytte.exe"));
        // A release without a signature can't be installed.
        let unsigned =
            br#"{"tag_name":"v2.1.0","assets":[{"name":"hytte.exe","browser_download_url":"u"}]}"#;
        assert!(release_from_json(unsigned)
            .unwrap_err()
            .contains("can't be verified"));
        assert!(release_from_json(b"<html>").is_err());
    }

    /// A signature of `data` by a fresh key: (public key, .minisig text).
    fn sign(data: &[u8]) -> (String, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = minisign::sign(
            Some(&kp.pk),
            &kp.sk,
            std::io::Cursor::new(data),
            Some("file:hytte.exe"),
            None,
        )
        .unwrap();
        (kp.pk.to_base64(), sig.into_string())
    }

    #[test]
    fn only_a_good_signature_by_our_key_passes() {
        let exe = b"MZ pretend this is hytte.exe".to_vec();
        let (key, sig) = sign(&exe);
        assert_eq!(verify(&exe, &sig, &key), Ok(()));
        // Tampered download.
        let mut bad = exe.clone();
        bad[3] ^= 1;
        assert!(verify(&bad, &sig, &key).is_err());
        // Signed by somebody else's key.
        let (_, other_sig) = sign(&exe);
        assert!(verify(&exe, &other_sig, &key).is_err());
        // Missing or garbage signature.
        assert!(verify(&exe, "", &key).is_err());
        assert!(verify(&exe, "not a signature", &key).is_err());
        // The built-in key is a real one.
        assert!(minisign_verify::PublicKey::from_base64(PUBKEY).is_ok());
    }
}
