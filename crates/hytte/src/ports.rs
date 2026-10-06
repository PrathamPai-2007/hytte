//! Port watcher worker: polls the listening-socket table and emits a
//! `UiEvent::Ports` only when the watched set changes.
//!
//! ponytail: Windows has no cheap "a socket started listening" event (ETW
//! TCPIP would be the upgrade). One syscall every `poll_secs` plus an instant
//! refresh when the Ports panel opens keeps idle cost negligible.

use crate::config::{Hog, Ports};
use crate::ui_state::UiEvent;
use crossbeam_channel::Sender;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

static WAKE: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();

fn wake() -> &'static (Mutex<bool>, Condvar) {
    WAKE.get_or_init(|| (Mutex::new(false), Condvar::new()))
}

static NEXT: Mutex<Option<Ports>> = Mutex::new(None);
static NEXT_HOG: Mutex<Option<Hog>> = Mutex::new(None);

/// New `[hog]` settings from a config reload.
pub fn reconfigure_hog(cfg: Hog) {
    *NEXT_HOG.lock().unwrap() = Some(cfg);
}

/// New `[ports]` settings from a config reload: used from the next scan, which starts now.
pub fn reconfigure(cfg: Ports) {
    *NEXT.lock().unwrap() = Some(cfg);
    request_refresh();
}

/// Re-scan immediately (panel opened, a process was killed).
pub fn request_refresh() {
    let (m, c) = wake();
    *m.lock().unwrap() = true;
    c.notify_one();
}

pub fn spawn_watcher(mut cfg: Ports, mut hog_cfg: Hog, ui_tx: Sender<UiEvent>) {
    std::thread::spawn(move || {
        #[cfg(windows)]
        crate::proc::eco_thread();
        let mut last: Option<Vec<hytte_proto::ports::PortInfo>> = None;
        // The resource-hog check shares this tick instead of waking the process on its own.
        #[cfg(windows)]
        let mut sampler = crate::hog::Sampler::new();
        let mut tracker = crate::hog::Tracker::default();
        loop {
            if let Some(next) = NEXT.lock().unwrap().take() {
                cfg = next;
            }
            if let Some(next) = NEXT_HOG.lock().unwrap().take() {
                hog_cfg = next;
            }
            #[cfg(windows)]
            if hog_cfg.enabled {
                let now = std::time::Instant::now();
                let samples = sampler.sample(now);
                for a in tracker.update(now, &samples, &hog_cfg) {
                    let _ = ui_tx.send(UiEvent::Hog(a));
                }
            }
            let poll = Duration::from_secs(cfg.poll_secs.max(1));
            let now = hytte_proto::ports::current(&cfg.watch, cfg.show_all);
            if last.as_ref() != Some(&now) {
                last = Some(now.clone());
                let _ = ui_tx.send(UiEvent::Ports(now));
            }
            let (m, c) = wake();
            let mut flag = m.lock().unwrap();
            if !*flag {
                flag = c.wait_timeout(flag, poll).unwrap().0;
            }
            *flag = false;
        }
    });
}
