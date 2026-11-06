# 4. IPC protocol and task lifecycle

Everything that shows up as a **task** in the pill arrives over one named pipe. This section covers the wire format, how the daemon defends itself, and what happens to a task from its first message until it disappears.

Code: `crates/proto/src/lib.rs` (the message type), `crates/hytte/src/pipe_server.rs` (the server), `crates/hytte/src/tasks.rs` (the registry), and `Model::apply_task` in `crates/hytte/src/ui_state.rs` (the UI side).

## The pipe

| Property | Value | Why |
|---|---|---|
| Name | `\\.\pipe\hytte-<your user's SID>` (`hytte_proto::pipe_name()`, e.g. `hytte-S-1-5-21-...-1001`) | One pipe per Windows user: two accounts can each run Hytte, and a stranger who creates the plain `hytte` name catches nothing. Before 1.6.0 the name was just `\\.\pipe\hytte`. |
| Direction | Duplex (`PIPE_ACCESS_DUPLEX`) | Task messages are fire-and-forget; only an [`Ask`](#asking-the-human-ask-and-answer) gets a reply line back. |
| Mode | Message mode, blocking | Simple reads; one `ReadFile` per client write. |
| Access control | DACL `D:(A;;GA;;;OW)`: only the owner (your user account) | Other users on the machine can't inject tasks. |
| Remote clients | Rejected (`PIPE_REJECT_REMOTE_CLIENTS`) | Nothing from the network. |
| First instance | Created with `FILE_FLAG_FIRST_PIPE_INSTANCE` | Another process can't squat the name before Hytte starts. |
| Instances | Unlimited (`PIPE_UNLIMITED_INSTANCES`) | Several clients can connect at the same moment, and pending [asks](#asking-the-human-ask-and-answer), which hold a connection for up to 30 s, can't use them all up. |
| Max message | 64 KiB (`MAX_MESSAGE_BYTES`) | Bounded memory per connection. |

The listener loop (`run_windows_loop`) creates a pipe instance, waits in `ConnectNamedPipe`, and hands each connected instance to a **new thread** that reads it until the client disconnects. It then immediately creates the next instance. One slow client can never block another.

Each connection thread (`serve_connection`) reads chunks into a buffer, splits it on `\n`, and parses each non-empty line as JSON. If a single line grows past 64 KiB, the connection is dropped. A line is parsed as a `HytteMessage` first, then as an `Ask`, then as a `Cmd` ([below](#script-commands-notify-and-shelf-add)). A line that fails to parse is ignored without a reply. A `HytteMessage` or `Cmd` that parses but fails validation (a wrong version, a text over its limit) is dropped and a line goes to `hytte.log`.

**Clients check who they are talking to.** The `hytte` CLI opens the pipe through `open_pipe`, which asks Windows who runs the server (`GetNamedPipeServerProcessId`, then that process's token) and refuses the pipe unless it is the same user (`hytte_proto::sys::pipe_server_is_current_user`). So a look-alike pipe made by another account can't read commands or answer a permission request. A busy pipe (every instance taken for a moment, as in a burst of shell hooks) is retried for up to about 200 ms instead of dropping the message; a missing pipe fails at once. `hytte run` starts a daemon only when the pipe is missing, never when it is merely busy.

## Message format

Newline-delimited JSON: one object per line, UTF-8. A client may send several lines on one connection; the `hytte` CLI sends exactly one and disconnects.

```json
{"v":1,"task_id":"agent:Claude Code:4242","event":"NeedsInput","label":"Claude Code",
 "source":"agent","pid":4242,"message":"Approve edit to src/main.rs?"}
```

| Field | Type | Required | Limit | Meaning |
|---|---|---|---|---|
| `v` | number | yes | must equal `1` | Protocol version (`PROTOCOL_VERSION`). |
| `task_id` | string | yes | 1–128 bytes | Identifies the task; later messages with the same id update it. |
| `event` | string | yes | see below | What happened. |
| `label` | string | no (defaults to `""`) | ≤ 1024 bytes | Text shown in the pill. An empty label on an update keeps the previous one. A label ending in `…` was cut by the sender: the pill doesn't offer Rerun for it. |
| `progress` | 0–100 | no | ≤ 100 | Determinate progress. Without it the bar shimmers. |
| `exit_code` | integer | no | | Shown as `exit N` for failed tasks without stderr. |
| `duration_ms` | integer | no | | Shown on finished tasks. |
| `stderr_tail` | string | no | ≤ 8192 bytes | The last lines of stderr, shown under a failed task. |
| `source` | string | no (absent = `"run"`) | ≤ 16 bytes | `"run"`, `"shell"` or `"agent"`. Changes some rules (below). |
| `pid` | integer | no | | The owning process: used for click-to-focus and liveness checks. |
| `delay_ms` | integer | no | | Don't show the task until this long after it starts. |
| `message` | string | no | ≤ 1024 bytes | Free text, shown in amber while `NeedsInput`. |
| `cwd` | string | no | ≤ 1024 bytes | The working directory: where **Rerun** starts the command again. `hytte run` and every shell hook send it. |
| `line` | string | no | ≤ 512 bytes | The command's latest output line (`hytte run` sends it), shown in mono under a running task in the expanded list. Additive: older daemons ignore it. |

**Limits are applied when sending.** `HytteMessage::to_line` first trims every text field to its limit, on a character boundary (`proto::cut`), so a long agent reply, a 20 KB error tail or a long non-English command arrives shortened instead of being rejected. The daemon still validates everything it receives.

`event` is one of:

| Event | Meaning | Pill shows |
|---|---|---|
| `Start` | The task began. | Spinner, elapsed time. |
| `Progress` | An update, usually with `progress`. | Spinner, percentage, filling bar. |
| `NeedsInput` | Blocked waiting for a human. | Amber glow, bell, the `message`. The pill peeks open. |
| `Resumed` | No longer blocked. | Back to the spinner. |
| `Done` | Finished successfully. | Green tick for 3 s, then gone. |
| `Failed` | Finished with an error. | Red cross and stderr until dismissed. |
| `Lost` | The owner vanished without reporting. | Grey `?` for 5 s. Normally produced by the daemon itself. |

### Validation

`HytteMessage::validate` is applied **twice**: once on the connection thread and again in the registry. A message is rejected if the version is wrong, if `task_id` is empty or too long, if `progress > 100`, or if any text field exceeds its limit. All strings are guaranteed UTF-8 by `serde_json`.

### Compatibility rules

- New fields must be **optional**: `#[serde(default, skip_serializing_if = "Option::is_none")]`. Old clients then keep working, and new clients don't send fields an old daemon would reject.
- Unknown fields are ignored by `serde`, so a newer client talking to an older daemon degrades gracefully.
- Bump `PROTOCOL_VERSION` only for a change that old daemons must refuse. Messages with any other version are silently dropped.
- The test `old_messages_still_parse` in `crates/proto/src/lib.rs` pins this behaviour; extend it when you add a field.

## Task id conventions

Each client picks ids so that its own updates land on the same task:

| Client | Id format | Example |
|---|---|---|
| `hytte run` | `<CLI pid>-<unix ms>` | `8120-1730000000000` |
| `hytte set` | a fresh id per call, unless `--task` is given | `upload-1` |
| PowerShell hook | `ps-<shell pid>-<sequence>` | `ps-4242-17` |
| bash / zsh / Nushell hooks | `sh-…`, `zsh-…`, `nu-…` with the same shape | `zsh-901-3` |
| `hytte agent` | `agent:<name>:<owner pid>` | `agent:Claude Code:4242` |

## The task registry

`tasks::spawn_registry` runs on its own thread and is the only place that decides a task has been **lost**.

For each valid message it builds a `TaskState`. `source` defaults to `"run"` and `delay_ms` to 0, and `updated` is stamped with the current time. It then:

- removes the id from its map if the event is `Done`, `Failed` or `Lost` (nothing left to watch), otherwise inserts or replaces it;
- forwards the state to the UI as `TaskUpdate::Upsert`;
- remembers a `Failed` row from a shell hook (`source = "shell"`) together with its `pid` (`ended`), so the row can go when its terminal does (below). `hytte run` is left out: its pid is the `hytte run` process, which exits right after reporting.

While its map or `ended` is non-empty, a 5 s ticker runs a **staleness check** on every task still in a waiting or running state (`Start`, `Progress`, `Resumed`, `NeedsInput`):

| The task has... | It is marked lost when... |
|---|---|
| a `pid` | that process is no longer alive (`proc::is_alive`: it can't be opened, or its exit code is no longer `STILL_ACTIVE`). |
| no `pid` | no message has arrived for 30 s (`STALE_AFTER`). |

A lost task is removed from the map and `TaskUpdate::Lost(id)` is sent to the UI. The same tick checks the owner of every remembered failed shell row, and when that terminal is gone sends `TaskUpdate::Gone(id)`. When both are empty, the registry blocks on messages alone and never wakes up.

Practical consequences:

- `hytte run` sends its own pid, so closing the terminal mid-build turns the task into "lost" within about 5 s.
- `hytte set` sends no pid, so a script that reports progress must send an update at least every 30 s or the task goes grey.
- Agents send the pid of their long-lived ancestor process (see [section 5](05-cli.md)), so a crashed agent is noticed.

## The UI side of a task

The bridge delivers `TaskUpdate`s as `UiEvent::Task` to `Ui::handle_events` (`window.rs`). Before handing the update to the model, the window layer does three things:

1. **Shell threshold.** A `"shell"` task's `Start` with `delay_ms == 0` gets `delay_ms = [shell] threshold_ms` (3000 by default), so quick commands never appear.
2. **Attention.** A `NeedsInput` event makes the Tasks panel peek open for `[agent] peek_secs` (6 s) and, if `[agent] sound = true`, plays the system asterisk sound.
3. **Terminal tab.** The first time a task with a `pid` is seen, the terminal-tabs worker is asked to remember which Windows Terminal tab is active (see [section 9](09-workers.md)).

`Model::apply_task` then applies the update:

- **Ignored commands.** A shell task whose command name (first word, lower-cased, without path or `.exe`) is in `[shell] ignore` is dropped.
- **Too fast to show.** A shell task that ends (`Done` or `Failed`) before its `visible_after` time is removed silently. This is how fast commands never flash up.
- **Stopped by the user.** A task of any source that ends with exit code 130 (SIGINT), 143 (SIGTERM) or -1073741510 (`0xC000013A`, Windows' Ctrl+C) was stopped, not broken, so it is removed silently instead of showing a red error. The PowerShell hook reports 130 for any command PowerShell itself records as stopped (Ctrl+C on a cmdlet or on a program), whatever code the program left behind ([section 5](05-cli.md#how-the-shell-integrations-work)). A program that handles Ctrl+C and exits with some other code still shows as failed: nothing says it was stopped.
- **New task.** It is added with `started = changed = now` and `visible_after = now + delay_ms`.
- **Existing task.** Its fields are updated. An empty label keeps the old label, a missing `progress` keeps the old progress, and a missing `pid` keeps the old pid.
- `stderr_tail` is split into lines, and `message` becomes the amber attention text while `NeedsInput`.
- **`Lost(id)`** marks the task `Lost` and restarts its clock.
- **`Gone(id)`** removes the task if it is a failed one (the terminal that ran it has closed, so there is nothing to rerun it in or jump to).

### Lifetime of a task in the pill

```text
           Start / Progress / Resumed                    NeedsInput
 (new) ─────────────► running ◄──────────────────────────► waiting (amber)
                        │                                     │
           Done ┌───────┼────────┐ Failed                     │ owner died / silent 30 s
                ▼       │        ▼                            ▼
        done (green)    │   failed (red) ──user dismisses──► gone
           │ 3 s        │        (persists; a shell
                              row goes when its
                              terminal closes)
           ▼            │ owner died / silent 30 s
          gone          ▼
                     lost (grey ?) ── 5 s ──► gone
```

The timed removals are done by `Model::expire` (see [section 6](06-ui-model.md)), driven by one Win32 timer armed for exactly the next deadline. There is no periodic sweep.

`Model::dismiss_task` (the ✕ button, or **Dismiss** on a failure) removes a finished task. Running tasks can't be dismissed.

## Asking the human: `Ask` and `Answer`

A client can put a yes/no question on the pill and wait for the answer on the same connection. `hytte agent ask` uses this to answer agents' permission prompts (Claude Code, Codex, OpenCode).

```json
{"v":1,"name":"Claude Code","pid":4242,"text":"Running: cargo publish"}
```

| Field | Rule |
|---|---|
| `v` | `PROTOCOL_VERSION` |
| `name` | 1 to 128 bytes; shown before the text |
| `pid` | Optional. When it is set and the answer is *allow*, the daemon also sends `Resumed` for `agent:<name>:<pid>`, so that agent's amber row clears |
| `text` | 1 to 1024 bytes, shown as is |

`pipe_server::answer` sends `UiEvent::Ask { id, summary, reply }` to the UI thread. It then blocks that connection's thread for up to `ASK_WAIT_SECS` (30 s). The UI shows an amber chip with **Allow** and **Deny**. A click writes one line back:

```json
{"allow":true}
```

No reply is sent if the time runs out or the chip is dismissed, and none while Hytte is paused or in game mode (the ask is dropped on arrival, and pending ones when a game starts). Asks from several agents queue: the oldest is on the chip, marked `(1/N)` while others wait, and answering or dismissing it shows the next. Each keeps its own `ASK_WAIT_SECS` from arrival, so a queued ask that waited too long is dropped and its agent asks in the terminal. A connection carries one ask: once it is answered or dropped, the daemon flushes and hangs up, so the client learns "no answer" at once instead of waiting out its own timeout. Clients must treat a closed pipe or a timeout as "no answer". The chip is forced visible over a maximised window or fullscreen video, where the pill would otherwise be the thin line. An `Ask` line never parses as a `HytteMessage`, so an older daemon ignores it and the client sees no answer.

## Script commands: `notify` and `shelf add`

A one-way request that is not a task: show a card, or put something on the shelf (`hytte notify`, `hytte shelf add`; [section 5](05-cli.md)). It is a `Cmd` (`hytte_proto::Cmd`):

```json
{"v":1,"cmd":"notify","text":"Build finished"}
{"v":1,"cmd":"shelf_add","path":"C:\\Users\\me\\report.pdf"}
{"v":1,"cmd":"shelf_add","text":"a snippet to keep"}
```

| Field | Rules |
|---|---|
| `cmd` | `"notify"` or `"shelf_add"`; anything else is rejected. |
| `text` | `notify`: 1–1024 bytes, the card text. `shelf_add`: up to 16 KiB, a snippet. |
| `path` | `shelf_add` only, an absolute path of at most 1024 bytes; exactly one of `text` and `path`. |

It has no `task_id` and no `name`, so it never parses as a `HytteMessage` or an `Ask`, and an older daemon ignores it. The daemon answers nothing. While Hytte is paused or a game is in front it does nothing (a shelf add would open the pill), a `notify` card never covers another card, and the shelf work goes through the same worker as a drop (`UiEvent::ShelfAdd`).

## Writing your own client

Any program can talk to the pipe directly; you don't need the `hytte` CLI. In PowerShell:

```powershell
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$c = New-Object System.IO.Pipes.NamedPipeClientStream('.', "hytte-$sid", [System.IO.Pipes.PipeDirection]::Out)
$c.Connect(100)
$b = [Text.Encoding]::UTF8.GetBytes('{"v":1,"task_id":"demo","event":"Start","label":"Hello"}' + "`n")
$c.Write($b, 0, $b.Length); $c.Dispose()
```

Keep messages small, always end lines with `\n`, and remember to send a final `Done` or `Failed`, or include a `pid` so the registry can clean up after you.

Next: [5. The `hytte` CLI and shell integrations](05-cli.md)
