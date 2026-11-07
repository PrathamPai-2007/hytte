# Implementation Plan: Hytte 2.0

## Overview

2.0 is everything already on local `main` (the panes, weather, notes, to-do, lyrics, skins, breaks,
unsigned updates) plus two headline features that make the pill a real notch rather than a set of
panes:

1. **Notifications in the pill.** Windows notifications (WhatsApp, Teams, Discord, Outlook…) show
   on the pill as a card with **Open** and **Dismiss**.
2. **Live cards.** Any script, CI job or app can put a card on the pill, update it in place, give it
   buttons, and optionally wait for which one was pressed.

Plus **winget** (`winget install Hytte.Hytte`) and a live test pass before anything ships.

**Release hold:** nothing after 1.6.1 is pushed or tagged on GitHub until the user lifts the hold
(every user must reach 1.6.1, the update that stops requiring signatures, first). All work below
stays on local `main`. `scripts/release.ps1` already refuses while main differs from origin/main.

Tasks are in [todo.md](todo.md).

## Architecture decisions

### Notifications

- **API:** `Windows.UI.Notifications.Management.UserNotificationListener` through the `windows`
  crate (features `UI_Notifications`, `UI_Notifications_Management`). Hytte isn't a packaged app.
  Community reports say an unpackaged app can request access and read notifications, but the
  `NotificationChanged` event fails with `0x80070490`. So Hytte reads, it doesn't subscribe.
  **This is unverified for us. Task 1 is a spike that settles it before anything else is built.**
- **When to read (zero idle frames still holds):** the preferred design reads only when Windows
  shows a toast, signalled by a WinEvent hook on the toast window appearing (`EVENT_OBJECT_SHOW`
  filtered to the shell's toast window). Hytte already runs WinEvent hooks (`window/win.rs`).
  Fallback if the spike finds no reliable signal: a worker thread calls `GetNotificationsAsync`
  every 2 s and diffs ids. That wakes a thread but draws no frames. A `// ponytail:` comment names
  the ceiling (up to 2 s late, a small periodic wakeup) and the upgrade path (the event, if
  Microsoft ever allows it for unpackaged apps).
- **Mirror, don't replace.** The listener can't hide only the corner banner (`RemoveNotification`
  also deletes it from the notification centre). Hytte shows its card, and the manual tells users
  who want Hytte *instead of* the banner to set that app to "notification centre only" in Windows
  Settings; Hytte still sees those notifications.
- **What a card shows:** app icon (from the notification's `AppInfo`), app name, title, first body
  line. **Open** launches the notification's app (toast arguments aren't available to other apps).
  **Dismiss** removes only Hytte's card. Several in a row stack as "3 new", newest on top.
- **Rules:** nothing in game mode, over a fullscreen app or in a meeting; during a Focus session
  notifications are held and counted in the existing "what you missed" line; Hytte's own toasts and
  a skip list never show; per-app mute (right-click a card → "Don't show <app>") writes
  `[notifications] muted`.
- **Privacy:** fresh installs ask once (Windows also shows its own permission prompt); existing
  installs stay off until switched on. `[notifications] show_text` turns body text off (app and
  title only). Body text respects `hide_from_capture` like everything else.

### Live cards

- **Built on what exists.** `hytte notify` (a `Cmd`) already shows a one-line card, and `Ask`
  already waits for a button on the same connection. A live card is a new `Cmd` kind `card` with
  optional fields, so old daemons ignore it and `old_messages_still_parse` keeps passing.
- **Fields:** `id` (update in place, 1–128 B), `title` (≤ 128 B), `body` (≤ 1024 B), `progress`
  (0–100), `icon` (fixed set: info, ok, warn, error, deploy, music), `buttons` (0–2 labels, ≤ 24 B
  each), `open` (URL or absolute path for a body click), `ttl` (seconds, default 10, max 3600; a card
  with `progress` lives until done or 30 s of silence, like `hytte set`), `done`, `clear`.
- **Buttons never run commands.** A press is only reported back: with `--wait` the CLI blocks (up
  to the card's ttl), prints the pressed label and exits 0; no press prints nothing and exits 1.
  Same pipe, owner-only DACL and server identity check as `Ask`.
- **CLI:**
  `hytte card --id deploy --title "Deploy" --body "Uploading…" --progress 40`
  `hytte card --id deploy --done --body "Live"`
  `hytte card --title "Restart server?" --button Restart --button Later --wait`
- Shows like a notify card, with progress and buttons. Queues behind Ask chips (permission prompts
  always win).

### winget

- Free. The first attempt (winget-pkgs PR 472228, 1.6.0) closed with **Needs-CLA** and
  **Validation-Defender-Error** (Defender blocked the unsigned exe in the install scan).
- Fix: sign Microsoft's CLA (a comment on the PR), submit `hytte.exe` to Microsoft's false-positive
  form, then resubmit in the multi-file manifest layout winget-pkgs requires (`version`,
  `installer`, `defaultLocale`), generated with `wingetcreate` or `komac`.
- **Submitting 1.6.1 to winget fits the hold** (it ships the release users should be on), but it's
  outward-facing, so the user decides when.
- Later releases: a release-workflow step opens the winget-pkgs PR (`komac update`, needs a token
  secret). Self-update still works for winget installs (the folder is user-writable); winget's own
  version record lags until the next winget PR merges.

## Task list

### Phase 0: Prove it works (fail fast)
- [ ] Task 1: Notification listener spike
- [ ] Task 2: Unblock winget (user-run steps)

### Checkpoint A
- [ ] Spike answers: access ok unpackaged? reading ok? which trigger (toast hook or 2 s poll)?
- [ ] User decides: go, or reshape notifications

### Phase 1: Live cards (no unknowns)
- [ ] Task 3: `card` command in the protocol
- [ ] Task 4: The card on the pill
- [ ] Task 5: `hytte card` CLI with `--wait`

### Checkpoint B
- [ ] Checks pass; snapshot of each card state reviewed by the user

### Phase 2: Notifications
- [ ] Task 6: Notification worker
- [ ] Task 7: Notification card and its rules
- [ ] Task 8: Permission and settings

### Checkpoint C
- [ ] Checks pass; snapshots reviewed; idle CPU and frame count unchanged with no notifications

### Phase 3: Ship-ready
- [ ] Task 9: Docs, manual, what's new, README images
- [ ] Task 10: Live test pass of everything on main
- [ ] Task 11: winget manifest for 2.0 and the release-workflow step

### Checkpoint D (release still on hold)
- [ ] User lifts the hold → compare 1.6.0 and 1.6.1 downloads → user decides → release

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| The listener refuses unpackaged apps on some Windows builds | High | Task 1 first; if it fails, notifications leave 2.0 and live cards carry it |
| No reliable "toast shown" signal | Med | 2 s poll on a worker thread, no frames; measure CPU in Task 6 |
| Defender keeps flagging the unsigned exe | Med | False-positive submission per release; winget is optional for 2.0 |
| Notification text shows in screenshots or calls | Med | `hide_from_capture`, `show_text = false`, nothing in meetings |
| A runaway script spams cards | Low | One card per `id`, at most 3 queued, 5 updates/s per id |

## Open questions (defaults in brackets, used unless the user says otherwise)

1. Notifications on fresh installs: ask once, or off until switched on? [ask once]
2. Show message text, or app and title only? [text on, one line, setting to turn off]
3. Live-card buttons: report-back only? [yes, plus the `open` body click; never run commands]
4. Submit 1.6.1 to winget now? [user's call; the CLA and Defender steps are theirs]
