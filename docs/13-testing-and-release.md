# 13. Testing and releasing

## Running the tests

```powershell
cargo test --workspace
```

Off Windows see [section 2](02-getting-started.md#working-from-macos-or-linux): you can type-check, lint and run `hytte-proto`'s tests, but not the `hytte` crate's. Tests are `#[cfg(test)] mod tests` blocks at the bottom of each file (the UI model's are in `ui_state/tests/`); there is no top-level `tests/` folder.

`cargo test -p hytte` runs 425 tests and leaves 36 ignored (the manual and snapshot tests below); `hytte-proto` has 19. These counts drift: treat them as a rough size, not a rule.

## What is covered

| File | Tests cover |
|---|---|
| `proto/src/lib.rs`, `sys.rs`, `ports.rs` | Message round trip, validation, old messages still parsing, an `Ask` or `Cmd` never mistaken for a task, text trimmed on a character boundary, per-user pipe name; ancestor chains stop at pid 0, a loop or the depth limit; a reused pid isn't the listed process (`same_exe`), port filtering, no self-kill. |
| `hytte/src/ui_state/tests/` | Scene priority, expiry, shell thresholds, ignored commands, peeking, panels and paging, fullscreen break-through, agent CLIs left to hooks, timer card, mic lock, ambient rules, cross-fades, reduced motion, Ctrl+C not shown as an error, failed rows leaving with their terminal, task list scrolling; `panes.rs`: pane switches, order and the custom-pane slots; `scripts.rs`: script pane text and rows, footers, due times, action arming and busy per (action, arg), pack-scoped action names, a failed run's footer. |
| `scriptpane.rs`, `packs.rs` | Output cleaning (colour codes, `
` bars, six lines, 2048 bytes), JSON rows (limits, tones, progress clamp, valid action names, non-rows output staying text), buttons and `arg` cleaning, `HYTTE_ARG` reaching the command and nothing stale, a timed-out command leaving no process behind, `due`; pack loading (prefixed actions, defaults, every kind of mistake), an answer holding only for the file you saw, answers kept outside the pack, packs sharing the 12-pane limit behind config panes, and `remap_slots` carrying a switch and place to the new slot. |
| `animation.rs`, `timer.rs`, `perf.rs` | Springs converge, are frame-rate independent, keep velocity on retarget; timer sleep gaps, pause, rounds, persistence, alarm WAV; percentile maths and the perf line. |
| `ui_state/tests/scene.rs`, `render/tween.rs`, `render/film.rs`, `render/tokens.rs` | Scene layers (a reversal continues, quick changes keep fading, a fourth drops the faintest, reduced motion leaves one), the glow colour blend; tweens start at their target, forget what isn't drawn, settle, snap under reduced motion, spring back from a kick; every scripted transition is continuous; radii nest. |
| `render/snap.rs` | The cached background matches the painted one at 1x, 1.5x and 2x. |
| `window/win.rs` | Ambient strides for 60, 120, 144 and 240 Hz; a quick click shows pressed for 80 ms. |
| `ollama.rs` | Start Ollama: the tray app wins over the bare command, relative `PATH` entries are skipped, the wait ends on an answer and on a timeout. `live_start_ollama` (ignored) starts the real one. |
| `convert.rs`, `transforms.rs`, `shelf.rs` | Compress limits, PDF structure and xref offsets, JPEG pass-through; never overwriting, JSON / YAML, image clean, QR, base64, Markdown, chip wrapping; shelf add / dedupe / remove / copy staging. |
| `config.rs`, `setup.rs` | Editing a key keeps comments, hotkey parsing, quiet hours across midnight, `config::problem` naming lines and typos (including `script_pane.comand`); script panes and actions read with their limits, script switches stored by slot and only when off; PATH edits, idempotent profile blocks (above Starship), `notch` migration, encodings. |
| `update.rs` | Version tags compare numerically, junk tags rejected, release JSON yields the `hytte.exe` asset (none is an error), the release-notes first line. |
| `fullscreen.rs` | Allow / deny precedence, shell windows never fullscreen, maximised hides while borderless or shell-reported fullscreen is a game, browsers and players get the thin line, overlays keep state, snapping geometry. |
| `position.rs`, `ui_state/day.rs` | The offset clamp (centred default, never off-screen, a bubble costs room on the right only, never an empty range), fraction round trip across resolutions, the drag threshold; the day totals (phases add up, a finished turn stops counting, midnight starts over, a running turn carries on) and the line's wording. |
| `cli/mcp_panes.rs`, `agent_guard.rs`, `registry.rs` | Agent-made panes are valid unanswered packs, edits change the hash, your panes are never touched, the four-pane cap; the rate limit and an unforgeable log line; SHA-256 vectors, install / tamper / update / remove / index rejection ([section 18](18-agent-control-and-registry.md#testing)). |
| `cli/mcp.rs` | The handshake and tool list, a waited-on card returning the pressed label (and "no answer"), bad input as a tool error with nothing sent, notes, to-dos and panes reaching the pill, through a fake `Backend`. |
| `cli/*`, `agents.rs`, `secrets.rs`, `logging.rs` | Agent message shape, `--ack`, tool phrases, ask text and decision schema; hook merging that keeps user hooks and is idempotent, `--undo` restoring exactly, Copilot's own hooks file, a Windows exe path seen from WSL, MCP registration read from each agent's file; progress scanning (OSC 9;4, `\r`, bounded buffers); exit codes, embedded scripts call `hytte`; `PATH`/`PATHEXT` lookup, quoting, cut labels; doctor report; secret masking and a fuzz run; at most three relaunches in ten minutes. |
| `hog.rs`, `downloads.rs`, `calendar.rs`, `power.rs`, `proc.rs`, `media.rs`, `privacy.rs` | CPU-over-time and once-only alerts; Chrome and Firefox partial-file sequences and cancels; meeting links (lookalike hosts refused), next-event choice; safe reads and text; app ids; art decoding; key → app name. |
| `window/win.rs` | `paste_step`, the decision behind delete-screenshot-after-paste (grace period, cancel on other copy, held Ctrl+V, none in game mode, ten-minute limit). |

New in 1.7.0 (the panes release): `textedit` (caret, selection, joined emoji), `jot` (expiry, undo, a damaged file), `weather` (parsers on sample replies, every weather code), `net` (encoding), `lyrics` (LRC, replies, a different master), `sleep` (the timer, finding the player, the fade curve), `breaks` (every reminder rule), `skin`, `dnd` (the marker text) and the new `ui_state` tests (holds, nudges, lyrics requests, panes, pane order, the wider place search). The ignored test `weather::wide_finds_sri_city` asks OpenStreetMap for a place Open-Meteo lacks. The ignored tests `dnd::real_switch_round_trips`, `sleep::manual_sessions` and `sleep::manual_fade` use the real Windows setting, audio sessions and a real player (`HYTTE_SLEEP_APP=Spotify.exe`). Snapshot tests `jot_snapshots`, `weather_snapshots`, `focus_snapshots`, `media_snapshots` and `skin_snapshots` draw the new panes in every state.

Motion review (ignored): `HYTTE_SNAP_DIR=<dir> cargo test filmstrips -- --ignored` writes frame-by-frame strips of scripted transitions and `motion.html` to play them ([section 8](08-rendering-and-animation.md#springs)); `play_pause_morph_picture` and `glow_cost_spike` are one-off pictures and measurements.

Ignored tests (run with `-- --ignored`): `render/snap.rs` draws the scenes offscreen (WARP, 200 %, both themes where it matters) into PNGs plus an `index.html` contact sheet, for checking UI changes without running the pill: `HYTTE_SNAP_DIR=<dir> cargo test -p hytte snapshots -- --ignored` (also runs `settings_window_snapshots`; the screenshots in `docs/img` come from these). Normal tests there check every settings-table row writes a key `config.toml` accepts, no key appears twice, and every reset reads back as the default. Manual tests: `mic` (`lists_live_apps`, and `mute_change_wakes_the_watcher`, which flips the real mute and restores it), `calendar_manual_create` / `_dump` / `_delete` (a throw-away calendar), `tabs` (`HYTTE_WT_PID=<pid> cargo test -p hytte selects_other_tab -- --ignored`, needs Windows Terminal with two tabs).

### The smoke test and the perf gate

`scripts/smoke.ps1` starts a built `hytte.exe` for real and checks it from outside:

```powershell
cargo build --release -p hytte
./scripts/smoke.ps1                 # smoke + idle budgets, about 100 s
./scripts/smoke.ps1 -SkipPerf       # smoke only, a few seconds
./scripts/smoke.ps1 -Out smoke.json # also write the numbers
```

It runs an isolated second copy: its own `%APPDATA%` in a temp folder (autostart, PATH edits and update checks switched off) and its own pipe and single-instance mutex through the `HYTTE_INSTANCE` environment variable (letters and digits, 16 at most). So it is safe beside your own running Hytte, and it never touches your files, PATH or autostart. It checks that the daemon opens its pipe, that `--version`, `doctor`, `notify`, `card`, `pane set` and `note` reach it, that a note really reaches `jot.json`, that it is still running afterwards and that the log holds no panic. Before any of those commands (a pack waiting for Run leaves a card on the pill, which keeps it drawing and is not idle), it waits 45 s, samples CPU for 30 s and reads private memory, and compares startup time, idle CPU and private memory with the `limit` block of `scripts/perf-baseline.json`. Exit code 1 on any failure.

CI runs it as the `smoke` job (with the numbers uploaded as an artifact, and one retry because a shared runner can have a noisy minute) and as a smoke-only step in the `arm64` job. The limits are loose on purpose (1.5 s, 1 % of a core, 40 MB) so only a real regression trips them; the `baseline` block records what the last measurement saw (about 0.1 s, 0.3 %, 17 MB). When you change what is normal, measure again and edit the file in the same commit.

### Not covered automatically

Anything needing a real desktop beyond the smoke test above: drawing, hover timing, drag and drop, focus, fullscreen detection, the tray, virtual desktops, media sessions, OCR, shell scripts in real shells, `hytte.exe`'s console behaviour, quitting cleanly, and the update flow ([section 15](15-updates.md#testing-an-update)). Check by hand before a release using [section 2](02-getting-started.md#putting-the-pill-into-every-state).

### Writing tests

- **Put logic where it can be tested:** pure functions or `Model` (no Win32), like `fullscreen::decide` and `ports::filter`, with OS inputs as parameters and a thin OS wrapper around them.
- **Inject time:** model methods take `now: Instant` so tests can jump forward.
- **Use temp directories** (`std::env::temp_dir().join("hytte-…")`) and clean up.

## Pre-release checklist

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
2. A manual pass over every scene, on a high-DPI monitor if you have one (the snapshot test is a quick first look).
3. `cargo build --release -p hytte`, then `./scripts/smoke.ps1` ([above](#the-smoke-test-and-the-perf-gate)): the built exe starts, answers, and idles within budget. It replaces the old hand idle check ([section 12](12-performance.md#measuring) says how to measure by hand).
4. `./scripts/pack-index.ps1 -Check` if you touched `examples/panes` ([section 18](18-agent-control-and-registry.md#publishing-a-pack)).
5. `scripts/release.ps1 <version>` (below): bumps the version everywhere, runs step 1 and a release build, commits and tags. The tag must match: the release job refuses a tag whose exe reports another version, since the in-app updater would refuse that download too.

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
scoop bucket add hytte C:\path\to\Hytte
scoop install hytte/hytte
```

Version, urls and hashes are kept by the release tooling like the winget manifest's. How Hytte behaves when installed this way is in [section 15](15-updates.md#scoop-installs).

## ARM64

Releases carry a native ARM64 build (`hytte-arm64.exe`, `hytte-<v>-arm64.zip`). The x64 asset names don't change, so every older install keeps updating. It is built and tested on GitHub's `windows-11-arm` runner and compiles from the same code; no ARM-only code exists. It has not been run on physical hardware by the maintainer.

## Code signing

Binaries are unsigned, so SmartScreen may warn on first run. Signing would be a release-workflow step between building and zipping.
