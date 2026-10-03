# Hytte

A dynamic notch and ambient cockpit for **Windows 11**, written in Rust.

Hytte hangs a small pill from the top-centre of your screen. It stays out of the way until something happens — a build finishes, music plays, your camera turns on, you drag a file near the top edge — then it springs open, shows what matters, and folds away again. It never steals focus, draws no frames while idle, and has no Electron, no web view, and no async runtime.

## Table of contents

- [Features](#features)
- [Requirements](#requirements)
- [Install](#install)
  - [Build from source](#build-from-source)
  - [Run at login](#run-at-login)
- [Quick start](#quick-start)
- [Using Hytte](#using-hytte)
  - [The pill at a glance](#the-pill-at-a-glance)
  - [Hover and click behaviour](#hover-and-click-behaviour)
  - [Tray menu](#tray-menu)
- [Process Sentinel (`notch`)](#process-sentinel-notch)
  - [`notch run`](#notch-run)
  - [`notch set`](#notch-set)
  - [Recipes](#recipes)
- [Drop Vault](#drop-vault)
- [Media cockpit](#media-cockpit)
- [Privacy indicators](#privacy-indicators)
- [Fullscreen and game behaviour](#fullscreen-and-game-behaviour)
- [Configuration](#configuration)
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
| **Process Sentinel** | Wrap any command with `notch run -- <cmd>` and watch it in the notch: live elapsed time, a shimmering progress filament, a green check on success, a red pulse plus the last stderr lines on failure. Scripts that know their progress can report it with `notch set`. |
| **Drop Vault** | Drag files or text toward the top edge. Images get EXIF/GPS stripped, a lossless WebP copy, and OCR text on your clipboard. JSON is prettified or minified, YAML is normalised, any other file gets its path copied and revealed in Explorer. |
| **Media cockpit** | Title, artist, album art, a live timeline, animated equaliser bars and prev / play-pause / next for whatever Windows reports as the current media session (Spotify, browsers, etc.). |
| **Privacy indicators** | A green dot while any app uses your camera, an orange dot for the microphone, with the app's name when you hover. |
| **Fullscreen aware** | Backs off to a hairline (or hides) when a game or video is fullscreen, with per-process allow/deny lists. |
| **Fluid animation** | Spring-driven resize, content cross-fades, a morphing corner radius, soft state-coloured glow, and a frame clock that parks completely at idle. Honours Windows' "Animation effects" setting. |

## Requirements

- Windows 11 22H2 or newer (x64). There are no Windows 10 fallbacks.
- The Rust stable toolchain with the MSVC target, only if you build from source (`rust-toolchain.toml` pins it).
- Optional: a Windows OCR language pack for image text recognition (*Settings → Time & language → Language & region*). Without one, everything else in the Drop Vault still works and the chip says *OCR unavailable*.

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
| `notch.exe` | A tiny CLI client used from terminals and scripts. |

Put both on your `PATH` (or copy them to a folder that is) so `notch` works from any shell.

> A winget manifest template lives in [`manifests/winget`](manifests/winget). It still points at a placeholder URL; fill in the real release asset before submitting it to winget-pkgs.

### Run at login

Either tick **Launch at startup** in the tray menu, or set `autostart = true` in the [config file](#configuration). Hytte writes (or removes) a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Nothing is installed system-wide.

## Quick start

```powershell
# 1. start the daemon (only one instance can run)
hytte

# 2. wrap something slow
notch run -- cargo build --release

# 3. watch the top of your screen
```

While the command runs you see a spinner, the command, elapsed time and a moving filament. When it ends the pill turns green (success, collapses after 3 s) or red (failure, stays until you dismiss it).

## Using Hytte

### The pill at a glance

| State | You see | When |
|---|---|---|
| **Idle** | A small near-black capsule with a faint bar | Nothing is happening |
| **Task** | Spinner / check / cross, label, elapsed time or `%`, progress filament, a count badge if several tasks are running | One or more `notch` tasks exist |
| **Media** | Cover thumbnail, `Title — Artist`, animated equaliser | Audio is playing and no task needs attention |
| **Drop shelf** | Dashed purple drop zone, bobbing arrow | You are dragging a file or text over the notch |
| **Result chip** | What was done, plus **Open / Copy / Dismiss** | A drop finished |
| **Sentinel** | A 3 px grey line | Fullscreen is active and the mode is `sentinel` |

A coloured glow reflects the state: blue running, green success, red failure, purple drops.

### Hover and click behaviour

- Rest the pointer on the pill for about **120 ms** and it expands. Leave and it collapses after **300 ms**; re-entering in that window cancels the collapse, so grazing the edge doesn't flicker.
- Expanded task list: up to four rows, failed first. A failed row shows the last stderr lines with **Copy error** and **Dismiss**; finished rows have a small ✕.
- Expanded media view: click ⏮ ⏯ ⏭, see the live timeline.
- Hover over any button to highlight it; the cursor becomes a hand.
- The window never takes focus, even when clicked.

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
- Exits with the **same exit code** as the wrapped command (`127` if it couldn't be spawned), so it is safe in scripts and CI-style chains.
- The label is the command line, truncated to 128 characters.
- Progress is indeterminate (a shimmer): an arbitrary command can't report a percentage.

### `notch set`

```text
notch set --progress <0-100> --label "text" [--task <id>]
```

For scripts that *do* know how far along they are. Messages with the same `--task` id update one entry; omit it and each call is a new task.

```powershell
foreach ($i in 0..10) {
    notch set --progress ($i * 10) --label "Exporting" --task export-1
    Start-Sleep 1
}
```

> Tasks whose client disappears without reporting an end are marked **lost** after 30 s of silence, shown with a `?`.

### Recipes

```powershell
notch run -- cargo test                      # tests
notch run -- npm run build                   # front-end build
notch run -- robocopy C:\src D:\backup /MIR  # long copy
function nb { notch run -- cargo build @args }   # shell shortcut (add to $PROFILE)
```

## Drop Vault

Drag something toward the top of the screen with the left button held. The notch becomes a drop zone as soon as the pointer is over it (a 40 px strip along the top edge becomes an invisible landing area while you drag, so you don't have to aim precisely). Release to process.

| You drop | Result |
|---|---|
| **PNG / JPEG** | `name.clean.png|jpg` (re-encoded, so EXIF and GPS are gone), `name.webp` (lossless), and OCR text copied to the clipboard |
| **WebP / BMP** | OCR text copied to the clipboard |
| **JSON file** | `name.pretty.json` if it was compact, `name.min.json` if it was already formatted; result also copied |
| **YAML file** | `name.pretty.yaml` with trailing whitespace trimmed, tabs expanded, runs of blank lines collapsed |
| **Any other file** | Its full path is copied and Explorer opens with it selected |
| **Text** (e.g. selected in an editor) | JSON is prettified or minified; other text is cleaned and copied |

Outputs go **next to the source file** with a suffix, or into `output_folder` if you set one. Hytte **never overwrites**: existing names get `-1`, `-2`, … appended. Dropped files are never executed.

The result chip offers **Open** (the produced file), **Copy** (the produced text), and **Dismiss**; it clears itself after 10 s.

## Media cockpit

Hytte reads Windows' System Media Transport Controls, the same source as the volume flyout, and updates on events rather than polling. Cover art is decoded once per track and cached at 64×64. While expanded, the timeline advances smoothly between updates and the equaliser bars animate only while something is actually playing. Transport buttons control the current session.

## Privacy indicators

A **green** dot means the camera is in use, an **orange** dot the microphone. Hytte watches the Windows `CapabilityAccessManager\ConsentStore` registry keys with change notifications, covering both Store apps and classic desktop apps (Zoom, OBS, browsers). The expanded home view names the app using the device. Dots stay still while the pill is collapsed (so an open call doesn't cost CPU) and gently breathe when you hover.

## Fullscreen and game behaviour

Hytte re-checks whenever the foreground window changes or moves, using the shell's *user notification state* (D3D fullscreen, presentation mode, busy) with a "window covers the whole monitor" fallback.

| `fullscreen_mode` | Behaviour while fullscreen |
|---|---|
| `"sentinel"` (default) | Shrinks to a 3 px grey line at the top edge. Hover it and the notch reveals itself. |
| `"hide"` | Fades out entirely. |

`allow_list` (never suppress) and `deny_list` (always hide) take executable file names such as `"Code.exe"`. Set `suppress_fullscreen = false` to turn the feature off. Hytte is a plain topmost window with no injection or hooks into other processes, so it cannot draw over exclusive-fullscreen games; suppression just keeps it from interfering.

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
```

Restart `hytte` (tray → Quit, then launch again) after editing. Invalid files fall back to defaults. Logs and panic traces go to `%APPDATA%\Hytte\hytte.log`.

## How it works

### Workspace layout

```text
crates/
  proto/    shared IPC message type + validation (used by both binaries)
  notch/    the CLI client (run / set)
  hytte/    the daemon
    src/main.rs          wiring: starts workers, then the UI loop
    src/window.rs        Win32 window, input, timers, OLE drop target, hooks
    src/render.rs        Direct2D/DirectWrite drawing of every scene
    src/ui_state.rs      pure UI model, scene selection, animation state
    src/animation.rs     spring integrator
    src/pipe_server.rs   ACL'd named-pipe server
    src/tasks.rs         task registry (stale detection)
    src/media.rs         media session events + album art
    src/privacy.rs       camera/mic registry watcher
    src/drop.rs          drop job queue + worker pool
    src/transforms.rs    image / JSON / YAML / OCR transforms
    src/fullscreen.rs    suppression decision
    src/tray.rs, config.rs, logging.rs, single_instance.rs, winrt.rs
manifests/winget/        packaging template
```

### Rendering and animation

- **One layered, topmost, no-activate window**, centred on the primary monitor's top edge. Rendering is Direct2D onto a premultiplied-alpha bitmap, then `UpdateLayeredWindow`. That gives anti-aliased rounded shapes with a true notch silhouette (concave "ears" flaring into the screen edge), gradients and glow. Fully transparent pixels are click-through, so Hytte never swallows clicks meant for other windows.
- Only the **bounding box of the pill** is drawn and presented each frame, not the whole canvas.
- Layout is expressed in logical pixels and scaled for **per-monitor DPI**.
- The pill's size, corner radius, glow, hover brightness, content fade and visibility are **critically-damped springs** (`ζ ≈ 0.78` for the shape, a bit stiffer for the rest), integrated with real frame deltas in small sub-steps. Scene changes cross-fade and slide the content in.
- **No idle loop.** A ticker thread parks when nothing moves. During motion it is paced by `DwmFlush` (vsync); for ambient effects such as the spinner and equaliser it ticks at about 30 fps. With *Animation effects* turned off in Windows, springs snap and ambient motion stops.

### Threading model

Plain `std::thread` plus channels; there is no async runtime. The UI thread owns the window and all UI state. Worker threads (pipe server, task registry, media, privacy, drop transforms) send `UiEvent`s into a queue and post one wake-up message; the UI thread drains it. WinRT async calls are driven by a tiny blocking helper with timeouts.

### IPC protocol

Newline-delimited JSON on `\\.\pipe\hytte`, one object per line, every message carries a version:

```json
{"v":1,"task_id":"1234-1700000000000","event":"Failed","label":"cargo build",
 "duration_ms":4210,"exit_code":101,"stderr_tail":"error[E0425]: ..."}
```

`event` is one of `Start`, `Progress`, `Done`, `Failed`, `Lost`. The server treats all input as untrusted: the pipe's DACL grants access only to the owning user, remote clients are rejected, and messages are length-bounded, version-checked and field-validated (`task_id` ≤ 128 bytes, `label` ≤ 1 KiB, `progress` ≤ 100, `stderr_tail` ≤ 8 KiB). Each client connection is served on its own thread.

### Performance budgets

| Budget | Target | Measured (release build) |
|---|---|---|
| Idle CPU, collapsed, nothing playing | < 1 % | ≈ 0.2 % |
| Idle RAM | < 30 MB stretch, 100 MB ceiling | ≈ 55–75 MB working set |
| Animation | Monitor refresh rate during transitions only | vsync-paced, parks when settled |

## Development

```powershell
cargo build --workspace      # debug build
cargo test --workspace       # unit tests (springs, UI model, transforms, protocol, ...)
cargo run -p hytte           # run the daemon
cargo run -p notch -- run -- cmd /c "exit 3"
```

Rules of thumb used throughout the code:

1. Never steal focus. No `SetForegroundWindow` on the notch window, `WM_MOUSEACTIVATE` returns `MA_NOACTIVATE`.
2. Event-driven, no polling. Where Windows forces a wait loop it is a blocking wait with a long timeout.
3. Render only on change; a settled, static pill draws nothing.
4. Treat everything from the pipe and from dropped data as untrusted.

To try states without a real workload:

```powershell
notch run -- cmd /c "echo boom 1>&2 & exit 3"      # failure + stderr
notch run -- powershell -c "Start-Sleep 8"          # running
notch set --progress 40 --label "Uploading" --task up1
```

## Troubleshooting

| Symptom | Fix |
|---|---|
| *"another instance is running"* | Hytte is already running; look for it in the tray. |
| Nothing shows up | A fullscreen or maximised-to-monitor window triggers suppression; look for the 3 px grey line, or set `suppress_fullscreen = false`. Check the tray **Pause** state. |
| `notch: daemon not running (message dropped)` | Start `hytte.exe`. `notch run` still runs your command in the meantime. |
| *OCR unavailable* | Install an OCR language pack in Windows language settings. Images larger than the OCR engine's limit (about 4096 px) are skipped. |
| No media controls | Hytte shows whatever Windows reports as the current media session; the app must publish one. |
| Dot shows the wrong app | The name comes from the registry entry Windows keeps; unusual apps can report oddly. |

## Known limitations

- **Primary monitor only.** The code carries a monitor concept, but multi-monitor placement isn't implemented.
- **`acrylic = true`** makes the pill translucent but doesn't blur what is behind it.
- **YAML** is whitespace-normalised, not re-serialised (no YAML parser dependency).
- **Lossless WebP only**; lossy WebP and AVIF are not included.
- **Browser image drags** (which carry image data rather than a file) aren't handled; save the image first, or drop the file.
- Unsigned binaries may trigger SmartScreen.
- Not yet exercised against exclusive-fullscreen games or anti-cheat-protected titles.

## License

MIT, as declared in the winget manifest.
