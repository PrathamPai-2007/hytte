# 16. What's new

What changed in each version, newest first. The first part is for people who use Hytte; each version ends with a short note for people who change it.

## Next release (on `main`, not released yet)

**Settings**

- **New defaults** (on a fresh `config.toml`; a file you already have keeps its values): starts with Windows, theme follows Windows, the open pill is 70 % opaque and an idle one 75 %, the pill closes after 0.1 s, the pin button is on, the pill peeks for 3 s when a task finishes, an agent that needs you beeps, resource alerts at 70 % CPU, and the timer has 2 rounds before a long break.
- **Glow strength is a slider** in the settings window, 0 to 100 %, where 100 % is 20 % stronger than the old "Strong". The pill's small pane has steps Off, Soft, Normal and Max.
- **Every setting has a clearer name**, and resting the pointer on one explains it. The names in the pill's small pane are shorter versions of the same.
- **A Custom step on every number** (waits, sizes, opacities, break lengths, thresholds): it opens `config.toml` at that setting with a note, and shows the value when it is none of the steps. The reset (**↺**) now sits in a column at the right of every row and restores the exact default.
- **Per-app volume only lists apps you can see or hear.** Idle background helpers that hold an audio session (an audio console, a wallpaper engine) are left out, and an app is named as it names itself ("Spotify", "Wallpaper Engine").
- **Search and reset in the settings window.** Start typing and only the settings that match stay (a section's name matches all of its rows). Esc clears the search, and closes the window when there is nothing to clear. A small **↺** appears beside any value that isn't its default; one click puts it back.
- **Two new optional settings, both off by default.** `pin_button` adds a pin to the open pill: click it and the pill stays open until you click it again (it lets go by itself when a fullscreen app takes the screen). `remember_panel` makes `start_panel = "last"` survive a restart.
- The settings window follows a change of the Windows theme or accent colour while it is open.

**Looks**

- A title too long for the pill (a song, a task) now **fades out at its end** instead of being cut with "…". Hover it and it still scrolls.
- A task that failed or is waiting for you gets a faint wash and a bar in its colour, and a finished task's name recedes.
- The pill opens with a little less bounce, and panes cross-fade a touch more softly.

**Fixes**

- **Hovering the small second pill** (the timer's clock, or the music's album art beside a compact task) opens that pane: the timer or the player, not the main pill's.
- **Ctrl+C in PowerShell no longer shows a failed task**, for a command that doesn't leave a recognisable exit code too. A failing cmdlet reports 1 instead of an old exit code from the last program. Open a new terminal after updating so its hook is the new one.
- **Screenshots: delete after paste is now on by default.** A screenshot is deleted about two seconds after you paste it. If your `config.toml` already says `delete_after_paste = false`, that stays; switch it on in the settings window.
- A second clipboard update from the Snipping Tool no longer cancels the paste watch.

**Timer and updates**

- The end-of-timer chime is softer: a rising chord with a gentle start, a slow fade and a longer pause.
- The update prompt says what is new in the release ("New in 1.4.0: …") when the release has notes.

**Startup**

- If `config.toml` has a mistake, Hytte says where (line and column) in a warning card you can click to open the file. A setting it doesn't know (usually a typo) is named and ignored. Settings written by older versions that no longer exist are ignored quietly.

**For people who change Hytte**

- `scripts/release.ps1` cuts a release (version bumped everywhere, checks, commit, tag). `scripts/version.ps1` keeps the version in both `Cargo.toml` files, `Cargo.lock` and the winget manifest in step; CI and the release workflow check it. After a release, a job commits the published zip's SHA-256 to the winget manifest ([section 13](13-testing-and-release.md#how-a-release-is-built)).
- The paste decision and the snapping geometry are plain functions with tests.

## 1.3.0

- **A big settings window** (the pane's popout button): every switch, slider and step in sections, with a section list, smooth scrolling and light and dark themes.
- **Nine new settings:** how long the pill waits before it closes, hover or click to open, size, idle opacity, motion speed, which panel it opens on, quiet hours for sounds (with a **Custom** option that opens the file with instructions), and how many task rows show before the list scrolls. Plus a **pill opacity** slider and a **hover delay** (0.2 s by default).
- **Snapped windows** (a window split to half the screen) now give the thin line, like a maximised one.
- **Screenshots** leave the shelf after a minute (`screenshot_secs`), and `delete_after_paste` can delete one when it is pasted.
- **Terminals:** a task row for a closed terminal no longer lingers as an error; a command you stopped with Ctrl+C doesn't show as failed (for programs that report it); the task list scrolls instead of overflowing.
- Fresh screenshots and a documentation pass with every figure checked.

Older versions: see the [releases page](https://github.com/PrathamPai-2007/hytte/releases).

Next: [documentation index](README.md)
