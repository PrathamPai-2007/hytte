# 17. Writing custom panes

A custom pane is a tab you define yourself. Hytte runs a command of yours (or you push text to it) and shows the result as plain lines or as rows with values, bars and clickable actions. You need no Rust: a pane is a script and a few lines of TOML.

This section is the whole contract: how to declare a pane, how it runs, what it may print, what a click can do, what the limits are and how to debug one. Everything here is checked against `crates/hytte/src/scriptpane.rs`, `packs.rs`, `config.rs`, `ui_state/script.rs`, `render/script.rs`, `window/win/scripts.rs` and `window/win/actions.rs`. Finished examples to copy are in [`examples/panes`](../examples/panes/README.md).

## The two ways to declare a pane

| | `[[script_pane]]` in `config.toml` | A pane pack folder |
|---|---|---|
| Where | `%APPDATA%\Hytte\config.toml` | `%APPDATA%\Hytte\panes\<folder>\pane.toml` |
| Approval | None: it is your own file | A card asks **Run** / **Ignore** first |
| Command runs in | your home folder (`%USERPROFILE%`) | the pack's folder |
| Its actions | `[[action]]` blocks in `config.toml` | `[[action]]` blocks in its own `pane.toml` |
| Push-only (no command) | yes | no: a pack must have a `command` |
| Good for | something only you use | something you share or copy between machines |

Both kinds are the same pane once loaded. Both can also be fed with `hytte pane set` ([push mode](#push-mode)).

### `[[script_pane]]`

```toml
[[script_pane]]
name = "Deploys"                 # the tab's label
command = "powershell -NoProfile -File C:\\tools\\deploys.ps1"
every = 120                      # seconds between runs while the pane is open

[[action]]                       # optional: what a click may run
name = "open-dashboard"
command = "start https://example.com/deploys"
```

| Key | Default | Rules |
|---|---|---|
| `name` | none | Required. Trimmed, cut to 16 characters. A pane with no name is skipped; a second pane with the same name (any case) is skipped. |
| `command` | `""` | Run with `cmd`. Empty means push-only: the pane shows only what `hytte pane set` sends. |
| `every` | `300` | Seconds between runs, never below 30 (a smaller number is raised to 30). |

`[[action]]` in `config.toml` takes the same four keys as a pack's (`name`, `command`, `confirm`, `timeout`). A name is letters, digits, `-` and `_`, up to 40 characters; an invalid name, an empty command and a repeated name (any case) are skipped; at most 32 actions are kept. Actions run in your home folder. A row names one exactly as written.

An unknown key in a `[[script_pane]]` is ignored, but Hytte tells you (a warning card, "no such setting script_pane.x"), so a typo such as `comand` is caught. Edits to `config.toml` apply live.

### A pane pack

```toml
# %APPDATA%\Hytte\panes\docker\pane.toml
name = "Docker"                  # tab label; default: the folder's name
command = "powershell -NoProfile -ExecutionPolicy Bypass -File containers.ps1"
every = 60

[[action]]                       # a row or button may name it as "restart-unhealthy"
name = "restart-unhealthy"       # Hytte calls it docker-restart-unhealthy
command = "powershell -NoProfile -ExecutionPolicy Bypass -File restart-unhealthy.ps1"
confirm = true
timeout = 120
```

| Key | Default | Rules |
|---|---|---|
| `name` | the folder's name | Trimmed, cut to 16 characters. |
| `command` | none | Required (a pack with none is flagged as a mistake). Runs in the pack's folder. |
| `every` | `300` | Never below 30. |
| `[[action]] name` | none | Letters, digits, `-` and `_`, 1 to **19** characters, unique within the pack (any case). |
| `[[action]] command` | none | Required. Runs in the pack's folder. |
| `[[action]] confirm` | `false` | `true`: the first click asks "Sure?", the second runs. |
| `[[action]] timeout` | `60` | Seconds until the action and everything it started is stopped; kept between 1 and 600. Leave it out (or write `0`) for the default. |

Rules the loader applies (`packs::parse`):

- **Action names get a prefix.** The folder name, lower-cased, with anything that is not a letter, digit or `_` turned into `-`, cut to 20 characters and trimmed of `-`, then `-` and your name: `docker` + `restart-unhealthy` = `docker-restart-unhealthy`. A row or button in the pack's output names its action *without* the prefix (`"action": "restart-unhealthy"`); Hytte adds the prefix itself, so a pack can only name its own actions. 20 + 1 + 19 characters is exactly the 40-character limit on a full name.
- **At most 8 actions** per pack. Further `[[action]]` blocks are ignored without a message.
- **A mistake makes the whole pack Bad** and it is not loaded: invalid TOML, no `command`, a folder name with no letters or digits, an action name that is invalid or longer than 19 characters, an action with no command, or two actions with the same name. A card says which ("Pane pack "docker": ...") and **Open** opens `pane.toml`. Keys Hytte does not know are ignored silently.
- **The folder is only read one level deep:** `panes\<folder>\pane.toml`. Folders without a `pane.toml` are skipped.

## Approval: a pack runs nothing until you say so

A new pack, or one whose `pane.toml` changed, is **Pending**. Hytte shows a card, `Run the pane "Docker"?`, with the pane's command (first 60 characters) and "+N action(s)", and the buttons **Run**, **Ignore** and **Open** (the pack's folder). Until you press Run the pane does not exist: no tab, no command, no actions.

How the answer is kept (`packs::answer`):

- The answer is a file in **`%APPDATA%\Hytte\pack-answers\`**, outside the pack: `<folder>.approved` or `<folder>.ignored`, holding a **hash of `pane.toml`**. A pack cannot bring its own answer; an `.approved` file inside the pack folder counts for nothing (before 1.9.3 it was kept inside the pack).
- The hash is **FNV-1a** over the bytes of `pane.toml`, 16 hex digits (`packs::hash`). The answer holds only while the hash matches, so **any edit to `pane.toml`, even a space, asks again**.
- Each state: **Approved** (hash matches a Run), **Ignored** (hash matches an Ignore: the pane is not loaded and nothing asks again), **Pending** (no answer, or the file changed), **Bad** (a mistake, above).
- The card is shown once per session for each version of the file. If it times out (it stays 60 seconds) or you dismiss it, the pack stays Pending and Hytte asks again at the next start. To change an Ignore, edit `pane.toml` or delete `pack-answers\<folder>.ignored`.

What the hash does **not** cover: **the scripts the pack runs.** `pane.toml` says `command = "powershell -File prs.ps1"`; the hash covers that line, not `prs.ps1`. Someone who can change `prs.ps1` after you pressed Run changes what runs, and Hytte will not ask. The card also shows only the *pane's* command, not the commands of its actions. Read the whole folder before you press Run, and treat a pack like any program you download. Approving a pack makes its actions callable by name (a card button `Label:docker-restart-unhealthy` from `hytte card` can run one, for example); the pipe still carries only names, never a command.

A pane shows when it is **Approved** and there is room: see [the pane cap](#the-pane-cap-slots-and-switches).

## How a command runs

The same runner (`scriptpane::run_with`) serves pane commands and actions.

| Aspect | Behaviour |
|---|---|
| Shell | `cmd /d /s /c "<your command>"`. `/d` skips `cmd`'s AutoRun, `/s` keeps your own quotes intact. Write the command as you would type it at a `cmd` prompt. |
| Window | None (`CREATE_NO_WINDOW`). Stdin is closed (null). |
| Working folder | The pack's folder for a pack's command and actions; your home folder for a `[[script_pane]]` and for `config.toml` actions. Use relative paths in a pack. |
| Environment | Hytte's own, plus **`HYTTE_ARG`**, always set: the `arg` of the row or button that was clicked, or empty (also empty for a pane's own command, never inherited from Hytte's environment). |
| Time limit | **10 seconds** for a pane command; an action's `timeout` (default 60 s, at most 600 s). |
| On timeout | The process **and everything it started** are killed (a Windows Job object), whatever the script did. Output printed before that is still shown. |
| Output read | Stdout; **stderr when stdout is empty**. At most 64 KB of each is read. Decoded as UTF-8 (invalid bytes become replacement characters), so set `[Console]::OutputEncoding` to UTF-8 in PowerShell. |
| Exit code | Shown, not required: a non-zero exit still shows what was printed, with the problem in the footer. |
| Priority | The worker thread runs at EcoQoS (low priority). |

**When a pane command runs** (`window/win/scripts.rs`):

- **Only while the pane is open** (the pill is showing that pane), **plus once at start** for every pane that has a command, so each has something to show. A pane you never open does not run again.
- When you open a pane whose last run is older than `every`, it runs at once. While it stays open, it repeats every `every` seconds, counted from the *start* of the previous run. A run never overlaps itself: if one is still going, the next waits.
- **The refresh button**, at the right end of the footer, runs it now. It appears on every pane with a command and is dimmed while a run is going.
- Nothing runs while Hytte is paused or in game mode, and pushed text is ignored then too.
- Changing a pane's `command` in the config runs it at the next chance. A pane keeps what it showed across config edits as long as its name stays the same; renaming it starts it empty.

**If it goes wrong** the footer (in amber, replacing the "Updated ..." line) says so:

| Footer | Meaning |
|---|---|
| `Exited with code N` | The command ended with a non-zero code. `-1` also means Hytte could not start it ("Couldn't start it: ...") or the worker crashed ("Hytte hit an error running it"). |
| `Stopped after 10 s` | The time limit hit. |
| `Updated just now` / `Updated 5 min ago` / `Updated 2 h ago`, then ` · every 2 min` or ` · every 45 s` | Fine. The `every` part appears only when the pane has a command. |

With no output yet the pane says `Running…` (and the command) while a run is going, `Waiting for output` after, or, for a push-only pane, `Nothing sent yet` and the exact `hytte pane set` line to use.

The text on the pane passes through Hytte's secret mask (`secrets::mask`), which hides common token shapes and `password=...` style values. It is a display aid, not a guarantee: do not print secrets.

## Output formats

Hytte decides by looking at the output once the command ends (or at pushed text): if, after trimming, it starts with `{`, parses as JSON and has a `rows` **array**, it is a **rows pane**. Anything else (plain text, broken JSON, JSON without `rows`) is shown as **text**. Broken JSON therefore appears on the pane as raw lines, which is a quick way to see that you have a syntax error.

### Plain text

The simplest pane: print lines.

| Rule | Value |
|---|---|
| Lines kept | **6** (the first six after cleaning) |
| Bytes kept | **2048**, cut on a character boundary |

Cleaning (`scriptpane::clean`), in order: terminal colour and cursor codes (`ESC [ ...`) and title or link codes (`ESC ] ... BEL`) are removed; other control characters are dropped; a tab becomes one space; a line redrawn with `\r` (a progress bar) keeps its last non-blank state; trailing spaces are trimmed (leading ones are kept); blank lines at the start and end are dropped. The first line is drawn a little brighter than the rest. Lines are drawn on one line each: keep them short, the pane is 380 px wide.

### JSON rows

```json
{
  "rows": [
    {"text": "api",   "value": "up",     "tone": "good"},
    {"text": "db",    "value": "78 %",   "progress": 78, "tone": "warn"},
    {"text": "queue", "value": "stuck",  "tone": "bad", "action": "restart", "arg": "queue"}
  ],
  "buttons": [
    {"label": "Restart all", "action": "restart-all"}
  ]
}
```

**Row fields**

| Field | Type | Limit and default |
|---|---|---|
| `text` | string or number | Left side. Cleaned like plain text, first line only, cut to **80 characters** (a cut ends in `…`). Default empty. |
| `value` | string or number | Right side, in the tone's colour. First line only, cut to **24 characters**. Default empty. |
| `progress` | number | A thin bar under the row. Rounded, **clamped to 0 to 100**. A string is ignored. Absent: no bar. |
| `tone` | `"good"`, `"warn"`, `"bad"` | A green, amber or red dot before the text, and the value in that colour (the bar too). Anything else, or absent: plain (no dot). |
| `action` | string | The name of an action to run when the row is clicked. A valid name only: letters, digits, `-`, `_`, 1 to 40 characters; anything else is dropped and the row is simply not clickable. |
| `arg` | string | Handed to that action as `HYTTE_ARG`. See [Actions](#actions). Only a string counts (a number is ignored). |

**Buttons** (`buttons`, optional, next to `rows`)

| Field | Type | Limit and default |
|---|---|---|
| `label` | string | Required, cleaned, one line, cut to **16 characters**. A button with an empty label is dropped. |
| `action` | string | Required, a valid action name (as above). |
| `arg` | string | Optional, as for rows. |

**What is dropped silently** (nothing reports it):

- Rows past the sixth, entries in `rows` that are not objects, and rows with neither `text` nor `value`.
- Buttons past the third, and buttons with no label or no valid `action`.
- Buttons when there are no rows (they sit under the rows).
- An invalid `action` name on a row or button; for a pack, an action whose name becomes invalid (over 40 characters) once the prefix is added.
- Unknown fields at any level.

`{"rows": []}` is valid and shows the empty-state hint, not an error. A row's text, value and button labels are drawn through the secret mask too.

Each row is 26 px tall (31 with a progress bar) and the buttons add a 30 px strip, so the pane grows with what you print.

## Actions

A row or button runs an action: a named command declared next to the pane (`[[action]]` in `config.toml`, or in the pack's `pane.toml`). The script only ever sends a **name**: it cannot send a command line, so a script (or anything that can write to the pipe) can press the buttons you declared and no others.

How a click is handled (`Ui::run_named`):

1. Hytte looks the name up (any case). An unknown name shows the card `No action called "x" in config.toml`.
2. **Confirm.** An action with `confirm = true` arms on the first click: the row's value (or the button's label) turns amber and reads **Sure?** for **3 seconds**. A second click within that time runs it. Only one thing is armed at a time; arming another disarms the first.
3. **Per (action, arg).** Arming and "still running" are tracked per action *and* arg. Two rows that name the same action with different args are separate: arming one does not arm the other, and one running does not block the other. A click on a press that is still running is ignored.
4. The command runs on a worker with `HYTTE_ARG` set, hidden, with the action's `timeout` and the Job object that kills its tree.
5. **A result card** appears: `<name> finished`, `<name> failed (exit N)` or `<name> stopped after N s` (`name` is the full, prefixed action name), with the first line the action printed (at most 80 characters) under it. Make your action print one useful line.

For `[[script_pane]]` panes the row names the action as written in `config.toml`. For a pack it names it without the prefix.

### `arg` and `HYTTE_ARG`

`arg` lets one action serve many rows ("restart *this* container"). Hytte passes it in the **environment variable `HYTTE_ARG`**, always set and empty when there is no arg. It is also set (empty) for pane commands.

Before it is passed, Hytte cleans the arg (`scriptpane::clean_arg`): one line only, at most **200 characters**, with `&`, `|`, `<`, `>`, `^`, `"`, `%`, the backtick and every control character removed; an arg empty after that is no arg.

**Treat `HYTTE_ARG` as untrusted text anyway.** The list above is the set of characters `cmd` and PowerShell escapes treat specially, not a guarantee that the value is safe for every command line. Spaces, `;`, `$`, parentheses, quotes of the other kind, braces and a leading `-` all get through. And the text may come from data you do not control: a container name, a pull request title, a file name.

- **Read it as `$env:HYTTE_ARG` inside a PowerShell script.** Do not build the command with it: `command = "powershell -Command Restart-Service %HYTTE_ARG%"` lets the `cmd` layer paste the value into a PowerShell command line, where `$(...)` or `;` can run code. `-File restart.ps1` reading `$env:HYTTE_ARG` treats it as one string and nothing else.
- **Validate it against what you expect** (`-match '^\d+$'` for a pid, a name pattern, a lookup in your own list) and refuse the rest, as `close.ps1` does in [example 3](#3-an-action-with-confirm-and-arg).
- Never pass it to `Invoke-Expression`, `cmd /c`, `-Command` or a string you then evaluate. Pass it as a separate argument to a program, with `--` before it when the program supports that, so a value starting with `-` is not read as an option.

## Push mode

Instead of (or as well as) a command, anything can push text to a pane by name:

```powershell
hytte pane set "Deploys" --text "web v2.0.0 live"
"line one`nline two" | hytte pane set "Deploys"
$json | hytte pane set "Deploys" --json
```

| Flag | Meaning |
|---|---|
| `<name>` | The pane's name, any case. Quote it if it has spaces. |
| `--text "..."` | The text. Without it, the text is read from **stdin** (up to 4096 bytes). Typed at a terminal with nothing piped, the command prints the usage and exits 2. |
| `--json` | Check, before sending, that the text is `{"rows": [...]}`. If it is not, nothing is sent and the exit code is 1. |

What happens (`cmd_pane`, `Cmd::pane_set`):

- The CLI reads `config.toml` (it never writes it) and scans the packs, then checks that a **`[[script_pane]]` or an approved pack pane** has that name (since 1.9.3 a pack pane counts). If not, it prints `no pane called "x"` with the names there are and exits 1. A pack that is only Pending does not count. The pane's tab being switched off does not matter.
- The text is cut at 4096 bytes and sent as a `pane_set` pipe command (`{"v":1,"cmd":"pane_set","id":"<name>","text":"..."}`; `id` is the name, at most 64 bytes; [section 4](04-ipc-protocol.md#script-pane-text-pane_set)). Exit 1 if Hytte is not running.
- The pane decides how to show it **by content**, exactly like command output: JSON with `rows` is shown as rows (and `buttons`), anything else as cleaned text (6 lines, 2048 bytes). `--json` only adds the early check. An empty text empties the pane. Pushed text clears a problem in the footer and sets "Updated just now".
- If the pane also has a command, the next run replaces what you pushed. For a purely pushed pane, set `command = ""` in a `[[script_pane]]`: the footer then shows only the age, and there is no refresh button.
- A pane's action names in pushed rows follow the same rule: a pack pane's rows name actions without the prefix.

**MCP.** The `hytte mcp` server offers a `pane_set` tool with `name` and `text` arguments (up to 4096 bytes) for agents ([section 5](05-cli.md#hytte-mcp-and-hytte-setup-mcp---undo-climcprs)). It accepts the same names as `hytte pane set`: a `[[script_pane]]` or an approved pack pane (since 1.9.3).

**Windows PowerShell 5.1 notes.** Pipe JSON in (as above) rather than passing it with `--text`: 5.1 drops the double quotes inside an argument to a native program. Set `$OutputEncoding = [Text.UTF8Encoding]::new($false)` first so non-ASCII text arrives as UTF-8 and without a byte-order mark; a BOM in front of the JSON makes it fail the `--json` check. PowerShell 7 needs neither.

## The pane cap, slots and switches

At most **12** custom panes exist at once (`MAX_SCRIPT_PANES`; it was 4 before 1.9.3). The `[[script_pane]]`s come first, in file order, then the **approved** packs, in folder-name order (case-insensitive), while there is room. A pack whose tab name is already taken (by a config pane or an earlier pack, any case) is skipped, and so is any pack that does not fit.

Each pane sits in a numbered **slot** (1 to 12) in that combined list. Two settings belong to the slot:

```toml
[panes]
script2 = false                      # written only when a pane is switched off; true is the default
order = "home,script3,media,script1" # the tab order; script1 .. script12 name the slots
```

The switches are `[panes] script1` to `script12`; the tab order uses the same names in `[panes] order` (panes not named keep their usual place after those that are). Settings → Panes has a row with a switch and move arrows for each pane, labelled with its name. A switched-off pane has no tab and does not repeat its command (it still runs once at start).

Slots shift when a pane is deleted or a pack is approved ahead of another. So that the next pane does not inherit the switch and place, Hytte remembers which name sat in which slot (`pane-slots.json` in the data folder) and, when the names change, moves each pane's switch and place with it (`packs::remap_slots`; it rewrites the `[panes]` keys for you). A new pane starts on and last in the order.

**Deleting a pack's folder removes its pane at once.** A directory watcher on `%APPDATA%\Hytte\panes` (including to the Recycle Bin, which is a rename) rescans after any change and an open Settings window drops the row too. Edits to a pack's `pane.toml` are noticed within a moment, so a changed file asks for approval again without a restart. (Edits to the scripts it runs are not watched: the next run just uses them.)

## How it looks

- **Tab icon.** Every custom pane uses the same generic apps glyph (a grid), not a code glyph; the label is the pane's name, shown on the current tab. The tab strip squeezes the icons together (down to about 15 px each) when many panes are open, and hovering an icon shows the name.
- **Pane size.** 380 px wide; height from the rows, buttons and the footer line.
- **Footer.** One quiet line: freshness, the schedule, or the problem, and the refresh button for panes with a command.

## Debugging a pane

1. **Run the command by hand, as Hytte does.** Open a terminal in the pack folder (a `[[script_pane]]` runs in your home folder) and run the same line with `cmd`:

   ```powershell
   cd "$env:APPDATA\Hytte\panes\docker"
   cmd /d /s /c "powershell -NoProfile -ExecutionPolicy Bypass -File containers.ps1"
   ```

2. **Check the JSON parses and has the shape you meant:**

   ```powershell
   $out = cmd /d /s /c "powershell -NoProfile -ExecutionPolicy Bypass -File containers.ps1"
   $j = $out | ConvertFrom-Json          # an error here is a syntax mistake
   $j.rows.Count                          # more than 6 are dropped
   $j.rows | Format-Table text, value, progress, tone, action, arg
   ```

3. **Test an action with its arg:**

   ```powershell
   $env:HYTTE_ARG = "web-1"
   cmd /d /s /c "powershell -NoProfile -ExecutionPolicy Bypass -File restart.ps1"; $LASTEXITCODE
   ```

4. **Read the footer.** `Exited with code N` means run it by hand and look at the exit code; `Stopped after 10 s` means the command is too slow (do the slow work elsewhere and push the result, or lower what it asks for). Raw JSON on the pane means it did not parse.
5. **Pack not showing?** The card may have timed out (restart Hytte or touch `pane.toml`), it may have been Ignored (`pack-answers\<folder>.ignored`), the pane may have no room (12 cap) or a clashing name, or `pane.toml` is Bad (a card named the problem; pack problems are not written to `hytte.log`). A name clash is silent.
6. **`hytte doctor`** checks the setup and `config.toml` (it reports a typo in a `[[script_pane]]` or `[[action]]` key as a failure) but does not look at packs.

## Limits at a glance

| What | Limit |
|---|---|
| Custom panes (config first, then approved packs) | 12 |
| Tab label | 16 characters |
| `every` | at least 30 s (default 300 s) |
| A pane command runs | 10 s |
| Output read | 64 KB per stream |
| Text pane | 6 lines, 2048 bytes |
| Rows | 6 |
| Row `text` / `value` | 80 / 24 characters |
| Row `progress` | 0 to 100 (clamped) |
| Buttons | 3, label 16 characters |
| `arg` | 200 characters, string only, `& \| < > ^ " %` and the backtick removed |
| Action name | letters, digits, `-`, `_`; 40 characters in `config.toml`, 19 in a pack (20 + 1 + 19 with the prefix) |
| Actions in a pack | 8 |
| Actions in `config.toml` | 32 |
| Action `timeout` | default 60 s, 1 to 600 s |
| "Sure?" window | 3 s |
| Action result line | first line, 80 characters |
| Pushed text | 4096 bytes (`pane_set`), name 64 bytes |
| Pane name via `hytte pane set` | any case; the pane must exist and, for a pack, be approved |

## Worked examples

All four run as written. The scripts print UTF-8 (`[Console]::OutputEncoding`), which keeps non-ASCII text intact. Each pack goes in its own folder under `%APPDATA%\Hytte\panes\` and asks for approval the first time.

### 1. A plain-text pane

`panes\uptime\pane.toml`:

```toml
name = "Uptime"
command = "powershell -NoProfile -ExecutionPolicy Bypass -File uptime.ps1"
every = 60
```

`panes\uptime\uptime.ps1`:

```powershell
# Three plain lines for the pane: uptime, who is signed in, free memory.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$os = Get-CimInstance Win32_OperatingSystem
$up = (Get-Date) - $os.LastBootUpTime
"Up {0}d {1}h {2}m" -f $up.Days, $up.Hours, $up.Minutes
"User  $env:USERNAME on $env:COMPUTERNAME"
"Free  {0:N1} of {1:N1} GB" -f ($os.FreePhysicalMemory / 1MB), ($os.TotalVisibleMemorySize / 1MB)
```

The output is three lines, so it is a text pane. Because it does not start with `{` it is never read as JSON.

### 2. Rows with progress bars and tones

`panes\load\pane.toml`:

```toml
name = "Load"
command = "powershell -NoProfile -ExecutionPolicy Bypass -File load.ps1"
every = 30
```

`panes\load\load.ps1`:

```powershell
# CPU, memory and system-drive use as rows with bars. Amber from 70 %, red from 90 %.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
function Tone($p) { if ($p -ge 90) { "bad" } elseif ($p -ge 70) { "warn" } else { "good" } }
$cpu = [int](Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average
$os = Get-CimInstance Win32_OperatingSystem
$mem = [int](100 * ($os.TotalVisibleMemorySize - $os.FreePhysicalMemory) / $os.TotalVisibleMemorySize)
$sys = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='$env:SystemDrive'"
$disk = [int](100 * ($sys.Size - $sys.FreeSpace) / $sys.Size)
$rows = @(
    @{ text = "CPU";    value = "$cpu %";  progress = $cpu;  tone = Tone $cpu },
    @{ text = "Memory"; value = "$mem %";  progress = $mem;  tone = Tone $mem },
    @{ text = "Disk $env:SystemDrive"; value = "$disk %"; progress = $disk; tone = Tone $disk }
)
@{ rows = $rows } | ConvertTo-Json -Depth 4 -Compress
```

Real output looks like `{"rows":[{"progress":3,"tone":"good","value":"3 %","text":"CPU"}, ...]}`. `-Compress` keeps it on one line and `-Depth 4` stops PowerShell flattening nested objects. Wrap a single row in `@(...)` so it is still an array.

### 3. An action with confirm and arg

A pane that lists Notepad windows. A click on a row closes that one (after "Sure?"), and a button closes all of them.

`panes\notepads\pane.toml`:

```toml
name = "Notepads"
command = "powershell -NoProfile -ExecutionPolicy Bypass -File list.ps1"
every = 30

[[action]]
name = "close"                    # Hytte calls it notepads-close
command = "powershell -NoProfile -ExecutionPolicy Bypass -File close.ps1"
confirm = true

[[action]]
name = "close-all"
command = "powershell -NoProfile -ExecutionPolicy Bypass -File close-all.ps1"
confirm = true
```

`panes\notepads\list.ps1`:

```powershell
# One row per running Notepad. Click a row to close it (after "Sure?"); the button closes all.
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
$app = "notepad"
$procs = @(Get-Process -Name $app -ErrorAction SilentlyContinue | Select-Object -First 6)
if (-not $procs) {
    @{ rows = @(@{ text = "No Notepad windows"; tone = "good" }) } | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
$rows = foreach ($p in $procs) {
    @{
        text   = "$app  (pid $($p.Id))"
        value  = "{0:N0} MB" -f ($p.WorkingSet64 / 1MB)
        action = "close"
        arg    = [string]$p.Id
    }
}
@{
    rows    = @($rows)
    buttons = @(@{ label = "Close all"; action = "close-all" })
} | ConvertTo-Json -Depth 4 -Compress
```

`panes\notepads\close.ps1`:

```powershell
# Close one Notepad. The pid comes from Hytte as $env:HYTTE_ARG: treat it as untrusted text.
$app = "notepad"
if ($env:HYTTE_ARG -notmatch '^\d{1,10}$') { "not a pid"; exit 1 }
$p = Get-Process -Id ([int]$env:HYTTE_ARG) -ErrorAction SilentlyContinue
if (-not $p -or $p.ProcessName -ne $app) { "that is not a $app"; exit 1 }
Stop-Process -Id $p.Id
"closed pid $($p.Id)"
```

`panes\notepads\close-all.ps1`:

```powershell
$n = @(Get-Process -Name notepad -ErrorAction SilentlyContinue)
$n | Stop-Process
"closed $($n.Count)"
```

Things to notice: the arg is a string (`[string]$p.Id`), because a number would be ignored; `close.ps1` checks the value is a pid *and* that the process is a Notepad before doing anything, so a wrong or hostile arg can only close a Notepad; the result card says `notepads-close finished` with `closed pid 4242` under it, or `notepads-close failed (exit 1)` with the reason.

### 4. A push-only pane fed by a script

Declare the pane in `config.toml` with no command (a pack cannot be push-only):

```toml
[[script_pane]]
name = "Builds"
command = ""
```

`C:\tools\build.ps1` keeps it up to date while a build runs:

```powershell
# build.ps1: runs the steps of a build and keeps the "Builds" pane up to date as it goes.
$OutputEncoding = [Text.UTF8Encoding]::new($false)   # Windows PowerShell 5.1: UTF-8 without a byte-order mark

function Show-Builds($rows) {
    (@{ rows = @($rows) } | ConvertTo-Json -Depth 4 -Compress) | hytte pane set "Builds" --json
}

$steps = "restore", "compile", "test", "package"
for ($i = 0; $i -lt $steps.Count; $i++) {
    Show-Builds @{ text = $steps[$i]; value = "step $($i + 1) of $($steps.Count)"
                   progress = [int](100 * $i / $steps.Count); tone = "warn" }
    Start-Sleep 2                                    # your real work goes here
}
Show-Builds @{ text = "build"; value = "done"; progress = 100; tone = "good" }
```

Run it from a terminal or a scheduled task. The pane shows `Nothing sent yet` and the exact `hytte pane set` line until the first push, then "Updated just now". A pushed pane never runs anything on its own: its content is whatever was sent last, and it has no refresh button.

Next: [How-to recipes](14-how-to-recipes.md) for changing Hytte itself, or [What's new](16-whats-new.md).
