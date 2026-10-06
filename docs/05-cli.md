# 5. The `notch` CLI and shell integrations

`notch.exe` is the client half of Hytte. It is built from `crates/hytte/src/cli/` as a second `[[bin]]` of the `hytte` package. It runs in terminals, shell prompts and agent hooks, often on every command, so it has to be **fast to start** and **must never break the caller**.

## Design rules

1. **Best effort.** If the daemon isn't running, `notch` says so at most once and carries on. `notch run` still runs your command, and shell hooks stay completely silent.
2. **No GUI side effects from hooks.** Only `notch run` may start the daemon; hooks never spawn windows.
3. **Small and quick.** The CLI uses only `std`, `windows`, `serde_json` and `hytte-proto`, with no runtime to initialise.
4. **Exit codes are preserved.** `notch run` exits with the wrapped command's code.

## How a message is sent

`send_msg` serialises a `HytteMessage` with `to_line()` (JSON plus `\n`) and opens `\\.\pipe\hytte` as a file for writing with `FILE_FLAG_WRITE_THROUGH`. It writes the line and closes the pipe. If the open fails (no daemon), it returns `false`. There is no retry and no wait, apart from `notch run`'s auto-start described below.

## Subcommands

Dispatch is in `cli/main.rs::main`.

### `notch run [--no-progress] -- <cmd> [args...]` (`cmd_run`)

1. Creates a task id (`<pid>-<unix ms>`) and a label (the command line, truncated to 128 characters).
2. Sends `Start` with `pid` = notch's own pid. If that fails and a `hytte.exe` exists **next to `notch.exe`**, it starts it detached (`DETACHED_PROCESS | CREATE_NO_WINDOW`), waits up to 3 s for the pipe to appear, and sends `Start` again.
3. Spawns the command with **stdin and stdout inherited** (the terminal behaves exactly as normal) and **stderr piped**.
4. Copies stderr to its own stderr as **raw bytes**, as they arrive, so `\r`-redrawn progress bars draw live and output that isn't UTF-8 passes through. Alongside, it keeps the last 5 lines in a ring buffer; for a redrawn line, only the text after its last `\r`, which is what the terminal showed.
   - Unless `--no-progress` is given, each chunk also goes through `progress::Scanner`, which reports the newest percentage it finds. It understands **OSC 9;4** sequences (`ESC ] 9 ; 4 ; 1 ; <pct>`, the ones Windows Terminal shows as a tab progress bar) and plain **`NN%`** text (1–3 digits, optional fraction, at most 100) in the current line. A new value is sent as a `Progress` message, at most every 250 ms and only when it changed. Sending stops for the rest of the run if the daemon is gone. The scanner's buffers are capped (512-byte line, 32-byte OSC body).
   - Whatever the setting, the newest thing the command printed is sent too, as the message's `line`: the line being drawn, else the last whole one, run through `progress::clean_line` (only the text after the last `\r`, escape sequences and control characters removed, capped at 120 characters). It goes at most every 250 ms (together with a percentage when both changed) and only when it differs from the last one sent. The expanded task row shows it live under the label, so you can see `Compiling foo v0.3.1` without opening the terminal. Only stderr is captured, as before.
   - Trade-off: a percentage in an ordinary log line (`coverage: 73%`) also moves the bar; `--no-progress` opts out. Tools that hide progress when stderr isn't a terminal need their own flag, such as `git clone --progress`. Running the child in a pseudo-console (ConPTY) would remove that limit.
5. On exit, sends `Done` (exit code 0) or `Failed` with the exit code, the duration and the stderr tail.
6. Exits with the child's exit code. If the command couldn't be spawned, it reports `Failed` with "spawn failed: …" and exits `127`.

Because the task's `pid` is the `notch` process, closing the terminal turns the task "lost" (see [section 4](04-ipc-protocol.md)), and clicking the row focuses the terminal hosting `notch`.

### `notch set --progress N --label L [--task ID]` (`cmd_set`)

Sends one `Progress` message with `progress = N` (0–100). Use the same `--task` on every call to update one entry; without it, every call creates a new task. No `pid` is sent, so the 30 s silence rule applies.

### `notch agent <start|needs-input|resume|tool|done|fail> [--name N] [--pid P] [--message M] [--detail D]` (`cli/agent.rs`)

| Subcommand | Event |
|---|---|
| `start` | `Start` |
| `needs-input` | `NeedsInput` |
| `resume` | `Resumed` |
| `done` | `Done` |
| `fail` | `Failed` |

- `--name` defaults to `Agent` and becomes the label.
- `--pid` defaults to `hytte_proto::sys::owner_pid(own pid)`. That climbs the process tree, skipping up to six throw-away shells (`cmd`, `sh`, `bash`, `zsh`, `fish`, `pwsh`, `powershell`, `nu`, `conhost`), to find the long-lived agent or terminal behind a hook. Agent hooks usually run as `agent → shell → notch`, so the shell's parent is what you want.
- The task id is `agent:<name>:<pid>`, so all of one agent's events update a single row.
- `needs-input` without `--message` reads **stdin** when it isn't a terminal (up to 64 KiB), parses it as JSON, and uses its `message` field (first 300 characters). That is exactly the payload Claude Code's `Notification` hook pipes in.

`notch agent tool` reports what the agent is doing *right now*, as a `Progress` update whose `line` is a short phrase. With `--detail "text"` the text is used as is; with no `--detail` it reads a Claude Code `PreToolUse` payload (`{"tool_name": ..., "tool_input": ...}`) from stdin and `describe_tool` turns it into a phrase: `Editing render.rs`, `Reading lib.rs`, `Running: cargo build`, `Searching: fn main`, `Subagent: ...`, `Using create_issue` (MCP tools), capped at 120 characters. If neither is available it sends nothing. Each agent keeps its own row (the task id is `agent:<name>:<pid>`), so two agents show two rows with their own step and elapsed time.

`notch agent hooks claude` prints a ready-made Claude Code hooks block:

| Claude Code hook | Runs |
|---|---|
| `UserPromptSubmit` | `notch agent resume --name "Claude Code"` |
| `Notification` | `notch agent needs-input --name "Claude Code"` |
| `Stop` | `notch agent done --name "Claude Code"` |

`notch agent hooks claude --tools` prints the same block plus a `PreToolUse` hook (`notch agent tool --name "Claude Code"`, matcher `*`) for the live view. It is opt-in because it runs `notch` before every tool call; `notch` starts in a few milliseconds, but it is still a process per call.

### `notch ports [--all]` and `notch kill :PORT [--force]` (`cli/portscmd.rs`)

These talk to the OS directly through `hytte_proto::ports` and work without the daemon.

- `ports` lists listeners on the default watch list (the same ports as the config default), or every non-system listener with `--all`.
- `kill` finds the listener on that port (from all listeners), asks `Stop node.exe (pid 1234) listening on :3000? [y/N]` unless `--force` is given, then terminates the process. Pids 0–4 and the CLI's own pid are refused.

### `notch setup [--undo]` (`setup.rs`)

Makes `notch` work without manual steps. `setup.rs` is shared with the daemon: `cli/main.rs` includes it with `#[path = "../setup.rs"]`, and the tray's **Set up terminal integration** calls the same functions.

1. **PATH** (`ensure_on_path`): adds the folder holding `notch.exe` to the user's `HKCU\Environment\Path`, keeping the value's type (`REG_EXPAND_SZ` or `REG_SZ`), then broadcasts `WM_SETTINGCHANGE "Environment"` on a background thread so newly opened terminals see it. It does nothing if `notch.exe` already resolves on `PATH` (for example a winget alias) or under `cargo run`. The added folder is recorded in `%APPDATA%\Hytte\path.txt`, and if Hytte has since moved, the stale entry is removed. Entries compare case-insensitively, ignoring quotes and a trailing `\`, after `%VAR%` expansion (`path_with`, `path_without`).
2. **Profiles** (`setup_shells`): adds a block between `# >>> hytte >>>` and `# <<< hytte <<<` to the Windows PowerShell 5.1 profile, the PowerShell 7 profile (if PowerShell 7 is installed) and `~/.bashrc` (if Git Bash is installed). The PowerShell line only runs if `notch` exists, so uninstalling never breaks the profile. The bash block is inserted **above** any `starship init` line (Starship chains an existing DEBUG trap); the PowerShell block is appended, so it loads **after** Starship (which replaces `prompt`). Files keep their encoding (UTF-8, UTF-8 with BOM, UTF-16 LE) and line endings, and are written through a temp file, except through symlinks. A second run changes nothing; `--undo` removes the blocks.
3. If Windows PowerShell's execution policy is `Restricted` or `AllSigned`, the output says how to allow profiles (`Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`). `notch setup` never changes the policy. The tray item asks first and, on Yes, writes `RemoteSigned` for the current user only (`allow_ps51_profiles`, the registry value that command sets).

A shell counts as set up only when **every** detected profile has the block, so a profile you added earlier by hand (say Git Bash) doesn't stop setup from adding the others.

The daemon also runs step 1 at every startup unless `[general] add_to_path = false`.

### `notch init <pwsh|bash|zsh|nu>` and `notch hook <start|end> ...` (`cli/hook.rs`)

`init` prints a shell integration script. The scripts live in `cli/shell/` and are embedded into the binary with `include_str!`, so the installed `notch.exe` is always in sync with its scripts.

`hook start --id I --cmd C --pid P [--cwd D]` and `hook end --id I --code N [--duration-ms MS]` are what the bash, zsh and Nushell scripts call. They build `source = "shell"` messages (`end` maps code 0 to `Done` and anything else to `Failed`) and always exit successfully, even if the daemon is down. `--cmd` is truncated to 512 characters.

## How the shell integrations work

All four scripts follow the same idea. Send a **start** event when a command is about to run, and an **end** event when the prompt comes back. The daemon applies the visibility threshold ([section 4](04-ipc-protocol.md)), so the scripts report *every* command and stay simple.

| Shell | "Command starts" hook | "Prompt returns" hook | How it sends |
|---|---|---|---|
| PowerShell (`init.ps1`) | A PSReadLine **Enter** key handler that reads the buffer, then accepts the line | A wrapped `prompt` function | Writes to the pipe **directly** with `NamedPipeClientStream` (40 ms connect timeout): no process spawn per command |
| bash (`init.bash`) | `trap … DEBUG` (first command of each line only; skips completion and its own functions) | `PROMPT_COMMAND` | `notch hook …` in a background subshell |
| zsh (`init.zsh`) | `preexec` via `add-zsh-hook` | `precmd` | `notch hook …` in a background subshell |
| Nushell (`init.nu`) | `hooks.pre_execution` | `hooks.pre_prompt` | `notch hook …` via `job spawn` |

Details worth knowing:

- Each script guards itself (`__notch_loaded`, `$global:__notch`) so sourcing it twice does nothing.
- Under Git Bash / MSYS, `$$` is not the Windows pid. The bash and zsh scripts read `/proc/$$/winpid` so click-to-focus and liveness use the real Windows process.
- PowerShell computes the exit code from `$?` and `$LASTEXITCODE`, and the duration with a `Stopwatch`. bash and zsh use whole seconds. Nushell uses `CMD_DURATION_MS`.
- The PowerShell integration replaces any existing custom **Enter** binding.
- Shell tasks have no stderr tail (the shell owns the command's streams), so failures show `exit N`.

## Changing the CLI

- Keep startup cheap: no new heavy dependencies, no config file reads, no network.
- Anything a hook calls must exit 0 and print nothing, whatever happens.
- When you change a script in `cli/shell/`, rebuild `notch.exe`. Users get the new script the next time their shell starts, because the profile line runs `notch init` each time.
- `cli/hook.rs` has a test that every embedded script mentions `notch`. Add behavioural tests next to the code you change.

Next: [6. The UI model](06-ui-model.md)
