# 12. Performance

Hytte runs all day beside everything else, so idle cost matters more than peak speed. This section lists the targets, everything that can wake the process while idle, and the rules that keep it cheap.

## Budgets

| Budget | Target | Last measured (release build, default settings, fresh data folder; rows marked 1.7.0 are from that release) |
|---|---|---|
| Idle CPU, collapsed, nothing playing | about 0.2 % (limit < 1 %) | ≈ 0.26 % of one core (78 ms CPU in 30 s, after 45 s settling), 0.36 % with a Focus session running (109 ms); measured on 1.7.0 with every feature on, 120 stored notes, a skin and all three break reminders enabled |
| Idle memory | under 25 MB (ceiling 100 MB) | ≈ 10.1 MB private bytes; working set ≈ 5.7 MB after the trim; 32 threads, ~404 handles (1.7.0, same setup) |
| Animation | Refresh rate during transitions only | vsync-paced; parks when settled |
| UI-thread frame time (`cost_us`) | p99 under 40 % of a refresh period | p50 ≈ 0.75 ms, p99 ≈ 1.0 ms with a spinner on a collapsed pill; p50 ≈ 1.2 ms, p99 ≈ 1.7 ms while a spring moves (60 Hz, WARP) |
| CPU with a running task, collapsed | low single digits | ≈ 5.5 % of one core at 30 fps (1.9.0: 14.6 %) |
| CPU with a running task, open | | ≈ 18 % at 60 fps (1.9.0: 27 % at 30 fps) |
| Missed frames while a spring moves | at most 2 per 240 | 0 (1.9.0: 4 to 7, the change from the ambient sleep to vblank pacing) |

Release builds use `lto = true` and `codegen-units = 1` (root `Cargo.toml`).

### Why the GPU renderer uses WARP

The pill is a few hundred pixels, so a real GPU gains nothing, but loading its user-mode driver is expensive. Measured once when WARP was introduced (AMD machine, idle, release; historical):

| Device | Private bytes | Threads | Handles | Frame cost p50 / p99 |
|---|---|---|---|---|
| Hardware driver (before) | 55 MB | 48 | 1,660 | 0.88 / 1.5 ms |
| WARP (now) | 10 MB | 34 | 400 | 0.45 / 1.0 ms |

The driver (`amdxx64.dll`) alone is ≈ 62 MB of mapped image plus its own thread pool. WARP draws the same frames in about half the time through the same DirectComposition path; pacing is unchanged.

To measure yourself: start the exe with an isolated `%APPDATA%`, wait 45 s, read `PrivateMemorySize64` (the working set is small only because of the trim). Nothing may be playing or the pill keeps drawing and never trims. For frame cost set `fullscreen = "off"` in that folder's `config.toml` (a maximised window in front would hide the pill), use `HYTTE_PERF=1`, and loop `hytte run -- ping -n 3 127.0.0.1` so the pill expands and collapses.

## The rules

1. **Render only on change.** A settled, static pill draws no frames; the frame clock parks ([section 8](08-rendering-and-animation.md)). A steady background is copied from bitmaps, not painted (the background cache).
2. **Every animation has a stop condition**, encoded in `Model::ambient`. None means 30 fps forever.
3. **Prefer OS notifications to polling.** If polling is unavoidable use a long interval, a `ponytail:` comment, and send a `UiEvent` only when the value changes.
4. **Wake the UI thread once per burst.** Events are queued and drained together; at most one `WM_TICK` is in flight.
5. **One timer for all deadlines.** `Model::expire` returns the next deadline and one Win32 timer fires then. No periodic sweeps.
6. **Heavy work off the UI thread:** file copies, conversion, OCR, UI Automation, WinRT, thumbnails.
7. **Allocate nothing per frame where avoidable.** Brushes, text formats, gradients, the hit list, text buffer and the pill silhouette (while its size is unchanged) are reused.
8. **Background workers run at EcoQoS.** Media, mic, power, privacy, ports, Drop Vault and terminal-tab threads call `proc::eco_thread()` first. The UI thread, frame clock, pipe listener and task registry stay normal, because a user is waiting on them.
9. **Hand memory back when idle.** After 30 s without a frame the window trims the working set (`T_TRIM`); cold pages fault back in on use.

## Everything that can wake Hytte while idle

When adding a feature, don't add a row here without good reason.

| Source | Interval | Notes |
|---|---|---|
| Port watcher | every `poll_secs` (5 s), skipped in game mode | One `GetExtendedTcpTable` per address family; processes opened only for watched ports; a snapshot finds ancestors only when the list changes. |
| Resource-hog sampler | with the port tick, skipped in game mode | One snapshot, two cheap calls per process. `[hog] enabled = false` turns it off. |
| Downloads watcher | on a Downloads folder change | Blocks in `ReadDirectoryChangesW`. |
| Calendar watcher (opt-in) | on a store change or when the next event starts (≤ every 30 min) | Reads the next 24 hours. |
| Mic watcher | on a mute or device callback; 60 s safety timeout | Reads cached endpoints; re-enumerates only after a device change. |
| Power watcher | on a power broadcast or mode callback; 5 min safety timeout | `GetSystemPowerStatus`; on laptops two IOCTLs on the cached battery device. |
| Privacy / media watchers | on registry change / WinRT event; 10 s / 30 s safety timeouts | |
| Config watcher | on a file change in `%APPDATA%\Hytte` | Reads `config.toml`; `shelf.json` / `timer.json` writes also wake it. |
| Task registry | every 5 s **only while tasks exist** | Checks owner liveness. |
| Clipboard listener | on a clipboard change | Looks at the owner first, so other apps' copies cost at most a process-name lookup. |
| Ask (`[ai]`) | only while a question is being answered or a model downloads | One worker thread and one `curl.exe` per request; the pill draws at the frame rate while the dots show and text streams in, and idles when the answer ends. The model's memory is Ollama's, freed `keep_alive` after the last question. |
| Screenshot timer (`T_SHOT`) | 1 s while a screenshot is on the shelf with `screenshot_secs` set; 60 ms for ≤ 10 min after each screenshot while `delete_after_paste` waits for a Ctrl+V | Never runs without a screenshot. |
| Settings window | 60 Hz **only while something eases** | Nothing runs while closed. |
| Text caret (`T_CARET`) | 530 ms, **only while a text field has focus** | One small redraw per blink; stops when typing ends. |
| Place search (`T_PLACE`) | once, 250 ms after the last keystroke | Then one request. |
| Break reminders | once, at the next due time (or a minute later while one waits) | Part of the single expiry timer. |
| Notes and to-dos | once, at the next expiry | Part of the single expiry timer; none when nothing expires. |
| Sleep timer | at its fade, its stop and each minute of its label | Only while a timer is set. |
| Focus line | every `total / 100` seconds (1 to 30 s) | Only while a Focus session runs on a bare pill; with a bubble or an open pane it is the 1 Hz timer. |
| Forecast, lyrics | one request per open of the pane (data over 30 min old) / per song | Never while the pane or page is closed. |
| Countdown timer | 1 Hz **only while a timer runs unpaused** | Redraws the fuse. |
| Expiry timer | once, at the next deadline | Also ends a one-hour keep-awake. |
| Memory trim | once, 30 s after the last frame | `SetProcessWorkingSetSize`. |
| GPU rebuild retry | 1 s doubling to 30 s, **only after a failed rebuild** | |
| Foreground change / move | per event, debounced to one check per 150 ms | |
| Low-level mouse hook | every system mouse event, **removed in game mode** | O(1): atomics only; posts only when a drag enters or leaves the top-edge zone. |

**Game mode:** both system-wide hooks are removed, the window hides, the hotkey is released, priority drops to background and the port and hog scans stop. What remains is a few blocked threads and the foreground hook ([section 7](07-window-and-input.md#fullscreen-suppression)). None of the sources above draws a frame unless the model changes.

## Frames: when they run

| Situation | Frames |
|---|---|
| Collapsed, nothing happening | none |
| Pointer moving over the pill | one per mouse move |
| A spring settling | one per vblank, until settled |
| Running task, waiting agent, playing media in a media scene, drag over the pill, expanded privacy dots | every few vblanks: about 60 fps in an open or pointed-at pill, about 30 fps collapsed |
| A rolling digit, a row sliding, a press, a scroll gliding | one per vblank, until settled |
| Failed task | ~30 fps for 8 s (the pulse fades), then none |
| Timer-finished card | ~30 fps while up (≤ 2 min) |
| Eye-break card | ~30 fps for its 20 s (the ring) |
| Text field with focus | one frame per blink (about 2 per second) |
| Sentinel, hidden, paused, game mode | none |

## Measuring

```powershell
$env:HYTTE_PERF = "1"; .\target\release\hytte.exe
```

Every 240 animated frames a line like this (illustrative numbers) goes to `%APPDATA%\Hytte\hytte.log`:

```text
perf every=1 n=240 hz=60 cpu_us=3906 cost_us p50=1211 p99=1662 max=1892 | draw_us p50=1068 p99=1515 | gap_us p50=16683 p99=17296 max=17488 | missed=0
```

Frames that come every vblank (`every=1`) and every few (`every=N`, slower ambient motion) are logged apart. `hz` is the refresh rate. `cpu_us` is the whole process's CPU time per frame: the CPU renderer (WARP) rasterises on a pool of threads, so this is several times `cost_us` and is the number to budget. `cost_us` is draw + present as the UI thread waits for it (keep it well under a refresh period); `draw_us` is painting alone, so `cost_us − draw_us` is the compositor hand-off; `gap_us` is the time between frames; `missed` counts frames whose gap exceeded 1.5× the median. The first frame after idle is excluded.

Other switches for measuring: `HYTTE_HOLD=1` starts the pill pinned open; `HYTTE_GPU=hardware` uses the graphics driver instead of WARP (about half the CPU for about 50 MB more memory; measured, not used); `HYTTE_SLOWMO=<n>` slows every animation `n` times.

For CPU and memory use Task Manager's *Details* tab, Process Explorer or Windows Performance Recorder, on **release** builds (debug is many times slower). A quick idle check: collapsed, nothing playing, watch it for a minute: CPU mostly 0 and a small "Context Switch Delta".

## Memory notes

- The canvas (two swap-chain buffers) is 460 × 310 logical px × 4 bytes, scaled by DPI (≈ 2.3 MB per buffer at 200 %).
- Album art and thumbnails are 64 × 64 BGRA (16 KB each); only visible shelf tiles have thumbnails and removed ones are freed.
- Drop Vault conversions are the only memory-heavy code: images over 60 megapixels are refused before decoding, the decoded original is freed before JPEG encoding, and JPEG → PDF reads only the header.
- Terminal tab handles are capped at 128.

Next: [13. Testing and releasing](13-testing-and-release.md)
