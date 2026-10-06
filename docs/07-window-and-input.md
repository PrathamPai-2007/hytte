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

The pill is drawn into a fixed **canvas** of 460 × 310 logical px, centred on the primary monitor's top edge. With the `gpu` renderer the window *is* that canvas and a region limits what is visible and clickable; with `classic`, only the part that holds the pill (plus margins for the glow) is drawn and presented each frame. See [section 8](08-rendering-and-animation.md).

The renderer is picked before the window exists, because the style depends on it: `run_windows` creates a `WS_EX_NOREDIRECTIONBITMAP` window and tries `Renderer::new_gpu`; if that fails (or `[general] renderer = "classic"`) it destroys that window and creates a layered one for `Renderer::new`. A static `LIVE` flag keeps the destroyed window's `WM_DESTROY` from ending the message loop before it starts.

### Monitor and DPI

`Ui::refresh_monitor` reads the primary monitor's rectangle and its effective DPI, computes `scale = dpi / 96`, resizes the renderer's bitmap (or swap chain), and recomputes the drop-zone rectangle. It runs at startup and on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`. The app manifest (`build.rs`) declares per-monitor DPI awareness v2, so Windows doesn't bitmap-stretch the pill.

## The `Ui` struct

Thread-local UI state (accessed through `with_ui`; see [section 3](03-architecture.md#ownership-and-re-entrancy)):

| Field(s) | Meaning |
|---|---|
| `cfg`, `model`, `anim`, `rend` | Config, the model ([section 6](06-ui-model.md)), animation state, and the renderer. |
| `scale`, `mon` | DPI scale and monitor rectangle. |
| `hover` | The pill is "open because of the mouse" (set after the dwell delay). |
| `inside` | The pointer is inside the pill shape right now. |
| `mouse`, `origin`, `crop_w`, `hits` | Pointer position, where the last frame was placed on screen, and its clickable regions. |
| `fs_hidden`, `paused`, `armed`, `shown` | Fullscreen suppression, the tray Pause state, a drag in progress near the top edge, and whether the window is shown. |
| `drag` | A shelf tile was pressed: it becomes an OLE drag once the pointer moves more than 4 px. |
| `wheel` | Accumulated wheel delta (precision touchpads send fractions of a notch). |
| `thumb_req`, `tabs`, `vdm` | Thumbnails already requested, the terminal-tabs worker, and the virtual desktop manager. |

Three methods do most of the work:

- `layout()`: computes the scene from the model and sets every spring target (size, glow, hover brightness, visibility).
- `kick()`: makes sure the frame clock is running.
- `after_model_change()`: called after **any** model change. It finishes a due timer, (re)arms `T_TIMER` and `T_EXPIRE`, requests missing thumbnails, then runs `layout()` and `kick()`.

## Messages handled by `wndproc`

| Message | What happens |
|---|---|
| `WM_MOUSEACTIVATE` | Returns `MA_NOACTIVATE`. |
| `WM_TICK` (custom) | `Ui::frame`: one animation frame. |
| `WM_UI` (custom) | `Ui::handle_events`: drains all queued `UiEvent`s. |
| `WM_FG` (custom) | The foreground window changed or moved; see [Fullscreen](#fullscreen-suppression). |
| `WM_DRAG_ARM` / `WM_DRAG_END` (custom) | A drag entered the top-edge zone / the button was released. |
| `WM_MOUSEMOVE` | Updates the pointer, starts leave-tracking, starts or cancels the hover timers, starts a tile drag past 4 px, and redraws so hover effects follow the pointer. |
| `WM_MOUSELEAVE` | The pointer left the window: clears it and arms the collapse timer. |
| `WM_MOUSEWHEEL` | Switches panels one per notch, or nudges the timer minutes in the timer panel. |
| `WM_SETCURSOR` | A hand cursor over clickable regions. |
| `WM_LBUTTONDOWN` | On a shelf tile: remembers it and captures the mouse, ready for a drag. |
| `WM_LBUTTONUP` | A click: runs the `Action` under the pointer. |
| `WM_RBUTTONUP` | Over the pill: toggles the timer panel. |
| `WM_TIMER` | See the timers below. |
| `WM_DISPLAYCHANGE`, `WM_DPICHANGED` | `refresh_monitor`, then re-evaluate fullscreen. |
| `WM_SETTINGCHANGE` | Re-reads the Windows "Animation effects" setting (`reduce_motion`). |
| `WM_TRAY` (custom) | Right-click on the tray icon: shows the menu. |
| `WM_COMMAND` | Tray menu items: 10 Pause, 11 Open settings folder, 12 Quit, 13 Launch at startup, 14 Set up terminal integration (`setup::tray_setup`, which works on its own thread and reports in a message box). |
| `TaskbarCreated` (registered) | Explorer restarted: re-adds the tray icon. |
| `WM_POWERBROADCAST` | Power source, charge level or resume changed: wakes the power worker (`power::poke`). |
| `WM_DESTROY` | Ends the message loop (unless the window was torn down during startup). |

## Timers

All are one-shot Win32 timers: the handler kills the timer first, so each fires once until re-armed.

| Id | Delay | Armed when | On fire |
|---|---|---|---|
| `T_DWELL` | 120 ms | The pointer enters the pill | If still inside: `hover = true` (the pill opens) and a port rescan is requested. |
| `T_COLLAPSE` | 300 ms | The pointer leaves while open | If still outside: `hover = false` and the timer panel closes. Re-entering in time cancels it, so grazing the edge doesn't flicker. |
| `T_EXPIRE` | Until the model's next deadline | After every model change | `after_model_change` (calls `Model::expire`). |
| `T_TIMER` | At most 1 s | While a countdown timer runs unpaused | `after_model_change`: redraws the fuse and fires the timer when it reaches zero. |
| `T_FS` | 150 ms | On a foreground change or move | Re-asserts topmost, re-evaluates fullscreen, follows the virtual desktop. |

## Clicks and actions

The renderer records a list of **hit regions** while drawing each frame (`Hit { rect, action }`); see [section 8](08-rendering-and-animation.md). `Ui::hit_at` finds the topmost region under the pointer, and `Ui::run_action` executes its `Action`:

| Action | Effect |
|---|---|
| `DismissTask`, `DismissChip` | Removes a finished task / the chip. |
| `CopyText`, `OpenPath` | Clipboard / opens Explorer. |
| `MediaPrev`, `MediaToggle`, `MediaNext`, `FocusMedia` | Media transport commands (on a short-lived thread) / brings the player to the front. |
| `FocusTerminal(pid, id)` | Finds the terminal window for `pid` and focuses it, then asks the tabs worker to select the remembered tab. |
| `SelectPanel` | Switches the panel. Opening Ports forces a rescan; opening Home re-reads the battery. |
| `ToggleMic` | Mutes or unmutes every capture device. |
| `OpenPort`, `KillPort` | Opens `http://localhost:<port>` / two-step kill (first click arms **Kill?** for 3 s). If the process is already gone, the row is just removed. |
| `ShelfTile`, `RemoveShelf`, `ShelfOp` | Select a tile / remove it / queue a Drop Vault job (shows "Working…"). |
| `TimerMinutes`, `TimerStart`, `TimerBreak`, `TimerAdd`, `TimerPause`, `TimerStop`, `TimerDismiss` | Timer controls. Every change is saved to `timer.json`. |
| `SetPowerMode` | Sets the Windows power mode. |

Every action ends with `after_model_change`.

### Focusing a terminal

`proc::terminal_window_for(pid)` walks up the process tree (up to 8 levels) until it finds a process that owns a visible, unowned, captioned top-level window. `proc::focus_window` restores it if minimised, calls `AllowSetForegroundWindow`, and then `SetForegroundWindow`. Windows only allows that because it happens in response to the user's click on Hytte. **Never call it from a background event**: it would fail, and it would also break the "never steal focus" promise.

## Drag and drop

### Arming the top edge

The window only receives drops where it is hit-testable (non-transparent pixels, or inside the window region), and the collapsed pill is small. So a **low-level mouse hook** (`mouse_ll`) watches for a left-button drag that enters a zone ±230 logical px around the screen centre and 40 px tall at the top edge. It then sets `ARMED` and posts `WM_DRAG_ARM`. The hook itself only touches atomics and posts messages.

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

- **Foreground changed** posts `WM_FG`, which re-asserts topmost immediately and arms `T_FS`.
- **The foreground window moved or resized** posts `WM_FG` with `FG_MOVED`, which only re-arms `T_FS`. Dragging a window fires this on every mouse move, so the expensive work waits until movement settles.

When `T_FS` fires, `Ui::evaluate_fullscreen` calls `fullscreen::evaluate`, which decides:

1. The foreground exe is on `deny_list` → **hide**.
2. `suppress_fullscreen = false`, or the exe is on `allow_list` → **show**.
3. The shell reports D3D fullscreen, presentation mode or "busy" (`SHQueryUserNotificationState`), **or** the foreground window covers the whole monitor and isn't a shell window → **hide**. Shell windows are the desktop (`Progman`, `WorkerW`, `SHELLDLL_DefView`), the taskbars (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd`), Hytte itself, and "no foreground window". The desktop covers the whole monitor and Windows can report "busy" while it is focused, so without this exemption the pill stayed a thin line on the desktop.
4. Otherwise → **show**.

"Hide" sets `fs_hidden`. With `fullscreen_mode = "sentinel"`, the scene becomes the 3 px `Sentinel` bar (hovering it still opens the pill). With `"hide"`, `Ui::hidden()` becomes true and the window fades out, unless a drag is armed or the model is forcing visibility.

Every change of decision is logged once to `hytte.log` (`fullscreen: hide=… foreground=… covers=… shell=…`, via `logging::note_fullscreen`), so a pill that backs off unexpectedly can be explained from the log.

`Ui::hidden()` combines everything that hides the window: the tray **Pause**, or fullscreen in hide mode. When hidden, the `vis` spring fades to 0, and the window is then actually hidden with `ShowWindow(SW_HIDE)`. It is shown again before fading back in.

## Virtual desktops

On every `T_FS`, `Ui::follow_desktop` asks `IVirtualDesktopManager` whether the pill is on the current desktop. If not, it moves the window to the foreground window's desktop. That keeps the shelf reachable after switching desktops.

## Tray

`tray.rs` adds a notification-area icon whose callback message is `WM_TRAY`. Right-clicking it builds a popup menu (Pause, Launch at startup and Set up terminal integration show check marks). `SetForegroundWindow` is called on Hytte's window first, because Windows requires it for the menu to close properly when you click elsewhere. The icon is re-added when Explorer restarts (`TaskbarCreated`) and removed on exit.

Next: [8. Rendering and animation](08-rendering-and-animation.md)
