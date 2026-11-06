# 8. Updates and troubleshooting

## Updating

Hytte checks a minute after it starts, then daily. When there's a new version, a card offers **Update** or **Later**. **Update** swaps in the new `hytte.exe` and restarts. Your settings, shelf and setup are kept.

To check now, use tray → **Check for updates…**. To turn off the automatic check, set `check_updates = false`.

Only `api.github.com` and `github.com` are contacted, and nothing about you is sent.

**Update by hand:** quit Hytte, replace `hytte.exe` with the one from the [releases page](https://github.com/PrathamPai-2007/hytte/releases/latest), and start it again.

**On version 1.6.0?** Update as soon as you can: use **Check for updates…**, or update by hand if it says a release "has no hytte.exe.minisig".

## Start here

Run this in a new terminal:

```powershell
hytte doctor
```

It checks the pill, `PATH`, your terminals, agents and `config.toml`, and says how to fix each problem. The log is `%APPDATA%\Hytte\hytte.log`.

## Common problems

| Problem | Try this |
|---|---|
| Nothing happens on start | It's probably already running. Check the tray. Only one copy runs. |
| I can't see the pill | A game or fullscreen app is in front, a window shrank it to a thin line (point at the line), or Hytte is paused in the tray. |
| `hytte: daemon not running` | Run `hytte`. Your command still ran. |
| `hytte` isn't recognised | Open a **new** terminal, or run `.\hytte.exe setup` from Hytte's folder. |
| Commands don't show up | Set up terminal integration and open a new terminal. Commands under 3 seconds are hidden on purpose. |
| An agent never shows up | Tray → **Connect AI agents**, restart the agent, and in Codex trust the hooks with `/hooks`. |
| Old terminals show no tasks after an update | Open a new terminal. |
| A task failed after Ctrl+C | Open a new terminal to get the current hook. |
| The update didn't install | The folder is probably read-only. Move `hytte.exe` under your user folder, or update by hand. |
| `config.toml` warning card | Click it to open the file at the mistake. Your previous settings stay in force. |
| A port isn't listed | Add it to `[ports] watch`, or set `show_all = true`. |
| The hotkey does nothing | Another app may own it, or it has no modifier. Try `"Ctrl+Alt+H"`. |
| A resource warning I don't want | **Ignore** it, raise `[hog] cpu_pct` / `mem_mb`, or set `[hog] enabled = false`. |
| "OCR unavailable" | Install a language pack in *Settings → Time & language → Language & region*. |
| No music controls | The player must report itself to Windows. Most browsers and players do. |
| A console window flashes at start | This happens on Windows 11 before 24H2, and it closes by itself. |
| The pill flickers on hover | [Open an issue](https://github.com/PrathamPai-2007/hytte/issues) with your Windows version and graphics driver. |

## Uninstalling

1. Run `hytte setup --undo` and `hytte setup agents --undo`.
2. Untick **Launch at startup** in the tray menu, then choose **Quit**.
3. Delete `hytte.exe` and the `%APPDATA%\Hytte` folder.
4. Delete the Hytte shortcut from `%APPDATA%\Microsoft\Windows\Start Menu\Programs`.
5. Optional: remove Hytte's folder from your user `PATH` in *Edit environment variables for your account*.

Back to the [manual](README.md).
