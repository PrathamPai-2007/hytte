# 9. Background workers

Each worker watches one part of the system and reports changes to the UI thread as a `UiEvent`. They all follow the same pattern:

1. Prefer an **OS notification** over polling. Where polling is unavoidable, use a slow interval and mark it with a `ponytail:` comment explaining the upgrade path.
2. Keep the **last value sent** and only send an event when it changes, so an unchanged world never wakes the UI thread.
3. **Never exit** on failure: fall back, retry later, or go quiet.

| Worker | File | Mechanism | Event |
|---|---|---|---|
| Media | `media.rs` | WinRT change events + 30 s safety timeout | `Media`, `MediaArt` |
| Privacy | `privacy.rs` | Registry change notifications + 10 s safety timeout | `Privacy` |
| Microphone | `mic.rs` | 2 s poll of cached endpoints; device changes by notification | `MicMute` |
| Battery | `power.rs` | 10 s poll | `Power` |
| Ports | `ports.rs` | `poll_secs` poll, woken early on demand | `Ports` |
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

- creates one `IMMDeviceEnumerator` and registers an `IMMNotificationClient` (`DeviceChanges`) that sets a "dirty" flag when a device is added, removed or changes state;
- keeps the activated endpoint list and re-enumerates only when the flag is set;
- every 2 s, reads the mute state from the cached endpoints and sends `UiEvent::MicMute` on change.

If notifications can't be registered, it re-enumerates on every poll instead. The 2 s poll exists because mute changes made elsewhere (a hardware key, Windows Settings) have no cheap notification here. The upgrade path is a per-endpoint `IAudioEndpointVolumeCallback`.

Clicking **Mute** in the pill calls `mic::toggle` directly and updates the model at once, so the UI doesn't wait for the poll.

## Battery and power mode (`power.rs`)

Every 10 s, `power::read` produces `Option<Battery>`:

- **Percentage and AC state** from `GetSystemPowerStatus`. No battery (flag 128) or unknown (255) → `None`, and the Home card hides the battery row.
- **Charge rate in watts** from the battery driver: `IOCTL_BATTERY_QUERY_TAG`, then `IOCTL_BATTERY_QUERY_STATUS`. The battery's device path is found once with SetupDi and cached. A failed query forgets the cached path so it is looked up again (the battery was swapped or the driver reloaded). A driver that reports an unknown rate keeps the path and just shows no wattage.
- **Power mode** (Saver / Balanced / Performance) from the undocumented but stable `powrprof.dll` exports `PowerGetActualOverlayScheme` and `PowerSetActiveOverlayScheme`. The DLL is loaded and the exports resolved once.

Opening the Home panel also calls `power::read` immediately, so the card is never 10 s stale when you look at it.

## Ports (`ports.rs` + `crates/proto/src/ports.rs`)

There is no cheap "a socket started listening" event in Windows (ETW would be the upgrade), so this worker polls:

1. `listeners()` calls `GetExtendedTcpTable` for IPv4 and IPv6 listening sockets with owning pids. It keeps only sockets bound to "any" (`0.0.0.0` / `::`) or loopback (`127.x` / `::1`).
2. `filter()` drops pid 0 / 4 and keeps only watched ports (or everything with `show_all`). It then looks up the exe name for each **remaining** row (that opens the process, so it is done last), drops known system hosts (`svchost.exe`, `lsass.exe`, `msedgewebview2.exe`, ...), removes duplicate IPv4/IPv6 rows, and sorts by port.
3. If the list changed, it sends `UiEvent::Ports`.
4. It waits on a condvar for `poll_secs` (default 5 s). `ports::request_refresh()` wakes it immediately; the UI calls it when the pill opens, when the Ports panel is selected, and after a kill.

Before acting on **Open** or **Kill**, the UI re-checks that the same pid still listens on that port (`port_alive`), so a stale row is removed instead of killing the wrong process. `hytte_proto::ports::kill` refuses pids 0–4 and its own pid.

The same module powers `notch ports` and `notch kill`, so the CLI and the panel always agree.

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
