# 1. Overview

## What Hytte is

Hytte is a "dynamic notch" for Windows 11: a small pill-shaped overlay that hangs from the top-centre of your screen (the primary monitor by default; see `[general] monitor`). It shows ambient information (running tasks, agents waiting for input, dev servers, media, the shelf, a timer) and expands when you hover it.

Three properties shape almost every design decision in the code:

1. **It must never get in the way.** The window never takes keyboard focus, clicks outside the pill pass through to the windows behind, and it backs off when a game or video goes fullscreen.
2. **It must cost nothing while idle.** No polling loops where Windows offers events, no frames drawn when nothing moves, and no async runtime.
3. **It must be safe.** Anything arriving over the named pipe, or dropped onto the pill, is treated as untrusted input.

## One program, two modes

The repository builds a single executable, `hytte.exe`, from the Cargo package `crates/hytte`. What it does depends on its arguments; `main()` decides before anything else runs:

| You run | Mode | Source | Role |
|---|---|---|---|
| `hytte` (no arguments) | **daemon** | `crates/hytte/src/main.rs` | The long-running process that owns the pill window, the tray icon and all background workers. Only one copy can run at a time. |
| `hytte <command> ...` | **CLI** | `crates/hytte/src/cli/` | A short-lived command that terminals, scripts and agent hooks run (`hytte run`, `hytte agent`, `hytte hook`, ...). It usually sends one message to the daemon over a named pipe and exits. It never touches daemon state: no log file, no mutex, no config. |

Before 1.0 these were two programs, `hytte.exe` and `notch.exe`; see [section 5](05-cli.md#upgrading-from-notch) for what changed.

`hytte.exe` is a **console-subsystem** program, so the CLI behaves like any command-line tool: the shell waits for it, output streams, and exit codes come back. The daemon must not keep a console, which is handled in two layers:

1. The app manifest sets `consoleAllocationPolicy` to `detached`. On Windows 11 24H2 and newer, Windows then gives no console to the exe when it's started from Explorer, the Start Menu or autostart. When started from a terminal, it still shares that terminal's console.
2. `main.rs::detach_console` covers the rest. If the process was given a console of its own (older Windows builds), it frees it, which can show as a brief flash. If it shares a terminal's console because someone typed `hytte`, it starts a detached copy of itself and exits, so the prompt comes straight back.

Both modes share types with a small library crate, `crates/proto` (`hytte-proto`).

## Vocabulary

These words are used consistently in the code and in these docs.

| Term | Meaning |
|---|---|
| **Pill** | The visible black shape at the top of the screen. |
| **Canvas** | The fixed-size, mostly transparent window the pill is drawn into (460 × 310 logical px). The pill animates *inside* it; the window itself never resizes. |
| **Scene** | What the pill is currently showing, for example `Idle`, `CompactTask` or `ExpShelf`. Exactly one scene is active at a time. See `ui_state::Scene`. |
| **Compact / expanded** | Compact scenes are the small, collapsed states (`Idle`, `CompactTask`, `CompactMedia`). Expanded scenes (`Exp*`) are shown on hover or when the pill "peeks". |
| **Panel** | A page of the expanded pill, left to right: Shelf, Media, Home (the quick controls, the default), Tasks (with the listening dev servers under them), Timer, Power. Panels are reached with the tab dots or the mouse wheel. Each panel maps to one expanded scene. |
| **Peek** | The pill opening on its own for a few seconds (an agent needs input, a file landed on the shelf). |
| **Sentinel** | The 3 px grey bar shown instead of the pill while a fullscreen app is in front. |
| **Task** | Something being tracked: a `hytte run` command, a shell command, an agent, or a `hytte set` progress report. |
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
    src/lib.rs             HytteMessage (the pipe protocol), TaskEvent, validation, Ask / Answer
    src/ports.rs           listening-port discovery, filtering and kill, DEFAULT_WATCH
    src/sys.rs             process-tree helpers (parent pid, ancestor chains, "owner" pid)
  hytte/                   the package that builds hytte.exe
    Cargo.toml
    build.rs               compiles hytte.rc into the exe (fails the build if it can't)
    hytte.rc               Win32 resources: app icon, tray icons, manifest, version info
    app.manifest           DPI awareness, UTF-8 code page, console allocation policy, ...
    assets/                logo.png, the generated .ico files and make_icons.py
    src/main.rs            entry point: CLI dispatch, console handling, then the daemon
    src/window.rs          Win32 window, message loop, input, timers, drag & drop
    src/render.rs          Direct2D / DirectWrite drawing of every scene
    src/gpu.rs             D3D11 + DirectComposition swap chain for the gpu renderer
    src/ui_state.rs        the pure UI model: tasks, panels, scenes, animation state
    src/animation.rs       spring physics
    src/pipe_server.rs     named-pipe listener
    src/tasks.rs           task registry (staleness, dead owner detection)
    src/proc.rs            process / window helpers (exe names, liveness, focusing)
    src/ports.rs           port-watcher worker
    src/media.rs           media session watcher + album art
    src/privacy.rs         camera / microphone in-use watcher
    src/mic.rs             global microphone mute, speaker volume and mute
    src/hog.rs             resource-hog alert (CPU / memory sampling and the "how long" rule)
    src/downloads.rs       browser downloads as tasks (folder change notifications)
    src/calendar.rs        heads-up before the next calendar event
    src/power.rs           battery and power mode
    src/tabs.rs            Windows Terminal tab tracking (UI Automation)
    src/shelf.rs           shelf items, persistence, thumbnails
    src/drop.rs            Drop Vault job queue and worker pool
    src/transforms.rs      Drop Vault actions (image clean, JSON/YAML, OCR, ...)
    src/convert.rs         "Compress under 5 MB" and the hand-written PDF writer
    src/timer.rs           Timer / Focus / Break state, persistence, alarm, local clock
    src/fullscreen.rs      fullscreen suppression decision
    src/tray.rs            tray icon (follows the taskbar theme) and its menu
    src/config.rs          config.toml, autostart, Start Menu shortcut
    src/setup.rs           PATH and shell-profile setup, migration of old notch hooks
    src/agents.rs          hytte setup agents: hooks for Claude Code, Codex, Gemini, Antigravity, Cursor
    src/agent_plugin.js    the OpenCode plugin agents.rs writes (embedded)
    src/update.rs          update check against GitHub Releases and in-place self-replace
    src/logging.rs         log file and panic hook
    src/perf.rs            optional frame-time telemetry (HYTTE_PERF=1)
    src/single_instance.rs named mutex so only one daemon runs
    src/winrt.rs           tiny blocking driver for WinRT async calls
    src/cli/mod.rs         the CLI: command dispatch, hytte run, hytte set, hytte setup
    src/cli/agent.rs       hytte agent ...
    src/cli/hook.rs        hytte hook ... and hytte init ...
    src/cli/portscmd.rs    hytte ports / hytte kill
    src/cli/progress.rs    finds progress (OSC 9;4, NN%) in hytte run's output
    src/cli/shell/*.ps1|bash|zsh|nu   shell integration scripts, embedded into hytte.exe
docs/                      this documentation and screenshots (docs/img)
manifests/winget/          winget package manifest template
.github/workflows/         ci.yml (fmt, clippy, tests on every push) and release.yml (on a version tag)
.gitattributes             keeps the embedded shell scripts LF in Windows checkouts
```

## Dependencies, and why there are so few

| Crate | Used for |
|---|---|
| `windows`, `windows-core`, `windows-numerics` | Every OS call: Win32, COM, Direct2D, DirectWrite, WinRT. |
| `serde`, `serde_json` | The pipe protocol, `shelf.json`, `timer.json`, JSON formatting in the Drop Vault. |
| `toml` | `config.toml`. |
| `toml_edit` | Changing one key in `config.toml` (the tray's autostart toggle) without losing the user's comments. |
| `crossbeam-channel` | Channels between threads, including `select!`. |
| `image` | Decoding and encoding images for the Drop Vault and album art. Only PNG, JPEG, WebP and BMP are enabled. |
| `miniz_oxide` | zlib compression for PDF image streams. |
| `embed-resource` (build only) | Compiling `hytte.rc` (icons, manifest, version info) into the exe. |

There is deliberately **no async runtime** (no tokio), no GUI framework and no web view. Everything is plain threads, channels and direct Windows API calls. When you add a dependency, it should be because the alternative is clearly worse, and it should keep the CLI fast to start (shell hooks run `hytte` on every command).

Next: [2. Getting started](02-getting-started.md)
