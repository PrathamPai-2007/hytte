# 8. Rendering and animation

One frame from start to finish, then the spring physics and the rules that keep an idle pill at zero frames per second.

Code: `Ui::frame` in `window.rs`, `render.rs` (and `render/`), `animation.rs`, `Anim` in `ui_state.rs`.

## Reaching the screen

Hytte draws with **Direct2D** and **DirectWrite** (anti-aliased shapes, gradients, text, per-pixel alpha so the glow fades into what's behind). A Direct2D **device context** on a D3D11 device (`gpu.rs`) draws into a composition swap chain shown through **DirectComposition** in a `WS_EX_NOREDIRECTIONBITMAP` window: no readback, no per-frame copy. If the pipeline can't be created, the reason goes to `hytte.log` and Hytte exits.

### The GPU renderer

- `Gpu::new` creates a WARP D3D11 device (BGRA; see [section 12](12-performance.md#why-the-gpu-renderer-uses-warp)), a Direct2D device and context, a **flip-model composition swap chain** (premultiplied alpha, always the whole 460 × 310 canvas scaled by DPI) and a DirectComposition device, target and visual. If WARP can't be created it falls back to hardware.
- Each frame: `Gpu::begin` wraps the back buffer as the target, `draw` clears and paints, `Gpu::end` releases, `Gpu::present` calls `Present(0, 0)` (pacing is the frame clock's job). The whole-window fade (`vis`) is a DirectComposition effect group's opacity, free to draw.
- The window never moves or resizes once the monitor is known; what's visible and clickable is a **window region** over the pill and its glow. Setting a region is a window-manager round trip, so `present` jumps it to where the pill is *heading*, leaves it alone while moving, and trims it to the exact box once settled.
- On a lost device (`D2DERR_RECREATE_TARGET`, `DXGI_ERROR_DEVICE_REMOVED` / `RESET`), `Renderer::rebuild_gpu` rebuilds the pipeline for the same window and clears everything the old device made (brushes, album art, thumbnails, which are re-fetched). If it fails (driver mid-update, adapter gone) it retries on a timer with backoff from 1 s to 30 s.
- WARP draws on the CPU and avoids loading the GPU driver (≈ 45 MB less memory).

Frame cost p50 as each renderer was introduced (60 Hz, `HYTTE_PERF=1`, a task starting and finishing repeatedly; historical, current figures in [section 12](12-performance.md#budgets)): layered window with GPU DC target 6.5 ms; layered software target (removed in 1.2.2) 3.6 ms; gpu hardware device 0.9 ms; gpu WARP device 0.45 ms.

### The settings window's renderer

`settings_win.rs` ([section 7](07-window-and-input.md#timers)) has its own `Renderer` (`Renderer::new_for_window`): same device, brushes, text formats and helpers, but a swap chain sized to that window (`resize_window`) and `draw_settings_window`. It paints an opaque background, the `settings_table` cards under a scrolling transform, and the unscrolled section list. Hit regions are stored at on-screen position (`Renderer::oy` is the scroll offset added in `hit` and `over`).

It animates with `Renderer::ease(key, target)`: per-key values moved by `1 - e^(-16 dt)` each frame. A key starts at its target the first time it's seen (nothing animates on open), and `ease` marks the frame busy while any value moves, keeping the window's 60 Hz timer alive. `ease` is active only while the settings window draws (`fx`); the pill's settings pane draws the same rows through `setting_row` with values snapping to target, since the pill's frame clock stops when nothing else moves.

`settings_window_snapshots` (ignored; run with the other snapshots) draws the window offscreen in dark, light and narrow layouts; the screenshots in `docs/img` come from it.

## The frame clock

[Section 3](03-architecture.md#talking-to-the-ui-thread) introduced the ticker. Speeds:

| State (`Shared`) | Ticker | Used for |
|---|---|---|
| `animating = false` | Parked: no frames or wake-ups | A settled pill. |
| `animating`, `fast = true` | One frame per **vblank** (`DwmFlush`) | Springs moving. |
| `animating`, `fast = false` | One frame per ~33 ms (~30 fps) | Ambient-only motion. |

After each `DwmFlush` the ticker samples DWM timing (`DwmGetCompositionTimingInfo`: vblank time and refresh period). `Ui::frame` computes `dt` as an exact number of refresh periods, not jittery wall-clock time, so motion stays even; without that info it uses wall-clock time clamped to 50 ms.

## One frame: `Ui::frame`

1. **Time step** (`dt`, above).
2. **Layout:** `layout()` refreshes every spring target from the model ([section 6](06-ui-model.md)).
3. **Step:** `anim.step(dt × speed)`; `speed` is `[general] animation` (0.7 calm, 1 normal, 1.4 snappy). The idle dim (`Ui::dim`) is eased here too and keeps frames coming until settled.
4. **Visibility:** show the window if it should be visible; hide it once the `vis` fade reaches ~0.
5. **Crop:** `Renderer::plan(w, h, armed, room, glow)` sizes the visible region: the pill plus 28 px glow margin on sides and bottom (4 px with no glow, so the thin line over fullscreen video takes almost no clicks), plus `room` for a bubble, or the full canvas width and the 40 px hot strip while a drag is armed. Clipped to the canvas, converted to device pixels; the crop is the whole canvas and its `clip` becomes the window region.
6. **Position:** centred horizontally on the monitor's top edge (`origin_for`).
7. **Draw:** `Renderer::draw(frame, crop, &mut hits)` paints and fills the hit list.
8. **Present:** `Renderer::present` shows it with the whole-window opacity (`vis` × idle dim): region update plus `Present`, passing the box the pill is heading towards.
9. **Telemetry:** with `HYTTE_PERF=1`, record draw time, draw+present cost and frame gap.
10. **Keep going?** `animating = moving || ambient` with `ambient = !reduce && shown && model.ambient(scene, now)`; `fast = moving`. If both are false the ticker parks.

Because the canvas is fixed and only the region changes, the pill can grow and shrink every frame **without resizing the window** (slow, flickery).

## Inside `Renderer::draw`

Works in **logical pixels**; one transform `mat(scale, ox, oy)` maps to device pixels and puts the origin at the pill's top-left (`ox` centres it in the crop). Paint order (`Renderer::scene`):

1. **Background** (`background`): the **silhouette** (`pill_geometry`: top edge flares into small concave "ears" blending into the screen edge when taller than 18 px, rounded bottom corners); the **glow** (the path stroked 5 times with widening, fading strokes; stroking is a frame's costliest step; a waiting agent makes it pulse); the silhouette is cached while its size is unchanged (`geom`); the **body** (near-black vertical gradient at `pill_opacity` 0.4–1, body fill only so text and icons stay solid); the **tint** (glow colour pooling from the bottom, a cached gradient brush whose opacity is the glow strength, so it needs no frames of its own); a **rim** (barely there at the screen edge, brighter round the bottom curve) and a faint hover brightening. `Sentinel` is a thin rounded bar and stops here.
2. **Timer fuse:** while a timer runs, a glowing line burns down the pill edge (`timer_filament`).
3. **Outgoing content:** after a scene change, the **previous** scene at opacity `out`, drifting up a few pixels, clipped to the current pill. Its hit regions are discarded and it gets no hover, so nothing fading out is clickable.
4. **Current content:** opacity `content` × a height factor (content fades as the pill gets very short), sliding in from 6 px above. `Renderer::content` dispatches per scene: `idle`, `compact_task`, `compact_media`, `exp_tasks` (with the `servers` section), `exp_media`, `exp_shelf`, `exp_home`, `exp_power`, `exp_drop`, `exp_chip`, `exp_timer`, `exp_timer_done`, `exp_charge`.
5. **Tab strip** (`tab_strip`): drawn once after both scenes so it stays put while pages slide: one Fluent icon per panel, the current one a pill with its name; while paging the old tab shrinks as the new grows, both driven by `content` (the highlight travels with no state of its own). A hovered tab shows a small name label. On collapse it fades with `out` and takes no clicks.
6. **Overlays:** privacy dots and the red mic lock where the scene doesn't already show them.

### Drawing helpers

| Helper | Draws |
|---|---|
| `fill_rr`, `stroke_rr` | Rounded rectangles (solid or dashed stroke). |
| `circle`, `ring`, `line`, `poly` | Basic shapes; `poly` builds paths for small icons. |
| `text` | DirectWrite text into a clipped box with an ellipsis. |
| `label` | Like `text` but, instead of an ellipsis, a too-long label scrolls on hover (marquee: pause, glide, pause) and otherwise fades over its last ~26 px (14 slices of falling opacity). Every label is measured; only a scrolling one asks for frames (`wants_frames`). |
| `button` | Pill-shaped button that registers a hit region and brightens on hover. |
| `hit(x, y, w, h, action)` | Registers a clickable region; returns whether the pointer is over it. |
| `cc(color, alpha)` | A colour with the current content opacity: use it for all content colours so fades work. |

One `ID2D1SolidColorBrush` is reused for every solid colour. Gradient brushes (body, tint, rim, progress shimmer) live in one slot each (`Renderer::grad`), rebuilt only when colour or theme changes; per frame only end points and opacity are set. In the light theme `cc` draws saturated accents 22 % darker for contrast.

`Frame::accent` is the Windows accent colour (`window::windows_accent`, from the registry `AccentPalette`, re-read on `WM_SETTINGCHANGE`): light-2 shade on the dark pill, dark-1 on the light one. It colours selection and primary actions only (current tab, chosen preset, **Start timer**, **Rerun**, focus ring); status colours stay fixed. Text formats (`Fmts`) are created once; text uses one reused UTF-16 buffer.

### Theme, icons and the shared element slide

- **Theme.** `Frame::light` (from `[general] theme`: `dark`, `light` or `auto`, which reads the Windows app mode) switches the palette. Text and lines use `Renderer::fg()` (white, or near-black on light) and `ink()` (the opposite, for things on an `fg` shape); `body_colors()` gives the pill gradient, cached per theme.
- **Icons.** Quick-control tiles, the volume bar, the bell and the album-art placeholder are Segoe Fluent Icons glyphs (`Renderer::glyph` / `glyph_l`), sharp at every DPI. Media transport (`play_icon`, `pause_icon`, `skip_icon`), spinner, lock, tick, cross and battery are drawn shapes. The finished-timer bell is rotated (`rotate`) to swing.
- **Shared elements.** Between a compact scene and its expanded twin (`CompactMedia` ↔ `ExpMedia`, `CompactTask` ↔ `ExpTasks`) the album art and the primary task's icon and label slide from one place to the other instead of cross-fading. `Renderer::morph_for` takes progress from the pill's own height spring, so elements move exactly as fast as the pill grows, overshoot included. `hero(compact, exp, natural)` returns the interpolated rect while morphing (fully opaque, without the scenes' slide offsets, which would leave two copies a few pixels apart and dim them), else `natural`. Both scenes must use identical end points, which is why `label_ends` and `exp_label_w` are shared.
- **Press, page, bubble.** A button under the pointer shrinks 6 % and brightens while the left button is down (`Frame::pressed`). Paged changes slide in from the side ([section 6](06-ui-model.md#paging-squash-and-other-motion)). The bubble ([section 6](06-ui-model.md#bubbles)) is drawn first, behind the pill, with its own body, rim and content.

### Hit regions

Every `hit()` appends to a list. At the end of `draw` the renderer **swaps** it with the window's, so hits always match exactly what is on screen, without copying. Rectangles are in canvas-logical coordinates, the same space as `Ui::mouse_logical`.

### Bitmaps and progress

Workers send `ArtBitmap`s (64 × 64 premultiplied BGRA); the renderer makes `ID2D1Bitmap`s: **album art** is re-created only when `Model::art_gen` changes (`sync_art`); **shelf thumbnails** are kept in a map by item id (`missing_thumbs` lists visible tiles without one and drops removed ones; `Ui::request_thumbs` fetches each missing one once on a short-lived thread). Both paint through a bitmap brush into a rounded rectangle.

Determinate progress eases towards the reported percentage (`Renderer::shown`, keyed by task id, pruned when tasks go).

## Springs

`animation.rs` implements a damped harmonic spring with a **closed-form** solution (under-, critically and over-damped), so one big step lands where many small ones would and motion is frame-rate independent (test `step_size_does_not_change_the_motion`). A `Spring` has `pos`, `vel`, `target`, damping ratio `zeta`, `period` and settle threshold `eps`; once position and velocity are within `eps` it snaps to target and reports "not moving", which lets the frame clock stop. A global `SLOW = 1.18` stretches every period slightly.

| Spring | Zeta / period | Character |
|---|---|---|
| Pill size, expanding (`GROW`) | 0.86 / 0.30 s | A touch of overshoot that settles. |
| Pill size, collapsing (`SHRINK`) | 1.0 / 0.40 s | Slower, no bounce. |
| Content fade-in / outgoing fade | 1.0 / 0.18 s / 0.16 s | |
| Glow / hover brightness / visibility | 1.0 / 0.30 s / 0.16 s / 0.22 s | |
| Bubble | 0.72 / 0.34 s | Small overshoot. |

`RectSpring::set_target` picks `GROW` or `SHRINK` **when the target changes** (by comparing areas), not every frame; otherwise an expansion's overshoot would read as a collapse. Retargeting keeps velocity, so leaving mid-expansion turns around smoothly.

**Reduced motion.** With Windows "Animation effects" off (`SPI_GETCLIENTAREAANIMATION`), `Anim::reduce` makes every spring snap, scene changes not fade, and ambient animation stop. Re-read on `WM_SETTINGCHANGE`.

## Ambient animation, and when it stops

Ambient animation is the only thing keeping frames running when no spring moves, so the rules (`Model::ambient`) are deliberately narrow:

| Runs while | Why |
|---|---|
| A visible task runs or waits for input | Spinner and amber pulse. |
| A failed task's red pulse is fading (first 8 s) | Afterwards it rests at a fixed value, so a forgotten failure costs nothing. |
| An expanded scene shows privacy dots | They breathe only expanded. |
| A drag is over the pill | The drop zone animates. |
| The plug-in / unplug card shows (3 s) | Battery fills or drains. |
| The timer-finished card shows (≤ 2 min) | Bell swings, ripples. |
| Media plays **and** a media scene is shown | Equaliser and timeline. |
| The pointer rests on a cut-off label (`wants_frames`) | It scrolls; stops when the pointer leaves. |

The bubble spring moves for a fraction of a second then rests; the timer bubble's clock is redrawn by the existing 1 Hz timer, not an animation. Never in the `Sentinel` scene, while hidden, or with reduced motion.

When you add motion, make it a function of `self.t` or `Frame::now`, decide when it may stop, and add that condition to `Model::ambient`. **An animation with no stop condition runs the CPU forever.**

Next: [9. Background workers](09-workers.md)
