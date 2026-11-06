# 11. Configuration and files on disk

## Where things live

Everything Hytte writes is under the current user's profile. Nothing is installed system-wide.

| Path | Written by | Contents |
|---|---|---|
| `%APPDATA%\Hytte\config.toml` | `config.rs` | Settings (below). Created with defaults on first run. |
| `%APPDATA%\Hytte\shelf.json` | `shelf::save` | The shelf list, when `[shelf] persist = true`. |
| `%APPDATA%\Hytte\shelf\` | `shelf::stage` | Copies (in `mode = "copy"`) and dropped text snippets. |
| `%APPDATA%\Hytte\timer.json` | `timer::save` | The running timer, so it survives a restart. Deleted when the timer stops or finishes. |
| `%APPDATA%\Hytte\hytte.log` | `logging.rs` | Panics; a line whenever the fullscreen decision changes; hotkey registration failures; GPU renderer lost or unavailable; startup failures (window, renderer, Start Menu shortcut, `PATH`); migrated shell profiles; update checks, prompts and installs; frame telemetry when `HYTTE_PERF=1`. |
| `%APPDATA%\Hytte\panel.txt` | `Ui::remember_panel` | With `[general] remember_panel`, the name of the panel the pill was on when it last collapsed. |
| `%APPDATA%\Hytte\update-skip` | `update.rs` | The release tag the user chose **Later** for, so the daily check doesn't ask about it again. A newer release is still offered. |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Hytte` | `config::ensure_autostart` | Present only while autostart is on: the quoted path of `hytte.exe`. With autostart off, a copy removes the value only when it points at that copy or at an exe that no longer exists, so a second copy (a dev build) can't switch off the installed one. |
| `HKCU\Environment`, value `Path` | `setup::ensure_on_path` | The Hytte folder is appended once so `hytte` resolves in new terminals (`[general] add_to_path`). |
| `%APPDATA%\Hytte\path.txt` | `setup::ensure_on_path` | The folder last added to `PATH`, so a stale entry can be removed after Hytte moves. |
| PowerShell profiles, `~/.bashrc` | `setup::setup_shells` | Only when the user chooses **Set up terminal integration** or runs `hytte setup`: a block between `# >>> hytte >>>` and `# <<< hytte <<<`. A block from before 1.0 (which ran `notch`) is rewritten at startup by `setup::migrate_profiles`. |
| Agent settings: `~/.claude/settings.json`, `~/.codex/hooks.json`, `~/.config/opencode/plugins/hytte.js`, `~/.gemini/settings.json`, `~/.gemini/config/hooks.json`, `~/.cursor/hooks.json` | `agents::connect` | Only when the user chooses **Connect AI agents** or runs `hytte setup agents`: Hytte's hooks merged in, the rest kept; the user's original saved once as `<file>.hytte-backup`. At startup `agents::repair` re-points them if Hytte moved. See [section 5](05-cli.md#hytte-setup-agents---undo-agentsrs). |
| `hytte.exe.new`, `hytte.exe.old` next to `hytte.exe` | `update.rs` | Only during an update: the download, and the previous exe moved aside. `.old` is deleted at the next start. |
| `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Hytte.lnk` | `config::ensure_start_menu` | A Start Menu shortcut so Windows Search finds Hytte. Rewritten on every launch, so it heals itself if the exe moves. |

`config::data_dir()` (the folder of `config.toml`) is the base for every Hytte file. If `APPDATA` isn't set, it falls back to the current directory.

## How the config is loaded

`config::load()`:

- File missing or empty → write a file with every default and use it.
- File present → parse it with `toml`. Every section and every key has a serde default, so a partial file is fine.
- File present but **invalid** → use the defaults in memory, without overwriting the file, so a typo doesn't destroy the user's settings. The line and column of the mistake are written to `hytte.log` and shown in a warning card (click it to open the file).

### Live reload

Edits apply without a restart. `config::spawn_watcher` blocks on `FindFirstChangeNotificationW` for the data folder (last-write and file-name changes), so it costs nothing while nothing changes. On a notification it waits 150 ms for the editor to finish saving, re-reads `config.toml`, and sends `UiEvent::Config` only if the text changed and parses. Writes to `shelf.json` or `timer.json` in the same folder are filtered out by the text comparison. An invalid file mid-edit is ignored, and the current settings stay until it parses; each time, `config::problem` says what is wrong (`UiEvent::ConfigProblem`: a warning card that opens the file, and a line in the log). The same function finds keys that aren't settings, by comparing the file's keys with those of `Config::default()` serialised (plus the few that only appear once set, and the old names above), so a typo such as `hover_dely_ms` is named instead of silently doing nothing.

On the UI thread, `Ui::apply_config` replaces `self.cfg` and refreshes what was derived from it at startup:

| Setting | How it takes effect |
|---|---|
| `[shell] ignore` | `model.ignore` is rebuilt. |
| `[general] autostart` | `ensure_autostart` updates the `Run` value. |
| `[general] hotkey` | The old hotkey is unregistered and the new one registered. |
| `[general] hide_from_capture` | `SetWindowDisplayAffinity` is called again. |
| `[general] monitor`, `pill_scale` | `refresh_monitor` moves the pill and recomputes its scale. |
| `[general] monitor_hotkey` | The old chord is unregistered and the new one registered (with `hotkey`). |
| `[general] theme`, `adaptive_glow`, `glow_strength`, `pill_opacity`, `idle_opacity`, `animation`, `open_on`, `hover_delay_ms`, `collapse_delay_ms`, `start_panel`, `remember_panel`, `pin_button`, `quiet_hours`, `task_rows` | Re-read; the next frame uses them. Switching `pin_button` off un-pins. |
| `[timer] rounds` | The running Focus cycle shows the new number of dots. |
| `[ports]` | `ports::reconfigure` hands the new settings to the port watcher and wakes it for an immediate rescan. |
| `[hog]` | `ports::reconfigure_hog` hands them to the same thread, used from the next tick. |
| `[general]` fullscreen keys | `evaluate_fullscreen` runs again. |
| Everything else | Already read at the point of use (`self.cfg`, or `config::load()` for `output_folder`). |

Lowering `[shelf] max_items` keeps the tiles already on the shelf; it only limits new ones. `[general] add_to_path` and `check_updates` only matter at the next startup. Keys Hytte no longer knows (`renderer`, `solid_pill`, `acrylic`, `config_version`, `top_offset`) are ignored without a word. Any other key it doesn't know is ignored too, but named (see below).

### Writing the file

Hytte only writes `config.toml` in four cases: creating it on first run (`config::save`), the tray's **Launch at startup** toggle, the settings pane in the pill and the settings window. The last three go through `config::set`, which edits only the one key with `toml_edit`, so the user's comments, ordering and other keys are kept. If the file doesn't parse, it isn't touched (the pane says so). The fourth case is **Custom** on a setting only the file can hold (`quiet_hours`): `config::ensure_noted` adds the key if it is missing and puts a comment above it saying what it takes, once, before the file is opened for the user. Both writes go through a temp file and a rename, so the watcher never sees a half-written file.

## Every setting

### `[general]`

| Key | Default | Meaning |
|---|---|---|
| `autostart` | `true` | Start Hytte at login (the `Run` registry value). Applied at every startup and when toggled from the tray. |
| `add_to_path` | `true` | At startup, add the folder holding `hytte.exe` to the user's `PATH` if `hytte` isn't found already. Set to `false` to manage `PATH` yourself. |
| `monitor` | `"primary"` | Where the pill hangs: `"primary"`, `"active"` (the foreground window's monitor) `"cursor"` (the monitor under the pointer) or a number (`"1"`, `"2"`, …: the monitors from left to right). The tray's **Move to next monitor** (shown with two or more monitors) and `monitor_hotkey` set the number for you, and it is kept across restarts. `active` and `cursor` are re-checked when the foreground window changes ([section 7](07-window-and-input.md#monitor-and-dpi)). |
| `theme` | `"auto"` | `"dark"`, `"light"`, or `"auto"` (follow the Windows app mode). Applies live. |
| `glow_strength` | `1.2` | Multiplier for the border glow around the pill: `1.0` is the original look, `0` turns it off, values above `3` are capped. The settings window's slider runs 0 to 1.92 (shown as 0 to 100 %), 20 % past the old "Strong" (1.6). |
| `pill_opacity` | `0.7` | Opacity of the pill's body, `0.4` to `1.0`. Text and icons stay solid. Applies live. |
| `pin_button` | `true` | Show a pin in the open pill's tab strip. Clicking it keeps the pill open until it is clicked again (or a fullscreen app takes the screen). The pinned state is not saved. |
| `remember_panel` | `false` | With `start_panel = "last"`, remember the panel across restarts (`panel.txt`). |
| `hover_delay_ms` | `200` | How long the pointer must rest on the pill (or the sentinel line) before it opens; `0` opens at once. |
| `collapse_delay_ms` | `100` | How long the pill stays open after the pointer leaves it. |
| `open_on` | `"hover"` | `"hover"` opens when the pointer rests on the pill; `"click"` waits for a click on the closed pill. |
| `pill_scale` | `1.0` | Size of the pill, `0.7` to `1.6`. |
| `idle_opacity` | `0.75` | The idle pill (nothing to show) fades to this, `0.2` to `1.0`, until it is pointed at. |
| `animation` | `"normal"` | `"calm"`, `"normal"` or `"snappy"`: how fast the pill moves. |
| `start_panel` | `"last"` | The panel it opens on: `"last"` (the one you left), `"auto"` (what is happening, else the quick controls) or `"home"`. |
| `quiet_hours` | `""` | No sounds (agent alerts, the timer alarm) inside this window, for example `"22:00-07:00"`; it may wrap past midnight. Empty = never. The settings window offers four presets and **Custom**, which writes the key into the file with a note above it and opens it. |
| `task_rows` | `4` | Task rows shown before the list scrolls (`2` to `8`). |
| `adaptive_glow` | `true` | Tint the media glow with the dominant colour of the album art (greyscale art keeps the white glow). |
| `monitor_hotkey` | `""` | A global chord such as `"Win+Alt+M"` that sends the pill to the next monitor (round to the first after the last). Empty = off. Needs a modifier and one key, like `hotkey`. |
| `hotkey` | `""` | A global chord such as `"Win+Alt+N"` that opens the pill for keyboard use ([section 7](07-window-and-input.md#keyboard)). Empty = off. Needs a modifier and one key. |
| `fullscreen` | `"line"` | Over a maximised window (whether or not the taskbar auto-hides), or a fullscreen browser / media player: `"line"` shrinks to a 3 px bar, `"hide"` fades out completely, `"off"` stays as usual. Real fullscreen apps (games) always hide fully ([section 7](07-window-and-input.md#fullscreen-suppression)), except with `"off"`. Settings from before 1.2.2 (`suppress_fullscreen = false`, `fullscreen_mode = "hide"`) are read as `"off"` / `"hide"`. |
| `finish_peek_secs` | `3` | Seconds the pill opens to show a task that finished or failed. `0` keeps it closed. |
| `check_updates` | `true` | Once a day (first 60 s after start), ask GitHub whether a newer release exists and offer to install it ([section 15](15-updates.md)). The tray's **Check for updates…** works either way. Live: while off, the daily check is skipped. |
| `hide_from_capture` | `true` | Keep the pill out of screenshots, screen recordings and screen share (`SetWindowDisplayAffinity` with `WDA_EXCLUDEFROMCAPTURE`). You still see it on your own screen. Applies live. |
| `output_folder` | none | Folder for Drop Vault results; if unset or missing, results go next to the source file. |
| `allow_list` | `[]` | Exe names (case-insensitive) that never trigger suppression, e.g. `"Code.exe"`. |
| `deny_list` | `[]` | Exe names that always get game mode while in front (for a game that isn't detected, or a browser game you want treated as one). |

### `[shell]`

| Key | Default | Meaning |
|---|---|---|
| `threshold_ms` | `3000` | Shell-hook commands are shown only after running this long. |
| `ignore` | `vim`, `nvim`, `vi`, `nano`, `less`, `more`, `man`, `ssh`, `top`, `htop`, `tmux`, `fzf`, `git-credential-manager` | Command names never shown. Compared against the first word of the command, lower-cased, without path or `.exe`. |

### `[shelf]`

| Key | Default | Meaning |
|---|---|---|
| `mode` | `"reference"` | `"reference"` or `"copy"`; see [section 10](10-shelf-and-drop-vault.md#two-modes). |
| `max_items` | `12` | Maximum tiles on the shelf. |
| `persist` | `true` | Save the shelf to `shelf.json` and restore it at startup. |
| `remove_after_drag` | `true` | Remove a tile after it has been dragged out successfully. |
| `screenshot_secs` | `60` | A screenshot leaves the shelf (and its PNG is deleted) after this many seconds; `0` keeps it. Applies to screenshots taken while Hytte runs. |
| `delete_after_paste` | `true` | Delete a screenshot about two seconds after you paste it with Ctrl+V while it is still on the clipboard. (A file written by an older version may say `false` explicitly; that is respected.) Only Hytte's own copy is deleted, and only while it is still on the shelf. |
| `screenshots` | `true` | A screenshot the Snipping Tool copies (Win+Shift+S) goes on the shelf as a PNG in `%APPDATA%\Hytte\shelf\` (`WM_CLIPBOARDUPDATE`, clipboard owner `SnippingTool.exe` or `ScreenClippingHost.exe`). Images other apps copy are left alone; nothing is shelved in game mode or while paused. Applies live. |

### `[ports]`

| Key | Default | Meaning |
|---|---|---|
| `watch` | `3000, 3001, 4200, 5000, 5173, 5432, 8000, 8080, 8888` | Ports shown under **SERVERS** in the Tasks panel. |
| `show_all` | `false` | Show every non-system listener instead. |
| `poll_secs` | `5` | Rescan interval (minimum 1). Opening the pill always rescans immediately. |

### `[agent]`

| Key | Default | Meaning |
|---|---|---|
| `peek_secs` | `6` | How long the pill peeks open when a task needs input. |
| `sound` | `true` | Play the system "asterisk" sound on `NeedsInput`. |

### `[hog]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Warn when one process keeps using a lot of CPU or memory ([section 9](09-workers.md#resource-hogs-hogrs-hog)). |
| `cpu_pct` | `70` | Percent of the **whole machine** (all cores = 100) that has to be exceeded. |
| `secs` | `30` | How long the CPU has to stay over the limit. |
| `mem_mb` | `4096` | Working set in MB; memory alerts at once. |

### `[downloads]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `true` | Show browser downloads as tasks. Switching it on starts the watcher; off, its tasks are hidden (the watcher stays until restart). |
| `folder` | none | Folder to watch (default: your Downloads folder). Read at startup. |

### `[calendar]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Show a heads-up before the next calendar event. Off by default: titles are private and the pill is on screen. Switching it on starts reading the calendar; off, heads-ups are dropped (the reader stays until restart). |
| `lead_min` | `10` | Minutes before the start to show it. |

### `[timer]`

| Key | Default | Meaning |
|---|---|---|
| `sound` | `true` | Ring when a timer ends, until **Stop alarm** (2 minutes at most). |
| `break_min` | `5` | Short break length after a Focus session. |
| `long_break_min` | `15` | Long break length. |
| `rounds` | `2` | Every `rounds`-th Focus session is followed by a long break. A Focus or Break timer shows this many round dots. |

## The timer's persistence

`timer.rs` stores the end time as **wall-clock** milliseconds since the Unix epoch (`ends_ms`), or the remaining milliseconds while paused (`paused_ms`). A restart or a sleep doesn't lose time, and a timer that ended while the PC was off fires shortly after Hytte starts. The alarm is a tiny WAV synthesised in code (`chime_wav`: three decaying bell notes and a pause) and looped asynchronously from memory (`PlaySound` with `SND_LOOP`), so there is no audio file to ship. `timer::ring(false)` stops it. The end times on the timer and the finished card ("ends 14:35", "ended 2:35 PM") come from `timer::clock`, which converts the wall-clock milliseconds to local time and formats them with `GetTimeFormatEx`, so they follow the user's 12 / 24-hour setting.

## Environment variables

| Variable | Effect |
|---|---|
| `HYTTE_PERF=1` | Log frame-time statistics to `hytte.log` ([section 12](12-performance.md)). |
| `HYTTE_WT_PID` | Used only by the ignored manual test in `tabs.rs`. |
| `HYTTE_UPDATE_API` | Replaces the GitHub "latest release" URL the updater asks, for testing ([section 15](15-updates.md#testing-an-update)). `file://` URLs work. |

## Adding a setting

See [section 14](14-how-to-recipes.md#add-a-setting). Always give the new key a serde default, so existing config files keep loading.

Next: [12. Performance](12-performance.md)
