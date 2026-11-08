# Hytte 2.0 tasks

Decisions, risks and checkpoints are in [plan.md](plan.md). "Checks" = fmt, clippy `-D warnings`,
tests. Every UI change gets an offscreen snapshot. Commits carry no trailers. Nothing is pushed.

## Phase 1: Live cards

### Task 1: `card` command in the protocol
Add `Cmd` kind `"card"` with the optional fields from the plan (all `Option`, skipped when empty).
`validate` checks every limit; `open` must be http(s) or an absolute path.
- [x] Round-trip, limit and bad-`open` tests; `old_messages_still_parse` still passes
- [x] A `card` line sent to a 1.6 daemon is ignored (no task row, no chip)
**Files:** `crates/proto/src/lib.rs`. **Scope:** S

### Task 2: The card on the pill
`UiEvent::Cmd` with `card` creates or updates a card by `id`; drawn as a chip with icon, progress
bar and up to two buttons; queues behind Ask and update chips; `done`, `clear`, ttl and 30 s
silence for progress cards. Body click opens `open`.
- [x] Unit tests: update in place, queue order, ttl/silence expiry
- [x] Snapshots: plain, progress 40 %, done, two buttons, long text
**Files:** `ui_state.rs`, `window/win/actions.rs`, `render/cards.rs`. **Scope:** M

### Task 3: `hytte card` CLI with `--wait`
Flags as in the plan. `--wait` keeps the pipe open; the daemon writes the pressed label back
(reusing the Ask reply path in `pipe_server.rs`). Exit 0 with the label, 1 on no press.
Note: a waited card may sit queued behind others; the wait allows the ttl plus 5 minutes of
queue (`CARD_QUEUE_SECS`) and ends at once when the card expires, is dismissed or cleared.
- [x] Arg-parsing tests; `hytte card --help` text
- [x] Live: a script waits, press **Restart**, script prints `Restart`; ttl expiry exits 1
**Files:** `cli/mod.rs`, `pipe_server.rs`. **Scope:** M

### Checkpoint A
- [x] Checks pass
- [ ] Snapshot page reviewed by the user

## Phase 2: Script panes

### Task 4: Config and the pane slot
`[[script_pane]]` (`name`, `command`, `every`) with limits (max 4, `every` ≥ 30). `Panel::Script(u8)`
in the tab strip, `[panes] order` (`script1`–`script4`, by place in the file), 1–9 keys and
settings arrows. Switches are `[panes] script1`–`script4`, written only when off. Empty pane
shows "Waiting for output".
- [x] Config tests: defaults, limits, unknown keys warned as usual, order parsing with script names
- [x] Snapshot: a script pane in the strip
**Files:** `config.rs`, `ui_state.rs`, `tabs.rs`, `render/script.rs`. **Scope:** M

### Task 5: Push (`hytte pane set`)
`Cmd` kind `"pane_set"` (`name`, `text` ≤ 2 KB). Daemon ignores unknown names; the CLI checks
`config.toml` and errors with exit 1. Piped stdin works.
- [x] Proto tests; CLI test for the unknown-name message
- [x] Live: `echo 3 users | hytte pane set Server` shows on the pane
**Files:** `crates/proto/src/lib.rs`, `cli/mod.rs`, `window/win/actions.rs`. **Scope:** S

### Task 6: Pull (Hytte runs the command)
`scriptpane.rs`: one worker thread; runs due panes only while their pane shows (and once at start);
`cmd /c` hidden, home folder, 10 s timeout, Job object kills the tree. Output cleaned (ANSI,
control chars, 6 lines, 2 KB, `secrets::mask`). Non-zero exit shown in warn colour.
- [x] Unit tests: output cleaning, due check, timeout result
- [x] Live: a 1 s command shows; a `timeout /t 60` command is killed at 10 s with no orphan left
- [x] Idle with the pane closed: no process spawned, CPU unchanged from the baseline
**Files:** `scriptpane.rs` (new), `window/win/*.rs`. **Scope:** M

### Task 7: Pane rendering and settings
"Updated 3 min ago" line, warn style for failures, long lines ellipsized. The settings window's
Panes list shows script panes with switches and ▲ ▼ (editing them stays in `config.toml`; a
"Edit in config.toml" link opens the file).
- [x] Snapshots: output, failure, stale, settings list
**Files:** `render/script.rs`, `render/settings_table.rs`. **Scope:** S

### Checkpoint B
- [x] Checks pass; a pull and a push pane run live together
- [ ] Snapshot page reviewed by the user

## Phase 3: Accent and key hints

### Task 8: Accent skin
`Skin::Accent`; accent read through WinRT at start and on `ImmersiveColorSet`; pure derive
function for body, glow and rim; disc in the Look tab and the settings window.
- [x] Unit test: contrast ≥ 4.5:1 for several accents (light, dark, saturated); fallback to graphite
- [ ] Live: change the Windows accent, the pill follows without a restart (user to check: Hytte doesn't change system settings)
- [x] Snapshots: four accents, dark and light pill
**Files:** `skin.rs`, `winrt.rs`, `window/win/wndproc.rs`, `render/settings*.rs`. **Scope:** M

### Task 9: Key hints
Hint line in keyboard mode (typing and card variants); stays while keyboard mode is on (no
fade). Tray gets "Open with keyboard" with the chord when a hotkey is set.
- [x] Snapshots: hint line on Home, on a pane, while typing
- [x] Live: the tray shows the chord; no hint outside keyboard mode
**Files:** `render/pill.rs`, `window/win/input.rs`, `tray.rs`. **Scope:** S

### Checkpoint C
- [x] Checks pass
- [ ] Snapshot page reviewed by the user

## Phase 4: Welcome tour

### Task 10: Welcome tour
`welcome.rs` with the two 3-card sets in one table; fresh vs update decided from whether
`config.toml` existed at start; `%APPDATA%\Hytte\welcome` marker written on Done or Skip; same
gates as the update chip; waits for the next hover instead of popping the pill open.
- [x] Unit tests: which set (fresh, update, already seen, later 2.x)
- [x] Snapshots: all six cards
- [x] Live: a profile with an existing config sees "What's new", Next, Next, Done writes the marker
- [ ] Live fresh profile not run: a fresh start writes the default config, which turns on autostart
      and PATH for this PC (covered by the unit test instead)
**Files:** `welcome.rs` (new), `window/win/actions.rs`, `render/cards.rs`, `main.rs`. **Scope:** M

### Checkpoint D
- [ ] Card words approved by the user

## Phase 5: Docs and launch

### Task 11: Docs and manual
`docs/16-whats-new.md` (2.0 section), `docs/04-ipc-protocol.md` (`card`, `pane_set`),
`docs/05-cli.md` (`hytte card`, `hytte pane set`), `docs/11-configuration-and-files.md`
(`[[script_pane]]`, `welcome` file), manual pages for live cards, script panes, the accent skin and
keyboard mode. README "Try it" gains a `hytte card` line.
- [x] Every new flag and config key appears in docs and manual
**Scope:** S

### Task 12: Landing page copy for 2.0
Feature copy for live cards, script panes and the accent skin; fix the FAQ line about screenshots.
No layout changes. `npm run build` passes.
- [x] Builds, lints and typechecks; new status lights checked at desktop and phone width
- [ ] Built site reviewed by the user locally (not deployed)
**Files:** `landing-page/src/`. **Scope:** S

### Task 13: Launch video
Run `/brag` against the updated landing page; MP4 plus a short GIF in `docs/img/`; README links the
video.
- [x] `docs/img/hytte-2.0.mp4` (21 s), poster `.jpg`, hook `.gif`; README links it
- [ ] User watches it and approves; nothing published until the release hold lifts

### Checkpoint E
- [ ] Everything on local `main`, checks pass, version 2.0.0; release waits on the hold
