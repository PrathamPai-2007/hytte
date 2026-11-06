# 6. The UI model

`crates/hytte/src/ui_state.rs` decides *what* the pill shows. It is plain Rust with no Win32 calls, so almost all of it is unit-tested. The window layer ([section 7](07-window-and-input.md)) feeds it events and ticks; the renderer ([section 8](08-rendering-and-animation.md)) reads it to draw.

Parts: **`UiEvent`** (worker → UI messages), **`Model`** (what the pill knows), **`Anim`** (animation state).

## `UiEvent`

| Variant | Sent by | Effect |
|---|---|---|
| `Task(TaskUpdate)` | task registry (via bridge) | `Model::apply_task` ([section 4](04-ipc-protocol.md)) |
| `Media(Option<MediaInfo>)`, `MediaArt(Option<Arc<ArtBitmap>>)` | media watcher | Set or clear `media`; art bumps `art_gen` so the renderer re-uploads. |
| `Privacy(cam, mic, app)`, `MicMute(bool)` | privacy / mic watchers | Camera and mic dots; the red lock. |
| `Speaker(Option<(f32, bool)>)` | mic watcher | Default speaker volume and mute; `None` without an output. Ignored while the volume bar is dragged. |
| `Power(Option<Battery>)` | power watcher | `set_power`: Power pane, plug-in / unplug card. |
| `Ports(Vec<PortInfo>, HashMap<pid, ancestors>)` | port watcher | `set_ports`; ancestors let `task_for_port` tie a server to the task that started it. Cancels an armed **Kill?** if the port is gone. |
| `ShelfAdd(DropJob)`, `ShelfStaged(Vec<ShelfItem>)`, `Screenshot(ShelfItem)`, `Thumb(id, bitmap)` | OLE drop target and shelf workers | Staging, inserting, saving and peeking the Shelf for 5 s; screenshots also get their short stay ([section 10](10-shelf-and-drop-vault.md#screenshots)). |
| `DropDone(DropResult)`, `SetClipboard(String)` | Drop Vault worker | Result chip; clipboard text (only the UI thread owns the clipboard window). |
| `Hog(Alert)` | port watcher tick | Warning card, unless the exe is muted or another card shows. |
| `DownloadDone { name, path }`, `TaskGone(id)` | downloads watcher | "Finished downloading" card with Open / Shelve; `remove_task` for a cancelled download. |
| `Calendar(Option<NextEvent>)` | calendar watcher | `set_calendar`: arms the heads-up. |
| `DragEnter` / `DragLeave` | OLE drop target | `drop_over`: the purple drop zone. |

## `Model`: the state

| Group | Fields |
|---|---|
| Tasks | `tasks: Vec<TaskView>`, `ignore`, `task_first` and `task_rows` (`[general] task_rows`, 0 = 4): `rows()` shows that many, the wheel scrolls (`scroll_tasks`), `rows_hidden()` counts those above and below |
| Media | `media`, `media_stamp` (extrapolates the timeline), `art`, `art_gen` |
| System | `cam`, `mic`, `privacy_app`, `mic_muted`, `speaker`, `awake` (`Off`, `Until(instant)`, `On`), `power`, `charge_since` |
| Servers | `ports`, `port_parents`, `kill_armed: Option<(port, until)>` |
| Shelf | `shelf`, `shelf_sel`, `shelf_off`; `shelf_page()` returns the `SHELF_VISIBLE` (5) in view, clamped so removals leave no gap |
| Navigation | `selected: Option<Panel>`, `peek_until`, `force_until`, `drop_over` |
| Results | `chip`, `chip_armed`, `hog_ignore` (exes muted this session) |
| Calendar / Timer | `calendar`, `calendar_seen`; `timer`, `timer_panel`, `timer_min`, `timer_done`, `timer_last`, `timer_break`, `focus_done`, `rounds` |
| Settings / Look | `settings` (pane open, on a group of `SETTINGS_GROUPS`: Look, Behaviour, Features; hovering away, picking a pane or Back closes it); `art_accent`, `adaptive_glow`, `glow_gain`, `body_alpha` |
| Clock | `now`: last time the model advanced; visibility rules compare against it |

`TaskView` is a task's display form: label, event, progress, stderr lines, pid, `attention` message, `source`, `cwd` (a `run` / `shell` task with a folder is `rerunnable()`), and instants `started`, `changed`, `visible_after`.

## Panels

`Model::panels()` lists panels with content, **in order**: **Shelf** (not empty), **Media** (a session exists), **Home** (always: mic, sound, keep-awake, timer tiles and volume bar), **Tasks** (a visible task or a listening server, shown as **SERVERS**), **Timer** (running), **Power** (has a battery).

- `panel()` is the selected panel if it still exists, else what the collapsed pill shows (Tasks if a task is visible, Media if music plays), else Home. Attention wins: a task that needs input or finishes `peek`s its pane, which selects it.
- `tabs()` is true with more than one panel (the tab strip is drawn only then). `step_panel(dir)` moves with wrap-around (sideways scroll, arrows).

## Choosing the scene

`Model::scene(hover, suppressed)` returns one `Scene`; the **first match** wins:

| # | Condition | Scene |
|---|---|---|
| 1 | Fullscreen suppression on, pointer off the pill, nothing forcing visibility | `Sentinel` |
| 2 | Dragging over the pill (`drop_over`) | `ExpDrop` |
| 3 | A card (chip) exists | `ExpChip` |
| 4 | Timer just finished (`timer_done`) | `ExpTimerDone` |
| 5 | Timer setup card open (`timer_panel`) | `ExpTimer` |
| 6 | Settings pane open (`settings`) | `ExpSettings` |
| 7 | Hovered, or peeking (`now < peek_until`) | the expanded scene for `panel()` (`ExpTasks`, `ExpTimer`, `ExpMedia`, `ExpShelf`, `ExpPower`, `ExpHome`) |
| 8 | Charger plugged / unplugged < 3 s ago (`charge_since`, `charge_in`) | `ExpCharge` |
| 9 | A visible task | `CompactTask` |
| 10 | Media **playing** | `CompactMedia` |
| 11 | otherwise | `Idle` |

**Forced visibility.** `Model::forced()` is true during a drag over the pill, while the timer-finished card or an Allow / Deny chip is up, or until `force_until` (5 s after a drop; `[agent] peek_secs` after `NeedsInput`). These stay visible over fullscreen (thin line or hide mode); game mode still hides everything ([section 7](07-window-and-input.md#fullscreen-suppression)).

**Peeking.** `Model::peek(panel, until)` selects a panel and opens the pill until `until`. Used for agent `NeedsInput` (6 s), items landing on the shelf (5 s), a tracked task finishing or failing (`[general] finish_peek_secs`, 5 s, 0 = off; Tasks opens unless a shell command was too short to show) and a sideways scroll on a collapsed pill (3 s).

## Size and glow

`Model::size(scene)` gives the target size in logical px; springs animate to it.

| Scene | Width × height |
|---|---|
| `Sentinel` | 96 × 2.5 |
| `Idle` | 120 × 24; 150 with privacy dots; 64 + 58 per chip (+34 with dots) for port / shelf chips; +26 with the mic lock |
| `CompactTask` / `CompactMedia` | 292 × 32 / 240 × 32 |
| `ExpTasks` | 380 × (22 + 34 per row shown (≤ `task_rows`, 4), +16 for the "N more · scroll" line, +20 per waiting row, +14 per output line, + stderr box for the first failure, + servers: 26 + 36 per server, ≤ 5), capped at 290 plus 34 per row beyond 4 |
| `ExpMedia` / `ExpHome` / `ExpPower` | 380 × 132 / 136 / 112 (Home +4 + 28 per app row while the per-app list is open; one row says "no apps") |
| `ExpShelf` | 380 × 118 (136 with a tile selected) |
| `ExpTimer` | 380 × 96 running, 152 setup card |
| `ExpDrop` / `ExpChip` / `ExpTimerDone` | 380 × 116 / 96 (80 for a one-line card) / 114 |
| `ExpCharge` / `ExpSettings` | 300 × 52 / 380 × 216 (no tab strip) |

Panels that show the tab strip get `TAB_H` (24 px) more.

`Model::glow(scene)`:

| Situation | Glow |
|---|---|
| Mic muted (Idle / CompactMedia / Home / Media) | red, strong |
| Task waiting for input | amber, full |
| Task failed / done / running / lost | red / green / blue / grey |
| Drop zone | purple, full |
| Shelf, result chip | purple, half |
| Timer panel / timer finished | blue, faint / green |
| Charger plugged in / unplugged | green / amber |
| Power | green plugged in, red ≤ 20 %, else white faint |
| Media, Home, Tasks with only servers | white, faint |
| Idle, Sentinel | none |

## Timed behaviour: `expire`

Instead of periodic checks, `Model::expire(now)` removes everything due and returns the **earliest future deadline** (including tasks still hidden by `visible_after`):

| What | Hold |
|---|---|
| `Done` task / `Lost` task | 3 s (`SUCCESS_HOLD`) / 5 s (`LOST_HOLD`); while the user is away, until they're back, then the hold from their first input |
| Card (chip) | its own `hold`: 10 s (`CHIP_HOLD`), 20 s resource-hog, 60 s calendar |
| Armed **Kill?** (chip or port) | 3 s (`KILL_CONFIRM`) |
| Timer-finished card and alarm | 2 min (`TIMER_DONE_HOLD`) |
| Keep-awake for an hour | `Awake::Until` its end (`AWAKE_FOR`) |
| Plug-in / unplug card | 3 s (`CHARGE_HOLD`) |
| Peek, forced visibility | their own deadline |

"Away" is no input anywhere for 60 s (`AWAY_AFTER`); the window layer feeds `GetLastInputInfo` through `Model::set_idle` before each `expire`. While a finished row is held and the user is away, the deadline is a 5 s poll (`AWAY_POLL`), the only way to notice their return without a hook.

The window layer arms one Win32 timer (`T_EXPIRE`) for exactly that instant and calls `expire` again when it fires, so the UI thread wakes only when something is due.

`Model::finish_timer` handles the countdown reaching zero: it moves to `timer_done` (and `timer_last`, for "25 min focus · ended 14:35") and the window layer picks the next break length. The finished card is forced visible over fullscreen (not game mode) for `TIMER_DONE_HOLD`, and with `[timer] sound` the alarm loops that long: `Ui::after_model_change` starts and stops it (`timer::ring`) whenever `timer_done` appears or goes (**Stop alarm**, a new timer, **+5 min**, or the hold ending).

## Rows, the primary task, and ambient motion

- `rows()` lists visible tasks: waiting, failed, running, then the rest; newest first within a group; truncated to 4. The Tasks panel draws exactly these.
- `primary()` is the first row (computed without building the list: it runs several times per frame); the compact pill shows it. `visible_count()` drives the count badge.
- `ambient(scene, now)` answers "does anything need continuous frames?": true for a running or waiting task, a failed task whose red pulse is fading (8 s, `FAIL_PULSE`), breathing privacy dots in an expanded scene, a drag over the pill, the plug-in card, the timer-finished card (its bell swings while the alarm rings), or playing media in a media scene. Always false for `Sentinel`. See [section 8](08-rendering-and-animation.md).

## `Anim`: animation state

| Field | Animates |
|---|---|
| `rect` (`RectSpring`) | Pill size. Expanding: lively with slight overshoot; collapsing: slower, critically damped. |
| `glow` + `glow_rgb` | Glow strength (colour switches instantly). |
| `content` | Fade-in of the current scene (0 → 1). |
| `out` + `prev` | Fade-out of the scene being left (1 → 0), drawn under the new one. |
| `hover` | Slight brightening under the pointer. |
| `vis` | Whole-window opacity, for hide and pause fades. |
| `t` | Animation seconds for spinners and pulses. |
| `reduce` | Windows "Animation effects" off: springs snap. |

`Anim::set_scene(scene, size, glow)` runs on every layout; on a scene change it moves the old scene to `prev`, starts `out` from the old content's opacity and restarts `content` at 0. `Anim::step(dt)` advances every spring and reports whether anything still moves.

## Cards (chips)

`Chip` is the single card scene (`ExpChip`, 380 × 96, or 80 for a one-line summary) that every "something happened" feature reuses, so a new alert needs no new scene.

| Field | Meaning |
|---|---|
| `summary` | The sentence (≤ two lines). |
| `open`, `copy` | The classic **Open** / **Copy** buttons (Drop Vault results). |
| `extra` | More `(label, ChipAction)` buttons, before **Dismiss**. |
| `hold` | How long it stays (default 10 s). |
| `tone` | Icon: `Done` (purple tick), `Warn` (amber `!`), `Info` (blue `i`). |

`ChipAction` values: `OpenUrl` (only `http` / `https` reach the shell), `Shelve(path)` (normal `ShelfAdd` path), `KillPid(pid, exe)` (first click arms it: label **Kill?**, button red 3 s; the second terminates via `hytte_proto::ports::kill`, which refuses pid 0, 4, itself, and a pid that no longer belongs to `exe`), `IgnoreExe(name)` (adds to `hog_ignore`), `InstallUpdate` / `UpdateLater(tag)` ([section 15](15-updates.md)), and `Answer(id, allow)` (agent ask; clicks within 400 ms of appearing are ignored, `ask_click_ready`).

| Feature | Card | Buttons |
|---|---|---|
| Drop Vault, server **Stop** | Result | Open, Copy, Dismiss |
| Resource hog (`hog.rs`, `[hog]`) | Warn: "`node.exe` has used 92% CPU for 30 s" / "is using 5.2 GB of memory" | Kill (confirmed), Ignore, Dismiss |
| Downloads (`downloads.rs`) | "`file.zip` finished downloading" | Open, Shelve, Dismiss |
| Calendar (`calendar.rs`) | Info: "Design review starts in 9 min" | Join (with a link), Dismiss |

A new card never replaces one that is showing: an alert arriving meanwhile is dropped (its watcher has already counted it as reported).

## Bubbles

`Model::bubble(scene)` decides whether a second activity splits off beside a **compact** scene as a small pill:

| Beside | Bubble | Shows |
|---|---|---|
| `CompactTask` or `CompactMedia`, timer running | `Bubble::Timer` | Remaining time as a clock (red under a minute) |
| `CompactTask`, music playing, no timer | `Bubble::Media` | The album art |

Expanded scenes show these as panels, so no bubble. The bubble is part of the hover area (`pill_contains`, `bubble_hit`; position from `Bubble::x`, shared with the renderer): hovering it opens the pill on **its** pane (`Bubble::panel`). While the pane is up both the compact pill's and the bubble's spots are kept (`BubbleZone`); otherwise leaving the bubble's spot would close the pane, bring the bubble back under the pointer, and reopen it. Gliding along the top strip between bubble and pill switches panes (`follow_spot`; entering from below changes nothing, so the pane's buttons are safe). The previous panel is restored when the pane closes. Layout passes the answer to `Anim::set_bubble`, which springs `bubble` (0.72 / 0.34 s, a little overshoot) to 1 or 0; `bubble_kind` remembers the last kind so a retracting bubble still draws; `bubble_room()` is the extra width (`BUBBLE_ROOM`, 72 px) the crop and window region need while any is out.

## Paging, squash and other motion

- `Anim::nav` is the paging direction (+1 / -1), set by sideways scroll, tab strip and keyboard just before layout. `set_scene` moves it to `slide`, and the renderer slides new content in from that side (old out the other) instead of the vertical drift; a change nobody paged for (a peek) keeps the drift. `after_model_change` clears `nav`.
- `Anim::squash` kicks the size springs (width +160, height −110 px/s) when items land on the shelf or a Drop Vault result arrives; nothing under reduced motion.
- `glow_gain` is `[general] glow_strength`, applied to the border glow in `render.rs`; `TaskView::line` / `shows_line` carry a running task's output ticker, and `LINE_H` (14 px) is the row height it adds in `size(ExpTasks)`.

## Per-app volume

The ▾ on the Home volume bar (`Action::ToggleApps`) sets `apps_open` and fills `Model::apps` from `mic::apps()`: the default speaker's audio sessions (`IAudioSessionManager2`), without system sounds, expired sessions, or apps that neither play nor have a visible window (audio consoles, wallpaper engines holding a session; `proc::has_window` ignores a suspended app's cloaked window). One row per exe (Chrome's sessions merge), playing first, at most `APPS_MAX` (4), named by the exe's file description (`proc::file_description`, cached per path) or the exe name. The pane grows by `apps_h()`. The list is re-read when the pane opens with it open, not while shown, so a drag isn't fighting a refresh. An app's bar (`Action::AppVolume(exe)`) drags like the master (`scrub`), scrolls 2 % a notch, toggles silent/full from the keyboard; `mic::set_app_volume` sets every session of that exe and unmutes when turned up.

## Tests

`ui_state.rs` has the project's largest test module (scene priority, expiry, thresholds, ignored commands, peeking, panels, fullscreen break-through, timer, mic lock, ambient rules, cross-fade, plug-in card, output line, bubbles, paging, squash, album-art colour). Change or add a test beside any rule you change. They need no Windows APIs.

Next: [7. Window, input and timers](07-window-and-input.md)
