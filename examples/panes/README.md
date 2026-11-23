# Example pane packs

A pane pack is a folder with a `pane.toml` and the scripts it runs. Each one here adds a tab to the Hytte pill. Use them as they are, or as a starting point for your own: the whole authoring guide (formats, limits, actions, debugging) is [docs/17-writing-panes.md](../../docs/17-writing-panes.md).

## Install one

```powershell
Copy-Item -Recurse .\disk-space "$env:APPDATA\Hytte\panes\"
```

Hytte notices the new folder at once and shows a card with the pane's command and **Run** / **Ignore**. Nothing runs until you press Run. Your answer is kept outside the folder (`%APPDATA%\Hytte\pack-answers\`) and tied to a hash of `pane.toml`, so editing the file asks again. The hash does not cover the scripts: read what a pack runs before you approve it. Delete the folder to remove the pane (it goes away at once).

A pane runs its command while you have it open (and once when Hytte starts), then again every `every` seconds; the small refresh button at the end of its footer runs it now. At most 12 custom panes exist, shared with any `[[script_pane]]` in `config.toml`.

## The gallery

| Pack | Shows | Needs | Shows off |
|---|---|---|---|
| [`github-prs`](github-prs) | your open pull requests (a click opens github.com/pulls) | the [GitHub CLI](https://cli.github.com) (`gh`), signed in | a row that runs an action |
| [`docker`](docker) | containers, unhealthy ones first; click an unhealthy one to restart just that container, or **Fix unhealthy** for all of them (both ask "Sure?") | Docker Desktop running | `arg` (the container's name reaches the action as `HYTTE_ARG`), a pane button, `confirm` |
| [`ci-status`](ci-status) | the latest GitHub Actions runs of one repository | `gh`, and `repo.txt` (copy `repo.txt.example`, put your `owner/name` in it) | reading a settings file next to the script |
| [`disk-space`](disk-space) | each drive's used space as a bar, amber then red as it fills | nothing | `progress` bars and `tone` |
| [`top-memory`](top-memory) | the five programs using the most RAM, a program's processes added together | nothing | bars, a short `every` (30 s, the minimum) |
| [`net-check`](net-check) | internet up or down, latency to 1.1.1.1, DNS working | nothing (contacts 1.1.1.1 and example.com) | three status rows, no actions |
| [`git-repos`](git-repos) | uncommitted and unpushed work in the repositories you list | `git`, and `repos.txt` (copy `repos.txt.example`) | a list the user edits |
| [`recycle-bin`](recycle-bin) | items and size in the Recycle Bin; a confirmed click empties it | nothing | `confirm = true` on an action |
| [`battery`](battery) | charge with a bar, charging or not, time left (says so on a desktop PC) | nothing | a pane that adapts to missing hardware |
| [`resources`](resources) | time since boot, CPU load and memory in use | nothing | tone from a threshold |
| [`top-cpu`](top-cpu) | the five programs using the most CPU over a second | nothing | sampling twice inside the 10 s limit |
| [`wifi`](wifi) | network name, signal bar, link speed | a Wi-Fi adapter, English Windows | parsing a command's text |
| [`gpu`](gpu) | load, video memory and temperature of NVIDIA GPUs | an NVIDIA GPU and its driver (`nvidia-smi`) | a missing tool shown as a row |
| [`time-zones`](time-zones) | the time in up to six places | nothing (`zones.txt` is optional) | an optional settings file with a default |
| [`github-notifications`](github-notifications) | your unread GitHub notifications (a click opens the inbox) | `gh`, signed in | an API call turned into rows |
| [`ssl-expiry`](ssl-expiry) | days left on the TLS certificate of each host you list | `hosts.txt` (copy `hosts.txt.example`) | doing several network calls at once inside the time limit |

## What a pack looks like

```toml
# pane.toml
name = "Docker"                       # the tab (default: the folder's name), 16 characters at most
command = "powershell -NoProfile -ExecutionPolicy Bypass -File containers.ps1"
every = 60                            # seconds; never below 30; default 300

[[action]]                            # up to 8 per pack; names are letters, digits, - and _ (19 characters)
name = "restart"                      # Hytte calls it docker-restart
command = "powershell -NoProfile -ExecutionPolicy Bypass -File restart.ps1"
confirm = true                        # the first click says "Sure?", the second runs it
timeout = 120                         # seconds until it is stopped (default 60, at most 600)
```

The command runs in the pack's folder (so `-File containers.ps1` finds the script next to `pane.toml`), hidden, and is stopped after 10 seconds. It prints one of:

- **Plain text:** up to six lines.
- **Rows:** `{"rows": [...], "buttons": [...]}`. Up to six rows with `text`, `value`, `progress` (0 to 100), `tone` (`good`, `warn` or `bad`), `action` and `arg`; up to three buttons with `label`, `action` and `arg`.

A row's or button's `action` names one of the pack's own `[[action]]`s **without** the prefix: Hytte adds the folder's name. Its `arg` reaches the action as the environment variable `HYTTE_ARG`: read it as `$env:HYTTE_ARG` in a PowerShell script and check it before you use it (see `docker\restart.ps1`); never paste it into a command line.

If a tool the pack needs is missing, the script prints a row that says so instead of failing, so the pane explains itself.

## Try one by hand

```powershell
cd "$env:APPDATA\Hytte\panes\disk-space"
cmd /d /s /c "powershell -NoProfile -ExecutionPolicy Bypass -File disks.ps1" | ConvertFrom-Json
```

That is exactly what Hytte runs. If `ConvertFrom-Json` complains, so would the pane (it would show the raw text instead of rows).
