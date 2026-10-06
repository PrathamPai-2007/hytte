# 9. Background workers

Each worker watches one part of the system and reports changes to the UI thread as a `UiEvent`. They all follow the same pattern:

1. Prefer an **OS notification** over polling. Where polling is unavoidable, use a slow interval and mark it with a `ponytail:` comment explaining the upgrade path.
2. Keep the **last value sent** and only send an event when it changes, so an unchanged world never wakes the UI thread.
3. **Never exit** on failure: fall back, retry later, or go quiet.

| Worker | File | Mechanism | Event |
|---|---|---|---|
| Media | `media.rs` | WinRT change events + 30 s safety timeout | `Media`, `MediaArt` |
| Privacy | `privacy.rs` | Registry change notifications + 10 s safety timeout | `Privacy` |
| Microphone | `mic.rs` | Endpoint-volume callbacks and device notifications (60 s safety timeout) | `MicMute` |
| Battery | `power.rs` | Power broadcasts forwarded by the window, plus a power-mode callback (5 min safety timeout) | `Power` |
| Ports | `ports.rs` | `poll_secs` poll, woken early on demand | `Ports` |
| Resource hogs | `hog.rs` (ticked by `ports.rs`) | Rides the port watcher's tick | `Hog` |
| Downloads | `downloads.rs` | `ReadDirectoryChangesW` (no polling) | `Task`, `DownloadDone`, `TaskGone` |
| Calendar | `calendar.rs` | Store-changed event, or the next event's due time (opt-in) | `Calendar` |
| Config | `config.rs` | Directory change notification (no polling) | `Config` ([section 11](11-configuration-and-files.md#live-reload)) |
| Terminal tabs | `tabs.rs` | Command channel (no polling) | none: it answers commands |
| Thumbnails | `shelf.rs` (`thumbnail`) | One short-lived thread per tile | `Thumb` |

Drop Vault workers and shelf staging are covered in [section 10](10-shelf-and-drop-vault.md).

## Waiting on WinRT: `winrt::block_on`

Several Windows features (media sessions, OCR, `StorageFile`) only have async WinRT APIs. Hytte has no async runtime, so `winrt::block_on(future, timeout)` drives one future on the current thread:

- It polls the future with a waker that **unparks this thread** when the operation's `Completed` handler fires, then parks until then (or until the timeout). There is no busy-waiting.
- On timeout it returns `None`, so a wedged system service can't hang a worker forever.

Use it only on worker threads, never on the UI thread.

## Media (`media.rs`)

Reads Windows' **System Media Transport Controls** (GSMTC): the same data the volume flyout shows.

1. Obtain the `GlobalSystemMediaTransportControlsSessionManager`, retrying every 5 s until it is available.
2. Subscribe to `CurrentSessionChanged`. For the current session, subscribe to `MediaPropertiesChanged`, `PlaybackInfoChanged` and `TimelinePropertiesChanged` (the `Sub` struct unsubscribes on drop).
3. Every handler does nothing but send `()` on an internal "poke" channel. The worker thread waits on that channel (with a 30 s safety timeout), sleeps 60 ms to **coalesce bursts**, drains extra pokes, then re-reads everything once.
4. It reads title, artist, playing state, source app id, position and duration into `MediaInfo`, and sends `UiEvent::Media` when that changes.
5. **Album art** is fetched only when the app, title or artist changes. The thumbnail stream (up to 8 MiB) is decoded with the `image` crate, cropped to fill 64 × 64, premultiplied, and sent as `UiEvent::MediaArt` (`art_from_bytes`).

Transport buttons call `media::control`, which runs the WinRT call on its own short-lived thread.

While a media scene is shown, the renderer extrapolates the timeline from `media_stamp`, so the progress bar moves smoothly between the app's (infrequent) updates.

## Privacy dots (`privacy.rs`)

Windows records camera and microphone use under:

```text
HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{webcam,microphone}
```

Each app has a subkey (classic desktop apps are one level deeper, under `NonPackaged`). A device is **in use** when an app's `LastUsedTimeStart` is non-zero and its `LastUsedTimeStop` is zero.

The worker opens both keys with `KEY_NOTIFY`, arms `RegNotifyChangeKeyValue` (subtree, value changes) on an event per key, and waits on both events with a 10 s safety timeout. When one fires, it re-arms only that key, rescans both devices (`in_use`), and sends `UiEvent::Privacy(cam, mic, app)` if anything changed. `friendly_name` turns a key name into an app name (`Microsoft.WindowsCamera_8wekyb3d8bbwe` → `WindowsCamera`, `C:#Program Files#Zoom#bin#Zoom.exe` → `Zoom`).

If the keys don't exist, it falls back to a 10 s poll.

## Microphone mute (`mic.rs`)

"Muted" means **every active capture endpoint** is muted (`IAudioEndpointVolume::GetMute`). Toggling mutes all of them, or unmutes all if they are all muted already.

The watcher (in a multithreaded COM apartment):

- creates one `IMMDeviceEnumerator` and registers an `IMMNotificationClient` (`DeviceChanges`) that wakes it when a device is added, removed or changes state;
- keeps the activated endpoint list, and subscribes an `IAudioEndpointVolumeCallback` (`MuteChanges`) to every endpoint, so a mute change made anywhere (a hardware key, Windows Settings, another app) wakes it at once;
- blocks on a small channel that both callbacks feed. On a wake it re-enumerates (and re-subscribes) only if a device changed, reads the mute state from the cached endpoints, and sends `UiEvent::MicMute` on change. One toggle notifies every endpoint, and the burst is folded into one recheck.

A 60 s timeout is a safety net against a missed callback. If device notifications can't be registered, the watcher falls back to re-enumerating every 2 s, as it used to.

Clicking **Mute** in the pill calls `mic::toggle` directly and updates the model at once, so the UI doesn't wait for the callback. The ignored test `mic::tests::mute_change_wakes_the_watcher` flips the real mute and checks that the callback, not the timeout, reports it (`cargo test -p hytte mic -- --ignored`).

## Battery and power mode (`power.rs`)

`power::read` produces `Option<Battery>`. It runs once at startup and again whenever `power::poke()` is called:

- **Percentage and AC state** from `GetSystemPowerStatus`. No battery (flag 128) or unknown (255) → `None`, and the Home card hides the battery row.
- **Charge rate in watts** from the battery driver: `IOCTL_BATTERY_QUERY_TAG`, then `IOCTL_BATTERY_QUERY_STATUS`. The battery's device path is found once with SetupDi and cached. A failed query forgets the cached path so it is looked up again (the battery was swapped or the driver reloaded). A driver that reports an unknown rate keeps the path and just shows no wattage.
- **Power mode** (Saver / Balanced / Performance) from the undocumented but stable `powrprof.dll` exports `PowerGetActualOverlayScheme` and `PowerSetActiveOverlayScheme`. The DLL is loaded and the exports resolved once.

`poke` is called from three places: the window procedure on `WM_POWERBROADCAST` (the window registers for the AC/DC power-source and battery-percentage settings with `RegisterPowerSettingNotification`, and also gets resume broadcasts), and a `PowerRegisterForEffectivePowerModeNotifications` callback that the worker registers itself. A 5 min timeout is only a safety net. Nothing polls.

The charge rate changes continuously while charging but has no notification, so it is refreshed when you look at it: selecting the Home panel, or hovering the pill open on it, calls `power::read` immediately.

When a reading moves from battery to AC, `Model::set_power` starts the plug-in card ([section 6](06-ui-model.md)).

## Ports (`ports.rs` + `crates/proto/src/ports.rs`)

There is no cheap "a socket started listening" event in Windows (ETW would be the upgrade), so this worker polls:

1. `listeners()` calls `GetExtendedTcpTable` for IPv4 and IPv6 listening sockets with owning pids. It keeps only sockets bound to "any" (`0.0.0.0` / `::`) or loopback (`127.x` / `::1`).
2. `filter()` drops pid 0 / 4 and keeps only watched ports (or everything with `show_all`). It then looks up the exe name for each **remaining** row (that opens the process, so it is done last), drops known system hosts (`svchost.exe`, `lsass.exe`, `msedgewebview2.exe`, ...), removes duplicate IPv4/IPv6 rows, and sorts by port.
3. If the list changed, it sends `UiEvent::Ports`.
4. It waits on a condvar for `poll_secs` (default 5 s). `ports::request_refresh()` wakes it immediately; the UI calls it when the pill opens, when the Ports panel is selected, and after a kill. `ports::reconfigure()` (a config reload) stores new settings that the loop picks up before its next scan, then wakes it the same way.

Before acting on **Open** or **Kill**, the UI re-checks that the same pid still listens on that port (`port_alive`), so a stale row is removed instead of killing the wrong process. `hytte_proto::ports::kill` refuses pids 0–4 and its own pid.

The same module powers `notch ports` and `notch kill`, so the CLI and the panel always agree.

### Resource hogs (`hog.rs`, `[hog]`)

On every tick of the port watcher (`poll_secs`, 5 s by default) the thread also samples every process: a `CreateToolhelp32Snapshot` walk, then `GetProcessTimes` (kernel + user time) and `K32GetProcessMemoryInfo` (working set) for each. CPU is the time used since the previous sample divided by the elapsed wall time and the core count, so 100 % means the whole machine. A sample taken less than a second after the previous one (the pill opened and woke the thread early) skips CPU, because such a short interval is noise. The idle process, System (pid 4), `hytte.exe` and ourselves are exempt.

`Tracker::update` is the pure part: a process must be over `cpu_pct` for `secs` (default 80 % for 30 s) to alert, while a memory level (`mem_mb`, default 4096) alerts straight away, since memory is a level and not a burst. Each process alerts once; falling below the limits (or exiting) forgets it, so a new burst alerts again. The thread sends `UiEvent::Hog`, and the window shows the warning card unless the exe is in `hog_ignore` or another card is up. Turn it off with `[hog] enabled = false`; a config edit applies at the next tick.

## Downloads (`downloads.rs`, `[downloads]`)

Browsers write a partial file and rename it when done, so the worker turns folder changes into tasks. It opens the Downloads folder (`FOLDERID_Downloads`, or `[downloads] folder`) and blocks in `ReadDirectoryChangesW` (file-name and size changes), so nothing is polled and nothing runs between downloads. `Machine::feed` is the pure part, following the renames:

| Browser | Sequence | Result |
|---|---|---|
| Chrome / Edge | `Unconfirmed 123.crdownload` → `name.zip.crdownload` → `name.zip` | A task for the first name that moves to the real name, then **finished** |
| Firefox | `name.mp4.part` → `name.mp4` | A task, then finished |
| Either, cancelled | the partial file is deleted | The task leaves quietly (`TaskGone`) |

Recognised suffixes are `.crdownload`, `.part`, `.partial`, `.opdownload` and `.download`. A running download is a task (`source = "download"`) labelled with the name and its current size (`foo.zip · 12.3 MB`), updated at most every 0.9 s, and hidden for its first 1.5 s so a tiny file doesn't flicker. Finishing marks the task done and sends `DownloadDone`, which shows the card with **Open** and **Shelve**.

## Calendar (`calendar.rs`, `[calendar]`)

Off by default, because event titles are private and the pill is on screen. When enabled, the worker asks for the appointment store (`AppointmentManager::RequestStoreAsync`, all calendars, read-only; unpackaged apps can get it without a prompt) and reads the next 24 hours. It sleeps on a channel fed by the store's `StoreChanged` event and wakes at the latest when the next event has started (plus a minute), so it never polls; a 30 min cap catches a changed clock.

`pick_next` takes the earliest event that is timed (not all-day), not cancelled, titled, and no more than 2 minutes past its start. `meeting_url` looks for a link in the provider's online-meeting field, the location, then the details; only `https://` links to Teams, Meet, Zoom, Webex or Whereby (host or subdomain) qualify, so a lookalike host or other scheme never reaches the shell. The worker sends `UiEvent::Calendar(Some(event))` only when the next event changes. `Model::set_calendar` arms the heads-up for `lead_min` (default 10) before the start, or at once if that moment has passed; `Model::expire` includes that deadline in the single timer, and `calendar_due` hands the event out once, remembering `(subject, start)` so it is never repeated. The card says "starts in N min" / "is starting" / "started N min ago" and offers **Join** when there is a link.

Two ignored manual tests (`calendar_manual_create`, `calendar_manual_delete`) create and remove a throw-away app calendar so the path can be tried against a real store: `cargo test -p hytte calendar_manual -- --ignored --nocapture`.

## Windows Terminal tabs (`tabs.rs`)

Focusing a window can't pick a tab, so Hytte remembers tabs itself, using **UI Automation**:

- **`Cmd::Snap(task_id, pid)`** is sent when a task with an owner pid is first seen. The worker finds the terminal window (`proc::terminal_window_for`). If it is Windows Terminal (window class `CASCADIA_HOSTING_WINDOW_CLASS`), it finds the **selected** `TabItem` element and stores it under the task id. The user has just pressed Enter there, so it is the right tab.
- **`Cmd::Select(task_id)`** is sent after a click focuses the terminal. The stored tab is selected through `SelectionItemPattern`.

UIA calls are cross-process and can be slow, so they run on this dedicated thread and never block the UI. At most 128 tabs are remembered (the map is cleared when full). Other terminals, or any failure, simply fall back to focusing the window.

## Shelf thumbnails

For each visible shelf tile without a thumbnail, `Ui::request_thumbs` starts a short-lived thread that calls `shelf::thumbnail(path)`. That uses the shell's `IShellItemImageFactory` (the same thumbnails Explorer shows, or the file-type icon) at 64 × 64, fixes bitmaps with no alpha channel, centres the image in a 64 × 64 tile, and pushes `UiEvent::Thumb`. Each item is requested at most once.

## Adding a worker

See [section 14](14-how-to-recipes.md#add-a-background-worker). The short version: spawn it in `main.rs`, give it a `Sender<UiEvent>`, send only on change, prefer notifications, and handle the new event in `Ui::handle_events`.

Next: [10. Shelf and Drop Vault](10-shelf-and-drop-vault.md)
