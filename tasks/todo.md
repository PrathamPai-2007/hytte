# Hytte 2.0 tasks

Decisions, budgets, risks and open questions are in [plan.md](plan.md). "Checks" = `cargo fmt`,
`cargo clippy -- -D warnings`, `cargo test`. "Filmstrip" = the motion page from Task 2,
regenerated. Branch `v2.0`. Commits carry no trailers. Nothing is pushed or tagged without the
user's go-ahead. Live runs use `target\release\hytte.exe` with an isolated `%APPDATA%`; the
Desktop copy is never started.

Every task also has to pass: Checks, no new source of idle wake-ups, and reduced motion still
snapping everything.

## Phase 0: Ask

### Task 0: Start Ollama button
When the Ask pane finds Ollama isn't running, offer to start it.
- [x] "Ollama isn't running." shows **Start Ollama** (primary) beside **Try again**; when Ollama
      isn't installed the button reads **Get Ollama** and opens ollama.com
- [x] The click starts Ollama's tray app, or `ollama serve` with no console window when only the
      command is installed; the pane shows "Starting Ollama…", looks every 0.5 s for up to 30 s,
      then continues as a normal probe, or says "Ollama didn't start." with **Try again**
- [x] Hytte never stops Ollama, starts it without a click, or keeps a timer running after the wait
**Verify:** Checks; unit tests for "which program to start" (tray app found, command only,
neither) and for the wait ending on success and on timeout against the test server in
`ollama.rs`; snapshots of the three new states; a live build with Ollama stopped, running and
uninstalled; install path confirmed against Ollama's Windows docs.
**Depends on:** none. **Files:** `ollama.rs`, `ui_state/ask.rs`, `render/ask.rs`,
`window/win/ask.rs`, `render.rs` (`Action`), `render/snap.rs`, `docs/`. **Scope:** M

## Phase 1: Foundation

### Task 1: Baseline numbers
Record where 1.9.0 stands so later phases can be compared. Add the refresh rate and whether the
frame was spring-paced or ambient to the `HYTTE_PERF` line.
- [x] Perf lines for expand / collapse, a running task (collapsed and open), and playing media, at
      the user's refresh rate, saved in `tasks/baseline.md`
- [x] Idle CPU and private bytes after 45 s, measured as `docs/12-performance.md` describes
- [x] The perf line shows `hz=`, `cpu_us=` and separate lines for spring and ambient frames
      (playing media was not measured: no player was driven; the display was at 60 Hz, not 144)
**Verify:** Checks; the log lines exist and parse.
**Depends on:** none. **Files:** `perf.rs`, `window/win/frame.rs`, `tasks/baseline.md`. **Scope:** S

### Task 1b: Cache the steady background (added: the baseline showed it is most of a frame)
- [x] Glow and body are painted into two bitmaps once they have been the same for two frames,
      and copied until something they depend on changes; freed after 30 s of stillness, on a
      device rebuild and on a scale change
- [x] A test draws the same frame painted and copied at 1x, 1.5x and 2x: within 2 levels of 255
- [x] CPU with a running task: 14.6 % to 5.6 % collapsed, 27.1 % to 11.6 % open
**Files:** `render/pill.rs`, `render.rs`, `window/win/wndproc.rs`, `render/snap.rs`. **Scope:** M

### Task 2: Motion review (filmstrips, slow motion, continuity test)
- [x] `HYTTE_SNAP_DIR=<dir> cargo test filmstrips -- --ignored` writes frames for at least:
      idle → compact task → tasks pane → idle, compact media ↔ media pane, paging two panes,
      A→B→A and A→B→C inside 80 ms, and a card arriving
- [x] `motion.html` beside them plays each strip at full, quarter and single-step speed, with a
      "before" strip next to the "after" one when a `before/` folder exists
- [x] `HYTTE_SLOWMO=4` makes a live build move at quarter speed
- [x] A normal unit test runs the same scripts and reports the largest opacity and position step.
      It is expected to fail on A→B→A today; it is marked `#[ignore]` with that reason until Task 7
**Verify:** Checks; open `motion.html` and confirm the reversal pop is visible in the strip.
**Depends on:** none. **Files:** `render/snap.rs` (or a new `render/film.rs`), `window/win/frame.rs`,
`ui_state/tests/scene.rs`. **Scope:** M

### Task 3: Motion tokens
- [x] `animation::motion` holds `GROW`, `SHRINK`, `ENTER`, `EXIT`, `QUICK`, `FADE`, `GLIDE`, `POP`
- [x] No `(zeta, period)` literal remains in `ui_state/anim.rs` or `render/`
- [x] Motion is unchanged: every token has the value of the literal it replaced (filmstrips are
      now drawn at script time, so a strip is byte-identical from run to run)
**Verify:** Checks; filmstrip diff is empty.
**Depends on:** 2. **Files:** `animation.rs`, `ui_state/anim.rs`. **Scope:** S

### Task 4: One easing mechanism (`Tweens`)
A keyed spring store in the renderer replaces `ease` / `fx_vals`, the `shown` progress map, the
idle dim's exponential and the settings scroll's exponential.
- [x] `tw(key, target, feel)`, `tw_px` and `kick(key, velocity)`; `u64` keys; no allocation in steady state
- [x] First sight of a key starts at its target; a key not asked for in a frame is dropped
- [x] `Renderer::tweening()` feeds the frame loop's `moving`; tests show it goes false within a
      second of the last change, and that reduced motion returns the target at once. The idle dim
      is a spring (`motion::DIM`); the settings scroll moves to a spring in Task 16
**Verify:** Checks; settings window snapshots unchanged; filmstrip shows the idle dim and task
progress still easing; idle CPU measured as in Task 1 is unchanged.
**Depends on:** 3. **Files:** `render.rs`, `render/settings.rs`, `render/compact.rs`,
`settings_win.rs`, `window/win/frame.rs`. **Scope:** M

### Task 5: Vblank-aligned ambient frames
- [x] `Shared` has `stride` (vblanks per frame) in place of `fast`; the ticker never sleeps 33 ms
- [x] Ambient frames: the divisor of the refresh rate nearest 66 Hz while the pill is open or
      hovered (60 / 60 / 72 / 60 fps on 60 / 120 / 144 / 240 Hz); nearest 30 Hz while collapsed.
      The divisor is a pure function with a test for those four rates
- [x] `dt` for ambient frames comes from the DWM clock, as it already does for springs
- [x] CPU measured for a running task collapsed and open, at the old and new pacing, in
      `tasks/baseline.md` (media playing and collapsed-at-60 were not measured)
**Verify:** Checks; perf line shows `gap_us` p50 equal to a whole number of refresh periods for
ambient frames; idle still parks (no frames, no wake-ups).
**Depends on:** 1. **Files:** `window/win.rs`, `window/win/frame.rs`, `docs/12-performance.md`.
**Scope:** M

### Task 6: Spike: shadow and glow from a cached mask
Time-boxed. Build a blurred alpha mask once and draw it as stretched slices with
`FillOpacityMask` in the offscreen renderer and one live build. No production wiring.
- [x] Draw cost of the mask against the five strokes, on WARP, at 100 % and 200 % scaling
- [x] Pictures: not made; the cost alone decided it (see the verdict)
- [x] A written verdict in `tasks/baseline.md`: neither (strokes for the glow and the shadow)
**Verify:** numbers and pictures exist; the user picks at Checkpoint 1.
**Depends on:** 1. **Files:** a throwaway branch or `#[cfg(test)]` code only. **Scope:** S

### Checkpoint 1: Foundation
The user asked (2026-11-15) for every phase to be completed in one go, so checkpoints are not
stops: what each one would have shown is kept for review at the end.
- [x] Checks pass; idle CPU and memory match the baseline (0.65 % and 13.4 MB)
- [ ] The user has seen `motion.html` and the baseline (kept for the end)
- [x] Ambient rates chosen from measurements: about 60 fps open, 30 collapsed (one constant each)
- [x] Mask verdict: neither (see `tasks/baseline.md`)
- [ ] Assumptions in plan.md confirmed or corrected (version still undecided by the user)

## Phase 2: Transitions

### Task 7: Scene layers (reversals don't pop)
- [x] `Anim` holds up to three layers; returning to a scene that is still leaving reuses its
      layer with opacity and velocity intact; a fourth evicts the faintest
- [x] `content()`, `out()` and `prev` stay readable, so the tab strip and `morph_for`
      draw as before for a simple A→B (filmstrips: the same pictures, a few frames off by the
      old scene now fading to its end instead of being cut at 1 %)
- [x] The continuity test from Task 2 is un-ignored and passes for A→B→A, A→B→C and a reversal
      during a paged slide (largest steps now 0.10 opacity, 0.6 px; were 0.25 and 8.8 px); only
      the top layer registers hits
**Verify:** Checks; filmstrip: simple transitions identical to before, reversals smooth.
**Depends on:** 2, 3. **Files:** `ui_state/anim.rs`, `render/pill.rs`, `render/shelf.rs` (tab
strip), `ui_state/tests/scene.rs`. **Scope:** M

### Task 8a: Staggered entrances
- [x] `self.group(i)` sets opacity and offset for what is drawn next, from the layer's own
      spring; no new springs
- [x] The panes with rows or tiles use it (Home, Media, Tasks, Agents, Notes, To-do, Shelf,
      Weather, Timer, Power): header, then rows (capped at five steps). Cards, the drop zone and
      Ask come in as one block. The tab strip and shared elements are exempt
- [x] The time from scene change to fully drawn is unchanged (every group arrives when the
      layer does); the largest opacity step stays 0.10
**Verify:** Checks; filmstrip; snapshots of settled scenes unchanged (stagger ends at identity).
**Depends on:** 7. **Files:** `render/pill.rs`, then one commit per two or three scene files in
`render/`. **Scope:** L, split by scene file

### Task 8b: Shared elements, extended
- [ ] `morph_for` is a pair table; progress falls back to the entering layer when the two scenes
      are the same height (not needed: every pair differs in height; left as is)
- [x] Media: the equaliser travels with the art (the title is a different text and font in
      the two scenes, so it cross-fades). Tasks: the right-hand value travels with the icon and
      label (the progress line cross-fades: its row position depends on optional lines)
- [ ] Timer: the bubble's clock travels to the timer pane's clock. Not built: the bubble is not a
      scene, so it needs its own pairing; left for the user to ask for
**Verify:** Checks; filmstrip shows one copy of each shared element at every frame (the
continuity test covers their positions).
**Depends on:** 7. **Files:** `render/pill.rs`, `render/compact.rs`, `render/media.rs`,
`render/tasks.rs`, `render/timer.rs`. **Scope:** M

### Task 9: Glow colour blends
- [x] The glow colour is eased with the same feel as its strength
- [x] When the glow is invisible the colour snaps (no blend from a colour nobody saw)
- [x] Unit test: amber → red passes through intermediate values and never leaves 0..1
**Verify:** Checks; filmstrip of running → failed → done.
**Depends on:** 3. **Files:** `ui_state/anim.rs`, `render/pill.rs`. **Scope:** S

### Checkpoint 2: Transitions
- [x] Checks and the continuity test pass
- [ ] The user has reviewed the filmstrip and a live build (kept for the end)
- [ ] Timer shared-element question (kept for the end; not built)

## Phase 3: Controls and values

### Task 10: Press and hover physics
- [x] `press`, `button`, `primary`, `round_btn`, `tile` and the play button ease hover and press
      through `Tweens` (`QUICK` in, `POP` on release)
- [x] The press scales the whole control about its centre (label and icon included), not just
      its box
- [x] A click shorter than a frame still shows: the pressed look is held for at least 80 ms
      (the filmstrips have a `press` script with a pointer)
**Verify:** Checks; unit test for the 80 ms hold; filmstrip of hover → press → release; live
build.
**Depends on:** 4. **Files:** `render/prims.rs`, `window/win/input.rs`, `render.rs`. **Scope:** M

### Task 11: Toggles, selection and tabs
- [x] Tile on/off tint (fading out in its own colour), the to-do tick (the box fills and the tick
      draws itself; the strike-through draws across), timer presets; settings switches and
      segments already eased and now use the same springs
- [x] The tab strip's highlight is driven by `Tweens` (each tab eased on its own, so a page
      turned back halfway turns back). Hover preview not built: it would change how the strip
      behaves, which this release doesn't
- [x] Nothing eases on first draw of a pane (a new key starts at its target)
**Verify:** Checks; settled snapshots unchanged; filmstrip.
**Depends on:** 10. **Files:** `render/prims.rs`, `render/home.rs`, `render/jot.rs`,
`render/shelf.rs`, `render/settings.rs`. **Scope:** M

### Task 12: Progress and play/pause
- [x] Task progress, the media timeline and the volume bar glide (`GLIDE`); a drag follows the
      pointer exactly. The timer ring and the battery are not eased: they change once a second
      or less, by an invisible step, and easing them would add a burst of frames to every tick
- [x] Play and pause are one shape that morphs between the two (`play-pause.png` from
      `play_pause_morph_picture`)
- [x] A new track or task starts at its value instead of sweeping from the last one (keys carry
      the task id and the song)
**Verify:** Checks; filmstrip; live build with media.
**Depends on:** 4. **Files:** `render/prims.rs`, `render/media.rs`, `render/compact.rs`,
`render/timer.rs`, `render/home.rs`. **Scope:** M

### Task 13: Tabular, rolling digits
- [x] `digits(text, …, roll)` draws numbers in fixed-width cells; a changed digit rolls (old one
      out, new one in; up when it grows, down when it shrinks) over one `QUICK`
- [x] Rolling: the timer clock and minutes, the media position, the battery percent, the
      temperature, a task's value in the list. Tabular without the roll: the bubble clock, the
      compact task's value, the song length
- [x] A clock that ticks once a second asks for frames only during the roll; nothing on a
      collapsed pill rolls, so it gains no frames
**Verify:** Checks; snapshots of every numeric label line up with the old ones to within 1 px;
wake-up count for a running timer equals the baseline plus the roll.
**Depends on:** 4. **Files:** `render/prims.rs`, `render/timer.rs`, `render/media.rs`,
`render/weather.rs`, `render/tasks.rs`. **Scope:** M

### Task 14: Rows slide when a list changes
- [x] Task, agent, to-do and note rows are positioned through `Tweens`, keyed by item id: a
      reorder or a removal above slides the rest into place. Clips and shelf tiles page by
      whole rows and are left to Task 17
- [x] A new row fades and slides in; rows present when the pane opens don't (filmstrip `rows`)
**Verify:** Checks; filmstrip of a task finishing and being dismissed; settled snapshots
unchanged.
**Depends on:** 4. **Files:** `render/tasks.rs`, `render/agents.rs`, `render/jot.rs`,
`render/shelf.rs`. **Scope:** M

### Task 15: Rows leave instead of vanishing
- [x] A deleted note or removed to-do fades out where it was, drifting left, while the rows
      below slide up; it takes no clicks. Done in the renderer (it keeps last frame's rows), so
      the model and the saved file never hold it. Dismissed tasks and shelf items still vanish:
      their rows are drawn inline, not by a function a leaving copy could reuse
- [x] Nothing changes in the model: no timer, no sweep
- [x] Saving is untouched (the model removes the item at once, as before)
**Verify:** Checks; filmstrip; live build.
**Depends on:** 14. **Files:** `ui_state/tasks.rs`, `ui_state/notes.rs`, `ui_state/shelf.rs`,
matching `render/` files, `ui_state/tests/`. **Scope:** M

### Checkpoint 3: Controls and values
- [x] Checks pass (perf measured together at Task 24)
- [ ] The user has tried a live build (kept for the end)

## Phase 4: Scrolling

### Task 16: Smooth scrolling for the answer boxes and the settings window
- [x] The model keeps the scroll target; the drawn offset follows through `Tweens` (`GLIDE`); the
      settings window's scroll is a spring
- [x] Scrolling past an end gives a small bounce (`kick`) and settles at the end
- [x] "Follow the end" while an answer streams still works (the model's scroll logic is
      unchanged; existing tests pass)
**Verify:** Checks; existing Ask scroll tests pass; live build with a mouse wheel and a touchpad.
**Depends on:** 4. **Files:** `render/ask.rs`, `window/win/ask.rs`, `settings_win.rs`,
`render/settings.rs`. **Scope:** M

### Task 17: Smooth scrolling for row lists
- [x] Notes and to-dos glide between positions instead of jumping a row (one row each side is
      drawn out of sight, clipped, taking no pointer); the lyrics already glided. Tasks, clips and
      shelf tiles still step: their windows come from the model and would need it to hand out one
      row more each side
- [ ] The same end bounce as Task 16. Not built: at a list's end the wheel goes on to page the
      panes, and a bounce there would fight that
- [x] Rows entering and leaving the window are clipped, not popped
**Verify:** Checks; existing scroll tests pass; filmstrip; live build.
**Depends on:** 14, 16. **Files:** `render/tasks.rs`, `render/jot.rs`, `render/media.rs`,
`render/shelf.rs`, `window/win/controls.rs`. **Scope:** M

### Checkpoint 4: Scrolling
- [x] Checks pass; the live build is kept for the end

## Phase 5: Look

### Task 18: Look tokens
- [x] `render/tokens.rs` holds the nested radii (pill, card, row), three text opacities and the
      pane gap; tiles, row highlights and the weather credit use them. Other literals were left
      where they already sat on the grid
- [x] Every pane leaves 12 px between its last element and the tab strip (Home was 18, Media 8,
      the running timer 23, the timer setup 14)
- [x] Before / after pictures for every scene (made at Checkpoint 5 into one page)
**Verify:** Checks; the snapshot page; no scene moves by more than 4 px except where listed in
the commit message.
**Depends on:** Checkpoint 3. **Files:** `render/tokens.rs`, then scene files in `render/`, two or
three per commit. **Scope:** L, split by scene file

### Task 19: Type scale
- [x] Five sizes (caption 10.5, body 12, title 13, heading 15, display 27); captions use Segoe UI
      Variable's Small cut; the 12.5 px format is gone (lyrics went from 14 to 15)
- [x] Credit text uses the tertiary opacity (the Open-Meteo credit no longer competes with
      content); secondary text keeps its grey
- [x] No label is cut off that wasn't before (snapshots compared)
**Verify:** Checks; before / after snapshot page.
**Depends on:** 18. **Files:** `render.rs` (`Fmts`), scene files that name a removed format.
**Scope:** M

### Task 20: Continuous-curvature corners
- [x] The silhouette's bottom corners and every large rounded shape drawn through `fill_rr` /
      `stroke_rr` (tiles, cards, the album art placeholder, the text fields) use the new corner,
      from a geometry cache (capped at 96). Full pills, circles and radii under 12 stay native
- [x] Inner radii follow the nesting rule from Task 18 (`R_CARD`, `R_ROW` from `inner`)
- [ ] Perf while the pill resizes is within budget (measured at Task 24)
**Verify:** Checks; before / after snapshot page at 100 % and 200 %; perf line.
**Depends on:** 18. **Files:** `render/pill.rs`, `render/prims.rs`, `render.rs`. **Scope:** M

### Task 21: Shadow and glow
Built to the verdict from Task 6.
- [ ] A soft shadow under the open or hovered pill, fading with the expansion; none under an
      idle or compact pill; the click region is no larger than today's
- [ ] If the mask won: the glow uses it and the five strokes are deleted
- [ ] Correct at `pill_opacity` 0.4, in light and dark, and in every skin
**Verify:** Checks; snapshot page; perf line no worse than Task 20's; memory within budget.
**Depends on:** 6, 20. **Files:** `render/pill.rs`, `render.rs`, `skin.rs`. **Scope:** M

### Task 22: One set of components
- [ ] One slider (track, fill, thumb, hover growth) for volume, the timeline, the timer and
      settings
- [ ] Fluent glyphs and the drawn transport shapes agree in visual weight and size
- [ ] Buttons share two heights and one set of opacities for rest, hover and press
**Verify:** Checks; before / after snapshot page.
**Depends on:** 19, 20. **Files:** `render/prims.rs`, `render/home.rs`, `render/media.rs`,
`render/timer.rs`, `render/settings.rs`. **Scope:** M

### Task 23: Settings window
- [ ] The window uses the same tokens, corners, slider and switch as the pill
- [ ] Layout and wording unchanged
**Verify:** Checks; `settings_window_snapshots` before / after.
**Depends on:** 22. **Files:** `render/settings.rs`, `render/settings_table.rs`, `render.rs`.
**Scope:** S

### Checkpoint 5: Look
- [ ] Checks pass; perf and memory within budget
- [ ] The user has approved the before / after page

## Phase 6: Release

### Task 24: Audit
- [ ] Reduced motion: every tween, layer, stagger, roll and bounce snaps; a test covers it
- [ ] Calm and snappy scale everything, tweens included
- [ ] Final numbers against every row of the budget table, at 60 Hz and at the user's rate, in
      `tasks/baseline.md`
**Verify:** Checks; the numbers.
**Depends on:** Checkpoint 5. **Files:** tests, `tasks/baseline.md`. **Scope:** S

### Task 25: Docs and pictures
The user asked (2026-11-15) for everything to be noted and refreshed in both the developer docs
(`docs/`) and the user manual (`manual/`). Each task updates what it changes; this task is the
full read-through of both.
- [ ] `docs/06`, `08` and `12` describe layers, tweens, pacing, tokens and the mask
- [ ] Every page of `manual/` read against the build: wording, pictures, anything that moved
- [ ] `docs/16-whats-new.md` and the welcome tour's 2.0 entry
- [ ] `docs/img` and the landing page frames regenerated
**Verify:** docs build as before; every picture referenced exists.
**Depends on:** 24. **Files:** `docs/`, `landing-page/public/frames`, `welcome.rs`. **Scope:** M

### Task 26: Version 2.0.0
- [ ] `version.ps1` bump and `version.ps1 check` pass
- [ ] `tasks/baseline.md` deleted (its final numbers live in `docs/12`)
- [ ] The user's go-ahead before any push or tag
**Verify:** Checks; a release build starts, expands, pages and collapses.
**Depends on:** 25. **Files:** version files. **Scope:** XS
