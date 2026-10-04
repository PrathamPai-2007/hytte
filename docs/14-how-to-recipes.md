# 14. How-to recipes

Step-by-step guides for the most common kinds of change. Each recipe lists every place you need to touch; the compiler catches most omissions, since the `match`es over `Scene`, `UiEvent` and `Action` are exhaustive.

## Add a background worker

Example: a worker that reports whether the PC is on a metered connection.

1. **Create the module**, e.g. `crates/hytte/src/network.rs`, and declare it in `main.rs` (`mod network;`, with `#[cfg(windows)]` if it can't compile elsewhere).
2. **Add an event** to `UiEvent` in `ui_state.rs`, e.g. `Metered(bool)`, and a field to `Model` for the state.
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
4. **Start it** in `main.rs` next to the other workers: `network::spawn_watcher(ui_tx.clone());`.
5. **Handle the event** in `Ui::handle_events` (`window.rs`): update the model. `after_model_change` runs automatically afterwards.
6. **Draw it** in the relevant scene functions in `render.rs`.
7. Add a row to the wake-up table in [section 12](12-performance.md) if it polls.

Checklist: never exits on error · sends only on change · no polling where a notification exists · heavy calls stay on this thread.

## Add a setting

Example: `[ports] open_https = false`.

1. Add the field to the right struct in `config.rs` with a serde default:
   ```rust
   #[serde(default)]
   pub open_https: bool,
   ```
   For a non-zero default, use `#[serde(default = "fn_name")]` and a function returning it, **and** update that struct's `impl Default`.
2. Read it where needed through `self.cfg` on the UI thread, or pass a clone to the worker that needs it.
3. Document it in [section 11](11-configuration-and-files.md#every-setting). If users will commonly change it, also add it to the main README's settings block.

Old config files keep working because of the default. Never rename or remove a key without keeping the old name readable (`#[serde(alias = "old")]`).

## Add a field to the pipe protocol

Example: let tasks carry a `url` to open when clicked.

1. Add it to `HytteMessage` in `crates/proto/src/lib.rs`:
   ```rust
   #[serde(default, skip_serializing_if = "Option::is_none")]
   pub url: Option<String>,
   ```
   Set it to `None` in `HytteMessage::start`.
2. Add a length limit in `validate()`. Pipe input is untrusted.
3. Extend the `old_messages_still_parse` test with a message containing the field.
4. Carry it through `TaskState` (`tasks.rs`) → `TaskView` (`Model::upsert` in `ui_state.rs`).
5. Teach the CLI to send it (`cli/main.rs` or `cli/agent.rs`).
6. Use it in the renderer or in an action.
7. Update the field table in [section 4](4-ipc-protocol.md#message-format).

Don't bump `PROTOCOL_VERSION` for an optional field.

## Add a panel (and its scene)

Example: a "Clipboard history" panel.

1. **Model** (`ui_state.rs`):
   - add `Panel::Clipboard` and `Scene::ExpClipboard`;
   - add the panel to `Model::panels()` at the right priority position, with the condition for when it has content;
   - map it in `Model::scene()` (the `hover || peeking` branch);
   - give the scene a size in `Model::size()` (include `tab` if it shows tab dots) and a glow in `Model::glow()`.
2. **Renderer** (`render.rs`):
   - add `Scene::ExpClipboard => self.exp_clipboard(fr, w, h)` to `Renderer::content`;
   - add it to the list of scenes that draw tab dots, if it should;
   - write `exp_clipboard`, drawing with `cc(...)` colours and registering clicks with `hit()` / `button()`.
3. **Ambient** (`Model::ambient`): only if the panel animates continuously, and only with a stop condition.
4. **Tests:** extend `panels_and_selection` and `scene_priority` in `ui_state.rs`.

## Add a clickable action

1. Add a variant to `render::Action`, carrying the data the click needs (an id, a path, ...).
2. Register it while drawing: `self.button("Label", x, y, w, h, ACCENT, Action::MyThing(id))`, or `self.hit(...)` around a custom drawing.
3. Handle it in `Ui::run_action` (`window.rs`). Anything slow (disk, network, COM calls into other processes) goes to a worker thread, reporting back with a `UiEvent`.
4. If it brings another window to the front, call that **only** from `run_action` (a user click). Background focus changes are forbidden ([section 7](7-window-and-input.md#focusing-a-terminal)).

## Add a Drop Vault action

Example: "Rotate 90°" for images.

1. Add a `Conv` variant in `transforms.rs`, e.g. `Conv::Rotate`.
2. Offer it in `chips_for` for the right extensions (keep labels short; they are pill buttons).
3. Implement it in `transforms::run`. Write the output with `place(src, ".rotated", ext)`, so it respects `output_folder` and never overwrites. Return an `Outcome` (summary, optional `open` path, optional `copy` text).
4. Guard memory: check image dimensions before decoding (as `convert::open` does) and avoid needless full-size copies.
5. Add a test with a tiny generated image in a temp directory, and update `chips_follow_file_type_and_actions_are_separate`.
6. Document it in the actions table in [section 10](10-shelf-and-drop-vault.md#the-actions) and in the README's feature list if it is user-visible.

The job runs on a Drop Vault worker automatically; you don't need any threading code.

## Add a `notch` subcommand

1. Add a match arm in `cli/main.rs::main`, ideally dispatching into its own module in `cli/`.
2. Build a `HytteMessage` and call `send_msg`. Decide what happens when it returns `false` (daemon down): silent for anything a hook calls, at most one line otherwise.
3. Add the command to `usage()`, to [section 5](5-cli.md), and to the README if users will type it.
4. Keep startup fast: no config reads and no heavy dependencies.

## Add continuous motion

1. Drive it from `self.t` (animation seconds) inside the scene function.
2. Decide **when it may stop**, and add exactly that condition to `Model::ambient(scene, now)`.
3. Make sure it rests at a sensible static value when it stops; the last drawn frame stays on screen.
4. Check it is disabled with reduced motion (it is, if it only runs through `ambient`).
5. Add a test in `ui_state.rs` that `ambient` turns false once the condition ends (see `idle_frames_stop_for_settled_failures_and_sentinel`).

## Add support for another shell

1. Write `cli/shell/init.<shell>`. It must: guard against double-loading; on command start, call `notch hook start --id <unique id> --cmd <command line> --pid <Windows pid of the shell> --cwd <dir>` in the background; when the prompt returns, call `notch hook end --id <same id> --code <exit code> --duration-ms <ms>` in the background; never print anything; never change the exit code it reports.
2. Embed it in `cli/hook.rs` with `include_str!`, add it to `init()`'s match and to `usage()`, and add it to the `scripts_are_embedded` test.
3. Document the profile line in the README and in [section 5](5-cli.md#how-the-shell-integrations-work).

## Before you commit any of these

- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- Try it by hand on a real desktop.
- Check the idle cost ([section 12](12-performance.md)) if you added a worker, a timer or an animation.
- Update the docs section you touched. These docs are only useful while they are true.

Back to the [index](README.md).
