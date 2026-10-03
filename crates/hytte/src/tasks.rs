//! Task registry: validates pipe input, tracks concurrent tasks,
//! times out stale tasks when the client dies.

use crossbeam_channel::{tick, Receiver, Sender};
use hytte_proto::{HytteMessage, TaskEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct TaskState {
    pub task_id: String,
    pub label: String,
    pub progress: Option<u8>,
    pub event: TaskEvent,
    pub duration_ms: Option<u64>,
    pub stderr_tail: Option<String>,
    pub source: String,
    pub pid: Option<u32>,
    pub delay_ms: u32,
    pub message: Option<String>,
    pub exit_code: Option<i32>,
    pub updated: Instant,
}

#[derive(Debug, Clone)]
pub enum TaskUpdate {
    Upsert(TaskState),
    Lost(String),
    Prune,
}

const STALE_AFTER: Duration = Duration::from_secs(30);

#[cfg(windows)]
fn owner_alive(pid: u32) -> bool {
    crate::proc::is_alive(pid)
}
#[cfg(not(windows))]
fn owner_alive(_pid: u32) -> bool {
    true
}

pub fn spawn_registry(
    msg_rx: Receiver<HytteMessage>,
    task_tx: Sender<TaskUpdate>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut map: HashMap<String, TaskState> = HashMap::new();
        let ticker = tick(Duration::from_secs(5));
        loop {
            crossbeam_channel::select! {
                recv(msg_rx) -> m => {
                    let Ok(msg) = m else { break };
                    if !msg.validate() { continue; }
                    // Stale-task timeout is implicit: every update refreshes `updated`.
                    let st = TaskState {
                        task_id: msg.task_id.clone(),
                        label: msg.label.clone(),
                        progress: msg.progress,
                        event: msg.event,
                        duration_ms: msg.duration_ms,
                        stderr_tail: msg.stderr_tail.clone(),
                        source: msg.source.clone().unwrap_or_else(|| "run".into()),
                        pid: msg.pid,
                        delay_ms: msg.delay_ms.unwrap_or(0),
                        message: msg.message.clone(),
                        exit_code: msg.exit_code,
                        updated: Instant::now(),
                    };
                    if matches!(msg.event, TaskEvent::Done | TaskEvent::Failed | TaskEvent::Lost) {
                        map.remove(&msg.task_id);
                    } else {
                        map.insert(msg.task_id.clone(), st.clone());
                    }
                    let _ = task_tx.send(TaskUpdate::Upsert(st));
                }
                recv(ticker) -> _ => {
                    // Mark stale running tasks Lost (pipe disconnect = Lost).
                    let now = Instant::now();
                    let stale: Vec<String> = map.iter()
                        .filter(|(_, s)| {
                            matches!(s.event, TaskEvent::Start | TaskEvent::Progress | TaskEvent::Resumed | TaskEvent::NeedsInput)
                                && match s.pid {
                                    // Owner known: lost only when that process is gone.
                                    Some(pid) => !owner_alive(pid),
                                    None => now.duration_since(s.updated) > STALE_AFTER,
                                }
                        })
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in stale {
                        map.remove(&k);
                        let _ = task_tx.send(TaskUpdate::Lost(k));
                    }
                    let _ = task_tx.send(TaskUpdate::Prune);
                }
            }
        }
    })
}
