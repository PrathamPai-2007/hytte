//! Downloads watcher: browsers write a partial file (`.crdownload`, `.part`, ...) and rename
//! it to the real name when done, so an in-progress download is a task and a rename is its
//! finish. Windows tells us about every change in the folder (`ReadDirectoryChangesW`), so
//! nothing is polled.

use std::collections::HashSet;

/// Suffixes browsers use for a file that is still downloading (lower-case, with the dot).
const PARTIAL: [&str; 5] = [".crdownload", ".part", ".partial", ".opdownload", ".download"];

/// `Some(name without the suffix)` when `name` is a partial download.
pub fn partial_base(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    PARTIAL
        .iter()
        .find(|s| lower.ends_with(*s))
        .map(|s| &name[..name.len() - s.len()])
        .filter(|b| !b.is_empty())
}

/// What a folder change means for the task list.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// A partial download exists or grew (`id` is its name without the suffix).
    Progress { id: String, file: String },
    /// A partial download was renamed to its final name.
    Finished { id: String, name: String },
    /// A partial download vanished without finishing (cancelled, or renamed to another partial).
    Gone { id: String },
}

pub const ADDED: u32 = 1;
pub const REMOVED: u32 = 2;
pub const MODIFIED: u32 = 3;
pub const RENAMED_OLD: u32 = 4;
pub const RENAMED_NEW: u32 = 5;

/// Turns raw (action, file name) notifications into download events.
#[derive(Default)]
pub struct Machine {
    active: HashSet<String>,
    renamed_from: Option<String>,
}

impl Machine {
    pub fn feed(&mut self, action: u32, name: &str) -> Vec<Out> {
        match action {
            ADDED | MODIFIED => match partial_base(name) {
                Some(id) => {
                    self.active.insert(id.to_string());
                    vec![Out::Progress {
                        id: id.to_string(),
                        file: name.to_string(),
                    }]
                }
                None => vec![],
            },
            REMOVED => match partial_base(name) {
                Some(id) if self.active.remove(id) => vec![Out::Gone { id: id.to_string() }],
                _ => vec![],
            },
            RENAMED_OLD => {
                self.renamed_from = Some(name.to_string());
                vec![]
            }
            RENAMED_NEW => {
                let Some(old) = self.renamed_from.take() else {
                    return self.feed(ADDED, name);
                };
                let mut out = vec![];
                let old_id = partial_base(&old).map(str::to_string);
                match (old_id, partial_base(name)) {
                    // Still downloading under a new name (Chrome's "Unconfirmed 123.crdownload").
                    (Some(o), Some(n)) => {
                        if o != n && self.active.remove(&o) {
                            out.push(Out::Gone { id: o });
                        }
                        self.active.insert(n.to_string());
                        out.push(Out::Progress {
                            id: n.to_string(),
                            file: name.to_string(),
                        });
                    }
                    // The finish.
                    (Some(o), None) => {
                        self.active.remove(&o);
                        out.push(Out::Finished {
                            id: o,
                            name: name.to_string(),
                        });
                    }
                    // A finished file renamed into a partial one: a download that starts by rename.
                    (None, Some(n)) => {
                        self.active.insert(n.to_string());
                        out.push(Out::Progress {
                            id: n.to_string(),
                            file: name.to_string(),
                        });
                    }
                    (None, None) => {}
                }
                out
            }
            _ => vec![],
        }
    }
}

/// "12.3 MB" style size.
pub fn human(bytes: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(windows)]
pub fn spawn_watcher(
    cfg: crate::config::Downloads,
    ui_tx: crossbeam_channel::Sender<crate::ui_state::UiEvent>,
) {
    if !cfg.enabled {
        return;
    }
    std::thread::spawn(move || {
        crate::proc::eco_thread();
        win::watch(cfg, ui_tx);
    });
}

#[cfg(not(windows))]
pub fn spawn_watcher(
    _cfg: crate::config::Downloads,
    _ui_tx: crossbeam_channel::Sender<crate::ui_state::UiEvent>,
) {
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::tasks::{TaskState, TaskUpdate};
    use crate::ui_state::UiEvent;
    use crossbeam_channel::Sender;
    use hytte_proto::TaskEvent;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

    /// The size is only re-sent this often per download: browsers write constantly.
    const SIZE_EVERY: Duration = Duration::from_millis(900);
    /// A download that finishes within this never shows up as a running task.
    const SHOW_AFTER_MS: u32 = 1500;

    fn downloads_dir(cfg: &crate::config::Downloads) -> Option<PathBuf> {
        if let Some(f) = cfg.folder.as_deref().filter(|f| !f.is_empty()) {
            return Some(PathBuf::from(f));
        }
        // SAFETY: the returned string is copied out and freed with CoTaskMemFree.
        unsafe {
            let p = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const _));
            s.map(PathBuf::from)
        }
    }

    pub fn watch(cfg: crate::config::Downloads, ui_tx: Sender<UiEvent>) {
        let Some(dir) = downloads_dir(&cfg) else { return };
        let wide: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        // SAFETY: a plain directory handle used only by this thread, closed on exit.
        let h = unsafe {
            CreateFileW(
                PCWSTR(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        };
        let Ok(h) = h else { return };
        let mut machine = Machine::default();
        let mut started: HashMap<String, Instant> = HashMap::new();
        let mut sent: HashMap<String, Instant> = HashMap::new();
        // u64 backing keeps the records aligned for the u32 fields.
        let mut buf = vec![0u64; 8192];
        loop {
            let mut got = 0u32;
            // SAFETY: the buffer outlives the call and its byte length is passed.
            let ok = unsafe {
                ReadDirectoryChangesW(
                    h,
                    buf.as_mut_ptr() as *mut _,
                    (buf.len() * 8) as u32,
                    false,
                    FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE,
                    Some(&mut got),
                    None,
                    None,
                )
            };
            if ok.is_err() {
                break;
            }
            if got == 0 {
                continue; // buffer overflow: nothing usable
            }
            let bytes: &[u8] =
                unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, got as usize) };
            for (action, name) in records(bytes) {
                for out in machine.feed(action, &name) {
                    let now = Instant::now();
                    match out {
                        Out::Progress { id, file } => {
                            let first = !started.contains_key(&id);
                            started.entry(id.clone()).or_insert(now);
                            // Throttle size updates, but always send the first.
                            if !first && sent.get(&id).is_some_and(|t| now - *t < SIZE_EVERY) {
                                continue;
                            }
                            sent.insert(id.clone(), now);
                            let size = std::fs::metadata(dir.join(&file)).map_or(0, |m| m.len());
                            let label = format!("{} · {}", id, human(size));
                            let _ = ui_tx.send(UiEvent::Task(TaskUpdate::Upsert(state(
                                &id,
                                label,
                                TaskEvent::Start,
                                None,
                            ))));
                        }
                        Out::Finished { id, name } => {
                            let ms = started
                                .remove(&id)
                                .map_or(0, |t| t.elapsed().as_millis() as u64);
                            sent.remove(&id);
                            let path = dir.join(&name);
                            let size = std::fs::metadata(&path).map_or(0, |m| m.len());
                            let _ = ui_tx.send(UiEvent::Task(TaskUpdate::Upsert(state(
                                &id,
                                format!("{name} · {}", human(size)),
                                TaskEvent::Done,
                                Some(ms),
                            ))));
                            let _ = ui_tx.send(UiEvent::DownloadDone { name, path });
                        }
                        Out::Gone { id } => {
                            started.remove(&id);
                            sent.remove(&id);
                            let _ = ui_tx.send(UiEvent::TaskGone(id));
                        }
                    }
                }
            }
        }
        // SAFETY: `h` is ours.
        unsafe {
            let _ = CloseHandle(h);
        }
    }

    fn state(id: &str, label: String, event: TaskEvent, duration_ms: Option<u64>) -> TaskState {
        TaskState {
            task_id: id.to_string(),
            label,
            progress: None,
            event,
            duration_ms,
            stderr_tail: None,
            source: "download".into(),
            pid: None,
            delay_ms: SHOW_AFTER_MS,
            message: None,
            exit_code: None,
            line: None,
            updated: Instant::now(),
        }
    }

    /// Walk a buffer of FILE_NOTIFY_INFORMATION records: (action, file name).
    fn records(buf: &[u8]) -> Vec<(u32, String)> {
        let mut out = vec![];
        let mut at = 0usize;
        loop {
            if at + 12 > buf.len() {
                break;
            }
            let rd = |o: usize| u32::from_le_bytes(buf[at + o..at + o + 4].try_into().unwrap());
            let (next, action, len) = (rd(0) as usize, rd(4), rd(8) as usize);
            let name_at = at + 12;
            if name_at + len > buf.len() {
                break;
            }
            let units: Vec<u16> = buf[name_at..name_at + len]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            out.push((action, String::from_utf16_lossy(&units)));
            if next == 0 {
                break;
            }
            at += next;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[Out]) -> Vec<String> {
        v.iter()
            .map(|o| match o {
                Out::Progress { id, .. } => format!("progress {id}"),
                Out::Finished { id, name } => format!("finished {id} -> {name}"),
                Out::Gone { id } => format!("gone {id}"),
            })
            .collect()
    }

    #[test]
    fn partial_names() {
        assert_eq!(partial_base("setup.exe.crdownload"), Some("setup.exe"));
        assert_eq!(partial_base("Movie.MP4.PART"), Some("Movie.MP4"));
        assert_eq!(partial_base("notes.txt"), None);
        assert_eq!(partial_base(".crdownload"), None);
    }

    #[test]
    fn chrome_download_from_start_to_finish() {
        let mut m = Machine::default();
        let mut log = vec![];
        log.extend(ids(&m.feed(ADDED, "Unconfirmed 123.crdownload")));
        log.extend(ids(&m.feed(MODIFIED, "Unconfirmed 123.crdownload")));
        // Chrome learns the real name, then finishes.
        log.extend(ids(&m.feed(RENAMED_OLD, "Unconfirmed 123.crdownload")));
        log.extend(ids(&m.feed(RENAMED_NEW, "setup.exe.crdownload")));
        log.extend(ids(&m.feed(MODIFIED, "setup.exe.crdownload")));
        log.extend(ids(&m.feed(RENAMED_OLD, "setup.exe.crdownload")));
        log.extend(ids(&m.feed(RENAMED_NEW, "setup.exe")));
        assert_eq!(
            log,
            vec![
                "progress Unconfirmed 123",
                "progress Unconfirmed 123",
                "gone Unconfirmed 123",
                "progress setup.exe",
                "progress setup.exe",
                "finished setup.exe -> setup.exe",
            ]
        );
    }

    #[test]
    fn firefox_part_file_and_a_cancel() {
        let mut m = Machine::default();
        assert_eq!(ids(&m.feed(ADDED, "movie.mp4.part")), ["progress movie.mp4"]);
        assert_eq!(ids(&m.feed(REMOVED, "movie.mp4.part")), ["gone movie.mp4"]);
        // Removing something that was never tracked, or an ordinary file, says nothing.
        assert!(m.feed(REMOVED, "movie.mp4.part").is_empty());
        assert!(m.feed(ADDED, "notes.txt").is_empty());
        assert!(m.feed(MODIFIED, "notes.txt").is_empty());
    }

    #[test]
    fn sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(5 * 1024 * 1024), "5.0 MB");
    }
}
