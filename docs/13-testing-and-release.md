# 13. Testing and releasing

## Running the tests

On Windows:

```powershell
cargo test --workspace
```

On other operating systems, see [section 2](02-getting-started.md#working-from-macos-or-linux). You can type-check and lint everything, and run `hytte-proto`'s tests, but not the `hytte` crate's tests.

Tests are ordinary `#[cfg(test)] mod tests` blocks at the bottom of each file, next to the code they cover. There is no separate `tests/` folder.

## What is covered

| File | Tests cover |
|---|---|
| `proto/src/lib.rs` | Message round trip, validation, old messages still parsing, bad progress rejected. |
| `proto/src/ports.rs` | Port filtering rules (system exes, duplicates, watch list, no lookups for unwatched ports); on Windows, seeing its own listener and refusing to kill itself. |
| `hytte/src/ui_state.rs` | Scene priority, expiry, shell thresholds and ignored commands, attention peeking, panels and the wheel, fullscreen break-through on drops, the timer card, the mic lock, ambient-frame rules, the primary task, scene cross-fades, reduced motion. |
| `hytte/src/animation.rs` | Springs converge, settle and snap; frame-rate independence; velocity kept on retarget; collapse slower than expand and without overshoot. |
| `hytte/src/convert.rs` | Compress hits its size limit by quality then by scaling; small files are skipped; transparency flattens to white; PDF structure and xref offsets; page fitting; PNG → PDF; JPEG pass-through from the header. |
| `hytte/src/transforms.rs` | Never overwriting, JSON toggling, YAML normalising, image clean + WebP, chips per file type. |
| `hytte/src/config.rs` | Editing `autostart` keeps comments and other keys, adds a missing key or section, and leaves invalid TOML alone. |
| `hytte/src/setup.rs` | PATH append / remove (case, quotes, trailing slash, `%VAR%`), profile blocks (idempotent, round trip, above Starship), profile encodings. |
| `hytte/src/shelf.rs` | Adding, de-duplication, removing (reference never deletes the original; owned copies are deleted), copy-mode staging and the limit re-check on insert. |
| `hytte/src/timer.rs` | Surviving a sleep gap, pause and resume, extending and formatting, long-break spacing, wheel steps, persistence, the chime WAV. |
| `hytte/src/media.rs` | Album art decoding and premultiplication. |
| `hytte/src/privacy.rs` | Registry key → app name. |
| `hytte/src/fullscreen.rs` | Allow / deny list precedence. |
| `hytte/src/power.rs` | Reading is safe on any machine; status text. |
| `hytte/src/proc.rs` | App ids → exe stems; own process alive and named. |
| `hytte/src/perf.rs` | Percentile maths. |
| `hytte/src/cli/agent.rs` | Agent message shape; the Claude hooks JSON is valid. |
| `hytte/src/cli/hook.rs` | Exit code mapping; scripts are embedded. |
| `hytte/src/tabs.rs` | One **ignored** manual test that needs a Windows Terminal with two tabs: `HYTTE_WT_PID=<pid> cargo test -p hytte selects_other_tab -- --ignored`. |

### What isn't covered by automated tests

Anything that needs a real desktop: drawing, hover timing, drag and drop in and out, focus behaviour, fullscreen detection, the tray, virtual desktops, media sessions, OCR, and the shell scripts inside real shells. Check these by hand before a release, using the commands in [section 2](02-getting-started.md#putting-the-pill-into-every-state).

### Writing tests

- **Put logic where it can be tested.** Decisions belong in pure functions or in `Model` (no Win32), like `fullscreen::decide` and `ports::filter`, which take their OS inputs as parameters. Keep the OS calls in a thin wrapper around them.
- **Inject time.** Model methods take `now: Instant` instead of reading the clock, so tests can jump forward (`t0 + Duration::from_secs(4)`).
- **Use temp directories** (`std::env::temp_dir().join("hytte-…")`) for file tests, and clean up anything the test creates.

## Pre-release checklist

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. A manual pass over every scene ([section 2](02-getting-started.md#putting-the-pill-into-every-state)), on a high-DPI monitor if you have one.
5. An idle check ([section 12](12-performance.md#cpu-and-memory)): about 0 % CPU when collapsed.
6. Bump the version in `crates/proto/Cargo.toml` and `crates/hytte/Cargo.toml` (they share a version), and in `manifests/winget/Hytte.Hytte.yaml`.

## How a release is built

`.github/workflows/release.yml` runs when a tag matching `v*` is pushed:

1. Checks out the repository on `windows-latest`, with a Rust build cache.
2. `cargo build --release --workspace`.
3. Zips `hytte.exe` and `notch.exe` into `hytte-<version>-x64.zip` (the tag without its leading `v`).
4. Creates a GitHub release for the tag with the zip attached and generated release notes.

```powershell
git tag v1.2.0
git push origin v1.2.0
```

## winget

`manifests/winget/Hytte.Hytte.yaml` is a single-file reference manifest for a **portable zip** install. It exposes `hytte` and `notch` as command aliases and requires Windows 11 22H2 (`10.0.22621.0`). Before submitting to `winget-pkgs`:

- replace the placeholder `InstallerUrl` with the release asset URL, and add its SHA-256;
- split it into the version / installer / locale files that `winget-pkgs` expects.

## Code signing

Binaries are currently unsigned, so SmartScreen may warn on first run. Signing would be a step in the release workflow, between building and zipping.

Next: [14. How-to recipes](14-how-to-recipes.md)
