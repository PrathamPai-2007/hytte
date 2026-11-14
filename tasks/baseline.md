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
