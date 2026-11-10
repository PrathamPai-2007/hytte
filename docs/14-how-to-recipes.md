# 14. How-to recipes

Step-by-step guides for common changes. The compiler catches most omissions, since the `match`es over `Scene`, `UiEvent` and `Action` are exhaustive.

## Add a background worker

Example: report whether the PC is on a metered connection.

1. **Create the module** (`crates/hytte/src/network.rs`) and declare it in `main.rs` (`#[cfg(windows)]` if it can't compile elsewhere).
2. **Add an event** to `UiEvent` (`Metered(bool)`) and a `Model` field for the state.
3. **Write the loop:**
   ```rust
   pub fn spawn_watcher(ui_tx: Sender<UiEvent>) {
       std::thread::spawn(move || {
           let mut last = None;
           loop {
               let now = read_state();               // the OS query
               if last != Some(now) {                 // send only on change
                   last = Some(now);
                   let _ = ui_tx.send(UiEvent::Metered(now));
               }
               wait_for_change_or_timeout();          // prefer a notification over sleep
           }
       });
   }
   ```
4. **Start it** in `main.rs` beside the other workers: `network::spawn_watcher(ui_tx.clone());`.
5. **Handle the event** in `Ui::handle_events` (`window.rs`): update the model (`after_model_change` runs after).
6. **Draw it** in the relevant scene functions in `render/`.
7. If it polls, add a row to the wake-up table in [section 12](12-performance.md).

Checklist: never exits on error · sends only on change · no polling where a notification exists · heavy calls stay on this thread.

## Show an alert card with buttons

Example: warn when the disk is nearly full, with a button that opens Storage settings.

1. Send a new `UiEvent` variant (`DiskLow(...)`) from your worker and handle it in `Ui::handle_events`.
2. Build the card with `Chip::new(summary, now)`, set `tone` (`Done`, `Warn`, `Info`) and `hold`, and add `extra` buttons:
   ```rust
   let mut c = Chip::new("C: has 3% free", now);
   c.tone = Tone::Warn;
   c.hold = Duration::from_secs(20);
   c.extra = vec![("Open".into(), ChipAction::OpenUrl("https://...".into()))];
   self.model.chip = Some(c);
   ```
   Create it only when `self.model.chip.is_none()`, so you never replace a card being read.
3. If no `ChipAction` fits, add a variant in `ui_state/cards.rs` and handle it in `Ui::chip_action`. Keep it safe: `OpenUrl` passes only `http` / `https` to the shell, and `KillPid` needs a second click.
4. Test the rule that decides *when* the card appears (a pure function or `Model`).

## Add a setting

Example: `[ports] open_https = false`.

1. Add the field to the right struct in `config.rs` with `#[serde(default)] pub open_https: bool,`. For a non-zero default use `#[serde(default = "fn_name")]` **and** update that struct's `impl Default`.
2. Read it through `self.cfg` on the UI thread so a live edit applies at once. If you derive state from it at startup or pass a clone to a worker, refresh that in `Ui::apply_config` ([section 11](11-configuration-and-files.md#live-reload)).
3. Document it in [section 11](11-configuration-and-files.md#every-setting) (and the README settings block if users commonly change it).
4. To offer it in the settings window add a row to `render::settings_table` (`pick_i` / `pick_f` / `pick_s` for steps, `Row::Switch` for a switch); use `-1` unless it belongs in the pill's pane (five rows per page). A test checks every row's key is one `config.toml` accepts. Number rows get a `Custom` step automatically; add a sentence to `render::help` (hover note, the note "Custom" writes, and what search matches) and a short name to `pane_label` if it's in the pane. A file-only string can use `pick_s_custom` (see `settings_win::custom`).

Never rename or remove a key without keeping the old name readable (`#[serde(alias = "old")]`).

## Add a field to the pipe protocol

Example: tasks carry a `url` to open on click.

1. Add it to `HytteMessage` in `crates/proto/src/lib.rs` (`#[serde(default, skip_serializing_if = "Option::is_none")] pub url: Option<String>`) and set `None` in `HytteMessage::start`.
2. Add a length limit in `validate()` (pipe input is untrusted).
3. Extend `old_messages_still_parse` with a message containing the field.
4. Carry it through `TaskState` (`tasks.rs`) → `TaskView` (`Model::upsert`).
5. Teach the CLI to send it (`cli/mod.rs` or `cli/agent.rs`), and use it in the renderer or an action.
6. Update the field table in [section 4](04-ipc-protocol.md#message-format).

Don't bump `PROTOCOL_VERSION` for an optional field.

## Add a panel (and its scene)

Example: "Clipboard history".

1. **Model** (`ui_state/scene.rs`): add `Panel::Clipboard` and `Scene::ExpClipboard`; add the panel to `Model::panels()` at its priority position with its content condition; map it in `Model::scene()` (the `hover || peeking` branch); give the scene a size in `Model::size()` (include `tab` if it shows the tab strip) and a glow in `Model::glow()`.
2. **Renderer** (`render/`): add `Scene::ExpClipboard => self.exp_clipboard(fr, w, h)` to `Renderer::content`; map it in `Panel::of` (that puts it in the tab strip); write `exp_clipboard` with `cc(...)` colours, registering clicks with `hit()` / `button()`.
3. **Ambient** (`Model::ambient`): only if it animates continuously, with a stop condition.
4. **Tests:** extend `panels_and_selection` and `scene_priority`.
5. **Look at it** without running the pill: add a `shoot(...)` call for the scene to the `snapshots` test (`render/snap.rs`), run `HYTTE_SNAP_DIR=<dir> cargo test -p hytte snapshots -- --ignored` and open `index.html` there. Pass `true` for `light` to check the light theme.

A small related feature can share a pane, as dev servers share Tasks: `panels()` shows Tasks for visible tasks **or** listening ports, `Model::size(ExpTasks)` adds `servers_h()`, and `exp_tasks` draws `servers` under the rows.

## Add a text field

Example: a "snippet name" field.

1. Add a variant to `ui_state::Field` and a `TextEdit` for it in `Model` (give it a limit in `init_jot`, `ui_state/notes.rs`); extend `edit_ref` / `edit_mut` and, in `window/win/typing.rs`, `field_panel` (the pane it lives in).
2. Draw it with `Renderer::text_field(fr, field, (x, y, w, h), "hint", right)`; it registers the `Action::Field` click for you.
3. React in `Ui::enter_field` (Enter) and, for changes as it is typed, in `Ui::edited`.
4. Keep the draft rules in `Model::end_typing` in mind (an edit of an existing item is dropped, a draft stays).
5. Look at it in the `jot_snapshots` test (empty, typing, long text, both themes) and try it in a dev build: click, type, Esc must hand focus back.

## Add a clickable action

1. Add a variant to `render::Action` carrying what the click needs.
2. Register it while drawing: `self.button("Label", x, y, w, h, ACCENT, Action::MyThing(id))`, or `self.hit(...)` around custom drawing.
3. Handle it in `Ui::run_action` (`window.rs`). Slow work (disk, network, cross-process COM) goes to a worker that reports back with a `UiEvent`.
4. Bringing another window to the front is allowed **only** from `run_action` (a user click) ([section 7](07-window-and-input.md#focusing-a-terminal)).

## Add a Drop Vault action

Example: "Rotate 90°".

1. Add a `Conv` variant (`Conv::Rotate`) in `transforms.rs` and offer it in `chips_for` for the right extensions (short labels: they're pill buttons).
2. Implement it in `transforms::run`, writing with `place(src, ".rotated", ext)` (respects `output_folder`, never overwrites) and returning an `Outcome` (summary, optional `open` path, optional `copy` text).
3. Guard memory: check dimensions before decoding (as `convert::open` does), avoid full-size copies.
4. Test with a tiny generated image in a temp directory and update `chips_follow_file_type_and_actions_are_separate`.
5. Document it in [section 10](10-shelf-and-drop-vault.md#the-actions) and the README if user-visible.

Jobs run on a Drop Vault worker automatically; no threading code needed.

## Script commands and new subcommands

Users can already do `hytte notify "Backup finished"`, `hytte set --progress 40 --label "Uploading" --task up1` (then `--done` / `--fail`) and `git diff | hytte shelf add` ([section 5](05-cli.md)). To add another script command: give `proto::Cmd` a new `cmd` value, validate it in `Cmd::validate`, handle it in `Ui::run_cmd`, and add a subcommand next to `cmd_notify`.

For a new `hytte` subcommand: add a match arm in `cli/mod.rs::run` (ideally dispatching into its own `cli/` module; the name must not collide with "no arguments" = the daemon); build a `HytteMessage` and `send_msg`, deciding what happens on `false` (silent for hooks, at most one line otherwise); add it to `USAGE`, [section 5](05-cli.md) and the README if users will type it; keep startup fast (no config reads or heavy dependencies).

## Add continuous motion

1. Drive it from `self.t` inside the scene function.
2. Decide **when it may stop** and add exactly that condition to `Model::ambient(scene, now)`.
3. Make it rest at a sensible static value (the last frame stays on screen); it is disabled under reduced motion if it only runs through `ambient`.
4. Add a test that `ambient` turns false when the condition ends (see `idle_frames_stop_for_settled_failures_and_sentinel`).

## Connect another AI agent

1. Find where the agent reads user-level hooks and what it sends on stdin, for three moments: prompt submitted (→ `hytte agent resume`), waiting for the user (→ `needs-input`, which reads a JSON `message` from stdin), turn ended (→ `done`).
2. In `agents.rs` add a `Kind`, a row in `targets()` (the config folder proving it's installed, and the file to write) and its hooks in `json_hooks` (or reuse `claude_hooks` for Claude Code's format, as Codex does). Add `--ack` if it reads hook stdout as JSON.
3. Gate a permission through `hytte agent ask` only if its hook can say "no decision" safely; an agent that blocks on an unexpected reply (Cursor) gets informational hooks only.
4. Add its CLI name to `ui_state::AGENT_CLIS` so the shell hook doesn't also show it as a command.
5. Test beside the others in `agents.rs` (connect, connect again changes nothing, undo restores the file); add a row to the README's agent table and [section 5](05-cli.md#hytte-setup-agents---undo-agentsrs).

## Add support for another shell

1. Write `cli/shell/init.<shell>`: guard against double-loading; on command start call `hytte hook start --id <unique id> --cmd <command line> --pid <Windows pid of the shell> --cwd <dir>` in the background; when the prompt returns call `hytte hook end --id <same id> --code <exit code> --duration-ms <ms>` in the background; print nothing; never change the exit code it reports.
2. Embed it in `cli/hook.rs` with `include_str!`, add it to `init()`'s match and `usage()`, and to the `scripts_are_embedded` test.
3. Document the profile line in the README and [section 5](05-cli.md#how-the-shell-integrations-work).

## Before you commit any of these

- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Try it by hand on a real desktop, and check idle cost ([section 12](12-performance.md)) if you added a worker, timer or animation.
- Update the docs section you touched: they're only useful while true.

Next: [15. Updates](15-updates.md)
