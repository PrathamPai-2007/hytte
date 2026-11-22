# 11. Configuration and files on disk

## Where things live

Everything Hytte writes is under the current user's profile; nothing is system-wide.

| Path | Written by | Contents |
|---|---|---|
| `%APPDATA%\Hytte\config.toml` | `config.rs` | Settings (below), created with defaults on first run. |
| `…\shelf.json`, `…\shelf\` | `shelf.rs` | The shelf list (`[shelf] persist`); copies (`mode = "copy"`) and text snippets. |
| `…\timer.json` | `timer::save` | The running timer, so it survives a restart; deleted when it stops or finishes. |
| `…\jot.json` | `jot::Store::save` | Notes and to-dos (`jot.json.bad` is a damaged file set aside). |
| `…\position.json` | `position.rs` | Where the pill sits sideways, per monitor (`\\.\DISPLAY1`: fraction of the monitor's width from the middle). State, not a setting; absent means centred. |
| `…\place.json` | `weather::Place::save` | The Weather pane's chosen place and coordinates. |
| `…\dnd-restore.txt` | `dnd.rs` | Only while a Focus session has Do Not Disturb on: the earlier value, restored at the next start if Hytte died. |
| `…\sleep-restore.txt` | `sleep.rs` | Only during a sleep fade: the player and its volume, restored at the next start if Hytte died. |
| `…\hytte.log` | `logging.rs` | Panics; fullscreen decision changes; hotkey, GPU and startup failures; migrated profiles; update activity; rejected pipe messages; frame telemetry with `HYTTE_PERF=1`. At startup a log over 1 MB is renamed `hytte.log.old` (one old log is kept). |
| `…\panes\<name>\pane.toml` | you (or a pack you copied) | A pane pack: a script pane, its actions and its scripts ([section 17](17-writing-panes.md)). The folder is created at startup and watched. |
| `…\pack-answers\<name>.approved` / `.ignored` | `packs::answer` | Your Run / Ignore for a pack: the FNV-1a hash of the `pane.toml` you saw. Outside the pack, so a pack cannot bring its own answer. An edited `pane.toml` no longer matches and asks again. |
| `…\mcp.log` | `agent_guard::audit` | One line per agent write or action attempt: unix seconds, tool, target, result (tab-separated, fields cut to 200 characters). About 256 KB, then one `mcp.log.old` ([section 18](18-agent-control-and-registry.md)). |
| `…\mcp-grants\<action>.grant` | `agent_guard::grant_in` | An action you allowed an agent to run without the card (`hytte mcp grant`): the hash of its name, command and folder. A changed command no longer matches and asks again. |
| `…\panes\<name>\.hytte-agent` | `cli/mcp_panes.rs` | Marks a pack folder an agent made, so only that folder can be changed or removed by an agent. |
| `…\panes\<name>\.hytte-pack.json` | `registry.rs` | Marks a pack `hytte pack install` made: name, version, source and the files it brought, so `update` and `remove` touch only those. |
| `…\pane-slots.json` | `packs::save_slot_names` | The pane names last held by the custom-pane slots (`["A","B"]`), so a changed list can move each pane's switch and place with it ([section 17](17-writing-panes.md#the-pane-cap-slots-and-switches)). |
| `…\panel.txt` | `Ui::remember_panel` | With `remember_panel`, the panel the pill last collapsed on. |
| `…\welcome` | `welcome::finish` | Present once the welcome tour (fresh installs only) was finished or skipped. |
| `…\update-skip` | `update.rs` | The release tag the user chose **Later** for (a newer one is still offered). |
| `…\path.txt` | `setup::ensure_on_path` | The folder last added to `PATH`, so a stale entry can be removed after Hytte moves. |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Hytte` | `config::ensure_autostart` | Present only while autostart is on: the quoted exe path. With autostart off, a copy removes it only if it points at that copy or a missing exe, so a dev build can't switch off the installed one. |
| `HKCU\Environment`, value `Path` | `setup::ensure_on_path` | The Hytte folder, appended once (`add_to_path`). |
| PowerShell profiles, `~/.bashrc` | `setup::setup_shells` | Only on **Set up terminal integration** / `hytte setup`: a block between `# >>> hytte >>>` and `# <<< hytte <<<`. A pre-1.0 block (which ran `notch`) is rewritten at startup (`migrate_profiles`). |
| Agent files: `~/.claude/settings.json`, `~/.codex/hooks.json`, `~/.config/opencode/plugins/hytte.js`, `~/.gemini/settings.json`, `~/.gemini/config/hooks.json`, `~/.cursor/hooks.json` | `agents::connect` | Only on **Connect AI agents** / `hytte setup agents`: Hytte's hooks merged in, the original saved once as `<file>.hytte-backup`; `agents::repair` re-points them if Hytte moved ([section 5](05-cli.md#hytte-setup-agents---undo-agentsrs)). |
| `hytte.exe.new`, `hytte.exe.old` | `update.rs` | Only during an update; `.old` is deleted at the next start. |
| `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Hytte.lnk` | `config::ensure_start_menu` | Start Menu shortcut, rewritten every launch so it heals if the exe moves. |

`config::data_dir()` (the folder of `config.toml`) is the base for every Hytte file; without `APPDATA` it falls back to the current directory.

## How the config is loaded

`config::load()`: a missing or empty file is written with every default; a present file is parsed with `toml` (every key has a serde default, so a partial file is fine); a present but **invalid** file means defaults in memory **without overwriting the file**, so a typo doesn't destroy settings. The line and column go to `hytte.log` and a warning card (click to open the file).

### Live reload

Edits apply without a restart. `config::spawn_watcher` blocks on `FindFirstChangeNotificationW` for the data folder, so it costs nothing while idle. On a notification it waits 150 ms for the editor to finish, re-reads `config.toml`, and sends `UiEvent::Config` only if the text changed and parses (writes to `shelf.json` / `timer.json` are filtered out by that comparison). An invalid file mid-edit is ignored and the current settings stay; each time `config::problem` says what's wrong (`UiEvent::ConfigProblem`: a warning card plus a log line). It also names keys that aren't settings, by comparing against the keys of `Config::default()` serialised (plus a few that only appear once set, and the old names below), so a typo like `hover_dely_ms` is named instead of silently ignored.

`Ui::apply_config` replaces `self.cfg` and refreshes derived state:

| Setting | How it takes effect |
|---|---|
| `[shell] ignore` | `model.ignore` rebuilt. |
| `autostart` | `ensure_autostart` updates the `Run` value. |
| `hotkey`, `monitor_hotkey` | Old chord unregistered, new one registered. |
| `hide_from_capture` | `SetWindowDisplayAffinity` again. |
| `[panes]` | `Model::apply_panes`; a hidden pane that was selected falls back to Home. |
| `[[script_pane]]`, `[[action]]` | `Ui::refresh_panes`: the effective panes and actions are rebuilt (config first, then approved packs), `Model::apply_scripts` keeps what a pane that kept its name showed, and a changed command runs again. |
| `skin` | Read each frame. |
| `note_hotkey` | Old chord unregistered, new one registered (hotkey id 3). |
| `[breaks]` | `Breaks::configure`: unchanged intervals keep their clocks, newly enabled ones start now. |
| `[weather] units`, `[media] lyrics` | Read at the point of use. |
| `monitor`, `pill_scale` | `refresh_monitor` moves the pill and recomputes scale. |
| `theme`, `adaptive_glow`, `glow`, `glow_strength`, `pill_opacity`, `idle_opacity`, `animation`, `open_on`, `hover_delay_ms`, `collapse_delay_ms`, `start_panel`, `remember_panel`, `pin_button`, `quiet_hours`, `task_rows` | Re-read; next frame uses them. Switching `pin_button` off un-pins. |
| `[timer] rounds` | The running Focus cycle shows the new dot count. |
| `[ports]`, `[hog]` | `ports::reconfigure` / `reconfigure_hog` hand them to the port watcher and wake it for an immediate rescan. |
| fullscreen keys | `evaluate_fullscreen` runs again. |
| Everything else | Read at the point of use (`self.cfg`, or `config::load()` for `output_folder`). |

Lowering `[shelf] max_items` keeps existing tiles and only limits new ones. `add_to_path` and `check_updates` matter at next startup. Old keys (`renderer`, `solid_pill`, `acrylic`, `config_version`, `top_offset`, `suppress_fullscreen`, `fullscreen_mode`) are ignored silently; any other unknown key is ignored but named, including an unknown key inside a `[[script_pane]]` (`script_pane.comand`).

### Writing the file

Hytte writes `config.toml` only to create it on first run (`config::save`), for the tray's **Launch at startup** and the settings window. The last two use `config::set`, which edits one key with `toml_edit` so comments, order and other keys are kept; an unparseable file isn't touched (a card says so). **Custom** on a file-only setting (`quiet_hours`) uses `config::ensure_noted`, which adds the key if missing with a comment above saying what it takes, once, before opening the file. Writes go through a temp file and rename, so the watcher never sees half a file.

## Every setting

### `[general]`

| Key | Default | Meaning |
|---|---|---|
| `autostart` | `true` | Start at login (the `Run` value). |
| `add_to_path` | `true` | At startup, add `hytte.exe`'s folder to the user `PATH` if `hytte` isn't found. `false` to manage `PATH` yourself. |
| `monitor` | `"primary"` | `"primary"`, `"active"` (foreground window's monitor), `"cursor"` or a number (`"1"`, `"2"`, left to right). The tray's **Move to next monitor** and `monitor_hotkey` set the number, kept across restarts ([section 7](07-window-and-input.md#monitor-and-dpi)). |
| `theme` | `"auto"` | `"dark"`, `"light"`, or `"auto"` (Windows app mode). |
| `glow` | `false` | The border glow. Off by default since 1.9.1; `General::glow_gain()` is `glow_strength` while on, else 0. Dragging the strength slider switches it on. |
| `glow_strength` | `1.75` (70 %) | Border glow multiplier while `glow` is on: `1.0` original, `0` none, capped at `3`. The settings slider runs 0–2.5 (shown 0–100 %). |
| `pill_opacity` | `0.9` | Pill body opacity `0.4`–`1.0`; text and icons stay solid. |
| `pin_button` | `true` | A pin in the tab strip keeps the pill open until clicked again (or a fullscreen app takes over). Not saved. |
| `remember_panel` | `false` | With `start_panel = "last"`, remember the panel across restarts. |
| `hover_delay_ms` | `200` | Rest time before the pill (or sentinel) opens; `0` = at once. |
| `collapse_delay_ms` | `100` | How long it stays open after the pointer leaves. |
| `open_on` | `"hover"` | `"hover"` or `"click"` (a click on the closed pill). |
| `pill_scale` | `1.0` | Pill size, `0.7`–`1.6`. |
| `idle_opacity` | `0.75` | The idle pill fades to this (`0.2`–`1.0`) until pointed at. |
| `animation` | `"normal"` | `"calm"`, `"normal"` or `"snappy"`. |
| `start_panel` | `"last"` | `"last"`, `"auto"` (what is happening, else Home) or `"home"`. |
| `quiet_hours` | `""` | No sounds (agent alerts, timer alarm) in this window, e.g. `"22:00-07:00"` (may wrap midnight). Empty = never. The settings window has presets and **Custom**. |
| `task_rows` | `4` | Task rows before the list scrolls (`2`–`8`). |
| `adaptive_glow` | `true` | Tint the media glow with the album art's dominant colour (greyscale art keeps white). |
| `hotkey`, `monitor_hotkey` | `""` | Global chords, e.g. `"Win+Alt+N"` (keyboard use, [section 7](07-window-and-input.md#keyboard)) and `"Win+Alt+M"` (next monitor, wrapping). Need a modifier and one key. Empty = off. |
| `fullscreen` | `"line"` | Over a maximised window or fullscreen browser / player: `"line"` (3 px bar), `"hide"` (fade out), `"off"`. Games always hide fully unless `"off"` ([section 7](07-window-and-input.md#fullscreen-suppression)). Pre-1.2.2 `suppress_fullscreen = false` / `fullscreen_mode = "hide"` read as `"off"` / `"hide"`. |
| `finish_peek_secs` | `3` | How long the pill opens for a finished or failed task; `0` = stays closed. |
| `check_updates` | `true` | Daily update check, first 60 s after start ([section 15](15-updates.md)). The tray item works either way. Live. |
| `hide_from_capture` | `false` | Keep the pill out of screenshots, recordings and screen share (`WDA_EXCLUDEFROMCAPTURE`); you still see it. Live. The default was `true` before 1.7; a file that says `true` keeps it. |
| `note_hotkey` | `""` | Global chord that opens Notes with the cursor ready, e.g. `"Win+Alt+J"`. Empty = off. Live. |
| `skin` | `"graphite"` | `"graphite"`, `"aurora"`, `"ember"`, `"moss"`, `"dusk"`, `"accent"` (the Windows accent colour; graphite until it is known) or a colour `"#rrggbb"` (the custom skin: only its hue and saturation count, made dark or light like the accent skin by `skin::accent_body`); anything else is graphite. Live. The settings show it as seven colour discs (graphite is named **Slate** there); with a colour, **Skin colour** (hue) and **Vividness** (saturation) sliders sit under them and write `"#rrggbb"` on release. |
| `output_folder` | none | Drop Vault results folder; unset or missing = next to the source. |
| `allow_list` / `deny_list` | `[]` | Exe names (case-insensitive) that never trigger suppression, e.g. `"Code.exe"` / always get game mode while in front. |

### `[shell]`

| Key | Default | Meaning |
|---|---|---|
| `threshold_ms` | `3000` | Shell commands show only after this long. |
| `ignore` | `vim`, `nvim`, `vi`, `nano`, `less`, `more`, `man`, `ssh`, `top`, `htop`, `tmux`, `fzf`, `git-credential-manager` | Command names never shown (first word, lower-cased, no path or `.exe`). |

### `[shelf]`

| Key | Default | Meaning |
|---|---|---|
| `mode` | `"reference"` | `"reference"` or `"copy"` ([section 10](10-shelf-and-drop-vault.md#shelf-items)). |
| `max_items` | `12` | Maximum tiles. |
| `persist` | `true` | Save to `shelf.json` and restore at startup. |
| `remove_after_drag` | `true` | Remove a tile after a successful drag-out. |
| `screenshots` | `true` | Snipping Tool screenshots (Win+Shift+S) go on the shelf as PNGs in `%APPDATA%\Hytte\shelf\` (clipboard owner `SnippingTool.exe` or `ScreenClippingHost.exe`); other apps' images are left alone; nothing in game mode or paused. Live. |
| `screenshot_secs` | `60` | A screenshot leaves the shelf (PNG deleted) after this long; `0` keeps it. Only for screenshots taken while Hytte runs. |
| `delete_after_paste` | `true` | Delete a screenshot ~2 s after you Ctrl+V it while still on the clipboard. Only Hytte's own copy, only while still on the shelf. An older file may say `false` explicitly; that is respected. |

| `shelve_copied` | `false` | Files you copy in Explorer (Ctrl+C; `Preferred DropEffect` says copy, not cut) go on the shelf **by reference**, whatever `mode` says. Apps that put a file list on the clipboard without a drop effect, and apps that mark their copy private, are skipped. Live. |

### Other sections

| Section / key | Default | Meaning |
|---|---|---|
| `[ports] watch` | `3000, 3001, 4200, 5000, 5173, 5432, 8000, 8080, 8888` | Ports shown under **SERVERS**. |
| `[ports] show_all` / `poll_secs` | `false` / `5` | Show every non-system listener / rescan interval (min 1; opening the pill always rescans). |
| `[agent] peek_secs` / `sound` | `6` / `true` | How long the pill peeks when a task needs input / play the asterisk sound on `NeedsInput`. |
| `[agent] quiet_after_secs` | `300` | A working agent (with a live process) that has reported nothing for this long shows a grey "quiet 6m" in the Agents pane. A label only: no sound, no alert. `0` = never. Live. |
| `[ai] enabled` | `true` | The Ask pane and the Explain / Summarise / Shorten actions ([manual 9](../manual/09-ask.md)). Off removes the tab and forgets the answers. Nothing contacts Ollama until the Ask tab is opened. Live. |
| `[ai] host` | `127.0.0.1:11434` | Where Ollama listens. Must be this PC (`127.0.0.1`, `localhost`, `[::1]`, optional port); anything else is refused ([section 9](09-workers.md#ask-ollamars-ai)). Live. |
| `[ai] model` | `qwen3:1.7b` | The model asked (the 1.4 GB default; Settings offers it and `qwen3:0.6b`; any other pulled model works, set here). Live. |
| `[ai] keep_alive` | `2m` | How long Ollama keeps the model loaded after a question (`1m`, `10m` ...). Live. |
| `[ai] hotkey` | `""` | Global hotkey that opens the Ask pane with the cursor ready, e.g. `Win+Alt+A`. Empty = off. |
| `[hog] enabled` | `true` | Warn about a process using lots of CPU or memory ([section 9](09-workers.md#resource-hogs-hogrs-hog)). |
| `[hog] cpu_pct` / `secs` / `mem_mb` | `70` / `30` / `4096` | Percent of the **whole machine** (all cores = 100) for this long; working set in MB, alerting at once. |
| `[downloads] enabled` / `folder` | `true` / none | Show browser downloads as tasks (switching on starts the watcher; off hides its tasks until restart) / folder to watch (default Downloads; read at startup). |
| `[calendar] enabled` / `lead_min` | `false` / `10` | Heads-up before the next event (off by default: titles are private; switching on starts reading; off drops heads-ups until restart) / minutes before the start. |
| `[timer] sound` | `true` | Ring when a timer ends until **Stop alarm** (2 min max). |
| `[timer] focus_dnd` | `true` | A Focus session turns on Windows Do Not Disturb and puts it back ([section 9](09-workers.md#do-not-disturb-dndrs)). Hytte holds its own alerts during Focus either way. |
| `[panes] media` / `shelf` / `tasks` / `timer` / `power` / `weather` (`true`), `notes` / `todo` / `ask` (`false` since 1.9.1) | see left | Show that pane's tab (Music, Shelf, Tasks, Timer and Battery still only when they have something to show). The quick controls (Home) have no switch: they always show. |
| `[panes] order` | `""` | The tab order as names ("home,media,..."). Set by the arrows in the settings window; names left out keep their usual place after those listed. |
| `[panes] script1` … `script12` | `true` | Switch for each custom pane, by its slot (the place among `[[script_pane]]`s, then approved packs); written only when off. Their keys in `order` are the same names. Hytte moves a pane's switch and place when slots shift (`packs::remap_slots`, `pane-slots.json`). |
| `[[script_pane]] name` | none | The tab's label (cut to 16 characters; unnamed and repeated names are skipped; at most 12 custom panes in all, config panes first, then approved packs). |
| `[[script_pane]] command` | `""` | Run with `cmd /d /s /c`, hidden, in your home folder, while the pane is open (and once at start), with `HYTTE_ARG` empty; stopped after 10 s with everything it started. Empty: only `hytte pane set` fills the pane. |
| `[[script_pane]] every` | `300` | Seconds between runs while the pane is open; 30 at least. |
| `[panes] agents` | `true` | The Agents pane (coding agents listed apart from terminal tasks). Off: agents list in Tasks. With this and `tasks` both off, agents show nowhere (not in the compact pill either). |
| `[moments] bluetooth` / `privacy` | `true` / `true` | A card when a Bluetooth device connects (with its battery when it reports one) / a 3 s card naming the app that starts using the camera or microphone (not during Focus or quiet hours). Bluetooth starts its watcher the first time it is on. |
| `[clipboard] history` | `false` | A text history under the shelf tiles: 20 clips, 4 KB each, memory only, cleared on lock, sleep and switch-off; never text `secrets::mask` would change. |
| `[clipboard] expire_secs` | `120` | Seconds before a clip is forgotten; `0` keeps it until lock, sleep or switch-off. Live. |
| `[clipboard] clear_when_emptied` | `false` | Forget the whole history when the Windows clipboard is emptied. Live. |
| `[clipboard] ignore_apps` | `keepass`, `keepassxc`, `1password`, `bitwarden` | Programs whose copies are never kept (exe name, any case, `.exe` optional). |
| `[[action]] name` / `command` | none | A command a card button (`Label:name`), a script pane row or a pane button (`"action": "name"`) may run, hidden, in your home folder, with the row's `arg` in `HYTTE_ARG`. A name is letters, digits, `-` and `_` (40 at most); repeats and empty commands are skipped; 32 actions at most. |
| `[[action]] confirm` / `timeout` | `false` / `60` | `true`: the first click arms ("Sure?"), the second runs. Seconds until it is stopped with everything it started (1 to 600). |
| `%APPDATA%\Hytte\panes\<name>\pane.toml` | none | A pane pack: `name`, `command`, `every` and its own `[[action]]`s (at most 8, names up to 19 characters, run in the pack's folder; [section 17](17-writing-panes.md)). Runs only after you press **Run** on the card; your answer is kept in `%APPDATA%\Hytte\pack-answers\` as a hash of the file you saw. |
| `[weather] units` | `"auto"` | `"auto"` (the Windows measurement setting), `"metric"` or `"imperial"`. |
| `[media] lyrics` | `true` | Look songs up on lrclib.net while the lyrics page is open (sends title and artist). |
| `[media] lyrics_live` | `true` | Follow the song line by line when the lyrics are timed; off shows the words as a list. |
| `[breaks] eyes_min` / `stretch_min` / `water_min` | `0` / `0` / `0` | Minutes between each reminder; 0 = off. |
| `[timer] break_min` / `long_break_min` / `rounds` | `5` / `15` / `2` | Short and long break lengths; every `rounds`-th Focus is followed by a long break, and a Focus or Break timer shows that many round dots. |

## The timer's persistence

`timer.rs` stores the end as **wall-clock** milliseconds since the Unix epoch (`ends_ms`), or remaining milliseconds while paused (`paused_ms`), so a restart or sleep loses no time and a timer that ended while the PC was off fires shortly after start. The alarm is a tiny WAV synthesised in code (`chime_wav`: three decaying bell notes and a pause) looped from memory (`PlaySound` with `SND_LOOP`): no audio file to ship; `timer::ring(false)` stops it. End times ("ends 14:35") come from `timer::clock`, which formats with `GetTimeFormatEx`, so they follow the user's 12 / 24-hour setting.

## Environment variables

| Variable | Effect |
|---|---|
| `HYTTE_PERF=1` | Log frame-time statistics ([section 12](12-performance.md)). |
| `HYTTE_WT_PID` | Used only by the ignored manual test in `tabs.rs`. |
| `HYTTE_ARG` | **Set by Hytte**, not read: the `arg` of the script pane row or button that ran an action (empty otherwise; always set, so a stale value is never inherited). See [section 17](17-writing-panes.md#arg-and-hytte_arg). |
| `HYTTE_TEST_PANIC` | Debug builds only: panic at startup, to see the relaunch limit work (at most three relaunches in ten minutes). |
| `HYTTE_UPDATE_API` | Debug builds only: replaces the GitHub "latest release" URL; `file://` works ([section 15](15-updates.md#testing-an-update)). |

## Adding a setting

See [section 14](14-how-to-recipes.md#add-a-setting). Always give the new key a serde default so existing files keep loading.

Next: [12. Performance](12-performance.md)
