//! ACL'd named-pipe server: `\\.\pipe\hytte`.
//! DACL restricted to current user SID, REJECT_REMOTE_CLIENTS, message-mode,
//! length-bounded reads. Blocking listener thread — no tokio.

use crossbeam_channel::Sender;
use hytte_proto::{HytteMessage, MAX_MESSAGE_BYTES, PIPE_NAME};

pub fn spawn_listener(tx: Sender<HytteMessage>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        #[cfg(windows)]
        run_windows_loop(tx);
        #[cfg(not(windows))]
        {
            let _ = tx;
            loop {
                std::thread::park();
            }
        }
    })
}

#[cfg(windows)]
fn run_windows_loop(tx: Sender<HytteMessage>) {
    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND,
    };
    use windows::Win32::System::Pipes::*;
    use windows::core::{HSTRING, PCWSTR};

    let sddl: Vec<u16> = "D:(A;;GA;;;OW)\0".encode_utf16().collect();

    let mut first = true;
    loop {
        let mut sd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: false.into(),
        };
        let mut use_sa = false;
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .is_ok()
            {
                sa.lpSecurityDescriptor = sd.0;
                use_sa = true;
            }
        }

        let h = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(PIPE_NAME),
                if first { PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_INBOUND },
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                8,
                4096,
                MAX_MESSAGE_BYTES as u32,
                0,
                use_sa.then_some(&sa),
            )
        };
        unsafe {
            if !h.is_invalid() {
                first = false;
            }
            if h.is_invalid() {
                if !sd.0.is_null() {
                    let _ = windows::Win32::Foundation::LocalFree(Some(
                        windows::Win32::Foundation::HLOCAL(sd.0 as _),
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            let connected = ConnectNamedPipe(h, None).is_ok()
                || windows::Win32::Foundation::GetLastError().0 == 535; // ERROR_PIPE_CONNECTED
            if connected {
                // One thread per client so a slow/long-lived client can't
                // starve the others; the next pipe instance is created at once.
                let tx = tx.clone();
                let addr = h.0 as usize;
                std::thread::spawn(move || {
                    let h = windows::Win32::Foundation::HANDLE(addr as *mut _);
                    serve_connection(h, &tx);
                    let _ = DisconnectNamedPipe(h);
                    let _ = windows::Win32::Foundation::CloseHandle(h);
                });
            } else {
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
            if !sd.0.is_null() {
                let _ = windows::Win32::Foundation::LocalFree(Some(
                    windows::Win32::Foundation::HLOCAL(sd.0 as _),
                ));
            }
        }
    }
}

#[cfg(windows)]
fn serve_connection(h: windows::Win32::Foundation::HANDLE, tx: &Sender<HytteMessage>) {
    use windows::Win32::Storage::FileSystem::ReadFile;
    let mut buf = vec![0u8; 8192];
    let mut acc: Vec<u8> = Vec::new();
    loop {
        let mut read: u32 = 0;
        let ok = unsafe { ReadFile(h, Some(&mut buf), Some(&mut read as *mut u32), None) };
        if ok.is_err() || read == 0 {
            break;
        }
        acc.extend_from_slice(&buf[..read as usize]);
        if acc.len() > MAX_MESSAGE_BYTES {
            break;
        }
        while let Some(pos) = acc.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = acc.drain(..=pos).collect();
            let trimmed = String::from_utf8_lossy(&line);
            let trimmed = trimmed.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(msg) = serde_json::from_str::<HytteMessage>(trimmed) {
                if msg.validate() {
                    let _ = tx.send(msg);
                }
            }
        }
    }
}
