# 4. AI agents

Hytte shows what your coding agents are doing and tells you when one needs you.

## Connect them

1. Tray → **Connect AI agents** (or run `hytte setup agents`). Hytte adds small hooks to each agent it finds.
2. Restart any agents that are running.
3. In Codex, type `/hooks` once to trust them.

Your own settings are kept, and the original file is saved as `<file>.hytte-backup`. To remove the hooks, choose the tray item again or run `hytte setup agents --undo`.

| Agent | The pill shows |
|---|---|
| Claude Code | working, current step, waiting, done, **Allow / Deny** |
| Codex | working, current step, done, **Allow / Deny** |
| OpenCode | working, waiting, done, **Allow / Deny** |
| Gemini CLI | working, waiting, done |
| Antigravity | working, done |
| Cursor | working, done |

## Day to day

- **Working:** the agent appears by name, with its current step where the agent reports one, such as `Editing src/main.rs`.
- **Waiting for you:** the pill glows amber and beeps, even over a maximised terminal. Click it to jump to the agent's terminal.
- **Done:** a card shows the first line of the reply, with **Copy**.

To silence the beep, turn it off in [Settings](07-settings.md) or set quiet hours.

## Allow and Deny

When a supported agent asks permission, the pill shows what it wants to do, with **Allow** and **Deny**, over any window.

Read it before clicking. Long or multi-line commands say so ("2 lines, see terminal"), which means you should check the full command in the terminal first.

If you don't answer within 30 seconds, the agent's normal prompt appears in the terminal instead, so nothing is approved by accident. The same happens if you dismiss the card, or Hytte is paused or hidden for a game.

When an agent asks you a question rather than asking permission, the pill shows it as waiting. Answer in the terminal.

## Other tools

Anything that can run a command can use the pill:

```powershell
hytte agent needs-input --name "MyTool" --message "waiting"
hytte agent done --name "MyTool"
```

Next: [5. Shelf and file actions](05-shelf.md)
