# 13. Testing and releasing

## Running the tests

```powershell
cargo test --workspace
```

Off Windows see [section 2](02-getting-started.md#working-from-macos-or-linux): you can type-check, lint and run `hytte-proto`'s tests, but not the `hytte` crate's. Tests are `#[cfg(test)] mod tests` blocks at the bottom of each file; there is no `tests/` folder.

## What is covered

| File | Tests cover |
|---|---|
| `proto/src/lib.rs`, `sys.rs`, `ports.rs` | Message round trip, validation, old messages still parsing, an `Ask` or `Cmd` never mistaken for a task, text trimmed on a character boundary, per-user pipe name; ancestor chains stop at pid 0, a loop or the depth limit; a reused pid isn't the listed process (`same_exe`), port filtering, no self-kill. |
| `hytte/src/ui_state/tests/` | Scene priority, expiry, shell thresholds, ignored commands, peeking, panels and paging, fullscreen break-through, agent CLIs left to hooks, timer card, mic lock, ambient rules, cross-fades, reduced motion, Ctrl+C not shown as an error, failed rows leaving with their terminal, task list scrolling. |
| `animation.rs`, `timer.rs`, `perf.rs` | Springs converge, are frame-rate independent, keep velocity on retarget; timer sleep gaps, pause, rounds, persistence, alarm WAV; percentile maths. |
| `convert.rs`, `transforms.rs`, `shelf.rs` | Compress limits, PDF structure and xref offsets, JPEG pass-through; never overwriting, JSON / YAML, image clean, QR, base64, Markdown, chip wrapping; shelf add / dedupe / remove / copy staging. |
| `config.rs`, `setup.rs` | Editing a key keeps comments, hotkey parsing, quiet hours across midnight, `config::problem` naming lines and typos; PATH edits, idempotent profile blocks (above Starship), `notch` migration, encodings. |
| `update.rs` | Version tags compare numerically, junk tags rejected, release JSON yields the `hytte.exe` asset (none is an error), the release-notes first line. |
| `fullscreen.rs` | Allow / deny precedence, shell windows never fullscreen, maximised hides while borderless or shell-reported fullscreen is a game, browsers and players get the thin line, overlays keep state, snapping geometry. |
| `cli/*`, `agents.rs`, `secrets.rs`, `logging.rs` | Agent message shape, `--ack`, tool phrases, ask text and decision schema; hook merging that keeps user hooks and is idempotent, `--undo` restoring exactly; progress scanning (OSC 9;4, `\r`, bounded buffers); exit codes, embedded scripts call `hytte`; `PATH`/`PATHEXT` lookup, quoting, cut labels; doctor report; secret masking and a fuzz run; at most three relaunches in ten minutes. |
| `hog.rs`, `downloads.rs`, `calendar.rs`, `power.rs`, `proc.rs`, `media.rs`, `privacy.rs` | CPU-over-time and once-only alerts; Chrome and Firefox partial-file sequences and cancels; meeting links (lookalike hosts refused), next-event choice; safe reads and text; app ids; art decoding; key → app name. |
| `window/win.rs` | `paste_step`, the decision behind delete-screenshot-after-paste (grace period, cancel on other copy, held Ctrl+V, none in game mode, ten-minute limit). |

New in 1.7.0 (the panes release): `textedit` (caret, selection, joined emoji), `jot` (expiry, undo, a damaged file), `weather` (parsers on sample replies, every weather code), `net` (encoding), `lyrics` (LRC, replies, a different master), `sleep` (the timer, finding the player, the fade curve), `breaks` (every reminder rule), `skin`, `dnd` (the marker text) and the new `ui_state` tests (holds, nudges, lyrics requests, panes, pane order, the wider place search). The ignored test `weather::wide_finds_sri_city` asks OpenStreetMap for a place Open-Meteo lacks. The ignored tests `dnd::real_switch_round_trips`, `sleep::manual_sessions` and `sleep::manual_fade` use the real Windows setting, audio sessions and a real player (`HYTTE_SLEEP_APP=Spotify.exe`). Snapshot tests `jot_snapshots`, `weather_snapshots`, `focus_snapshots`, `media_snapshots` and `skin_snapshots` draw the new panes in every state.

Ignored tests (run with `-- --ignored`): `render/snap.rs` draws the scenes offscreen (WARP, 200 %, both themes where it matters) into PNGs plus an `index.html` contact sheet, for checking UI changes without running the pill: `HYTTE_SNAP_DIR=<dir> cargo test -p hytte snapshots -- --ignored` (also runs `settings_window_snapshots`; the screenshots in `docs/img` come from these). Normal tests there check every settings-table row writes a key `config.toml` accepts, no key appears twice, and every reset reads back as the default. Manual tests: `mic` (`lists_live_apps`, and `mute_change_wakes_the_watcher`, which flips the real mute and restores it), `calendar_manual_create` / `_dump` / `_delete` (a throw-away calendar), `tabs` (`HYTTE_WT_PID=<pid> cargo test -p hytte selects_other_tab -- --ignored`, needs Windows Terminal with two tabs).

### Not covered automatically

Anything needing a real desktop: drawing, hover timing, drag and drop, focus, fullscreen detection, the tray, virtual desktops, media sessions, OCR, shell scripts in real shells, `hytte.exe`'s console behaviour, quitting cleanly, and the update flow ([section 15](15-updates.md#testing-an-update)). Check by hand before a release using [section 2](02-getting-started.md#putting-the-pill-into-every-state).

### Writing tests

- **Put logic where it can be tested:** pure functions or `Model` (no Win32), like `fullscreen::decide` and `ports::filter`, with OS inputs as parameters and a thin OS wrapper around them.
- **Inject time:** model methods take `now: Instant` so tests can jump forward.
- **Use temp directories** (`std::env::temp_dir().join("hytte-…")`) and clean up.

## Pre-release checklist

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
2. A manual pass over every scene, on a high-DPI monitor if you have one (the snapshot test is a quick first look).
3. An idle check ([section 12](12-performance.md#measuring)): about 0 % CPU when collapsed.
4. `scripts/release.ps1 <version>` (below): bumps the version everywhere, runs step 1 and a release build, commits and tags. The tag must match: the release job refuses a tag whose exe reports another version, since the in-app updater would refuse that download too.

## How a release is built

Two workflows in `.github/workflows/`, with a Rust cache:

- **`ci.yml`** on every push and pull request to `main`: `scripts/version.ps1 check` (same version in both `Cargo.toml` files, `Cargo.lock`, the winget manifest and the Scoop manifest), fmt, clippy, tests, an `arm64` job (clippy, tests, a release build and `--version` on a real `windows-11-arm` runner) and a separate `audit` job (`cargo audit`). Both workflows are read-only (`contents: read`) except jobs that publish or push, and every action is pinned by commit SHA (tag in a comment): bump them on purpose. `rust-toolchain.toml` pins the x64 host, so the ARM jobs set `RUSTUP_TOOLCHAIN=stable-aarch64-pc-windows-msvc`.
- **`release.yml`** on a pushed `v*` tag, or by hand (**Run workflow** with a tag; GitHub creates the tag on publish):
  1. the `arm64` job builds and tests on the ARM runner, checks `--version` says the tag's version and hands `hytte-arm64.exe` to the next job;
  2. the `build` job: `version.ps1 check <tag>` (a mismatching tag fails before building), then fmt, clippy and tests, then `cargo build --release --workspace`; check `hytte.exe --version` says the tag's version;
  3. zip into `hytte-<version>-x64.zip` and `hytte-<version>-arm64.zip` (each holds `hytte.exe`);
  4. create the GitHub release "Hytte v<version>" with four assets: the two zips, for people, winget and Scoop, and the bare **`hytte.exe`** (x64) and **`hytte-arm64.exe`**, which the in-app updater downloads. Release notes are generated from the commits.

A **dry run** (*dry_run* ticked) does everything except publishing and committing.

After publishing, a second job (`manifests`) downloads the published zips, writes their SHA-256 into `manifests/winget/Hytte.Hytte.yaml` and `bucket/hytte.json` (`scripts/version.ps1 hash`, once per zip) and commits that to `main` as "manifests: hashes for X". Pull before the next release.

### Cutting a release

```powershell
scripts/release.ps1 1.4.0          # bump, check, commit "Release 1.4.0", tag v1.4.0: nothing pushed
git push origin main v1.4.0        # starts the release workflow
```

`release.ps1` refuses unless the tree is clean, you're on `main`, `main` equals `origin/main` and the tag doesn't exist. `-SkipChecks` skips local checks (CI repeats them); `-Push` pushes for you. `scripts/version.ps1` has `check [tag]`, `set <version>` (the winget and Scoop hashes become zeros until the release job fills them) and `hash <zip>`.

Releases before the single-exe version were renumbered `v0.1.0`–`v0.4.0`; `v1.0.0` is the first merged `hytte.exe`.

## winget

`manifests/winget/Hytte.Hytte.yaml` is a single-file reference manifest for a **portable zip** install. It exposes `hytte` as the command alias and requires Windows 11 22H2 (`10.0.22621.0`). Version, `InstallerUrl` and `InstallerSha256` are kept by the release tooling, so nothing is edited by hand. Before submitting to `winget-pkgs`, split it into the version / installer / locale files it expects. Microsoft runs a Defender scan and a CLA check on every submission; an unsigned exe can be flagged as a false positive, which is cleared by submitting the installer to Microsoft Defender as incorrectly detected.

## Scoop

`bucket/hytte.json` is a Scoop manifest for both architectures, with `checkver` and `autoupdate` so the community tooling can bump it too. Use it straight from a checkout:

```powershell
scoop bucket add hytte C:\path	o\Hytte
scoop install hytte/hytte
```

Version, urls and hashes are kept by the release tooling like the winget manifest's. How Hytte behaves when installed this way is in [section 15](15-updates.md#scoop-installs).

## ARM64

Releases carry a native ARM64 build (`hytte-arm64.exe`, `hytte-<v>-arm64.zip`). The x64 asset names don't change, so every older install keeps updating. It is built and tested on GitHub's `windows-11-arm` runner and compiles from the same code; no ARM-only code exists. It has not been run on physical hardware by the maintainer.

## Code signing

Binaries are unsigned, so SmartScreen may warn on first run. Signing would be a release-workflow step between building and zipping.
