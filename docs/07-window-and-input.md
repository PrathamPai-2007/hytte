# 7. Window, input and timers

`crates/hytte/src/window.rs` is the Win32 layer. It owns the window, turns OS messages into model changes, runs the timers, and handles drag and drop, fullscreen suppression and the tray. Everything in it runs on the UI thread except a few tiny callbacks, which only set atomics and post messages.

## The window

Created once in `run_windows`:

| Setting | Effect |
|---|---|
| `WS_POPUP` | No frame or title bar. |
| `WS_EX_NOREDIRECTIONBITMAP` (renderer `gpu`) | The window has no GDI surface; DirectComposition supplies its pixels. Clicks pass through everywhere outside a **window region** that tracks the pill and its glow. |
| `WS_EX_LAYERED` (renderer `classic`) | Per-pixel alpha via `UpdateLayeredWindow`. Fully transparent pixels are **click-through**, so the window only "exists" where the pill is drawn. |
| `WS_EX_TOPMOST` | Above normal windows. Re-asserted whenever the foreground window changes. |
| `WS_EX_TOOLWINDOW` | No taskbar button and no Alt-Tab entry. |
| `WS_EX_NOACTIVATE` | Never becomes the active window. Together with `WM_MOUSEACTIVATE → MA_NOACTIVATE`, clicking the pill never steals keyboard focus. |
| Class `HyttePill`, `CS_DBLCLKS` | The class name is also how fullscreen detection recognises Hytte's own window. |

The pill is drawn into a fixed **canvas** of 460 × 310 logical px, centred on the top edge of the monitor chosen by `[general] monitor` (below). With the `gpu` renderer the window *is* that canvas and a region limits what is visible and clickable; with `classic`, only the part that holds the pill (plus margins for the glow) is drawn and presented each frame. See [section 8](08-rendering-and-animation.md).

The renderer is picked before the window exists, because the style depends on it: `run_windows` creates a `WS_EX_NOREDIRECTIONBITMAP` window and tries `Renderer::new_gpu`; if that fails (or `[general] renderer = "classic"`) it destroys that window and creates a layered one for `Renderer::new`. A static `LIVE` flag keeps the destroyed window's `WM_DESTROY` from ending the message loop before it starts.

### Monitor and DPI

`Ui::refresh_monitor` reads the rectangle and effective DPI of the monitor the pill hangs from, computes `scale = dpi / 96`, resizes the renderer's bitmap (or swap chain), and recomputes the drop-zone rectangle. The top edge is the work area's top when that is lower than the monitor's, so with a taskbar docked at the top the pill hangs below it. It runs at startup, on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`, and when `[general] monitor` changes. The app manifest (`app.manifest`, embedded through `hytte.rc` by `build.rs`) declares per-monitor DPI awareness v2, so Windows doesn't bitmap-stretch the pill.

`[general] monitor` chooses the monitor (`Ui::wanted_monitor`):

| Value | The pill hangs from |
|---|---|
| `"primary"` (default) | The primary monitor. |
| `"active"` | The monitor holding the foreground window. |
| `"cursor"` | The monitor under the pointer. |

For `active` and `cursor`, `Ui::follow_monitor` runs when the foreground window settles (the `T_FS` timer, 150 ms after a change), moves the pill if the monitor or its DPI changed, and then re-runs the fullscreen check against the new monitor. A live edit of the key applies at once. Limit: the drag-arming zone ([below](#arming-the-top-edge)) belongs to the monitor the pill is on, so dragging a file to the top edge of a different monitor doesn't arm it until the pill has moved there.

## The `Ui` struct

Thread-local UI state (accessed through `with_ui`; see [section 3](03-architecture.md#ownership-and-re-entrancy)):

| Field(s) | Meaning |
|---|---|
| `cfg`, `model`, `anim`, `rend` | Config, the model ([section 6](06-ui-model.md)), animation state, and the renderer. |
| `scale`, `mon` | DPI scale and monitor rectangle. |
| `hover` | The pill is "open because of the mouse" (set after the dwell delay). |
| `inside` | The pointer is inside the pill shape right now. |
| `mouse`, `origin`, `crop_w`, `hits` | Pointer position, where the last frame was placed on screen, and its clickable regions. |
| `fs_hidden`, `gaming`, `paused`, `armed`, `shown` | Fullscreen suppression, game mode, the tray Pause state, a drag in progress near the top edge, and whether the window is shown. |
| `drag` | A shelf tile was pressed: it becomes an OLE drag once the pointer moves more than 4 px. |
| `wheel` | Accumulated wheel delta (precision touchpads send fractions of a notch). |
| `pressed`, `light` | The left button is down over a clickable region; the resolved `[general] theme`. |
| `kb`, `kb_focus` | Keyboard mode (the window that had focus before, to give it back) and the region that has the focus ring. |
| `thumb_req`, `tabs`, `vdm` | Thumbnails already requested, the terminal-tabs worker, and the virtual desktop manager. |
| `hmon` | The `HMONITOR` the pill hangs from; fullscreen checks compare against it. |
| `scrub` | The volume bar is being dragged: its left and right edge. |
| `ringing`, `awake_on` | The timer alarm is playing; keep-awake is held. |
| `gpu_retry` | GPU rebuild backoff in ms (0 while the renderer is healthy). |

Three methods do most of the work:

- `layout()`: computes the scene from the model and sets every spring target (size, glow, hover brightness, visibility).
- `kick()`: makes sure the frame clock is running.
- `after_model_change()`: called after **any** model change. It finishes a due timer, (re)arms `T_TIMER` and `T_EXPIRE`, starts or stops the alarm (`timer::ring`, while `timer_done` is set and `[timer] sound` is on), takes or releases keep-awake (`SetThreadExecutionState` with `ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED` while `Model::awake` is active; Windows releases it if Hytte exits), turns a due calendar heads-up into a card, requests missing thumbnails, then runs `layout()` and `kick()`.

## Messages handled by `wndproc`

| Message | What happens |
|---|---|
| `WM_MOUSEACTIVATE` | Returns `MA_NOACTIVATE`. |
| `WM_TICK` (custom) | `Ui::frame`: one animation frame. |
| `WM_UI` (custom) | `Ui::handle_events`: drains all queued `UiEvent`s. |
| `WM_FG` (custom) | The foreground window changed or moved; see [Fullscreen](#fullscreen-suppression). |
| `WM_DRAG_ARM` / `WM_DRAG_END` (custom) | A drag entered the top-edge zone / the button was released. |
| `WM_MOUSEMOVE` | While the volume bar is dragged, sets the volume (`scrub_to`) and nothing else. Otherwise updates the pointer, starts leave-tracking, starts or cancels the hover timers, starts a tile drag past 4 px, and redraws so hover effects follow the pointer. |
| `WM_MOUSELEAVE` | The pointer left the window: clears it and arms the collapse timer. |
| `WM_HOTKEY` | The `[general] hotkey` was pressed: enters or leaves [keyboard mode](#keyboard). |
| `WM_KEYDOWN` | In keyboard mode: Esc, Left/Right, Tab / Shift+Tab, Enter / Space. |
| `WM_KILLFOCUS` | Ends keyboard mode without taking focus back. |
| `WM_MOUSEWHEEL` | Over the volume bar or the **Sound** tile: the volume, 2 % per notch. On the timer setup card: the minutes. Otherwise switches panels, one per notch. |
| `WM_SETCURSOR` | A hand cursor over clickable regions. |
| `WM_LBUTTONDOWN` | On a shelf tile: remembers it and captures the mouse, ready for a drag. On the volume bar: sets the volume where it was pressed and captures the mouse for the drag (`scrub`). |
| `WM_CAPTURECHANGED` | Capture taken away mid-drag (Alt+Tab, a dialog): the volume drag ends. |
| `WM_LBUTTONUP` | Ends a volume drag (and re-reads the real level), otherwise a click: runs the `Action` under the pointer. |
| `WM_RBUTTONUP` | Over the pill: opens or closes the timer setup card (`timer_panel`). |
| `WM_TIMER` | See the timers below. |
| `WM_DISPLAYCHANGE`, `WM_DPICHANGED` | `refresh_monitor`, then re-evaluate fullscreen. |
| `WM_SETTINGCHANGE` | Re-reads the Windows "Animation effects" setting (`reduce_motion`). |
| `WM_TRAY` (custom) | Right-click on the tray icon: shows the menu. |
| `WM_COMMAND` | Tray menu items: 10 Pause, 11 Open settings folder, 12 Quit, 13 Launch at startup, 14 Set up terminal integration (`setup::tray_setup`, which works on its own thread and reports in a message box), 15 Check for updates (`update::check_now`), 16 Connect AI agents (`agents::tray`, likewise). |
| `TaskbarCreated` (registered) | Explorer restarted: re-adds the tray icon and posts `WM_FG` to re-check fullscreen. |
| `WM_POWERBROADCAST` | Power source, charge level or resume changed: wakes the power worker (`power::poke`). On resume (`PBT_APMRESUMEAUTOMATIC`) it also posts `WM_FG`. |
| `WM_DESTROY` | Ends the message loop (unless the window was torn down during startup). |

## Timers

All are one-shot Win32 timers: the handler kills the timer first, so each fires once until re-armed.

| Id | Delay | Armed when | On fire |
|---|---|---|---|
| `T_DWELL` | 120 ms | The pointer enters the pill | If still inside: `hover = true` (the pill opens), a port rescan is requested, and the battery is re-read when the Power panel is the one opening. |
| `T_COLLAPSE` | 300 ms | The pointer leaves while open | If still outside: `hover = false` and the timer setup card closes. Re-entering in time cancels it, so grazing the edge doesn't flicker. |
| `T_EXPIRE` | Until the model's next deadline | After every model change | `after_model_change` (calls `Model::expire`). |
| `T_TIMER` | At most 1 s | While a countdown timer runs unpaused | `after_model_change`: redraws the fuse and fires the timer when it reaches zero. |
| `T_FS` | 150 ms | On a foreground change or move | Follows the monitor (`active` / `cursor`), re-evaluates fullscreen, re-asserts topmost (except over a game), follows the virtual desktop. |
| `T_TRIM` | 30 s | A frame finds nothing moving | If still settled: trims the working set ([section 12](12-performance.md)). |
| `T_GPU` | 1 s, doubling to 30 s | A GPU rebuild failed | Tries `rebuild_gpu` again ([section 8](08-rendering-and-animation.md#the-gpu-renderer)). |

## Clicks and actions

The renderer records a list of **hit regions** while drawing each frame (`Hit { rect, action }`); see [section 8](08-rendering-and-animation.md). `Ui::hit_at` finds the topmost region under the pointer, and `Ui::run_action` executes its `Action`:

| Action | Effect |
|---|---|
| `DismissTask`, `DismissChip` | Removes a finished task / the chip. |
| `CopyText`, `OpenPath` | Clipboard / opens Explorer. |
| `MediaPrev`, `MediaToggle`, `MediaNext`, `FocusMedia` | Media transport commands (on a short-lived thread) / brings the player to the front. |
| `FocusTerminal(pid, id)` | Finds the terminal window for `pid` and focuses it, then asks the tabs worker to select the remembered tab. Used by task rows and by server rows that belong to a task. |
| `SelectPanel` | Switches the panel. Opening Tasks forces a port rescan; opening Power re-reads the battery. |
| `ToggleMic` | Mutes or unmutes every capture device (the **Mic** tile). |
| `ToggleSpeaker`, `Volume` | Mutes or unmutes the default speaker (the **Sound** tile; `Volume` is the bar, whose mouse presses are a drag handled by `scrub_to`, so this action is only reached from the keyboard). |
| `CycleAwake` | Keep-awake: Off → 1 hour → On → Off (`Awake::next`). |
| `OpenTimer` | The **Timer** tile: the Timer panel if a timer runs, otherwise the setup card. |
| `OpenPort`, `KillPort` | A server's **Open** / **Stop**: opens `http://localhost:<port>` / two-step kill (first click arms **Kill?** for 3 s). If the process is already gone, the row is just removed. |
| `ShelfTile`, `RemoveShelf`, `ShelfOp` | Select a tile / remove it / queue a Drop Vault job (shows "Working…"). |
| `TimerMinutes`, `TimerNudge`, `TimerStart`, `TimerBreak`, `TimerAdd`, `TimerPause`, `TimerStop`, `TimerDismiss` | Timer controls: a preset, − / +, **Start timer** / **Start focus**, the offered break, **+5** (on the finished card it starts a new 5 min timer), the ring (pause / resume), **Stop**, **Stop alarm** / **Dismiss**. Every change to a running timer is saved to `timer.json`. |
| `SetPowerMode` | Sets the Windows power mode. |

Every action ends with `after_model_change`.

### Focusing a terminal

`proc::terminal_window_for(pid)` walks up the process tree (up to 8 levels) until it finds a process that owns a visible, unowned, captioned top-level window. `proc::focus_window` restores it if minimised, calls `AllowSetForegroundWindow`, and then `SetForegroundWindow`. Windows only allows that because it happens in response to the user's click on Hytte. **Never call it from a background event**: it would fail, and it would also break the "never steal focus" promise.

## Keyboard

Everything above is mouse-driven, and the window never takes focus on its own. For keyboard use, set `[general] hotkey` to a chord such as `"Win+Alt+N"` (empty = off). `config::parse_hotkey` needs at least one modifier (`Win`, `Ctrl`, `Alt`, `Shift`) and exactly one key (a letter, a digit or `F1`–`F24`), so a typo can't claim a bare key system-wide. `Ui::register_hotkey` registers it with `RegisterHotKey` (and `MOD_NOREPEAT`) at startup and when the config changes; if another app owns the chord, or the text isn't understood, a line goes to `hytte.log`.

Pressing it enters **keyboard mode** (`Ui::toggle_keyboard`): the pill remembers the foreground window, opens and stays open (the collapse timer is ignored), and takes focus with `SetForegroundWindow` (allowed, because the user just pressed a key). Then:

| Key | Does |
|---|---|
| Left / Right | Previous / next panel, with the paging slide. |
| Tab / Shift+Tab | Moves a blue **focus ring** over the clickable regions in the order they are drawn. |
| Enter / Space | Runs the action of the focused region. |
| Esc, or the hotkey again | Closes the pill and gives focus back to the remembered window. |

Clicking another window ends keyboard mode without taking focus back (`WM_KILLFOCUS`). The ring is drawn by `Renderer::hit` for the region whose index equals `Frame::focus`, only while the live scene is drawing (the scene fading out never shows it).

## Drag and drop

### Arming the top edge

The window only receives drops where it is hit-testable (non-transparent pixels, or inside the window region), and the collapsed pill is small. So a **low-level mouse hook** (`mouse_ll`) watches for a left-button drag that enters a zone ±230 logical px around the screen centre and 40 px tall at the top edge. It then sets `ARMED` and posts `WM_DRAG_ARM`. A window being moved or resized by its title bar (`GetGUIThreadInfo` reports `GUI_INMOVESIZE`, `moving_window`) is not a file, so it doesn't arm. The hook itself only touches atomics and posts messages.

While `armed`, the renderer widens the presented area to the whole canvas width and fills the 40 px strip with an almost invisible colour (alpha 0.008). Windows then routes drops anywhere in the strip to Hytte, and the user doesn't have to aim. Releasing the button posts `WM_DRAG_END`.

### Dropping onto the pill

`Target` implements OLE `IDropTarget`:

- `DragEnter` accepts the data if it offers files (`CF_HDROP`) or Unicode text (`CF_UNICODETEXT`), and pushes `UiEvent::DragEnter`. The model shows the purple drop zone.
- `DragLeave` pushes `UiEvent::DragLeave`.
- `Drop` extracts the file paths (or, failing that, up to 4 M characters of text) into a `DropJob` and pushes `UiEvent::ShelfAdd`. The file work then happens on a worker ([section 10](10-shelf-and-drop-vault.md)).

### Dragging a tile out

Once a pressed tile moves past 4 px, `shelf_drag` asks the shell for a real file data object (`SHCreateItemFromParsingName` → `BindToHandler(BHID_DataObject)`) and runs `DoDragDrop` with copy and move allowed. Any app that accepts files from Explorer accepts it. `DoDragDrop` runs a modal message loop, which is why it is called **outside** the `Ui` borrow. Afterwards, the tile is removed if the drop succeeded and `remove_after_drag` is on, and any tile whose file no longer exists (it was moved) is pruned.

## Fullscreen suppression

Two `SetWinEventHook` callbacks feed it:

- **Foreground changed** posts `WM_FG`, which arms `T_FS`.
- **The foreground window moved or resized** posts `WM_FG` too. Dragging a window fires this on every mouse move, so the expensive work waits until movement settles.

Neither re-asserts topmost on the spot: the new foreground may be a game entering exclusive fullscreen, and pushing a topmost window over it at that moment can knock it out of fullscreen. `T_FS` re-asserts after the decision, and only when it isn't a game.

When `T_FS` fires, `Ui::evaluate_fullscreen` calls `fullscreen::evaluate`, which decides:

The foreground window is first taken up to its root (`GetAncestor(GA_ROOT)`), so a game's own popup still counts as the game, and measured against **its own** monitor (`MonitorFromWindow`), with 2 px of slack for DPI rounding. Then:

1. The foreground exe is on `deny_list` → **game**.
2. `suppress_fullscreen = false`, the exe is on `allow_list`, or the foreground is the shell → **show**. The shell is the desktop (`Progman`, `WorkerW`, `SHELLDLL_DefView`), the taskbars (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`), Hytte itself, "no foreground window", and any window of `explorer.exe` (except File Explorer, `CabinetWClass`), `ShellExperienceHost`, `StartMenuExperienceHost`, `SearchHost`, `LockApp` or `ShellHost`. Task View, Alt-Tab and the Snap overlay are explorer windows that cover the monitor without a title bar; without this they counted as games.
3. The foreground is a tool window, click-through (`WS_EX_TRANSPARENT`) or cloaked → **no change**. These are the Xbox Game Bar, GPU and OEM overlays: they say nothing about what is underneath, and flipping on them showed the pill over a game.
4. The shell reports exclusive D3D fullscreen or presentation mode, and the window covers its monitor without being maximised → **game**. The shell's state is system-wide: a game left running behind a maximised window still reports it, so it only counts when the foreground looks fullscreen itself.
5. The window covers its monitor and either the shell reports some fullscreen app (`QUNS_BUSY`, `QUNS_APP`) and the window isn't a captioned maximised one, or it isn't maximised (`IsZoomed`) and has no frame (no `WS_CAPTION`, no `WS_THICKFRAME`) → **hide** for browsers and media players (`fullscreen::MEDIA`: a video gets the thin line), **game** for everything else. A window that goes fullscreen from maximised (Edge, many games) stays zoomed but drops its title bar, so `fs` decides there.
6. The window covers its monitor otherwise, or is maximised → **hide**. A maximised window fills only the work area when the taskbar is visible, so it doesn't cover the monitor, but it still sits under the pill. Frameless ones (Electron, VS Code) are included, where drag and drop must keep working.
7. Otherwise → **show**.

"Hide" only applies when that monitor is the pill's (`Ui::hmon`); a game hides Hytte on any monitor. The inputs live in `fullscreen::Fg` and the decision is the pure `fullscreen::decide`, which is unit-tested.

The check also re-runs after resume from sleep and after explorer restarts (`TaskbarCreated`), when no foreground event arrives.

The window region is the pill plus its glow. Scenes without a glow (the sentinel, idle) get a 4 px margin instead of 28, so the thin line over a fullscreen browser doesn't swallow clicks on the tabs around it (a cross-process `HTTRANSPARENT` would not pass them on).

"Hide" sets `fs_hidden`. With `fullscreen_mode = "sentinel"`, the scene becomes the 3 px `Sentinel` bar (hovering it still opens the pill). With `"hide"`, `Ui::hidden()` becomes true and the window fades out, unless a drag is armed or the model is forcing visibility.

"Game" also sets `fs_hidden`, and `Ui::set_gaming` makes Hytte step aside completely, so it costs a game no frames and no input latency:

- `hidden()` is true whatever `fullscreen_mode` says, even over a drop or forced visibility, and `vis` is snapped to 0 so no fade frames are drawn over the game. A visible overlay would force DWM to compose the game instead of letting it flip independently.
- `input_hooks(false)` removes the low-level mouse hook and the LOCATIONCHANGE hook, which every mouse move in the system otherwise passes through. The foreground hook stays, because it is how leaving the game is noticed. Limit: a game that switches itself to windowed mode without a focus change keeps Hytte hidden until the next Alt-Tab.
- The global hotkey is unregistered (`register_hotkey` skips while gaming), so the game can use the chord.
- `SetPriorityClass` with `PROCESS_MODE_BACKGROUND_BEGIN` (background I/O and memory priority) and `IDLE_PRIORITY_CLASS` (CPU: thread priorities drop from 8 to 4). Background mode alone left the thread priorities unchanged, hence both.
- `fullscreen::GAMING` is set, and the port and resource-hog checks skip their scans (`ports.rs`).

Leaving reverses each step, and asks for a fresh port scan.

Every change of decision is logged once to `hytte.log` (`fullscreen: Some(Game) foreground=… Fg { busy, fs, covers, caption, frame, zoomed, shell, ignore, same_monitor }`, via `logging::note_fullscreen`), as are `game mode on` / `game mode off`. A pill that backs off unexpectedly, or a game that isn't detected, can be explained from the log.

`Ui::hidden()` combines everything that hides the window: the tray **Pause**, game mode, or fullscreen in hide mode. When hidden, the `vis` spring fades to 0, and the window is then actually hidden with `ShowWindow(SW_HIDE)`. It is shown again before fading back in.

## Virtual desktops

On every `T_FS`, `Ui::follow_desktop` asks `IVirtualDesktopManager` whether the pill is on the current desktop. If not, it moves the window to the foreground window's desktop. That keeps the shelf reachable after switching desktops.

## Tray

`tray.rs` adds a notification-area icon whose callback message is `WM_TRAY`. Right-clicking it builds a popup menu (Pause, Launch at startup, Set up terminal integration and Connect AI agents show check marks). `SetForegroundWindow` is called on Hytte's window first, because Windows requires it for the menu to close properly when you click elsewhere. The icon is re-added when Explorer restarts (`TaskbarCreated`) and removed on exit.

Next: [8. Rendering and animation](08-rendering-and-animation.md)
