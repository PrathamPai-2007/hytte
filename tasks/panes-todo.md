# Tasks: Hytte 1.7 panes

Plan and decisions: [panes-plan.md](panes-plan.md). Work on branch `panes`, cut from local `main`.

Every task also needs `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`. UI tasks add `shoot(...)` calls and meet the UI bar in the plan. Dev builds
run from `target\release` with an isolated `%APPDATA%`. Never start the Desktop `hytte.exe`.

Sizes: S is 1-2 files, M is 3-5 files.

## Phase 1: Risky parts first

### 1. Do Not Disturb test (S)
Prove the undocumented switch works here before anything depends on it. The user has agreed to one
test on this PC. If it fails, stop and ask before task 14.
- [ ] An ignored manual test sets Do Not Disturb on, reads it back, then restores the old value
- [ ] Windows Settings or Quick Settings shows the change, and a real toast stays hidden
- [ ] The result and the exact state name go in `dnd.rs`'s header comment
- Verify: `cargo test -p hytte dnd -- --ignored --nocapture`
- Files: `dnd.rs`, `main.rs`

### 2. Text editing logic (S)
- [ ] `TextEdit` handles insert, Backspace, Delete, arrows, Home, End, Ctrl+arrows, Shift selection and select all
- [ ] Caret moves by whole characters, so emoji and combining marks stay intact
- [ ] Pasted text loses line breaks and respects the length limit
- Verify: unit tests, no Win32
- Files: `textedit.rs`, `main.rs`

### 3. Typing focus (M)
- [ ] Clicking a field takes keyboard focus and holds the pill open, even with the pointer away
- [ ] Esc returns focus to the previous window; clicking elsewhere ends typing and keeps the draft
- [ ] Typing wins over keyboard-mode keys, so `1` to `9` type digits
- [ ] Ctrl+C, X, V and A work; an IME commits text and its window sits by the caret
- Verify: unit tests for the state rules; a dev build with a throwaway field
- Files: `window/win/input.rs`, `window/win/wndproc.rs`, `window/win/controls.rs`, `ui_state.rs`
- Depends on: 2

### 4. Text field drawing (M)
- [ ] `text_field` draws placeholder, text, selection and caret, and scrolls long text sideways
- [ ] Clicking and dragging place the caret and selection through `IDWriteTextLayout`
- [ ] The caret blinks on a one-shot timer and frames stop once focus is lost
- [ ] Right-to-left and mixed-script text stays readable
- Verify: snapshots; the `HYTTE_PERF=1` log shows no frames after focus is lost
- Files: `render/prims.rs`, `render.rs`, `window/win/wndproc.rs`, `render/snap.rs`
- Depends on: 3

### 5. New panes and Settings switches (M)
- [ ] `Panel::Notes`, `Todo` and `Weather` exist, in order Shelf, Media, Home, Notes, To-do, Weather, Tasks, Timer, Power
- [ ] `[panes] notes`, `todo` and `weather` default on and appear in Settings
- [ ] All nine tabs fit beside the pin at 0.7 and 1.6 scale, and `1` to `9` reach each one
- [ ] With all three off, Home looks as it does today
- Verify: extend `panels_and_selection` and `scene_priority`; snapshot of the full strip
- Files: `ui_state.rs`, `config.rs`, `render.rs`, `render/settings_table.rs`, `render/shelf.rs`

**Checkpoint A:** type, paste and use an IME on a dev build; focus returns correctly.

## Phase 2: Notes and To-do

### 6. Saved notes and to-dos (S)
- [ ] `jot.json` is written through a temp file and rename; a damaged file is kept aside, never overwritten
- [ ] Items survive a restart; expired ones leave at startup and at their deadline through `Model::expire`
- [ ] Limits hold: 200 items, 2,000 characters each
- Verify: unit tests with a temp folder, including expiry across a restart
- Files: `jot.rs`, `ui_state.rs`, `main.rs`

### 7. Notes pane (M)
- [ ] The field has a `24 h` / `Keep` switch, and Enter saves with the switch's value
- [ ] Temporary rows show a ring and time left; permanent rows show a note icon
- [ ] Hovering a row offers Copy, Keep (temporary only) and Delete; clicking the text edits it
- [ ] Delete offers Undo; four rows show and the wheel scrolls the rest; the empty state invites a first note
- Verify: `ui_state` tests; snapshots of empty, full, editing and long text
- Files: `render/jot.rs`, `render.rs`, `ui_state.rs`, `window/win/actions.rs`, `render/snap.rs`
- Depends on: 4, 5, 6

### 8. To-do pane (M)
- [ ] Enter adds; a click ticks with a short check animation and the row sinks to the bottom
- [ ] Ticked rows leave after 24 h; Delete offers Undo; editing works as in Notes
- Verify: `ui_state` tests; snapshots of empty, mixed and all done
- Files: `render/jot.rs`, `ui_state.rs`, `window/win/actions.rs`, `render/snap.rs`
- Depends on: 7

### 9. Note hotkey (S)
- [ ] `[general] note_hotkey` opens Notes with the cursor in the field and applies live
- [ ] Esc returns focus to the previous window; the hotkey is released in game mode
- Verify: `parse_hotkey` tests; dev build from inside another app
- Files: `config.rs`, `window/win/input.rs`, `window/win/wndproc.rs`, `render/settings_table.rs`
- Depends on: 7

**Checkpoint B:** snapshot page, user review.

## Phase 3: Weather

### 10. Shared fetch helper (S)
- [ ] `net::get` runs System32 `curl.exe`, HTTPS only, with a timeout and size limit per caller
- [ ] The updater uses it and behaves the same
- Verify: `update.rs` tests; end-to-end update from a local fake release
- Files: `net.rs`, `update.rs`, `main.rs`

### 11. Location search with suggestions (M)
- [ ] First open shows "Where should Hytte check the weather?" and a field
- [ ] After two characters and a 250 ms pause, up to four matches show with region and country
- [ ] Arrow keys, Enter and clicks pick one; a late reply never replaces a newer one
- [ ] No match, offline and service errors each say so in the pane
- [ ] Clicking the place name in the pane changes it; `place`, `lat` and `lon` are saved in `[weather]`
- [ ] Open-Meteo's terms are confirmed on its own site and credit is shown where they require it
- Verify: parser and encoding tests with odd names (`Tromsø`, `São Paulo`); dev build
- Files: `weather.rs`, `render/weather.rs`, `ui_state.rs`, `window/win/actions.rs`, `config.rs`
- Depends on: 4, 5, 10

### 12. Forecast (S)
- [ ] Opening the pane fetches only if data is over 30 minutes old, and nothing fetches while closed
- [ ] Reply parses to current, high and low, and six hours at the location's local time
- [ ] Weather codes map to seven icons with day and night forms; units follow Windows or `[weather] units`
- Verify: tests on a saved sample reply and every weather code; the wake-up table gains no row
- Files: `weather.rs`, `ui_state.rs`, `config.rs`
- Depends on: 11

### 13. Weather pane (M)
- [ ] Layout matches mockup A, with hand-drawn sharp icons and a cached sky tint
- [ ] Old data stays on screen with "Updated 14:05" when offline; loading shows no jump in size
- Verify: snapshots of each sky, day and night, offline and error, in both themes
- Files: `render/weather.rs`, `render.rs`, `ui_state.rs`, `render/snap.rs`
- Depends on: 12

**Checkpoint C:** snapshot page, user review.

## Phase 4: Focus and breaks

### 14. Do Not Disturb during Focus (M)
- [ ] A Focus timer turns it on and every way of ending restores the earlier value
- [ ] If the user already had it on, Hytte leaves it on
- [ ] The old value is saved in `timer.json` and restored at start if Hytte died mid-session
- [ ] A failed read-back logs once and Focus carries on, muting only Hytte
- [ ] `[timer] focus_dnd` (default on) switches this off
- Verify: unit tests with a fake switch; dev build, including killing the process mid-session
- Files: `dnd.rs`, `timer.rs`, `window/win/controls.rs`, `config.rs`, `render/settings_table.rs`
- Depends on: 1

### 15. Hold alerts and the summary (M)
- [ ] During Focus, downloads, hogs and finish peeks are counted, not shown; so are their sounds
- [ ] Agent asks, the timer ending and calendar heads-ups still show; cards the user caused still show
- [ ] The timer-finished card shows one line such as "2 downloads, 1 task failed"; stopping early shows a card
- Verify: `ui_state` tests for each alert type; snapshot of the finished card
- Files: `ui_state.rs`, `window/win/events.rs`, `render/timer.rs`, `render/snap.rs`
- Depends on: 14

### 16. Quiet hairline (S)
- [ ] During Focus the fuse becomes a thin, pale line with no head or glow, shifting colour only in the last minute
- [ ] While collapsed it redraws only on a whole-pixel step; plain timers keep today's fuse
- Verify: snapshots; `HYTTE_PERF=1` frame count over five minutes
- Files: `render/timer.rs`, `window/win/controls.rs`, `timer.rs`
- Depends on: None

### 17. Break schedule (S)
- [ ] Pure logic returns the next due nudge and its deadline through `Model::expire`
- [ ] Follows every break rule in the plan, and a nudge that waited appears once when the way is clear
- [ ] Sleep and clock changes don't cause a burst of nudges
- Verify: unit tests for each rule
- Files: `breaks.rs`, `ui_state.rs`, `config.rs`, `render/settings_table.rs`
- Depends on: 15

### 18. Nudge card (M)
- [ ] `ExpNudge` is 300 × 52: icon, line, Skip; hovering adds Snooze, which waits 10 minutes
- [ ] Closes after 10 s; eye breaks count down 20 s on a ring; no sound
- [ ] Never replaces a card or timer scene
- [ ] `ambient` turns false when the nudge ends
- Verify: scene and ambient tests; snapshots of all three nudges
- Files: `render/cards.rs`, `render.rs`, `ui_state.rs`, `render/snap.rs`
- Depends on: 17

**Checkpoint D:** a full focus session on a dev build, then snapshot page, user review.

## Phase 5: Media

### 19. Sleep timer (M)
- [ ] The moon opens chips for 15, 30, 45, 60 and End of track; clicking it again cancels
- [ ] The time left shows by the moon and the deadline runs through `Model::expire`
- [ ] `media::Cmd::Pause` pauses without ever starting playback
- [ ] End of track also fires when the title changes, for players with poor timelines
- Verify: `ui_state` tests; Spotify and a browser
- Files: `media.rs`, `ui_state.rs`, `render/media.rs`, `window/win/actions.rs`, `render/snap.rs`
- Depends on: None

### 20. Sleep fade-out (S)
- [ ] The player's volume fades over the last 20 s and returns after the pause
- [ ] An unknown player exe skips the fade and just pauses
- Verify: `mic::apps` check on a real player; volume is back afterwards
- Files: `window/win/controls.rs`, `mic.rs`
- Depends on: 19

### 21. Finding lyrics (M)
- [ ] `[media] lyrics` defaults off and its Settings note says the song title and artist go to LRCLIB
- [ ] `MediaInfo` carries the album; lookup tries an exact match, then search
- [ ] Timed lines parse, including several stamps on one line; instrumentals are recognised
- [ ] Results stay in memory for the last 20 songs and are never written to disk
- [ ] LRCLIB's rules are confirmed on its own site
- Verify: parser tests on invented sample files, no real lyrics in the repo; dev build
- Files: `lyrics.rs`, `net.rs`, `media.rs`, `ui_state.rs`, `config.rs`
- Depends on: 10

### 22. Lyrics page (M)
- [ ] With lyrics on, a button on the Media pane pages to three centred lines plus transport controls
- [ ] The current line follows the timeline and changes with a soft cross-fade
- [ ] Plain lyrics scroll with the wheel, and lookups show Searching, No lyrics found or Instrumental
- [ ] Long lines wrap to two lines; art colour is used only when adaptive glow is on
- Verify: snapshots of each state; drift check against a real player
- Files: `render/media.rs`, `ui_state.rs`, `window/win/actions.rs`, `render/snap.rs`
- Depends on: 21

**Checkpoint E:** snapshot page, user review, tried with Spotify and a browser.

## Phase 6: Look and finish

### 23. Colour skins (M)
- [ ] `[general] skin` offers `graphite`, `aurora`, `ember`, `moss` and `dusk`, and applies live
- [ ] Each changes body, rim and neutral glow in both themes; status colours stay readable
- [ ] Settings shows the choice as small swatches
- [ ] An idle pill with any skin draws no frames
- Verify: a snapshot sheet of every skin across every scene; `HYTTE_PERF=1`
- Files: `render/prims.rs`, `render/pill.rs`, `config.rs`, `render/settings_table.rs`, `render/snap.rs`
- Depends on: None

### 24. Show the pill in screenshots (S)
- [ ] `hide_from_capture` defaults to `false` for new installs; existing files stay as they are
- [ ] README, manual and Settings text match; landing FAQ change is listed for the site work
- Verify: config tests; a screenshot from a fresh `%APPDATA%` shows the pill
- Files: `config.rs`, `render/settings_table.rs`, `README.md`, `manual/02-the-pill.md`, `manual/07-settings.md`
- Depends on: None

### 25. Performance pass (S)
- [ ] With every feature on, idle CPU stays under 1 % and private bytes near 11 MB, under the 30 MB goal
- [ ] Collapsed and idle draws zero frames with a skin, stored notes and breaks enabled
- [ ] Frame cost with the full strip is within 10 % of the 1.6.0 numbers
- Verify: the measuring steps in `docs/12-performance.md`; update that table
- Files: `docs/12-performance.md`
- Depends on: all feature tasks

### 26. Docs and manual (M)
- [ ] Docs 06 to 12 and 16, the README and a new manual page cover every feature and setting
- [ ] Fresh screenshots replace any that changed
- Verify: read-through; every new config key appears in `docs/11`
- Files: `docs/`, `manual/`, `README.md`
- Depends on: all feature tasks

### 27. Version 1.7.0 (S)
- [ ] Version bumped through `scripts/version.ps1`; a full run from a clean data folder passes
- [ ] Before any push or tag, the signing check from the hold is done with the user
- Verify: `hytte --version`; user sign-off
- Files: `Cargo.toml`, `Cargo.lock`
- Depends on: 25, 26

**Checkpoint F:** full checks, idle budgets met, user sign-off. Publishing is a separate go-ahead.
