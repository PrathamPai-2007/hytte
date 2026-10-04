# 12. Performance

Hytte runs all day next to everything else on the machine, so its idle cost matters more than its peak speed. This section lists the targets, everything that can wake the process while idle, and the rules that keep it cheap.

## Budgets

| Budget | Target | Last measured (release build) |
|---|---|---|
| Idle CPU, collapsed, nothing playing | < 1 % | ≈ 0.2 % |
| Idle memory | < 30 MB stretch goal, 100 MB ceiling | ≈ 55–75 MB working set |
| Animation | Monitor refresh rate during transitions only | vsync-paced; parks when settled |

Release builds use `lto = true` and `codegen-units = 1` (root `Cargo.toml`) for a smaller binary and less resident code.

## The rules

1. **Render only on change.** A settled, static pill draws no frames. The frame clock parks ([section 8](08-rendering-and-animation.md)).
2. **Every animation has a stop condition**, encoded in `Model::ambient`. No condition means 30 fps forever.
3. **Prefer OS notifications to polling.** Where polling is unavoidable, use a long interval, document it with a `ponytail:` comment, and only send a `UiEvent` when the value changes.
4. **Wake the UI thread once per burst.** Events are queued and drained together (`Ui::handle_events`); the frame clock keeps at most one `WM_TICK` in flight.
5. **Use one timer for all deadlines.** `Model::expire` returns the next deadline, and one Win32 timer fires exactly then. Never add a periodic sweep.
6. **Heavy work goes off the UI thread.** File copies, image conversion, OCR, UI Automation, WinRT calls and thumbnails all run on workers.
7. **Allocate nothing per frame where you can avoid it.** Brushes, text formats, gradients, the hit list and the text buffer are reused across frames.

## Everything that can wake Hytte while idle

When adding a feature, check it doesn't add a row here without a good reason.

| Source | Interval | Notes |
|---|---|---|
| Port watcher | every `poll_secs` (5 s) | One `GetExtendedTcpTable` call per address family. Processes are opened only for watched ports. |
| Mic watcher | every 2 s | Reads the cached endpoints' mute state; re-enumerates only after a device change notification. |
| Power watcher | every 10 s | `GetSystemPowerStatus`. On laptops, also two IOCTLs on the cached battery device. |
| Privacy watcher | on registry change, or 10 s safety timeout | |
| Media watcher | on WinRT event, or 30 s safety timeout | |
| Task registry | every 5 s **only while tasks exist** | Checks owner liveness. |
| Countdown timer | 1 Hz **only while a timer runs unpaused** | Redraws the fuse. |
| Expiry timer | once, at the next deadline | |
| Foreground change / move | per event, debounced to one check per 150 ms | |
| Low-level mouse hook | every mouse event system-wide | The callback is O(1): atomics only, and it posts a message only when a drag enters or leaves the top-edge zone. |

None of these draws a frame unless the model actually changes.

## Frames: when they run

| Situation | Frames |
|---|---|
| Collapsed, nothing happening | none |
| Pointer moving over the pill | one per mouse move (hover effects follow the pointer) |
| A spring settling (resize, fade) | one per vblank, until settled |
| Running task, waiting agent, playing media in a media scene, drag over the pill, expanded privacy dots | ~30 fps |
| Failed task | ~30 fps for the first 8 s (the pulse fades), then none |
| Fullscreen sentinel | none (it never animates) |
| Hidden or paused | none |

## Measuring

### Frame cost

```powershell
$env:HYTTE_PERF = "1"; .\target\release\hytte.exe
```

Every 240 animated frames, a line like this is appended to `%APPDATA%\Hytte\hytte.log` (the numbers are illustrative):

```text
perf n=240 cost_us p50=310 p99=900 max=1400 | gap_us p50=16667 p99=17100 max=33000 | missed=2
```

- **`cost_us`** is the time to draw and present one frame. It should stay well under a refresh period.
- **`gap_us`** is the time between frames: about 16 667 µs at 60 Hz while springs move.
- **`missed`** counts frames whose gap exceeded 1.5× the median, meaning a vblank was missed.

The first frame after idle is excluded (its gap is the idle time).

### CPU and memory

Use Task Manager's *Details* tab (CPU, working set), Process Explorer, or Windows Performance Recorder / Analyzer for a timeline. Measure **release** builds: debug builds are many times slower at drawing and decoding.

A quick idle check: start Hytte, leave it collapsed with nothing playing, and watch it in Process Explorer for a minute. CPU should read 0 most of the time, and the "Context Switch Delta" column should stay small.

## Memory notes

- The canvas DIB is 460 × 310 logical px × 4 bytes, scaled by DPI (about 2.3 MB at 200 %).
- Album art and thumbnails are 64 × 64 BGRA (16 KB each). Only visible shelf tiles have thumbnails, and thumbnails of removed items are freed.
- Drop Vault conversions are the only memory-heavy code. Images over 60 megapixels are refused before decoding, the decoded original is freed before JPEG encoding, and JPEG → PDF reads only the image header.
- Terminal tab handles are capped at 128.

Next: [13. Testing and releasing](13-testing-and-release.md)
