# 9. Background workers

Each worker watches one part of the system and reports changes to the UI thread as a `UiEvent`. All follow one pattern:

1. Prefer an **OS notification** to polling. Where polling is unavoidable, use a slow interval and a `ponytail:` comment naming the upgrade path.
2. Keep the **last value sent** and send only on change, so an unchanged world never wakes the UI thread.
3. **Never exit** on failure: fall back, retry later, or go quiet.

| Worker | File | Mechanism | Event |
|---|---|---|---|
| Media | `media.rs` | WinRT change events + 30 s safety timeout | `Media`, `MediaArt` |
| Privacy | `privacy.rs` | Registry change notifications + 10 s timeout | `Privacy` |
| Mic and speaker | `mic.rs` | Endpoint-volume callbacks and device notifications (60 s timeout) | `MicMute`, `Speaker` |
| Battery | `power.rs` | Power broadcasts plus a power-mode callback (5 min timeout) | `Power` |
| Ports | `ports.rs` | `poll_secs` poll, woken early on demand | `Ports` |
| Resource hogs | `hog.rs` | Rides the port watcher's tick | `Hog` |
| Downloads | `downloads.rs` | `ReadDirectoryChangesW` | `Task`, `DownloadDone`, `TaskGone` |
| Calendar | `calendar.rs` | Store-changed event or the next event's due time (opt-in) | `Calendar` |
| Config | `config.rs` | Directory change notification | `Config` ([section 11](11-configuration-and-files.md#live-reload)) |
| Terminal tabs | `tabs.rs` | Command channel | none (answers commands) |
| Thumbnails | `shelf.rs` | One short-lived thread per tile | `Thumb` |
| Place search, forecast | `weather.rs` | One short-lived thread per request, only while the Weather pane is showing | `Places`, `WidePlaces`, `Forecast` |
| Lyrics | `lyrics.rs` | One short-lived thread per song, only with the lyrics page open | `Lyrics` |
| Sleep fade | `sleep.rs` | One thread from the fade's start to the pause (≤ 20 s) | none |

Drop Vault workers and shelf staging are in [section 10](10-shelf-and-drop-vault.md). Also event-driven: the window listens for clipboard changes (`AddClipboardFormatListener` → `WM_CLIPBOARDUPDATE`) to shelve Snipping Tool screenshots, whose short stay and delete-after-paste run on `T_SHOT` ([section 7](07-window-and-input.md#timers)).

## Waiting on WinRT: `winrt::block_on`

Several Windows features (media sessions, OCR, `StorageFile`) are async-only WinRT APIs, and Hytte has no async runtime. `winrt::block_on(future, timeout)` polls the future with a waker that **unparks this thread** when the operation's `Completed` handler fires, then parks (no busy-waiting). On timeout it returns `None`, so a wedged service can't hang a worker. Worker threads only, never the UI thread.

## Media (`media.rs`)

Reads Windows' **System Media Transport Controls** (GSMTC), the data the volume flyout shows.

1. Get the `GlobalSystemMediaTransportControlsSessionManager`, retrying every 5 s until available.
2. Subscribe to `CurrentSessionChanged`; for the current session subscribe to `MediaPropertiesChanged`, `PlaybackInfoChanged` and `TimelinePropertiesChanged` (`Sub` unsubscribes on drop).
3. Handlers only send `()` on a "poke" channel. The worker waits on it (30 s timeout), sleeps 60 ms to **coalesce bursts**, drains extra pokes, and re-reads everything once.
4. It reads title, artist, playing state, source app id, position and duration into `MediaInfo` and sends `UiEvent::Media` on change.
5. **Album art** is fetched only when app, title or artist changes: the thumbnail stream (≤ 8 MiB) is decoded with `image`, cropped to 64 × 64, premultiplied, and sent as `UiEvent::MediaArt` (`art_from_bytes`).

Transport buttons call `media::control` on a short-lived thread. While a media scene shows, the renderer extrapolates the timeline from `media_stamp` so the bar moves smoothly between the app's infrequent updates.

## Privacy dots (`privacy.rs`)

Windows records camera and microphone use under `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{webcam,microphone}`. Each app has a subkey (classic desktop apps one level deeper, under `NonPackaged`). A device is **in use** when an app's `LastUsedTimeStart` is non-zero and `LastUsedTimeStop` is zero.

Windows never writes the stop time for a classic app that crashed or was killed mid-capture, leaving it "in use" for weeks (a badly exited game kept the mic dot lit). So a `NonPackaged` entry counts only while a process with that exe name runs (`privacy::running`, one snapshot per scan).

The worker opens both keys with `KEY_NOTIFY`, arms `RegNotifyChangeKeyValue` (subtree, value changes) on an event per key, and waits on both with a 10 s timeout. On a trigger it re-arms that key, rescans both devices and sends `UiEvent::Privacy(cam, mic, app)` if anything changed. `friendly_name` makes app names (`Microsoft.WindowsCamera_8wekyb3d8bbwe` → `WindowsCamera`, `C:#Program Files#Zoom#bin#Zoom.exe` → `Zoom`). Without the keys it polls every 10 s.

## Microphone mute and speaker volume (`mic.rs`)

"Muted" means **every active capture endpoint** is muted (`IAudioEndpointVolume::GetMute`); toggling mutes all, or unmutes all if all are muted.

The watcher (MTA) creates one `IMMDeviceEnumerator` with an `IMMNotificationClient` (`DeviceChanges`) that wakes it when a device is added, removed or changes state, or the default speaker changes; keeps the activated capture endpoints and the **default render endpoint** (`eRender`, `eConsole`), each subscribed with an `IAudioEndpointVolumeCallback` (`MuteChanges`) so a change made anywhere (hardware key, flyout, Settings, another app) wakes it at once; and blocks on a channel both callbacks feed. On a wake it re-enumerates only if a device changed, reads the cached endpoints, and sends `UiEvent::MicMute` and `UiEvent::Speaker((volume, muted))` (`None` without an output) on change. One toggle notifies every endpoint; the burst folds into one recheck. A 60 s timeout is a safety net, and without device notifications it falls back to re-enumerating every 2 s.

The **Mic** tile calls `mic::toggle`, **Sound** `mic::toggle_speaker`, the bar and wheel `mic::set_volume` (which unmutes above 0). Each returns the new state so the model updates at once; while the bar is dragged `Speaker` events are ignored until release. The ignored test `mic::tests::mute_change_wakes_the_watcher` flips the real mute and checks the callback, not the timeout, reports it (`cargo test -p hytte mic -- --ignored`).

## Battery and power mode (`power.rs`)

`power::read` returns `Option<Battery>` at startup and whenever `power::poke()` is called:

- **Percentage and AC state** from `GetSystemPowerStatus`. No battery (flag 128) or unknown (255) → `None` and no Power pane. On battery, `BatteryLifeTime` gives "3 h 10 min left", rounded to 5 minutes so it doesn't wake the UI every tick.
- **Charge rate (W)** from the battery driver: `IOCTL_BATTERY_QUERY_TAG`, then `IOCTL_BATTERY_QUERY_STATUS`. The device path is found once with SetupDi and cached; a failed query forgets it (battery swapped, driver reloaded); an unknown rate keeps the path and shows no wattage.
- **Power mode** (Saver / Balanced / Performance) from the undocumented but stable `powrprof.dll` exports `PowerGetActualOverlayScheme` and `PowerSetActiveOverlayScheme`, resolved once.

`poke` comes from the window procedure on `WM_POWERBROADCAST` (`RegisterPowerSettingNotification` for AC/DC source and battery percentage, plus resume) and from a `PowerRegisterForEffectivePowerModeNotifications` callback the worker registers. The 5 min timeout is only a safety net. The charge rate has no notification, so selecting or hover-opening the Power panel calls `power::read` immediately. A battery ↔ AC move makes `Model::set_power` start the plug-in / unplug card ([section 6](06-ui-model.md)).

## Ports (`ports.rs` + `crates/proto/src/ports.rs`)

Windows has no cheap "a socket started listening" event (ETW would be the upgrade), so this polls:

1. `listeners()` calls `GetExtendedTcpTable` for IPv4 and IPv6 listening sockets with pids, keeping only sockets bound to "any" (`0.0.0.0` / `::`) or loopback.
2. `filter()` drops pid 0 / 4, keeps only watched ports (or all with `show_all`), looks up exe names for the **remaining** rows last (it opens the process), drops known system hosts (`svchost.exe`, `lsass.exe`, `msedgewebview2.exe`, ...), removes IPv4/IPv6 duplicates and sorts by port.
3. If the list changed, it walks each port's process up its ancestors (`hytte_proto::sys::ancestors`, one Toolhelp snapshot, ≤ 8 levels) and sends `UiEvent::Ports(ports, ancestors)`. `Model::task_for_port` ties a server to the running task whose pid is the server's own or an ancestor's, so the row shows the task's label and focuses its terminal.
4. It waits on a condvar for `poll_secs` (5 s). `ports::request_refresh()` wakes it at once (pill opens, Tasks selected, after a kill, leaving game mode); `ports::reconfigure()` stores new settings the loop picks up before its next scan, then wakes it.

Before **Open** or **Kill**, the UI re-checks the same pid still listens on that port (`port_alive`), so a stale row is removed instead of killing the wrong process; `hytte_proto::ports::kill` refuses pids 0–4 and its own. The same module powers `hytte ports` and `hytte kill`.

### Resource hogs (`hog.rs`, `[hog]`)

On every port-watcher tick the thread samples every process: a `CreateToolhelp32Snapshot` walk, then `GetProcessTimes` and `K32GetProcessMemoryInfo` (working set). CPU is time used since the last sample over elapsed wall time and core count (100 % = the whole machine); a sample under a second after the previous one (the pill opened and woke the thread) skips CPU as noise. The idle process, System (pid 4), `hytte.exe` and ourselves are exempt.

`Tracker::update` is the pure part: a process must exceed `cpu_pct` for `secs` (defaults 70 % / 30 s) to alert, while memory (`mem_mb`, 4096) alerts at once (a level, not a burst). Each process alerts once; dropping below the limits or exiting forgets it, so a new burst alerts again. The thread sends `UiEvent::Hog`; the window shows the card unless the exe is in `hog_ignore` or another card is up. Off with `[hog] enabled = false`; a config edit applies at the next tick.

## Downloads (`downloads.rs`, `[downloads]`)

Browsers write a partial file and rename it when done, so the worker turns folder changes into tasks. It opens the Downloads folder (`FOLDERID_Downloads` or `[downloads] folder`) and blocks in `ReadDirectoryChangesW` (name and size changes): nothing polls. `Machine::feed` follows the renames:

| Browser | Sequence | Result |
|---|---|---|
| Chrome / Edge | `Unconfirmed 123.crdownload` → `name.zip.crdownload` → `name.zip` | A task for the first name that moves to the real name, then **finished** |
| Firefox | `name.mp4.part` → `name.mp4` | A task, then finished |
| Either, cancelled | the partial file is deleted | The task leaves quietly (`TaskGone`) |

Partial suffixes: `.crdownload`, `.part`, `.partial`, `.opdownload`, `.download`. A running download is a task (`source = "download"`) labelled with name and current size (`foo.zip · 12.3 MB`), updated at most every 0.9 s and hidden for its first 1.5 s so a tiny file doesn't flicker. Finishing marks it done and sends `DownloadDone` (card with **Open** and **Shelve**). A partial file whose size hasn't changed for 5 minutes (`DOWNLOAD_STALE`) leaves the list, and returns if it changes again.

## Calendar (`calendar.rs`, `[calendar]`)

Off by default (event titles are private and the pill is on screen). Enabled, the worker asks for the appointment store (`AppointmentManager::RequestStoreAsync`, all calendars, read-only; unpackaged apps get it without a prompt) and reads the next 24 hours. It sleeps on a channel fed by `StoreChanged` and wakes at the latest a minute after the next event starts; a 30 min cap catches a changed clock.

`pick_next` takes the earliest event that is timed (not all-day), not cancelled, titled, and ≤ 2 minutes past its start. `meeting_url` looks in the online-meeting field, the location, then the details; only `https://` links to Teams, Meet, Zoom, Webex or Whereby (host or subdomain) qualify, so lookalike hosts and other schemes never reach the shell. The worker sends `UiEvent::Calendar(Some(event))` only when the next event changes. `Model::set_calendar` arms the heads-up `lead_min` (10) before the start, or at once if past; `Model::expire` includes that deadline in the single timer and `calendar_due` hands the event out once, remembering `(subject, start)`. The card says "starts in N min" / "is starting" / "started N min ago" and offers **Join** with a link.

Two ignored manual tests (`calendar_manual_create`, `calendar_manual_delete`) create and remove a throw-away app calendar: `cargo test -p hytte calendar_manual -- --ignored --nocapture`.

## Windows Terminal tabs (`tabs.rs`)

Focusing a window can't pick a tab, so Hytte remembers tabs with **UI Automation**:

- **`Cmd::Snap(task_id, pid)`**, sent when a task with an owner pid is first seen: the worker finds the terminal window (`proc::terminal_window_for`); if it is Windows Terminal (`CASCADIA_HOSTING_WINDOW_CLASS`) it stores the **selected** `TabItem` under the task id (the user just pressed Enter there).
- **`Cmd::Select(task_id)`**, sent after a click focuses the terminal: selects the stored tab via `SelectionItemPattern`.

UIA calls are cross-process and slow, so they run on this dedicated thread. At most 128 tabs are remembered (cleared when full). Other terminals, or any failure, just focus the window.

## Shelf thumbnails

For each visible shelf tile without one, `Ui::request_thumbs` starts a short-lived thread calling `shelf::thumbnail(path)`: the shell's `IShellItemImageFactory` (Explorer's thumbnails or the file-type icon) at 64 × 64, fixing bitmaps with no alpha and centring in the tile, then `UiEvent::Thumb`. Each item is requested at most once.

## Weather, lyrics and fetching (`net.rs`)

`net::fetch(url, out, secs, max_bytes)` runs System32's `curl.exe` by absolute path (no HTTP stack is linked), HTTPS only in release builds, with a User-Agent of `hytte/<version>`; the updater, the weather and the lyrics all use it. `net::encode` percent-encodes query values as UTF-8.

**Weather** uses Open-Meteo's geocoding and forecast endpoints (free for non-commercial use, CC BY 4.0, which is why the pane credits it). Nothing runs while the pane is closed. **The wider place search** uses OpenStreetMap's Nominatim (`weather::fetch_wide`, ODbL, credited under its results). Its usage policy forbids search-as-you-type, so it runs only when the user clicks or presses Enter on the "search more widely" row, which appears two seconds after Open-Meteo found nothing; answers are cached for the session, and the User-Agent names the app. **Lyrics** use lrclib.net: `/api/get` with title, artist, album and length, then with title and artist alone, then `/api/search`. A 404 is "no such song"; any other failure is "can't reach".

## The sleep fade (`sleep.rs`)

The thread initialises COM, finds the player's audio session (`pick_exe` over `mic::apps`), reads its volume with `mic::app_volume`, writes `sleep-restore.txt`, steps the volume down once a second, pauses the player (`media::Cmd::Pause`, `TryPauseAsync`: it never starts playback), waits a moment and puts the volume back. `sleep::cancel` stops it and restores the volume at once. The ignored tests `manual_sessions` and `manual_fade` try it on a real player.

## Do Not Disturb (`dnd.rs`)

A Focus session writes `NOC_GLOBAL_SETTING_TOASTS_ENABLED = 0` under `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings` and reads it back. Microsoft's own `FocusSessionManager.TryStartFocusSession` needs a Limited Access Feature token, so it isn't used. The value is undocumented; tested on Windows 11 26H2, where it writes, reads back and restores, but a screenshot test could not show a toast banner being suppressed (the control run with the switch off showed none either), so the visible effect is confirmed by hand. `dnd-restore.txt` holds the earlier value while a session runs.

## Adding a worker

See [section 14](14-how-to-recipes.md#add-a-background-worker): spawn it in `main.rs`, give it a `Sender<UiEvent>`, send only on change, prefer notifications, handle the event in `Ui::handle_events`.

Next: [10. Shelf and Drop Vault](10-shelf-and-drop-vault.md)
