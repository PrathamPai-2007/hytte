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
| Copilot CLI | working, done |

**Allow / Deny** needs a hook that fires only when the agent is about to ask you. Gemini CLI, Antigravity, Cursor and Copilot CLI don't have one (their hooks run on every tool call, and Cursor ignores an "allow" answer), so their prompts stay in the terminal.

## Agents inside WSL

If you run Claude Code, Codex, Copilot CLI or another supported agent inside a WSL distro, run `hytte setup agents --wsl` (and `hytte setup agents --wsl --undo` to remove it). Hytte adds the same hooks inside each distro's agent folder, pointing at `hytte.exe` through `/mnt/c/...`. The agent then shows up on the pill like any other, and clicking its row brings its terminal forward. This is never done for you: plain **Connect AI agents** only touches Windows. Restart running agents afterwards. Hytte starts a stopped distro to find its home folder.

## Day to day

- **Coding agents pane:** every agent has a row in its own tab: its project, what it is doing or asking, how long it has waited, and what it has done this turn (files edited, subagents started, context size). Click a row to jump to its terminal. This tab is on by default.
- **Where agents show:** two switches under **Panes** in [Settings](07-settings.md#choosing-and-ordering-panes) decide.

  | Coding agents | Terminal tasks | Agents show |
  |---|---|---|
  | on | on | In the Coding agents pane. Terminal tasks lists only commands. |
  | on | off | In the Coding agents pane. |
  | off | on | Inside Terminal tasks, with your commands. |
  | off | off | Nowhere. Neither agents nor terminal tasks show, even on the compact pill. |
- **Today:** a quiet line under the rows adds up the day: time agents spent working, time they waited for you, and turns finished ("Today  2 h 10 min working · 14 min waiting for you · 9 turns"). It lives in memory only, starts over at midnight and when Hytte restarts, and is never written to disk.
- **Working:** the agent appears by name, with its current step where the agent reports one, such as `Editing src/main.rs`.
- **How long it has run:** a working agent shows its time on this turn (it starts over with each prompt), and the finished card says "Ran 8 min" on its quiet line when the turn took ten seconds or more.
- **Quiet:** a working agent that has said nothing for 5 minutes (`[agent] quiet_after_secs`, `0` = never) shows a grey "quiet 6m". It only means no news: a long model call or a long test run looks the same. There is no sound and no alert.
- **Several agents:** with more than one row the pill shows a count; it turns amber while any agent waits for you, and says "2 waiting" when more than one does.
- **Waiting for you:** the pill shows a bell and beeps (and glows amber, with the glow on), even over a maximised terminal. Click it to jump to the agent's terminal.
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

## Let an agent talk to the pill (optional)

Hytte can also be a tool the agent calls. Run `hytte setup mcp` and Claude Code and Codex (whichever are installed) get a `hytte` tool server; `hytte setup mcp --undo` removes it. Nothing registers it unless you run that. New agent sessions then have these tools:

- **notify**: a short message on the pill.
- **card**: a card with a title, text, progress bar or up to two buttons. With `wait`, it asks you and the agent receives the label of the button you press, or "no answer" if you don't press one in time.
- **note_add** and **todo_add**: add to your Notes and To-do lists.
- **pane_set**: set the text of one of your script panes.

- **pane_list** and **action_list**: the names of your panes and of the actions you set up (never their commands).
- **pane_get**, **agent_status** and **events**: read what a pane shows, which agents the pill is tracking (and whether one is waiting for you), and what happened lately (an action finished, you pressed a card, you answered an ask). An agent can also wait up to 25 seconds for the next thing to happen instead of asking again and again.
- **pane_create**, **pane_update**, **pane_remove**: the agent can add a pane of its own (up to four). It does not run until you press **Run** on the card that shows its command, and a change asks again. It can never change or remove your own panes.
- **action_run**: the agent can ask to run one of your named actions. You see **Allow** or **Deny** every time (unless you allowed that one action in advance with `hytte mcp grant`), and no answer in 30 seconds counts as no.

A card's button press only reports back to the agent; it never runs anything. The agent cannot run a command it makes up, only an action you already defined. Every pane change and action attempt is written to `%APPDATA%\Hytte\mcp.log`. The details, and what these checks do not cover, are on [page 11](11-pane-store.md#when-an-agent-wants-to-add-a-pane-or-run-something). Each call does nothing while Hytte is paused or not running (the agent is told so).

## Other tools

Anything that can run a command can use the pill:

```powershell
hytte agent needs-input --name "MyTool" --message "waiting"
hytte agent done --name "MyTool"
```

Next: [5. Shelf and file actions](05-shelf.md)
