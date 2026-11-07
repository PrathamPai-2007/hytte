# Hytte 2.0 tasks

Decisions, risks and open questions are in [plan.md](plan.md). "Checks" means
`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`. Every UI change
gets an offscreen snapshot (`HYTTE_SNAP_DIR`). Commits carry no trailers. Nothing is pushed.

## Task 1: Notification listener spike

**Description:** A throwaway `examples/notif_spike.rs` (not shipped) proves, from an unpackaged
exe on this machine: `RequestAccessAsync` returns Allowed, `GetNotificationsAsync(Toast)` lists
real notifications with app name, title and body, and whether a WinEvent signal fires when a toast
appears (log every `EVENT_OBJECT_SHOW` from the shell process while a test toast arrives).

**Acceptance criteria:**
- [ ] Written result in this file: access ok / fails, read ok / fails, toast signal found (class
      name and event) or not
- [ ] CPU cost of one `GetNotificationsAsync` call measured

**Verification:** run it with `hytte notify` and a real app sending a toast; paste the output.
**Dependencies:** none. **Files:** `crates/hytte/examples/notif_spike.rs`, `Cargo.toml` (features).
**Scope:** S

## Task 2: Unblock winget (user-run steps)

**Description:** The user signs the Microsoft CLA, submits `hytte.exe` 1.6.1 to the Defender
false-positive form, and decides whether 1.6.1 goes to winget now. I prepare the multi-file manifest
for 1.6.1 (real SHA-256 of the published zip) in `manifests/winget/1.6.1/` for them to submit.

**Acceptance criteria:**
- [ ] Manifest passes `winget validate`
- [ ] `winget install --manifest manifests/winget/1.6.1` installs and runs a test copy (isolated
      `%APPDATA%`), then uninstalls cleanly

**Dependencies:** none. **Files:** `manifests/winget/`. **Scope:** S

## Checkpoint A
- [ ] Spike result reviewed with the user; go or reshape Phase 2

## Task 3: `card` command in the protocol

**Description:** Add `card` to `hytte_proto::Cmd` with optional `id`, `title`, `body`, `progress`,
`icon`, `buttons`, `open`, `ttl`, `done`, `clear`; validation limits from the plan; a reply line
`{"pressed":"Restart"}` for a waiting card.

**Acceptance criteria:**
- [ ] `validate` rejects over-limit fields, 3+ buttons, bad icons, progress > 100
- [ ] `old_messages_still_parse` extended; a card line never parses as `HytteMessage` or `Ask`

**Verification:** checks. **Dependencies:** none. **Files:** `crates/proto/src/lib.rs`,
`pipe_server.rs`. **Scope:** S

## Task 4: The card on the pill

**Description:** `Model` keeps live cards by `id` (at most 3 queued, newest shown, Ask chips first).
Render title, body, a progress bar, icon and up to 2 buttons in the existing card style; a body
click opens `open`; ttl expiry through the existing deadline timer (no new idle frames). Rate limit
5 updates/s per id.

**Acceptance criteria:**
- [ ] Update with the same `id` changes the card in place (no close/open animation)
- [ ] Snapshots: plain, progress, two buttons, done, three stacked
- [ ] Nothing shows paused, in game mode or over a fullscreen app

**Verification:** checks and snapshots. **Dependencies:** 3. **Files:** `ui_state.rs`,
`render/` (new `render/card.rs`), `window/win/events.rs`, `render/snap.rs`. **Scope:** M

## Task 5: `hytte card` CLI with `--wait`

**Description:** `hytte card [--id] [--title] [--body] [--progress N] [--icon] [--button L]...
[--open X] [--ttl S] [--done|--clear] [--wait]`. `--wait` blocks for the ttl, prints the pressed
label (exit 0) or nothing (exit 1). Exit 1 with "daemon not running" when there's no pill.

**Acceptance criteria:**
- [ ] Unit tests for argument parsing and exit codes
- [ ] A PowerShell example in `docs/05-cli.md` works end to end against a dev build

**Verification:** checks; run against `target\release\hytte.exe` with isolated `%APPDATA%`.
**Dependencies:** 3, 4. **Files:** `cli/mod.rs` (or `cli/card.rs`), `docs/05-cli.md`. **Scope:** S

## Checkpoint B
- [ ] Checks pass; the user reviews the card snapshots

## Task 6: Notification worker

**Description:** `notifications.rs` worker: request access once, read on the trigger chosen in
Task 1 (toast hook, else a 2 s poll), diff by notification id, send
`UiEvent::Notification { app, icon, title, body, id }`. Skip Hytte's own and muted apps. Not started
at all while the feature is off.

**Acceptance criteria:**
- [ ] Pure diff and filter functions unit-tested
- [ ] With the feature on and no notifications: no frames drawn; idle CPU within the
      `docs/12-performance.md` baseline

**Verification:** checks; perf measurement written into `docs/12-performance.md`.
**Dependencies:** 1. **Files:** `notifications.rs`, `main.rs`, `Cargo.toml`. **Scope:** M

## Task 7: Notification card and its rules

**Description:** Card with the app icon, app name, title and one body line; **Open** launches the
app, **Dismiss** removes Hytte's card; stacking as "N new". Held during Focus (counted in the
"what you missed" line), dropped in game mode, fullscreen and meetings. Right-click → "Don't show
<app>" writes `[notifications] muted`.

**Acceptance criteria:**
- [ ] Unit tests: held in focus, dropped in a game, muted app ignored, stack count
- [ ] Snapshots: one, stacked, text off, light and dark

**Verification:** checks and snapshots. **Dependencies:** 4, 6. **Files:** `ui_state.rs`,
`render/card.rs`, `window/win/actions.rs`, `config.rs`. **Scope:** M

## Task 8: Permission and settings

**Description:** `[notifications] enabled, show_text, muted` in config (off for existing installs).
Fresh installs get a one-time card "Show your notifications here?" (Yes triggers the Windows
prompt). Settings → Features gets the switch and "Show message text"; the window lists muted apps
with an unmute button. A denied permission shows a row explaining where to allow it in Windows.

**Acceptance criteria:**
- [ ] Existing config without the section keeps the feature off
- [ ] Denied access never retries in a loop

**Verification:** checks and snapshots. **Dependencies:** 6, 7. **Files:** `config.rs`,
`render/settings_table.rs`, `settings_win.rs`. **Scope:** M

## Checkpoint C
- [ ] Checks pass; snapshots reviewed; idle CPU and frames unchanged

## Task 9: Docs, manual, what's new, README images

**Description:** README feature list, `manual/06` and `manual/07`, `docs/04` (card command),
`docs/05` (CLI), `docs/09` (worker), `docs/11` (config), `docs/16` (2.0 section), new images
`docs/img/card.png` and `docs/img/notification.png` from snapshots.

**Acceptance criteria:** every new setting, command and message is documented.
**Dependencies:** 4–8. **Scope:** M

## Task 10: Live test pass of everything on main

**Description:** A dev build with isolated `%APPDATA%` (never the Desktop `hytte.exe`): the user
and I go through a checklist: pane switches and arrows, Live lyrics box and glide, touchpad scroll,
typing over a fullscreen app, skin discs, More settings, the wider town search, lyrics skip list,
live cards, notifications. Bugs found are fixed and get a test.

**Acceptance criteria:** checklist written here with pass / fixed for each item.
**Dependencies:** 9. **Scope:** M

## Task 11: winget manifest for 2.0 and the release-workflow step

**Description:** Replace the singleton `manifests/winget/Hytte.Hytte.yaml` with the multi-file
layout; the release workflow fills the hash and opens the winget-pkgs PR with `komac`
(needs a token secret the user adds). Not run until the hold is lifted.

**Acceptance criteria:**
- [ ] `winget validate` passes; CI version check updated for the new files
- [ ] The workflow step is skipped cleanly when the token secret is missing

**Dependencies:** 2. **Files:** `manifests/winget/`, `.github/workflows/release.yml`,
`.github/workflows/ci.yml`, `scripts/version.ps1`. **Scope:** S

## Checkpoint D (release still on hold)
- [ ] The user lifts the hold → compare 1.6.0 and 1.6.1 downloads → the user decides → release
