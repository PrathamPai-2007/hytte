# Hytte developer documentation

This folder explains how Hytte works on the inside. It is written for people who want to change the code. If you only want to *use* Hytte, the [main README](../README.md) is all you need.

Read the sections in order the first time: each one builds on the previous ones. After that, jump straight to the section for the part you are changing.

| # | Section | Read it to learn |
|---|---|---|
| 1 | [Overview](01-overview.md) | What Hytte is made of, the vocabulary used everywhere, and a map of the repository. |
| 2 | [Getting started](02-getting-started.md) | How to build, run, test and debug, including from a non-Windows machine. |
| 3 | [Architecture](03-architecture.md) | The two programs, every thread, and how an event travels from a terminal to a pixel. |
| 4 | [IPC protocol and task lifecycle](04-ipc-protocol.md) | The named pipe, the message format, validation, and how tasks are born, updated and expire. |
| 5 | [The `notch` CLI and shell integrations](05-cli.md) | Every `notch` subcommand and how the PowerShell / bash / zsh / Nushell hooks work. |
| 6 | [The UI model](06-ui-model.md) | `Model`, scenes, panels, the scene-priority rules, sizes, glow, and timed expiry. |
| 7 | [Window, input and timers](07-window-and-input.md) | The Win32 window, the message loop, hover and click handling, drag and drop, fullscreen. |
| 8 | [Rendering and animation](08-rendering-and-animation.md) | Direct2D drawing, the frame clock, springs, cross-fades, and why idle costs nothing. |
| 9 | [Background workers](09-workers.md) | Media, privacy, microphone, battery, ports, Windows Terminal tabs and thumbnails. |
| 10 | [Shelf and Drop Vault](10-shelf-and-drop-vault.md) | Storing files on the shelf and the image / JSON / PDF / OCR actions. |
| 11 | [Configuration and files on disk](11-configuration-and-files.md) | Every setting, every file Hytte writes, autostart and the Start Menu shortcut. |
| 12 | [Performance](12-performance.md) | The budgets, every source of idle wake-ups, and the rules that keep Hytte cheap. |
| 13 | [Testing and releasing](13-testing-and-release.md) | What the tests cover, how to run them, and how a release is built and published. |
| 14 | [How-to recipes](14-how-to-recipes.md) | Step-by-step guides for common changes: a new panel, event, setting, protocol field or file action. |

## Conventions used in these docs

- File paths are relative to the repository root, for example `crates/hytte/src/window.rs`.
- `Type::method` names refer to Rust items you can search for directly.
- "UI thread" always means the one thread that owns the window (see [section 3](03-architecture.md)).
- Numbers such as "120 ms" or "3 s" are the defaults in the code at the time of writing. When you change one, update the docs too.
