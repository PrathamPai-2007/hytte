//! Tiny process-tree helpers shared by the daemon and the `notch` client.

#[cfg(windows)]
pub fn parent_pid(pid: u32) -> Option<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
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

/// Shell wrappers that exit right after running a hook; an agent's real
/// process is the first ancestor that is *not* one of these.
pub const SHELL_EXES: &[&str] = &[
    "cmd.exe", "sh.exe", "bash.exe", "zsh.exe", "fish.exe", "pwsh.exe", "powershell.exe", "nu.exe", "conhost.exe",
];

/// The long-lived process behind a hook invocation: climb past throw-away shells.
#[cfg(windows)]
pub fn owner_pid(start: u32) -> u32 {
    let mut cur = start;
    for _ in 0..6 {
        let Some(parent) = parent_pid(cur).filter(|p| *p != 0 && *p != cur) else { break };
        let exe = crate::ports::exe_name(parent).unwrap_or_default().to_ascii_lowercase();
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
