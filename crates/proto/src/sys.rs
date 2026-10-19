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
