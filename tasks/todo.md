# Hytte 1.8 tasks

Decisions, assumptions, risks and checkpoints are in [plan.md](plan.md). "Checks" = fmt, clippy
`-D warnings`, tests. Every UI change gets an offscreen snapshot. Branch `v1.8`. Commits carry no
trailers. Nothing is pushed or tagged without the user's go-ahead.

## Phase 0: Release plumbing

### Task 1: Drop release signing (1.7.1 leftover)
Remove the Sign step and `hytte.exe.minisig` from `release.yml` and section 15 of the docs.
- [ ] Dry-run release passes with no signing step
- [ ] User runs or approves `gh secret delete MINISIGN_SECRET_KEY` after the next release
**Files:** `.github/workflows/release.yml`, `docs/15-updates.md`. **Scope:** XS

### Task 2: ARM64 build
CI test job on `windows-11-arm`; release builds `aarch64-pc-windows-msvc` and publishes
`hytte-arm64.exe` and `hytte-<v>-arm64.zip` beside the unchanged x64 assets. Updater picks its
asset by `cfg!(target_arch)`. Winget manifest gets an `arm64` installer; `version.ps1 hash`
fills both.
- [ ] Updater test: x64 build picks `hytte.exe`, arm64 picks `hytte-arm64.exe`; an old release
      with no arm64 asset offers no update to an arm64 install
- [ ] CI green on both runners; dry-run release produces four assets
- [ ] The arm64 exe reports `hytte <v>` from `--version` on the ARM runner
**Files:** `ci.yml`, `release.yml`, `update.rs`, `scripts/version.ps1`, winget manifest. **Scope:** M

### Task 3: Scoop bucket
`bucket/hytte.json` (x64 + arm64, `checkver`, `autoupdate`, `pre_install` stops Hytte). Release
job fills its hashes. A Scoop install turns off self-update, skips PATH setup, writes autostart
through `apps\hytte\current\`.
- [ ] Unit test for the Scoop path check (`\scoop\apps\hytte\` in either form, case-insensitive)
- [ ] Live: `scoop bucket add` + `scoop install hytte` from a local checkout; Hytte runs, update
      card says `scoop update hytte`, `scoop update` with Hytte running succeeds
- [ ] `hytte doctor` says it is a Scoop install
**Files:** `bucket/hytte.json`, `release.yml`, `update.rs`, `setup.rs`, `config.rs`, `cli/doctor.rs`.
**Scope:** M

### Checkpoint 0
- [ ] Checks pass; dry-run release builds both arches and fills both manifests

## Phase 1: Foundations

### Task 4: Split `ui_state.rs` into `ui_state/`
Move code, change nothing: `mod.rs`, `anim.rs`, `cards.rs`, and one file per pane with its
state struct and `size` / `glow` / `ambient` / `expire` parts. One pane per commit.
- [ ] Every existing test passes, unchanged except its path
- [ ] All offscreen snapshots byte-identical to before the split
- [ ] No file over ~800 lines; docs section 6 points at the new files
**Files:** `ui_state.rs` → `ui_state/*`, imports across `window/` and `render/`. **Scope:** L

### Task 5: One layout description per pane
`layout(&Model) -> Rows` per pane, used by both `size` and the renderer. Tasks and Notes first,
then the rest; delete the duplicated sums.
- [ ] Size tests unchanged; snapshots byte-identical
- [ ] Adding a row type to a pane touches one place (shown in the section 14 recipe)
**Files:** `ui_state/*`, `render/*`. **Scope:** L

### Task 6: One card queue
`VecDeque` of cards with priority (ask, timer done, update, live card, alert, info), same-key
replace, at most 8 waiting, waiting alerts expire on their hold.
- [ ] Tests: priority order, an ask jumps the queue, an alert during another card waits
      instead of being dropped, expiry while waiting, same key replaces
- [ ] Existing chip, card and `--wait` tests still pass
**Files:** `ui_state/cards.rs`, `window/win/actions.rs`, `events.rs`. **Scope:** M

### Task 7: Remove the in-pill settings pane
Delete `Scene::ExpSettings`, its rendering and `SETTINGS_GROUPS`; the gear opens the settings
window (mouse and keyboard). Keep anything the settings window still uses.
- [ ] Gear click and keyboard Enter open the settings window; no scene left that can't be reached
- [ ] Snapshot: Home with the gear; manual section 7 and screenshots updated
**Files:** `ui_state/*`, `render/settings.rs`, `window/win/*`, `manual/07-settings.md`. **Scope:** S

### Checkpoint A
- [ ] Checks pass; idle cost no higher than 1.7
- [ ] User reviews the settings-pane removal

## Phase 2: Agent cockpit

### Task 8: Richer agent data from existing hooks
CLI counts files touched and subagent launches from `PreToolUse`, reads context tokens from the
transcript's last `usage`; new optional fields on the agent message.
- [ ] Unit tests on sample payloads and a sample transcript; `old_messages_still_parse` passes
- [ ] A 1.7 daemon ignores the new fields
**Files:** `cli/agent.rs`, `crates/proto/src/lib.rs`, `tasks.rs`. **Scope:** M

### Task 9: Agents pane
`Panel::Agents`, `[panes] agents`; rows with state, step, wait age, context, files, subagents;
click jumps to the terminal; agents leave the Tasks pane.
- [ ] Tests: pane shows only with a session, row order (waiting first), Tasks pane no longer
      lists agents
- [ ] Snapshots: one agent working, one waiting + two working, long project names
**Files:** `ui_state/agents.rs`, `render/agents.rs`, `tabs.rs`, `config.rs`. **Scope:** M

### Task 10: Done card line
"Edited 4 files · 84k context" on the agent reply card when known.
- [ ] Snapshot; test that the line is left out when nothing is known
**Files:** `ui_state/cards.rs`, `render/cards.rs`. **Scope:** XS

### Task 11: Allow / Deny for more agents (research first)
Check Gemini CLI, Cursor and Antigravity's current hook docs for a blocking decision. Build it
only where supported; update the manual's agent table either way.
- [ ] Short findings note with doc links, before any code
- [ ] For each supported agent: setup writes the hook, a live ask is answered from the pill
**Files:** `agents.rs`, `cli/agent.rs`, `manual/04-ai-agents.md`. **Scope:** M

### Checkpoint B
- [ ] Checks pass; snapshot page reviewed
- [ ] Live: a real Claude Code session shows context, files and a subagent count

## Phase 3: Platform

### Task 12: Rows in script panes
JSON `{"rows":[…]}` output (pulled or pushed, `hytte pane set --json`) draws up to six rows;
anything else is plain text as before.
- [ ] Parse tests: valid, too many rows, bad JSON falls back to text, limits, masking
- [ ] Snapshots: rows with values, progress and tones
**Files:** `scriptpane.rs`, `ui_state/script.rs`, `render/script.rs`, `cli/mod.rs`. **Scope:** M

### Task 13: Named actions
`[[action]]` (`name`, `command`, `confirm`, `timeout`); card buttons `Label:action` and row
`action`; run hidden with a Job object; result card.
- [ ] Tests: unknown name ignored, `confirm` arms then runs, timeout kills the tree, a pipe
      message can't carry a command
- [ ] Live: a card button restarts a dummy server; a timed-out action leaves no process behind
**Files:** `config.rs`, `scriptpane.rs`, `crates/proto/src/lib.rs`, `cli/mod.rs`,
`window/win/actions.rs`. **Scope:** M

### Task 14: Packs
`%APPDATA%\Hytte\panes\<name>\pane.toml`, loaded at start and on change; shares the four-pane
limit; first run asks with the command shown.
- [ ] Tests: loading, limit with `[[script_pane]]`, bad `pane.toml` reported like a config error,
      action names prefixed by the pack
- [ ] Live: copy a pack in while Hytte runs; the ask card appears; **Run** shows it
**Files:** `scriptpane.rs`, `config.rs`, `window/win/scripts.rs`. **Scope:** M

### Task 15: Example packs
`examples/panes/` with GitHub PRs (`gh`), Docker containers and CI status, each with a README.
- [ ] Each runs on the user's PC (or says clearly what it needs)
**Files:** `examples/panes/*`, `manual/06-everyday-tools.md`. **Scope:** S

### Checkpoint C
- [ ] Checks pass; snapshot page reviewed; idle cost with no packs equals Checkpoint A

## Phase 4: System moments

### Task 16: Bluetooth connect card
`bluetooth.rs` worker on WinRT `DeviceWatcher`; battery where reported; card through the queue;
`[moments] bluetooth`.
- [ ] Unit tests for the pure parts (name, battery parsing, debounce of reconnect flapping)
- [ ] Live: connect earbuds or a mouse on the user's PC; card shows name and battery
- [ ] No wake-ups while nothing connects (idle cost unchanged)
**Files:** `bluetooth.rs`, `main.rs`, `config.rs`, `ui_state/cards.rs`. **Scope:** M

### Task 17: Camera and microphone start card
A 3 s card naming the app when it starts using the camera or mic; `[moments] privacy`.
- [ ] Tests: start shows once, stop shows nothing, held during Focus and quiet hours
- [ ] Live: start the camera app
**Files:** `privacy.rs`, `ui_state/*`, `config.rs`. **Scope:** S

### Checkpoint D
- [ ] Checks pass; both moments seen live

## Phase 5: Clipboard

### Task 18: Ctrl+C auto-shelve
`[shelf] shelve_copied = false`; on clipboard change add copied (not cut) files by reference,
skipping Hytte's own copies and excluded content; switch in Settings → Shelf.
- [ ] Tests for the decision function: copy vs cut, own clipboard, exclusion formats, duplicates
- [ ] Live: Ctrl+C a file in Explorer lands on the shelf; Ctrl+X doesn't; shelf Copy doesn't loop
**Files:** `window/win/controls.rs`, `shelf.rs`, `config.rs`, settings rows. **Scope:** S

### Task 19: Clipboard history on the Shelf pane
`[clipboard] history = false`, `ignore_apps`; text only, 20 clips, memory only; skip rules;
cleared on lock, sleep, switch-off; clip chips under the shelf tiles; click copies back.
- [ ] Tests: each skip rule, masking refusal, 20-clip cap, 4 KB cap, clear on lock
- [ ] Snapshots: shelf with tiles and clips, clips only
- [ ] Live: copy from KeePassXC or Bitwarden is not recorded; nothing written to `%APPDATA%`
**Files:** `clipboard.rs`, `ui_state/shelf.rs`, `render/shelf.rs`, `window/win/*`, `config.rs`.
**Scope:** M

### Checkpoint E
- [ ] Checks pass; snapshot page reviewed; password-manager test passed

## Phase 6: Docs, welcome, release

### Task 20: Docs and manual
Section 16 (1.8 notes), sections 3, 6, 8, 11, 14 for the new structure; manual for agents,
platform, moments, clipboard, ARM and Scoop install.
- [ ] Every new config key documented with its default
**Files:** `docs/*`, `manual/*`, `README.md`. **Scope:** M

### Task 21: Welcome "What's new in 1.8"
Three cards for updaters (marker `1.8`); fresh-install cards updated for the removed settings pane.
- [ ] Snapshots; words approved by the user
**Files:** `welcome.rs`. **Scope:** XS

### Task 22: Release 1.8.0
Version bump, dry run, then tag only on the user's go-ahead.
- [ ] Dry run green; user approves; release has x64 and arm64 assets; winget and Scoop hashes
      committed
**Files:** `Cargo.toml`s, `scripts/*`. **Scope:** S

### Checkpoint F
- [ ] User approves the release
