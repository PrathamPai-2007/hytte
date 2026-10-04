# Hytte

A dynamic notch and ambient cockpit for **Windows 11**, written in Rust.

Hytte hangs a small pill from the top-centre of your screen. It stays out of the way until something happens — a build finishes, an AI agent needs you, a dev server starts, music plays, you drag a file near the top edge — then it springs open, shows what matters, and folds away again. It never steals focus, draws no frames while idle, and has no Electron, no web view, and no async runtime.

<p align="center">
  <img src="docs/img/task.png" alt="Compact pill showing two running tasks" width="420"><br>
  <img src="docs/img/tasks-open.png" alt="Expanded task list with progress filaments" width="420">
</p>

## Table of contents

- [Features](#features)
- [Requirements](#requirements)
- [Install](#install)
  - [Build from source](#build-from-source)
  - [Run at login](#run-at-login)
- [Quick start](#quick-start)
- [Using Hytte](#using-hytte)
  - [The pill at a glance](#the-pill-at-a-glance)
  - [Panels and tabs](#panels-and-tabs)
  - [Hover and click behaviour](#hover-and-click-behaviour)
  - [Tray menu](#tray-menu)
- [Process Sentinel (`notch`)](#process-sentinel-notch)
  - [`notch run`](#notch-run)
  - [`notch set`](#notch-set)
- [Automatic shell tracking](#automatic-shell-tracking)
- [AI agent monitor](#ai-agent-monitor)
- [Dev server and port watcher](#dev-server-and-port-watcher)
- [Staging shelf and Drop Vault](#staging-shelf-and-drop-vault)
- [Media cockpit](#media-cockpit)
- [Privacy indicators](#privacy-indicators)
- [Fullscreen and game behaviour](#fullscreen-and-game-behaviour)
- [Configuration](#configuration)
- [`notch` command reference](#notch-command-reference)
- [How it works](#how-it-works)
  - [Workspace layout](#workspace-layout)
  - [Rendering and animation](#rendering-and-animation)
  - [Threading model](#threading-model)
  - [IPC protocol](#ipc-protocol)
  - [Performance budgets](#performance-budgets)
- [Development](#development)
- [Troubleshooting](#troubleshooting)
- [Known limitations](#known-limitations)
- [License](#license)

## Features

| Feature | What it does |
|---|---|
| **Process Sentinel** | Wrap any command with `notch run -- <cmd>`: live elapsed time, a shimmering progress filament, a green check on success, a red pulse plus the last stderr lines on failure. Scripts can report real progress with `notch set`. |
| **Automatic shell tracking** | One line in your shell profile (PowerShell, bash/Git Bash, zsh, Nushell) and any command that runs longer than 3 seconds appears in the pill automatically. No wrapper to remember. |
| **AI agent monitor** | Unattended agents (Claude Code, Aider, eval scripts) report in; when one needs a human the pill turns amber, pulses, and one click brings that terminal to the front. |
| **Port watcher** | Shows dev servers listening on `localhost` (3000, 5173, 8080, 5432, …) with one-click **Open** in the browser and a two-step **Kill** for hung processes. `notch kill :3000` does the same from a shell. |
| **Timer** | Right-click the pill, scroll to set minutes, press **Timer** or **Focus** (Pomodoro with breaks). A glowing fuse burns down along the pill edge and replaces the idle dash; hover the pill for Pause / +5 min / Stop. A soft chime plays when it ends. Settings in `[timer]`. |
| **Staging shelf** | Drag files or text onto the notch to park them. Switch folders, desktops or apps, then drag them back out of the pill into any destination. The shelf follows you across virtual desktops and survives restarts. |
| **Drop Vault** | Select a shelved file to get one-click actions: **Compress** an image to under 5 MB (Discord/e-mail limits), **To PDF**, **Remove metadata** (EXIF/GPS), **Read text** (OCR to clipboard), JSON/YAML formatting, copy path. Results land next to the original; nothing is overwritten. |
| **Media cockpit** | Title, artist, album art, live timeline, animated equaliser and prev / play-pause / next for the current Windows media session. |
| **Mic mute** | One global toggle (the Home card's **Mute** button, or click the lock) mutes every microphone; while muted a bold red lock and red glow sit on the pill, whatever it is showing. |
| **Battery cockpit** | On laptops: charge %, live charge / discharge power in watts, and a Saver / Balanced / Performance power-mode switch. |
| **Privacy indicators** | A green dot while any app uses your camera, an orange dot for the microphone, with the app name when expanded. |
| **Fullscreen aware** | Backs off to a hairline (or hides) when a game or video is fullscreen, with per-process allow/deny lists. |
| **Fluid animation** | Spring-driven resize, content cross-fades, a morphing silhouette, state-coloured glow, and a frame clock that parks completely at idle. Honours Windows' "Animation effects" setting. |

## Requirements

- Windows 11 22H2 or newer (x64). There are no Windows 10 fallbacks.
- The Rust stable toolchain with the MSVC target, only if you build from source (`rust-toolchain.toml` pins it).
- Optional: a Windows OCR language pack for image text recognition (*Settings → Time & language → Language & region*). Without one everything else still works and the result says *OCR unavailable*.

## Install

### Build from source

```powershell
git clone <this repo>
cd Hytte
cargo build --release --workspace
```

This produces two binaries in `target\release\`:

| Binary | Role |
|---|---|
| `hytte.exe` | The daemon that owns the notch window. Run it once; it lives in the tray. |
| `notch.exe` | A tiny CLI client used from terminals, scripts and agent hooks. Built from the same package; `notch run` starts the daemon if it is not running. |

Put both on your `PATH` (or copy them to a folder that is) so `notch` works from any shell.

> A winget manifest template lives in [`manifests/winget`](manifests/winget). It still points at a placeholder URL; fill in the real release asset before submitting it to winget-pkgs.

### Run at login

Tick **Launch at startup** in the tray menu, or set `autostart = true` in the [config file](#configuration). Hytte writes (or removes) a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Nothing is installed system-wide.

## Quick start

```powershell
# 1. start the daemon (only one instance can run)
hytte

# 2. wrap something slow
notch run -- cargo build --release

# 3. or track every long command automatically (add to $PROFILE)
notch init pwsh | Out-String | Invoke-Expression

# 4. start a dev server and watch it appear next to the pill
npm run dev
```

## Using Hytte

### The pill at a glance

| State | You see | When |
|---|---|---|
| **Idle** | A small near-black capsule with a faint bar, or small chips: a green dot and `:3000` for listening ports, a stacked-tile glyph and a count for shelved items | Nothing is happening |
| **Task** | Spinner / check / cross, label, elapsed time or `%`, progress filament, a count badge if several tasks are visible | One or more tasks exist |
| **Needs input** | Pulsing amber glow, ringing bell, the agent's name and message | An agent is waiting for you |
| **Media** | Cover thumbnail, `Title — Artist`, animated equaliser | Audio is playing and no task is visible |
| **Drop zone** | Dashed purple drop area reading "Drop to add to your shelf" | You are dragging a file or text over the notch |
| **Result chip** | What was done, plus **Open / Copy / Dismiss** | A transform or kill finished |
| **Sentinel** | A 3 px grey line | Fullscreen is active and the mode is `sentinel` |

| Idle | Home card | Muted | Muted, expanded |
|---|---|---|---|
| <img src="docs/img/idle.png" width="200"> | <img src="docs/img/home.png" width="200"> | <img src="docs/img/muted.png" width="200"> | <img src="docs/img/muted-open.png" width="200"> |

A coloured glow reflects the state: blue running, green success, red failure, amber waiting for you, purple shelf and drops.

### Panels and tabs

When expanded, the pill shows one **panel** at a time: **Tasks**, **Shelf**, **Media**, **Ports**, and the always-present **Home** card (microphone mute, plus battery and power mode on laptops). If more than one panel exists, small tab dots appear along the bottom edge; click a dot or **scroll the mouse wheel over the pill** to switch (it wraps around). Panels with attention-worthy content open first, and some events make the pill briefly **peek** open on their own: an agent asking for input (6 s) or a new item landing on the shelf (5 s).

### Hover and click behaviour

- Rest the pointer on the pill for about **120 ms** and it expands. Leave and it collapses after **300 ms**; re-entering in that window cancels the collapse, so grazing the edge doesn't flicker.
- **Click a task row** to bring the terminal that owns it to the front (works for `notch run`, shell-tracked commands and agents). In **Windows Terminal** it also switches to the tab the task was started in. Finished rows have a small ✕ to dismiss.
- A failed task shows the last stderr lines with **Copy error** and **Dismiss**.
- Media panel: click ⏮ ⏯ ⏭, or click the cover art / title to bring the playing app to the front.
- Home card: **Mute / Unmute**, and the three power modes (laptops).
- Hovering a button highlights it and the cursor becomes a hand.
- The window never takes focus when clicked.

### Tray menu

Right-click the Hytte tray icon:

| Item | Effect |
|---|---|
| **Pause** | Fades the notch out and keeps it hidden until you resume. Tasks are still tracked. |
| **Launch at startup** | Toggles the autostart entry and saves it to the config. |
| **Open settings folder** | Opens `%APPDATA%\Hytte` in Explorer. |
| **Quit** | Exits cleanly and removes the tray icon. |

## Process Sentinel (`notch`)

`notch.exe` talks to the daemon over the named pipe `\\.\pipe\hytte`. It is deliberately tiny so it starts instantly in a shell. **If the daemon isn't running, `notch run` still runs your command** — the notch is best-effort and never gets in the way.

### `notch run`

```text
notch run -- <command> [args...]
```

- Runs the command with stdout and stdin passed straight through, so the terminal behaves normally. Stderr is forwarded line by line while a ring buffer keeps the last 5 lines.
- Reports `Start`, then `Done` or `Failed` with the duration.
- Exits with the **same exit code** as the wrapped command (`127` if it couldn't be spawned), so it is safe in scripts.
- The label is the command line, truncated to 128 characters. Progress is indeterminate (a shimmer): an arbitrary command can't report a percentage.
- The task is tied to the `notch` process, so clicking its row focuses the terminal it runs in.

### `notch set`

```text
notch set --progress <0-100> --label "text" [--task <id>]
```

For scripts that *do* know how far along they are. Messages with the same `--task` id update one entry.

```powershell
foreach ($i in 0..10) {
    notch set --progress ($i * 10) --label "Exporting" --task export-1
    Start-Sleep 1
}
```

Tasks with a known owner process are marked **lost** (`?`) only when that process dies; tasks without one are marked lost after 30 s of silence.

## Automatic shell tracking

Instead of typing `notch run --` in front of everything, install the shell integration once. Every command is reported when it starts and when the prompt returns, but the **daemon only shows commands that outlive the threshold** (default 3 s), so quick commands never flash up. A command that finishes early, or is cancelled with Ctrl-C, is dropped silently.

| Shell | Add this |
|---|---|
| **PowerShell** | `notch init pwsh \| Out-String \| Invoke-Expression` in `$PROFILE` |
| **bash / Git Bash** | `eval "$(notch init bash)"` in `~/.bashrc` |
| **zsh** | `eval "$(notch init zsh)"` in `~/.zshrc` |
| **Nushell** | `notch init nu \| save -f ~/.config/nushell/notch.nu`, then `source` it in `config.nu` |
| **Starship** | Starship only draws the prompt; use the line for your underlying shell and put it *before* `starship init` |

Details:

- The PowerShell integration wraps `prompt` and binds Enter through PSReadLine (replacing any custom Enter binding). It writes to the pipe directly, so there is no process spawn per command.
- bash and zsh call `notch hook start|end` in the background. Under Git Bash the Windows process id is used so row clicks focus the right terminal.
- Commands in `[shell] ignore` (editors, pagers, `ssh`, …) are never shown. Shell tasks show `exit N` when they fail, since stderr isn't captured.
- The integrations never print anything and never fail the prompt if the daemon is down.

## AI agent monitor

Long-running agents often stop and wait for a human. Tell the notch about it and the pill turns amber with a pulsing glow, rings a small bell, peeks open for a few seconds, and stays amber until you deal with it. **Click the row to jump to the terminal.**

```text
notch agent start       [--name N] [--pid P]
notch agent needs-input [--name N] [--pid P] [--message M]
notch agent resume      [--name N] [--pid P]
notch agent done        [--name N] [--pid P]
notch agent fail        [--name N] [--pid P]
```

- `--name` defaults to `Agent`. `--pid` defaults to the nearest ancestor process that is not a throw-away shell (`cmd`, `bash`, `pwsh`, …), which is the long-lived agent or terminal. It is used for click-to-focus and to detect an agent that died.
- `needs-input` reads a hook JSON payload from stdin when no `--message` is given and uses its `message` field.
- Messages with the same name and pid update a single entry.

**Claude Code:** print a ready-to-paste hooks block and merge it into your `settings.json`:

```powershell
notch agent hooks claude
```

It maps `UserPromptSubmit` → `resume`, `Notification` → `needs-input` (with Claude's own message), and `Stop` → `done`.

**Aider and others:** point the tool's notification command at `notch agent needs-input --name Aider --message "waiting"`, or call `notch agent …` from any script. Set `[agent] sound = true` for a chime on attention events.

## Dev server and port watcher

The Ports panel lists processes listening on loopback or all interfaces, for the ports in `[ports] watch` (or every non-system listener with `show_all = true`). Windows shell/service hosts and system processes are always excluded.

- **Open** launches `http://localhost:<port>` in your default browser.
- **Kill** is two-step: the first click turns it into **Kill?** for 3 seconds, the second terminates that process. Only processes you own can be stopped; system pids and Hytte itself are refused. A chip reports the result.
- When something is listening, the collapsed pill shows a small chip like `● :3000` (or `:3000 +2`).

From a terminal:

```powershell
notch ports            # list watched dev listeners (add --all for every listener)
notch kill :3000       # asks "Stop node.exe (pid 1234) listening on :3000? [y/N]"
notch kill :3000 --force
```

Windows has no cheap "a socket started listening" event, so the daemon re-scans every `poll_secs` (default 5 s, one cheap syscall) and updates the UI only when the set changes; opening the Ports tab refreshes immediately.

## Staging shelf and Drop Vault

Drag a file, folder or selected text toward the top of the screen with the left button held. A 40 px invisible landing strip along the top edge arms while you drag, so you don't have to aim precisely. Release over the notch and the item lands on the **shelf**.

- Each item is a tile with a thumbnail and name. Up to 12 are kept (`max_items`), five are shown at a time with a `+N` overflow.
- **Drag a tile out** of the pill onto any destination — Explorer, an email draft, Slack, an editor. It is a real file drag (shell data object), so anything that accepts files accepts it. By default the tile is removed afterwards (`remove_after_drag`).
- **Switch desktops freely.** Hytte moves its window to whichever virtual desktop you are on, so the shelf is always at the top of your screen.
- The shelf **persists** across restarts (`shelf.json` in `%APPDATA%\Hytte`); entries whose file has disappeared are pruned.
- In `mode = "reference"` (default) a tile points at the original file and removing it never touches the file. In `mode = "copy"` Hytte stores its own copy under `%APPDATA%\Hytte\shelf` and deletes that copy when you remove the tile. Dropped text is stored as a `.txt` snippet so it can be dragged out like a file.
- Dragging a tile out as a **move** moves the real file (that is what the destination does); the tile disappears and the shelf prunes the missing path.

**Drop Vault actions:** click a tile to select it and use the action button under the strip. Results land next to the source (or in `output_folder`) and **never overwrite**; existing names get `-1`, `-2`, … appended.

| Tile | Action | Result |
|---|---|---|
| **PNG / JPEG** | Compress · To PDF · Remove metadata · Read text | `name-5mb.jpg` (JPEG quality search, then downscale; skipped if already under 5 MB), `name.pdf` (one image per page), `name.clean.png|jpg` (re-encoded, so EXIF and GPS are gone), OCR text copied to the clipboard |
| **WebP / BMP** | Compress · To PDF · Read text | As above, minus Remove metadata |
| **JSON** | Format | `name.pretty.json` if it was compact, `name.min.json` if already formatted; text also copied |
| **YAML** | Format | `name.pretty.yaml` with trailing whitespace trimmed, tabs expanded, blank-line runs collapsed |
| **Text snippet** (.txt / .md) | Copy text | Contents copied; JSON is formatted on the way |
| **Anything else** | Copy path | Path copied, Explorer opens with the file selected |

The result appears as a chip offering **Open / Copy / Dismiss**. Dropped files are never executed.

## Media cockpit

Hytte reads Windows' System Media Transport Controls, the same source as the volume flyout, and updates on events rather than polling. Cover art is decoded once per track and cached at 64×64. While expanded, the timeline advances smoothly between updates, and the equaliser animates only while something is actually playing. Transport buttons control the current session.

## Microphone mute

The Home card's **Mute** button toggles every active capture device through the Windows audio endpoint API. While muted, a red lock sits at the right edge of the pill in every state, the glow turns red and the idle pill widens to fit it; click the lock to unmute. State changed elsewhere (a laptop mute key, Settings) is picked up within 2 s.

## Battery and power mode

On machines with a battery, the Home card shows the charge, whether it is charging or discharging and at how many watts (read from the battery driver; shown as *plugged in* when the driver reports no rate), and a **Saver / Balanced / Performance** switch that sets the Windows power mode. Desktops without a battery don't see this row.

## Privacy indicators

A **green** dot means the camera is in use, an **orange** dot the microphone. Hytte watches the Windows `CapabilityAccessManager\ConsentStore` registry keys with change notifications, covering both Store apps and classic desktop apps (Zoom, OBS, browsers). The Home card names the app using the device. Dots stay still while the pill is collapsed (an open call costs no CPU) and gently breathe when expanded.

## Fullscreen and game behaviour

Hytte re-checks whenever the foreground window changes or moves, using the shell's *user notification state* (D3D fullscreen, presentation mode, busy) with a "window covers the whole monitor" fallback.

| `fullscreen_mode` | Behaviour while fullscreen |
|---|---|
| `"sentinel"` (default) | Shrinks to a 3 px grey line at the top edge. Hover it and the notch reveals itself. |
| `"hide"` | Fades out entirely. |

`allow_list` (never suppress) and `deny_list` (always hide) take executable file names such as `"Code.exe"`. Set `suppress_fullscreen = false` to turn the feature off. Dragging a file toward the top edge still works while fullscreen is active: the pill steps out of the way of nothing, shows the drop zone, and peeks the shelf for 5 s after the drop (even in `hide` mode). Hytte is a plain topmost window with no injection or hooks into other processes, so it cannot draw over exclusive-fullscreen games; suppression just keeps it from interfering.

## Configuration

Settings live in one TOML file, created on first run:

```text
%APPDATA%\Hytte\config.toml
```

```toml
[general]
autostart = false              # start with Windows
monitor = "primary"            # only the primary monitor is supported today
solid_pill = true              # reserved; the pill is solid by default
acrylic = false                # translucent pill instead of solid near-black
suppress_fullscreen = true     # back off during fullscreen apps and games
fullscreen_mode = "sentinel"   # "sentinel" | "hide"
output_folder = "D:\\Hytte"    # optional; default is next to the source file
allow_list = []                # e.g. ["Code.exe"]: never suppress for these
deny_list = []                 # e.g. ["game.exe"]: always hide for these

[shell]
threshold_ms = 3000            # shell commands shorter than this never show
ignore = ["vim", "nvim", "ssh", "less", "man", "top", "htop", "tmux", "fzf"]

[shelf]
mode = "reference"             # "reference" (point at the original) | "copy" (own copy)
max_items = 12
persist = true                 # remember the shelf across restarts
remove_after_drag = true       # drop the tile once it has been dragged out

[ports]
watch = [3000, 3001, 4200, 5000, 5173, 5432, 8000, 8080, 8888]
show_all = false               # true: every non-system listener
poll_secs = 5

[agent]
peek_secs = 6                  # how long the pill stays open for an attention event
sound = false                  # beep on attention events
```

Every section and key is optional. Restart `hytte` (tray → Quit, then launch again) after editing. Invalid files fall back to defaults. Logs and panic traces go to `%APPDATA%\Hytte\hytte.log`.

## `notch` command reference

| Command | Purpose |
|---|---|
| `notch run -- <cmd…>` | Run a command and track it in the pill |
| `notch set --progress N --label L [--task ID]` | Report scripted progress |
| `notch agent start\|needs-input\|resume\|done\|fail …` | Report an AI agent's state |
| `notch agent hooks claude` | Print Claude Code hooks config |
| `notch init pwsh\|bash\|zsh\|nu` | Print the shell integration script |
| `notch hook start\|end …` | Low-level shell-integration events (used by the scripts) |
| `notch ports [--all]` | List dev listeners |
| `notch kill :PORT [--force]` | Stop the process listening on a port |

## How it works

### Workspace layout

```text
crates/
  proto/    shared IPC message type + validation, port discovery/kill, process-tree helpers
  notch/    the CLI client
    shell/               init.ps1 / init.bash / init.zsh / init.nu (embedded into the binary)
    src/agent.rs         notch agent …
    src/hook.rs          notch hook … and notch init …
    src/portscmd.rs      notch ports / kill
  hytte/    the daemon
    src/main.rs          wiring: starts workers, then the UI loop
    src/window.rs        Win32 window, input, timers, OLE drop target and drag source, hooks
    src/render.rs        Direct2D/DirectWrite drawing of every scene
    src/ui_state.rs      pure UI model: tasks, panels, scene selection, animation state
    src/animation.rs     spring integrator
    src/pipe_server.rs   ACL'd named-pipe server
    src/tasks.rs         task registry (stale / dead-owner detection)
    src/proc.rs          process and window helpers (exe names, liveness, focus a terminal)
    src/ports.rs         port-watcher worker
    src/shelf.rs         shelf items, persistence, shell thumbnails
    src/media.rs         media session events + album art
    src/privacy.rs       camera/mic registry watcher
    src/mic.rs           global microphone mute
    src/power.rs         battery, charge rate and power mode
    src/drop.rs          Drop Vault job queue + worker pool
    src/transforms.rs    shelf chips: image / JSON / YAML / OCR transforms
    src/convert.rs       Compress (under 5 MB) and hand-written PDF export
    src/timer.rs         Timer and Focus (Pomodoro) state
    src/fullscreen.rs    suppression decision
    src/tray.rs, config.rs, logging.rs, single_instance.rs, winrt.rs
manifests/winget/        packaging template
```

### Rendering and animation

- **One layered, topmost, no-activate window**, centred on the primary monitor's top edge. Rendering is Direct2D onto a premultiplied-alpha bitmap, then `UpdateLayeredWindow`: anti-aliased rounded shapes, a notch silhouette with concave "ears" flaring into the screen edge, gradients and glow. Fully transparent pixels are click-through, so Hytte never swallows clicks meant for other windows.
- Only the **bounding box of the pill** is drawn and presented each frame, not the whole canvas.
- Layout is in logical pixels and scaled for **per-monitor DPI**.
- Size, corner radius, glow, hover brightness, content fade and visibility are **critically-damped springs** integrated with real frame deltas in small sub-steps. Scene changes cross-fade and slide the content in.
- **No idle loop.** A ticker thread parks when nothing moves. During motion it is paced by `DwmFlush` (vsync); for ambient effects (spinner, equaliser, the amber pulse, a fresh failure's red pulse, which settles after a few seconds) it ticks at about 30 fps. The fullscreen sentinel bar never animates. With *Animation effects* off in Windows, springs snap and ambient motion stops.

### Threading model

Plain `std::thread` plus channels; there is no async runtime. The UI thread owns the window and all UI state. Worker threads (pipe server, task registry, media, privacy, ports, drop transforms, thumbnail loaders) push `UiEvent`s into a queue and post one wake-up message; the UI thread drains it. WinRT async calls are driven by a tiny blocking helper with timeouts. Dragging a tile out runs OLE's modal drag loop outside the UI-state borrow so animation keeps running.

### IPC protocol

Newline-delimited JSON on `\\.\pipe\hytte`, one object per line, every message carries a version:

```json
{"v":1,"task_id":"agent:Claude Code:4242","event":"NeedsInput","label":"Claude Code",
 "source":"agent","pid":4242,"message":"Approve edit to src/main.rs?"}
```

`event` is one of `Start`, `Progress`, `Done`, `Failed`, `Lost`, `NeedsInput`, `Resumed`. Optional fields (`source`, `pid`, `delay_ms`, `message`, `cwd`, `exit_code`, `duration_ms`, `stderr_tail`) are additive: older clients' messages still parse. The server treats all input as untrusted: the pipe's DACL grants access only to the owning user, remote clients are rejected, and messages are length-bounded, version-checked and field-validated. Each client connection is served on its own thread.

### Performance budgets

| Budget | Target | Measured (release build) |
|---|---|---|
| Idle CPU, collapsed, nothing playing | < 1 % | ≈ 0.2 % |
| Idle RAM | < 30 MB stretch, 100 MB ceiling | ≈ 55–75 MB working set |
| Animation | Monitor refresh rate during transitions only | vsync-paced, parks when settled |

## Development

```powershell
cargo build --workspace      # debug build
cargo test --workspace       # unit tests (springs, UI model, transforms, protocol, ports, shelf, …)
cargo run -p hytte           # run the daemon
cargo run -p notch -- run -- cmd /c "exit 3"
```

Rules of thumb used throughout the code:

1. Never steal focus. `WM_MOUSEACTIVATE` returns `MA_NOACTIVATE`; the only window Hytte ever foregrounds is a terminal, and only in response to a click on the pill.
2. Event-driven, no polling. Where Windows forces a wait loop (the port scan) it is documented and rate-limited.
3. Render only on change; a settled, static pill draws nothing.
4. Treat everything from the pipe and from dropped data as untrusted.

To try states without a real workload:

```powershell
notch run -- cmd /c "echo boom 1>&2 & exit 3"      # failure + stderr
notch run -- powershell -c "Start-Sleep 8"          # running
notch set --progress 40 --label "Uploading" --task up1
notch agent needs-input --name "Claude Code" --message "Approve edit?"
python -m http.server 3000                          # shows up in Ports
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| *"another instance is running"* | Hytte is already running; look for it in the tray. |
| Nothing shows up | A fullscreen or monitor-sized window triggers suppression; look for the 3 px grey line, or set `suppress_fullscreen = false`. Check the tray **Pause** state. |
| `notch: daemon not running (message dropped)` | Start `hytte.exe`. `notch run` still runs your command in the meantime. |
| Shell commands never appear | Run `notch init <shell>` output in your profile, check `notch` is on `PATH`, and remember commands under 3 s (or in `[shell] ignore`) are hidden on purpose. |
| Clicking an agent/task row does nothing | The owning terminal window couldn't be found from its process; pass `--pid` explicitly. In Windows Terminal the tab is selected too, as long as the task was first seen while its tab was active. |
| A port you stopped still shows | The list refreshes whenever the pill opens and is re-checked on click; a stale entry is removed instead of erroring. |
| A port isn't listed | Only `[ports] watch` ports are shown by default; add it or set `show_all = true`. Listeners bound to a specific non-loopback address are hidden. |
| *OCR unavailable* | Install an OCR language pack in Windows language settings. Images larger than the engine's limit (about 4096 px) are skipped. |
| No media controls | Hytte shows whatever Windows reports as the current media session; the app must publish one. |
| Dot shows the wrong app | The name comes from the registry entry Windows keeps; unusual apps can report oddly. |

## Known limitations

- **Primary monitor only.** The code carries a monitor concept, but multi-monitor placement isn't implemented.
- **`acrylic = true`** makes the pill translucent but doesn't blur what is behind it.
- **YAML** is whitespace-normalised, not re-serialised (no YAML parser dependency).
- **Lossless WebP only**; lossy WebP and AVIF are not included.
- **Browser image drags** (which carry image data rather than a file) aren't handled; save the image first, or drop the file.
- **Shell integrations:** PowerShell and the drag/agent/port features were exercised on a real desktop; the bash, zsh and Nushell scripts are written to the same protocol but have had less testing. Shell tasks have no stderr tail.
- **Ports** are discovered by polling (5 s default); a very short-lived server may never appear.
- **Power mode** uses Windows' power-mode overlay (the Settings → System → Power mode switch); it isn't the separate Battery Saver feature.
- **Windows Terminal tab targeting** is best effort: Hytte records the selected tab when it first sees a task, so a task started from a background tab (for example an agent launched, then you switched away before it reported in) may select the wrong tab. Other terminals get window focus only.
- Unsigned binaries may trigger SmartScreen.
- Not yet exercised against exclusive-fullscreen games or anti-cheat-protected titles.

## License

MIT, as declared in the winget manifest.
