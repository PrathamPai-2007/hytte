# 12. Performance

Hytte runs all day next to everything else on the machine, so its idle cost matters more than its peak speed. This section lists the targets, everything that can wake the process while idle, and the rules that keep it cheap.

## Budgets

| Budget | Target | Last measured (1.3.0 release build, default settings, a fresh data folder) |
|---|---|---|
| Idle CPU, collapsed, nothing playing | < 1 % | ≈ 0.5 to 0.7 % of one core (156 to 203 ms of CPU in 30 s, in two runs, on a PC with about 270 processes running) |
| Idle memory | < 30 MB stretch goal, 100 MB ceiling | ≈ 11 MB private (committed) bytes; working set ≈ 43 MB at startup and ≈ 4 MB after the 30 s trim. 35 threads, about 410 handles. The trim only runs once nothing has been drawing for 30 s: with music playing it never did (a run with VLC playing stayed at 45 to 50 MB). |
| Animation | Monitor refresh rate during transitions only | vsync-paced; parks when settled |
| Frame cost while a spring moves | well under one refresh period | `gpu`: draw + present p50 ≈ 2.0 ms, p99 ≈ 2.5 to 5.8 ms, with a running task's spinner in the open pill (60 Hz display at 200 % scaling, WARP device; the draw alone is p50 ≈ 1.8 ms). A refresh period is 16.6 ms here. |

Release builds use `lto = true` and `codegen-units = 1` (root `Cargo.toml`) for a smaller binary and less resident code.

### Why the GPU renderer uses WARP

The pill is a few hundred pixels, so a real GPU has nothing to gain, but loading its user-mode driver is expensive. Measured once, when the WARP device was introduced, on an AMD machine, idle, release build (a historical comparison; the current figures are in the table above):

| `gpu` device | Private bytes | Threads | Handles | Frame cost p50 / p99 |
|---|---|---|---|---|
| Hardware driver (before) | 55 MB | 48 | 1,660 | 0.88 / 1.5 ms |
| WARP (now) | 10 MB | 34 | 400 | 0.45 / 1.0 ms |

The driver (`amdxx64.dll`) alone is ≈ 62 MB of mapped image and a pool of its own threads. WARP draws the same frames in about half the time and the same DirectComposition path. Frame pacing is unchanged (gap p50 8.3 ms, the display's 120 Hz).

Measure it yourself: start the exe with an isolated `%APPDATA%`, wait 45 s, and read `PrivateMemorySize64` (the working set is small only because of the trim). Nothing may be playing, or the pill keeps drawing and never trims. For frame cost put `fullscreen = "off"` in that folder's `config.toml` (otherwise a maximised window in front hides the pill and no frames are drawn), then use `HYTTE_PERF=1` and run `hytte run -- ping -n 3 127.0.0.1` in a loop, which makes the pill expand and collapse.

## The rules

1. **Render only on change.** A settled, static pill draws no frames. The frame clock parks ([section 8](08-rendering-and-animation.md)).
2. **Every animation has a stop condition**, encoded in `Model::ambient`. No condition means 30 fps forever.
3. **Prefer OS notifications to polling.** Where polling is unavoidable, use a long interval, document it with a `ponytail:` comment, and only send a `UiEvent` when the value changes.
4. **Wake the UI thread once per burst.** Events are queued and drained together (`Ui::handle_events`); the frame clock keeps at most one `WM_TICK` in flight.
5. **Use one timer for all deadlines.** `Model::expire` returns the next deadline, and one Win32 timer fires exactly then. Never add a periodic sweep.
6. **Heavy work goes off the UI thread.** File copies, image conversion, OCR, UI Automation, WinRT calls and thumbnails all run on workers.
7. **Allocate nothing per frame where you can avoid it.** Brushes, text formats, gradients, the hit list, the text buffer and the pill silhouette (while its size is unchanged) are reused across frames.
8. **Background workers run at EcoQoS.** The media, mic, power, privacy, ports, Drop Vault and terminal-tab threads call `proc::eco_thread()` first, so Windows may run them on efficiency cores. The UI thread, the frame clock, the pipe listener and the task registry stay at normal priority, because a user is waiting on them.
9. **Hand memory back when idle.** After 30 s without a frame, the window trims the working set (`T_TRIM`). Cold pages fault back in on use.

## Everything that can wake Hytte while idle

When adding a feature, check it doesn't add a row here without a good reason.

| Source | Interval | Notes |
|---|---|---|
| Port watcher | every `poll_secs` (5 s), skipped in game mode | One `GetExtendedTcpTable` call per address family. Processes are opened only for watched ports. When the list changes, one process snapshot finds each server's ancestors. |
| Resource-hog sampler | with the port watcher's tick (`poll_secs`, 5 s), skipped in game mode | One process snapshot, then two cheap calls per process. Turn off with `[hog] enabled = false`. |
| Downloads watcher | on a change in the Downloads folder | Blocks in `ReadDirectoryChangesW`; wakes only for file-name and size changes there. |
| Calendar watcher (opt-in) | on a store change, or when the next event has started (at most every 30 min) | Reads the next 24 hours. |
| Mic watcher | on a mute or device callback, or 60 s safety timeout | Reads the cached endpoints' mute state and the default speaker's volume; re-enumerates only after a device change notification. |
| Power watcher | on a power broadcast or power-mode callback, or 5 min safety timeout | `GetSystemPowerStatus`. On laptops, also two IOCTLs on the cached battery device. |
| Memory trim | once, 30 s after the last frame | `SetProcessWorkingSetSize`. |
| Privacy watcher | on registry change, or 10 s safety timeout | |
| Config watcher | on a file change in `%APPDATA%\Hytte` | Reads `config.toml` (a few KB); writes to `shelf.json` / `timer.json` also wake it. |
| Media watcher | on WinRT event, or 30 s safety timeout | |
| Task registry | every 5 s **only while tasks exist** (running ones, or failed shell rows waiting for their terminal to close) | Checks owner liveness. |
| Clipboard listener | on a clipboard change | `WM_CLIPBOARDUPDATE`; looks at the owner first, so other apps' copies cost a process-name lookup at most. |
| Screenshot timer (`T_SHOT`) | 1 s while a screenshot is on the shelf with `screenshot_secs` set; 60 ms for up to 10 minutes after each screenshot while `delete_after_paste` (on by default) waits for a Ctrl+V | Never runs without a screenshot. |
| Settings window | 60 Hz **only while something eases** (hover fades, scroll, fade-in) | Stops when everything has settled; nothing runs while the window is closed. |
| Countdown timer | 1 Hz **only while a timer runs unpaused** | Redraws the fuse. |
| Expiry timer | once, at the next deadline | Also ends a one-hour keep-awake. |
| GPU rebuild retry | 1 s doubling to 30 s, **only after a failed rebuild** | Stops once the renderer is back. |
| Foreground change / move | per event, debounced to one check per 150 ms | |
| Low-level mouse hook | every mouse event system-wide, **removed in game mode** | The callback is O(1): atomics only, and it posts a message only when a drag enters or leaves the top-edge zone. |

**Game mode.** While a game or other real fullscreen app is in front, Hytte removes both system-wide hooks, hides its window, releases its hotkey, runs at background priority and skips the port and hog scans. What is left is a few blocked threads and the foreground hook. See [section 7](07-window-and-input.md#fullscreen-suppression).

None of these draws a frame unless the model actually changes.

## Frames: when they run

| Situation | Frames |
|---|---|
| Collapsed, nothing happening | none |
| Pointer moving over the pill | one per mouse move (hover effects follow the pointer) |
| A spring settling (resize, fade) | one per vblank, until settled |
| Running task, waiting agent, playing media in a media scene, drag over the pill, expanded privacy dots | ~30 fps |
| Failed task | ~30 fps for the first 8 s (the pulse fades), then none |
| Timer-finished card | ~30 fps while it is up (at most 2 min; the bell swings) |
| Fullscreen sentinel | none (it never animates) |
| Hidden, paused or in game mode | none |

## Measuring

### Frame cost

```powershell
$env:HYTTE_PERF = "1"; .\target\release\hytte.exe
```

Every 240 animated frames, a line like this is appended to `%APPDATA%\Hytte\hytte.log` (the numbers are illustrative):

```text
perf n=240 cost_us p50=310 p99=900 max=1400 | draw_us p50=260 p99=800 | gap_us p50=16667 p99=17100 max=33000 | missed=2
```

- **`cost_us`** is the time to draw and present one frame. It should stay well under a refresh period.
- **`draw_us`** is the painting alone. `cost_us` minus `draw_us` is the hand-off to the compositor, so a regression can be pinned on one or the other.
- **`gap_us`** is the time between frames: about 16 667 µs at 60 Hz while springs move.
- **`missed`** counts frames whose gap exceeded 1.5× the median, meaning a vblank was missed.

The first frame after idle is excluded (its gap is the idle time).

### CPU and memory

Use Task Manager's *Details* tab (CPU, working set), Process Explorer, or Windows Performance Recorder / Analyzer for a timeline. Measure **release** builds: debug builds are many times slower at drawing and decoding.

A quick idle check: start Hytte, leave it collapsed with nothing playing, and watch it in Process Explorer for a minute. CPU should read 0 most of the time, and the "Context Switch Delta" column should stay small.

## Memory notes

- The canvas surface (the swap chain's two buffers) is 460 × 310 logical px × 4 bytes, scaled by DPI (about 2.3 MB per buffer at 200 %).
- Album art and thumbnails are 64 × 64 BGRA (16 KB each). Only visible shelf tiles have thumbnails, and thumbnails of removed items are freed.
- Drop Vault conversions are the only memory-heavy code. Images over 60 megapixels are refused before decoding, the decoded original is freed before JPEG encoding, and JPEG → PDF reads only the image header.
- Terminal tab handles are capped at 128.

Next: [13. Testing and releasing](13-testing-and-release.md)
