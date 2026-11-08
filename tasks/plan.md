# Implementation Plan: Hytte 2.0

## Overview

2.0 is everything already on local `main` (panes, notes, to-do, weather, lyrics, skins, breaks,
unsigned updates) plus five new things that turn the pill from an app into something you build on:

1. **Live cards.** Any script, CI job or app puts a card on the pill, updates it in place, gives it
   a progress bar and buttons, and can wait for which button was pressed.
2. **Script panes.** Your own panes, fed by a command Hytte runs on a timer, by your scripts
   pushing text in, or both.
3. **Accent skin.** A sixth skin that follows the Windows accent colour (which Windows picks from
   the wallpaper when Colours is on Automatic).
4. **Key hints.** Keyboard mode shows how to drive it, and the hotkey is shown where people look.
5. **Welcome tour.** Three cards on the pill, shown once: "What's new in 2.0" for updaters,
   "What Hytte does" for fresh installs.

Then a **launch video** for the README and landing page.

Out of scope (the old plan, dropped 2026-11-08): notifications in the pill, winget, live test pass
as a separate phase. Release hold still applies: nothing after 1.6.1 is pushed or tagged until the
user lifts it. All work stays on local `main`.

Tasks are in [todo.md](todo.md).

## Architecture decisions

### Live cards

- **Built on what exists.** `hytte notify` already sends a `Cmd` (`cmd: "notify"`), and `Ask`
  already waits for a reply on the same connection. A live card is a new `Cmd` kind `"card"` with
  optional fields, so old daemons ignore it and `old_messages_still_parse` keeps passing. No
  protocol version bump.
- **Fields:** `id` (update in place, 1–128 B), `title` (≤ 128 B), `body` (≤ 1024 B), `progress`
  (0–100), `icon` (fixed set: info, ok, warn, error), `buttons` (0–2 labels, ≤ 24 B each), `open`
  (http(s) URL or absolute path, opened on a body click), `ttl` (seconds, default 10, max 3600),
  `done`, `clear`. A card with `progress` lives until `done` or 30 s of silence, like `hytte set`.
- **Buttons never run commands.** A press is only reported back. With `--wait` the CLI keeps the
  connection open (up to the card's ttl), prints the pressed label and exits 0; no press prints
  nothing and exits 1. Same pipe, owner-only DACL and server check as `Ask`.
- **CLI:**
  `hytte card --id deploy --title "Deploy" --body "Uploading…" --progress 40`
  `hytte card --id deploy --done --body "Live"`
  `hytte card --title "Restart server?" --button Restart --button Later --wait`
- **On the pill:** drawn like a notify chip plus an icon, a progress bar and up to two buttons.
  Queues behind Ask chips (permission prompts always win) and the update offer. Text goes through
  `secrets::mask`. Nothing shows while paused, in a game or over fullscreen (the existing rule).

### Script panes

- **Declared in config only.** `[[script_pane]]` entries in `config.toml`: `name` (tab label,
  ≤ 16 chars), optional `command`, optional `every` (seconds, min 30, default 300). Max 4 panes.
  Nothing can be added from the pill or by a script, so a stray script can't fill the tab strip.
  Anyone who can edit `config.toml` already runs code as the user, so `command` adds no new trust.
- **Pull (Hytte runs it):** `cmd /c <command>` with `CREATE_NO_WINDOW`, in the user's home folder,
  10 s timeout then killed. Runs once at start, then every `every` seconds **only while the pane is
  showing** (plus once when it opens, if the last result is older than `every`). That keeps zero
  idle work when the pane is closed. Stdout: ANSI stripped, first 6 lines, ≤ 2 KB, through
  `secrets::mask`. A non-zero exit shows the output in the warn colour with "exit N".
- **Push (scripts feed it):** `hytte pane set <name> --text "…"` (or piped stdin) sends a `Cmd`
  kind `"pane_set"`. Push to a name not in config is ignored by the daemon; the CLI reads
  `config.toml` itself and says "no [[script_pane]] called <name>" with exit 1. A pushed value
  replaces the pane's text until the next pull or push. Not kept across restarts.
- **Both:** one pane type. Last writer wins; the pane shows "updated 3 min ago".
- **In the code:** `Panel::Script(u8)` (index into the config list), so the existing tab strip,
  `[panes] order` (keys `script1`–`script4`, by place in the file), 1–9 keys and settings arrows
  work unchanged. The settings window labels them "Script pane 1"… (its labels are static); the
  tab shows the pane's own name. A new
  `scriptpane.rs` holds the runner (a short-lived thread per run, like the weather fetch) and pure helpers (output
  cleaning, due check) with unit tests. Render in `render/script.rs`.

### Accent skin

- `Skin::Accent`, name `accent`, disc in the pill's Look tab and the settings window. Not the
  default.
- Colour from the registry's `Explorer\Accent\AccentPalette` (the accent itself, index 3), which
  Hytte already read for its button colour; read at start and on `WM_SETTINGCHANGE`
  (`ImmersiveColorSet`). Fallback when it can't be read: graphite.
- Body gradient, glow and rim are derived from the accent (darkened for the dark pill, washed out
  for the light one) by a pure function with a unit test that the text stays readable (contrast
  ≥ 4.5:1 against `fg` for a few known accents, including a very light and a very dark one).
- `Skin` stays `Copy`: the live colour sits in one atomic in `skin.rs` (`set_accent`), and the
  body brush cache is keyed on it.

### Key hints

- In keyboard mode a faint one-line hint sits under the pill content:
  `← → panes · Tab buttons · Enter press · Esc close` (shorter when typing: `Enter save · Esc
  cancel`; over a card the arrows are left out). It stays while keyboard mode is on.
- The hotkey is shown in the tray menu as "Open with keyboard  <chord>" (only when one is set;
  clicking it opens keyboard mode), and the first card of the welcome tour mentions it when
  one is set. No default hotkey (user's choice).

### Welcome tour

- Three cards on the pill with **Next** and **Skip** (the last card: **Done**). Shown once, after
  any Ask or update chip, never while paused, gaming, fullscreen, in a call or during focus.
- **Who sees which:** a fresh install (no `config.toml` at start) gets "What Hytte does": watch
  commands and agents · drop files, music and timers · point at the pill (or your hotkey). An
  update from 1.x (`config.toml` exists, no marker) gets "What's new in 2.0": Notes, To-do and
  Weather · live cards and script panes · skins, including Accent.
- **Marker:** `%APPDATA%\Hytte\welcome` holding the version seen (`2.0`). Written when the tour is
  finished or skipped. A later 2.x shows nothing; 3.0 can reuse it with new cards.
- **Content lives in one table** (`welcome.rs`) so the words can be changed without touching
  drawing code. Built last among the features so the cards describe what really shipped.

### Launch video

- Made with the `/brag` skill (Hyperframes), which reads the landing page code. The landing page
  copy is updated first for the 2.0 features (and the FAQ line that still says the pill hides
  from screenshots by default). No redesign; the landing redesign stages are separate work.
- Output: an MP4 (and a short GIF for the README) in `docs/img/`. Not published anywhere until
  the user says so.

## Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `cmd /c` scripts that hang or spawn children | Stuck worker, orphan processes | 10 s timeout, kill the process tree with a Job object (`KILL_ON_JOB_CLOSE`) |
| Script output with control chars or huge lines | Broken layout | Strip ANSI and control chars, cut to 6 lines / 2 KB, ellipsize per line in render |
| `--wait` connection held open | Pipe instances used up | Same limit as Ask; card ttl caps the wait at 3600 s; daemon drops the reply side on dismiss |
| Accent colour too light or too dark | Unreadable pill | Clamp lightness in the derive function; contrast test |
| Welcome tour shows at a bad time | Annoying first impression | Same gates as the update chip; never auto-opens the pill, it waits for the next hover |
| Scope creep before release | 2.0 never ships | Each phase ends in a checkpoint; anything new goes to a 2.1 list |

## Phases and checkpoints

1. Live cards (Tasks 1–3) → **Checkpoint A**: checks pass, snapshot page reviewed by the user.
2. Script panes (Tasks 4–7) → **Checkpoint B**: snapshot page; live run of a pull and a push pane.
3. Accent skin (Task 8) and key hints (Task 9) → **Checkpoint C**: snapshot page with accents.
4. Welcome tour (Task 10) → **Checkpoint D**: snapshot page of both tours; words approved.
5. Docs, landing copy, video (Tasks 11–13) → **Checkpoint E**: user watches the video; release
   still waits on the hold.

"Checks" means `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
UI changes get offscreen snapshots (`HYTTE_SNAP_DIR`) shown on one before/after page. Dev builds
only (isolated `%APPDATA%`); the Desktop Hytte stays stopped. Commits carry no trailers.
