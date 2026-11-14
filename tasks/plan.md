# Implementation Plan: Hytte 2.0 (motion and look)

## Overview

2.0 changes how the pill moves and how it looks. It adds one feature (a **Start Ollama** button
in the Ask pane) and does not change navigation. Every state change gets motion that can be interrupted at any moment without a pop;
ambient animation runs on the display's own clock; the look is refined into one system (corner
shape, depth, type, spacing, components).

Tasks are in [todo.md](todo.md). The 1.8 and 1.9 plans are in git history.

## Scope (user, 2026-11-15)

In:

1. Choreographed scene changes (staggered elements).
2. Shared elements between scenes, extended beyond media and tasks.
3. Press and hover physics per control.
4. Value transitions: rolling digits, sprung progress, morphing play/pause, rows that slide.
5. Smooth scrolling with a bounce at the ends.
6. Glow colour blends instead of jumping.
7. Frame pacing measured and held to a budget at the user's refresh rate.
8. One motion token table, and one easing mechanism instead of four.
9. Continuous-curvature corners.
10. Depth: a soft shadow.
11. A tighter type scale with tabular numerals.
12. Ambient animation at the display's rate instead of a 33 ms sleep.
13. Fast scene reversals no longer pop.
14. A way to review motion (filmstrips, slow motion, a continuity test).
15. Shadow and glow from cached bitmaps (rendering is on the CPU).
16. A **Start Ollama** button in the Ask pane, shown when Ollama isn't running (added by the
    user after the first draft of this plan).

Out: finger-tracked paging, a full carousel slide between panes, any other new pane or feature, any
change to tab order or navigation, and everything on the rejected list (glance Home, live strip,
idle face, backdrops, a pet).

## Assumptions (not confirmed; correct me before Task 1)

1. **Version undecided** (user, 2026-11-15: "I will let you know later"). Working branch `v2.0`; no version bump until the user names it. Nothing is pushed or tagged without the user's go-ahead.
   Commits carry no trailers.
2. **The look is a refinement of the current dark glass**, not a new visual language.
3. **Navigation, pane order and pane sizes stay.** Spacing inside panes may move by a few pixels
   to fit a 4 px grid.
4. **The settings window is in scope** only as far as it shares the pill's tokens, easing,
   scrolling and components. No layout change there.
5. **No new config keys.** `[general] animation` (calm / normal / snappy) and Windows' "Animation
   effects" switch keep working as they do now.
6. **Weather's illustrated icons stay.** The icon work is about the Fluent glyphs and the drawn
   transport shapes agreeing in weight and size.

## Budgets

| Budget | Target |
|---|---|
| UI-thread frame time (`cost_us`) while anything moves | p99 under 40 % of a refresh period at the user's rate (2.8 ms at 144 Hz, 6.6 ms at 60 Hz) |
| CPU per frame (`cpu_us`, all threads) | steady frames no higher than after the background cache (3 ms collapsed, 5 ms open); transition frames no higher than 1.9.0 (10 to 23 ms) |
| Missed frames while a spring moves | at most 2 per 240 |
| Idle CPU, collapsed, nothing playing | unchanged: about 0.2 %, limit 1 % |
| Idle memory | unchanged: under 25 MB. Cached masks may add at most 1 MB |
| CPU with a running task on a collapsed pill | no higher than 1.9.0 measured the same way |
| CPU with ambient motion in an open pill | measured at Checkpoint 1; the user accepts the number or picks a lower rate |
| Continuity | no opacity step above 0.12 and no position step above 6 px between two 120 Hz frames in any scripted transition |

## Architecture decisions

### Motion foundation

- **Tokens.** `animation.rs` gets a `motion` module of named feels, each a `(zeta, period)`:
  `GROW`, `SHRINK`, `ENTER`, `EXIT`, `QUICK` (hover, press, toggles), `GLIDE` (values, scroll),
  `POP` (bubble, release overshoot). Every literal in `Anim::new` and the renderer points at one
  of them. `SLOW` and the speed setting keep scaling all of them together.
- **One keyed spring store, `Tweens`, in the renderer.** It replaces the settings window's
  `ease` map, the task progress map (`shown`), and the hand-written exponential eases for the
  idle dim and settings scroll. API: `tw(key, target, feel) -> f32`, `kick(key, velocity)`.
  Keys are `u64` (a tag hashed with an item id), so nothing is allocated per frame. A key seen
  for the first time starts at its target (nothing animates on first draw). A key not asked for
  during a frame is dropped. While any key moves the renderer reports `moving()`, which the
  frame loop treats like a spring (vblank pacing). Reduced motion makes `tw` return the target.
- **Why in the renderer:** hover, press and row positions are only known while drawing. The
  model stays free of presentation state.

### Scene transitions

- **Layers instead of `prev` / `out` / `content`.** `Anim` holds up to three `Layer`s (scene,
  opacity spring, slide direction). Changing scene retargets every layer: the new scene's layer
  goes to 1, the rest to 0. If the new scene is still on its way out, its existing layer is
  reused with its opacity and velocity intact, so A→B→A continues instead of restarting. A→B→C
  keeps A fading. A fourth layer evicts the faintest. Only the top layer takes hover and clicks.
- **Stagger is derived, not stored.** A scene splits its drawing into groups with
  `self.group(i)`. Group `i`'s opacity and offset come from the layer's own spring position,
  delayed by `i` steps. No new springs, so no new stop conditions. Leaving scenes exit as one
  block, faster than the entrance. Shared elements are exempt.
- **Shared elements generalised.** `morph_for` becomes a table of scene pairs. Progress comes
  from the pill's height spring when the two scenes differ in height, and from the entering
  layer's spring when they don't. `hero` is unchanged.

### Frame pacing

- **All pacing is vblank-aligned.** The ticker's 33 ms sleep is replaced by counting `DwmFlush`
  returns, so ambient frames land on refresh boundaries instead of beating against them.
  `Shared` gets one `pace` value in place of `animating` + `fast`:

  | Pace | Frames | When |
  |---|---|---|
  | Parked | none | settled |
  | Every vblank | refresh rate | a spring or a tween moves |
  | Every `round(hz / 66)`-th vblank | 60 / 60 / 72 / 60 (on 60 / 120 / 144 / 240 Hz) | ambient motion in an open or hovered pill |
  | Every `round(hz / 30)`-th vblank | 30 / 30 / 28.8 / 30 | ambient motion on a collapsed pill |

- **Why not the full refresh rate:** a frame costs 3 ms of CPU collapsed and 5 ms open even
  with the background cached. A build can run for an hour on a collapsed pill; 30 fps there is
  5.6 % of a core (1.9.0: 14.6 %). An open pill at 60 fps is about what 1.9.0 spent at 30.
  Aligning to vblank removes the uneven stepping either way. Both rates are one constant each.

### Frame cost (found while measuring the baseline)

- **The perf line's `cost_us` was a quarter of the truth.** It is what the UI thread waits; WARP
  rasterises on a pool of threads. The line now also reports `cpu_us`, the process's CPU time
  per frame, and budgets are in that number.
- **The steady background is cached.** Glow, body, tint, rim and hover are 60 to 75 % of a
  frame. Once they have been the same for two frames they are painted into two bitmaps (the
  glow, and the rest) and copied until something they depend on changes; the bitmaps are freed
  when the pill has been still for 30 s. A waiting agent's breathing glow is the glow bitmap
  drawn at a changing opacity, so that long wait is cached too; the tint inside the pill no
  longer breathes with it.
- **Ambient rates follow from the measured cost**, not from the 1 ms a frame the first draft of
  this plan assumed: about 60 fps (the nearest divisor of the refresh rate to 66 Hz) in an open
  or hovered pill, about 30 fps collapsed, both counted in vblanks.

### Look

- **Shape.** The silhouette's bottom corners and every large rounded shape (tiles, cards, album
  art, the bubble) use a continuous-curvature corner built from Béziers, cached by size. Small
  radii and full-pill buttons stay native rounded rectangles: at those sizes the difference
  can't be seen and native is cheaper.
- **Radius, spacing and type tokens** in one place (`render/tokens.rs`): radii nest (inner =
  outer − padding), spacing is a 4 px grid, type is five sizes with three text opacities.
- **Shadow and glow from an alpha mask.** A blurred mask is built once per scale and corner
  radius and drawn as stretched slices tinted by the brush (`FillOpacityMask`). The spike in
  Task 6 decides whether this replaces the five glow strokes; it must be cheaper and look at
  least as good, or the strokes stay and only the shadow uses the mask.
- **The shadow only shows where the glow margin already exists** (open or hovered pill). The
  window region is what takes clicks, and there is no hit-test pass-through, so a shadow under an
  idle pill would block 28 px of the app below it.
- **Digits are drawn in fixed-width cells** (the width of "0" in that format). That gives tabular
  numerals without a second text path, and is what rolling digits need anyway.

### Start Ollama

- **Where.** The Ask pane's "Ollama isn't running" state gets a **Start Ollama** button beside
  **Try again**. Only that state, and only when `[ai] host` is this PC (it always is: other hosts
  are already refused).
- **What it starts.** Ollama's own tray app if it is installed (so the user can quit it from the
  tray as usual); otherwise `ollama serve` from `PATH`, with no console window. The exact install
  path and executable name are checked against Ollama's Windows docs when the task is built, not
  taken from memory.
- **Not installed.** The button reads **Get Ollama** and opens ollama.com. Hytte still never
  installs anything.
- **After the click.** The pane says "Starting Ollama…" and a worker looks for it every half
  second for up to 30 s, then carries on as a normal probe (ready, or "get the model"). If it
  never answers: "Ollama didn't start." with **Try again**. The wait is a worker thread that
  ends, not a timer that keeps running.
- **Hytte never stops Ollama.** It belongs to the user; quitting Hytte leaves it running.
- **Order.** It doesn't depend on any motion work, so it is Task 0 and could ship on its own as
  1.9.1 if the user wants it sooner.

### Review

- **Filmstrips.** A new ignored test drives `Anim` and the renderer with a fixed time step
  through scripted transitions and writes the frames plus a `motion.html` that plays them at
  full, quarter and single-step speed. It runs before and after each phase; the page shows both.
- **`HYTTE_SLOWMO=<n>`** divides the time step in a live build.
- **Continuity test.** The same scripts run as a normal unit test that fails if any layer
  opacity or shared-element position steps more than the budget allows.

## Dependency graph

```
T0 Start Ollama (independent)
T1 baseline
T2 filmstrips ──► every later task is reviewed on it
T3 tokens ──► T4 tweens ──► T9 glow colour
                   │──► T10 press/hover ──► T11 toggles
                   │──► T12 progress, play/pause
                   │──► T13 digits
                   │──► T14 row slide ──► T15 row exits
                   │──► T16 pixel scroll ──► T17 row-list scroll
T5 pacing (independent of T3/T4)
T6 mask spike ──► T21 shadow and glow
T7 layers ──► T8 stagger and shared elements
T18 look tokens ──► T19 type ──► T20 squircle ──► T21 ──► T22 components ──► T23 settings window
T24 audit ──► T25 docs and pictures ──► T26 release
```

## Phases and checkpoints

| Phase | Tasks | The user reviews |
|---|---|---|
| 0. Ask | 0 | A live build: the button with Ollama stopped, running, and not installed |
| 1. Foundation | 1–6 | Baseline numbers, the filmstrip page, ambient rate and its CPU cost, the mask spike result |
| 2. Transitions | 7–9 | Filmstrips of every scene change, reversals included |
| 3. Controls and values | 10–15 | Filmstrips plus a live dev build |
| 4. Scrolling | 16–17 | A live dev build |
| 5. Look | 18–23 | Before / after snapshot page |
| 6. Release | 24–26 | Final numbers, docs, the go-ahead to tag |

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Full-rate ambient frames cost too much CPU | High | Measured in Task 5 against a budget the user sets at Checkpoint 1. The cap and the collapsed rate are one constant each |
| `FillOpacityMask` is slow or blurry when stretched on WARP | Med | Task 6 is a time-boxed spike with numbers. Fallback: keep the strokes, use the mask for the shadow only, or drop the shadow |
| A shadow shows through a translucent body (`pill_opacity` under 1) | Med | The mask is hollow under the body. Task 6 checks it at 40 % opacity while the pill resizes |
| The layer rewrite breaks the tab strip or the shared elements, which read `content`, `out` and `prev` | High | Task 7 keeps those three readable as accessors, changes no drawing, and lands with the continuity test |
| Stagger makes the pill feel slower | Med | Total entrance time is held to today's. Only the order changes. Judged on the filmstrip |
| Tweens keep the frame clock running forever | High | A key settles and is dropped; a test asserts `moving()` goes false within 2 s of the last change for every scripted transition |
| Squircle paths are slower than native rounded rectangles | Low | Only large shapes use them, cached by size. Perf is re-measured in Task 20 |
| Per-cell digits misplace text in right-aligned or centred formats | Low | Snapshot of every numeric label before and after in Task 13 |
| Snapshot pictures in `docs/img` and the landing page all change | Low | Regenerated once, in Task 25 |
| Starting Ollama launches the wrong program (something else named `ollama` on `PATH`) | Med | Prefer the tray app at Ollama's documented install path; only fall back to `PATH`; never pass user text to the command |
| GUI behaviour can't be judged from tests | Med | Each checkpoint includes a live dev build from `target\release` with an isolated `%APPDATA%`. The Desktop copy is never started |

## Open questions

- Start Ollama: ship it first as 1.9.1, or hold it for 2.0?
- Is 2.0.0 the right version, and may the settings window be included as described?
- Shared elements for the timer (bubble clock → timer pane, Home's Timer tile → timer pane):
  build both, or only the bubble? Decided on the filmstrip at Checkpoint 2.
- Collapsed ambient rate: 30 or 60? Decided on measured CPU at Checkpoint 1.
