//! Listening-port discovery and kill, shared by the daemon (Ports panel) and
//! the `notch ports` / `notch kill :PORT` commands.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortInfo {
    pub port: u16,
    pub pid: u32,
    pub exe: String,
}

/// Executables that listen on ports but are never a dev server.
const SYSTEM_EXES: &[&str] = &[
    "svchost.exe", "system", "lsass.exe", "services.exe", "wininit.exe", "spoolsv.exe", "msedgewebview2.exe",
    "searchhost.exe", "onedrive.exe", "dashost.exe",
];

/// Pure selection: keep `watch`ed ports (or every non-system listener when
/// `show_all`), drop pid 0/4 and system hosts, de-duplicate v4/v6 pairs.
pub fn filter(rows: &[(u16, u32)], watch: &[u16], show_all: bool, exe_of: impl Fn(u32) -> Option<String>) -> Vec<PortInfo> {
    let mut out: Vec<PortInfo> = vec![];
    for &(port, pid) in rows {
        if pid <= 4 || out.iter().any(|p| p.port == port) {
            continue;
        }
        let exe = exe_of(pid).unwrap_or_else(|| "?".into());
        if SYSTEM_EXES.contains(&exe.to_ascii_lowercase().as_str()) {
            continue;
        }
        if show_all || watch.contains(&port) {
            out.push(PortInfo { port, pid, exe });
        }
    }
    out.sort_by_key(|p| p.port);
    out
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::NetworkManagement::IpHelper::*;
    use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    use windows::Win32::System::Threading::*;

    /// All (port, pid) pairs listening on loopback or any address.
    pub fn listeners() -> Vec<(u16, u32)> {
        let mut out = vec![];
        unsafe {
            for af in [AF_INET.0 as u32, AF_INET6.0 as u32] {
                let mut size = 0u32;
                let _ = GetExtendedTcpTable(None, &mut size, false, af, TCP_TABLE_OWNER_PID_LISTENER, 0);
                if size == 0 {
                    continue;
                }
                let mut buf = vec![0u8; size as usize + 256];
                size = buf.len() as u32;
                if GetExtendedTcpTable(Some(buf.as_mut_ptr() as *mut _), &mut size, false, af, TCP_TABLE_OWNER_PID_LISTENER, 0) != 0 {
                    continue;
                }
                let n = *(buf.as_ptr() as *const u32) as usize;
                let rows = buf.as_ptr().add(4);
                for i in 0..n {
                    if af == AF_INET.0 as u32 {
                        let r = &*(rows as *const MIB_TCPROW_OWNER_PID).add(i);
                        // Address is network-order: 0.0.0.0 or 127.x.x.x only.
                        let a = r.dwLocalAddr.to_le_bytes();
                        if r.dwLocalAddr == 0 || a[0] == 127 {
                            out.push((u16::from_be(r.dwLocalPort as u16), r.dwOwningPid));
                        }
                    } else {
                        let r = &*(rows as *const MIB_TCP6ROW_OWNER_PID).add(i);
                        let a = r.ucLocalAddr;
                        let any = a.iter().all(|b| *b == 0);
                        let lo = a[..15].iter().all(|b| *b == 0) && a[15] == 1;
                        if any || lo {
                            out.push((u16::from_be(r.dwLocalPort as u16), r.dwOwningPid));
                        }
                    }
                }
            }
        }
        out
    }

    pub fn exe_name(pid: u32) -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 520];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(h);
            ok.ok()?;
            String::from_utf16_lossy(&buf[..len as usize]).rsplit('\\').next().map(str::to_owned)
        }
    }

    pub fn current(watch: &[u16], show_all: bool) -> Vec<PortInfo> {
        filter(&listeners(), watch, show_all, exe_name)
    }

    /// Terminate a listener's process. Refuses pid 0/4 and ourselves.
    pub fn kill(pid: u32) -> Result<(), String> {
        if pid <= 4 || pid == std::process::id() {
            return Err("refusing to kill a protected process".into());
        }
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, false, pid).map_err(|e| format!("can't open process: {e}"))?;
            let r = TerminateProcess(h, 1).map_err(|e| format!("terminate failed: {e}"));
            let _ = CloseHandle(h);
            r
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_rules() {
        let exe = |pid: u32| Some(match pid {
            10 => "node.exe".to_string(),
            11 => "svchost.exe".to_string(),
            12 => "python.exe".to_string(),
            _ => "x.exe".into(),
        });
        let rows = [(3000, 10), (135, 11), (9999, 12), (3000, 10), (80, 4), (5432, 13)];
        let got = filter(&rows, &[3000, 5432], false, exe);
        assert_eq!(got.iter().map(|p| p.port).collect::<Vec<_>>(), vec![3000, 5432]);
        let all = filter(&rows, &[3000], true, exe);
        assert_eq!(all.iter().map(|p| p.port).collect::<Vec<_>>(), vec![3000, 5432, 9999]);
    }

    #[cfg(windows)]
    #[test]
    fn sees_our_own_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(listeners().contains(&(port, std::process::id())));
        assert!(kill(std::process::id()).is_err());
    }
}
