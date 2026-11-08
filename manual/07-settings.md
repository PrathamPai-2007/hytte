# 7. Settings

## The settings pane

Click the **gear** on the Home pane (or, if you have hidden Home, the tray icon → **Settings…**). The pane covers the common settings, such as theme, opacity, glow and feature switches. The blue **More settings** button at its bottom opens the full window.

<img src="../docs/img/settings-pane.png" alt="The settings pane inside the pill" width="420">

## The settings window

The **More settings** button on the settings pane opens the full window; so does tray → **Settings…**. It has every switch and slider.

- **Type** to search.
- **Rest the pointer** on a setting for an explanation.
- **Custom** opens `config.toml` at that setting, for a value the presets don't have.
- **↺** puts a changed setting back to its default.

### Choosing and ordering panes

The **Panes** section lists every pane: Quick controls, Music, Shelf, Terminal tasks, Timer, Battery, Notes, To-do and Weather. Each has a switch (the Quick controls say **Always on**: they are where the pill starts), and the **▲ ▼** buttons move it along the tab strip. Panes that only appear when they have something to show (Music, Shelf, Timer, Battery, Terminal tasks) keep their place in the order for when they do. Your own [script panes](03-terminal.md#your-own-panes) are listed as **Script pane 1** to **4**, in the order they appear in `config.toml`.

The **Accent** skin, the last of the six colour discs, follows your Windows accent colour (Settings → Personalisation → Colours). With the accent on **Automatic**, Windows picks it from your wallpaper, and the pill changes with it.

<img src="../docs/img/settings-window.png" alt="The settings window" width="560">

## Editing config.toml

Lists, the hotkey and folders are only in the file. Tray → **Open settings folder**, then open `config.toml`.

Every setting is optional and most apply as soon as you save. If you make a mistake, Hytte keeps your previous settings and shows a card with the line and column.

```toml
[general]
theme = "auto"            # "dark", "light" or "auto"
monitor = "primary"       # "primary", "active", "cursor" or a number
hotkey = ""               # e.g. "Win+Alt+N": opens the pill for keyboard use
monitor_hotkey = ""       # e.g. "Win+Alt+M": moves the pill to the next monitor
open_on = "hover"         # or "click"
fullscreen = "line"       # "line", "hide" or "off" (games always hide unless "off")
hide_from_capture = false # true keeps the pill out of screenshots and screen share
quiet_hours = ""          # no sounds, e.g. "22:00-07:00"
note_hotkey = ""          # e.g. "Win+Alt+J": opens Notes with the cursor ready
skin = "graphite"         # "graphite", "aurora", "ember", "moss", "dusk" or "accent"
output_folder = 'C:\Users\you\Hytte output'  # where file actions save; leave out to save next to the file
allow_list = []           # apps that never make Hytte back off
deny_list = []            # apps that always count as a game

[shell]
threshold_ms = 3000       # how long a command must run before it shows

[shelf]
mode = "reference"        # or "copy": keep a copy in Hytte's own folder

[ports]
watch = [3000, 3001, 4200, 5000, 5173, 5432, 8000, 8080, 8888]
show_all = false

[calendar]
enabled = false           # off by default: event titles are private

[panes]
media = true               # switch any pane's tab off here
shelf = true
tasks = true
timer = true
power = true
notes = true
todo = true
weather = true
order = ""                # tab order, set by the arrows in the settings window

[weather]
units = "auto"            # "auto" (follow Windows), "metric" or "imperial"

[media]
lyrics = true             # looks songs up on lrclib.net (sends title and artist)
lyrics_live = true        # follow the song line by line when the lyrics are timed

[breaks]
eyes_min = 0              # minutes between reminders; 0 = off
stretch_min = 0
water_min = 0

[timer]
focus_dnd = true          # a focus session turns on Windows Do Not Disturb
```

A hotkey needs at least one of `Win`, `Ctrl`, `Alt` or `Shift`, plus one key.

## Fullscreen apps and games

Use exe names in the two lists:

```toml
deny_list = ["MyGame.exe"]   # a game Hytte didn't detect
allow_list = ["Code.exe"]    # never hide or shrink for this app
```

`hytte.log` records what Hytte decided, if you need to find out why it hid.

For the complete list of settings, see the [configuration guide](../docs/11-configuration-and-files.md#every-setting).

Next: [8. Updates and troubleshooting](08-updates-and-troubleshooting.md)
