# 6. Everyday tools

## Quick controls

<img src="../docs/img/home.png" alt="The Home pane" width="300">

The Home pane has four tiles, a volume bar and the settings gear.

| Control | Does |
|---|---|
| **Mic** | Mutes every microphone. A red lock stays on the pill. |
| **Sound** | Mutes your speakers. Scroll over it to change the volume. |
| **Awake** | Keeps the PC awake. Click to cycle: off, one hour, on. |
| **Timer** | Opens the timer card. |
| Volume bar | Drag or scroll to change the volume. |
| ▾ on the volume bar | Shows a volume bar for each app making sound. |

## Music

<img src="../docs/img/media.png" alt="The Media pane" width="300">

The Media pane controls whatever is playing, as long as the player reports itself to Windows (most browsers and players do). Drag the timeline to seek, and click the album art to raise the player.

## Timer and focus sessions

<img src="../docs/img/timer.png" alt="The Timer pane" width="300">

1. Home → **Timer**, or right-click the pill.
2. Pick the minutes. Scrolling works here too.
3. Choose **Start timer** for a plain countdown, or **Start focus** for focus sessions with breaks.

While the pill is closed, the timer burns down as a thin fuse along it. Click the ring to pause. A timer survives a restart.

When it ends, an alarm rings for up to two minutes. Choose **Stop alarm** or **+5 min**. After a focus session, Hytte offers your break: 5 minutes, with a 15-minute long break every second round. You can change these in [Settings](07-settings.md).

## Dev servers

Anything listening on a common port (`3000`, `5173`, `8080` and others) appears under **SERVERS** in the Tasks pane. Servers are named after their project folder.

- **Open** opens it in your browser.
- **Stop** stops it, and asks twice.
- **Restart** appears for a server you started from a tracked command.

To list another port, add it to `watch`, or set `show_all = true` ([Settings](07-settings.md#editing-configtoml)).

## Alerts

| Alert | Buttons | Turn it off |
|---|---|---|
| **Browser downloads** show as tasks, then a finished card. | **Open**, **Shelve** | `[downloads] enabled = false` |
| **Resource hogs:** an app using over 70 % of the CPU for 30 seconds, or over 4 GB of memory. | **Kill** (asks twice), **Ignore** | `[hog] enabled = false` |
| **Next meeting**, 10 minutes before it starts. Off by default, because titles are private. | **Join**, when there's a link | Turn on with `[calendar] enabled = true` |
| **Camera and mic dots** while an app uses them. | | Always on |

## Battery

On laptops, the Power pane shows your charge and time left, and lets you switch between **Saver**, **Balanced** and **Performance**.

Next: [7. Settings](07-settings.md)
