# 2. Getting started

## Prerequisites

- **Windows 11 22H2 or newer, x64.** Hytte uses APIs that don't exist on Windows 10, and there are no fallbacks.
- **Rust stable with the MSVC toolchain.** `rust-toolchain.toml` pins `stable-x86_64-pc-windows-msvc`, so `rustup` installs the right one automatically the first time you build.
- **Visual Studio Build Tools** (the "Desktop development with C++" workload) for the MSVC linker.
- Optional: an OCR language pack, to exercise the "Read text" action.

## Build

```powershell
cargo build --workspace            # debug build
cargo build --release --workspace  # optimised build (LTO; slower to compile)
```

The one program, `hytte.exe`, lands in `target\debug\` or `target\release\`. The build also compiles `crates/hytte/hytte.rc` (icons, manifest, version info) with the Windows SDK's resource compiler, which comes with the Build Tools.

## Run

```powershell
cargo run -p hytte                                # start the daemon
cargo run -p hytte -- run -- cmd /c "exit 3"      # CLI mode: send it a failing task
```

Only one daemon can run per user session (`single_instance.rs` holds the named mutex `Local\HytteSingleInstance`). If you already have an installed Hytte running, quit it from the tray first, or your dev build exits immediately with "another instance is running".

`hytte run` starts the daemon automatically (this same exe, detached) if the pipe isn't answering.

To try a dev build without touching your real settings, point `APPDATA` somewhere else first. Every file Hytte writes lives under `%APPDATA%\Hytte` ([section 11](11-configuration-and-files.md)). Also set `add_to_path = false` in that copy's `config.toml`, so the dev folder isn't put on your `PATH`:

```powershell
$env:APPDATA = "$env:TEMP\hytte-dev"; .\target\release\hytte.exe
```

### Putting the pill into every state

You rarely need a real workload to test a scene:

```powershell
$n = "target\debug\hytte.exe"
& $n run -- cmd /c "echo boom 1>&2 & exit 3"           # failed task with stderr
& $n run -- powershell -c "Start-Sleep 8"               # running task (spinner)
& $n set --progress 40 --label "Uploading" --task up1   # determinate progress
& $n agent needs-input --name "Claude Code" --message "Approve edit?"   # amber
& $n agent resume --name "Claude Code"                  # back to running
python -m http.server 3000                              # appears in the Ports panel
```

For the rest: drag a file onto the pill (shelf, Drop Vault), play music in any app (media), right-click the pill (timer), and use the Home card's **Mute** button (mic lock).

## Debugging

In daemon mode `hytte.exe` has **no console** (see [section 1](01-overview.md#one-program-two-modes)), so `println!` and `eprintln!` output goes nowhere. Use these instead:

| Tool | How |
|---|---|
| Log file | `crate::logging::line("...")` appends to `%APPDATA%\Hytte\hytte.log`. Panics from any thread are written there too, by the hook installed in `logging::init`. |
| Frame telemetry | Set `HYTTE_PERF=1` before starting the daemon. Every 240 animated frames, `perf.rs` logs p50 / p99 / max draw+present cost and draw-only cost, the gap between frames, and how many frames missed a vblank. |
| A debugger | Visual Studio, WinDbg or VS Code with the C++ debugger can attach to `hytte.exe`. Debug builds include symbols in the `.pdb` next to the exe. |

In CLI mode it is a normal console program, so its errors print to the terminal.

### Debugging tips

- The UI thread state lives in a thread-local (`window.rs`, `UI`). A breakpoint in `Ui::frame` fires on every frame while anything moves. Prefer `Ui::handle_events` or `Ui::after_model_change` to catch state changes.
- Mouse hover depends on timers (`T_DWELL`, `T_COLLAPSE`). Stopping in the debugger while hovering will make the pill open or close unexpectedly when you resume.
- If the pill seems "stuck", check the tray **Pause** item and whether a fullscreen window is in front (the sentinel bar).

## Working from macOS or Linux

The daemon only runs on Windows, but you can still **type-check and lint** the full Windows build from another OS:

```bash
rustup target add x86_64-pc-windows-msvc
# rust-toolchain.toml names the Windows-host toolchain; override it on other hosts
export RUSTUP_TOOLCHAIN=stable
cargo check  --workspace --all-targets --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo fmt --all --check
cargo test -p hytte-proto        # the shared crate's tests run on any OS
```

`cargo check` does not need the MSVC linker, so this works anywhere. What you **can't** do on a non-Windows host:

- Link or run `hytte.exe`.
- Run the `hytte` crate's tests. They don't compile for a non-Windows host, because COM `#[implement]` types pull in Windows-only parts of `windows-core`.

If you need to run the platform-independent tests (springs, UI model, shelf, conversions, timer, ...) off Windows, create a throwaway crate whose `main.rs` declares just those modules (`animation`, `config`, `convert`, `drop`, `power`, `shelf`, `tasks`, `timer`, `transforms`, `ui_state`, `media`, `perf`, `logging`, `fullscreen`) pointing at the files in `crates/hytte/src`. Those modules keep their Windows-only code behind `#[cfg(windows)]`.

## Before you open a pull request

Run the same checks the codebase is kept clean against:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Then try the states you touched by hand (see the commands above). Several features, such as drag and drop, focus behaviour and fullscreen suppression, can only be verified on a real desktop.

Next: [3. Architecture](03-architecture.md)
