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

### `notch run -- <cmd> [args...]` (`cmd_run`)

1. Creates a task id (`<pid>-<unix ms>`) and a label (the command line, truncated to 128 characters).
2. Sends `Start` with `pid` = notch's own pid. If that fails and a `hytte.exe` exists **next to `notch.exe`**, it starts it detached (`DETACHED_PROCESS | CREATE_NO_WINDOW`), waits up to 3 s for the pipe to appear, and sends `Start` again.
3. Spawns the command with **stdin and stdout inherited** (the terminal behaves exactly as normal) and **stderr piped**.
4. Forwards each stderr line straight to its own stderr, keeping the last 5 lines in a ring buffer.
5. On exit, sends `Done` (exit code 0) or `Failed` with the exit code, the duration and the stderr tail.
6. Exits with the child's exit code. If the command couldn't be spawned, it reports `Failed` with "spawn failed: …" and exits `127`.

Because the task's `pid` is the `notch` process, closing the terminal turns the task "lost" (see [section 4](04-ipc-protocol.md)), and clicking the row focuses the terminal hosting `notch`.

### `notch set --progress N --label L [--task ID]` (`cmd_set`)

Sends one `Progress` message with `progress = N` (0–100). Use the same `--task` on every call to update one entry; without it, every call creates a new task. No `pid` is sent, so the 30 s silence rule applies.

### `notch agent <start|needs-input|resume|done|fail> [--name N] [--pid P] [--message M]` (`cli/agent.rs`)

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

`notch agent hooks claude` prints a ready-made Claude Code hooks block:

| Claude Code hook | Runs |
|---|---|
| `UserPromptSubmit` | `notch agent resume --name "Claude Code"` |
| `Notification` | `notch agent needs-input --name "Claude Code"` |
| `Stop` | `notch agent done --name "Claude Code"` |

### `notch ports [--all]` and `notch kill :PORT [--force]` (`cli/portscmd.rs`)

These talk to the OS directly through `hytte_proto::ports` and work without the daemon.

- `ports` lists listeners on the default watch list (the same ports as the config default), or every non-system listener with `--all`.
- `kill` finds the listener on that port (from all listeners), asks `Stop node.exe (pid 1234) listening on :3000? [y/N]` unless `--force` is given, then terminates the process. Pids 0–4 and the CLI's own pid are refused.

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
