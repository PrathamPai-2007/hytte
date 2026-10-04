# 11. Configuration and files on disk

## Where things live

Everything Hytte writes is under the current user's profile. Nothing is installed system-wide.

| Path | Written by | Contents |
|---|---|---|
| `%APPDATA%\Hytte\config.toml` | `config.rs` | Settings (below). Created with defaults on first run. |
| `%APPDATA%\Hytte\shelf.json` | `shelf::save` | The shelf list, when `[shelf] persist = true`. |
| `%APPDATA%\Hytte\shelf\` | `shelf::stage` | Copies (in `mode = "copy"`) and dropped text snippets. |
| `%APPDATA%\Hytte\timer.json` | `timer::save` | The running timer, so it survives a restart. Deleted when the timer stops or finishes. |
| `%APPDATA%\Hytte\hytte.log` | `logging.rs` | Panics, and frame telemetry when `HYTTE_PERF=1`. Not written during normal operation. |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Hytte` | `config::ensure_autostart` | Present only while autostart is on: the quoted path of `hytte.exe`. |
| `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Hytte.lnk` | `config::ensure_start_menu` | A Start Menu shortcut so Windows Search finds Hytte. Rewritten on every launch, so it heals itself if the exe moves. |

`config::data_dir()` (the folder of `config.toml`) is the base for every Hytte file. If `APPDATA` isn't set, it falls back to the current directory.

## How the config is loaded

`config::load()`:

- File missing or empty → write a file with every default and use it.
- File present → parse it with `toml`. Every section and every key has a serde default, so a partial file is fine.
- File present but **invalid** → use the defaults in memory, without overwriting the file, so a typo doesn't destroy the user's settings.

The daemon reads the config **once at startup**. Users must restart Hytte after editing it. There is one exception: `[general] output_folder` is re-read each time a Drop Vault action runs.

`config::save()` is only used when the tray's **Launch at startup** item is toggled. It rewrites the whole file from the in-memory config, which **drops any comments** the user added.

## Every setting

### `[general]`

| Key | Default | Meaning |
|---|---|---|
| `autostart` | `false` | Start Hytte at login (the `Run` registry value). Applied at every startup and when toggled from the tray. |
| `monitor` | `"primary"` | Reserved; only the primary monitor is supported. |
| `solid_pill` | `true` | Reserved. |
| `acrylic` | `false` | Draw the pill body at 84 % opacity instead of solid. There is no blur behind it. |
| `suppress_fullscreen` | `true` | Back off while a fullscreen app is in front. |
| `fullscreen_mode` | `"sentinel"` | `"sentinel"`: shrink to a 3 px bar; `"hide"`: fade out completely. |
| `output_folder` | none | Folder for Drop Vault results; if unset or missing, results go next to the source file. |
| `allow_list` | `[]` | Exe names (case-insensitive) that never trigger suppression, e.g. `"Code.exe"`. |
| `deny_list` | `[]` | Exe names that always hide the pill while in front. |

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

### `[ports]`

| Key | Default | Meaning |
|---|---|---|
| `watch` | `3000, 3001, 4200, 5000, 5173, 5432, 8000, 8080, 8888` | Ports shown in the Ports panel. |
| `show_all` | `false` | Show every non-system listener instead. |
| `poll_secs` | `5` | Rescan interval (minimum 1). Opening the pill always rescans immediately. |

### `[agent]`

| Key | Default | Meaning |
|---|---|---|
| `peek_secs` | `6` | How long the pill peeks open when a task needs input. |
| `sound` | `false` | Play the system "asterisk" sound on `NeedsInput`. |

### `[timer]`

| Key | Default | Meaning |
|---|---|---|
| `sound` | `true` | Play the chime when a timer ends. |
| `break_min` | `5` | Short break length after a Focus session. |
| `long_break_min` | `15` | Long break length. |
| `rounds` | `4` | Every `rounds`-th Focus session is followed by a long break. |

## The timer's persistence

`timer.rs` stores the end time as **wall-clock** milliseconds since the Unix epoch (`ends_ms`), or the remaining milliseconds while paused (`paused_ms`). A restart or a sleep doesn't lose time, and a timer that ended while the PC was off fires shortly after Hytte starts. The chime is a tiny WAV synthesised in code (`chime_wav`: two decaying sine notes) and played asynchronously from memory, so there is no audio file to ship.

## Environment variables

| Variable | Effect |
|---|---|
| `HYTTE_PERF=1` | Log frame-time statistics to `hytte.log` ([section 12](12-performance.md)). |
| `HYTTE_WT_PID` | Used only by the ignored manual test in `tabs.rs`. |

## Adding a setting

See [section 14](14-how-to-recipes.md#add-a-setting). Always give the new key a serde default, so existing config files keep loading.

Next: [12. Performance](12-performance.md)
