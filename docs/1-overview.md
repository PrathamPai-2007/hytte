# 1. Overview

## What Hytte is

Hytte is a "dynamic notch" for Windows 11: a small pill-shaped overlay that hangs from the top-centre of the primary monitor. It shows ambient information (running tasks, agents waiting for input, dev servers, media, the shelf, a timer) and expands when you hover it.

Three properties shape almost every design decision in the code:

1. **It must never get in the way.** The window never takes keyboard focus, transparent pixels let clicks through to the windows behind, and it backs off when a game or video goes fullscreen.
2. **It must cost nothing while idle.** No polling loops where Windows offers events, no frames drawn when nothing moves, and no async runtime.
3. **It must be safe.** Anything arriving over the named pipe, or dropped onto the pill, is treated as untrusted input.

## The two programs

The repository builds two executables from one Cargo package (`crates/hytte`):

| Binary | Source | Role |
|---|---|---|
| `hytte.exe` | `crates/hytte/src/main.rs` | The **daemon**: a long-running GUI process that owns the pill window, the tray icon and all background workers. Only one copy can run at a time. |
| `notch.exe` | `crates/hytte/src/cli/main.rs` | The **client**: a short-lived console program that terminals, scripts and agent hooks run. It sends one message to the daemon over a named pipe and exits. |

They share types through a small library crate, `crates/proto` (`hytte-proto`).

## Vocabulary

These words are used consistently in the code and in these docs.

| Term | Meaning |
|---|---|
| **Pill** | The visible black shape at the top of the screen. |
| **Canvas** | The fixed-size, mostly transparent layered window the pill is drawn into (460 × 310 logical px). The pill animates *inside* it; the window itself never resizes. |
| **Scene** | What the pill is currently showing, for example `Idle`, `CompactTask` or `ExpShelf`. Exactly one scene is active at a time. See `ui_state::Scene`. |
| **Compact / expanded** | Compact scenes are the small, collapsed states (`Idle`, `CompactTask`, `CompactMedia`). Expanded scenes (`Exp*`) are shown on hover or when the pill "peeks". |
| **Panel** | A page of the expanded pill: Tasks, Shelf, Media, Ports, Timer, Home. Panels are reached with the tab dots or the mouse wheel. Each panel maps to one expanded scene. |
| **Peek** | The pill opening on its own for a few seconds (an agent needs input, a file landed on the shelf). |
| **Sentinel** | The 3 px grey bar shown instead of the pill while a fullscreen app is in front. |
| **Task** | Something being tracked: a `notch run` command, a shell command, an agent, or a `notch set` progress report. |
| **Shelf** | The list of files and text snippets parked on the pill. |
| **Drop Vault** | The one-click actions offered for shelf items (compress, PDF, remove metadata, OCR, format). |
| **Chip** | A small result card shown after an action ("Stopped :3000", "PDF · 1.2 MB"). |
| **Worker** | A background thread that watches something (media, ports, privacy, ...) and sends events to the UI thread. |
| **`UiEvent`** | The message type workers use to talk to the UI thread. |
| **Ambient animation** | Continuous motion that is not a spring settling: the spinner, the equaliser, the amber pulse. It runs the frame clock at about 30 fps. |

## Repository map

```text
Cargo.toml                 workspace: members, shared dependency versions, release profile
rust-toolchain.toml        pins stable Rust with the x86_64-pc-windows-msvc target
crates/
  proto/                   hytte-proto: code shared by daemon and CLI
    src/lib.rs             HytteMessage (the pipe protocol), TaskEvent, validation
    src/ports.rs           listening-port discovery, filtering and kill
    src/sys.rs             process-tree helpers (parent pid, "owner" pid)
  hytte/                   the package that builds both binaries
    Cargo.toml
    build.rs               embeds the Windows manifest (per-monitor DPI v2)
    app.manifest
    src/main.rs            daemon entry point: starts workers, then the UI loop
    src/window.rs          Win32 window, message loop, input, timers, drag & drop
    src/render.rs          Direct2D / DirectWrite drawing of every scene
    src/ui_state.rs        the pure UI model: tasks, panels, scenes, animation state
    src/animation.rs       spring physics
    src/pipe_server.rs     named-pipe listener
    src/tasks.rs           task registry (staleness, dead owner detection)
    src/proc.rs            process / window helpers (exe names, liveness, focusing)
    src/ports.rs           port-watcher worker
    src/media.rs           media session watcher + album art
    src/privacy.rs         camera / microphone in-use watcher
    src/mic.rs             global microphone mute
    src/power.rs           battery and power mode
    src/tabs.rs            Windows Terminal tab tracking (UI Automation)
    src/shelf.rs           shelf items, persistence, thumbnails
    src/drop.rs            Drop Vault job queue and worker pool
    src/transforms.rs      Drop Vault actions (image clean, JSON/YAML, OCR, ...)
    src/convert.rs         "Compress under 5 MB" and the hand-written PDF writer
    src/timer.rs           Timer / Focus / Break state, persistence, chime
    src/fullscreen.rs      fullscreen suppression decision
    src/tray.rs            tray icon and its menu
    src/config.rs          config.toml, autostart, Start Menu shortcut
    src/logging.rs         log file and panic hook
    src/perf.rs            optional frame-time telemetry (HYTTE_PERF=1)
    src/single_instance.rs named mutex so only one daemon runs
    src/winrt.rs           tiny blocking driver for WinRT async calls
    src/cli/main.rs        notch: run, set, and command dispatch
    src/cli/agent.rs       notch agent ...
    src/cli/hook.rs        notch hook ... and notch init ...
    src/cli/portscmd.rs    notch ports / notch kill
    src/cli/shell/*.ps1|bash|zsh|nu   shell integration scripts, embedded into notch.exe
docs/                      this documentation and screenshots (docs/img)
manifests/winget/          winget package manifest template
.github/workflows/         release workflow (builds and publishes on a version tag)
```

## Dependencies, and why there are so few

| Crate | Used for |
|---|---|
| `windows`, `windows-core`, `windows-numerics` | Every OS call: Win32, COM, Direct2D, DirectWrite, WinRT. |
| `serde`, `serde_json` | The pipe protocol, `shelf.json`, `timer.json`, JSON formatting in the Drop Vault. |
| `toml` | `config.toml`. |
| `crossbeam-channel` | Channels between threads, including `select!`. |
| `image` | Decoding and encoding images for the Drop Vault and album art. Only PNG, JPEG, WebP and BMP are enabled. |
| `miniz_oxide` | zlib compression for PDF image streams. |
| `embed-manifest` (build only) | Embedding the application manifest. |

There is deliberately **no async runtime** (no tokio), no GUI framework and no web view. Everything is plain threads, channels and direct Windows API calls. When you add a dependency, it should be because the alternative is clearly worse, and it should keep `notch.exe` fast to start.

Next: [2. Getting started](2-getting-started.md)
