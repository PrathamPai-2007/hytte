# 6. The UI model

`crates/hytte/src/ui_state.rs` is the brain of the pill. It decides *what* to show, as plain Rust with no Win32 calls, so almost all of its behaviour is unit-tested. The window layer ([section 7](07-window-and-input.md)) feeds it events and clock ticks, and the renderer ([section 8](08-rendering-and-animation.md)) reads it to draw.

It has three parts:

- **`UiEvent`**: the messages workers send to the UI thread.
- **`Model`**: everything the pill knows about the world.
- **`Anim`**: the animation state that turns "what to show" into smooth motion.

## `UiEvent`

| Variant | Sent by | Effect |
|---|---|---|
| `Task(TaskUpdate)` | task registry (via the bridge) | `Model::apply_task` ([section 4](04-ipc-protocol.md)) |
| `Media(Option<MediaInfo>)` | media watcher | Sets or clears `media`. Clearing also clears the album art. |
| `MediaArt(Option<Arc<ArtBitmap>>)` | media watcher | Sets `art` and bumps `art_gen` so the renderer re-uploads it. |
| `Privacy(cam, mic, app)` | privacy watcher | The camera / mic dots and the app name. |
| `MicMute(bool)` | mic watcher | The red lock. |
| `Power(Option<Battery>)` | power watcher | `Model::set_power`: the Home card battery row, and the plug-in card when the charger was just connected. |
| `Ports(Vec<PortInfo>)` | port watcher | `set_ports`. Also cancels an armed **Kill?** if that port is gone. |
| `ShelfAdd(DropJob)` | OLE drop target | Starts staging on a worker ([section 10](10-shelf-and-drop-vault.md)). |
| `ShelfStaged(Vec<ShelfItem>)` | shelf staging worker | Inserts the items, saves the shelf, and peeks the Shelf panel for 5 s. |
| `Thumb(id, bitmap)` | thumbnail loader | Uploads a shelf thumbnail to the renderer. |
| `DropDone(DropResult)` | Drop Vault worker | Shows the result chip. |
| `SetClipboard(String)` | Drop Vault worker | Puts text on the clipboard (only the UI thread owns the clipboard window). |
| `DragEnter` / `DragLeave` | OLE drop target | `drop_over`: the purple drop zone. |

## `Model`: the state

The important fields, grouped:

| Group | Fields |
|---|---|
| Tasks | `tasks: Vec<TaskView>`, `ignore` (lower-cased shell commands to drop) |
| Media | `media`, `media_stamp` (when the info arrived, used to extrapolate the timeline), `art`, `art_gen` |
| System | `cam`, `mic`, `privacy_app`, `mic_muted`, `power`, `charge_since` |
| Ports | `ports`, `kill_armed: Option<(port, until)>` |
| Shelf | `shelf: Vec<ShelfItem>`, `shelf_sel` (the selected tile) |
| Navigation | `selected: Option<Panel>`, `peek_until`, `force_until`, `drop_over` |
| Results | `chip: Option<Chip>` |
| Timer | `timer`, `timer_panel` (the right-click panel is open), `timer_min`, `timer_done`, `timer_break`, `focus_done` |
| Clock | `now`: the last time the model was advanced; visibility rules compare against it |

`TaskView` is the display form of a task: the label, event, progress, stderr lines, pid, `attention` message, and three instants (`started`, `changed`, `visible_after`).

## Panels

When expanded, the pill shows one **panel**. `Model::panels()` lists the panels that currently have content, **in this order**:

1. **Tasks**: at least one visible task
2. **Timer**: a timer is running
3. **Shelf**: the shelf isn't empty
4. **Media**: a media session exists
5. **Ports**: something is listening
6. **Home**: always present (mic mute, battery, power mode)

- `Model::panel()` is the selected panel if it still exists, otherwise the first one in that list. So attention-worthy panels win by default.
- `Model::tabs()` is true when there is more than one panel; the tab dots are drawn only then.
- `Model::step_panel(dir)` moves the selection with wrap-around. It is used by the mouse wheel.

## Choosing the scene

`Model::scene(hover, suppressed)` returns exactly one `Scene`. The rules are checked **in order**, and the first match wins:

| # | Condition | Scene |
|---|---|---|
| 1 | Fullscreen suppression is on, the pointer isn't on the pill, and nothing is forcing it visible | `Sentinel` |
| 2 | Something is being dragged over the pill (`drop_over`) | `ExpDrop` |
| 3 | A result chip exists | `ExpChip` |
| 4 | A timer just finished (`timer_done`) | `ExpTimerDone` |
| 5 | The right-click timer panel is open | `ExpTimer` |
| 6 | Hovered, or peeking (`now < peek_until`) | the expanded scene for `panel()`: `ExpTasks`, `ExpShelf`, `ExpMedia`, `ExpPorts`, `ExpTimer` or `ExpHome` |
| 7 | The charger was plugged in less than 3 s ago (`charge_since`) | `ExpCharge` |
| 8 | At least one visible task | `CompactTask` |
| 9 | Media is **playing** | `CompactMedia` |
| 10 | otherwise | `Idle` |

**Forced visibility.** `Model::forced()` is true while something is dragged over the pill, or until `force_until`. Hytte sets `force_until` for 5 s after a drop lands and for 8 s after a timer finishes, so these stay visible even over a fullscreen app.

**Peeking.** `Model::peek(panel, until)` selects a panel and opens the pill until the given instant. It is used when an agent needs input (`[agent] peek_secs`, 6 s), when items land on the shelf (5 s), when a tracked task finishes or fails (`[general] finish_peek_secs`, 5 s, `0` turns it off; the Tasks panel opens unless a shell command was too short to be shown), and when the wheel scrolls a collapsed pill (3 s).

## Size and glow

`Model::size(scene)` returns the target pill size in logical pixels. The springs animate towards it.

| Scene | Width × height |
|---|---|
| `Sentinel` | 96 × 2.5 |
| `Idle` | 120 × 24; 150 with privacy dots; 64 + 58 per chip (+34 with privacy dots) with port / shelf chips; +26 when the mic lock shows |
| `CompactTask` | 292 × 32 |
| `CompactMedia` | 240 × 32 |
| `ExpTasks` | 380 × (22 + 34 per row, +20 per waiting row, + stderr box for the first failure), capped at 290 |
| `ExpPorts` | 380 × (20 + 34 per port, at most 5) |
| `ExpMedia` / `ExpHome` / `ExpShelf` / `ExpTimer` | 380 × 128 / 132 (86 without a battery) / 118 / 92 |
| `ExpDrop` / `ExpChip` / `ExpTimerDone` | 380 × 116 / 96 / 96 |
| `ExpCharge` | 300 × 52 |

Expanded panels that show tab dots get 14 px more height.

`Model::glow(scene)` returns a colour and strength for the soft halo:

| Situation | Glow |
|---|---|
| Mic muted, in Idle / CompactMedia / Home / Media / Ports | red, strong |
| Task waiting for input | amber, full |
| Task failed / done / running / lost | red / green / blue / grey |
| Drop zone | purple, full |
| Shelf, result chip | purple, half |
| Timer panel / timer finished | blue, faint / green |
| Charger plugged in | green |
| Media, Home, Ports | white, faint |
| Idle, Sentinel | none |

## Timed behaviour: `expire`

Many things disappear on their own: finished tasks, chips, peeks, an armed **Kill?**, forced visibility, the timer-finished card. Instead of checking them periodically, `Model::expire(now)`:

1. removes everything whose time has passed:

   | What | Hold |
   |---|---|
   | `Done` task | 3 s (`SUCCESS_HOLD`) |
   | `Lost` task | 5 s (`LOST_HOLD`) |
   | Chip | 10 s (`CHIP_HOLD`) |
   | Armed **Kill?** | 3 s (`KILL_CONFIRM`) |
   | Timer-finished card | 30 s (`TIMER_DONE_HOLD`) |
   | Plug-in card | 3 s (`CHARGE_HOLD`) |
   | Peek, forced visibility | their own deadline |

2. returns the **earliest future deadline**, which also includes tasks that are still hidden (`visible_after`).

The window layer arms a single Win32 timer (`T_EXPIRE`) for exactly that instant, then calls `expire` again when it fires. So the model wakes the UI thread only when something is actually due.

`Model::finish_timer` does the same for the countdown timer: when it reaches zero, the timer moves to `timer_done` and the window layer plays the chime, works out the next break length, and forces the pill visible.

## Rows, the primary task, and ambient motion

- `Model::rows()` lists the visible tasks in this order: waiting for input, failed, running, everything else. Newest comes first within each group, and the list is truncated to 4. The expanded Tasks panel draws exactly these rows.
- `Model::primary()` is the first of those rows, chosen without building the list (it runs several times per frame). The compact pill shows it.
- `Model::visible_count()` drives the count badge on the compact pill.
- `Model::ambient(scene, now)` answers "does anything need continuous frames right now?". It returns true for a running or waiting task, a failed task whose red pulse is still fading (8 s, `FAIL_PULSE`), breathing privacy dots in an expanded scene, a drag over the pill, the plug-in card, or playing media in a media scene. It is always false for the `Sentinel`, which never moves. [Section 8](08-rendering-and-animation.md) explains why this matters.

## `Anim`: animation state

`Anim` holds springs (see `animation.rs`) and the current scene:

| Field | Animates |
|---|---|
| `rect` (`RectSpring`) | Pill width and height. Expanding uses a lively spring with a slight overshoot; collapsing uses a slower, critically damped one. |
| `glow` + `glow_rgb` | Glow strength (the colour switches instantly). |
| `content` | Fade-in of the current scene's content (0 → 1). |
| `out` + `prev` | Fade-out of the scene being left (1 → 0), drawn under the new one. |
| `hover` | The slight brightening while the pointer is over the pill. |
| `vis` | Whole-window opacity, for fading out when hidden or paused. |
| `t` | Seconds of animation time, used by spinners and pulses. |
| `reduce` | Windows "Animation effects" is off: springs snap instead of moving. |

`Anim::set_scene(scene, size, glow)` is called on every layout. When the scene changes, it moves the old scene to `prev`, starts `out` from the old content's current opacity, and restarts `content` at 0. `Anim::step(dt)` advances every spring and returns whether anything is still moving.

## Tests

`ui_state.rs` has the largest test module in the project: scene priority, expiry, shell thresholds, ignored commands, attention peeking, panels and the wheel, fullscreen break-through on drops, the timer, the mic lock, ambient rules, and the cross-fade. When you change a rule above, change or add a test next to it. These tests need no Windows APIs.

Next: [7. Window, input and timers](07-window-and-input.md)
