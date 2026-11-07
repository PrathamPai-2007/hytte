# Implementation Plan: Hytte 1.6.0

## Overview

One release, 1.6.0: fix the bugs from the 2026-11-06 review, do all the hardening, change three
existing behaviours, add a small set of features, then refresh the docs, bump the version and ship.
The `render.rs` / `window.rs` split comes last and only if it is provably safe. Task details are in
[todo.md](todo.md).

## Decisions

| Topic | Decision |
|---|---|
| Release | One release, 1.6.0 |
| Code signing | Stay unsigned. No SignPath, no paid certificate |
| Distribution | Submit to `winget-pkgs` after 1.6.0 is published |
| Update verification | Minisign signature, public key built into Hytte |
| Update prompt | A chip on the pill, not a message box |
| `hytte run` | Fix `.cmd` / `.bat` lookup. Stdout stays inherited: no stdout reading, no ConPTY |
| Progress bar | Real values only, from stderr. No time estimates |
| Agent questions | Not answered on the pill. Clicking still jumps to the terminal |
| Displays | One pill, plus a "next monitor" action |
| CLI (default, see Open Questions) | `hytte shelf add` and `hytte notify` only |

## Out of scope

Build-time estimates, answering agent questions on the pill, scrollable reply card, stdout reading,
ConPTY, a pill per monitor, task history, phone alerts, audio output switcher, screen reader
support, `hytte status --json`, timer and mic CLI commands, browser media drops, CI watcher.

## Task list

### Phase 1: Bugs
- [x] 1. Message size limits
- [x] 2. `hytte run` finds `.cmd` and `.bat`
- [x] 3. Rerun keeps quoting
- [x] 4. Allow prompt shows what it approves
- [x] 5. Allow click guard
- [x] 6. Pipe send reliability
- [x] 7. PowerShell multi-line rows
- [x] 8. Stale downloads expire
- [x] 9. `hytte set --done` and `--fail`
- [x] 10. Safe file writes and cleanup

**Checkpoint A:** full checks plus a dev-build GUI pass.

### Phase 2: Hardening
- [x] 11. Updater verification
- [x] 12. Update prompt chip
- [x] 13. Per-user pipe and owner check
- [x] 14. Secret masking
- [x] 15. Crash relaunch
- [x] 16. Kill and shelf-copy guards
- [x] 17. CI hardening

**Checkpoint B:** full checks, plus an end-to-end update from a local fake release.

### Phase 3: Existing features
- [x] 18. Follow the Windows animation setting
- [x] 19. Real progress from `[N/M]` counts

### Phase 4: New features
- [x] 20. `hytte doctor`
- [x] 21. `hytte shelf add` and `hytte notify`
- [x] 22. Port labels
- [x] 23. Port Restart button
- [x] 24. Shelf QR code
- [x] 25. Shelf copy-as actions
- [x] 26. Next monitor action

**Checkpoint C:** full checks, GUI pass, idle CPU and memory compared with `docs/12-performance.md`.

### Phase 5: Ship
- [x] 27. Final review
- [x] 28. Docs refresh
- [x] 29. Version increment to 1.6.0
- [ ] 30. Publish the release (needs your go-ahead)
- [ ] 31. Submit to winget (needs your go-ahead)

### Phase 6: Last priority, after 1.6.0
- [x] 32. Split `render.rs` and `window.rs`, only if safe

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| A signing bug makes updates fail for everyone on 1.6.0+ | High | Test good, tampered and missing signatures end to end before release. A bad build needs one manual download, so the release dry run must include signing |
| Losing the minisign secret key | High | You keep an offline backup. Recovery is a new key plus one manual update |
| The per-user pipe name disconnects open terminals | Low | New terminals pick it up. Note it in the release text |
| Secret masking hides harmless text or misses a secret | Low | Fixed list of common patterns, tests both ways. It does not replace "keep out of screen share" |
| Crash relaunch loops | Medium | At most 3 relaunches in 10 minutes, then stop and log |
| Splitting the big files changes behaviour | Medium | Moves only, in its own branch, after the release. Abandon it if any check differs |

## Testing rules

- Every task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Run tests with `APPDATA` pointed at a temp folder: shelf tests use the real data folder otherwise.
- GUI checks use a dev build from `target\release` with an isolated `%APPDATA%`. Stop the Desktop
  `hytte.exe` if it is running and never restart it.
- Commits are small and carry no Co-Authored-By or Claude-Session trailers.

## Open Questions

- The CLI question came back unanswered. The plan assumes `hytte shelf add` and `hytte notify`.
- The talking-while-muted alert from 2026-10-04 is not in the code. It stays out unless you want it.

## Status (2026-11-06)

Done: tasks 1 to 29 on `main` (version 1.6.0, checks green), and 32 on the branch `split-big-files` (not merged: merge it after the release). Not done on purpose: 30 (publish) and 31 (winget), which need your go-ahead.

Notes and what was not verified:
- Task 18 needed no change: the Windows animation setting was already followed (`reduce_motion`, live via `WM_SETTINGCHANGE`, tested).
- Not run: a second Windows account (pipe owner check, task 13), two monitors (task 26: it falls back cleanly on one), a phone scan of the QR code (task 24), `AskUserQuestion` against real Claude Code (task 4), Restart on a real dev server (task 23), the GitHub release dry run (needs the `MINISIGN_SECRET_KEY` secret; its signing and verify steps were run locally with the real key).
- The pill is hidden behind a maximised terminal, so the GUI pass was done through the daemon log and the offscreen snapshot tests, not by eye. Screenshots in `docs/img` were not redone.
- The minisign secret key is at `%USERPROFILE%/.hytte-signing/hytte.key` (public half in `update.rs`). Back it up, add it as the `MINISIGN_SECRET_KEY` Actions secret, then remove the local copy if you like.
