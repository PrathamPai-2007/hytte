# Baseline and measurements

Working notes for the motion and look release. Deleted at release; the final numbers move to
`docs/12-performance.md`.

How: release build from `target\release`, isolated `%APPDATA%`, `fullscreen = "off"`,
`HYTTE_PERF=1`. "CPU" is the whole process's processor time (all threads) over a window, as a
share of one core. `HYTTE_HOLD=1` starts the pill pinned open. The display was at **60 Hz** for
every run here (not 144).

## 1.9.0 behaviour (2026-11-15, before any motion change)

| Situation | CPU | Frame, UI thread (`cost_us` p50 / p99) |
|---|---|---|
| Idle, collapsed, after 45 s | 0.78 % (234 ms in 30 s) | no frames |
| Private bytes / threads / handles | 13.3 MB / 32 / 435 | |
| Task starting and finishing (springs, 60 fps) | not sampled | 1.9–2.1 ms / 3.3–7.0 ms, 5–7 "missed" per 240 |
| Running task, collapsed (spinner, 30 fps) | 16.9 % and 14.6 % (two runs) | 1.3–1.9 ms / 1.7–3.3 ms |
| Open (pinned), nothing running | 0.73 % | no frames |
| Running task, open (30 fps) | 26.2 % and 27.6 % | 1.9 ms / 2.4–2.7 ms |

Media playing was not measured (no player was driven).

## What these numbers mean

- **`cost_us` under-reports by about four times.** It is what the UI thread waits. WARP hands the
  rasterising to a pool: in a 10 s sample the busiest thread used 3.6 % and a dozen others shared
  about 11 %. So a collapsed pill's frame is about 5 ms of processor time, and an open one about
  9 ms, not the 1 to 2 ms the docs say. The perf line now reports `cpu_us` (process CPU per
  frame) and that is the number to budget.
- **The plan's pacing assumption was wrong.** It assumed about 1 ms a frame. At 9 ms, an open
  pill animating at 60 fps would cost about half a core; at 120 fps, a whole one.
- **The spring "missed" counts may be pace changes, not dropped frames.** Frames were classed by
  the pace chosen at the end of the frame; the gap was set by the pace before. Fixed in the perf
  line (it now uses the pace that timed the gap); to be re-measured.

## Experiments (each one 15 s sample, so differences under about 3 points are noise)

| Change | Running task, collapsed | Running task, open | Verdict |
|---|---|---|---|
| None | 14.6–16.9 % | 26.2–27.6 % | |
| Glow off (`glow_strength = 0`) | 10.6 % | | The five glow strokes are about a quarter of a collapsed frame |
| Clear, draw and present only the visible box (dirty rectangle) | 12.8 % | 27.6 % | No real gain. Reverted |
| WARP on the calling thread only (`PREVENT_INTERNAL_THREADING_OPTIMIZATIONS`) | 10.7 % | 20.2 % | Less CPU, but the UI thread's frame went from 1.3 to 3.2 ms (6.1 ms open). Reverted: it would miss frames at high refresh rates |

With WARP on one thread the UI-thread cost equals the processor cost, which gives a clean split
(measured with that flag, since reverted):

| | Whole frame | Without the background | Background only |
|---|---|---|---|
| Collapsed, spinner | 3.2 ms | 1.4 ms | 2.7 ms |
| Open, spinner | 6.1 ms | 2.3 ms | 4.7 ms |

**The pill's background (glow strokes, body, tint, rim, hover) is 60 to 75 % of every frame.**
Drawing it once and reusing the picture while it doesn't change is the lever; the gain is capped
at about 2 to 3 times.

## Re-measured with the corrected perf line (median of three 12 s samples)

`cpu_us` is the process's CPU time per frame. Frames are now classed by the clock that timed
their gap.

| Situation | CPU | `cpu_us` per frame | `cost_us` p50 / p99 |
|---|---|---|---|
| Running task, collapsed (30 fps) | 14.6 % | about 5.1 ms | 1.3 / 1.8 ms |
| Running task, open (30 fps) | 27.1 % | about 9.2 ms | 1.9 / 2.4 ms |
| Task starting and finishing (springs) | | 10 to 23 ms | 1.9–2.2 / 3.2–3.8 ms, 4–6 "missed" |

The "missed" spring frames are one per change from ambient to spring pacing: the ticker is in
its 33 ms sleep when the spring starts, so the first spring frame arrives up to two periods
late. Vblank-counted pacing (Task 5) removes the sleep.

### The graphics driver instead of WARP (`HYTTE_GPU=hardware`, a data point, not a plan)

| Situation | CPU | Private bytes | `cost_us` p50 |
|---|---|---|---|
| Running task, collapsed | 8.7 % | 66 MB (WARP: 17 MB) | 2.1 ms (WARP: 1.3) |
| Running task, open | 12.9 % | 66 MB | 2.9 ms (WARP: 1.9) |

About half the CPU for about 50 MB more memory and a slower UI thread. Not taken: the memory
budget is 25 MB.

## After the background cache (the steady background kept as two bitmaps)

| Situation | Before | After |
|---|---|---|
| Running task, collapsed (30 fps) | 14.6 % | **5.6 %** |
| Running task, open (30 fps) | 27.1 % | **11.6 %** |
| Idle, collapsed | 0.78 % (one 30 s sample) | 0.52 % |
| Private bytes, idle / collapsed task / open task | 13.3 / 17.0 / 20.6 MB | 13.4 / 18.9 / 20.8 MB |
| Task starting and finishing (springs), `cpu_us` | 10 to 23 ms | 12 to 17 ms (unchanged: the background changes every frame of a resize) |

With the glow off, spring frames cost about 9.5 ms: the five glow strokes are about a third of
a transition frame. That is what the mask in Task 6 has to beat.

**Pacing this allows:** an open pill at 60 fps now costs about what 30 fps did in 1.9.0.
