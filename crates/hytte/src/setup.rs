//! Terminal setup: puts `notch` on the user's PATH and adds the shell hook to PowerShell and
//! Git Bash profiles. Shared by `hytte.exe` (startup, tray item) and `notch.exe` (`notch setup`),
//! so it uses std + windows only.
#![cfg_attr(not(windows), allow(dead_code))]

use std::path::{Path, PathBuf};

const BEGIN: &str = "# >>> hytte >>>";
const END: &str = "# <<< hytte <<<";
const PWSH_LINE: &str =
    "if (Get-Command notch -ErrorAction SilentlyContinue) { notch init pwsh | Out-String | Invoke-Expression }";
const BASH_LINE: &str = "command -v notch >/dev/null && eval \"$(notch init bash)\"";

/// `%VAR%` expansion with the current environment; unknown variables stay as written.
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let name = &after[..b];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push_str(&rest[a..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// PATH entries compare case-insensitively, ignoring quotes and a trailing slash.
fn norm(entry: &str) -> String {
    expand(entry.trim().trim_matches('"'))
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// `existing` with `dir` appended, or `None` if it is already there.
pub fn path_with(existing: &str, dir: &str) -> Option<String> {
    let d = norm(dir);
    if existing.split(';').any(|e| norm(e) == d) {
        return None;
    }
    let base = existing.trim_end_matches(';');
    Some(if base.is_empty() {
        dir.to_string()
    } else {
        format!("{base};{dir}")
    })
}

/// `existing` without any entry equal to `dir`, or `None` if there was none.
pub fn path_without(existing: &str, dir: &str) -> Option<String> {
    let d = norm(dir);
    let all: Vec<&str> = existing.split(';').collect();
    let kept: Vec<&str> = all.iter().copied().filter(|e| norm(e) != d).collect();
    (kept.len() != all.len()).then(|| kept.join(";"))
}

fn eol_of(text: &str, default: &'static str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else {
        default
    }
}

/// `text` with the Hytte block added, or `None` if it already has one. The block is appended, or
/// inserted just above the first line containing `before` (bash must load before Starship, which
/// chains an existing DEBUG trap; PowerShell must load after it, since Starship replaces `prompt`).
pub fn add_block(
    text: &str,
    line: &str,
    default_eol: &'static str,
    before: Option<&str>,
) -> Option<String> {
    if text.contains(BEGIN) {
        return None;
    }
    let eol = eol_of(text, default_eol);
    if let Some(at) = before.and_then(|m| text.find(m)) {
        let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        return Some(format!(
            "{}{BEGIN}{eol}{line}{eol}{END}{eol}{}",
            &text[..start],
            &text[start..]
        ));
    }
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push_str(eol);
        }
        out.push_str(eol);
    }
    out.push_str(&format!("{BEGIN}{eol}{line}{eol}{END}{eol}"));
    Some(out)
}

/// `text` without the Hytte block (and the blank line before it), or `None` if it has none.
pub fn remove_block(text: &str) -> Option<String> {
    let start = text.find(BEGIN)?;
    let end = start + text[start..].find(END)? + END.len();
    let mut s = start;
    if text[..s].ends_with("\r\n\r\n") {
        s -= 2;
    } else if text[..s].ends_with("\n\n") {
        s -= 1;
    }
    let tail = &text[end..];
    let tail = tail
        .strip_prefix("\r\n")
        .or_else(|| tail.strip_prefix('\n'))
        .unwrap_or(tail);
    Some(format!("{}{}", &text[..s], tail))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
}

fn decode(bytes: &[u8]) -> (String, Enc) {
    if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        (String::from_utf8_lossy(b).into_owned(), Enc::Utf8Bom)
    } else if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = b
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        (String::from_utf16_lossy(&units), Enc::Utf16Le)
    } else {
        (String::from_utf8_lossy(bytes).into_owned(), Enc::Utf8)
    }
}

fn encode(text: &str, enc: Enc) -> Vec<u8> {
    match enc {
        Enc::Utf8 => text.as_bytes().to_vec(),
        Enc::Utf8Bom => [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat(),
        Enc::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            v.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            v
        }
    }
}

/// Writes via a temp file + rename, except through a symlink (dotfile setups), which is written in place.
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let linked = std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if linked {
        return std::fs::write(path, bytes);
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".hytte-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

struct Profile {
    shell: &'static str,
    path: PathBuf,
    line: &'static str,
    eol: &'static str,
    before: Option<&'static str>,
}

/// Adds (or with `undo`, removes) the hook block in one profile; returns a line for the user.
fn update(p: &Profile, undo: bool) -> String {
    let shown = p.path.display();
    let (text, enc) = match std::fs::read(&p.path) {
        Ok(b) => decode(&b),
        // A new PowerShell profile gets a BOM so 5.1 reads it as UTF-8; bash files never do.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let enc = if p.eol == "\n" {
                Enc::Utf8
            } else {
                Enc::Utf8Bom
            };
            (String::new(), enc)
        }
        Err(e) => return format!("{}: couldn't read {shown}: {e}", p.shell),
    };
    let next = if undo {
        remove_block(&text)
    } else {
        add_block(&text, p.line, p.eol, p.before)
    };
    let Some(next) = next else {
        return if undo {
            format!("{}: not set up", p.shell)
        } else {
            format!("{}: already set up ({shown})", p.shell)
        };
    };
    match write_file(&p.path, &encode(&next, enc)) {
        Ok(()) if undo => format!("{}: removed from {shown}", p.shell),
        Ok(()) => format!("{}: added to {shown}", p.shell),
        Err(e) => format!("{}: couldn't write {shown}: {e}", p.shell),
    }
}

fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Hytte")
}

fn on_process_path(exe: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(exe).is_file()))
        .unwrap_or(false)
}

#[cfg(windows)]
mod sys {
    use windows::core::{w, HSTRING};
    use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegGetValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
        KEY_SET_VALUE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
        RRF_RT_REG_SZ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    fn wide(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    /// The user's own PATH (unexpanded) and its registry type.
    pub fn user_path() -> (String, REG_VALUE_TYPE) {
        unsafe {
            let mut key = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Environment"),
                None,
                KEY_QUERY_VALUE,
                &mut key,
            ) != ERROR_SUCCESS
            {
                return (String::new(), REG_EXPAND_SZ);
            }
            let mut ty = REG_VALUE_TYPE::default();
            let mut len = 0u32;
            let mut out = (String::new(), REG_EXPAND_SZ);
            if RegQueryValueExW(key, w!("Path"), None, Some(&mut ty), None, Some(&mut len))
                == ERROR_SUCCESS
            {
                let mut buf = vec![0u8; len as usize];
                if RegQueryValueExW(
                    key,
                    w!("Path"),
                    None,
                    Some(&mut ty),
                    Some(buf.as_mut_ptr()),
                    Some(&mut len),
                ) == ERROR_SUCCESS
                {
                    buf.truncate(len as usize);
                    out = (wide(&buf), ty);
                }
            }
            let _ = RegCloseKey(key);
            out
        }
    }

    pub fn set_user_path(value: &str, ty: REG_VALUE_TYPE) -> Result<(), String> {
        unsafe {
            let mut key = HKEY::default();
            let r = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                w!("Environment"),
                None,
                KEY_SET_VALUE,
                &mut key,
            );
            if r != ERROR_SUCCESS {
                return Err(format!("can't open HKCU\\Environment ({})", r.0));
            }
            let units: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2);
            let r = RegSetValueExW(key, w!("Path"), None, ty, Some(bytes));
            let _ = RegCloseKey(key);
            if r != ERROR_SUCCESS {
                return Err(format!("can't write PATH ({})", r.0));
            }
        }
        // Tell Explorer (and so newly opened terminals) that the environment changed.
        std::thread::spawn(|| unsafe {
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(w!("Environment").as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                5000,
                None,
            );
        });
        Ok(())
    }

    fn reg_str(root: HKEY, sub: &str, value: &str) -> Option<String> {
        let mut buf = [0u8; 128];
        let mut len = buf.len() as u32;
        let r = unsafe {
            RegGetValueW(
                root,
                &HSTRING::from(sub),
                &HSTRING::from(value),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut _),
                Some(&mut len),
            )
        };
        (r == ERROR_SUCCESS).then(|| wide(&buf[..len as usize]))
    }

    /// Windows PowerShell 5.1's effective policy for running the profile (group policy first).
    pub fn ps51_policy() -> String {
        const SHELL: &str = r"Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell";
        const GPO: &str = r"Software\Policies\Microsoft\Windows\PowerShell";
        reg_str(HKEY_LOCAL_MACHINE, GPO, "ExecutionPolicy")
            .or_else(|| reg_str(HKEY_CURRENT_USER, GPO, "ExecutionPolicy"))
            .or_else(|| reg_str(HKEY_CURRENT_USER, SHELL, "ExecutionPolicy"))
            .or_else(|| reg_str(HKEY_LOCAL_MACHINE, SHELL, "ExecutionPolicy"))
            .unwrap_or_else(|| "Restricted".into())
    }

    /// What `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` does for Windows PowerShell.
    pub fn allow_ps51_profiles() -> Result<(), String> {
        unsafe {
            let mut key = HKEY::default();
            let r = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                w!(r"Software\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell"),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut key,
                None,
            );
            if r != ERROR_SUCCESS {
                return Err(format!("can't open the PowerShell policy key ({})", r.0));
            }
            let units: Vec<u16> = "RemoteSigned".encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(units.as_ptr() as *const u8, units.len() * 2);
            let r = RegSetValueExW(key, w!("ExecutionPolicy"), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(key);
            if r != ERROR_SUCCESS {
                return Err(format!("can't write the policy ({})", r.0));
            }
        }
        Ok(())
    }

    pub fn documents() -> Option<std::path::PathBuf> {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{
            FOLDERID_Documents, SHGetKnownFolderPath, KNOWN_FOLDER_FLAG,
        };
        unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_Documents, KNOWN_FOLDER_FLAG(0), None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s.map(Into::into)
        }
    }

    pub fn message(text: &str, ask: bool) -> bool {
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
        };
        let style =
            MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST | if ask { MB_YESNO } else { MB_OK };
        unsafe { MessageBoxW(None::<HWND>, &HSTRING::from(text), w!("Hytte"), style) == IDYES }
    }
}

/// Puts the folder holding `notch.exe` on the user's PATH, and drops the entry a moved install left
/// behind. Returns whether PATH changed. Skipped under `cargo run` and when `notch` already resolves.
#[cfg(windows)]
pub fn ensure_on_path() -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let Some(dir) = exe.parent().filter(|d| d.join("notch.exe").is_file()) else {
        return Ok(false);
    };
    if std::env::var_os("CARGO").is_some() {
        return Ok(false);
    }
    let dir_s = dir.display().to_string();
    let (cur, ty) = sys::user_path();
    let mut next = cur.clone();
    let record = data_dir().join("path.txt");
    if let Ok(prev) = std::fs::read_to_string(&record) {
        let prev = prev.trim();
        if !prev.is_empty()
            && norm(prev) != norm(&dir_s)
            && !Path::new(prev).join("notch.exe").is_file()
        {
            if let Some(p) = path_without(&next, prev) {
                next = p;
            }
        }
    }
    if !on_process_path("notch.exe") {
        if let Some(p) = path_with(&next, &dir_s) {
            next = p;
        }
    }
    if next == cur {
        return Ok(false);
    }
    if next.len() > 32_000 {
        return Err("PATH is too long to extend".into());
    }
    sys::set_user_path(&next, ty)?;
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(&record, &dir_s);
    Ok(true)
}

#[cfg(windows)]
fn profiles() -> Vec<Profile> {
    let mut v = Vec::new();
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    if let Some(docs) = sys::documents().or_else(|| home.as_ref().map(|h| h.join("Documents"))) {
        v.push(Profile {
            shell: "Windows PowerShell",
            path: docs.join(r"WindowsPowerShell\Microsoft.PowerShell_profile.ps1"),
            line: PWSH_LINE,
            eol: "\r\n",
            before: None,
        });
        let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        let pwsh7 = docs.join("PowerShell").is_dir()
            || on_process_path("pwsh.exe")
            || program_files.is_some_and(|p| p.join(r"PowerShell\7\pwsh.exe").is_file());
        if pwsh7 {
            v.push(Profile {
                shell: "PowerShell 7",
                path: docs.join(r"PowerShell\Microsoft.PowerShell_profile.ps1"),
                line: PWSH_LINE,
                eol: "\r\n",
                before: None,
            });
        }
    }
    if let Some(home) = home {
        let rc = home.join(".bashrc");
        let git = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .is_some_and(|p| p.join(r"Git\bin\bash.exe").is_file());
        if rc.is_file() || git {
            v.push(Profile {
                shell: "Git Bash",
                path: rc,
                line: BASH_LINE,
                eol: "\n",
                before: Some("starship init"),
            });
        }
    }
    v
}

#[cfg(windows)]
fn has_block(p: &Profile) -> bool {
    std::fs::read(&p.path)
        .map(|b| decode(&b).0.contains(BEGIN))
        .unwrap_or(false)
}

/// Whether any shell profile already has the Hytte block.
#[cfg(windows)]
fn any_set_up() -> bool {
    profiles().iter().any(has_block)
}

/// Whether every detected shell profile has the Hytte block. A shell set up by hand earlier
/// (say Git Bash) must not hide that PowerShell still needs it.
#[cfg(windows)]
pub fn is_set_up() -> bool {
    let p = profiles();
    !p.is_empty() && p.iter().all(has_block)
}

/// Adds (or removes) the shell hook in every detected profile; one line per shell for the user.
#[cfg(windows)]
pub fn setup_shells(undo: bool) -> Vec<String> {
    let profiles = profiles();
    let mut out: Vec<String> = profiles.iter().map(|p| update(p, undo)).collect();
    if undo {
        return out;
    }
    let ps51 = profiles.iter().any(|p| p.shell == "Windows PowerShell");
    if ps51 && blocked_by_policy() {
        let policy = sys::ps51_policy();
        out.push(format!(
            "Windows PowerShell won't run profiles under the '{policy}' policy. To allow it, run:\n    Set-ExecutionPolicy -Scope CurrentUser RemoteSigned"
        ));
    }
    out.push(
        "zsh / Nushell: add the line from `notch init zsh` or `notch init nu` yourself.".into(),
    );
    out.push("Open a new terminal window to start using it.".into());
    out
}

/// Whether Windows PowerShell's policy stops it from loading a profile.
#[cfg(windows)]
fn blocked_by_policy() -> bool {
    let policy = sys::ps51_policy();
    policy.eq_ignore_ascii_case("Restricted") || policy.eq_ignore_ascii_case("AllSigned")
}

/// The tray item: set up, or offer to undo if it's already set up. Shows the result in a message box.
#[cfg(windows)]
pub fn tray_setup() {
    std::thread::spawn(|| {
        let mut lines = if is_set_up() {
            if !sys::message(
                "Terminal integration is already set up.\n\nRemove it from your shell profiles?",
                true,
            ) {
                return;
            }
            setup_shells(true)
        } else {
            let mut lines = match ensure_on_path() {
                Ok(true) => vec![
                    "PATH: added the Hytte folder, so `notch` works in new terminals".to_string(),
                ],
                Ok(false) => vec![],
                Err(e) => vec![format!("PATH: {e}")],
            };
            lines.extend(setup_shells(false));
            lines
        };
        if !any_set_up() || !blocked_by_policy() {
            sys::message(&lines.join("\n"), false);
            return;
        }
        // Windows PowerShell won't read the profile we just edited: offer the usual one-line fix.
        lines.retain(|l| !l.starts_with("Windows PowerShell won't run"));
        lines.push(
            "\nWindows PowerShell's execution policy blocks profiles, so the hook wouldn't load.\n\
             Allow it now? (sets the policy to RemoteSigned for your user only)"
                .into(),
        );
        if sys::message(&lines.join("\n"), true) {
            let done = match sys::allow_ps51_profiles() {
                Ok(()) => "Done. Open a new PowerShell window to start using it.".to_string(),
                Err(e) => format!("Couldn't change the policy: {e}\nRun this yourself:\n    Set-ExecutionPolicy -Scope CurrentUser RemoteSigned"),
            };
            sys::message(&done, false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_append_is_idempotent_and_case_insensitive() {
        assert_eq!(path_with("", r"C:\Hytte").as_deref(), Some(r"C:\Hytte"));
        assert_eq!(
            path_with(r"C:\a;C:\b;", r"C:\Hytte").as_deref(),
            Some(r"C:\a;C:\b;C:\Hytte")
        );
        assert_eq!(path_with(r"C:\a;c:\hytte\", r"C:\Hytte"), None);
        assert_eq!(path_with(r#"C:\a;"C:\Hytte""#, r"C:\Hytte"), None);
    }

    #[test]
    fn path_expands_variables_when_comparing() {
        std::env::set_var("HYTTE_TEST_BASE", r"C:\Users\me");
        assert_eq!(
            path_with(r"%HYTTE_TEST_BASE%\bin", r"C:\Users\me\bin"),
            None
        );
        assert_eq!(expand("%NO_SUCH_HYTTE_VAR%\\x"), "%NO_SUCH_HYTTE_VAR%\\x");
        assert_eq!(expand("100%"), "100%");
    }

    #[test]
    fn path_remove_keeps_other_entries() {
        assert_eq!(
            path_without(r"C:\a;C:\old\;C:\b", r"C:\Old").as_deref(),
            Some(r"C:\a;C:\b")
        );
        assert_eq!(path_without(r"C:\a", r"C:\old"), None);
    }

    #[test]
    fn block_round_trips_and_is_idempotent() {
        for (text, eol) in [
            ("", "\n"),
            ("alias ll='ls -l'\n", "\n"),
            ("Set-Alias g git\r\n", "\r\n"),
        ] {
            let added = add_block(text, BASH_LINE, eol, None).unwrap();
            assert!(added.contains(BASH_LINE));
            assert!(
                !added.replace("\r\n", "").contains('\r'),
                "mixed line endings"
            );
            assert_eq!(add_block(&added, BASH_LINE, eol, None), None);
            assert_eq!(remove_block(&added).as_deref(), Some(text));
        }
        assert_eq!(remove_block("nothing here\n"), None);
        // A file without a final newline still gets a clean block.
        assert_eq!(
            add_block("x", "y", "\n", None).as_deref(),
            Some("x\n\n# >>> hytte >>>\ny\n# <<< hytte <<<\n")
        );
    }

    #[test]
    fn block_goes_above_starship_when_asked() {
        let rc = "export A=1\neval \"$(starship init bash)\"\n";
        let added = add_block(rc, "y", "\n", Some("starship init")).unwrap();
        assert_eq!(
            added,
            "export A=1\n# >>> hytte >>>\ny\n# <<< hytte <<<\neval \"$(starship init bash)\"\n"
        );
        assert_eq!(remove_block(&added).as_deref(), Some(rc));
        // On the first line too; without the marker the block is appended.
        let first = add_block("starship init\n", "y", "\n", Some("starship init")).unwrap();
        assert!(first.starts_with(BEGIN));
        let plain = add_block("a\n", "y", "\n", Some("starship init")).unwrap();
        assert!(plain.starts_with("a\n\n"));
    }

    #[test]
    fn encodings_round_trip() {
        for enc in [Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Le] {
            let (text, got) = decode(&encode("héllo\r\n", enc));
            assert_eq!((text.as_str(), got), ("héllo\r\n", enc));
        }
    }
}
