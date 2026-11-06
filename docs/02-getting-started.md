# 2. Getting started

## Prerequisites

- **Windows 11 22H2+, x64.** There are no Windows 10 fallbacks.
- **Rust stable (MSVC).** `rust-toolchain.toml` pins it; `rustup` installs it on first build.
- **Visual Studio Build Tools** ("Desktop development with C++") for the linker and resource compiler.
- Optional: an OCR language pack, to try "Read text".

## Build and run

```powershell
cargo build --workspace            # debug
cargo build --release --workspace  # optimised (LTO, slower to compile)
cargo run -p hytte                 # start the daemon
cargo run -p hytte -- run -- cmd /c "exit 3"   # CLI mode: send a failing task
```

`hytte.exe` lands in `target\debug\` or `target\release\`. Only one daemon runs per user session (named mutex `Local\HytteSingleInstance`), so quit an installed Hytte from the tray first or your dev build exits at once. `hytte run` starts the daemon itself if the pipe isn't answering.

To keep a dev build away from your real settings, point `APPDATA` elsewhere and set `add_to_path = false` in that copy's `config.toml` ([section 11](11-configuration-and-files.md)):

```powershell
$env:APPDATA = "$env:TEMP\hytte-dev"; .\target\release\hytte.exe
```

### Putting the pill into every state

```powershell
$n = "target\debug\hytte.exe"
& $n run -- cmd /c "echo boom 1>&2 & exit 3"           # failed task with stderr
& $n run -- powershell -c "Start-Sleep 8"               # running task
& $n set --progress 40 --label "Uploading" --task up1   # determinate progress
& $n agent needs-input --name "Claude Code" --message "Approve edit?"   # amber
& $n agent resume --name "Claude Code"                  # back to running
& $n run -- python -m http.server 3000                  # a server under SERVERS
```

Also: drag a file on (shelf, Drop Vault), play music (media), press **Timer** or right-click the pill, press **Mic** (mic lock).

## Debugging

The daemon has **no console** ([section 1](01-overview.md#one-program-two-modes)), so `println!` goes nowhere. Use:

| Tool | How |
|---|---|
| Log file | `crate::logging::line("...")` appends to `%APPDATA%\Hytte\hytte.log`; panics from any thread land there too. |
| Frame telemetry | `HYTTE_PERF=1` before starting: every 240 animated frames `perf.rs` logs p50 / p99 / max frame cost, frame gaps and missed vblanks. |
| Debugger | Attach Visual Studio, WinDbg or VS Code; debug builds have a `.pdb`. |

The CLI is a normal console program: errors print to the terminal.

Tips: UI state lives in a thread-local (`window.rs`, `UI`). A breakpoint in `Ui::frame` fires every frame while anything moves, so prefer `Ui::handle_events` or `Ui::after_model_change`. Hover relies on timers (`T_DWELL`, `T_COLLAPSE`), so pausing in the debugger makes the pill open or close on resume. A "stuck" pill usually means tray **Pause** or a fullscreen window (the sentinel bar).

## Working from macOS or Linux

You can type-check and lint, not link or run:

```bash
rustup target add x86_64-pc-windows-msvc
export RUSTUP_TOOLCHAIN=stable      # rust-toolchain.toml names the Windows host
cargo check  --workspace --all-targets --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo fmt --all --check
cargo test -p hytte-proto           # runs on any OS
```

The `hytte` crate's own tests don't compile off Windows (COM `#[implement]` types). To run the platform-independent ones (springs, UI model, shelf, conversions, timer, ...), build a throwaway crate that declares just those modules (`animation`, `config`, `convert`, `drop`, `power`, `shelf`, `tasks`, `timer`, `transforms`, `ui_state`, `media`, `perf`, `logging`, `fullscreen`) pointing at `crates/hytte/src`.

## Before a pull request

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Then try the states you touched by hand: drag and drop, focus and fullscreen suppression can only be verified on a real desktop.

Next: [3. Architecture](03-architecture.md)
