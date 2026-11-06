//! Windowing & interaction engine.
//!
//! One layered, NOACTIVATE, topmost window hanging from the top-centre of the
//! monitor chosen by `[general] monitor`. The pill is animated *inside* a fixed transparent canvas
//! (see `render.rs`), so there is no per-frame window resize and transparent
//! pixels stay click-through. UI state lives in a thread-local `Ui`; other
//! threads only push `UiEvent`s and post a wake-up message.

use crate::config::Config;
use crate::drop::DropJob;
use crate::tasks::TaskUpdate;
use crate::ui_state::UiEvent;
use crossbeam_channel::{Receiver, Sender};

pub const WM_TRAY: u32 = 0x8001;

/// Where `[general] remember_panel` keeps the panel the pill was last on.
fn panel_file() -> std::path::PathBuf {
    crate::config::data_dir().join("panel.txt")
}

/// Whether the theme is light, and the accent colour that suits it.
#[cfg(windows)]
pub fn look(theme: &str) -> (bool, [f32; 3]) {
    win::look(theme)
}

/// Ends the daemon the way the tray's Quit does (tray icon removed, message loop left).
/// Safe from any thread; does nothing before the window exists.
pub fn quit() {
    #[cfg(windows)]
    win::quit();
}

pub fn run(
    cfg: Config,
    task_rx: Receiver<TaskUpdate>,
    ui_rx: Receiver<UiEvent>,
    drop_tx: Sender<DropJob>,
    _pipe_handle: std::thread::JoinHandle<()>,
    _tasks_handle: std::thread::JoinHandle<()>,
) {
    #[cfg(not(windows))]
    {
        let _ = (cfg, task_rx, ui_rx, drop_tx);
        eprintln!("hytte: Windows-only daemon");
    }
    #[cfg(windows)]
    win::run_windows(cfg, task_rx, ui_rx, drop_tx);
}

#[cfg(windows)]
mod win;
