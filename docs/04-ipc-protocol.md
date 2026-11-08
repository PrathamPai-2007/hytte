# 4. IPC protocol and task lifecycle

Every **task** arrives over one named pipe. This section covers the wire format, how the daemon defends itself, and a task's life from first message to disappearance.

Code: `crates/proto/src/lib.rs` (messages), `pipe_server.rs` (server), `tasks.rs` (registry), `Model::apply_task` in `ui_state.rs` (UI side).

## The pipe

| Property | Value | Why |
|---|---|---|
| Name | `\\.\pipe\hytte-<user SID>` (`hytte_proto::pipe_name()`) | One pipe per Windows user; a stranger creating plain `hytte` catches nothing. (Before 1.6.0: `\\.\pipe\hytte`.) |
| Direction / mode | Duplex, message mode, blocking | Tasks are fire-and-forget; only an [`Ask`](#asking-the-human-ask-and-answer) gets a reply line. |
| Access | DACL `D:(A;;GA;;;OW)` (owner only); remote clients rejected | No other user or network injection. |
| First instance | `FILE_FLAG_FIRST_PIPE_INSTANCE` | Nobody can squat the name first. |
| Instances | Unlimited | Bursts and pending asks (up to 30 s) can't exhaust them. |
| Max message | 64 KiB (`MAX_MESSAGE_BYTES`) | Bounded memory per connection. |

The listener creates an instance, waits in `ConnectNamedPipe`, hands it to a **new thread** and creates the next one, so a slow client never blocks another. `serve_connection` splits input on `\n` and parses each line as JSON: a `HytteMessage`, then an `Ask`, then a `Cmd`. A line over 64 KiB drops the connection; an unparseable line is ignored silently; a message that parses but fails validation is dropped with a log line.

**Clients check who they talk to.** `open_pipe` asks Windows who runs the server (`GetNamedPipeServerProcessId`, then its token) and refuses unless it is the same user (`pipe_server_is_current_user`), so a look-alike pipe can't read commands or answer permission requests. A busy pipe is retried for about 200 ms; a missing pipe fails at once. `hytte run` starts a daemon only when the pipe is missing, never when merely busy.

## Message format

Newline-delimited UTF-8 JSON, one object per line. The CLI sends one line and disconnects.

```json
{"v":1,"task_id":"agent:Claude Code:4242","event":"NeedsInput","label":"Claude Code",
 "source":"agent","pid":4242,"message":"Approve edit to src/main.rs?"}
```

| Field | Required | Limit | Meaning |
|---|---|---|---|
| `v` | yes | must be `1` | `PROTOCOL_VERSION`. |
| `task_id` | yes | 1–128 B | Later messages with the same id update the task. |
| `event` | yes | below | What happened. |
| `label` | no | ≤ 1024 B | Text on the pill. Empty on an update keeps the old one. A label ending in `…` was cut by the sender: no Rerun. |
| `progress` | no | ≤ 100 | Determinate progress; without it the bar shimmers. |
| `exit_code`, `duration_ms` | no | | `exit N` on failures without stderr; time on finished tasks. |
| `stderr_tail` | no | ≤ 8192 B | Last stderr lines under a failed task. |
| `source` | no (`"run"`) | ≤ 16 B | `"run"`, `"shell"` or `"agent"`; changes some rules. |
| `pid` | no | | Owning process: click-to-focus and liveness. |
| `delay_ms` | no | | Hide the task until this long after it starts. |
| `message` | no | ≤ 1024 B | Free text, amber while `NeedsInput`. |
| `cwd` | no | ≤ 1024 B | Where **Rerun** starts the command (`hytte run` and every hook send it). |
| `line` | no | ≤ 512 B | Latest output line, mono under a running task (`hytte run`). Older daemons ignore it. |

**Limits are applied when sending:** `HytteMessage::to_line` trims each text field on a character boundary (`proto::cut`), so long replies or error tails arrive shortened, not rejected. The daemon still validates everything.

| Event | Meaning | Pill shows |
|---|---|---|
| `Start` | Began. | Spinner, elapsed time. |
| `Progress` | Update, usually with `progress`. | Percentage, filling bar. |
| `NeedsInput` | Blocked on a human. | Amber glow, bell, `message`; the pill peeks. |
| `Resumed` | Unblocked. | Spinner again. |
| `Done` | Succeeded. | Green tick for 3 s, then gone. |
| `Failed` | Errored. | Red cross and stderr until dismissed. |
| `Lost` | Owner vanished unreported (normally made by the daemon). | Grey `?` for 5 s. |

**Validation** (`HytteMessage::validate`) runs twice, on the connection thread and in the registry: wrong version, bad `task_id`, `progress > 100` or an over-limit text field rejects the message.

**Compatibility:** new fields must be optional (`#[serde(default, skip_serializing_if = "Option::is_none")]`); unknown fields are ignored; bump `PROTOCOL_VERSION` only for changes old daemons must refuse (other versions are dropped silently). `old_messages_still_parse` in `crates/proto/src/lib.rs` pins this: extend it when you add a field.

## Task ids

| Client | Format | Example |
|---|---|---|
| `hytte run` | `<CLI pid>-<unix ms>` | `8120-1730000000000` |
| `hytte set` | fresh per call unless `--task` | `upload-1` |
| PowerShell hook | `ps-<shell pid>-<seq>` | `ps-4242-17` |
| bash / zsh / Nushell | `sh-…`, `zsh-…`, `nu-…` | `zsh-901-3` |
| `hytte agent` | `agent:<name>:<owner pid>` | `agent:Claude Code:4242` |

## The task registry

`tasks::spawn_registry` is the only place that decides a task is **lost**. For each valid message it builds a `TaskState` (`source` defaults to `"run"`, `delay_ms` to 0, `updated` = now), removes the id on `Done` / `Failed` / `Lost` (else inserts), forwards it as `TaskUpdate::Upsert`, and remembers a `Failed` shell row with its `pid` (`ended`) so the row can go when its terminal does. (`hytte run` is excluded: its pid exits right after reporting.)

While anything is tracked, a 5 s tick checks every waiting or running task (`Start`, `Progress`, `Resumed`, `NeedsInput`):

| Task has | Lost when |
|---|---|
| a `pid` | that process is gone (`proc::is_alive`). |
| no `pid` | no message for 30 s (`STALE_AFTER`). |

A lost task is removed and `TaskUpdate::Lost(id)` goes to the UI. The same tick sends `TaskUpdate::Gone(id)` for remembered failed shell rows whose terminal has closed. With nothing tracked, the registry sleeps on messages alone.

So: closing the terminal mid-`hytte run` turns it "lost" within ~5 s; a `hytte set` script (no pid) must update at least every 30 s or go grey; agents send their long-lived ancestor's pid ([section 5](05-cli.md)) so crashes are noticed.

## The UI side of a task

The window layer first: (1) a `"shell"` `Start` with `delay_ms == 0` gets `[shell] threshold_ms` (3000), so quick commands never appear; (2) `NeedsInput` makes the Tasks panel peek for `[agent] peek_secs` (6 s) and, with `[agent] sound`, plays the asterisk sound; (3) the first time a task with a `pid` appears, the tabs worker remembers the active Windows Terminal tab ([section 9](09-workers.md)).

Then `Model::apply_task`:

- **Ignored:** a shell task whose command (first word, lower-cased, no path or `.exe`) is in `[shell] ignore` is dropped.
- **Too fast:** a shell task ending before its `visible_after` is removed silently.
- **Stopped by the user:** any task ending with exit 130, 143 or -1073741510 (`0xC000013A`, Windows Ctrl+C) is removed silently, not shown red. The PowerShell hook reports 130 for anything PowerShell records as stopped ([section 5](05-cli.md#how-the-shell-integrations-work)). A program that catches Ctrl+C and exits with another code still shows as failed.
- **New:** added with `started = changed = now`, `visible_after = now + delay_ms`. **Existing:** fields update; empty label, missing `progress` or missing `pid` keep the old value.
- `stderr_tail` becomes lines; `message` becomes the amber text while `NeedsInput`.
- **`Lost(id)`** marks it `Lost` and restarts its clock. **`Gone(id)`** removes a failed task (its terminal closed).

```text
           Start / Progress / Resumed                    NeedsInput
 (new) ─────────────► running ◄──────────────────────────► waiting (amber)
                        │                                     │
           Done ┌───────┼────────┐ Failed                     │ owner died / silent 30 s
                ▼       │        ▼                            ▼
        done (green)    │   failed (red) ──user dismisses──► gone
           │ 3 s        │   (a shell row also goes when its terminal closes)
           ▼            │ owner died / silent 30 s
          gone          ▼
                     lost (grey ?) ── 5 s ──► gone
```

Timed removals are done by `Model::expire` ([section 6](06-ui-model.md)) from one Win32 timer armed for the next deadline; there is no sweep. `Model::dismiss_task` (✕, or **Dismiss**) removes a finished task; running tasks can't be dismissed.

## Asking the human: `Ask` and `Answer`

A client can put a yes/no question on the pill and wait on the same connection. `hytte agent ask` uses it for agent permission prompts (Claude Code, Codex, OpenCode).

```json
{"v":1,"name":"Claude Code","pid":4242,"text":"Running: cargo publish"}
```

`v` is `PROTOCOL_VERSION`; `name` 1–128 B, shown before the text; `pid` optional (on *allow* the daemon also sends `Resumed` for `agent:<name>:<pid>`, clearing the amber row); `text` 1–1024 B, shown as is.

`pipe_server::answer` sends `UiEvent::Ask { id, summary, reply }` to the UI and blocks that connection's thread up to `ASK_WAIT_SECS` (30 s). The amber chip's **Allow** / **Deny** writes one line back: `{"allow":true}`.

No reply on timeout or dismissal, nor while Hytte is paused or in game mode (the ask is dropped on arrival, pending ones when a game starts). Asks queue: the oldest shows, marked `(1/N)`, and each keeps its own 30 s from arrival, so a stale one is dropped and its agent asks in the terminal. One ask per connection: after an answer or drop the daemon flushes and hangs up, so the client learns "no answer" at once; clients must treat a closed pipe or timeout as "no answer". The chip is forced visible over a maximised window or fullscreen video. An `Ask` never parses as a `HytteMessage`, so an older daemon ignores it.

## Script commands: `notify` and `shelf add`

One-way requests that aren't tasks (`hytte notify`, `hytte shelf add`; [section 5](05-cli.md)), as a `hytte_proto::Cmd`:

```json
{"v":1,"cmd":"notify","text":"Build finished"}
{"v":1,"cmd":"shelf_add","path":"C:\\Users\\me\\report.pdf"}
{"v":1,"cmd":"shelf_add","text":"a snippet to keep"}
```

`cmd` is `"notify"` or `"shelf_add"`. `text`: 1–1024 B for `notify`, up to 16 KiB for `shelf_add`. `path` (`shelf_add` only): absolute, ≤ 1024 B, exactly one of `text` / `path`. No `task_id` or `name`, so it never parses as the other types, and the daemon replies nothing. Paused or in game mode it does nothing (a shelf add would open the pill); a `notify` card never covers another card; shelf work uses the drop worker (`UiEvent::ShelfAdd`).

### Live cards: `card`

`hytte card` sends `cmd: "card"` with optional fields: `id` (1–128 B; the same id updates the card in place), `title` (≤ 128 B), `text` (the body, ≤ 1024 B), `progress` (0–100), `icon` (`info`, `ok`, `warn`, `error`), `buttons` (0–2 labels, ≤ 24 B each), `open` (an http(s) URL or an absolute path), `ttl` (1–3600 s, default 10), `done`, `clear` (needs `id`) and `wait` (needs a button).

```json
{"v":1,"cmd":"card","id":"deploy","title":"Deploy","text":"Uploading…","progress":40}
{"v":1,"cmd":"card","title":"Restart server?","buttons":["Restart","Later"],"ttl":60,"wait":true}
```

Card fields on any other command make it invalid; a daemon from before 1.7 drops a `card` line as an unknown command. Cards queue for the chip behind agent asks and the update offer (`Model::cards`, shown from `after_model_change`). With `wait` the connection stays open like an `Ask`: a press writes one line, `{"pressed":"Restart"}`; the card expiring, being dismissed or cleared drops the reply sender, so the client sees a hang-up. The wait is capped at the ttl plus `CARD_QUEUE_SECS` (300 s) for a card stuck in the queue. Buttons never run anything.

### Script pane text: `pane_set`

```json
{"v":1,"cmd":"pane_set","id":"Deploys","text":"web v2.0.0 live"}
```

`id` is the `[[script_pane]]` name (1–64 B, any case), `text` up to 4 KiB (the pane keeps 2 KiB after cleaning; empty empties it). A name no pane has is ignored by the daemon; `hytte pane set` checks `config.toml` first and says so.

## Writing your own client

Any program can use the pipe directly. In PowerShell:

```powershell
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$c = New-Object System.IO.Pipes.NamedPipeClientStream('.', "hytte-$sid", [System.IO.Pipes.PipeDirection]::Out)
$c.Connect(100)
$b = [Text.Encoding]::UTF8.GetBytes('{"v":1,"task_id":"demo","event":"Start","label":"Hello"}' + "`n")
$c.Write($b, 0, $b.Length); $c.Dispose()
```

Keep messages small, end lines with `\n`, and send a final `Done` or `Failed` (or a `pid`) so the registry can clean up.

Next: [5. The `hytte` CLI and shell integrations](05-cli.md)
