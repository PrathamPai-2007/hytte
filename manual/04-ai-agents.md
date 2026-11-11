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

**Allow / Deny** needs a hook that fires only when the agent is about to ask you. Gemini CLI, Antigravity and Cursor don't have one (their hooks run on every tool call, and Cursor ignores an "allow" answer), so their prompts stay in the terminal.

## Day to day

- **Agents pane:** every agent has a row in the **Agents** tab: its project, what it is doing or asking, how long it has waited, and what it has done this turn (files edited, subagents started, context size). Click a row to jump to its terminal. Turn the tab off in [Settings](07-settings.md) and agents list with your terminal tasks again.
- **Working:** the agent appears by name, with its current step where the agent reports one, such as `Editing src/main.rs`.
- **How long it has run:** a working agent shows its time on this turn (it starts over with each prompt), and the finished card says "Ran 8 min" on its quiet line when the turn took ten seconds or more.
- **Quiet:** a working agent that has said nothing for 5 minutes (`[agent] quiet_after_secs`, `0` = never) shows a grey "quiet 6m". It only means no news: a long model call or a long test run looks the same. There is no sound and no alert.
- **Several agents:** with more than one row the pill shows a count; it turns amber while any agent waits for you, and says "2 waiting" when more than one does.
- **Waiting for you:** the pill glows amber and beeps, even over a maximised terminal. Click it to jump to the agent's terminal.
- **Done:** a card shows the first line of the reply, with **Copy**, and a quiet line such as "Edited 4 files · 84k context" when Hytte knows it.

To silence the beep, turn it off in [Settings](07-settings.md) or set quiet hours.

## My agent doesn't show up

Run `hytte doctor`. For each agent it finds it says one of:

- **connected**: the hooks are in place. For Codex, type `/hooks` in Codex once to approve them. Then restart the agent.
- **installed but not connected**: run `hytte setup agents` (or the tray item).
- **its hooks point at a hytte.exe that is gone**: Hytte moved. Run `hytte setup agents`; Hytte also repairs this when it starts.
- **its settings file isn't plain JSON**: the file has comments or a trailing comma, so Hytte left it alone rather than risk breaking it. Remove them, then run `hytte setup agents`.

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
