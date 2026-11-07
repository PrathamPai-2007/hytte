# 7. Window, input and timers

`crates/hytte/src/window.rs` (and `window/`) is the Win32 layer: it owns the window, turns OS messages into model changes, runs the timers, and handles drag and drop, fullscreen suppression and the tray. It all runs on the UI thread except a few tiny callbacks that only set atomics and post messages.

## The window

Created once in `run_windows`:

| Setting | Effect |
|---|---|
| `WS_POPUP` | No frame or title bar. |
| `WS_EX_NOREDIRECTIONBITMAP` | No GDI surface; DirectComposition supplies the pixels. Clicks pass through everywhere outside a **window region** that tracks the pill and its glow. |
| `WS_EX_TOPMOST` | Above normal windows; re-asserted when the foreground changes. |
| `WS_EX_TOOLWINDOW` | No taskbar button or Alt-Tab entry. |
| `WS_EX_NOACTIVATE` | Never active. With `WM_MOUSEACTIVATE → MA_NOACTIVATE`, clicking the pill never steals keyboard focus. |
| Class `HyttePill`, `CS_DBLCLKS` | The class name is how fullscreen detection recognises Hytte's own window. |

The pill is drawn into a fixed **canvas** of 460 × 310 logical px, centred on the top edge of the monitor chosen by `[general] monitor`; a region limits what is visible and clickable ([section 8](08-rendering-and-animation.md)). If the renderer can't start, the window is destroyed and Hytte exits (reason in `hytte.log`); a static `LIVE` flag stops that `WM_DESTROY` from ending a loop that never started.

### Monitor and DPI

`Ui::refresh_monitor` reads the monitor rectangle and effective DPI, computes `scale = dpi / 96 × [general] pill_scale` (0.7–1.6), resizes the swap chain and recomputes the drop zone. The top edge is the work area's top when lower than the monitor's, so a taskbar docked on top leaves the pill below it. It runs at startup, on `WM_DISPLAYCHANGE` / `WM_DPICHANGED`, and when `monitor` or `pill_scale` changes. `app.manifest` declares per-monitor DPI awareness v2.

`[general] monitor` (`Ui::wanted_monitor`): `"primary"` (default), `"active"` (monitor of the foreground window), `"cursor"` (under the pointer), or a number. For `active` / `cursor`, `Ui::follow_monitor` runs when the foreground settles (`T_FS`, 150 ms), moves the pill if monitor or DPI changed, and re-runs the fullscreen check there. Limit: the drag-arming zone belongs to the pill's monitor, so a drag to another monitor's top edge doesn't arm until the pill has moved.

## The `Ui` struct

Thread-local, accessed through `with_ui` ([section 3](03-architecture.md#ownership-and-re-entrancy)):

| Field(s) | Meaning |
|---|---|
| `cfg`, `model`, `anim`, `rend` | Config, model ([section 6](06-ui-model.md)), animation, renderer. |
| `scale`, `mon`, `hmon` | DPI scale, monitor rectangle, the `HMONITOR` (fullscreen checks compare against it). |
| `hover`, `inside` | Open because of the mouse (after `hover_delay_ms`, or a click with `open_on = "click"`) / pointer inside the pill right now. |
| `mouse`, `origin`, `crop_w`, `hits` | Pointer, where the last frame was placed, its clickable regions. |
| `fs_hidden`, `gaming`, `paused`, `armed`, `shown` | Fullscreen suppression, game mode, tray Pause, drag near the top edge, window shown. |
| `drag` | A shelf tile was pressed: an OLE drag starts once the pointer moves > 4 px. |
| `wheel`, `swipe` | Accumulated wheel delta (touchpads send fractions); `ui_state::Swipe` turns sideways scroll into one page per gesture: it pages once a gesture moves 80; a gesture ends after a 100 ms pause or two opposite deltas in a row (one is touchpad noise); deltas are smoothed; after a page, the same stream pages again only after 160 ms, once below 45 % of its peak and then 1.8× faster (a new swipe in old inertia). |
| `pressed`, `light` | Left button down over a clickable region; resolved `theme`. |
| `kb`, `kb_focus` | Keyboard mode (window to give focus back to) and the focus-ring region. |
| `thumb_req`, `tabs`, `vdm` | Thumbnails requested, terminal-tabs worker, virtual desktop manager. |
| `scrub` | Volume bar or timeline being dragged (`Action::Volume` / `Seek`) and its edges. |
| `ringing`, `awake_on` | Timer alarm playing; keep-awake held. |
| `dim` | Idle pill visibility: eases towards `idle_opacity` while nothing shows and the pointer is away, else 1; scales `present`'s alpha. |
| `last_snip`, `snip_seq`, `shots`, `paste` | Screenshot tracking: hash of the last one (the Snipping Tool can announce twice), clipboard sequence number, shelf screenshots with arrival times, newest awaiting Ctrl+V ([section 10](10-shelf-and-drop-vault.md#screenshots)). |
| `gpu_retry` | GPU rebuild backoff in ms (0 when healthy). |

Key methods: `layout()` (scene from the model; sets every spring target); `kick()` (ensure the frame clock runs); `after_model_change()`, called after **any** model change: finishes a due timer, (re)arms `T_TIMER` / `T_EXPIRE`, starts or stops the alarm (`timer::ring`, with `timer_done` and `[timer] sound`), takes or releases keep-awake (`SetThreadExecutionState` with `ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED`; Windows releases it if Hytte exits), turns a due calendar heads-up into a card, requests missing thumbnails, then `layout()` and `kick()`.

## Messages handled by `wndproc`

| Message | What happens |
|---|---|
| `WM_MOUSEACTIVATE` | `MA_NOACTIVATE`. |
| `WM_TICK` / `WM_UI` / `WM_FG` (custom) | `Ui::frame` / `Ui::handle_events` (drain all queued events) / foreground changed or moved ([Fullscreen](#fullscreen-suppression)). |
| `WM_DRAG_ARM` / `WM_DRAG_END` (custom) | A drag entered the top-edge zone / the button was released. |
| `WM_MOUSEMOVE` | While scrubbing: sets volume or timeline knob (`scrub_to`) and nothing else. Otherwise updates the pointer, starts leave-tracking, starts or cancels hover timers (`hover_delay_ms` 0 opens at once; `open_on = "click"` never starts them), starts a tile drag past 4 px, and redraws for hover effects. |
| `WM_MOUSELEAVE` | Clears the pointer and arms the collapse timer. |
| `WM_HOTKEY` | `hotkey`: enter or leave [keyboard mode](#keyboard). `monitor_hotkey` (id 2): `Ui::next_monitor`. |
| `WM_KEYDOWN` / `WM_KILLFOCUS` | Keyboard mode keys (Esc, Left/Right, Tab, Enter/Space) / ends keyboard mode (and typing) without taking focus back. While a text field has focus, `WM_KEYDOWN` goes to `Ui::typing_key` instead. |
| `WM_CHAR` | A typed character (UTF-16 unit; an emoji arrives as two) into the focused text field, via `Ui::type_char`. |
| `WM_MOUSEWHEEL` | Over the volume bar or **Sound** tile: volume, 2 % a notch. Over an app's bar: that app. Over the shelf row: scrolls a tile per notch. Over a task list with more rows than `task_rows`: a row per notch (at either end the wheel pages again). On the timer card: minutes. Otherwise a mouse wheel (whole 120 notches) pages the panes (down = next), not while the settings pane is open. A precision touchpad's vertical scroll (fractional deltas, as do free-spin wheels) does nothing there, so a two-finger scroll can't flip panes. On the lyrics page a notch moves a line, and a touchpad needs 600 units of scroll per line (`Ui::lyric_wheel`) so its stream of small deltas and its coasting don't fling the page. |
| `WM_MOUSEHWHEEL` | Touchpad swipe or tilt wheel: next pane right, previous left, one per swipe; a collapsed pill peeks 3 s. |
| `WM_SETCURSOR` | Hand cursor over clickable regions. |
| `WM_LBUTTONDOWN` | On a shelf tile: remember it, capture the mouse for a drag. On the volume bar or timeline: set there and capture for dragging (`scrub`). |
| `WM_CAPTURECHANGED` | Capture lost mid-drag (Alt+Tab, dialog): the drag ends. |
| `WM_LBUTTONUP` | Ends a volume drag (re-reads the real level) or timeline drag (asks the player to seek once via `media::Cmd::Seek`, GSMTC `TryChangePlaybackPositionAsync`; `Media` updates are ignored during the drag). Otherwise a click: with `open_on = "click"` the first click on the closed pill just opens it; else it runs the `Action` under the pointer. |
| `WM_RBUTTONUP` | Over the pill: toggles the timer setup card. |
| `WM_TIMER` | See [Timers](#timers). |
| `WM_DISPLAYCHANGE`, `WM_DPICHANGED` | `refresh_monitor`, re-evaluate fullscreen. |
| `WM_SETTINGCHANGE` | Re-reads "Animation effects" (`reduce_motion`), theme and accent. |
| `WM_TRAY` (custom) | Tray click: shows the menu. |
| `WM_COMMAND` | Tray items: 10 Pause, 11 Open settings folder, 12 Quit, 13 Launch at startup, 14 Set up terminal integration (`setup::tray_setup`, on its own thread, reports in a message box), 15 Check for updates (`update::check_now`), 16 Connect AI agents (`agents::tray`, likewise), 17 Move to next monitor (`Ui::next_monitor`, listed with two or more monitors), 18 Help (opens the manual on GitHub). |
| `TaskbarCreated` | Explorer restarted: re-add the tray icon, post `WM_FG`. |
| `WM_POWERBROADCAST` | Wakes the power worker (`power::poke`); on resume also posts `WM_FG`. |
| `WM_DESTROY` | Ends the message loop (unless torn down during startup). |

**Settings pane.** The gear on Home (`Action::Settings`) opens a header (back, three page tabs, an open button) over five rows. Each switch is an `Action::Set(key, Val)`: `config::set` writes that one key to `config.toml` (comments kept) and returns the parsed file, which goes straight through `apply_config`, so it shows on the next frame; the config watcher re-applies it a moment later (no change). If the file doesn't parse, nothing is written and a card says so. Downloads and the calendar start their watcher the first time they are switched on; off, the UI ignores their reports. Rows come from `render::settings_table`, the one table that also feeds the settings window: each row carries its pane page (or -1 for window only), and a test checks every row writes a key `config.toml` accepts.

**Settings window.** The open button (`Action::OpenConfig`) opens `settings_win.rs`: an ordinary resizable window (class `HytteSettings`, themed title bar) on the same UI thread, drawn by a second `Renderer` (`Renderer::new_for_window`, `draw_settings_window`) into its own swap chain. It shows every `settings_table` row in cards, with a section list on the left (following the scroll at ≥ 720 logical px wide). It redraws on input, and a 60 Hz `WM_TIMER` runs while anything eases. Typing filters the page (`WM_CHAR` builds a query; rows whose label, `render::help` text or section name don't match are hidden; Backspace erases, Esc clears the query before closing, the cross clears it via `Action::ClearSearch`). A row whose value differs from `Config::default()` shows a ↺ in a fixed right column (`RESET_GUT`), which writes the exact default (an ordinary `Action::Set`). Resting on a label for 0.6 s fades in a note (`render::help`, `Renderer::tip`). It follows live theme/accent changes. Clicks are `Action::Set`, `Jump` (section), `Opacity` (slider: live while dragged, written on release) and `EditConfig` ("Custom": `config::ensure_noted` adds the key to the file with its default under a comment saying what it takes, the file opens, a note appears; a number that matches no step lights Custom and shows its value). The pill hears of changes through its config watcher; `settings_win::refresh` shows changes made elsewhere. Lists, the hotkey and folders stay in `config.toml`.

**Behaviour settings here.** `hover_delay_ms`, `collapse_delay_ms`, `open_on` drive hover timers and clicks. `start_panel` applies on collapse (`"auto"` forgets the panel, `"home"` selects Home, `"last"` leaves it). `animation` scales the spring delta (calm 0.7, normal 1, snappy 1.4). `idle_opacity` drives `dim`. `quiet_hours` ("HH:MM-HH:MM", `config::in_quiet_hours`, may wrap midnight) silences agent beeps and the timer alarm (`Ui::quiet`). `pin_button` (off by default) adds a pin to the tab strip: `Action::Pin` toggles `Model::pinned`, and a pinned pill ignores `T_COLLAPSE` until unpinned, the setting is switched off, or a fullscreen app takes the screen. `remember_panel` (off; only with `start_panel = "last"`) writes the panel to `%APPDATA%\Hytte\panel.txt` on collapse and restores it at next start.

## Timers

All are one-shot Win32 timers (the handler kills the timer first).

| Id | Delay | Armed when | On fire |
|---|---|---|---|
| `T_DWELL` | `hover_delay_ms` (200; 0 skips it; not with `open_on = "click"`) | Pointer enters | If still inside: `hover = true`, port rescan requested, battery re-read when Power is opening. |
| `T_COLLAPSE` | `collapse_delay_ms` (300) | Pointer leaves while open | If still outside and not pinned: `hover = false`, timer card and settings pane close, `start_panel` applied. Re-entering in time cancels it. |
| `T_EXPIRE` | Until the model's next deadline | After every model change | `after_model_change` (calls `Model::expire`). |
| `T_TIMER` | ≤ 1 s | Countdown timer running | Redraws the fuse; fires the timer at zero. |
| `T_FS` | 150 ms | Foreground change or move | Follows the monitor, re-evaluates fullscreen (not while Hytte holds focus for typing or keyboard mode: Hytte in front says nothing about the app under it, and reading it as "no fullscreen" used to collapse the pill under the click), re-asserts topmost (not over a game), follows the virtual desktop. |
| `T_TRIM` | 30 s | A frame finds nothing moving | If still settled, trims the working set ([section 12](12-performance.md)). |
| `T_GPU` | 1 s doubling to 30 s | GPU rebuild failed | Retries `rebuild_gpu` ([section 8](08-rendering-and-animation.md#the-gpu-renderer)). |
| `T_CARET` | 530 ms | A text field takes focus, and after every blink | Toggles the caret and re-arms, while a field has focus; otherwise leaves it shown and stops. |
| `T_PLACE` | 250 ms | The weather place field changed | Runs the place search for what is typed (`Ui::place_search`). |
| `T_WIDE` | 2 s | Open-Meteo answered a place search with nothing | Offers "search more widely" (`Ui::wide_offer_due`) if the field still holds that text. Typing cancels it. |
| `T_SHOT` | 60 ms while a screenshot awaits Ctrl+V (`delete_after_paste`, ≤ 10 min), else 1 s while screenshots are on the shelf and `screenshot_secs` ≠ 0 | A screenshot was shelved | `Ui::tick_shots`: removes expired and pasted screenshots; re-arms while any is pending. |

## Clicks and actions

While drawing, the renderer records **hit regions** (`Hit { rect, action }`, [section 8](08-rendering-and-animation.md)). `Ui::hit_at` finds the topmost under the pointer; `Ui::run_action` runs its `Action`:

| Action | Effect |
|---|---|
| `DismissTask`, `DismissChip` | Remove a finished task / the chip. |
| `CopyText`, `OpenPath` | Clipboard / Explorer. |
| `MediaPrev`, `MediaToggle`, `MediaNext`, `FocusMedia` | Media commands (short-lived thread) / raise the player. |
| `FocusTerminal(pid, id)` | Focus the terminal for `pid`, then have the tabs worker select the remembered tab. Used by task rows and servers that belong to a task. |
| `Pin`, `SelectPanel` | Toggle pin (unpinning with the pointer away starts the collapse timer) / switch panel (Tasks forces a port rescan; Power re-reads the battery). |
| `ToggleMic`, `ToggleSpeaker`, `Volume` | Mute every capture device / mute the speaker / the volume bar (mouse presses are a `scrub_to` drag, so the action is reached from the keyboard). |
| `CycleAwake`, `OpenTimer` | Off → 1 hour → On → Off (`Awake::next`) / Timer panel if a timer runs, else the setup card. |
| `OpenPort`, `KillPort` | Open `http://localhost:<port>` / two-step kill (first click arms **Kill?** for 3 s); a vanished process just removes the row. |
| `ShelfTile`, `RemoveShelf`, `ShelfOp` | Select / remove a tile / queue a Drop Vault job ("Working…"). |
| `TimerMinutes`, `TimerNudge`, `TimerStart`, `TimerBreak`, `TimerAdd`, `TimerPause`, `TimerStop`, `TimerDismiss` | Timer controls (preset, − / +, start timer or focus, offered break, **+5** (on the finished card, a new 5 min timer), ring pause/resume, Stop, Stop alarm / Dismiss). Changes to a running timer save to `timer.json`. |
| `SetPowerMode` | Sets the Windows power mode. |
| `Field(f)`, `NoteKeep`, `ToggleJot`, `EditJot`, `KeepJot`, `DeleteJot`, `UndoJot` | Focus a text field and place the caret / the 24 h / Keep switch / tick a to-do / edit an item in the field / keep a temporary note / delete (Undo for 6 s). |
| `PickPlace`, `ChangePlace`, `CancelPlace`, `RetryWeather`, `SearchWide` | Weather: choose a suggestion, open the chooser, close it, fetch again, run the OpenStreetMap search the user asked for. |
| `PaneMove(key, ±1)` | Settings window only: move a pane in `[panes] order` (written with `config::set_text`). |
| `NudgeSnooze`, `NudgeSkip` | Break reminder: ten minutes later / done. |
| `SleepMenu`, `SleepSet`, `SleepCancel`, `LyricsOpen`, `LyricsClose`, `LyricsRetry` | The Media pane's sleep timer chips and lyrics page. |

Every action ends with `after_model_change`.

### Focusing a terminal

`proc::terminal_window_for(pid)` walks up the process tree (≤ 8 levels) to a process owning a visible, unowned, captioned top-level window. `proc::focus_window` restores it if minimised, calls `AllowSetForegroundWindow`, then `SetForegroundWindow`. Windows allows that only because it answers the user's click on Hytte. **Never call it from a background event**: it would fail and break the "never steal focus" promise.

## Keyboard

The window never takes focus on its own. For keyboard use, set `[general] hotkey` to a chord like `"Win+Alt+N"` (empty = off). `config::parse_hotkey` needs at least one modifier (`Win`, `Ctrl`, `Alt`, `Shift`) and exactly one key (letter, digit, `F1`–`F24`), so a typo can't claim a bare key system-wide. `Ui::register_hotkey` calls `RegisterHotKey` (`MOD_NOREPEAT`) at startup and on config change; a clash or unparseable text logs to `hytte.log`.

The hotkey enters **keyboard mode** (`Ui::toggle_keyboard`): the pill remembers the foreground window, opens and stays open (collapse timer ignored), and takes focus (allowed: the user just pressed a key).

| Key | Does |
|---|---|
| Left / Right | Previous / next panel, with the paging slide. |
| 1 to 9 | Straight to that panel along the tab strip. |
| Digits, Backspace (timer card) | Type minutes (`timer::typed`): a digit within 1.5 s of the last extends the number ("2","5" is 25), else starts a new one; 1–180. |
| Tab / Shift+Tab | Moves an accent-coloured **focus ring** over clickable regions in draw order. |
| Enter / Space | Runs the focused region's action. |
| Esc, or the hotkey | Closes the pill and returns focus. |

Clicking another window ends keyboard mode without taking focus back (`WM_KILLFOCUS`). The ring is drawn by `Renderer::hit` for the region whose index equals `Frame::focus`, only in the live scene.

## Typing

A text field (Notes, To-do, the weather place) is drawn by Hytte, since a native edit control can't draw into the composition surface. Clicking it (`Action::Field`) or pressing `[general] note_hotkey` (hotkey id 3, which opens Notes) calls `Ui::begin_typing`: the foreground window is remembered in `kb`, the pill is held open and takes focus, exactly as keyboard mode does, and a click places the caret (`Renderer::field_byte_at`). Esc, a click elsewhere or leaving the pane ends it and hands focus back (`end_keyboard`).

| Key | Does |
|---|---|
| Characters | Typed in (`WM_CHAR`); control characters are ignored. |
| Left / Right / Home / End | Move the caret; Shift extends the selection, Ctrl jumps by word. |
| Backspace / Delete | Delete (Ctrl: by word). |
| Ctrl+A / C / X / V | Select all / copy / cut / paste (line breaks become spaces). |
| Up / Down | In the place field: move the highlighted suggestion. |
| Enter | Save the note or to-do; in the place field take the highlighted place. |
| Esc | Stop typing, give focus back. |

Limit: an IME's composition window appears where Windows puts it by default, not beside the caret.

## Drag and drop

### Arming the top edge

The window receives drops only where it is hit-testable, and the collapsed pill is small. So a **low-level mouse hook** (`mouse_ll`) watches for a left-button drag entering a zone ±230 logical px around the screen centre and 40 px tall at the top, then sets `ARMED` and posts `WM_DRAG_ARM`. A window being moved or resized by its title bar (`GUI_INMOVESIZE`, `moving_window`) doesn't arm. The hook only touches atomics and posts messages.

While `armed`, the renderer widens the presented area to the whole canvas width and fills the 40 px strip with an almost invisible colour (alpha 0.008), so Windows routes drops anywhere in it to Hytte. Releasing posts `WM_DRAG_END`.

### Dropping onto the pill

`Target` implements `IDropTarget`: `DragEnter` accepts files (`CF_HDROP`) or Unicode text (`CF_UNICODETEXT`) and pushes `UiEvent::DragEnter` (purple drop zone); `DragLeave` pushes `DragLeave`; `Drop` extracts the paths (or up to 4 M characters of text) into a `DropJob` and pushes `UiEvent::ShelfAdd`. File work happens on a worker ([section 10](10-shelf-and-drop-vault.md)).

### Dragging a tile out

Once a pressed tile moves past 4 px, `shelf_drag` asks the shell for a real file data object (`SHCreateItemFromParsingName` → `BindToHandler(BHID_DataObject)`) and runs `DoDragDrop` (copy and move), so any app that accepts files from Explorer accepts it. `DoDragDrop` pumps messages, hence it runs **outside** the `Ui` borrow. Afterwards the tile is removed if the drop succeeded and `remove_after_drag` is on, and tiles whose file is gone are pruned.

## Fullscreen suppression

Two `SetWinEventHook` callbacks post `WM_FG` (arming `T_FS`): foreground changed, and foreground window moved or resized (fires on every mouse move while dragging, so expensive work waits until movement settles). Neither re-asserts topmost immediately: the new foreground may be a game entering exclusive fullscreen, and a topmost window pushed over it can knock it out. `T_FS` re-asserts after the decision, and only when not a game.

`T_FS` runs `Ui::evaluate_fullscreen` → `fullscreen::evaluate`. The foreground window is taken to its root (`GetAncestor(GA_ROOT)`, so a game's popup counts as the game) and measured against **its own** monitor with 2 px of slack for DPI rounding. Rules, first match wins:

1. Foreground exe on `deny_list` → **game**.
2. `fullscreen = "off"`, exe on `allow_list`, or the foreground is the shell → **show**. The shell means the desktop (`Progman`, `WorkerW`, `SHELLDLL_DefView`), taskbars, Hytte itself, no foreground window, and any `explorer.exe` window (except File Explorer, `CabinetWClass`), `ShellExperienceHost`, `StartMenuExperienceHost`, `SearchHost`, `LockApp`, `ShellHost`. Task View, Alt-Tab and Snap are explorer windows covering the monitor without a title bar and used to count as games.
3. Tool window, click-through (`WS_EX_TRANSPARENT`) or cloaked → **no change** (Xbox Game Bar, GPU and OEM overlays say nothing about what's underneath).
4. The shell reports exclusive D3D fullscreen or presentation mode, and the window covers its monitor without being maximised → **game**. (The shell's state is system-wide; a game left running behind a maximised window still reports it, so it counts only when the foreground itself looks fullscreen.)
5. The window covers its monitor and either the shell reports a fullscreen app (`QUNS_BUSY`, `QUNS_APP`) and the window isn't a captioned maximised one, or it isn't maximised (`IsZoomed`) and has no frame (`WS_CAPTION`, `WS_THICKFRAME`) → **hide** for browsers and media players (`fullscreen::MEDIA`: video gets the thin line), **game** for everything else. A window going fullscreen from maximised (Edge, many games) stays zoomed but drops its title bar, so `fs` decides.
6. Covers its monitor otherwise, or is maximised, or **snapped** → **hide**. (A maximised window fills only the work area when the taskbar shows, but still sits under the pill; frameless ones like Electron and VS Code are included, where drag and drop must keep working.) *Snapped* (`Fg::snapped`): resizable (`WS_THICKFRAME`), not maximised, not covering the monitor, visible top (`DWMWA_EXTENDED_FRAME_BOUNDS`) at the work area's top and reaching the monitor's centre line: a half of a split screen, a quarter along the top, or a floating window dragged across the centre (it would sit under the pill).
7. Otherwise → **show**.

"Hide" applies only when that monitor is the pill's (`Ui::hmon`); a game hides Hytte on any monitor. Inputs are `fullscreen::Fg`; the decision is the pure, unit-tested `fullscreen::decide`. The check also re-runs after resume and after Explorer restarts.

The window region is the pill plus its glow; scenes without glow (sentinel, idle) get a 4 px margin instead of 28, so the thin line over a fullscreen browser doesn't swallow clicks on its tabs (a cross-process `HTTRANSPARENT` wouldn't pass them on).

"Hide" sets `fs_hidden`. With `fullscreen = "line"` the scene becomes the `Sentinel` (hovering still opens the pill); with `"hide"`, `Ui::hidden()` turns true and the window fades out, unless a drag is armed or visibility is forced.

"Game" also sets `fs_hidden`, and `Ui::set_gaming` makes Hytte step aside completely so it costs a game no frames or input latency:

- `hidden()` is true whatever `fullscreen` says, and `vis` snaps to 0 (no fade frames; a visible overlay would force DWM to compose the game instead of letting it flip).
- `input_hooks(false)` removes the mouse hook and LOCATIONCHANGE hook (every system mouse move passes through them). The foreground hook stays: it detects leaving the game. Limit: a game switching to windowed mode without a focus change keeps Hytte hidden until the next Alt-Tab.
- The global hotkey is unregistered (`register_hotkey` skips while gaming).
- `SetPriorityClass` with `PROCESS_MODE_BACKGROUND_BEGIN` (I/O and memory) and `IDLE_PRIORITY_CLASS` (thread priorities 8 → 4; background mode alone left them unchanged).
- `fullscreen::GAMING` is set and the port and resource-hog checks skip their scans.

Leaving reverses each step and asks for a fresh port scan. Each decision change is logged once (`fullscreen: Some(Game) foreground=… Fg { busy, fs, covers, caption, frame, zoomed, snapped, shell, ignore, same_monitor }`, `logging::note_fullscreen`), as are `game mode on` / `off`, so unexpected back-offs or undetected games can be explained from the log.

`Ui::hidden()` combines tray **Pause**, game mode and fullscreen hide mode. When hidden, `vis` fades to 0 and the window is then `ShowWindow(SW_HIDE)`n; it is shown again before fading in.

## Virtual desktops

On every `T_FS`, `Ui::follow_desktop` asks `IVirtualDesktopManager` whether the pill is on the current desktop; if not it moves to the foreground window's desktop, keeping the shelf reachable.

## Tray

`tray.rs` adds a notification-area icon (callback `WM_TRAY`); a click builds a popup menu (Pause, Launch at startup, Set up terminal integration and Connect AI agents show check marks). `SetForegroundWindow` is called on Hytte's window first, since Windows needs it for the menu to close properly on an outside click. The icon is re-added after Explorer restarts and removed on exit.

Next: [8. Rendering and animation](08-rendering-and-animation.md)
