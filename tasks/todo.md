# Hytte 1.6.0 tasks

Decisions, risks and test rules are in [plan.md](plan.md). "Checks" means fmt, clippy and tests as
listed there. Each task is one or more small commits.

## Phase 1: Bugs

### 1. Message size limits (M)
One clamp in `HytteMessage::to_line()`, so every Rust sender stays inside the limits.
- [ ] A reply of up to 4000 characters reaches the pill, and the agent row finishes
- [ ] A 20 KB error tail and a long non-English command are trimmed on a character boundary, not dropped
- [ ] The daemon logs each rejected message; `init.ps1` trims its label by bytes
- Verify: unit tests for all three cases; checks
- Files: `proto/src/lib.rs`, `pipe_server.rs`, `cli/shell/init.ps1`

### 2. `hytte run` finds `.cmd` and `.bat` (S)
Search PATH with PATHEXT, then spawn the full path, so Rust's batch-file argument escaping applies.
- [ ] `hytte run -- npm --version` works
- [ ] `.exe` lookup and exit codes are unchanged
- [ ] An argument Rust refuses to pass to a batch file gives a clear error
- Verify: test of the lookup function; run `npm` through a dev build
- Files: `cli/mod.rs`

### 3. Rerun keeps quoting (S)
Quote arguments when building the label, and do not offer Rerun for a command that was cut off.
- [ ] `git commit -m "fix the bug"` reruns with its quotes
- [ ] A command over the label limit has no Rerun button
- Verify: unit tests against `rerun::plan`
- Files: `cli/mod.rs`, `rerun.rs`, `ui_state.rs`
- Depends on: 1

### 4. Allow prompt shows what it approves (M)
- [ ] A command with leading blank lines shows its first real line, and is flagged when lines are hidden
- [ ] File tools show the parent folder as well as the name; MCP tools show the server, tool and a short view of the arguments
- [ ] `AskUserQuestion` shows no Allow / Deny: it shows as waiting, and a click jumps to the terminal
- Verify: unit tests in `cli/agent.rs`; try `AskUserQuestion` against real Claude Code
- Files: `cli/agent.rs`

### 5. Allow click guard (S)
- [ ] Allow and Deny ignore clicks for about 400 ms after an ask appears, including the next queued ask
- [ ] A normal click after the delay works
- Verify: unit test of the guard; double-click two queued asks in a dev build
- Files: `window.rs`, `ui_state.rs`

### 6. Pipe send reliability (S)
- [ ] A client that finds the pipe busy waits briefly and retries, staying fast enough for shell hooks
- [ ] Pending asks can't use up every pipe instance
- [ ] A busy pipe no longer makes `hytte run` try to start a daemon
- Verify: scratch stress test with 50 parallel senders against a dev build, none lost
- Files: `cli/mod.rs`, `pipe_server.rs`

### 7. PowerShell multi-line rows (S)
- [ ] Reproduce first: Enter at a continuation prompt leaves a stuck row
- [ ] One command gives one row, however many lines it has
- Verify: manual test in PowerShell 5.1 and 7
- Files: `cli/shell/init.ps1`

### 8. Stale downloads expire (S)
- [ ] A partial file with no size change for 5 minutes leaves the running list
- [ ] A live download is never expired, and one that resumes comes back
- Verify: unit test with fake clocks
- Files: `downloads.rs`

### 9. `hytte set --done` and `--fail` (S)
- [ ] A script can finish its row green or red
- [ ] Existing `hytte set` calls behave as before
- Verify: unit test of the built message; run against a dev build
- Files: `cli/mod.rs`

### 10. Safe file writes and cleanup (M)
- [ ] `shelf.json` and `timer.json` go through `setup::write_file`
- [ ] Shelf files no list entry points to are removed at startup, but never when `shelf.json` failed to parse
- [ ] One shared shelf mutex; `hytte.exe.new` is removed on failure; `hytte.log` rolls over at 1 MB
- Verify: unit tests, including a corrupted `shelf.json`
- Files: `shelf.rs`, `timer.rs`, `update.rs`, `logging.rs`

### Checkpoint A
- [ ] Checks pass
- [ ] Dev-build GUI pass: agent reply card, failed `hytte run`, Rerun, Allow / Deny

## Phase 2: Hardening

### 11. Updater verification (M)
**You do first:** generate the minisign key, keep an offline backup, add the secret to GitHub Actions.
- [ ] The release job signs `hytte.exe` and uploads the signature
- [ ] Hytte verifies the signature with the built-in key before running the download; a bad or missing signature changes nothing
- [ ] `--version` has a timeout, curl is limited to HTTPS, `HYTTE_UPDATE_API` works in debug builds only
- Verify: tests with good, tampered and missing signatures; workflow dry run
- Files: `update.rs`, `Cargo.toml`, `release.yml`, `scripts/`
- Depends on: 10

### 12. Update prompt chip (M)
- [ ] An update shows a chip with **Update** and **Later**, with no message box and no focus change
- [ ] Nothing shows over a game or while paused; it shows when that ends
- [ ] The tray's manual check still reports up to date and errors
- Verify: fake release against a dev build, including with a fullscreen window in front
- Files: `update.rs`, `window.rs`, `ui_state.rs`
- Depends on: 11

### 13. Per-user pipe and owner check (M)
- [ ] The pipe name includes the user's SID, in daemon, CLI and `init.ps1`
- [ ] The CLI refuses a pipe whose server runs as another user, so an impostor can't answer an ask
- [ ] Two Windows users can each run Hytte
- Verify: unit test of the name; second-account test if possible, otherwise say it wasn't run
- Files: `proto/src/lib.rs`, `proto/src/sys.rs`, `pipe_server.rs`, `cli/mod.rs`, `cli/shell/init.ps1`
- Depends on: 6

### 14. Secret masking (S)
- [ ] Bearer tokens, `password=` / `token=` style values, `-p<password>` and URL credentials show as `•••` on the pill
- [ ] Rerun and Copy keep the real text
- Verify: unit tests for masked and untouched strings
- Files: `ui_state.rs` or a small new module

### 15. Crash relaunch (S)
- [ ] A UI-thread panic logs and relaunches Hytte
- [ ] At most 3 relaunches in 10 minutes, then it stops and logs why
- [ ] A worker thread panic is logged by thread name
- Verify: unit test of the limit; forced panic in a debug build
- Files: `logging.rs`, `main.rs`

### 16. Kill and shelf-copy guards (S)
- [ ] Kill and Stop check that the PID still belongs to the same exe
- [ ] Shelf copy mode doesn't follow junctions or symlinks, and has a size cap with a clear message
- Verify: unit tests, including a junction loop
- Files: `proto/src/ports.rs`, `window.rs`, `shelf.rs`

### 17. CI hardening (S)
- [ ] All actions pinned by commit SHA
- [ ] `cargo audit` runs in CI
- [ ] Write permission only on the jobs that need it
- Verify: CI run on a branch; release dry run
- Files: `.github/workflows/`

### Checkpoint B
- [ ] Checks pass
- [ ] End-to-end update from a local fake release, including a rejected tampered one

## Phase 3: Existing features

### 18. Follow the Windows animation setting (S)
- [ ] With Windows animation effects off, the pill snaps instead of springing and ambient motion stops
- [ ] Changing the setting applies without a restart
- [ ] With animations on, nothing changes
- Verify: toggle the Windows setting against a dev build
- Files: `window.rs`, `animation.rs`

### 19. Real progress from `[N/M]` counts (S)
Extends the stderr scanner only, and accepts bracketed forms so dates and paths don't match.
- [ ] `[12/345]` and `(3/25)` fill the bar; `12/25/2026` and `src/1/2` don't
- [ ] A command with no real numbers keeps its spinner
- Verify: unit tests in `cli/progress.rs`
- Files: `cli/progress.rs`

## Phase 4: New features

### 20. `hytte doctor` (M)
- [ ] Reports daemon, exe path and version, PATH, shell hooks, execution policy, each agent's hooks, and config problems
- [ ] Each failed line says how to fix it; the exit code is non-zero when something is wrong
- Verify: tests of the report; run with the daemon up and down
- Files: new `cli/doctor.rs`, `cli/mod.rs`, `setup.rs`, `agents.rs`

### 21. `hytte shelf add` and `hytte notify` (M)
Additive protocol messages, validated like everything else on the pipe.
- [ ] `hytte shelf add <file>` and piped text land on the shelf
- [ ] `hytte notify "text"` shows a card, and respects quiet hours, games and pause
- Verify: protocol tests; run against a dev build
- Files: `proto/src/lib.rs`, `pipe_server.rs`, `cli/mod.rs`, `window.rs`

### 22. Port labels (S)
- [ ] A dev server shows its project folder name where it can be found, else its exe
- [ ] Docker and WSL listeners are labelled as such
- Verify: unit tests; run a Node server and a Docker container
- Files: `proto/src/ports.rs`, `ui_state.rs`, `render.rs`

### 23. Port Restart button (S)
- [ ] Only offered for a server tied to a tracked terminal command
- [ ] It stops the server and reruns that command in its folder
- Verify: unit test of the offer rule; try it with a real dev server
- Files: `window.rs`, `ui_state.rs`, `render.rs`
- Depends on: 3, 16, 22

### 24. Shelf QR code (M)
- [ ] A text or URL item can show a QR code that a phone can scan
- [ ] Text too long for a QR code gets a clear message
- Verify: unit test; scan with a phone
- Files: `transforms.rs`, `render.rs`, `ui_state.rs`, `Cargo.toml` if a small crate is the simplest route

### 25. Shelf copy-as actions (S)
- [ ] Image items offer **Copy as base64** and **Copy as Markdown**
- [ ] Files over a size cap don't offer base64
- Verify: unit tests in `transforms.rs`
- Files: `transforms.rs`

### 26. Next monitor action (S)
- [ ] A tray item and a hotkey send the pill to the next monitor
- [ ] Hidden on a one-monitor setup; the choice survives a restart
- Verify: test on two monitors, or say it wasn't run
- Files: `window.rs`, `tray.rs`, `config.rs`

### Checkpoint C
- [ ] Checks pass
- [ ] Full GUI pass
- [ ] Idle CPU and memory still match the claims in the README and `docs/12-performance.md`

## Phase 5: Ship

### 27. Final review
- [ ] `/code-review` on the full diff, findings fixed or listed
- [ ] Security review of tasks 4, 5, 11, 13 and 21

### 28. Docs refresh
- [ ] README: what is new, new commands, update checks, config keys
- [ ] `docs/`: protocol, CLI, updater and signing, testing and release, performance, recipes
- [ ] Screenshots redone where the UI changed (update chip, port rows, shelf actions)
- [ ] Landing page text checked against the README
- Verify: every new command and config key is mentioned; every doc link resolves

### 29. Version increment to 1.6.0
- [ ] `scripts/version.ps1 set 1.6.0`, then `scripts/version.ps1 check`
- [ ] `hytte --version` prints `hytte 1.6.0`
- [ ] Release workflow dry run passes, including signing

### 30. Publish the release (needs your go-ahead)
- [ ] `scripts/release.ps1`, tag pushed, assets include the exe, zip and signature
- [ ] The winget hash job commits to `main`
- [ ] A 1.5.0 build updates to 1.6.0
- [ ] Release text says open terminals need reopening

### 31. Submit to winget (needs your go-ahead)
- [ ] Manifest split into version, installer and locale files
- [ ] `winget validate` and a local install pass
- [ ] Pull request opened on `microsoft/winget-pkgs`

## Phase 6: Last priority, after 1.6.0

### 32. Split `render.rs` and `window.rs`, only if safe
On its own branch, after the release, so it can't put 1.6.0 at risk. Moves only: no logic changes.
- [ ] `render.rs` split by pane, then `window.rs` by concern, one small commit per move
- [ ] All tests pass unchanged, and render snapshot output is identical before and after
- [ ] Release binary size and idle numbers don't change meaningfully
- Stop rule: if any check differs and the cause isn't obvious, abandon the branch
