# 4. IPC protocol and task lifecycle

Everything that shows up as a **task** in the pill arrives over one named pipe. This section covers the wire format, how the daemon defends itself, and what happens to a task from its first message until it disappears.

Code: `crates/proto/src/lib.rs` (the message type), `crates/hytte/src/pipe_server.rs` (the server), `crates/hytte/src/tasks.rs` (the registry), and `Model::apply_task` in `crates/hytte/src/ui_state.rs` (the UI side).

## The pipe

| Property | Value | Why |
|---|---|---|
| Name | `\\.\pipe\hytte` (`hytte_proto::PIPE_NAME`) | One well-known name for every client. |
| Direction | Duplex (`PIPE_ACCESS_DUPLEX`) | Task messages are fire-and-forget; only an [`Ask`](#asking-the-human-ask-and-answer) gets a reply line back. |
| Mode | Message mode, blocking | Simple reads; one `ReadFile` per client write. |
| Access control | DACL `D:(A;;GA;;;OW)`: only the owner (your user account) | Other users on the machine can't inject tasks. |
| Remote clients | Rejected (`PIPE_REJECT_REMOTE_CLIENTS`) | Nothing from the network. |
| First instance | Created with `FILE_FLAG_FIRST_PIPE_INSTANCE` | Another process can't squat the name before Hytte starts. |
| Instances | Up to 8 at once | Several clients can connect at the same moment. |
| Max message | 64 KiB (`MAX_MESSAGE_BYTES`) | Bounded memory per connection. |

The listener loop (`run_windows_loop`) creates a pipe instance, waits in `ConnectNamedPipe`, and hands each connected instance to a **new thread** that reads it until the client disconnects. It then immediately creates the next instance. One slow client can never block another.

Each connection thread (`serve_connection`) reads chunks into a buffer, splits it on `\n`, and parses each non-empty line as JSON. If a single line grows past 64 KiB, the connection is dropped. A line is parsed as a `HytteMessage` first and, if that fails, as an `Ask`. A line that fails to parse or validate is ignored without a reply.

## Message format

Newline-delimited JSON: one object per line, UTF-8. A client may send several lines on one connection; `notch.exe` sends exactly one and disconnects.

```json
{"v":1,"task_id":"agent:Claude Code:4242","event":"NeedsInput","label":"Claude Code",
 "source":"agent","pid":4242,"message":"Approve edit to src/main.rs?"}
```

| Field | Type | Required | Limit | Meaning |
|---|---|---|---|---|
| `v` | number | yes | must equal `1` | Protocol version (`PROTOCOL_VERSION`). |
| `task_id` | string | yes | 1–128 bytes | Identifies the task; later messages with the same id update it. |
| `event` | string | yes | see below | What happened. |
| `label` | string | no (defaults to `""`) | ≤ 1024 bytes | Text shown in the pill. An empty label on an update keeps the previous one. |
| `progress` | 0–100 | no | ≤ 100 | Determinate progress. Without it the bar shimmers. |
| `exit_code` | integer | no | | Shown as `exit N` for failed tasks without stderr. |
| `duration_ms` | integer | no | | Shown on finished tasks. |
| `stderr_tail` | string | no | ≤ 8192 bytes | The last lines of stderr, shown under a failed task. |
| `source` | string | no (absent = `"run"`) | ≤ 16 bytes | `"run"`, `"shell"` or `"agent"`. Changes some rules (below). |
| `pid` | integer | no | | The owning process: used for click-to-focus and liveness checks. |
| `delay_ms` | integer | no | | Don't show the task until this long after it starts. |
| `message` | string | no | ≤ 1024 bytes | Free text, shown in amber while `NeedsInput`. |
| `cwd` | string | no | ≤ 1024 bytes | The working directory (carried; not displayed today). |
| `line` | string | no | ≤ 512 bytes | The command's latest output line (`notch run` sends it), shown in mono under a running task in the expanded list. Additive: older daemons ignore it. |

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
| `notch run` | `<notch pid>-<unix ms>` | `8120-1730000000000` |
| `notch set` | a fresh id per call, unless `--task` is given | `upload-1` |
| PowerShell hook | `ps-<shell pid>-<sequence>` | `ps-4242-17` |
| bash / zsh / Nushell hooks | `sh-…`, `zsh-…`, `nu-…` with the same shape | `zsh-901-3` |
| `notch agent` | `agent:<name>:<owner pid>` | `agent:Claude Code:4242` |

## The task registry

`tasks::spawn_registry` runs on its own thread and is the only place that decides a task has been **lost**.

For each valid message it builds a `TaskState`. `source` defaults to `"run"` and `delay_ms` to 0, and `updated` is stamped with the current time. It then:

- removes the id from its map if the event is `Done`, `Failed` or `Lost` (nothing left to watch), otherwise inserts or replaces it;
- forwards the state to the UI as `TaskUpdate::Upsert`.

While its map is non-empty, a 5 s ticker runs a **staleness check** on every task still in a waiting or running state (`Start`, `Progress`, `Resumed`, `NeedsInput`):

| The task has... | It is marked lost when... |
|---|---|
| a `pid` | that process is no longer alive (`proc::is_alive`: it can't be opened, or its exit code is no longer `STILL_ACTIVE`). |
| no `pid` | no message has arrived for 30 s (`STALE_AFTER`). |

A lost task is removed from the map and `TaskUpdate::Lost(id)` is sent to the UI. When the map is empty, the registry blocks on messages alone and never wakes up.

Practical consequences:

- `notch run` sends its own pid, so closing the terminal mid-build turns the task into "lost" within about 5 s.
- `notch set` sends no pid, so a script that reports progress must send an update at least every 30 s or the task goes grey.
- Agents send the pid of their long-lived ancestor process (see [section 5](05-cli.md)), so a crashed agent is noticed.

## The UI side of a task

The bridge delivers `TaskUpdate`s as `UiEvent::Task` to `Ui::handle_events` (`window.rs`). Before handing the update to the model, the window layer does three things:

1. **Shell threshold.** A `"shell"` task's `Start` with `delay_ms == 0` gets `delay_ms = [shell] threshold_ms` (3000 by default), so quick commands never appear.
2. **Attention.** A `NeedsInput` event makes the Tasks panel peek open for `[agent] peek_secs` (6 s) and, if `[agent] sound = true`, plays the system asterisk sound.
3. **Terminal tab.** The first time a task with a `pid` is seen, the terminal-tabs worker is asked to remember which Windows Terminal tab is active (see [section 9](09-workers.md)).

`Model::apply_task` then applies the update:

- **Ignored commands.** A shell task whose command name (first word, lower-cased, without path or `.exe`) is in `[shell] ignore` is dropped.
- **Too fast to show.** A shell task that ends (`Done` or `Failed`) before its `visible_after` time, or that exits with code 130 (Ctrl-C), is removed silently. This is how fast commands never flash up.
- **New task.** It is added with `started = changed = now` and `visible_after = now + delay_ms`.
- **Existing task.** Its fields are updated. An empty label keeps the old label, a missing `progress` keeps the old progress, and a missing `pid` keeps the old pid.
- `stderr_tail` is split into lines, and `message` becomes the amber attention text while `NeedsInput`.
- **`Lost(id)`** marks the task `Lost` and restarts its clock.

### Lifetime of a task in the pill

```text
           Start / Progress / Resumed                    NeedsInput
 (new) ─────────────► running ◄──────────────────────────► waiting (amber)
                        │                                     │
           Done ┌───────┼────────┐ Failed                     │ owner died / silent 30 s
                ▼       │        ▼                            ▼
        done (green)    │   failed (red) ──user dismisses──► gone
           │ 3 s        │        (persists)
           ▼            │ owner died / silent 30 s
          gone          ▼
                     lost (grey ?) ── 5 s ──► gone
```

The timed removals are done by `Model::expire` (see [section 6](06-ui-model.md)), driven by one Win32 timer armed for exactly the next deadline. There is no periodic sweep.

`Model::dismiss_task` (the ✕ button, or **Dismiss** on a failure) removes a finished task. Running tasks can't be dismissed.

## Asking the human: `Ask` and `Answer`

A client can put a yes/no question on the pill and wait for the answer on the same connection. `notch agent ask` uses this to answer Claude Code permission prompts.

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

No reply is sent if the time runs out, the chip is dismissed, or a newer ask replaces it. Only one ask is shown at a time, and the replaced one's sender is dropped. Clients must treat a closed pipe or a timeout as "no answer". An `Ask` line never parses as a `HytteMessage`, so an older daemon ignores it and the client sees no answer.

## Writing your own client

Any program can talk to the pipe directly; you don't need `notch.exe`. In PowerShell:

```powershell
$c = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'hytte', [System.IO.Pipes.PipeDirection]::Out)
$c.Connect(100)
$b = [Text.Encoding]::UTF8.GetBytes('{"v":1,"task_id":"demo","event":"Start","label":"Hello"}' + "`n")
$c.Write($b, 0, $b.Length); $c.Dispose()
```

Keep messages small, always end lines with `\n`, and remember to send a final `Done` or `Failed`, or include a `pid` so the registry can clean up after you.

Next: [5. The `notch` CLI and shell integrations](05-cli.md)
