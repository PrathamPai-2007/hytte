# Implementation Plan: Hytte 1.8

## Overview

1.8 makes Hytte the place where your agents, your scripts and your machine report to you, and
cleans up the code underneath so later panes are cheap to add:

1. **Agent cockpit.** One Agents pane for every coding-agent session: state, current step, wait
   age, context used, files touched, subagents. Allow / Deny for more agents where their hooks allow.
2. **Hytte as a platform.** Script panes can show rows (text, value, progress, tone) instead of
   plain text; card buttons and pane rows can run **named actions** you wrote in `config.toml`;
   a pane can live in its own folder (a "pack") that you copy in yourself.
3. **System moments.** A short peek when a Bluetooth device connects (with its battery), and
   when an app starts using the camera or microphone.
4. **Clipboard.** Off by default: files you copy with Ctrl+C go on the shelf
   (`[shelf] shelve_copied`), and recent text clips show under the shelf tiles
   (`[clipboard] history`).
5. **Reach.** An ARM64 build and a Scoop bucket.
6. **Foundations.** `ui_state.rs` split into one module per pane, one layout description shared
   by sizing and drawing, one card queue, and the in-pill settings pane removed.

Release plumbing first (it finishes 1.7.1's leftover), then foundations (no visible change but
the settings pane), then the features on top. Work on a `v1.8` branch; nothing is pushed or tagged
without the user's go-ahead.

Tasks are in [todo.md](todo.md). The 1.7 plan is in git history.

## Decisions (user, 2026-11-10)

- **Volume and brightness: skipped.** Windows has no supported way to hide its own flyout, so
  showing ours as well would double up and hiding it needs an undocumented hack. Caps Lock is
  skipped too: it needs a low-level keyboard hook, which looks like a keylogger to antivirus
  software. Charging already has its card.
- **The in-pill settings pane is removed.** The gear on Home opens the settings window.
- **Clipboard history shows inside the Shelf pane**, not a pane of its own.
- **Platform stays local.** Named actions come only from `config.toml`; packs are folders you
  copy in. No installer, no `hytte pane add <url>`.
- Rejected earlier and not revived: any pill layout or navigation redesign (glance Home, live
  strip, idle face, backdrops), signing, a pet, notifications in the pill, a launch video.

## Assumptions (correct me before Task 1)

1. Version **1.8.0**; the welcome tour shows "What's new in 1.8" to updaters (marker `1.8`).
2. Agents leave the Tasks pane and live in the Agents pane; the compact pill is unchanged (it
   still shows the most urgent row, agent or command).
3. "Context used" is shown in tokens (`84k`), not a percentage: the transcript doesn't say the
   model's window size, and a wrong percentage is worse than none. Usage limits are **out of
   scope**: no hook reports them.
4. Ctrl+C auto-shelve always uses reference mode (never copies the file, whatever `[shelf] mode`
   says) and ignores Ctrl+X (a moved file would break the reference).
5. Clipboard history is never written to disk.
6. ARM64 is built and tested in CI on GitHub's `windows-11-arm` runner; nobody tests the GUI on
   real ARM hardware before release.

## Architecture decisions

### Foundations

- **Split `ui_state.rs` (4,234 lines) into `ui_state/`.** `mod.rs` keeps `Model`'s shared parts,
  `Scene`, scene choice and `UiEvent`; `anim.rs` holds `Anim` and `Swipe`; `cards.rs` holds `Chip`
  and the queue; one file per pane (`tasks`, `agents`, `media`, `shelf`, `home`, `jot`,
  `weather`, `timer`, `power`, `script`) holds that pane's state struct and its `size`, `glow`,
  `ambient` and `expire` parts. `Model` owns one struct per pane. Plain modules: the dispatch
  `match` on `Panel` stays in one place, and no `Pane` trait unless it deletes more code than it
  adds. Tests move with the code they test.
- **Behaviour stays identical**, proved two ways: every existing test passes unchanged except for
  its path, and every offscreen snapshot is byte-identical before and after.
- **One layout description.** Today a pane's height is a hand-written sum in `Model::size`
  (`12 + 34 + 8 + 30 per row`) and the same numbers are written again in the drawing code. Each
  pane gets one `layout(&Model) -> Rows` that both use: `size` sums the rows, the renderer walks
  them. Snapshots stay identical. Done pane by pane, Tasks and Notes first (the busiest sums).
- **One card queue.** `Chip` becomes a `VecDeque` with a priority: agent ask, timer finished,
  update offer, live card, alert (CPU, download, calendar), info. A higher card goes in front, a
  lower one waits; a waiting alert expires on its own hold. Nothing is dropped while another card
  shows (today a download or CPU alert arriving meanwhile is lost). At most 8 waiting; same key
  replaces. System moments and clipboard peeks use it.
- **In-pill settings pane removed.** `Scene::ExpSettings`, the pill's settings rendering and
  `SETTINGS_GROUPS` go; the gear (and keyboard Enter on it) opens the settings window. Pill skin
  and opacity stay in the settings window, which already has them. The manual, the welcome copy
  and screenshots are updated.

### Agent cockpit

- **Agents pane** (`Panel::Agents`, `[panes] agents`, on by default, shown while any agent
  session exists). One row per session: agent name and project folder, state (working, waiting,
  done), current step, how long it has waited, context tokens, files touched, subagents running.
  Up to four rows, the wheel scrolls. A click jumps to the agent's terminal (the existing jump).
- **Data from the hooks Hytte already installs.** `PreToolUse` gives the current step today; it
  now also counts distinct files from `Edit` / `Write` / `MultiEdit` ("files touched": a denied
  edit still counts, said so in the manual) and subagent launches (`Task` / `Agent` tool calls,
  reset on `Stop`). `Stop` and `PreToolUse` read the transcript's last assistant `usage`
  (input + cache read + cache creation tokens) for context used, from the last 512 KB as today.
  No new hooks, so nobody has to rerun setup.
- **Protocol:** optional fields on the agent message (`context`, `files`, `subagents`); old
  daemons ignore them, `old_messages_still_parse` keeps passing.
- **Done card** gains one line: "Edited 4 files · 84k context".
- **Allow / Deny for Gemini CLI, Cursor and Antigravity:** a research task first, against each
  agent's current hook docs. Built only for agents whose hook can block and return a decision;
  the others stay as they are and the manual's table says so.

### Platform

- **Rows in script panes.** If a pane's output (pulled or pushed) is a JSON object
  `{"rows":[{"text","value","progress","tone","action"}]}`, the pane draws up to six rows: text on
  the left, value on the right, an optional thin progress bar, a tone colour (ok, warn, error).
  Anything else is plain text, as in 1.7. `hytte pane set <name> --json`. Limits as today
  (2 KB, six rows, `secrets::mask` on every string).
- **Named actions.** `[[action]]` in `config.toml`: `name`, `command`, optional `confirm` (two
  clicks, like **Kill?**) and `timeout` (default 60 s, max 600). A card button
  (`hytte card --button Restart:restart-api`) or a row's `action` names one; Hytte runs it hidden
  with the script-pane runner (Job object, killed at the timeout) and shows a result card ("restart-api
  done" / "failed · exit 1" with the last line). **Nothing on the pipe can supply a command**: an
  unknown action name is ignored. A button without an action still only reports back, as in 1.7.
- **Packs.** A folder `%APPDATA%\Hytte\panes\<name>\` with `pane.toml` (`name`, `command` relative
  to the folder, `every`, and its own `[[action]]`s, named `<pack>/<action>`). Loaded at start and
  on change like `config.toml`; together with `[[script_pane]]` still at most four panes; each has
  a switch in `[panes]`. Hytte shows a one-time card naming a newly found pack and its command
  before running it the first time (**Run** / **Ignore**).
- **Examples in the repo:** `examples/panes/` with three packs (GitHub pull requests via `gh`,
  Docker containers, CI status), linked from the manual. Not a gallery service.

### System moments

- **Bluetooth.** A new worker (`bluetooth.rs`) watches device connections with WinRT
  `DeviceWatcher` (events, no polling) and reads battery level where the device reports it
  (`System.Devices.Aep.Bluetooth.Le.Battery…` / the GATT Battery Service for LE devices). A
  connect shows a small card: icon, name, battery. Off with `[moments] bluetooth = false`.
- **Camera and microphone.** The privacy watcher already knows when an app starts using them;
  a start now shows a 3 s card naming the app (`[moments] privacy`). The dots stay.
- All moments go through the card queue, respect quiet hours, Focus, fullscreen and games, and
  never take focus.

### Clipboard

- **Ctrl+C auto-shelve** (`[shelf] shelve_copied = false`). On `WM_CLIPBOARDUPDATE`: skip if Hytte
  owns the clipboard (its own Copy buttons), skip unless `CF_HDROP` is present and
  `Preferred DropEffect` is copy, skip if an exclusion format is present (below). Add the paths
  through `shelf::add_paths` in reference mode; duplicates are ignored as today; the usual 5 s
  shelf peek, held during Focus like other alerts. Switch in Settings → Shelf.
- **Clipboard history** (`[clipboard] history = false`). Text only, at most 20 clips of up to
  4 KB, **kept in memory only**. Not recorded when the clipboard carries
  `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0 or
  `Clipboard Viewer Ignore` (what password managers set), when the owning app is in
  `[clipboard] ignore_apps` (default: KeePass, KeePassXC, 1Password, Bitwarden), or when
  `secrets::mask` would change it. Cleared on lock, sleep and when switched off.
- **On the pill:** a row of clip chips under the shelf tiles (five visible, the wheel scrolls);
  a click copies the clip back (Hytte owns it, so it isn't recorded again). The Shelf pane shows
  when the shelf has items **or** history has clips. A Settings line says Windows has its own
  history on Win+V, so people choose knowingly.

### Reach

- **ARM64.** CI adds a test job on `windows-11-arm`; release builds
  `aarch64-pc-windows-msvc` there too. Assets: `hytte.exe` stays the x64 exe (every existing
  updater downloads that name), plus `hytte-arm64.exe` and `hytte-<v>-arm64.zip`. The updater
  picks its asset by `cfg!(target_arch)`. x64 installs on ARM PCs stay x64 (they work under
  emulation); the manual says how to switch. The winget manifest gets an `arm64` installer entry.
- **Scoop bucket** in this repo (`bucket/hytte.json`, x64 and arm64, `checkver` and `autoupdate`
  from GitHub releases): `scoop bucket add hytte https://github.com/PrathamPai-2007/hytte`. The
  release job fills its hashes like the winget one. A Scoop install (exe under
  `\scoop\apps\hytte\`) turns off the self-updater (the update card says `scoop update hytte`),
  skips adding itself to PATH (Scoop's shim does it), and writes the autostart entry through
  `apps\hytte\current\` so it survives updates. `pre_install` stops a running Hytte (the exe would
  be locked). These are the only trade-offs found, and each is handled.
- **1.7.1 leftover:** remove the release workflow's Sign step, then
  `gh secret delete MINISIGN_SECRET_KEY` (user runs it or approves it).

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Refactor changes behaviour | Regressions everywhere | Tests unchanged, snapshots byte-identical, one pane per commit |
| Named actions run the wrong thing | Destructive command | Only names from `config.toml` / packs; `confirm`; Job object; timeout; result card shows what ran |
| A pack runs a surprise command | Code execution | Packs are copied in by hand; first run asks with the command shown |
| Clipboard history catches a password | Secret on screen | Exclusion formats, ignore list, `secrets::mask` refuses, memory only, cleared on lock, off by default |
| Auto-shelve floods the shelf | Noise | Off by default, shelf limits and duplicate check apply, skips Hytte's own copies |
| Bluetooth battery not reported | Empty field | Show the name without a battery; no polling to find it |
| ARM build untested on hardware | A crash on ARM | CI tests on ARM; release notes say ARM is new; x64 still works there |
| Scoop and self-update fight | Broken install | Self-update off under Scoop, autostart through `current` |
| Scope too big for one release | 1.8 never ships | Phases end in checkpoints; phases 4–6 can ship as 1.9 if needed |

## Phases and checkpoints

0. Release plumbing (Tasks 1–3) → **Checkpoint 0**: dry-run release builds both arches.
1. Foundations (Tasks 4–7) → **Checkpoint A**: tests unchanged, snapshots identical (except the
   removed settings pane), user reviews the settings-pane removal.
2. Agent cockpit (Tasks 8–11) → **Checkpoint B**: snapshot page; live run with Claude Code.
3. Platform (Tasks 12–15) → **Checkpoint C**: snapshot page; live action and pack run.
4. System moments (Tasks 16–17) → **Checkpoint D**: live Bluetooth connect on the user's PC.
5. Clipboard (Tasks 18–19) → **Checkpoint E**: live test with a password manager.
6. Docs, welcome and release (Tasks 20–22) → **Checkpoint F**: user approves the release.

"Checks" means `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
UI changes get offscreen snapshots shown on one before/after page. Idle cost is measured after
each phase against 1.7 (0.26 % of one core, 10.1 MB) and must not grow with the new features
switched off. Dev builds only (isolated `%APPDATA%`); the Desktop Hytte stays stopped. Commits
carry no trailers.
