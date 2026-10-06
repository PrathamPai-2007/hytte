//! Drop Vault job queue + worker pool.
//! The COM `IDropTarget` lives in `window.rs` (UI-thread STA) and feeds
//! `DropJob`s here; short-lived workers run transforms and report back.

use crate::ui_state::UiEvent;
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DropJob {
    pub paths: Vec<PathBuf>,
    pub text: Option<String>,
    /// Explicit shelf action for `paths`; `None` = per-type default.
    pub op: Option<crate::transforms::Conv>,
}

#[derive(Debug, Clone)]
pub struct DropResult {
    pub summary: String,
    pub open: Option<PathBuf>,
    pub copy: Option<String>,
}

pub fn spawn_workers(rx: Receiver<DropJob>, ui_tx: Sender<UiEvent>) {
    // Tiny pool of 2 workers; jobs are short-lived.
    for _ in 0..2 {
        let rx = rx.clone();
        let ui_tx = ui_tx.clone();
        std::thread::spawn(move || {
            #[cfg(windows)]
            crate::proc::eco_thread();
            while let Ok(job) = rx.recv() {
                let res = handle_job(&job);
                if let Some(c) = &res.copy {
                    let _ = ui_tx.send(UiEvent::SetClipboard(c.clone()));
                }
                let _ = ui_tx.send(UiEvent::DropDone(res));
            }
        });
    }
}

fn handle_job(job: &DropJob) -> DropResult {
    let mut summaries = vec![];
    let mut open = None;
    let mut copy = None;
    if let Some(t) = &job.text {
        let o = handle_text(t);
        summaries.push(o.summary);
        copy = o.copy.or(copy);
    }
    for p in &job.paths {
        let o = crate::transforms::run(job.op.unwrap_or(crate::transforms::Conv::Auto), p);
        summaries.push(o.summary);
        open = o.open.or(open);
        copy = o.copy.or(copy);
    }
    DropResult {
        summary: summaries.join(" | "),
        open,
        copy,
    }
}

fn handle_text(t: &str) -> crate::transforms::Outcome {
    let trimmed = t.trim();
    if trimmed.is_empty() {
        return crate::transforms::Outcome {
            summary: "Empty text".into(),
            ..Default::default()
        };
    }
    if let Ok(out) = crate::transforms::toggle_json(trimmed) {
        let kind = if trimmed.contains('\n') {
            "minified"
        } else {
            "prettified"
        };
        return crate::transforms::Outcome {
            summary: format!("JSON {kind} · copied"),
            open: None,
            copy: Some(out),
        };
    }
    crate::transforms::Outcome {
        summary: format!("Text copied · {} chars", trimmed.chars().count()),
        open: None,
        copy: Some(trimmed.to_string()),
    }
}
