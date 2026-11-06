# 16. What's new

What changed in each version, newest first. The first part is for people who use Hytte; each version ends with a short note for people who change it.

## 1.6.0

**Updates**

- **The update offer is a card on the pill** with **Update** and **Later**, instead of a message box that stole focus. It waits while you are in a game or Hytte is paused, and shows when that ends. The tray's **Check for updates…** answers on the pill too.
- **Updates are checked against a signature.** Each release carries `hytte.exe.minisig`, a [minisign](https://jedisct1.github.io/minisign/) signature made with a key only the release job holds, and Hytte refuses a download that doesn't match the public key built into it. Downloads are HTTPS-only and the version check can't hang. Hytte itself is still not Authenticode-signed, so Windows may warn about a freshly downloaded `hytte.exe`.

**New**

- **`hytte doctor`** checks the pill, `PATH`, the terminal hooks, PowerShell's execution policy, each agent's hooks and `config.toml`, and says how to fix what's wrong.
- **`hytte notify "text"`** shows a card on the pill, and **`hytte shelf add <file>`** (or text piped in) puts things on the shelf, for your own scripts. **`hytte set --done`** and **`--fail`** finish a row green or red.
- **Real build progress from step counts:** `[12/345]` and `(3/25)` fill the bar. A command that prints no numbers keeps its spinner; there are no time estimates.
- **Ports:** a dev server is named after its project folder (when a terminal command Hytte tracks started it), Docker and WSL listeners are named as such, and a server started from a tracked terminal command has a **Restart** button (it stops the server and runs that command again in its folder).
- **Shelf:** a text snippet has **QR code**; an image has **Copy as base64** (a data URI, files up to 2 MB) and **Copy as Markdown**.
- **Next monitor:** the tray's **Move to next monitor** (with two or more monitors) and an optional `monitor_hotkey` move the pill; the choice is kept. `monitor` also takes a number now.

**Safer**

- **One pipe per Windows user**, and the `hytte` command refuses a pipe served by another user, so nobody else can answer an agent's permission request. Open terminals need reopening after the update: they talk to the old pipe name.
- **Secrets are hidden on the pill:** bearer tokens, `password=` / `token=` values, `--token x`, URL credentials, `mysql -pSECRET` and well-known token formats show as `•••` (Rerun and Copy keep the real text).
- **Allow shows what it approves:** a command that starts with blank lines shows its first real line, a file shows its folder, an MCP tool shows its server and input, and clicks within 400 ms of an ask appearing are ignored. An agent's question (not a permission) shows as waiting; click it to go to the terminal.
- **Stop / Kill checks the process is still the one listed**, shelf copies skip junctions and symlinks and stop at 500 MB, and `shelf.json` and `timer.json` are written safely.
- **A crash relaunches Hytte** (at most three times in ten minutes), and panics are logged with their thread.

**Fixes**

- A long agent reply, a long error tail or a long non-English command no longer makes the daemon drop the whole message: text is trimmed to the limits first.
- **`hytte run -- npm ...`** (and any `.cmd` / `.bat` program) works.
- **Rerun keeps quoting** (`git commit -m 'fix the bug'` reruns whole) and isn't offered for a command that was cut off.
- A burst of shell hooks no longer loses messages when the pipe is momentarily busy, and a pending agent ask can't use up every pipe instance.
- **PowerShell:** a multi-line command (an open brace, a trailing pipe, a here-string) is one row, not a stuck one per line.
- A download that stalls for five minutes leaves the list. Leftover files in the shelf folder from a crash are cleaned up at startup, `hytte.log` rolls over at 1 MB and a failed update leaves no `hytte.exe.new`.

**For people who change Hytte**

- New files: `secrets.rs` (the masker), `cli/doctor.rs`. New protocol message: `proto::Cmd`. `proto::pipe_name()` replaces the `PIPE_NAME` constant.
- Release builds ignore `HYTTE_UPDATE_API`; debug builds also read `HYTTE_UPDATE_PUBKEY` and have `hytte __update` ([section 15](15-updates.md)). The release job signs with the `MINISIGN_SECRET_KEY` secret; CI pins actions by commit SHA and runs `cargo audit`.
- Dependencies added: `minisign-verify` and `qrcodegen` (and `minisign` for tests only).

## 1.5.0

**Settings**

- **New defaults** (on a fresh `config.toml`; a file you already have keeps its values): starts with Windows, theme follows Windows, the open pill is 70 % opaque and an idle one 75 %, the pill closes after 0.1 s, the pin button is on, the pill peeks for 3 s when a task finishes, an agent that needs you beeps, resource alerts at 70 % CPU, and the timer has 2 rounds before a long break.
- **Glow strength is a slider** in the settings window, 0 to 100 %, where 100 % is 20 % stronger than the old "Strong". The pill's small pane has steps Off, Soft, Normal and Max.
- **Every setting has a clearer name**, and resting the pointer on one explains it. The names in the pill's small pane are shorter versions of the same.
- **A Custom step on every number** (waits, sizes, opacities, break lengths, thresholds): it opens `config.toml` at that setting with a note, and shows the value when it is none of the steps. The reset (**↺**) now sits in a column at the right of every row and restores the exact default.
- **Per-app volume only lists apps you can see or hear.** Idle background helpers that hold an audio session (an audio console, a wallpaper engine) are left out, and an app is named as it names itself ("Spotify", "Wallpaper Engine").
- **Search and reset in the settings window.** Start typing and only the settings that match stay (a section's name matches all of its rows). Esc clears the search, and closes the window when there is nothing to clear. A small **↺** appears beside any value that isn't its default; one click puts it back.
- **Two new settings.** `pin_button` (on by default from this version) adds a pin to the open pill: click it and the pill stays open until you click it again (it lets go by itself when a fullscreen app takes the screen). `remember_panel` makes `start_panel = "last"` survive a restart.
- The settings window follows a change of the Windows theme or accent colour while it is open.

**Looks**

- A title too long for the pill (a song, a task) now **fades out at its end** instead of being cut with "…". Hover it and it still scrolls.
- A task that failed or is waiting for you gets a faint wash and a bar in its colour, and a finished task's name recedes.
- The pill opens with a little less bounce, and panes cross-fade a touch more softly.

**Fixes**

- **Hovering the small second pill** (the timer's clock, or the music's album art beside a compact task) opens that pane: the timer or the player, not the main pill's. Gliding along the top edge between the pill and its bubble switches the pane to match.
- **Ctrl+C in PowerShell no longer shows a failed task**, for a command that doesn't leave a recognisable exit code too. A failing cmdlet reports 1 instead of an old exit code from the last program. Open a new terminal after updating so its hook is the new one.
- **Screenshots: delete after paste is now on by default.** A screenshot is deleted about two seconds after you paste it. If your `config.toml` already says `delete_after_paste = false`, that stays; switch it on in the settings window.
- A second clipboard update from the Snipping Tool no longer cancels the paste watch.

**Timer and updates**

- The end-of-timer chime is softer: a rising chord with a gentle start, a slow fade and a longer pause.
- The update prompt says what is new in the release ("New in 1.4.0: …") when the release has notes.

**Startup**

- If `config.toml` has a mistake, Hytte says where (line and column) in a warning card you can click to open the file. A setting it doesn't know (usually a typo) is named and ignored. Settings written by older versions that no longer exist are ignored quietly.

**For people who change Hytte**

- `scripts/release.ps1` cuts a release (version bumped everywhere, checks, commit, tag). `scripts/version.ps1` keeps the version in both `Cargo.toml` files, `Cargo.lock` and the winget manifest in step; CI and the release workflow check it. After a release, a job commits the published zip's SHA-256 to the winget manifest ([section 13](13-testing-and-release.md#how-a-release-is-built)).
- The paste decision and the snapping geometry are plain functions with tests.
- Settings rows are described in one place (`render::help`: the hover note, the note "Custom" writes into the file, and what search matches); numbers get their Custom step by themselves ([section 14](14-how-to-recipes.md)).

## 1.3.0

- **A big settings window** (the pane's popout button): every switch, slider and step in sections, with a section list, smooth scrolling and light and dark themes.
- **Nine new settings:** how long the pill waits before it closes, hover or click to open, size, idle opacity, motion speed, which panel it opens on, quiet hours for sounds (with a **Custom** option that opens the file with instructions), and how many task rows show before the list scrolls. Plus a **pill opacity** slider and a **hover delay** (0.2 s by default).
- **Snapped windows** (a window split to half the screen) now give the thin line, like a maximised one.
- **Screenshots** leave the shelf after a minute (`screenshot_secs`), and `delete_after_paste` can delete one when it is pasted.
- **Terminals:** a task row for a closed terminal no longer lingers as an error; a command you stopped with Ctrl+C doesn't show as failed (for programs that report it); the task list scrolls instead of overflowing.
- Fresh screenshots and a documentation pass with every figure checked.

Older versions: see the [releases page](https://github.com/PrathamPai-2007/hytte/releases).

Next: [documentation index](README.md)
