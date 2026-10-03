//! Task registry: validates pipe input, tracks concurrent tasks,
//! times out stale tasks when the client dies.

use crossbeam_channel::{Receiver, Sender, after};
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
    pub updated: Instant,
}

#[derive(Debug, Clone)]
pub enum TaskUpdate {
    Upsert(TaskState),
    Lost(String),
    Prune,
}

const STALE_AFTER: Duration = Duration::from_secs(30);
const SUCCESS_HOLD: Duration = Duration::from_secs(3);

pub fn spawn_registry(
    msg_rx: Receiver<HytteMessage>,
    task_tx: Sender<TaskUpdate>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut map: HashMap<String, TaskState> = HashMap::new();
        let ticker = after(Duration::from_secs(5));
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
                        updated: Instant::now(),
                    };
                    map.insert(msg.task_id.clone(), st.clone());
                    let _ = task_tx.send(TaskUpdate::Upsert(st));
                }
                recv(ticker) -> _ => {
                    // Mark stale running tasks Lost (pipe disconnect = Lost).
                    let now = Instant::now();
                    let stale: Vec<String> = map.iter()
                        .filter(|(_, s)| matches!(s.event, TaskEvent::Start | TaskEvent::Progress) && now.duration_since(s.updated) > STALE_AFTER)
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in stale {
                        map.remove(&k);
                        let _ = task_tx.send(TaskUpdate::Lost(k));
                    }
                    let _ = task_tx.send(TaskUpdate::Prune);
                    let _ = SUCCESS_HOLD; // hold duration used by UI layer
                }
            }
        }
    })
}
