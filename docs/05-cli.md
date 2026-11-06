# 5. The `hytte` CLI and shell integrations

`hytte <command>` is the client half of Hytte: the same `hytte.exe` started with arguments ([section 1](01-overview.md#one-program-two-modes)). Code: `crates/hytte/src/cli/`. `main()` hands arguments to `cli::run` before the daemon's logging, mutex or config are touched. It runs in terminals, prompts and agent hooks, often on every command, so it must be **fast to start** and **never break the caller**.

## Design rules

1. **Best effort.** With no daemon, `hytte` says so at most once. `hytte run` still runs your command; hooks stay silent.
2. **No GUI side effects from hooks.** Only `hytte run` may start the daemon.
3. **Small and quick.** Only `std`, `windows`, `serde_json` and `hytte-proto`; no runtime to initialise.
4. **Exit codes preserved.** `hytte run` exits with the wrapped command's code.

## How a message is sent

`send_msg` serialises a `HytteMessage` with `to_line()` (JSON plus `\n`, text trimmed to the protocol limits) and opens this user's pipe ([section 4](04-ipc-protocol.md#the-pipe)) for writing with `FILE_FLAG_WRITE_THROUGH`. `open_pipe` also: retries a **busy** pipe every 20 ms for about 200 ms (a hook burst doesn't drop messages); fails at once on a **missing** pipe (`send_msg` returns `false`); and refuses a pipe whose server runs as another user. `hytte run` starts a daemon only when the pipe is missing, and keeps sending through a busy spell.

## Subcommands

`cli/mod.rs::run` dispatches. `hytte help` (`-h`, `--help`) lists commands and `hytte --version` (`-V`) prints the version, both exit 0; an unknown command prints the list to stderr and exits 2.

### `hytte run [--no-progress] -- <cmd> [args...]`

1. Makes a task id (`<pid>-<unix ms>`) and a label: the command line with each argument quoted for PowerShell (`quote_arg`), because **Rerun** runs the label through PowerShell. Over 1000 bytes it is cut and ends in `…` (`fit_label`); no Rerun for a cut one.
2. Sends `Start` with the CLI's own pid. If that fails it starts the daemon (this exe, no arguments, `DETACHED_PROCESS | CREATE_NO_WINDOW`, **null stdio** so a pipe the caller reads, like `hytte run -- x | tee`, isn't held open), waits up to 3 s for the pipe, and sends again.
3. Finds the program (`find_program`): a bare name is searched on `PATH` with `PATHEXT`, so `npm`, `yarn` and other `.cmd` / `.bat` work (Rust's own lookup only finds `.exe`). `Command` escapes arguments for batch files; one it refuses gives "an argument can't be passed safely to a .cmd/.bat file". The child gets **stdin and stdout inherited** and **stderr piped**.
4. Copies stderr to its own stderr as **raw bytes** as they arrive (so `\r` progress bars draw live) and keeps the last 5 lines in a ring buffer (for a redrawn line, the text after its last `\r`).
   - Unless `--no-progress`, `progress::Scanner` reports the newest percentage: **OSC 9;4** sequences (Windows Terminal's tab progress), plain **`NN%`** in the current line (1–3 digits, ≤ 100), and **bracketed counts** like `[12/345]` or `(3/25)` (dates and paths never match). No time estimates: a command with no numbers keeps its spinner. Sent as `Progress` at most every 250 ms and only on change, and stops for the run if the daemon is gone. Buffers are capped (512-byte line, 32-byte OSC body).
   - Always, the newest output goes as `line` (the drawn line, else the last whole one, through `progress::clean_line`: text after the last `\r`, escapes and control characters removed, ≤ 120 chars), at most every 250 ms and only when changed. The expanded task row shows it under the label. Only stderr is captured.
   - Trade-off: a percentage in an ordinary log line (`coverage: 73%`) also moves the bar (`--no-progress` opts out). Tools that hide progress when stderr isn't a terminal need their own flag (`git clone --progress`). A pseudo-console (ConPTY) would remove that limit.
5. On exit sends `Done` (code 0) or `Failed` with code, duration and stderr tail, and exits with the child's code. If it can't spawn, it reports `Failed` ("spawn failed: …") and exits `127`.

The task's `pid` is the `hytte run` process, so closing the terminal makes the task "lost" ([section 4](04-ipc-protocol.md)) and clicking the row focuses its terminal.

### `hytte set (--progress N | --done | --fail) --label L [--task ID]`

`--progress N` sends one `Progress` (0–100). `--done` / `--fail` finish the row green, or red with exit 1. Reuse `--task` to update one entry; without it every call is a new task. No `pid` is sent, so the 30 s silence rule applies.

### `hytte agent <start|needs-input|resume|tool|ask|done|fail|hooks> [--name N] [--pid P] [--message M] [--detail D] [--ack]` (`cli/agent.rs`)

| Subcommand | Event |
|---|---|
| `start` / `needs-input` / `resume` / `fail` | `Start` / `NeedsInput` / `Resumed` / `Failed` |
| `done` | `Done`; the agent's reply (`--message`, else `last_assistant_message` / `prompt_response` from the hook's stdin JSON, else the last assistant text in its `transcript_path`, ≤ 4000 chars) becomes a card with its first line and **Copy**, held for `[general] finish_peek_secs` (0: no card) |

- `--name` defaults to `Agent` and is the label. The task id is `agent:<name>:<pid>`, so one agent's events update one row.
- `--pid` defaults to `hytte_proto::sys::owner_pid(own pid)`, which climbs the process tree past up to six throw-away shells (`cmd`, `sh`, `bash`, `zsh`, `fish`, `pwsh`, `powershell`, `nu`, `conhost`) to the long-lived agent or terminal.
- `needs-input` without `--message` reads **stdin** (when not a terminal, ≤ 64 KiB) as JSON and uses its `message` (first 300 chars): the payload Claude Code's and Gemini CLI's `Notification` hooks send.
- `--ack` prints `{}` when done: Gemini CLI, Antigravity and Cursor read hook stdout as JSON.

`hytte agent tool` reports what the agent is doing *right now* as a `Progress` update whose `line` is a short phrase. `--detail "text"` is used as is; otherwise it reads a Claude Code `PreToolUse` payload from stdin and `describe_tool` makes `Editing src/render.rs`, `Reading lib.rs`, `Running: cargo build`, `Searching: fn main`, `Subagent: ...`, or `Using github: create_issue {...}` for MCP tools (≤ 120 chars; a command's first non-blank line). With neither, it sends nothing. Two agents show two rows.

`hytte agent hooks claude` prints the hooks block `hytte setup agents` installs (`agents::claude_hooks`), for pasting by hand. The pre-1.1 `--tools` and `--approve` flags are accepted and do nothing.

### `hytte agent ask [--name N] [--pid P]`

Answers an agent's permission prompt from the pill. Claude Code and Codex call it from `PermissionRequest`; the OpenCode plugin from `permission.ask`.

1. Reads the payload from stdin; `ask_text` makes one line (the `describe_tool` phrase or `Using <tool>`). An `AskUserQuestion` (the agent asking *you*, not for permission) has no yes/no: it sends `NeedsInput` with the question and prints nothing.
2. A multi-line Bash command starts the line with `(N lines, see terminal)`; over 60 characters, `(long, see terminal)`. The chip clips the end, so the warning goes first.
3. Sends an [`Ask`](04-ipc-protocol.md#asking-the-human-ask-and-answer) over a read-write connection (`ask_pipe`) and waits up to `ASK_WAIT_SECS` + 2 s.
4. On **Allow** / **Deny** prints the decision. The pill ignores clicks for 400 ms after an ask appears (`ASK_CLICK_GUARD`), so a stray double-click can't answer it or the next one:

```json
{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}
```

A deny adds `"message": "Denied from Hytte"` (read as the reason). With no answer it prints nothing and exits 0, and the agent shows its normal terminal prompt, which only appears after the hook returns. So the daemon hangs up as soon as it knows there will be no answer: chip dismissed, Hytte paused or in game mode, or 30 s elapsed. Hooks are installed with a `timeout` above that wait.

### `hytte setup agents [--undo]` (`agents.rs`)

Connects the agents installed for this user (the tray's **Connect AI agents** calls `agents::connect`). Each is found by its config folder and gets Hytte's hooks merged into its file:

| Agent | File | Hooks → `hytte agent …` |
|---|---|---|
| Claude Code | `~/.claude/settings.json` (or `%CLAUDE_CONFIG_DIR%`) | `UserPromptSubmit` → `resume`, `PreToolUse` → `tool`, `PermissionRequest` → `ask`, `Notification` (`permission_prompt\|elicitation_dialog`) → `needs-input`, `Stop` → `done` |
| Codex | `~/.codex/hooks.json` (or `%CODEX_HOME%`) | the same without `Notification`; the user trusts them once with `/hooks` |
| OpenCode | `~/.config/opencode/plugins/hytte.js` | embedded plugin (`agent_plugin.js`): `session.status` busy → `resume`, `permission.asked` → `needs-input`, `permission.replied` → `resume`, `session.idle` → `done` (sub-agents skipped), `permission.ask` → `ask` |
| Gemini CLI | `~/.gemini/settings.json` | `BeforeAgent` → `resume`, `Notification` → `needs-input`, `AfterAgent` → `done`, all `--ack` |
| Antigravity | `~/.gemini/config/hooks.json` (shared by `agy`, the IDE, Antigravity 2.0) | named hook `"hytte"`: `PreInvocation` → `resume`, `Stop` → `done`, `--ack` |
| Cursor | `~/.cursor/hooks.json` | `postToolUse` → `resume`, `stop` → `done`, `--ack`; informational only, since Cursor blocks the action if a gating hook's reply doesn't match its schema |

How the merge behaves:

- **Ours vs. theirs.** A handler is Hytte's when its command mentions `hytte` and runs ` agent ` (`agents::ours`), which also catches a block pasted by an older version. Those are replaced by the current set; everything else stays in place (`serde_json` `preserve_order`, `hooks` edited in place). Empty matcher groups and events are dropped. A second run changes nothing.
- **Safety.** A file that isn't plain JSON (comments, say) is left alone and reported. The first write to an existing file saves `<file>.hytte-backup`. Writes go through `setup::write_file` (temp file and rename).
- **Command line.** `exe_cmd` writes this exe's full path with forward slashes (cmd, PowerShell and Git Bash accept it), quoted only if it has a space. No `PATH` lookup.
- **`--undo`** removes Hytte's handlers and deletes files Hytte created that hold nothing else.
- **Moved exe.** At startup `agents::repair` rewrites entries only for files whose Hytte entries *all* point at an exe that no longer exists, so a second copy (a dev build) doesn't take over working entries.

Daemon side: a row is `agent:<name>:<pid>` with `pid` from `owner_pid`, so liveness and click-to-focus follow the agent. A shell command named in `ui_state::AGENT_CLIS` (`claude`, `codex`, `opencode`, `gemini`, `agy`, `cursor-agent`) isn't shown as a terminal task, since the agent reports itself. `NeedsInput` and a pending ask are forced visible over a maximised window or fullscreen video, but not in game mode.

### `hytte ports [--all]` and `hytte kill :PORT [--force]` (`cli/portscmd.rs`)

Use the OS directly through `hytte_proto::ports`; no daemon needed. `ports` lists the default watch list, or every non-system listener with `--all`. `kill` finds the listener, asks `Stop node.exe (pid 1234) listening on :3000? [y/N]` unless `--force`, then terminates it. Pids 0–4, the CLI's own pid, and a pid that no longer belongs to the listed exe (`ports::kill(pid, exe)`; pids get reused) are refused. The pill's Stop and Kill buttons apply the same check.

### `hytte notify "text"` and `hytte shelf add`

One-way `Cmd` requests ([section 4](04-ipc-protocol.md#script-commands-notify-and-shelf-add)); both exit 1 with "daemon not running" when there is no pill.

- `hytte notify "Build finished"` shows a card (words joined; over 1024 bytes cut). Nothing shows while paused or in game mode.
- `hytte shelf add <file>...` shelves files (made absolute; a missing file exits 2). With no files and text piped in, the text becomes a snippet: `git diff | hytte shelf add` (≤ 16 KiB; more exits 2 and says to use a file).

### `hytte doctor` (`cli/doctor.rs`)

One line per check: `ok`, `warn` or `FAIL`, with a `fix:` line under the last two; exits 1 if anything failed.

| Check | Problem when | Fix |
|---|---|---|
| Version and exe path | never | |
| Pill running | pipe missing, or held by another user | run `hytte` |
| `hytte` on `PATH` | not found on this terminal's `PATH` | `hytte setup`, new terminal |
| Command tracking | warns: a detected PowerShell / Git Bash profile lacks the block | `hytte setup` |
| PowerShell execution policy | Windows PowerShell has the hook but `Restricted` / `AllSigned` won't load profiles | `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` |
| Each agent | warns: not connected; fails: hooks point at a missing `hytte.exe` | `hytte setup agents` |
| `config.toml` | `config::problem` finds a mistake or unknown setting | edit the file |

`render` and `exit_code` are pure and tested; `checks` gathers facts (`setup::shell_status`, `setup::ps51_blocked`, `agents::status`).

### `hytte setup [--undo]` (`setup.rs`)

The CLI, daemon startup and the tray's **Set up terminal integration** share these functions.

1. **PATH** (`ensure_on_path`): adds `hytte.exe`'s folder to `HKCU\Environment\Path` (keeping `REG_EXPAND_SZ` / `REG_SZ`), then broadcasts `WM_SETTINGCHANGE "Environment"` on a background thread. It does nothing if `hytte.exe` already resolves on `PATH` (a winget alias, say) or under `cargo run`. The added folder is recorded in `%APPDATA%\Hytte\path.txt`; if Hytte later moves, the stale entry is removed (unless that folder still has a `hytte.exe` or old `notch.exe`). Entries compare case-insensitively, ignoring quotes and a trailing `\`, after `%VAR%` expansion.
2. **Profiles** (`setup_shells`): adds a block between `# >>> hytte >>>` and `# <<< hytte <<<` to the PowerShell 5.1 profile, the PowerShell 7 profile (if installed) and `~/.bashrc` (if Git Bash is installed). The PowerShell line runs only if `hytte` exists, so uninstalling never breaks the profile. The bash block goes **above** any `starship init` (Starship chains an existing DEBUG trap); the PowerShell block is appended, so it loads **after** Starship (which replaces `prompt`). Encoding (UTF-8, UTF-8 BOM, UTF-16 LE) and line endings are kept; writes use a temp file except through symlinks. Idempotent; `--undo` removes the blocks.
3. If PowerShell's policy is `Restricted` / `AllSigned`, the output explains how to allow profiles. `hytte setup` never changes the policy; the tray item asks first and on Yes writes `RemoteSigned` for the current user (`allow_ps51_profiles`).

A shell counts as set up only when **every** detected profile has the block. The daemon also runs step 1 at startup unless `add_to_path = false`.

#### Upgrading from `notch`

Before 1.0 the CLI was `notch.exe` and profile blocks ran `notch init ...`; after an upgrade that line would silently stop working (its `Get-Command notch` guard fails). `setup::migrate_block` replaces such a block with the current line in the same position (bash stays above Starship). The daemon runs it for every detected profile at startup (`migrate_profiles`), as does `hytte setup`. Claude Code hooks pasted by hand before 1.1 call bare `hytte` and still work; **Connect AI agents** swaps them for the full-path set.

### `hytte init <pwsh|bash|zsh|nu>` and `hytte hook <start|end> ...` (`cli/hook.rs`)

`init` prints a shell script (`powershell` and `nushell` are aliases). The scripts live in `cli/shell/` and are embedded with `include_str!`, so the exe and its scripts never drift. `.gitattributes` keeps them LF (bash and zsh choke on `\r`).

`hook start --id I --cmd C --pid P [--cwd D]` and `hook end --id I --code N [--duration-ms MS]` are what the bash, zsh and Nushell scripts call. They build `source = "shell"` messages (`end` maps 0 to `Done`, else `Failed`) and always exit successfully. `--cmd` is cut by `fit_label` (1000 bytes).

## How the shell integrations work

Send a **start** event when a command is about to run and an **end** event when the prompt returns. The daemon applies the visibility threshold ([section 4](04-ipc-protocol.md)), so the scripts report *every* command and stay simple.

| Shell | "Command starts" | "Prompt returns" | Sends via |
|---|---|---|---|
| PowerShell (`init.ps1`) | PSReadLine **Enter** handler that reads the buffer, then accepts the line. Enter on an unfinished command (open brace, trailing pipe, here-string) starts nothing: `__notch_incomplete` asks the parser (`IncompleteInput`) and the row starts when the command is whole | wrapped `prompt` function | the pipe **directly** via `NamedPipeClientStream` (40 ms connect timeout; name `hytte-<SID>` built once): no process per command |
| bash (`init.bash`) | `trap … DEBUG` (first command of each line; skips completion and its own functions) | `PROMPT_COMMAND` | `hytte hook …` in a background subshell |
| zsh (`init.zsh`) | `preexec` (`add-zsh-hook`) | `precmd` | `hytte hook …` in a background subshell |
| Nushell (`init.nu`) | `hooks.pre_execution` | `hooks.pre_prompt` | `hytte hook …` via `job spawn` |

- Each script guards against double sourcing (`__notch_loaded`, `$global:__notch`).
- Under Git Bash / MSYS `$$` isn't the Windows pid; bash and zsh read `/proc/$$/winpid` so click-to-focus and liveness use the real process.
- PowerShell computes the exit code from `$?` and `$LASTEXITCODE` and the duration with a `Stopwatch`. A command recorded as **Stopped** (`Get-History -Count 1` `ExecutionStatus`; what Ctrl+C leaves for cmdlets and programs; a failing program is `Completed`, a `throw` is `Failed`) reports 130, which the daemon takes as "stopped by the user". When a cmdlet fails (new `$Error` record) the code is 1, not a stale `$LASTEXITCODE`. bash and zsh use whole seconds; Nushell uses `CMD_DURATION_MS`.
- The PowerShell integration replaces any custom **Enter** binding. Its label is cut at 250 characters (`…`), so four-byte characters still fit 1024 bytes.
- Terminals opened before the 1.6.0 update keep the old script and the old pipe name: open a new terminal.
- Shell tasks have no stderr tail (the shell owns the streams), so failures show `exit N`.

## Changing the CLI

- Keep startup cheap: no heavy dependencies, config reads or network.
- Anything a hook calls must exit 0 and print nothing, whatever happens.
- After changing a script in `cli/shell/`, rebuild: users get it next time their shell starts (the profile line runs `hytte init` each time).
- `cli/hook.rs` tests that every embedded script names `hytte init` and that bash, zsh and Nushell call `hytte hook` (never `notch`). Add behavioural tests next to your change.

Next: [6. The UI model](06-ui-model.md)
