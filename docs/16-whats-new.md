# 16. What's new

What changed in each version, newest first. Each version ends with a short note for people who change Hytte.

## 1.7.0

This is the panes and cards release. It includes the 1.6.1 change that releases are no longer signed.

**Live cards and your own panes**

- **Live cards** (`hytte card`). Any script, CI job or app can put a card on the pill with a title, a line of text, a progress bar, an icon (info, ok, warn, error), an **Open** link and up to two buttons. Sending the same `--id` again updates it in place, on the pill or while it waits its turn; a progress card stays until `--done` or 30 s without an update. With `--wait` the command prints the label of the button you press (exit 0), or exits 1 when the card expires, is dismissed or cleared. Buttons only report back; they never run anything. An agent's permission question still wins and puts the card back first in line.
- **Script panes** (`[[script_pane]]` in `config.toml`, up to four). A tab with your own name, showing what a command prints (Hytte runs it hidden, at most every 30 s, only while that pane is open, plus once at start; stopped with everything it started after 10 s) and/or what scripts send it with `hytte pane set <name>`. Colour codes are stripped, six lines are kept, secrets are masked, and a failed command shows in amber with its exit code. Each has a switch (`[panes] script1`…`script4`) and a place in `[panes] order`.

**Look and keys**

- **Accent skin.** A sixth disc that follows the Windows accent colour (which Windows picks from your wallpaper when Colours is on Automatic), kept as dark or light as the other skins so text reads, and changing with the accent while Hytte runs.
- **Key hints.** In keyboard mode a faint line under the tabs says which keys do what (and, while typing, Enter and Esc). With a `hotkey` set, the tray has **Open with keyboard** showing the chord.

**Welcome**

- **A three-card tour, once.** Updaters see what's new in 1.7, fresh installs what Hytte does, the next time they point at the pill (never over games, fullscreen apps, calls, focus or typing). **Done** or **Skip** writes `welcome`, so later 1.7.x releases show nothing.

**Choosing and ordering panes**

- **Every pane but the quick controls has a switch and a place.** `[panes]` now has `media`, `shelf`, `tasks`, `timer` and `power` besides `notes`, `todo` and `weather`, and the settings window's **Panes** section lists all nine with **▲ ▼** to put them in your order (`[panes] order`). The quick controls (Home) are the starting point and cannot be switched off, but can be moved.
- **Pill skin** moved to Look and feel (it was hidden under Behaviour), is in the pill's own settings pane in place of Open delay, and shows colour discs instead of names that did not fit (graphite is called Slate there).
- **New defaults** for fresh installs: pill opacity when open 80 % (was 70 %) and lyrics on. Existing installs keep what their `config.toml` says.
- **Tray → Settings…** opens the settings window, so hiding the Home pane never hides the settings.
- **The settings pane has a clear blue "More settings" button** that opens the settings window, instead of an icon nobody read as that. The window's search box now shows that it has focus (border and caret).

**Weather and lyrics, refined**

- **Search more widely.** If Open-Meteo finds nothing for two seconds, a row offers an OpenStreetMap search, run once when you click it, credited, and cached for the session. It finds places such as Sri City that Open-Meteo lacks.
- **Lyrics skip video and photo apps** (Photos, Movies & TV, Windows Media Player, VLC, mpv, PotPlayer, streaming-video apps): nothing is looked up or sent for them.
- **Live lyrics glide** from line to line instead of switching. A **Live** tick box (`[media] lyrics_live`) turns that off for a scrolling list, and it is greyed out for songs without timed lyrics. Touchpad scrolling on the lyrics page is slower and steadier.

**Fixed**

- **Typing in Notes or To-do over a fullscreen or maximised app no longer collapses the pill.** Hytte taking focus made the fullscreen check think no app was fullscreen; the check now waits until focus is handed back.

**The first panes build**

**New panes**

- **Notes.** Type in the pill (the first time it takes text input): click the field, type, **Enter**. A note lasts 24 hours or is kept (**24 h / Keep**), shows a ring with the time left, and can be copied, kept, edited or deleted with **Undo**. An optional `note_hotkey` opens Notes with the cursor ready from any app.
- **To-do.** The same field; ticked items sink and leave after a day.
- **Weather.** Pick a place with suggestions as you type, then the temperature now, the day's range and the next five hours, with the pill's glow following the sky. Open-Meteo; nothing is fetched while the pane is closed. `[weather] units`.
- All three have switches in `[panes]` and Settings, and show from the start.

**Focus and breaks**

- **A focus session now does something.** It turns on Windows Do Not Disturb (put back afterwards, and after a crash at the next start; `[timer] focus_dnd`), holds Hytte's downloads, finished tasks, agent reply cards and CPU warnings and tells you what you missed in one line, and shows a thin pale line instead of the bright fuse.
- **Break reminders.** Eye (20-20-20, with a countdown ring), stretch and water, as a small card with **Snooze** and **Skip**. Off until `[breaks]` sets an interval. They wait for focus, calls, quiet hours, fullscreen and games, and five minutes away restarts the clocks.

**Music**

- **Sleep timer**: the moon on the Media pane. 15, 30, 45, 60 minutes or the end of the song; the player's volume fades out over the last 20 seconds, it pauses, and the volume comes back.
- **Lyrics** from lrclib.net, on by default (`[media] lyrics`): the line being sung with its neighbours, or the plain words when only a different recording is known.

**Look**

- **Skins**: `graphite` (as before), `aurora`, `ember`, `moss`, `dusk`.
- **`hide_from_capture` is now off for new installs**, so the pill shows in screenshots. Installs from before 1.7 keep what their `config.toml` says.

**Also**

- A user manual, in [`manual/`](../manual/README.md); the tray's **Help** item opens it.
- **Releases are no longer signed.** Hytte 1.6.0 needs a manual download to move on from it ([section 15](15-updates.md#signing)).

For people who change Hytte: the cards-and-panes work adds `scriptpane` (runner and output cleaning), `welcome`, `render/script.rs` and `window/win/scripts.rs`; `hytte_proto::Cmd` gains optional `card` fields, a `pane_set` kind and the `Pressed` reply ([section 4](04-ipc-protocol.md#live-cards-card)); `Panel::Script(u8)` and `Scene::ExpScript(u8)` give script panes four fixed slots. The panes work before it added `textedit`, `jot`, `weather`, `net` (the updater's `curl` moved there), `lyrics`, `sleep`, `breaks`, `skin` and `dnd`, and `render/{jot,weather,nudge}.rs`, `window/win/{typing,forecast,mediactl}.rs`. See [typing, notes and weather in section 6](06-ui-model.md#typing-notes-and-to-dos) and the new recipe in [section 14](14-how-to-recipes.md#add-a-text-field).

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
