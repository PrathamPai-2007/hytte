# Hytte 1.9 plan: Ask (local AI) and an honest cockpit

## Why

Notch and pill apps are everywhere and they all compete on widgets. 1.9 gives Hytte two things
they don't have:

1. **Ask**: a one-shot question box inside the pill, answered by a small model running in the
   user's own Ollama. Nothing leaves the PC. The same model powers one-click **Explain**,
   **Summarise** and **Shorten** on things Hytte already holds (clipboard clips, shelf text files,
   failed commands); those answers arrive as a card, not in the Ask pane.
2. **An honest cockpit**: turn timers, a quiet state, hook health and a pill badge for agents.

## Decisions (from the user, 2026-11-12)

- The interface lives **inside the pill**. No pop-out window. The answer area scrolls.
- The main feature is a **one-shot Ask pane**. Not a chat: no threads, no history.
- Default model **`qwen3:1.7b`** (1.4 GB, Apache 2.0), thinking off. Any model the user already
  has can be picked.
- A **Get model** button: Hytte asks the user's Ollama to pull the model and shows progress.
- AI actions on **clipboard clips, shelf text files, failed command cards** (not agent replies).
  The action buttons live on those things, **never in the Ask pane**, and the answer shows in a
  **card** with Copy. The Ask pane is only the question field and its answer.
- The four cockpit tasks stay in 1.9.
- No push, no webhook, no network beyond the user's own Ollama on this PC.

## Assumptions (correct me if any is wrong)

1. **Off by default.** `[ai] enabled = false`; the Ask pane appears only when switched on, like
   clipboard history. Hytte never installs Ollama; it links to ollama.com.
2. **Loopback only.** Hytte talks only to `127.0.0.1` / `localhost` (`[ai] host`, default
   `127.0.0.1:11434`). Any other host is refused, so "nothing leaves the PC" stays true.
3. **The question box is one line** (the existing `TextEdit`). Pasted newlines become spaces.
4. **Action cards grow to fit.** The AI card is as tall as its answer needs, growing as it
   streams. Only past a screen cap (about half the screen height) does the rest scroll inside the
   card, so it can never run off the screen. Prompts still ask for a brief answer.
5. **Nothing on disk.** The last answer stays in memory until cleared or Hytte quits.
6. **Plain text answers.** Markdown markers (`**`, `#`, backticks) are stripped; code blocks are
   shown in the monospace font if that is cheap, otherwise as plain lines.
7. **Short answers.** A small system prompt asks for brevity; `num_predict` caps the length
   (about 400 tokens in the pane), `num_ctx` 4096, action inputs cut to about 6 KB.
8. **Model unloads soon.** `keep_alive` 2 minutes (setting), so the 1–2 GB model doesn't sit in
   RAM after a question. That memory is Ollama's, but people will blame Hytte.

## What exists (verified in code)

- `net.rs`: HTTP through System32 `curl.exe`, HTTPS-only in release, blocking, on a worker thread.
  No HTTP stack linked. Ask needs a separate loopback path (plain HTTP, streaming).
- `textedit.rs`: one-line editor (caret, selection, word jumps), used by Notes and the weather place.
- `render.rs`: a DirectWrite format with `DWRITE_WORD_WRAPPING_WRAP` already exists.
- `scriptpane.rs`: runs a command on a thread with a time limit and kills it; same shape as
  streaming curl with a Stop button.
- `ui_state/scene.rs`: `Panel` enum; `config.rs` `Panes` switches and `order`.
- `ui_state/shelf.rs` + `render/shelf.rs`: clip rows with Copy and ×; `transforms.rs` `chips_for`
  gives each shelf file its chips.
- `proto`: failed tasks carry `stderr_tail` (up to 8 KB). Only `hytte run` fills it; shell-hook
  tasks don't, so Explain shows only where the tail exists.
- `config.rs`: hotkeys (`hotkey`, `note_hotkey`) parsed and registered already.

## Ollama facts used (from ollama.com and its API docs, read 2026-11-12)

- `GET /api/version`, `GET /api/tags` (installed models with size and parameter count).
- `POST /api/chat`: streams one JSON object per line (`message.content`, `done`); final line has
  `done_reason`, token counts, durations. Params: `model`, `messages`, `think`, `options`
  (`num_predict`, `num_ctx`, `temperature`), `stream`, `keep_alive` (default 5m).
- `POST /api/pull`: streams `status`, `digest`, `total`, `completed` (completed may be absent
  until a layer finishes); ends with `success`.
- `qwen3:1.7b`: 1.4 GB, 40K context, tools + thinking. `think: false` turns thinking off.
- To confirm in Task 1: the minimum Ollama version that accepts `think`; how `OLLAMA_HOST` is set.

## Build order

```
Phase A  A1 ollama client ── A2 config + status ── A3 Ask pane ──┬── A4 Get model
                                                                  └── A5 hotkey
Phase B  A3 ── B1 AI card ──┬── B2 clips
              ├── B3 shelf files
              └── B4 failed commands
Phase C  C1 turn timers, C2 quiet, C3 hook health  (independent of A/B) ── C4 pill badge
Phase D  D1 docs ── D2 release
```

Phase C can run in parallel with A and B; it touches different files except `config.rs` and
`settings_table.rs`.

## Phase A: Ask

### A1: Loopback Ollama client (`ollama.rs`)
`version()`, `models()`, `chat(req) -> stream of chunks`, `pull(name) -> stream of progress`,
each cancellable. Through `curl.exe` like `net.rs`, but: only loopback hosts, `--proto =http`,
`--noproxy *` (a system proxy must never see the text), request body on stdin (`--data-binary @-`)
so the question never appears in a process list, `-N` for unbuffered streaming. Cancel kills curl.
The NDJSON line parser and request builder are pure functions.
- [ ] Tests: parse chat chunks, final line, error line, pull progress with and without
      `completed`; request JSON (`think: false`, options, keep_alive); non-loopback host refused
- [ ] Live: a question against a real Ollama streams; Stop ends it within a second
**Files:** new `ollama.rs`, `main.rs` (mod). **Scope:** M

### A2: `[ai]` config, Settings section, status
Keys below. Settings → Ask: switch, model (installed models, plus "qwen3:1.7b (get it)"),
keep-alive. Status, checked when the pane opens (not polled): Ollama not installed / not running /
model missing / ready.
- [ ] Tests: defaults, host validation, status from version + tags results
- [ ] Every key documented with its default (D1)
**Files:** `config.rs`, `render/settings_table.rs`, `ui_state/` (new `ask.rs`). **Scope:** S

### A3: The Ask pane
`Panel::Ask` (`[panes] ask`, shown when `[ai] enabled`). Top: one-line question field, Ask/Stop.
Below: the answer, word-wrapped in a fixed-height box (about 8 lines) that the wheel scrolls;
it follows the stream until the user scrolls up. Under it: model name, "Local · can be wrong",
Copy, Clear. Empty states for each A2 status, each with one action (Download Ollama link,
Get model, Retry).
**Risk first:** the model computes layout without DirectWrite, but wrapped height needs it.
Spike before the rest: renderer measures the wrapped text and stores the content height; the
model clamps the scroll offset with last frame's height. Keep that the only feedback path.
- [ ] Spike done and agreed before the pane is finished
- [ ] Snapshot page: each status, streaming, long answer scrolled, code block, Stop mid-answer
- [ ] Enter asks, Esc stops then clears; the pane never steals focus from a game (existing rules)
**Files:** `ui_state/ask.rs`, `ui_state/scene.rs`, `render/ask.rs`, `render.rs`,
`window/win/actions.rs`, `window/win/events.rs`. **Scope:** L

### A4: Get model
The button pulls `[ai] model` through Ollama with a progress bar (bytes done / total) and Cancel.
On success the pane is ready. Errors are shown as Ollama reports them.
- [ ] Tests: progress maths across layers, success, error line
- [ ] Live: pull `qwen3:0.6b` (smaller) on a real Ollama; cancel halfway, then resume
**Files:** `ollama.rs`, `ui_state/ask.rs`, `render/ask.rs`. **Scope:** S

### A5: Ask hotkey
`[ai] hotkey` (empty = off) opens the pill on the Ask pane with the caret in the field, the same
way `note_hotkey` opens Notes.
- [ ] Test: parse; live: works over a maximised window
**Files:** `config.rs`, `window/win/*`. **Scope:** XS

## Phase B: AI actions

### B1: The AI card
One action = `{kind: Explain | Summarise | Shorten, source label, text}` → a fixed prompt template
→ a streamed answer shown as a card: title ("Summary of clip", "Why it failed"), the whole answer
word-wrapped, the card growing line by line as it streams, then **Copy** and ×. Past the screen
cap (about half the screen) the text scrolls inside the card with the wheel. It reuses A3's wrapped
answer box (same measurement path), so there is one wrapping and scrolling implementation. A spinner and Stop while it runs. It goes
through the card queue below agent asks; a new action replaces a running one. The Ask pane is not
opened or touched. Text cut to 6 KB, with "(cut)" in the title when it was.
Shown only while `[ai] enabled`; if Ollama isn't ready the card says so with the one fix
(Open Settings → Ask / Get model).
- [ ] Tests: prompt per kind, cut marker, queue order (ask beats AI card), replace-running
- [ ] Snapshots: streaming (card growing), short answer, long answer at the cap scrolled,
      Ollama not running
- [ ] Card height comes from the measured text, so the pill's size and the drawing never disagree
**Files:** `ui_state/cards.rs`, `ui_state/ask.rs` (shared request), render card code. **Scope:** M

### B2: Clipboard clips
A small ✦ glyph on each clip row (between Copy and ×). Clicking it shows that clip's chips under
it: Explain, Summarise, Shorten (the same pattern as a selected shelf item's chips). A chip starts
the AI card. Only while `[ai] enabled` and clipboard history is on.
- [ ] Snapshot: clip row with the glyph, chips open; hit areas don't overlap Copy and ×
**Files:** `ui_state/shelf.rs`, `render/shelf.rs`, `render.rs` (action), `window/win/actions.rs`.
**Scope:** S

### B3: Shelf text files
"Summarise" and "Explain" chips for text-like files (txt, md, log, csv, json, yaml, toml, common
code extensions), sending the first 6 KB. Binary or non-UTF-8 files don't get them.
- [ ] Tests: which extensions get them; non-UTF-8 refused
**Files:** `transforms.rs`, `ui_state/shelf.rs`. **Scope:** XS

### B4: Failed command cards
An "Explain" button on a failed task's card when it has a `stderr_tail`. Sends the command line,
exit code and the tail; the answer replaces the failed card with the AI card.
- [ ] Test: button only with a tail; live: `hytte run -- cargo build` with a broken file
**Files:** `ui_state/cards.rs`, render card code. **Scope:** XS

### Checkpoint AB
- [ ] Snapshot page reviewed by you; live run with Ollama and `qwen3:1.7b` on this PC

## Phase C: Honest cockpit

### C1: Turn timers
Live agent row: "working 12m" from the prompt being submitted. Done card line gains the duration:
"Ran 8 min · Edited 4 files · 84k context". Agents without a prompt hook (Antigravity, Cursor)
start at their first event; the manual says which are exact.
- [ ] Tests: format; `resume` after an ask doesn't reset; a new prompt does
**Files:** `tasks.rs`, `ui_state/agents.rs`, `cli/agent.rs`. **Scope:** S

### C2: Quiet state
A running agent with a live pid and no hook event for `quiet_after_secs` (300, 0 = off) shows a
grey "quiet 6m". Label only: no beep, no glow, never "failed".
- [ ] Tests: threshold, any event clears, 0 = off, no pid = no label
**Files:** `ui_state/agents.rs`, `config.rs`, `settings_table.rs`. **Scope:** S

### C3: Hook health
Last-reported time per agent (memory). Empty Agents pane: "No agents reporting. Connect AI agents
in the tray menu." `hytte doctor`: recorded exe missing, Codex hooks not approved, config
unreadable, connected but never reported since Hytte started.
- [ ] Tests: each doctor line
**Files:** `cli/doctor.rs`, `agents.rs`, `ui_state/agents.rs`. **Scope:** M

### C4: Pill badge
Two or more live agents: a small count on the compact pill; amber "2 waiting" when asks are
pending. One agent: unchanged.
- [ ] Snapshots: one, three, two waiting
**Files:** `render/compact.rs`, `ui_state/agents.rs`. **Scope:** S

### Checkpoint C
- [ ] Snapshot page reviewed; live run with Claude Code (timer, quiet, badge)

## Phase D: Ship

### D1: Docs
Manual: new "Ask" section (install Ollama, Get model, privacy, what small models are bad at),
section 4 for C1–C4, section 5 for clip and file actions. Docs 06, 09 (worker), 11 (keys),
16 (1.9 notes, broad, no what's-new cards for updaters).
**Files:** `docs/*`, `manual/*`, `README.md`. **Scope:** M

### D2: Release 1.9.0
Tests, clippy, `scripts/version.ps1 check`, dry run, tag only on your go-ahead. Signing stays.
Say plainly what wasn't verified (ARM hardware, agents other than Claude Code, Ollama on a PC
without a GPU if this one has one).
**Files:** `Cargo.toml`s, `scripts/*`. **Scope:** S

## New config keys

| Key | Default | Meaning |
|---|---|---|
| `[ai] enabled` | false | Show the Ask pane and the AI actions |
| `[ai] host` | `127.0.0.1:11434` | Ollama address; loopback only |
| `[ai] model` | `qwen3:1.7b` | Model to ask |
| `[ai] keep_alive` | `2m` | How long Ollama keeps the model loaded after a question |
| `[ai] hotkey` | empty | Opens the Ask pane with the caret ready |
| `[panes] ask` | true | The Ask pane's switch (only matters while `[ai] enabled`) |
| `[agents] quiet_after_secs` | 300 | "quiet" label after this long without news (0 = off) |

## Out of scope

- Chat threads, history, saved answers, a pop-out window.
- Remote or cloud models, API keys, any host that isn't loopback.
- Tools, agent mode, reading files the user didn't attach, images.
- AI on agent replies (not chosen).
- Bundling or installing Ollama or models without the user pressing Get model.
- Session history, Recent list, away recap, push (dropped earlier).

## Risks

- **Wrapped text measurement (A3).** The one new rendering problem. Spiked first; if it gets
  messy, fall back to a fixed character-width wrap in the model (worse, but no feedback path).
- **Small-model quality.** 1.7B will be wrong sometimes. The pane says "can be wrong"; the
  manual says what it's good for (explaining errors, summarising, shortening) and not.
- **First answer is slow** while Ollama loads the model (seconds on a CPU-only laptop). The pane
  shows "Loading model…" from Ollama's first silence until the first token.
- **Users without Ollama** see nothing new unless they switch Ask on; then one clear empty state.
- **A growing card resizes the pill while it streams.** Resizing every token would flicker, so
  height changes are batched per frame and only grow (no shrink until the answer ends). Every
  other card stays as it is today.
- **Size of 1.9.** 15 tasks. Phase C is independent, so if A3 runs long, C ships and A/B wait
  rather than the other way round. The user decides at Checkpoint AB.

## Checkpoints

1. After the A3 spike: you see the wrapped, scrolling answer box in a snapshot.
2. Checkpoint AB: snapshot page + live run with Ollama.
3. Checkpoint C: snapshot page + live run with Claude Code.
4. Release decision after D1.
