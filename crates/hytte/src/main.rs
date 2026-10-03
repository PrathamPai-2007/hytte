//! Hytte daemon — dynamic notch + ambient cockpit.

// GUI subsystem: double-clicking hytte.exe must not open a console window.
// Diagnostics go to %APPDATA%\Hytte\hytte.log (see logging.rs).
#![cfg_attr(windows, windows_subsystem = "windows")]

mod animation;
mod config;
mod drop;
mod fullscreen;
mod logging;
mod media;
mod mic;
mod pipe_server;
mod power;
mod ports;
#[cfg(windows)]
mod proc;
mod privacy;
mod shelf;
mod single_instance;
#[cfg(windows)]
mod tabs;
mod tasks;
mod transforms;
mod tray;
mod ui_state;
mod window;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod winrt;

use crossbeam_channel::unbounded;

fn main() {
    logging::init();
    let _guard = match single_instance::acquire("HytteSingleInstance") {
        Ok(g) => g,
        Err(_) => {
            eprintln!("hytte: another instance is running");
            std::process::exit(0);
        }
    };

    let cfg = config::load();
    config::ensure_autostart(cfg.general.autostart);

    let (msg_tx, msg_rx) = unbounded::<hytte_proto::HytteMessage>();
    let (task_tx, task_rx) = unbounded::<tasks::TaskUpdate>();

    let pipe_handle = pipe_server::spawn_listener(msg_tx);
    let tasks_handle = tasks::spawn_registry(msg_rx, task_tx);

    let (ui_tx, ui_rx) = unbounded::<ui_state::UiEvent>();
    media::spawn_watcher(ui_tx.clone());
    privacy::spawn_watcher(ui_tx.clone());
    mic::spawn_watcher(ui_tx.clone());
    power::spawn_watcher(ui_tx.clone());
    ports::spawn_watcher(cfg.ports.clone(), ui_tx.clone());

    let (drop_tx, drop_rx) = unbounded::<drop::DropJob>();
    drop::spawn_workers(drop_rx, ui_tx.clone());

    window::run(cfg, task_rx, ui_rx, drop_tx, pipe_handle, tasks_handle);
}
