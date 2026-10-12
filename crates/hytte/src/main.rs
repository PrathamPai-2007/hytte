//! hytte.exe — one program, two modes. With no arguments it is the daemon (the pill and tray
//! icon); with a command (`hytte run -- ...`, `hytte agent ...`) it is the short-lived CLI.
//!
//! It is a console-subsystem exe so the CLI behaves like any command-line tool (the shell waits,
//! output streams, exit codes propagate). The manifest's `consoleAllocationPolicy=detached` keeps
//! Explorer and autostart from giving the daemon a console window on Windows 11 24H2+; see
//! `detach_console` for older builds and for `hytte` typed in a terminal.
//! Daemon diagnostics go to %APPDATA%\Hytte\hytte.log (see logging.rs).

// Direct2D drawing helpers take geometry + colour; Win32 structs and test models are built field by field.
#![allow(clippy::too_many_arguments, clippy::field_reassign_with_default)]

mod animation;
mod calendar;
mod cli;
mod config;
mod convert;
mod downloads;
mod drop;
mod fullscreen;
#[cfg(windows)]
mod gpu;
mod hog;
mod logging;
mod media;
mod mic;
mod perf;
mod pipe_server;
mod ports;
mod power;
mod privacy;
#[cfg(windows)]
mod proc;
#[cfg(windows)]
mod render;
mod setup;
mod shelf;
mod single_instance;
#[cfg(windows)]
mod tabs;
mod tasks;
mod timer;
mod transforms;
mod tray;
mod ui_state;
mod update;
mod window;
#[cfg(windows)]
mod winrt;

use crossbeam_channel::unbounded;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        cli::run(&args);
        return;
    }
    #[cfg(windows)]
    detach_console();
    logging::init();
    // Retried briefly: after an update the old process is still quitting when the new one starts.
    let mut guard = single_instance::acquire("HytteSingleInstance");
    for _ in 0..50 {
        if guard.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        guard = single_instance::acquire("HytteSingleInstance");
    }
    let Ok(_guard) = guard else {
        logging::line("another instance is running; exiting");
        std::process::exit(0);
    };
    update::cleanup();

    let cfg = config::load();
    config::ensure_autostart(cfg.general.autostart);
    config::ensure_start_menu();
    #[cfg(windows)]
    {
        let add_to_path = cfg.general.add_to_path;
        std::thread::spawn(move || {
            if add_to_path {
                if let Err(e) = setup::ensure_on_path() {
                    logging::line(&format!("setup: PATH not updated: {e}"));
                }
            }
            // 1.x shell hooks called `notch`; point them at `hytte` so tracking keeps working.
            for l in setup::migrate_profiles() {
                logging::line(&format!("setup: migrated {l}"));
            }
        });
    }

    let (msg_tx, msg_rx) = unbounded::<hytte_proto::HytteMessage>();
    let (task_tx, task_rx) = unbounded::<tasks::TaskUpdate>();

    let (ui_tx, ui_rx) = unbounded::<ui_state::UiEvent>();
    let pipe_handle = pipe_server::spawn_listener(msg_tx, ui_tx.clone());
    let tasks_handle = tasks::spawn_registry(msg_rx, task_tx);

    media::spawn_watcher(ui_tx.clone());
    privacy::spawn_watcher(ui_tx.clone());
    mic::spawn_watcher(ui_tx.clone());
    power::spawn_watcher(ui_tx.clone());
    ports::spawn_watcher(cfg.ports.clone(), cfg.hog.clone(), ui_tx.clone());
    downloads::spawn_watcher(cfg.downloads.clone(), ui_tx.clone());
    calendar::spawn_watcher(cfg.calendar.clone(), ui_tx.clone());
    config::spawn_watcher(ui_tx.clone());
    update::spawn_checker(cfg.general.check_updates);

    let (drop_tx, drop_rx) = unbounded::<drop::DropJob>();
    drop::spawn_workers(drop_rx, ui_tx.clone());

    window::run(cfg, task_rx, ui_rx, drop_tx, pipe_handle, tasks_handle);
}

/// The daemon must not hold a console. Three ways to arrive here:
/// - no console at all (24H2+ from Explorer/autostart, or spawned detached): nothing to do;
/// - a console of our own (double-click before 24H2): close it, a brief flash at most;
/// - a terminal's console (`hytte` typed at a prompt): the shell is waiting on us, so start a
///   detached copy and exit, handing the prompt back.
#[cfg(windows)]
fn detach_console() {
    use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut ids = [0u32; 2];
    match unsafe { GetConsoleProcessList(&mut ids) } {
        0 => {}
        1 => unsafe {
            let _ = FreeConsole();
        },
        _ => {
            if let Ok(exe) = std::env::current_exe() {
                if spawn_detached(&exe).is_ok() {
                    std::process::exit(0);
                }
            }
            // Couldn't respawn: keep running attached rather than not at all.
            unsafe {
                let _ = FreeConsole();
            }
        }
    }
}

/// Starts `exe` with no arguments (the daemon) with no console and no inherited stdio, so a
/// pipe the caller was writing to (`hytte run -- x | tee`) is not held open by the daemon.
#[cfg(windows)]
pub fn spawn_detached(exe: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(exe)
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}
