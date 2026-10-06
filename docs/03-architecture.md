# 3. Architecture

This section explains how the pieces fit together at runtime. Later sections zoom into each piece.

## The big picture

```text
 terminals / scripts / agent hooks
        │  notch.exe (one short-lived process per message)
        ▼
 \\.\pipe\hytte ── pipe listener ── per-connection thread ──┐  HytteMessage
                                                            ▼
                                                  task registry thread
                                                            │  TaskUpdate
 workers: media, privacy, mic, power, ports,               │
 drop pool, thumbnails, shelf staging  ── UiEvent ──┐       │
                                                    ▼       ▼
                                              bridge thread
                                                    │  push into Shared.pending
                                                    │  + PostMessage(WM_UI)
                                                    ▼
                       UI thread: message loop → wndproc → Ui (model + animation)
                                                    │  kick(): wake the frame clock
                                                    ▼
                                ticker thread ── PostMessage(WM_TICK) ──► Ui::frame
                                                    │
                                                    ▼
                         Renderer: Direct2D → swap chain → DirectComposition → screen
                                   (classic fallback: Direct2D → DIB → UpdateLayeredWindow)
```

Two rules hold everywhere:

1. **Only the UI thread touches UI state and the window.** Every other thread communicates by sending a message.
2. **Nothing polls when Windows can notify.** Threads block on channels, events or OS notifications. The few deliberate polls are listed in [section 12](12-performance.md).

## Startup sequence

`crates/hytte/src/main.rs`, in order:

1. `logging::init()` installs the panic hook that writes to `hytte.log`.
2. `single_instance::acquire("HytteSingleInstance")` takes a named mutex. If another daemon holds it, this process exits with code 0.
3. `config::load()` reads `config.toml`, creating it with defaults if it is missing. Then `ensure_autostart` writes or removes the `Run` registry value, and `ensure_start_menu` (on its own thread) refreshes the Start Menu shortcut.
4. Two channels for the task pipeline are created: `HytteMessage` (pipe → registry) and `TaskUpdate` (registry → UI).
5. `pipe_server::spawn_listener` and `tasks::spawn_registry` start.
6. A `UiEvent` channel is created and handed to the workers: `media`, `privacy`, `mic`, `power`, `ports` and the `config` watcher. A background thread also runs `setup::ensure_on_path` unless `add_to_path = false`.
7. The Drop Vault job channel is created and `drop::spawn_workers` starts two workers.
8. `window::run` takes over the main thread and never returns until Quit.

`window::win::run_windows` then:

1. Creates the `Shared` struct (see below) and stores it in a global `OnceLock`.
2. Calls `OleInitialize` (the UI thread is a COM single-threaded apartment, which OLE drag and drop requires).
3. Registers the window class, then creates the topmost, tool, no-activate popup window together with its renderer: a `WS_EX_NOREDIRECTIONBITMAP` window and the GPU renderer, or (if that fails or `[general] renderer = "classic"`) a layered window and the classic one.
4. Builds the `Ui` struct: model, animation state, config, shelf loaded from disk, saved timer restored. It then sizes everything for the monitor's DPI and draws the first frame.
5. Registers the OLE drop target, power-setting notifications (`WM_POWERBROADCAST`), the `TaskbarCreated` message (to re-add the tray icon if Explorer restarts), two WinEvent hooks (foreground change and foreground location change) and a low-level mouse hook (to arm the drop zone during drags).
6. Adds the tray icon.
7. Starts the **bridge** thread and the **ticker** (frame clock) thread.
8. Runs the standard `GetMessageW` / `DispatchMessageW` loop.

## Every thread

| Thread | Started in | Lifetime | Blocks on |
|---|---|---|---|
| **UI thread** (the main thread) | `main` | whole process | the Win32 message queue |
| Pipe listener | `pipe_server::spawn_listener` | whole process | `ConnectNamedPipe` |
| Pipe client | the listener, one per connection | one connection | `ReadFile` on the pipe |
| Task registry | `tasks::spawn_registry` | whole process | messages; a 5 s tick only while tasks exist |
| Bridge | `run_windows` | whole process | both worker channels (`select!`) |
| Ticker (frame clock) | `run_windows` | whole process | `park()` while idle, `DwmFlush` or a 33 ms sleep while animating |
| Media watcher | `media::spawn_watcher` | whole process | WinRT change events (30 s safety timeout) |
| Media command | `media::control` | one button press | the WinRT call |
| Privacy watcher | `privacy::spawn_watcher` | whole process | registry change notifications (10 s safety timeout) |
| Mic watcher | `mic::spawn_watcher` | whole process | a channel fed by endpoint-volume and device callbacks (60 s safety timeout) |
| Power watcher | `power::spawn_watcher` | whole process | a channel fed by `power::poke` (power broadcasts and mode callback; 5 min safety timeout) |
| Config watcher | `config::spawn_watcher` | whole process | `FindFirstChangeNotificationW` on the data folder; applies edits to `config.toml` live |
| Port watcher | `ports::spawn_watcher` | whole process | a condvar: woken early on demand, otherwise every `poll_secs` |
| Drop Vault workers (2) | `drop::spawn_workers` | whole process | the job channel |
| Terminal tabs | `tabs::spawn` | whole process | its command channel |
| Thumbnail loader | `Ui::request_thumbs` | one thumbnail | the shell thumbnail call |
| Shelf staging | `Ui::handle_events` (`ShelfAdd`) | one drop | file copies |
| Start Menu shortcut | `config::ensure_start_menu` | a moment at startup | COM shell link calls |
| PATH setup | `setup::ensure_on_path` | a moment at startup | one registry read and, if needed, write (`[general] add_to_path`) |
| Terminal setup | `setup::tray_setup` | one tray click | profile file writes and message boxes |

## How threads talk to the UI thread

### The `Shared` struct

`window.rs` keeps exactly one piece of cross-thread state, `Shared`:

| Field | Purpose |
|---|---|
| `pending: Mutex<Vec<UiEvent>>` | Events waiting for the UI thread. |
| `animating: AtomicBool` | True while the frame clock should run. |
| `fast: AtomicBool` | True while springs are moving: pace frames on vblank rather than at ~30 fps. |
| `tick_pending: AtomicBool` | Prevents piling up more than one `WM_TICK` in the queue. |
| `ticker: OnceLock<Thread>` | Handle used to `unpark` the frame clock. |
| `drop_tx` | Sender for Drop Vault jobs. |

A few lock-free statics sit beside it, because the low-level mouse hook and WinEvent callbacks run without access to `Ui`: `HWND_ADDR`, the drop-zone rectangle (`ZONE_L/R/B`), the mouse-button and drag-armed flags (`LBTN`, `ARMED`), DWM vblank timing (`VBLANK`, `VPERIOD`, `REFRESH_S`) and `TASKBAR_CREATED`.

### The delivery path

1. A worker calls `ui_tx.send(UiEvent::...)`. Task updates come from the registry on their own channel.
2. The **bridge** thread receives from both channels and calls `push_event`, which appends to `Shared.pending` and posts `WM_UI` to the window.
3. The window procedure handles `WM_UI` by calling `Ui::handle_events`, which drains *all* pending events at once (so a burst costs one wake-up), applies each one to the model, then calls `after_model_change`.
4. `after_model_change` re-arms timers, works out the target scene and sizes (`layout`), and calls `kick()` to start the frame clock.

Some events are pushed straight into `Shared.pending` without going through a channel: thumbnail results, shelf staging results, and the OLE drop target's `DragEnter` / `DragLeave` / drop events.

### The frame clock

The ticker thread is the heartbeat:

- While `animating` is false, it is **parked** and costs nothing.
- `Ui::kick()` sets `animating`, unparks it, and posts one `WM_TICK` immediately.
- While animating, it posts `WM_TICK` (at most one outstanding, thanks to `tick_pending`), then waits: on `DwmFlush()` (the next vblank) when springs are moving, or about 33 ms for ambient-only animation.
- Each `WM_TICK` runs `Ui::frame`, which steps the springs, draws, presents, and then decides whether to keep animating. When nothing moves, it clears `animating` and the ticker parks again.

[Section 8](08-rendering-and-animation.md) covers the frame in detail.

## Ownership and re-entrancy

`Ui` lives in a thread-local `RefCell<Option<Ui>>`, accessed only through `with_ui`, which uses `try_borrow_mut`. This matters because some Windows calls **pump messages**: `DoDragDrop` (dragging a tile out of the shelf) runs a modal loop that dispatches `WM_TICK` and others back into `wndproc`. If `Ui` were still borrowed at that point, every frame during the drag would be dropped. So the drag is started *after* the borrow ends, and `with_ui` quietly skips a call if it ever finds the cell already borrowed, instead of panicking.

When you add code that calls into COM or shows UI from the UI thread, ask whether it can pump messages, and if so, call it outside `with_ui`.

## COM apartments

| Thread | Apartment | Why |
|---|---|---|
| UI thread | STA, via `OleInitialize` | OLE drag and drop, the drop target, the virtual desktop manager. |
| Mic watcher | MTA | Audio endpoint APIs; the device-change callback arrives on a system thread. |
| Terminal tabs | MTA | UI Automation calls into other processes. |
| Start Menu shortcut | STA, for its short life | `IShellLinkW`. |

## Error handling philosophy

Hytte is ambient. A failure in one feature must never take down the pill or block the user:

- OS calls that may fail return `Option` / `Result` and are mostly ignored with `let _ =` or a fallback value (for example, no battery → the battery row disappears).
- Workers retry or fall back to a slow safety poll instead of exiting.
- The CLI never fails a shell prompt: if the daemon is down, messages are dropped silently (shell hooks) or with a one-line note (`notch set`, `notch agent`), and `notch run` still runs the command.
- Panics in a worker thread end only that thread; the panic message goes to `hytte.log`.

Next: [4. IPC protocol and task lifecycle](04-ipc-protocol.md)
