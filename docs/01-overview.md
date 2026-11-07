# 1. Overview

## What Hytte is

Hytte is a "dynamic notch" for Windows 11: a small pill-shaped overlay hanging from the top-centre of the screen (primary monitor by default, see `[general] monitor`). It shows ambient information (tasks, agents waiting, dev servers, media, the shelf, a timer) and expands on hover.

Three properties shape almost every design decision:

1. **Never in the way.** The window never takes keyboard focus, clicks outside the pill pass through, and it backs off when a game or video goes fullscreen.
2. **Free while idle.** No polling where Windows offers events, no frames when nothing moves, no async runtime.
3. **Safe.** Anything arriving over the named pipe, or dropped on the pill, is untrusted input.

## One program, two modes

The repo builds one executable, `hytte.exe` (package `crates/hytte`). `main()` picks the mode from the arguments:

| You run | Mode | Role |
|---|---|---|
| `hytte` | **daemon** (`src/main.rs`) | The long-running process that owns the pill, the tray icon and all workers. One copy at a time. |
| `hytte <command> ...` | **CLI** (`src/cli/`) | A short-lived command run by terminals, scripts and agent hooks. Usually sends one message over the named pipe and exits. It never touches daemon state: no log, no mutex, no config. |

Before 1.0 these were two programs (`hytte.exe` and `notch.exe`); see [section 5](05-cli.md#upgrading-from-notch).

`hytte.exe` is a **console-subsystem** program so the CLI behaves normally (the shell waits, exit codes return). The daemon must not keep a console:

1. The manifest sets `consoleAllocationPolicy` to `detached`, so on Windows 11 24H2+ an exe started from Explorer, the Start Menu or autostart gets no console.
2. `main.rs::detach_console` covers the rest. If the process got its own console (older Windows), it frees it (a brief flash). If it shares a terminal's console because someone typed `hytte`, it starts a detached copy of itself and exits, so the prompt returns at once.

Both modes share the small `crates/proto` library (`hytte-proto`).

## Vocabulary

| Term | Meaning |
|---|---|
| **Pill** | The visible black shape at the top of the screen. |
| **Canvas** | The fixed-size, mostly transparent window the pill is drawn into (460 × 310 logical px). The pill animates *inside* it; the window never resizes. |
| **Scene** | What the pill shows right now (`Idle`, `CompactTask`, `ExpShelf`, ...). Exactly one is active. See `ui_state::Scene`. |
| **Compact / expanded** | Compact scenes are the collapsed states (`Idle`, `CompactTask`, `CompactMedia`); `Exp*` scenes show on hover or a peek. |
| **Panel** | A page of the expanded pill: Shelf, Media, Home (quick controls, the starting point), Notes, To-do, Weather, Tasks (dev servers under it), Timer, Power. Each maps to one expanded scene; all but Home can be switched off and reordered (`[panes]`). |
| **Peek** | The pill opening by itself for a few seconds (agent needs input, file landed on the shelf). |
| **Sentinel** | The thin grey bar (96 × 2.5 px) shown instead of the pill under a maximised, snapped or fullscreen window. |
| **Task** | Something tracked: a `hytte run` command, a shell command, an agent, or a `hytte set` report. |
| **Shelf / Drop Vault** | Files and text parked on the pill / the one-click actions offered for them. |
| **Chip** | A small result card after an action ("Stopped :3000"). |
| **Worker** | A background thread watching something and sending `UiEvent`s to the UI thread. |
| **Ambient animation** | Continuous motion that isn't a spring settling (spinner, equaliser, amber pulse); runs the frame clock at about 30 fps. |

## Repository map

```text
Cargo.toml, rust-toolchain.toml   workspace, shared dependency versions, pinned toolchain
scripts/                          release.ps1, version.ps1 (version and winget hash in step)
crates/proto/                     hytte-proto: pipe protocol (lib.rs), port discovery (ports.rs), process-tree helpers (sys.rs)
crates/hytte/                     builds hytte.exe
  build.rs, hytte.rc, app.manifest   icons, version info, DPI / UTF-8 / console policy
  assets/                            logo and generated .ico files
  src/main.rs                        entry: CLI dispatch, console handling, daemon
  src/window.rs + window/            Win32 window, message loop, input, timers, drag and drop
  src/render.rs + render/            Direct2D / DirectWrite drawing, one file per pane
  src/settings_win.rs                the big settings window
  src/gpu.rs                         D3D11 + DirectComposition swap chain
  src/ui_state.rs, animation.rs      pure UI model, spring physics
  src/pipe_server.rs, tasks.rs       pipe listener, task registry
  src/ports, media, privacy, mic, hog, downloads, calendar, power, tabs .rs   background workers
  src/weather, lyrics, net .rs       weather and lyrics lookups, and the curl wrapper they share
  src/jot, textedit .rs              notes and to-dos, the one-line text editor
  src/breaks, sleep, dnd, timer .rs  break reminders, sleep timer, Do Not Disturb, timer
  src/shelf, drop, transforms, convert .rs   shelf and Drop Vault
  src/fullscreen, tray, config, skin, setup, agents, update .rs
  src/logging, secrets, perf, single_instance, winrt, proc, rerun .rs   support
  src/cli/                           command dispatch, doctor, agent, hook/init, ports, progress, shell scripts
docs/                             these docs and screenshots (docs/img)
manifests/winget/                 winget manifest (kept in step by the release tooling)
.github/workflows/                ci.yml (fmt, clippy, tests), release.yml (on a version tag)
```

## Dependencies

| Crate | Used for |
|---|---|
| `windows`, `windows-core`, `windows-numerics` | Every OS call: Win32, COM, Direct2D, DirectWrite, WinRT. |
| `serde`, `serde_json` | Pipe protocol, `shelf.json`, `timer.json`, `jot.json`, JSON formatting. |
| `toml`, `toml_edit` | Reading `config.toml`; changing one key without losing comments. |
| `crossbeam-channel` | Thread channels, including `select!`. |
| `image` | Image decode/encode (PNG, JPEG, WebP, BMP only). |
| `miniz_oxide` | zlib for PDF image streams. |
| `qrcodegen` | The shelf's QR code. |
| `embed-resource` (build) | Compiling `hytte.rc`. |

There is deliberately **no async runtime**, GUI framework or web view: plain threads, channels and direct Windows calls. Add a dependency only when the alternative is clearly worse and it keeps CLI startup fast (shell hooks run `hytte` on every command).

Next: [2. Getting started](02-getting-started.md)
