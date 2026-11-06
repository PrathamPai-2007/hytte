# 3. Architecture

How the pieces fit at runtime. Later sections zoom into each.

## The big picture

```text
 terminals / scripts / agent hooks
        │  hytte <command> (one short-lived CLI process per message)
        ▼
 \\.\pipe\hytte-<SID> ── pipe listener ── per-connection thread ──┐  HytteMessage
                                                            ▼
                                                  task registry thread
                                                            │  TaskUpdate
 workers: media, privacy, mic, power, ports,                │
 downloads, calendar, drop pool,                            │
 thumbnails, shelf staging  ── UiEvent ─────────────┐       │
                                                    ▼       ▼
                                              bridge thread
                                                    │  push into Shared.pending
                                                    │  + PostMessage(WM_UI)
                                                    ▼
                       UI thread: message loop → wndproc → Ui (model + animation)
                                                    │  kick(): wake the frame clock
                                                    ▼
                                ticker thread ── PostMessage(WM_TICK) ──► Ui::frame
                                                    ▼
                         Renderer: Direct2D → swap chain → DirectComposition → screen
```

Two rules hold everywhere:

1. **Only the UI thread touches UI state and the window.** Other threads send messages.
2. **Nothing polls when Windows can notify.** Threads block on channels, events or OS notifications. The deliberate polls are in [section 12](12-performance.md).

## Startup

`main.rs`, in order:

0. With arguments, `cli::run` handles them and exits ([section 5](05-cli.md)). Otherwise `detach_console` ([section 1](01-overview.md#one-program-two-modes)).
1. `logging::init()` installs the panic hook that writes `hytte.log`.
2. `single_instance::acquire("HytteSingleInstance")` takes a named mutex, retrying up to 5 s (so a freshly updated exe waits for the old one to quit). If another daemon holds it, exit 0. `update::cleanup` deletes a leftover `hytte.exe.old`.
3. `config::load()` reads (or creates) `config.toml`; `ensure_autostart` writes or removes the `Run` value; `ensure_start_menu` refreshes the shortcut on its own thread.
4. Create the `HytteMessage` (pipe → registry) and `TaskUpdate` (registry → UI) channels, then the `UiEvent` channel; start `pipe_server::spawn_listener` and `tasks::spawn_registry` (the listener also gets a `UiEvent` sender for [asks](04-ipc-protocol.md#asking-the-human-ask-and-answer)).
5. Hand the `UiEvent` sender to the workers: `media`, `privacy`, `mic`, `power`, `ports`, `downloads`, `calendar` and the `config` watcher. Start `update::spawn_checker`. A background thread runs `setup::ensure_on_path` (unless `add_to_path = false`), `setup::migrate_profiles` (old `notch` hooks) and `agents::repair` (re-points hooks whose exe moved).
6. Start two Drop Vault workers (`drop::spawn_workers`).
7. `window::run` takes over the main thread until Quit.

`window::win::run_windows` then: creates `Shared` (below) in a global `OnceLock`; calls `OleInitialize` (the UI thread is an STA, needed for OLE drag and drop); creates the topmost, tool, no-activate popup window with its renderer (`WS_EX_NOREDIRECTIONBITMAP`, Direct2D + DirectComposition; if the renderer can't start the reason goes to the log and Hytte exits); builds `Ui` (model, animation, config, shelf, restored timer) and draws the first frame; registers the OLE drop target, power notifications, `TaskbarCreated`, the clipboard listener (screenshots), two WinEvent hooks (foreground change and location) and a low-level mouse hook (arms the drop zone during drags); adds the tray icon; starts the **bridge** and **ticker** threads; runs the `GetMessageW` loop.

**On Quit** the UI thread closes the settings window (it has its own renderer), then **drops `Ui` itself** before removing the tray icon. `Ui` owns D3D11 / DXGI / DirectComposition objects; left to the thread-local destructor they would be released during `ExitProcess` under the loader lock, where GPU drivers can deadlock, and the process would never exit or release the single-instance mutex.

## Threads

| Thread | Blocks on |
|---|---|
| **UI** (main) | the Win32 message queue |
| Pipe listener / one pipe client per connection | `ConnectNamedPipe` / `ReadFile` |
| Task registry | messages; a 5 s tick only while tasks exist |
| Bridge | both worker channels (`select!`) |
| Ticker (frame clock) | `park()` while idle; `DwmFlush` or a 33 ms sleep while animating |
| Media watcher | WinRT change events (30 s safety timeout) |
| Privacy watcher | registry change notifications (10 s safety timeout) |
| Mic watcher | a channel fed by endpoint-volume and device callbacks (60 s timeout) |
| Power watcher | a channel fed by `power::poke` (5 min timeout) |
| Config watcher | `FindFirstChangeNotificationW` on the data folder; applies `config.toml` edits live |
| Port watcher | a condvar: woken on demand, otherwise every `poll_secs`; the resource-hog sampler (`hog.rs`) shares its tick |
| Downloads watcher | `ReadDirectoryChangesW` on Downloads (off with `[downloads] enabled = false`) |
| Calendar watcher | a store-changed channel, timing out when the next event is due (only with `[calendar] enabled`) |
| Drop Vault workers (2), terminal tabs | their job / command channels |
| Short-lived | media command, thumbnail loader, shelf staging, Start Menu shortcut, PATH setup, update install, manual update check, terminal setup |
| Update checker | a 60 s sleep, then one check a day (`check_updates`) |

## Talking to the UI thread

`window.rs` keeps one piece of cross-thread state, `Shared`:

| Field | Purpose |
|---|---|
| `pending: Mutex<Vec<UiEvent>>` | Events waiting for the UI thread. |
| `animating`, `fast`, `tick_pending` | Frame clock running / springs moving (pace on vblank, not ~30 fps) / at most one `WM_TICK` queued. |
| `ticker: OnceLock<Thread>` | Handle to `unpark` the frame clock. |
| `drop_tx` | Sender for Drop Vault jobs. |

A few lock-free statics sit beside it because the mouse hook and WinEvent callbacks can't reach `Ui`: `HWND_ADDR`, the drop-zone rectangle (`ZONE_L/R/B`), `LBTN`, `ARMED`, DWM timing (`VBLANK`, `VPERIOD`, `REFRESH_S`) and `TASKBAR_CREATED`.

**Delivery:** a worker sends `UiEvent`; the bridge thread appends it to `Shared.pending` and posts `WM_UI`; `wndproc` calls `Ui::handle_events`, which drains *all* pending events at once (a burst costs one wake-up), applies each to the model, then runs `after_model_change` (re-arm timers, pick the target scene and sizes via `layout`, `kick()` the frame clock). Thumbnail results, shelf staging results and OLE `DragEnter` / `DragLeave` / drop events skip the channel and go straight into `pending`.

**Frame clock:** the ticker is parked (free) while `animating` is false. `Ui::kick()` sets it, unparks the ticker and posts one `WM_TICK`. While animating it posts `WM_TICK`, then waits on `DwmFlush()` (springs moving) or ~33 ms (ambient only). `Ui::frame` steps springs, draws, presents and clears `animating` when nothing moves. Details in [section 8](08-rendering-and-animation.md).

## Ownership and re-entrancy

`Ui` lives in a thread-local `RefCell<Option<Ui>>`, reached only through `with_ui` (`try_borrow_mut`). Some Windows calls **pump messages**: `DoDragDrop` (dragging a tile off the shelf) runs a modal loop that dispatches `WM_TICK` back into `wndproc`. If `Ui` were still borrowed, every frame of the drag would drop. So the drag starts *after* the borrow ends, and `with_ui` skips a call that finds the cell borrowed instead of panicking. When adding COM or UI code on the UI thread, ask whether it can pump messages; if so, call it outside `with_ui`.

## COM apartments

| Thread | Apartment | Why |
|---|---|---|
| UI | STA (`OleInitialize`) | OLE drag and drop, drop target, virtual desktop manager. |
| Mic watcher | MTA | Audio endpoint APIs; callbacks arrive on a system thread. |
| Terminal tabs | MTA | UI Automation into other processes. |
| Start Menu shortcut | STA (short-lived) | `IShellLinkW`. |

## Error handling

Hytte is ambient: a failure in one feature must never take down the pill or block the user.

- OS calls that may fail return `Option` / `Result` and are mostly ignored or given a fallback (no battery means no Power pane).
- Workers retry or fall back to a slow safety poll instead of exiting; a worker panic ends only that thread and is logged.
- The CLI never fails a shell prompt: with the daemon down, hook messages drop silently, `hytte set` / `hytte agent` print one line, and `hytte run` still runs the command.

Next: [4. IPC protocol and task lifecycle](04-ipc-protocol.md)
