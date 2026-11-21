# 18. Agent control and the pack registry

Two features let things outside Hytte change what runs on your machine: **agents** (through `hytte mcp`) and **downloaded packs** (through `hytte pack`). This section is the security model for both, and the specification of the registry. Read it before changing `cli/mcp.rs`, `cli/mcp_panes.rs`, `cli/pack.rs`, `registry.rs`, `agent_guard.rs` or the `action_run` branch of `pipe_server.rs`.

## The one rule

**Nothing new runs because a program asked. It runs because a person pressed a button on the pill.** Everything below is a way of arranging that.

## Who is on the other end of the pipe

`hytte mcp` is a separate process that an agent starts. It reads `config.toml` and the pack folders itself and writes one-way messages (`hytte_proto::Cmd`) to the daemon's pipe. The pipe is restricted to the current Windows user (DACL, SID in its name), and **the daemon cannot tell which program is on the other end**: every process of yours looks the same. The `clientInfo.name` an MCP client sends is self-declared, so it proves nothing.

So permissions are **not per agent**. They are per **capability**, enforced where a person can see and answer, in the daemon:

| Capability | Tools | What stands in the way |
|---|---|---|
| Read | `pane_list`, `action_list` | Nothing: it shows what the pill already shows. `action_list` gives names and whether they ask "Sure?", never commands. |
| Show things | `notify`, `card`, `note_add`, `todo_add`, `pane_set` | Unchanged since 1.9.2: they show or store text, and run nothing. |
| Make panes | `pane_create`, `pane_update`, `pane_remove` | The pane is written as a **pack**, so it arrives unanswered: the pill shows its command with **Run** / **Ignore**, and nothing runs until Run. Any edit changes the file's hash and asks again. At most four agent-made panes; only folders the agent made (a `.hytte-agent` marker) can be changed or removed; your own panes are refused. |
| Run an action | `action_run` | The pill shows **"agent: run the action X?"** with Allow / Deny, every time. Only a *name* defined in your `config.toml` or an approved pack runs, never a command string. No answer in 30 s is a no. |

### `action_run`, step by step (`pipe_server.rs`, `run_action_request`)

1. The MCP process checks the name against its own list and sends `Cmd::action_run(name, arg)`.
2. The pipe thread (**not** the MCP process, so a client that skips MCP gets the same treatment) validates the message (`valid_action_id`, arg up to 256 bytes), looks the name up in `config.toml` plus approved packs, and refuses an unknown one.
3. It applies a daemon-wide rate limit (10 a minute), then sends `UiEvent::Ask` and blocks for the answer, like an agent's permission prompt.
4. Only a yes sends **`UiEvent::ActionApproved(name, arg)`**. Nothing that arrives on the pipe can construct that event, and `run_cmd` ignores an `action_run` that reaches it. The UI thread runs it through the usual `run_named_with(.., approved = true)`, which skips the action's own "Sure?" arming (a person just said yes) but keeps the busy check, the timeout and the result card.
5. Every outcome (unknown, rate limited, allowed, denied, no answer) is written to `mcp.log` and sent back as one `Pressed` line, which the agent reads as the tool result.

### What this does and does not protect against

Protects against: an agent that only has the MCP tools (a sandboxed one), a prompt-injected tool call, a runaway loop, a malicious or buggy pack arriving by `hytte pack`.

Does **not** protect against software already running as you with shell or file access. It can edit `config.toml`, write a pack folder with an `.approved` answer into the right place, or call the pipe itself. That is the same threat level the pack approval has always accepted (it keeps the *answer* outside the pack folder so a **downloaded** pack cannot approve itself, not so local malware cannot). Say so plainly in anything user-facing.

`mcp.log` is `%APPDATA%\Hytte\mcp.log`: `unix-seconds <TAB> tool <TAB> target <TAB> result`, fields cut to 200 characters with control characters turned into spaces (so a payload cannot forge a line), about 256 KB and then one old copy (`agent_guard::audit_line`, `audit_to`).

### Limits worth knowing

- Not built yet: `pane_get` and `agent_status` (they need a reply path from the UI thread's model, not just from config files), event subscription, and an "Always allow this action" answer. Each `action_run` asks again.
- The rate limits (10 a minute) are per MCP process for the pane tools (`McpIo.limit`) and daemon-wide for `action_run`.
- `pane_create`'s `command` is a command line the *agent* chose, run by `cmd /c` after you press Run. The card shows its first 60 characters; a long command is cut there. Read what an agent asks you to approve.

## The pack registry

`hytte pack search | info | install | update | remove | list [name] [--source <https-url|folder>]`.

### Where packs come from

By default `https://raw.githubusercontent.com/PrathamPai-2007/hytte/main/examples/panes`: an `index.json` and one folder per pack, in the Hytte repository itself. `--source` takes another `https://` base or a local folder with the same layout (your own collection, or offline use). Release builds fetch over HTTPS only (`net::fetch`, curl's `--proto =https`); a `--source` folder is read from disk.

### `index.json` (version 1)

```json
{ "version": 1,
  "packs": [
    { "name": "docker",
      "description": "Containers with their status, unhealthy and stopped ones first.",
      "version": "1.0.0",
      "min_hytte": "1.9.3",
      "files": [ { "path": "pane.toml", "sha256": "<64 lower-case hex>", "size": 196 } ] } ] }
```

`registry::parse_index` rejects, with a message naming the pack: a version other than 1; a name that is not its own `packs::prefix` (lower-case letters, digits, `-`, `_`, 20 at most); a duplicate pack; no `pane.toml`; more than 20 files; a file name that is not plain (`safe_file`: letters, digits, `.`, `_`, `-`; no leading dot, no `..`, no separators, 64 at most); a size of 0 or over 64 KB; a hash that is not 64 lower-case hex digits; a repeated file. A leading byte-order mark is tolerated.

### Trust chain

1. HTTPS from the source you chose.
2. Every file must match the **size and SHA-256** the index lists; any mismatch writes **nothing** (all files are fetched and checked in memory before the first write, so there is no half-installed folder).
3. What arrives is still an unanswered pack: the card shows the command and waits for Run.
4. An install or update **forgets any earlier answer** for that folder (`packs::forget_answer`). An answer covers `pane.toml` only, so without this an update could swap a script under an old approval.

The SHA-256 is implemented in `registry.rs` (about 50 lines, tested against the FIPS vectors including a million `a`s), so no hashing crate is linked for one use. The index is trusted only as far as its source: it lists the hashes, so a source that is compromised can list the hashes of bad files. The defence is step 3.

### What each command does

| Command | Behaviour |
|---|---|
| `search [word]` | Lists name, version, description; matches name or description. |
| `info <name>` | Downloads and verifies the files, then prints the pane's command, `every`, each action and its command, the file list and a reminder to read the scripts. Nothing is written. |
| `install <name>` | Refuses a pack that needs a newer Hytte (`min_hytte`), refuses an existing folder (says "update" if `hytte pack` made it, "left alone" if you did), verifies, writes the files and `.hytte-pack.json` (name, version, source, file list). |
| `update [name]` | One pack or all installed. Same version: "up to date". Otherwise fetches and verifies, overwrites the listed files, deletes files the old version had and the new one lacks, and **never touches other files** (your `repo.txt`). |
| `remove <name>` | Only for a folder with the marker; deletes the files the marker lists; if you added files the folder stays and says so. |
| `list` | Installed names and versions. |

An agent-made pane (`.hytte-agent`) and a registry pack (`.hytte-pack.json`) are separate: each tool refuses the other's folders.

### Publishing a pack

1. Add a folder under `examples/panes/` with a `pane.toml` (a pack is flat: no sub-folders) and a `README.md` whose first line of prose is its description.
2. Run `./scripts/pack-index.ps1`. It writes the index, hashing the bytes with CR removed (`.gitattributes` keeps `examples/panes/**` LF, which is what GitHub serves) and giving a pack whose files changed a patch bump.
3. Commit the folder and `index.json` together. CI's `./scripts/pack-index.ps1 -Check` fails when they disagree.

To check by hand: `hytte pack info <name> --source examples/panes`, then `install` into a throwaway `%APPDATA%`.

## Testing

- `cli/mcp.rs` (fake `Backend`): the read tools list names only; `action_run` reports the answer, refuses unknown names and bad arguments *before* asking; pane tools produce a pack that waits for Run and are logged; a flood is stopped.
- `cli/mcp_panes.rs`: a created pane parses as `State::Pending`; an update changes the hash; bad names, commands and `every`; your panes and unmarked folders are never changed; the four-pane cap; a folder with foreign files is not removed.
- `agent_guard.rs`: the limiter, a log line that cannot be forged, rotation.
- `registry.rs`: the SHA-256 vectors, install, tamper (same size, different bytes), refusing foreign folders and too-new Hytte, update keeping your files, remove, index rejection, `info`.
- `proto`: `action_run` validation.
- `scripts/smoke.ps1` drives the built exe through `hytte mcp` (tools, `pane_list`, `pane_create`, unknown `action_run`, refusing to remove your pane) and through `hytte pack` (install, list, update, remove) against `examples/panes`.
- Not automated: pressing **Allow** on the card (it needs a person). Check by hand: put an `[[action]]` in `config.toml`, call `action_run` from an agent, press Allow, then Deny, then wait 30 s, and read `mcp.log`.
