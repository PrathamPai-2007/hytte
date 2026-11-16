# 10. Custom panes

A custom pane is a tab of your own on the pill. It shows what a small script prints: a few lines of text, or rows with a coloured dot, a value and a progress bar. A row (or a button under the rows) can run something when you click it, such as restarting a service.

You can use panes other people made, or write your own. This chapter is for using them. For writing your own, see the [developer guide](../docs/17-writing-panes.md).

## Ready-made panes

Eight example packs come with Hytte's source, in the `examples/panes` folder. The full table is in the [examples README](../examples/panes/README.md).

| Pack | Shows |
|---|---|
| `github-prs` | Your open pull requests. Needs the GitHub CLI (`gh`), signed in. |
| `docker` | Containers, unhealthy ones first, with a confirmed **restart unhealthy** action. Needs Docker Desktop running. |
| `ci-status` | The latest GitHub Actions runs of one repository. Needs `gh` and a `repo.txt` you fill in. |
| `disk-space` | Each drive's used space as a bar. |
| `top-memory` | The five programs using the most memory. |
| `net-check` | Internet up or down, latency and DNS. |
| `git-repos` | Uncommitted and unpushed work in the repositories you list. Needs `git` and a `repos.txt` you fill in. |
| `recycle-bin` | Items and size in the Recycle Bin. A confirmed click empties it. |

## Installing a pack

A pack is a folder with a file called `pane.toml` in it (plus the scripts it runs).

1. Copy the whole folder into `%APPDATA%\Hytte\panes\`. For example:

   ```powershell
   Copy-Item -Recurse .\disk-space "$env:APPDATA\Hytte\panes\"
   ```

2. Hytte notices within a second and shows a card: **Run the pane "..."?** with **Run** and **Ignore**. The card shows the start of the command the pane would run, and how many actions it brings.
3. Press **Run** to switch the pane on. Press **Ignore** to keep it off.

**Nothing runs before you press Run.** The pane does not appear and its command does not start.

### What to check before you press Run

- **Read the command** on the card. It is what Hytte will run, on your PC, with your rights. Long commands are cut short on the card.
- **Open the folder.** The card has an **Open** button that opens the pack's folder. Look at `pane.toml` and at the scripts next to it. If you don't understand what they do, press **Ignore**.
- Only install packs from people you trust, the same as any program.

### How your answer is remembered

- Your answer is tied to the exact `pane.toml` you saw. **If the file changes, Hytte asks again.**
- The answer is kept in `%APPDATA%\Hytte\pack-answers\`, outside the pack. A downloaded pack therefore cannot arrive already approved.
- If you close the card without answering, the pane stays off and Hytte asks again the next time it starts.
- To be asked again about a pack you ignored, edit its `pane.toml` (even adding a space counts), or delete `<folder name>.ignored` from `pack-answers` and restart Hytte.

If a pack's `pane.toml` has a mistake, you get an amber card ("Pane pack ...") saying what is wrong. Its **Open** button opens `pane.toml`.

## Removing a pack

Delete its folder from `%APPDATA%\Hytte\panes\`. While Hytte is running, the pane and its row in Settings disappear within about a second.

The other custom panes keep their switch and their place in the tab strip. Hytte moves each pane's switch and position along with it when the panes around it change.

## Using a pane

- **The tab** has a general apps icon and the pane's name (16 characters at most).
- **Text panes** show the first six lines the script printed.
- **Row panes** show up to six rows. Each row can have text on the left, a value on the right, a progress bar, and a colour: green (`good`), amber (`warn`) or red (`bad`).
- **Click a row** that has an action to run it. Up to three **buttons** can sit under the rows; click one to run its action.
- **Sure?** Some actions ask you to confirm. The first click turns the row or button into **Sure?**, and the second click runs it.
- **The footer** is a quiet line under the pane:

  | Footer | Meaning |
  |---|---|
  | Updated just now · every 5 min | How fresh the text is, and how often the command runs. Later it reads "Updated 3 min ago". |
  | Exited with code 2 | The command failed. It shows in amber. |
  | Stopped after 10 s | The command ran too long and was stopped. |

- **The refresh button** at the end of the footer runs the pane's command now. Panes with no command (filled by `hytte pane set`) have no refresh button.

A pane only refreshes **while it is open**. Hytte also runs each pane once when it starts, so it has something to show. Nothing runs for a pane you are not looking at. While Hytte is paused or hidden for a game, nothing runs.

If a pane prints something that looks like a token or password, Hytte shows it as `•••`.

## Settings → Panes

Open [Settings](07-settings.md#choosing-and-ordering-panes) and find the **Panes** section.

- Each custom pane has a row under its own name, with a **switch** to show or hide its tab, and **▲ ▼** to move it along the tab strip.
- You can have **up to 12** custom panes. Panes from `config.toml` come first, then approved packs, while there is room.
- A pane whose name is already taken (names are compared without regard to upper or lower case) is skipped.

## The `config.toml` way

If you would rather not use a folder, add the pane to `config.toml` (tray → **Open settings folder**). Panes you write there need no approval, because you wrote them yourself.

```toml
[[script_pane]]
name = "Server"                          # the tab; 16 characters at most
command = "curl -s localhost:3000/health"
every = 300                              # seconds between runs; 30 at least

[[script_pane]]
name = "Deploys"                         # no command: your scripts fill it

[[action]]
name = "restart-api"                     # letters, digits, - and _
command = "docker restart api"
confirm = true                           # first click says "Sure?"
timeout = 60                             # seconds before it is stopped
```

A script prints rows like this:

```json
{"rows": [
  {"text": "api", "value": "up 3d", "tone": "good", "action": "restart-api"},
  {"text": "disk", "value": "82%", "progress": 82, "tone": "warn"}
], "buttons": [
  {"label": "Restart all", "action": "restart-api"}
]}
```

If the output is not valid rows, it simply shows as plain text.

## Sending text from your own scripts

`hytte pane set` pushes text into a pane from any script:

```powershell
hytte pane set "Deploys" --text "web v2.0.0 live"
git log -3 --oneline | hytte pane set "Deploys"                 # text can be piped in
hytte pane set "Status" --json --text '{"rows":[{"text":"api","value":"up"}]}'
```

- `--text "..."` gives the text; without it, Hytte reads what is piped in.
- `--json` checks that the text is valid rows. If it is not, nothing is sent and you are told why.
- Text over 4 KB is cut.
- The pane has to exist already, as a `[[script_pane]]` or an approved pack. If it does not, the command says so, names the panes there are, and exits with 1. It also exits with 1 when Hytte is not running.
- The text stays until the pane's next run or the next `hytte pane set`.

An agent can do the same through the `pane_set` tool ([chapter 4](04-ai-agents.md#let-an-agent-talk-to-the-pill-optional)).

## What Hytte protects, and what it doesn't

Hytte protects you from a few things:

- **No surprise runs.** A pack does nothing until you press **Run**, and asks again when `pane.toml` changes.
- **Answers stay outside the pack,** so a pack cannot approve itself.
- **A time limit.** A pane's command is stopped after 10 seconds, together with everything it started. Actions are stopped after their `timeout` (60 seconds unless they say otherwise; 600 at most).
- **Only listed actions.** A row or button can only name an action from the approved list. A script cannot send a command of its own.
- **Cleaned values.** An `arg` a row or button passes to its action is cut to one line of 200 characters, and the characters `& | < > ^ " %` and the backtick are removed. Actions read it as the environment variable `HYTTE_ARG`.
- Everything runs hidden and without a window.

What it does **not** protect:

- **Approval covers `pane.toml`, not the scripts it runs.** If `pane.toml` says `powershell -File prs.ps1`, you approved that line. Hytte does not check `prs.ps1`, so a later change to the script runs without asking again.
- A command runs **as you**, with everything you can do on this PC.

So: read before you press **Run**, and keep packs from people you trust.

## When something is wrong

| Problem | What to check |
|---|---|
| The pane is missing | Is it **switched on** in Settings → **Panes**? Is its name taken by an earlier pane? Are there already 12 custom panes? For a pack: have you pressed **Run** (an ignored or unanswered pack shows nothing)? Is there an amber "Pane pack ..." card about a mistake in `pane.toml`? Is the folder directly inside `%APPDATA%\Hytte\panes\`, with `pane.toml` in it? |
| It says "Waiting for output" | The pane has a command that has not printed anything yet. Open the pane and press the refresh button, or check that the command works in a terminal. |
| It says "Nothing sent yet" | The pane has no command: it shows what `hytte pane set "Name" --text "..."` sends. Send something. |
| It shows nothing after I pressed Run | Press the refresh button. Check the footer for "Exited with code N" or "Stopped after 10 s". Run the command yourself in the pack's folder and see what it prints. A command that needs a tool you don't have (such as `gh`) often prints its own message. |
| Clicking a row does nothing | The row's `action` has to match an action in the pack or in `config.toml`. If it doesn't, a card says "No action called ...". |
| I changed `pane.toml` and the pane went away | Editing it asks you again: answer the card. |

If it is still unclear, run `hytte doctor` and look at `%APPDATA%\Hytte\hytte.log`. See also [8. Updates and troubleshooting](08-updates-and-troubleshooting.md).

Back to the [manual](README.md).
