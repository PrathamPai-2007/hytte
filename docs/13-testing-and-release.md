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
| `proto/src/lib.rs` | Message round trip, validation, old messages still parsing, bad progress rejected, an `Ask` is never mistaken for a task message. |
| `proto/src/sys.rs` | Ancestor chains walk up from each pid and stop at pid 0, a loop or the depth limit. |
| `proto/src/ports.rs` | Port filtering rules (system exes, duplicates, watch list, no lookups for unwatched ports); on Windows, seeing its own listener and refusing to kill itself. |
| `hytte/src/ui_state.rs` | Scene priority, expiry, shell thresholds and ignored commands, attention peeking, panels and paging (a listening port alone brings up the Tasks panel), fullscreen break-through on drops and agent asks, agent CLIs left to their hooks, the timer card, the mic lock, ambient-frame rules, the primary task, scene cross-fades, reduced motion, a command stopped with Ctrl+C is not shown as an error, a failed row goes when its terminal closes (and a running one does not), and the task list scrolls past its row limit. |
| `hytte/src/animation.rs` | Springs converge, settle and snap; frame-rate independence; velocity kept on retarget; collapse slower than expand and without overshoot. |
| `hytte/src/convert.rs` | Compress hits its size limit by quality then by scaling; small files are skipped; transparency flattens to white; PDF structure and xref offsets; page fitting; PNG → PDF; JPEG pass-through from the header. |
| `hytte/src/transforms.rs` | Never overwriting, JSON toggling, YAML normalising, image clean + WebP, chips per file type. |
| `hytte/src/config.rs` | Editing `autostart` keeps comments and other keys, adds a missing key or section, and leaves invalid TOML alone; hotkey parsing; a quiet-hours window that wraps past midnight (and bad values mean never); the note "Custom" puts above a key goes in once; `config::problem` names the line of a mistake and the key of a typo, says nothing about a default file or the old keys older versions wrote, and is clean on this machine's own file (an ignored test). |
| `hytte/src/setup.rs` | PATH append / remove (case, quotes, trailing slash, `%VAR%`), profile blocks (idempotent, round trip, above Starship), migrating a pre-1.0 `notch` block exactly once, profile encodings. |
| `hytte/src/update.rs` | Version tags compare numerically (`2.0.10` > `2.0.9`), junk tags are rejected, the release JSON yields the `hytte.exe` asset and a release without one is an error, and the first real line of a release's notes is picked out for the prompt. |
| `hytte/src/shelf.rs` | Adding, de-duplication, removing (reference never deletes the original; owned copies are deleted), copy-mode staging and the limit re-check on insert. |
| `hytte/src/timer.rs` | Surviving a sleep gap, pause and resume, extending and formatting, long-break spacing, wheel steps, persistence, the alarm WAV (valid, and soft: well under full scale), focus round dots. |
| `hytte/src/media.rs` | Album art decoding and premultiplication. |
| `hytte/src/privacy.rs` | Registry key → app name. |
| `hytte/src/fullscreen.rs` | Allow / deny list precedence; the desktop and other shell windows are never treated as fullscreen; a maximised window (frameless included, and with the taskbar visible) only hides, while borderless or shell-reported fullscreen is a game, even when it went fullscreen from maximised; the system-wide exclusive-fullscreen flag only counts for a foreground that covers its monitor unmaximised; a fullscreen browser or media player gets the thin line; overlays keep the current state; another monitor only matters for games; a snapped window gets the thin line unless it is on another monitor or on the allow list; the snapping geometry (`is_snapped`) for halves, thirds and quarters of a 1440 x 900 screen, floating windows and a top taskbar. |
| `hytte/src/power.rs` | Reading is safe on any machine; status and time-left text. |
| `hytte/src/proc.rs` | App ids → exe stems; own process alive and named. |
| `hytte/src/perf.rs` | Percentile maths. |
| `hytte/src/cli/agent.rs` | Agent message shape; `--ack` is a flag without a value; tool calls become short phrases; ask text flags multi-line and long commands up front; the permission decision matches Claude Code's schema (`message` only with a deny). |
| `hytte/src/agents.rs` | Merging hooks keeps the user's own hooks and key order and is idempotent, and `--undo` restores the file exactly; a new exe path replaces old entries; Codex and Cursor files come and go; Antigravity's named hook; the OpenCode plugin is written and removed; quoted paths with spaces; files that aren't plain JSON are left alone. |
| `hytte/src/cli/progress.rs` | OSC 9;4 with BEL and ST terminators, sequences split across reads, `\r` bars, implausible numbers ignored, bounded buffers. |
| `hytte/src/cli/hook.rs` | Exit code mapping; scripts are embedded and call `hytte`, not the old `notch`. |
| `hytte/src/hog.rs` | How long a process must stay over the CPU limit, once-only alerts that re-arm after a dip, memory alerting at once, exemptions. |
| `hytte/src/downloads.rs` | Partial-file names; Chrome's rename sequence from start to finish; Firefox's `.part`; cancels; sizes. |
| `hytte/src/calendar.rs` | WinRT time conversion, meeting-link extraction (including lookalike hosts and non-https), picking the next event, sleep timing, summaries. Three ignored manual tests (`calendar_manual_create`, `calendar_manual_dump`, `calendar_manual_delete`) create, list and delete a throw-away calendar. |
| `hytte/src/render.rs` | One **ignored** snapshot test that draws the scenes offscreen (a WARP device context into a bitmap) at 200 % (timer setup, running, paused and finished; media; power; home; tasks with linked dev servers; shelf; a card; the drop zone; idle and the sentinel; every pane with the pointer on a tab; a scrolled shelf; the timeline under the pointer; some in light theme too) and writes one PNG each into a folder, plus an `index.html` contact sheet of all of them, for checking UI changes without running the pill: `HYTTE_SNAP_DIR=<dir> cargo test -p hytte snapshots -- --ignored`. That filter also runs `settings_window_snapshots`, which draws the big settings window (dark and light, at the top, scrolled, with the pointer on a row, and narrow); the screenshots in `docs/img` come from these. Normal (not ignored) tests there also check that every row of the settings table writes a key `config.toml` accepts, that no key appears twice, and that every row's reset writes a default that reads back as the default. The same test file draws the search results, a search with no match, a long title fading out, and the optional pin. |
| `hytte/src/window.rs` | `paste_step`, the decision behind "delete a screenshot after it is pasted": a Ctrl+V press starts a two second grace and then deletes, a copy of anything else cancels the watch, a Ctrl+V already held when the screenshot arrived is not a paste until pressed again, nothing is polled in game mode, and a forgotten screenshot is let go after ten minutes. |
| `hytte/src/mic.rs` | Grouping of an app's sessions, playing first; and one **ignored** manual test that lists the real apps (`lists_live_apps`); another that flips the real microphone mute and expects the endpoint callback to report it: `cargo test -p hytte mic -- --ignored`. It restores the mute afterwards. |
| `hytte/src/tabs.rs` | One **ignored** manual test that needs a Windows Terminal with two tabs: `HYTTE_WT_PID=<pid> cargo test -p hytte selects_other_tab -- --ignored`. |

### What isn't covered by automated tests

Anything that needs a real desktop: drawing, hover timing, drag and drop in and out, focus behaviour, fullscreen detection, the tray (and its icon following the taskbar theme), virtual desktops, media sessions, OCR, the shell scripts inside real shells, the console behaviour of `hytte.exe` (double-click, typed in a terminal), quitting cleanly, and the update flow ([section 15](15-updates.md#testing-an-update)). Check these by hand before a release, using the commands in [section 2](02-getting-started.md#putting-the-pill-into-every-state).

### Writing tests

- **Put logic where it can be tested.** Decisions belong in pure functions or in `Model` (no Win32), like `fullscreen::decide` and `ports::filter`, which take their OS inputs as parameters. Keep the OS calls in a thin wrapper around them.
- **Inject time.** Model methods take `now: Instant` instead of reading the clock, so tests can jump forward (`t0 + Duration::from_secs(4)`).
- **Use temp directories** (`std::env::temp_dir().join("hytte-…")`) for file tests, and clean up anything the test creates.

## Pre-release checklist

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. A manual pass over every scene ([section 2](02-getting-started.md#putting-the-pill-into-every-state)), on a high-DPI monitor if you have one. The `render.rs` snapshot test (above) is a quick first look at every pane, in both themes where it matters.
5. An idle check ([section 12](12-performance.md#cpu-and-memory)): about 0 % CPU when collapsed.
6. Cut the release with `scripts/release.ps1 <version>` (below). It bumps the version everywhere it lives, runs steps 1 to 3 and a release build, commits, and tags. The tag must match: the release job refuses a tag whose exe reports a different version, because the in-app updater would refuse that download too.

## How a release is built

Two workflows live in `.github/workflows/`. Both run on `windows-latest` with a Rust build cache:

- **`ci.yml`** runs on every push and pull request to `main`: `scripts/version.ps1 check` (the version is the same in both `Cargo.toml` files, in `Cargo.lock` and in the winget manifest), `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- **`release.yml`** runs when a tag matching `v*` is pushed, or by hand (**Run workflow** with a tag; GitHub creates the tag when the release is published). It:
  1. runs `version.ps1 check <tag>` (so a tag that doesn't match the files fails before anything is built), then the same three checks;
  2. `cargo build --release --workspace`;
  3. checks that `hytte.exe --version` says the tag's version;
  4. zips `hytte.exe` into `hytte-<version>-x64.zip` (the tag without its leading `v`);
  5. creates the GitHub release "Hytte v<version>" with two assets: the zip, for people, and the bare **`hytte.exe`**, which is what the in-app updater downloads ([section 15](15-updates.md)). Release notes are generated from the commits.

A **dry run** (**Run workflow** with *dry_run* ticked) does all of the above except publishing and committing, to rehearse a release.

After the release is published, a second job (`winget`) downloads the published zip, writes its SHA-256 into `manifests/winget/Hytte.Hytte.yaml` (`scripts/version.ps1 hash`) and commits that to `main` as "winget: InstallerSha256 for X". Pull before the next release. (The job refuses to run if the manifest on `main` is for a different version than the tag.)

### Cutting a release

```powershell
scripts/release.ps1 1.4.0          # bump, check, commit "Release 1.4.0", tag v1.4.0: nothing is pushed
git push origin main v1.4.0        # this starts the release workflow
```

`release.ps1` refuses to run unless the tree is clean, you are on `main`, `main` is level with `origin/main` and the tag doesn't exist. `-SkipChecks` skips the local checks (CI repeats them), and `-Push` pushes for you. `scripts/version.ps1` has three commands: `check [tag]`, `set <version>` (the winget hash becomes a placeholder of zeros until the release job fills it in) and `hash <zip>`.

Release history: the releases made before the single-exe version were renumbered `v0.1.0` to `v0.4.0`, and `v1.0.0` is the first release of the merged `hytte.exe`.

## winget

`manifests/winget/Hytte.Hytte.yaml` is a single-file reference manifest for a **portable zip** install. It exposes `hytte` as the command alias and requires Windows 11 22H2 (`10.0.22621.0`). Its version, `InstallerUrl` and `InstallerSha256` are kept by the release tooling above, so nothing in it is edited by hand. Before submitting to `winget-pkgs`, split it into the version / installer / locale files that `winget-pkgs` expects.

## Code signing

Binaries are currently unsigned, so SmartScreen may warn on first run. Signing would be a step in the release workflow, between building and zipping.

Next: [14. How-to recipes](14-how-to-recipes.md)
