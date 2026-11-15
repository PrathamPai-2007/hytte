# Implementation Plan: Hytte 1.9.2 — agents that talk back, and reach

## Overview

Make the pill useful *to* agents (not only a viewer of them), reach the people Hytte misses today
(WSL users), and add the two small utilities that were missing: capturing a note from a terminal
and a day summary for agents. Every new behaviour is **opt-in or inert until used**; no hotkey is
ever set by default (the Notes hotkey, `general.note_hotkey`, already exists and ships empty).

Scope facts checked in the code on 2026-11-15 (so we don't rebuild them):
- Already shipped: `hytte doctor`, `hytte card/notify/pane/shelf`, `hytte agent ask`, six agents
  (Claude Code, Codex, OpenCode, Gemini CLI, Antigravity, Cursor), Calendar from the Windows
  appointment store, packs (`examples/panes`), `note_hotkey` (empty = off), Ollama Ask.
- Missing: MCP server, WSL hooks, CLI to add a note/to-do, agent day summary, data-driven agent
  adapters, more example packs.

## Assumptions (correct me, or I proceed)

1. Version is **1.9.2** as asked. It is a large patch; **1.10.0** would fit better. Only
   `scripts/version.ps1 set` changes if you switch.
2. Rejected ideas stay rejected (see memory `project-hytte-1-8-plan`): no notifications in the
   pill beyond cards, no answering agent *questions* on the pill, no signing, no pet, no redesign.
3. No new dependencies: the MCP server is hand-written stdio JSON-RPC on `serde_json`.
4. Agent history stays **in memory only** (never on disk), per the 1.8 plan.
5. No default hotkey for anything. Quick notes: the existing setting, plus a CLI.
6. Calendar ICS import is dropped: the Windows appointment store already covers Outlook/Calendar.

## Architecture decisions

- New IPC `Cmd` kinds (`note_add`, `todo_add`) follow `shelf_add`: one-way, silent when paused or
  in game mode, validated in `hytte_proto`, older daemons ignore them.
- `hytte mcp` is a subcommand of the same exe (design rule: CLI only `std` + `windows` +
  `serde_json` + `hytte-proto`). Tools map 1:1 to existing commands, so no new daemon behaviour:
  `notify`, `card` (with `wait` = ask the human), `note_add`, `todo_add`, `pane_set`.
- WSL is a **spike first**: the liveness/click-to-focus model uses a Windows pid, which a WSL
  process doesn't have. We decide from evidence whether to ship it.
- Agent adapters as data only for *new* agents; the six existing ones are not refactored.

## Task list

### Phase 1: Reach and capture (small, valuable, low risk)
- [x] T0: Drag the pill sideways (top strip, per-monitor, free placement, drag-mode scene)
- [x] T1: `hytte note` / `hytte todo` CLI (+ proto kinds, daemon handler, docs)
- [ ] T2: WSL spike (hooks from a WSL distro reach the pill; decide ship/drop)
- [ ] T3: WSL support if T2 says go (`hytte setup agents --wsl`, doctor line)

### Checkpoint A
- [ ] tests, clippy (stable 1.100) clean; `hytte note "x"` lands in Notes in a dev build

### Phase 2: Agents talk to the pill
- [x] T4: `hytte mcp` stdio server (initialize, tools/list, tools/call) for the five tools
- [x] T5: opt-in registration: `hytte setup mcp` (Claude Code, Codex) + `--undo` + doctor line
- [x] T6: agent day summary in the Agents pane (in memory)

### Checkpoint B
- [ ] a real Claude Code session calls `card` with buttons and gets the pressed label back
- [ ] tests, clippy clean, Agents pane snapshot reviewed

### Phase 3: Breadth
- [x] T7: adapter format for new agents + one new agent (only if its hooks are documented)
- [x] T8: 8 example packs + gallery README, linked from manual and landing page

### Phase 4: Ship
- [ ] T9: docs (04, 05, 16), manual chapters, snapshots, landing page FAQ
- [ ] T10: version 1.9.2, release (needs your go-ahead; test the built app first)

## Tasks

## Task 0: Drag the pill sideways
**Description:** Press on the pill's top strip and move sideways to place it anywhere along the
top of its monitor. While dragging, the pill's contents are replaced by drag-mode instructions
("Drag left or right · Esc cancels · double-click resets"). Decisions (user, 2026-11-15):
offset is **per monitor**, placement is **free** (no snapping), edge cases are mine to handle.

**Design:**
- Offset stored as a **fraction of the monitor width** (survives resolution, DPI and scale
  changes), keyed by monitor device name, in a small state file (`%APPDATA%\Hytte\position.json`),
  not `config.toml`. A missing key means centred, so snapshots and today's behaviour are unchanged.
- `origin_for` (`window/win/view.rs`) adds the offset; the file-drop hot zone (`ZONE_L/R`) follows it.
- Drag starts after the pointer moves ~6 px (scaled) from a press on the top strip, so a plain
  click is unchanged. Uses the existing `SetCapture` path (as the shelf-tile drag does).
- Drag mode is its own scene (fixed, narrow) so the pill is not resized by whatever pane was open.
- The stored offset is kept as the user left it; the *effective* offset is clamped each layout to
  keep the pill (collapsed or open) fully on its monitor, so a wide pane near an edge slides in
  and the pill returns to the chosen spot when it collapses.

**Edge cases to handle:**
- Esc, `WM_CAPTURECHANGED`, Alt+Tab or a dialog during a drag cancel it and restore the old spot.
- No drag while paused, in game mode or over fullscreen video; a drag in progress ends if one starts.
- Hover logic must not open, close or switch panes during a drag.
- The strip must be hit-testable while the pill is collapsed (check it isn't click-through there).
- Monitor unplugged or renamed: no key → centred; "next monitor" uses that monitor's own offset.
- Double-click on the strip resets this monitor; Settings gets "Reset position" (all monitors).
- Different DPI per monitor: pixel math uses that monitor's scale.
- `hide_from_capture`, taskbar docked at the top and multi-monitor negative coordinates still work.
- The pill's overflow guard keeps a minimum margin from both screen edges.

**Acceptance:**
- [ ] drag moves the pill and it stays after restart; each monitor remembers its own spot
- [ ] drag-mode scene shows instructions; snapshots reviewed on the snapshot page
- [ ] a click on the strip still behaves as before; file drops still work on a moved pill
- [ ] clamp, fraction-to-pixel and per-monitor lookup are pure functions with unit tests
**Verification:** `cargo test -p hytte`; stable clippy; dev build on one and (if available) two
monitors; never the Desktop `hytte.exe`.
**Dependencies:** none. **Files:** `window/win/view.rs`, `window/win.rs`, `window/win/wndproc.rs`,
`render/*` (drag scene), a small `position.rs`, `settings_win.rs`, docs, manual 2 and 7.
**Scope:** M

## Task 1: `hytte note` and `hytte todo`
**Description:** Add one-way commands so a terminal, script or agent can add a note or a to-do.
`hytte note "text"`, `hytte todo "text"`; with no text, piped stdin (≤ 2000 chars, `jot::MAX_CHARS`).
**Acceptance:**
- [ ] `Cmd` gains `note_add` / `todo_add` (1–2000 B text); invalid combos rejected in `validate`
- [ ] daemon adds a temporary note (24 h rule) or a to-do; ignored while paused/game mode
- [ ] exits 1 with "daemon not running" when no pill; over-length exits 2
- [ ] no hotkey added or defaulted
**Verification:** `cargo test -p hytte-proto -p hytte`; dev build: `hytte note hello` shows in Notes.
**Dependencies:** none. **Files:** `crates/proto/src/lib.rs`, `cli/mod.rs`, `pipe_server.rs`, `ui_state/*`, `docs/04,05`. **Scope:** M

## Task 2: WSL spike
**Description:** From a WSL distro, call `/mnt/c/.../hytte.exe agent ...` as a hook and see what
works: delivery, owner pid, liveness, click-to-focus, `ask`. Write findings to this file.
**Acceptance:** a short findings table (works / broken / needs change) and a ship-or-drop call.
**Verification:** run in a real distro against a dev build (never the Desktop `hytte.exe`).
**Dependencies:** none. **Scope:** S

## Task 3: WSL support (conditional on T2)
**Description:** `hytte setup agents --wsl` writes hooks into each distro's agent config using a
Windows-exe path; `doctor` reports WSL agents; `--undo` reverses it.
**Acceptance:** idempotent, backs up like the Windows path, never touches a non-JSON file.
**Verification:** tests for the merge on a temp tree; manual run in WSL.
**Dependencies:** T2. **Scope:** M

## Task 4: `hytte mcp`
**Description:** Minimal MCP stdio server: `initialize`, `tools/list`, `tools/call`.
Tools: `notify`, `card` (buttons + wait = "ask the human"), `note_add`, `todo_add`, `pane_set`.
**Acceptance:**
- [ ] tool schemas validate inputs with the same limits as the CLI
- [ ] with no daemon every call returns a tool error, never hangs; `wait` capped by ttl
- [ ] stdout carries only protocol JSON (logs go to stderr)
**Verification:** pure request→response unit tests; scripted stdio session; Claude Code `/mcp`.
**Dependencies:** T1. **Files:** `cli/mcp.rs`, `cli/mod.rs`. **Scope:** M–L

## Task 5: register the MCP server
**Description:** `hytte setup mcp [--undo]` adds a `hytte` server entry for Claude Code and Codex
(merge like `agents.rs`: ours/theirs, backup, temp+rename). Not run automatically.
**Acceptance:** idempotent; plain-JSON-only; doctor shows connected / not connected.
**Dependencies:** T4. **Scope:** M

## Task 6: agent day summary
**Description:** In the Agents pane, per agent: time working, time waiting for you, finished runs.
In memory, resets at midnight and on restart; off the disk.
**Acceptance:** numbers survive pane close/open; nothing written to disk; snapshot reviewed.
**Dependencies:** none. **Files:** `ui_state/*`, `render/*`, snapshot. **Scope:** M

## Task 7: adapters for new agents
**Description:** A small data format (config file, JSON path, event → `hytte agent` subcommand)
plus Copilot CLI if its hooks are documented; otherwise ship the format with a docs example.
**Acceptance:** an adapter file adds an agent without Rust changes; the six existing are untouched.
**Dependencies:** doc research task first (source-driven). **Scope:** M

## Task 8: pack gallery
**Description:** 8 example packs under `examples/` with a README, an install line, and links.
**Acceptance:** each pack loads in a dev build; README has one screenshot each.
**Dependencies:** none. **Scope:** S–M (content)

## Task 9: docs and site
**Description:** docs 04/05/16, manual (agents, CLI, tools), landing page FAQ + a short GIF of the
agent flow at the top.
**Dependencies:** T1–T8. **Scope:** M

## Task 10: release
**Description:** `version.ps1 set 1.9.2`, build, start the built app for you to try, then (with
your go-ahead) tag and release. CI uses stable clippy; run `cargo +stable-x86_64-pc-windows-msvc clippy`.
**Dependencies:** all. **Scope:** S

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| WSL processes have no Windows pid | High (T3) | T2 spike decides; drop T3 if click-to-focus can't work |
| MCP clients differ in protocol version | Med | implement the minimum, test with Claude Code and Codex |
| `card --wait` blocks an agent turn | Med | ttl cap, documented, hang-up = "no answer" like `ask` |
| Patch version holds a minor's worth of work | Low | switch to 1.10.0 (one command) |
| Day summary tempts a disk history | Low | in memory only; Task acceptance enforces it |

## Open questions
- 1.9.2 or 1.10.0?
- Is Copilot CLI the right first adapter, or another agent you use?
