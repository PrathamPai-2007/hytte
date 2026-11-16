# 8. Rendering and animation

One frame from start to finish, then the spring physics and the rules that keep an idle pill at zero frames per second.

Code: `Ui::frame` in `window/win/frame.rs`, `render.rs` (and `render/`), `animation.rs`, `Anim` in `ui_state/anim.rs`.

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

It animates with `Renderer::ease(key, target)`: per-key values moved by `1 - e^(-16 dt)` each frame. A key starts at its target the first time it's seen (nothing animates on open), and `ease` marks the frame busy while any value moves, keeping the window's 60 Hz timer alive. `ease` is active only while the settings window draws (`fx`); the pill's frame clock stops when nothing else moves, so the pill itself never eases.

`settings_window_snapshots` (ignored; run with the other snapshots) draws the window offscreen in dark, light and narrow layouts; the screenshots in `docs/img` come from it.

## The frame clock

[Section 3](03-architecture.md#talking-to-the-ui-thread) introduced the ticker. Speeds:

| State (`Shared`) | Ticker | Used for |
|---|---|---|
| `animating = false` | Parked: no frames or wake-ups | A settled pill. |
| `animating`, `stride = 1` | One frame per **vblank** (`DwmFlush`) | A spring or a renderer tween moving. |
| `animating`, `stride = round(hz / 66)` | Every few vblanks: 60 fps at 60 / 120 / 240 Hz, 72 at 144 Hz | Ambient motion in an open or pointed-at pill. |
| `animating`, `stride = round(hz / 30)` | About 30 fps | Ambient motion on a collapsed pill. |

Every frame lands on a vblank, so slower ambient motion steps evenly instead of beating against the display, and a spring that starts while the ticker waits out a long stride gets its frame at the next vblank. The two ambient rates are `AMBIENT_OPEN_HZ` and `AMBIENT_COLLAPSED_HZ` in `window/win.rs`. If `DwmFlush` fails or returns at once (the compositor is restarting, the display is changing mode, a full-screen game owns it) the ticker sleeps 16 ms instead of spinning.

After each wait the ticker samples DWM timing (`DwmGetCompositionTimingInfo`: vblank time and refresh period). `Ui::frame` computes `dt` as an exact number of refresh periods, not jittery wall-clock time, so motion stays even; without that info it uses wall-clock time clamped to 50 ms. `HYTTE_SLOWMO=<n>` divides it by `n`, for looking at motion closely.

## One frame: `Ui::frame`

1. **Time step** (`dt`, above).
2. **Layout:** `layout()` refreshes every spring target from the model ([section 6](06-ui-model.md)).
3. **Step:** `anim.step(dt × speed)`; `speed` is `[general] animation` (0.7 calm, 1 normal, 1.4 snappy). The idle dim (`Ui::dim`, a spring) steps here too. The frame's `dt` carries the same speed, so the renderer's tweens scale with it.
4. **Visibility:** show the window if it should be visible; hide it once the `vis` fade reaches ~0.
5. **Crop:** `Renderer::plan(w, h, armed, room, glow)` sizes the visible region: the pill plus 28 px glow margin on sides and bottom (4 px with no glow, so the thin line over fullscreen video takes almost no clicks), plus `room` for a bubble, or the full canvas width and the 40 px hot strip while a drag is armed. Clipped to the canvas, converted to device pixels; the crop is the whole canvas and its `clip` becomes the window region.
6. **Position:** centred horizontally on the monitor's top edge (`origin_for`).
7. **Draw:** `Renderer::draw(frame, crop, &mut hits)` paints and fills the hit list.
8. **Present:** `Renderer::present` shows it with the whole-window opacity (`vis` × idle dim): region update plus `Present`, passing the box the pill is heading towards.
9. **Telemetry:** with `HYTTE_PERF=1`, record draw time, draw+present cost and frame gap.
10. **Keep going?** `moving` also counts the renderer's tweens (`Renderer::tweening`). `animating = moving || ambient` with `ambient = !reduce && shown && model.ambient(scene, now)`; `stride` is 1 while moving, otherwise the ambient stride for an open or collapsed pill. If neither, the ticker parks.

Because the canvas is fixed and only the region changes, the pill can grow and shrink every frame **without resizing the window** (slow, flickery).

## Inside `Renderer::draw`

Works in **logical pixels**; one transform `mat(scale, ox, oy)` maps to device pixels and puts the origin at the pill's top-left (`ox` centres it in the crop). Paint order (`Renderer::scene`):

1. **Background** (`background`): the **silhouette** (`pill_geometry`: top edge flares into small concave "ears" blending into the screen edge when taller than 18 px; continuous-curvature bottom corners, `smooth_corner`); the **glow** (the path stroked 5 times with widening, fading strokes; a waiting agent makes it breathe); the silhouette is cached while its size is unchanged (`geom`); a soft **shadow** under an open pill's lower half (4 dark strokes moved 5 px down, growing in as the pill opens, none below `pill_opacity` 0.6 so it never shows through the body); the **body** (near-black vertical gradient at `pill_opacity` 0.4–1); the **tint** (glow colour pooling from the bottom); a **rim** and a faint hover brightening. `Sentinel` is a thin rounded bar and stops here.

   **The background cache.** Stroking and filling the silhouette is 60 to 75 % of a frame on the CPU renderer, and a steady pill draws the same background every frame while a spinner moves in front of it. So once everything it depends on (size, scale, sub-pixel position, glow strength and colour, opacity, theme, skin, hover, shadow) has been the same for two frames, it is painted into two bitmaps, the glow and the rest (`BgCache`), and copied from them until something changes. A waiting agent's breathing is the glow bitmap drawn at a changing opacity, so that long wait stays cached (the tint inside no longer breathes with it). The bitmaps are freed with the 30 s memory trim, on a device rebuild and on a scale change. `the_cached_background_matches_the_painted_one` checks the copy against painting.
2. **Timer fuse:** while a timer runs, a glowing line burns down the pill edge (`timer_filament`). A Focus session draws a thin pale line instead, amber in the last minute; on a bare pill it redraws only about every `total / 100` seconds (a pixel's worth).
3. **Scenes leaving:** every layer in `Anim::shown()` that isn't the current scene, back to front, at its own opacity and offset, clipped to the current pill. Their hit regions are discarded and they get no hover, so nothing fading out is clickable.
4. **Current content:** the current layer's opacity × a height factor (content fades as the pill gets very short), at its offset. While it fades in, `Renderer::group(i)` staggers what a scene draws after it: group `i` comes in `STAGGER_STEP` (6 %) of the fade behind group `i − 1` and rises 4 px into place, up to 5 groups, so a pane builds from the top. Groups come from the layer's own spring: nothing new to settle. Settled, `group` does nothing. `Renderer::content` dispatches per scene: `idle`, `compact_task`, `compact_media`, `exp_tasks` (with the `servers` section), `exp_media` (and its lyrics page and sleep chips), `exp_shelf`, `exp_home`, `exp_power`, `exp_notes`, `exp_todo`, `exp_weather`, `exp_nudge`, `exp_drop`, `exp_chip`, `exp_timer`, `exp_timer_done`, `exp_charge`.
5. **Tab strip** (`tab_strip`): drawn once after both scenes so it stays put while pages slide: one Fluent icon per panel, the current one a pill with its name; while paging the old tab shrinks as the new grows, both driven by `content` (the highlight travels with no state of its own). A hovered tab shows a small name label. On collapse it fades with `out` and takes no clicks.
6. **Overlays:** privacy dots and the red mic lock where the scene doesn't already show them.

### Drawing helpers

| Helper | Draws |
|---|---|
| `fill_rr`, `stroke_rr` | Rounded rectangles (solid or dashed stroke). |
| `circle`, `ring`, `line`, `poly` | Basic shapes; `poly` builds paths for small icons. |
| `text` | DirectWrite text into a clipped box with an ellipsis. |
| `label` | Like `text` but, instead of an ellipsis, a too-long label scrolls on hover (marquee: pause, glide, pause) and otherwise fades over its last ~26 px (14 slices of falling opacity). Every label is measured; only a scrolling one asks for frames (`wants_frames`). |
| `button`, `primary`, `round_btn`, `tile` | Controls built on `press`: hover and press are eased (`QUICK` in, `POP` on release), and the whole control scales 6 % about its centre while held, springing back when let go. A click quicker than a frame still shows pressed for 80 ms (`shows_pressed`). Buttons are 24 or 32 px tall. |
| `track` | The one thin slider (media timeline, settings window): track, fill, thumb; thickens and grows its thumb under the pointer. |
| `digits` | A number with every digit in a cell as wide as a "0" (no sideways jitter). With a roll key a changed digit rolls in (up when it grows, down when it shrinks); only open panes roll, since a roll asks for full-rate frames for its quarter second. |
| `play_pause` | Play and pause as one shape: each half of the triangle becomes a bar. |
| `smooth_rr` | Used by `fill_rr` / `stroke_rr` for radii of 12 or more that aren't full pills: a rounded rectangle with continuous-curvature corners, cached by box. |
| `row(key, y)` | A list row's drawn position and opacity: it slides when the rows above change; one appearing in a pane already showing (`steady`: fully shown this frame and the last) fades in from 10 px below. |
| `hit(x, y, w, h, action)` | Registers a clickable region; returns whether the pointer is over it. |
| `cc(color, alpha)` | A colour with the current content opacity: use it for all content colours so fades work. |

One `ID2D1SolidColorBrush` is reused for every solid colour. Gradient brushes (body, tint, rim, progress shimmer) live in one slot each (`Renderer::grad`), rebuilt only when colour or theme changes; per frame only end points and opacity are set. In the light theme `cc` draws saturated accents 22 % darker for contrast.

`Frame::accent` is the Windows accent colour (`window::windows_accent`, from the registry `AccentPalette`, re-read on `WM_SETTINGCHANGE`): light-2 shade on the dark pill, dark-1 on the light one. It colours selection and primary actions only (current tab, chosen preset, **Start timer**, **Rerun**, focus ring); status colours stay fixed. Text formats (`Fmts`) are created once; text uses one reused UTF-16 buffer.

### Tweens: what the renderer eases

Hover, press, toggles, progress, positions and scrolling are only known while drawing, so they are eased in the renderer, not the model: `Renderer::tw(key, target, feel)` (and `tw_px`, `tw_rgb`, `tw_drag`) return where a value is this frame (`render/tween.rs`). Keys are `tween::key(tag, id)`. A key seen for the first time starts at its target, so nothing animates when a pane opens; a key not asked for during a frame is forgotten; `kick` pushes a value off its target (the scroll bounce); `set` puts it somewhere at once (a drag follows the pointer exactly). While any value moves, `tweening()` keeps the frame clock at the refresh rate. With reduced motion every value is its target. Snapshots call `reset_tweens` so a picture shows values settled.

What uses it: the settings window's hover and selections (`ease`), task progress, the timeline, the volume, tile tints, the to-do tick and strike-through, timer presets, the tab highlight, rows sliding and leaving, rolling digits, answer scrolling.

**Rows leaving.** A note or to-do drawn last frame and gone from the list now fades out where it was, drifting left (`jot_leaving`). It is the renderer's copy: the model removes the item at once, so nothing is kept or saved.

### The look's tokens

`render/tokens.rs`: nested radii (`R_PILL` 24, `R_CARD = inner(R_PILL, 10)`, `R_ROW = inner(R_PILL, 14)`), `INK_3` for fine print, and `PANE_GAP`, the 12 px every pane leaves between its last element and the tab strip. The type scale is five sizes (`Fmts`): caption 10.5 in Segoe UI Variable Small, body 12, title 13, heading 15, display 27; mono 10.

### Theme, icons and the shared element slide

- **Theme.** `Frame::light` (from `[general] theme`: `dark`, `light` or `auto`, which reads the Windows app mode) switches the palette. Text and lines use `Renderer::fg()` (white, or near-black on light) and `ink()` (the opposite, for things on an `fg` shape); `body_colors()` gives the pill gradient, cached per theme.
- **Skins.** `Frame::model.skin` picks the body gradient, the rim tint and the colour of a scene's neutral glow; the cached gradient brushes are keyed by it, so changing skin rebuilds three brushes once and an idle pill draws nothing.
- **Icons.** Quick-control tiles, the volume bar, the bell and the album-art placeholder are Segoe Fluent Icons glyphs (`Renderer::glyph` / `glyph_l`), sharp at every DPI. Media transport (`play_icon`, `pause_icon`, `skip_icon`), spinner, lock, tick, cross and battery are drawn shapes. The finished-timer bell is rotated (`rotate`) to swing.
- **Shared elements.** Between a compact scene and its expanded twin (`CompactMedia` ↔ `ExpMedia`, `CompactTask` ↔ `ExpTasks`) the album art and the equaliser, and the primary task's icon, label and right-hand value, slide from one place to the other instead of cross-fading (`eq_ends`, `label_ends`, `value_ends`). `Renderer::morph_for` takes progress from the pill's own height spring, so elements move exactly as fast as the pill grows, overshoot included. `hero(compact, exp, natural)` returns the interpolated rect while morphing (fully opaque, without the scenes' slide offsets, which would leave two copies a few pixels apart and dim them), else `natural`. Both scenes must use identical end points, which is why `label_ends` and `exp_label_w` are shared.
- **Press, page, bubble.** See `press` above for buttons. Paged changes slide in from the side ([section 6](06-ui-model.md#paging-squash-and-other-motion)). The bubble ([section 6](06-ui-model.md#bubbles)) is drawn first, behind the pill, with its own body, rim and content.

### Hit regions

Every `hit()` appends to a list. At the end of `draw` the renderer **swaps** it with the window's, so hits always match exactly what is on screen, without copying. Rectangles are in canvas-logical coordinates, the same space as `Ui::mouse_logical`.

### Bitmaps and progress

Workers send `ArtBitmap`s (64 × 64 premultiplied BGRA); the renderer makes `ID2D1Bitmap`s: **album art** is re-created only when `Model::art_gen` changes (`sync_art`); **shelf thumbnails** are kept in a map by item id (`missing_thumbs` lists visible tiles without one and drops removed ones; `Ui::request_thumbs` fetches each missing one once on a short-lived thread). Both paint through a bitmap brush into a rounded rectangle.

Determinate progress glides to the reported percentage (a tween keyed by task id).

## Springs

`animation.rs` implements a damped harmonic spring with a **closed-form** solution (under-, critically and over-damped), so one big step lands where many small ones would and motion is frame-rate independent (test `step_size_does_not_change_the_motion`). A `Spring` has `pos`, `vel`, `target`, damping ratio `zeta`, `period` and settle threshold `eps`; once position and velocity are within `eps` it snaps to target and reports "not moving", which lets the frame clock stop. A global `SLOW = 1.18` stretches every period slightly.

Every spring takes its feel from `animation::motion`:

| Feel | Zeta / period | Used for |
|---|---|---|
| `GROW` | 0.86 / 0.30 s | The pill growing: a touch of overshoot that settles. |
| `SHRINK` | 1.0 / 0.40 s | The pill shrinking: slower, no bounce. |
| `ENTER` / `EXIT` | 1.0 / 0.21 s / 0.17 s | A scene, a row or a tab arriving / leaving. |
| `QUICK` | 1.0 / 0.16 s | Hover, press, toggles, a digit rolling. |
| `FADE` | 1.0 / 0.22 s | The whole pill fading. |
| `GLIDE` | 1.0 / 0.30 s | Values, rows sliding, scrolling, the glow. |
| `DIM` | 1.0 / 0.45 s | The idle pill dimming. |
| `POP` | 0.72 / 0.34 s | The bubble, a released press, the to-do tick. |

`RectSpring::set_target` picks `GROW` or `SHRINK` **when the target changes** (by comparing areas), not every frame; otherwise an expansion's overshoot would read as a collapse. Retargeting keeps velocity, so leaving mid-expansion turns around smoothly.

**Reduced motion.** With Windows "Animation effects" off (`SPI_GETCLIENTAREAANIMATION`), `Anim::reduce` makes every spring snap, scene changes cut (leaving layers are dropped), tweens return their targets, nothing staggers, rolls or arrives, and ambient animation stops. Re-read on `WM_SETTINGCHANGE`.

**Reviewing motion.** `HYTTE_SNAP_DIR=<dir> cargo test filmstrips -- --ignored` drives scripted transitions (`render/film.rs`: opening and closing a task, the media morph, paging, quick reversals, a press, rows changing, scrolling, a card) with a fixed 120 Hz step and writes every 60 Hz frame plus `motion.html`, which plays them at full speed, a quarter, or a frame at a time; an older run in `<dir>/before` shows beside each strip. Frames are drawn at script time, so a strip is the same pictures on every run. `every_scripted_transition_is_continuous` runs the same scripts without drawing and fails if any scene's opacity steps more than 0.12, or its offset more than 6 px, between two 120 Hz frames.

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
| An eye-break card shows (20 s) | The countdown ring drains. |
| Media plays **and** a media scene is shown | Equaliser and timeline. |
| The pointer rests on a cut-off label (`wants_frames`) | It scrolls; stops when the pointer leaves. |

The bubble spring moves for a fraction of a second then rests; the timer bubble's clock is redrawn by the existing 1 Hz timer, not an animation. Never in the `Sentinel` scene, while hidden, or with reduced motion.

When you add motion, make it a function of `self.t` or `Frame::now`, decide when it may stop, and add that condition to `Model::ambient`. **An animation with no stop condition runs the CPU forever.**

Next: [9. Background workers](09-workers.md)
