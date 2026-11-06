# 16. What's new

What changed in each version, newest first. Each version ends with a short note for people who change Hytte.

## Unreleased

- **A user manual**, in [`manual/`](../manual/README.md). The tray's new **Help** item opens it.

## 1.6.1

- **Updater no longer verifies a signature.** It still uses HTTPS and checks the download reports the expected version ([section 15](15-updates.md#signing)). Releases still carry `hytte.exe.minisig` so 1.6.0 installs can update to this version.

## 1.6.0

**Updates**

- **The update offer is a card on the pill** with **Update** and **Later**, instead of a message box that stole focus. It waits while you are in a game or Hytte is paused. The tray's **Check for updates…** answers on the pill too.
- **Updates are checked against a signature.** Each release carried `hytte.exe.minisig` (minisign), and Hytte refused a download that didn't match the public key built into it. Downloads are HTTPS-only and the version check can't hang. Hytte itself is not Authenticode-signed, so Windows may warn about a freshly downloaded `hytte.exe`.

**New**

- **`hytte doctor`** checks the pill, `PATH`, terminal hooks, PowerShell's execution policy, each agent's hooks and `config.toml`, and says how to fix what's wrong.
- **`hytte notify "text"`**, **`hytte shelf add <file>`** (or piped text) and **`hytte set --done` / `--fail`** for your own scripts.
- **Real build progress from step counts:** `[12/345]` and `(3/25)` fill the bar. No numbers keeps the spinner; there are no time estimates.
- **Ports:** a dev server is named after its project folder, Docker and WSL listeners are named as such, and a server started from a tracked terminal command has **Restart**.
- **Shelf:** a text snippet has **QR code**; an image has **Copy as base64** (files up to 2 MB) and **Copy as Markdown**.
- **Next monitor:** the tray's **Move to next monitor** and an optional `monitor_hotkey`; the choice is kept. `monitor` also takes a number.

**Safer**

- **One pipe per Windows user**, and `hytte` refuses a pipe served by another user, so nobody else can answer an agent's permission request. Reopen terminals after updating.
- **Secrets are hidden on the pill:** bearer tokens, `password=` / `token=` values, `--token x`, URL credentials, `mysql -pSECRET` and well-known token formats show as `•••` (Rerun and Copy keep the real text).
- **Allow shows what it approves** (first real line, file with folder, MCP server and input), clicks within 400 ms of an ask appearing are ignored, and an agent's question shows as waiting.
- **Stop / Kill checks the process is still the one listed**, shelf copies skip junctions and symlinks and stop at 500 MB, and `shelf.json` / `timer.json` are written safely.
- **A crash relaunches Hytte** (at most three times in ten minutes); panics are logged with their thread.

**Fixes**

- Long agent replies, error tails or non-English commands no longer make the daemon drop the message: text is trimmed first.
- `hytte run -- npm ...` (any `.cmd` / `.bat`) works; Rerun keeps quoting and isn't offered for a cut-off command.
- A burst of shell hooks no longer loses messages when the pipe is busy, and a pending ask can't use up every pipe instance.
- PowerShell: a multi-line command is one row, not a stuck one per line.
- A download stalled for five minutes leaves the list; leftover shelf files from a crash are cleaned at startup; `hytte.log` rolls over at 1 MB; a failed update leaves no `hytte.exe.new`.

**For people who change Hytte:** new files `secrets.rs` and `cli/doctor.rs`; new protocol message `proto::Cmd`; `proto::pipe_name()` replaces `PIPE_NAME`; release builds ignore `HYTTE_UPDATE_API` and debug builds have `hytte __update` ([section 15](15-updates.md)); CI pins actions by commit SHA and runs `cargo audit`; dependency added: `qrcodegen`.

## 1.5.0

- **New defaults** (fresh `config.toml` only): starts with Windows, theme follows Windows, open pill 70 % opaque and idle 75 %, closes after 0.1 s, pin button on, 3 s peek when a task finishes, agents beep when they need you, resource alerts at 70 % CPU, 2 rounds before a long break.
- **Settings window:** glow strength is a slider; every setting has a clearer name and a hover explanation; every number has a **Custom** step that opens `config.toml` at it; a **↺** beside any non-default value restores it; typing searches the settings.
- **Two new settings:** `pin_button` (keep the pill open until clicked again) and `remember_panel`.
- **Per-app volume** lists only apps you can see or hear, named as they name themselves. The settings window follows Windows theme and accent changes.
- **Looks:** a too-long title fades out instead of "…" (hover still scrolls); failed or waiting tasks get a faint wash and bar; the pill opens with less bounce.
- **Fixes:** hovering the second small pill opens its own pane; Ctrl+C in PowerShell no longer shows a failed task (open a new terminal after updating); screenshot delete-after-paste is on by default (an explicit `false` is respected); a second Snipping Tool clipboard update no longer cancels the paste watch.
- **Timer and updates:** a softer end-of-timer chime; the update prompt shows what is new in the release.
- **Startup:** a mistake in `config.toml` is reported with line and column in a clickable card; unknown settings are named and ignored.
- **For people who change Hytte:** `scripts/release.ps1` cuts a release, `scripts/version.ps1` keeps the version (and winget manifest) in step, and a post-release job commits the zip's SHA-256 to the winget manifest ([section 13](13-testing-and-release.md#how-a-release-is-built)); the paste decision and snapping geometry are plain tested functions; settings rows are described in one place (`render::help`).

## 1.3.0

- **A big settings window** with every switch and slider in sections, a section list, smooth scrolling, light and dark.
- **Nine new settings:** close delay, hover or click to open, size, idle opacity, motion speed, start panel, quiet hours, task rows; plus a pill-opacity slider and a hover delay (0.2 s).
- **Snapped windows** give the thin line like a maximised one.
- **Screenshots** leave the shelf after a minute, and `delete_after_paste` can delete one on paste.
- **Terminals:** a closed terminal's row no longer lingers as an error; a Ctrl+C-stopped command doesn't show as failed; the task list scrolls.

Older versions: see the [releases page](https://github.com/PrathamPai-2007/hytte/releases).

Next: [documentation index](README.md)
