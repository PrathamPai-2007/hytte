//! Tiny process-tree helpers shared by the daemon and the `hytte` CLI.

#[cfg(windows)]
pub fn parent_pid(pid: u32) -> Option<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = None;
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                if e.th32ProcessID == pid {
                    out = Some(e.th32ParentProcessID);
                    break;
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        out
    }
}

#[cfg(not(windows))]
pub fn parent_pid(_pid: u32) -> Option<u32> {
    None
}

/// The SID (as `S-1-5-21-...`) of the user this process runs as.
#[cfg(windows)]
pub fn user_sid() -> Option<String> {
    use windows::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: the pseudo handle for this process needs no closing.
    token_sid(unsafe { GetCurrentProcess() })
}

#[cfg(not(windows))]
pub fn user_sid() -> Option<String> {
    None
}

/// The user SID string of the process `process` (a handle with query rights).
#[cfg(windows)]
fn token_sid(process: windows::Win32::Foundation::HANDLE) -> Option<String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows::Win32::System::Threading::OpenProcessToken;
    // SAFETY: plain token queries; every handle and buffer is released before returning.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        // u64 backing keeps the TOKEN_USER aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let got = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            len,
            &mut len,
        );
        let _ = CloseHandle(token);
        got.ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let out = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0 as _)));
        out
    }
}

/// Whether the process serving the named pipe `pipe` runs as the current user. A client
/// checks this before it sends anything private, so a look-alike pipe made by someone else
/// can't read commands or answer a permission request.
#[cfg(windows)]
pub fn pipe_server_is_current_user(pipe: windows::Win32::Foundation::HANDLE) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let Some(mine) = user_sid() else {
        return false;
    };
    // SAFETY: the process handle is closed right after the token is read.
    unsafe {
        let mut pid = 0u32;
        if GetNamedPipeServerProcessId(pipe, &mut pid).is_err() {
            return false;
        }
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let theirs = token_sid(p);
        let _ = CloseHandle(p);
        theirs.as_deref() == Some(mine.as_str())
    }
}

/// Each of `pids` with its ancestors (parent first, up to `depth`), from one process snapshot.
#[cfg(windows)]
pub fn ancestors(pids: &[u32], depth: usize) -> std::collections::HashMap<u32, Vec<u32>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    let mut parent = std::collections::HashMap::new();
    unsafe {
        if let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut e = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snap, &mut e).is_ok() {
                loop {
                    parent.insert(e.th32ProcessID, e.th32ParentProcessID);
                    if Process32NextW(snap, &mut e).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snap);
        }
    }
    chains(pids, depth, |p| parent.get(&p).copied())
}

#[cfg(not(windows))]
pub fn ancestors(pids: &[u32], depth: usize) -> std::collections::HashMap<u32, Vec<u32>> {
    chains(pids, depth, |_| None)
}

/// Walk up from each pid; stops at 0, a loop, or `depth`.
fn chains(
    pids: &[u32],
    depth: usize,
    parent: impl Fn(u32) -> Option<u32>,
) -> std::collections::HashMap<u32, Vec<u32>> {
    pids.iter()
        .map(|&pid| {
            let mut v = vec![];
            let mut cur = pid;
            while v.len() < depth {
                match parent(cur) {
                    Some(p) if p != 0 && p != cur && !v.contains(&p) && p != pid => {
                        v.push(p);
                        cur = p;
                    }
                    _ => break,
                }
            }
            (pid, v)
        })
        .collect()
}

/// Shell wrappers that exit right after running a hook; an agent's real
/// process is the first ancestor that is *not* one of these.
pub const SHELL_EXES: &[&str] = &[
    "cmd.exe",
    "sh.exe",
    "bash.exe",
    "zsh.exe",
    "fish.exe",
    "pwsh.exe",
    "powershell.exe",
    "nu.exe",
    "conhost.exe",
];

/// The long-lived process behind a hook invocation: climb past throw-away shells.
#[cfg(windows)]
pub fn owner_pid(start: u32) -> u32 {
    let mut cur = start;
    for _ in 0..6 {
        let Some(parent) = parent_pid(cur).filter(|p| *p != 0 && *p != cur) else {
            break;
        };
        let exe = crate::ports::exe_name(parent)
            .unwrap_or_default()
            .to_ascii_lowercase();
        cur = parent;
        if !SHELL_EXES.contains(&exe.as_str()) {
            break;
        }
    }
    cur
}

#[cfg(not(windows))]
pub fn owner_pid(start: u32) -> u32 {
    start
}

#[cfg(test)]
mod chain_tests {
    #[test]
    fn walks_up_and_stops_at_loops() {
        let parent = |p: u32| match p {
            30 => Some(20),
            20 => Some(10),
            10 => Some(0),
            7 => Some(8),
            8 => Some(7),
            _ => None,
        };
        let c = super::chains(&[30, 7, 99], 8, parent);
        assert_eq!(c[&30], vec![20, 10]);
        assert_eq!(c[&7], vec![8]);
        assert!(c[&99].is_empty());
        assert_eq!(super::chains(&[30], 1, parent)[&30], vec![20]);
    }
}
