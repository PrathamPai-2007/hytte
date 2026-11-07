# Implementation Plan: Hytte 1.7 panes

## Overview

Add Notes, To-do and Weather panes, make Focus mode actually quiet, add break reminders, lyrics,
a sleep timer and colour skins. The pill has never accepted typing, so text input comes first:
three of these features depend on it. Task details are in [panes-todo.md](panes-todo.md).
Mockups: [Hytte pane concepts](https://claude.ai/artifact/6eeTT27W6AGA9hQPFzAY6U).

## Decisions

| Topic | Decision |
|---|---|
| Notes and To-do | Two panes, sharing one text field and one row design |
| Note lifetime | A `24 h` / `Keep` switch in the field. New notes default to 24 h and show a small ring with the time left. One click keeps a note for good |
| Quick capture | `[general] note_hotkey`, empty until the user picks a chord |
| New tabs | Notes, To-do and Weather show from day one; each has a switch in Settings |
| Weather | Asks for a place and suggests matches as you type. Pane shows now plus the next six hours, with a soft tint that follows the sky |
| Focus: Windows | Turns on Do Not Disturb through the undocumented Windows switch, then restores it. Falls back to muting only Hytte |
| Focus: Hytte | Cards, peeks and sounds wait, then arrive as one summary. Nothing else is hidden: the pill keeps showing music and tasks |
| Focus line | Quiet hairline: thin, pale, no spark, colour changes only in the last minute |
| Break reminders | Small card (like the plug-in card), no sound, closes itself |
| Lyrics | Separate lyrics page reached from the Media pane. Off until switched on |
| Sleep timer | Moon button on the Media pane: 15, 30, 45, 60 minutes or end of track |
| Fun | Five colour skins. **No pet** |
| Screenshots | `hide_from_capture` defaults to `false`, so the pill appears in screenshots |

## Assumptions (say so if any are wrong)

| Topic | Assumption |
|---|---|
| Existing installs | `config.toml` is written with every default, so current users keep `hide_from_capture = true`. Only new installs change. Migrating would silently expose agent prompts and notes in calls |
| Focus exceptions | Agent permission asks, the timer ending and calendar heads-ups get through. Downloads, resource hogs and task finishes wait |
| Focus trigger | Focus mode means a Focus timer is running, including while paused. Plain timers and breaks stay as they are, with today's fuse |
| Summary | Held alerts become counts only ("2 downloads, 1 task failed"), shown on the existing timer-finished card. A session stopped early shows them on a normal card |
| Break defaults | Off until switched on, each type separately. Eyes every 20 min, stretch every 60, water every 90 |
| Break rules | Wait during Focus, quiet hours, fullscreen, games and camera or mic use. Five minutes away from the PC resets the clocks. Two nudges never come back to back |
| Done to-dos | Sink to the bottom and leave 24 h after being ticked. Deleting anything offers Undo |
| Notes format | One line each, up to 2,000 characters, editable by clicking |
| Weather units | From the Windows measurement setting; `[weather] units` overrides |
| Sleep timer | Lost on restart. A manual pause doesn't cancel it. No sleeping the PC |
| Release | One release, 1.7.0, built on branch `panes`. Nothing is pushed or tagged without asking (signing hold) |

## Architecture

**Typing.** The pill keeps `WS_EX_NOACTIVATE` and never takes focus by itself. Clicking a field or
pressing `note_hotkey` calls `SetForegroundWindow`, as keyboard mode already does, and Esc or a
click elsewhere gives focus back. A native edit control can't draw into the DirectComposition
surface, so the field is drawn by Hytte: `textedit.rs` holds the text, caret and selection as pure
tested logic, and a `text_field` primitive draws it with `IDWriteTextLayout`.

**One store.** Notes and to-dos are the same record with a `kind`, in one module (`jot.rs`) and one
file (`jot.json`). Expiry uses wall-clock milliseconds like `timer.rs` and feeds `Model::expire`,
so it adds no timer.

**Network.** `update::curl` moves to `net.rs` and is reused: still no HTTP library in the binary.
Weather is fetched only when the pane opens and the data is over 30 minutes old, so a closed pill
makes no requests. Lyrics are fetched only with the page open and are never written to disk.

**One quiet gate.** `Model::held()` is true during Focus. Background alerts pass through one
function that counts them instead of showing them; cards the user caused still show. Break nudges
ask the same gate.

**Zero idle frames.** Skins are static brushes. The hairline redraws only when it moves a whole
pixel (about every 12 s in a 25 minute session, less often than today's 1 Hz fuse while collapsed).
The caret blinks on a one-shot timer only while a field has focus. The eye-break ring is the only
new `ambient` motion and stops within 20 s.

## UI bar

Every new surface must meet this before its checkpoint.

- **Shared parts:** 34 px fields, 30 px rows, 16 px side padding, the existing `cc` colours and fonts. No new font sizes.
- **Colour:** accent only for selection and primary actions; status colours keep their meaning.
- **States drawn:** empty, loading, error, offline, hover, pressed, keyboard focus ring, long text.
- **Checked in:** dark and light themes, 0.7 and 1.6 `pill_scale`, reduced motion, all five skins.
- **Words:** short and plain. Say what happened and what to do next.
- **Motion:** the same springs and paging slide as other panes. Nothing moves that doesn't need to.

## Task list

### Phase 1: Risky parts first
- [x] 1. Do Not Disturb test
- [x] 2. Text editing logic
- [x] 3. Typing focus
- [x] 4. Text field drawing
- [x] 5. New panes and Settings switches

**Checkpoint A:** type, paste and use an IME in a throwaway field on a dev build; focus returns correctly.

### Phase 2: Notes and To-do
- [x] 6. Saved notes and to-dos
- [x] 7. Notes pane
- [x] 8. To-do pane
- [x] 9. Note hotkey

**Checkpoint B:** snapshot page, user review.

### Phase 3: Weather
- [x] 10. Shared fetch helper
- [x] 11. Location search with suggestions
- [x] 12. Forecast
- [x] 13. Weather pane

**Checkpoint C:** snapshot page, user review.

### Phase 4: Focus and breaks
- [x] 14. Do Not Disturb during Focus
- [x] 15. Hold alerts and the summary
- [x] 16. Quiet hairline
- [x] 17. Break schedule
- [x] 18. Nudge card

**Checkpoint D:** a full focus session on a dev build, then snapshot page, user review.

### Phase 5: Media
- [x] 19. Sleep timer
- [x] 20. Sleep fade-out
- [x] 21. Finding lyrics
- [x] 22. Lyrics page

**Checkpoint E:** snapshot page, user review, tried with Spotify and a browser.

### Phase 6: Look and finish
- [x] 23. Colour skins
- [x] 24. Show the pill in screenshots
- [x] 25. Performance pass
- [x] 26. Docs and manual
- [x] 27. Version 1.7.0

**Checkpoint F:** full checks, idle budgets met, user sign-off. Publishing is a separate go-ahead.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| The Do Not Disturb switch doesn't work on this Windows build, or a later update breaks it | Medium | Task 1 tests it before anything depends on it. Read it back after every write and fall back to Hytte-only |
| Hytte crashes with Do Not Disturb on | Medium | Save the previous state with the timer and restore it at next start |
| IME composition misbehaves in a no-activate window | Medium | Tested at Checkpoint A, before any pane relies on it. Falls back to the default IME window |
| Taking focus breaks the "never steals focus" promise | High | Only from a click or hotkey. Test that Esc returns focus to the previous window |
| The strip now always shows, adding 24 px to Home for everyone | Low | Expected result of always-on tabs. Switching all three off restores today's Home |
| Open-Meteo or LRCLIB goes down or changes terms | Medium | The panes show a clear offline state and nothing else depends on them. Confirm both services' terms in tasks 11 and 21; I only found third-party summaries so far |
| Browsers report poor song data, so lyrics miss or drift | Medium | Clear "No lyrics found" state. Without a timeline, plain lyrics show unsynced |
| Hytte quits during a sleep fade and leaves the player quiet | Low | Fade lasts 20 s and only touches that app's volume; no fade if the player's exe is unknown |
| New users share prompts or notes in calls by accident | Medium | The setting stays one switch away; README, manual and landing FAQ say the new default plainly |

## Out of scope

A pet or idle animation, hiding the taskbar or desktop icons, line-only pill in Focus, background
rain alerts, temperature on the idle pill, to-do due dates and reordering, note sync, a `hytte note`
command, shuffle and repeat, sleeping the PC.

## Built, and where it differs from the plan

All 27 tasks are built and committed on `panes`. Differences:

- **Place storage:** the chosen place is in `place.json`, not `[weather]` in `config.toml` (the config writer only takes fixed strings); `[weather]` has only `units`.
- **Text field:** click places the caret; dragging to select is not built (Shift+arrows, Ctrl+A, double-use of the keyboard are). An IME composition window appears where Windows puts it, not beside the caret.
- **Nudge card:** Snooze and Skip are always visible (not on hover), and the card is 360 wide.
- **Skins:** chosen by name in Settings; there are no colour swatches.
- **Lyrics:** a near-miss recording shows the words untimed (3 s for timed, 10 s for words), found while testing a real song.
- **Do Not Disturb:** the write, read-back, restore and crash recovery are verified on this PC. That Windows then actually suppresses toast banners is not verified (the screenshot control failed); the user confirms by eye.
- **Not measured:** idle cost with the pill collapsed on a screen with a *different* DPI or refresh rate.
