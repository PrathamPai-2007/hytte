# 8. Rendering and animation

This section follows one frame from start to finish, then explains the spring physics and the rules that keep an idle pill at zero frames per second.

Code: `Ui::frame` in `crates/hytte/src/window.rs`, `crates/hytte/src/render.rs`, `crates/hytte/src/animation.rs`, and `Anim` in `crates/hytte/src/ui_state.rs`.

## Two ways to reach the screen

Hytte draws with **Direct2D** and **DirectWrite**: anti-aliased shapes, gradients and text, with real per-pixel alpha so the soft glow fades into whatever is behind it. There are two renderers, chosen once at startup by `[general] renderer` (the window's style depends on it, so it can't change while running):

| | `gpu` (default) | `classic` |
|---|---|---|
| Draws with | A Direct2D **device context** on a D3D11 device (`gpu.rs`) | An `ID2D1DCRenderTarget` bound to a 32-bit premultiplied **DIB section** |
| Shown with | A composition swap chain through **DirectComposition** | `UpdateLayeredWindow` |
| Window style | `WS_EX_NOREDIRECTIONBITMAP` | `WS_EX_LAYERED` |
| Click-through | A **window region** (see below) | Fully transparent pixels |
| Cost | No readback, no per-frame copy | One CPU copy to the compositor per frame |

The `gpu` renderer falls back to `classic` by itself if the D3D11 / DirectComposition pipeline can't be created (the reason is written to `hytte.log`). `Renderer` hides the difference: every drawing function uses the same `ID2D1RenderTarget`, and only `new`, `plan`, `draw` and `present` branch.

### The GPU renderer

- `Gpu::new` creates a hardware D3D11 device (BGRA support), a Direct2D device and device context, a **flip-model composition swap chain** with premultiplied alpha, and a DirectComposition device, target and visual. The swap chain is always the whole canvas (460 × 310 logical px, scaled by DPI).
- Each frame, `Gpu::begin` wraps the current back buffer as the device context's target, `draw` clears it and paints, and `Gpu::end` releases it. `Gpu::present` calls `Present(0, 0)` (pacing is the frame clock's job, not the swap chain's). The whole-window fade (`vis`) is a DirectComposition effect group's opacity, so it costs nothing to draw.
- The window never moves or resizes after the monitor is known: it is the canvas, centred on the top edge. What is visible and clickable is a **window region** covering the pill and its glow (the same box the classic path presents). Setting a region is a window-manager round trip, so `present` jumps it straight to where the pill is *heading*, leaves it alone while the pill moves, and trims it to the exact box once it settles.
- If Direct2D or DXGI reports a lost device (`D2DERR_RECREATE_TARGET`, `DXGI_ERROR_DEVICE_REMOVED` / `RESET`), `Renderer::rebuild_gpu` drops the old pipeline, builds a new one for the same window, and clears everything the old device made (brushes, album art and thumbnails, which are then fetched again).

### The classic renderer

Direct2D draws into a DIB, then `UpdateLayeredWindow` hands the bitmap to Windows at the crop's size and position. The target is a **software** (CPU) one: the pill is a few hundred pixels, and a GPU-backed DC target spends more on reading the result back than it saves drawing (measured p50 6.5 ms → 3.6 ms per frame). `HYTTE_HARDWARE=1` selects the GPU-backed DC target for comparison.

### Measured

On the development machine (60 Hz display, `HYTTE_PERF=1`, a task starting and finishing repeatedly), frame cost p50:

| Renderer | draw + present |
|---|---|
| classic, GPU-backed DC target (the old default) | 6.5 ms |
| classic, software target | 3.6 ms |
| gpu | 2.1 ms |

## The frame clock

[Section 3](03-architecture.md#the-frame-clock) introduced the ticker thread. Its two speeds:

| State (`Shared`) | Ticker behaviour | Used for |
|---|---|---|
| `animating = false` | Parked: no frames, no wake-ups | A settled pill. |
| `animating`, `fast = true` | One frame per **vblank** (`DwmFlush`) | Springs moving (resize, fades). |
| `animating`, `fast = false` | One frame every ~33 ms (about 30 fps) | Ambient-only motion: spinner, equaliser, pulses. |

Right after each `DwmFlush`, the ticker samples DWM's composition timing (`DwmGetCompositionTimingInfo`): the vblank time and the refresh period. `Ui::frame` uses them to compute `dt` as an exact number of refresh periods, rather than jittery wall-clock time, so motion stays perfectly even. When that information isn't available, it falls back to wall-clock time, clamped to 50 ms.

## One frame: `Ui::frame`

1. **Time step.** Compute `dt` (above).
2. **Layout.** `layout()` refreshes every spring's target from the model ([section 6](06-ui-model.md)).
3. **Step.** `anim.step(dt)` advances all springs and reports whether anything is still moving.
4. **Visibility.** Show the window if it should become visible. Hide it once the `vis` fade-out has reached ~0.
5. **Crop.** `Renderer::plan(w, h, armed)` sizes the region to draw: the pill plus 28 px of margin for the glow on the sides and bottom, or the whole canvas width and the 40 px hot strip while a drag is armed. Clipped to the canvas, converted to device pixels. A `Crop` also carries `clip`, the box that holds the pill and glow: the classic renderer's crop *is* that box, while the gpu renderer's crop is the whole canvas and `clip` becomes the window region.
6. **Position.** The crop is centred horizontally on the monitor at its top edge (`origin_for`).
7. **Draw.** `Renderer::draw(frame, crop, &mut hits)` paints the scene and fills the hit list.
8. **Present.** `Renderer::present` shows the frame with the whole-window opacity (from the `vis` spring): `UpdateLayeredWindow` (classic), or a window-region update plus `Present` (gpu). The caller also passes the box the pill is heading towards, which the gpu renderer uses to avoid chasing every frame with the region.
9. **Telemetry.** With `HYTTE_PERF=1`, record the draw time, the draw+present cost and the frame gap.
10. **Keep going?** `animating = moving || ambient`, where `ambient = !reduce && shown && model.ambient(scene, now)`. `fast = moving`. If both are false, the ticker parks after this frame.

Because the canvas is fixed and only the crop (classic) or the region (gpu) changes, the pill can grow and shrink every frame **without resizing the window**. Window resizes are slow and flicker.

## Inside `Renderer::draw`

The renderer works in **logical pixels**. One transform, `mat(scale, ox, oy)`, maps them to device pixels and moves the origin to the pill's top-left corner (`ox` centres the pill in the crop).

Order of painting (`Renderer::scene`):

1. **Background** (`background`)
   - The pill **silhouette** (`pill_geometry`): a path whose top edge flares out into small concave "ears" that blend into the screen edge (only when the pill is taller than 18 px), with rounded bottom corners.
   - The **glow**: the same path stroked 5 times with widening, fading strokes in the glow colour (each stroke stands in for a pair of the nine it used to take: stroking is the costliest thing a frame does). An agent waiting for input makes it pulse.
   - The silhouette is kept between frames while its size is unchanged (`geom`), so ambient frames don't rebuild it.
   - The **body**: a near-black vertical gradient (slightly translucent with `acrylic = true`).
   - A hairline rim, and a faint brightening while hovered.
   - The `Sentinel` scene is just a thin rounded bar and stops here.
2. **Timer fuse.** While a timer runs, a glowing line burns down along the pill edge (`timer_filament`).
3. **Outgoing content.** If the scene just changed, the **previous** scene's content is drawn with opacity `out`, drifting up a few pixels and clipped to the current pill rectangle. Its hit regions are thrown away and it gets no hover state, so you can't click something that is fading out.
4. **Current content.** The current scene, with opacity `content` × a height factor (content fades away as the pill gets very short), sliding down from 6 px above into place. `Renderer::content` dispatches to one function per scene: `idle`, `compact_task`, `compact_media`, `exp_tasks`, `exp_media`, `exp_ports`, `exp_shelf`, `exp_home`, `exp_drop`, `exp_chip`, `exp_timer`, `exp_timer_done`, plus the tab dots.
5. **Overlays.** The privacy dots and the red mic lock, where the scene doesn't already show that information.

### Drawing helpers

| Helper | Draws |
|---|---|
| `fill_rr`, `stroke_rr` | Rounded rectangles (solid or dashed stroke). |
| `circle`, `ring`, `line`, `poly` | Basic shapes; `poly` builds a path for small icons. |
| `text` | DirectWrite text into a box, clipped, with an ellipsis when it doesn't fit. |
| `button` | A pill-shaped button that registers a hit region and brightens on hover. |
| `hit(x, y, w, h, action)` | Registers a clickable region and returns whether the pointer is over it, so callers can draw a hover state. |
| `cc(color, alpha)` | A colour with the current content opacity applied. Use it for all content colours so fades work. |

One `ID2D1SolidColorBrush` is reused for every solid colour (`solid` just changes its colour). The two gradient brushes (pill body and progress shimmer) are built once and only have their end points and opacity updated per frame. Text formats (`Fmts`) are created once at startup. Text goes through one reused UTF-16 buffer.

### Hit regions

Every `hit()` call during drawing appends to a list. At the end of `draw`, the renderer **swaps** that list with the window's, so the hits always match exactly what is on screen, without copying. Hit rectangles are stored in canvas-logical coordinates, the same space as `Ui::mouse_logical`.

### Bitmaps: album art and thumbnails

Images arrive from workers as `ArtBitmap`: 64 × 64 premultiplied BGRA bytes. The renderer turns them into `ID2D1Bitmap`s:

- **Album art** is re-created only when `Model::art_gen` changes (`sync_art`).
- **Shelf thumbnails** are kept in a map by item id. `missing_thumbs` lists the visible tiles without one, and drops thumbnails for removed items. The window asks a short-lived thread to fetch each missing one (`Ui::request_thumbs`, at most once per item).

Both are painted through a bitmap brush into a rounded rectangle, so they get rounded corners.

### Progress smoothing

Determinate progress bars ease towards the reported percentage instead of jumping. The eased values live in `Renderer::shown`, keyed by task id, and are pruned when tasks disappear.

## Springs

`animation.rs` implements a damped harmonic spring with a **closed-form** solution (under-damped, critically damped and over-damped cases). Advancing by one big step lands in exactly the same place as many small steps, so the motion doesn't depend on the frame rate. The test `step_size_does_not_change_the_motion` checks this.

A `Spring` has `pos`, `vel`, `target`, a damping ratio `zeta`, a `period`, and a settle threshold `eps`. Once both position and velocity are within `eps`, it snaps to the target and reports "not moving". That is what lets the frame clock stop. A global `SLOW = 1.18` factor stretches every period slightly.

| Spring | Zeta / period | Character |
|---|---|---|
| Pill size, **expanding** (`GROW`) | 0.78 / 0.28 s | Lively, a small overshoot. |
| Pill size, **collapsing** (`SHRINK`) | 1.0 / 0.40 s | Slower, no bounce: eases shut. |
| Content fade-in | 1.0 / 0.18 s | |
| Outgoing content fade | 1.0 / 0.16 s | |
| Glow | 1.0 / 0.30 s | |
| Hover brightness | 1.0 / 0.16 s | |
| Visibility | 1.0 / 0.22 s | |

`RectSpring::set_target` picks `GROW` or `SHRINK` **when the target changes** (by comparing the target areas), not every frame. Otherwise an expansion's overshoot would be mistaken for a collapse.

Retargeting keeps velocity: if the pill is expanding and you leave before it finishes, it turns around smoothly instead of jumping.

### Reduced motion

If Windows' "Animation effects" setting is off (`SPI_GETCLIENTAREAANIMATION`), `Anim::reduce` is set. Every spring then snaps to its target, scene changes don't fade, and ambient animation stops (the `ambient` check includes `!reduce`). It is re-read on `WM_SETTINGCHANGE`.

## Ambient animation, and when it stops

Ambient animation is the only thing that keeps frames running when no spring is moving, so the rules (`Model::ambient`) are deliberately narrow:

| Runs while | Why it may run |
|---|---|
| A visible task is running or waiting for input | The spinner and the amber pulse are the point. |
| A failed task's red pulse is still fading (first 8 s) | After that the pulse rests at a fixed value, so a forgotten failure costs nothing. |
| An expanded scene shows privacy dots | They breathe only when expanded; collapsed, they are static. |
| Something is dragged over the pill | The drop zone animates. |
| The plug-in card is showing (3 s) | The battery fills up; it expires on its own. |
| Media is playing **and** a media scene is shown | The equaliser and timeline. |

Never in the `Sentinel` (fullscreen) scene, never while hidden, and never with reduced motion.

When you add motion, make it a function of `self.t` (animation time) or `Frame::now`, decide when it may stop, and add that condition to `Model::ambient`. **An animation with no stop condition runs the CPU forever.**

Next: [9. Background workers](09-workers.md)
