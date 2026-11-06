//! Self-update from GitHub Releases.
//!
//! A background check (60 s after start, then daily; `[general] check_updates`) and the tray's
//! "Check for updates…" ask GitHub for the latest release. If it is newer, a message box asks;
//! on Yes the release's `hytte.exe` asset is downloaded next to this exe, checked (`--version`
//! must name the release), swapped in by renaming (a running exe can be renamed, not
//! overwritten), started, and this process quits. The leftover `hytte.exe.old` is removed on the
//! next start. Declining remembers that version (`update-skip`) so the daily check stays quiet.
//!
//! Downloads use Windows' own curl.exe (System32, by absolute path), so no HTTP stack is linked.
//! `HYTTE_UPDATE_API` replaces the release URL for testing; curl also reads `file://` URLs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const API: &str = "https://api.github.com/repos/PrathamPai-2007/hytte/releases/latest";
const ASSET: &str = "hytte.exe";
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
    let url = v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"] == ASSET)
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| format!("release {tag} has no {ASSET} to download"))?;
    Ok(Release {
        tag: tag.into(),
        url: url.into(),
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
    cmd.args(["-fsSL", "--max-time", "120", "-H"])
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
    let api = std::env::var("HYTTE_UPDATE_API").unwrap_or_else(|_| API.into());
    release_from_json(&curl(&api, None)?)
}

/// What `exe --version` prints, e.g. "hytte 2.0.1".
fn version_of(exe: &Path) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let o = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
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
            {"name":"hytte.exe","browser_download_url":"https://x/exe"}]}"#;
        assert_eq!(
            release_from_json(body),
            Ok(Release {
                tag: "v2.1.0".into(),
                url: "https://x/exe".into(),
                note: String::new(),
            })
        );
        // Pre-1.0 releases only had the zip.
        let old = br#"{"tag_name":"v1.3.0","assets":[{"name":"hytte-1.3.0-x64.zip","browser_download_url":"u"}]}"#;
        assert!(release_from_json(old).unwrap_err().contains("no hytte.exe"));
        assert!(release_from_json(b"<html>").is_err());
    }
}
