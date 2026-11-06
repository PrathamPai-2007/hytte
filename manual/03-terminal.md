# 3. Your terminal

<img src="../docs/img/task.png" alt="The pill showing two running tasks" width="420">

## Track every slow command

Tray → **Set up terminal integration** (or run `hytte setup`), then open a new terminal.

Anything that runs longer than 3 seconds now appears on the pill by itself. Editors and pagers such as `vim`, `less` and `ssh` are ignored. To remove the integration, choose the tray item again or run `hytte setup --undo`.

This sets up PowerShell and Git Bash. For zsh or Nushell, add one line yourself and open a new terminal:

| Shell | Line | File |
|---|---|---|
| PowerShell | `hytte init pwsh \| Out-String \| Invoke-Expression` | `$PROFILE` |
| bash / Git Bash | `eval "$(hytte init bash)"` | `~/.bashrc` |
| zsh | `eval "$(hytte init zsh)"` | `~/.zshrc` |
| Nushell | `hytte init nu \| save -f ~/.config/nushell/hytte.nu`, then `source` it | `config.nu` |

If PowerShell says scripts are disabled, run this once:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

If you use Starship, put the line *before* `starship init` in bash and *after* it in PowerShell.

## Track one command

Put `hytte run --` in front of it:

```powershell
hytte run -- npm run build
```

Your command runs as normal and keeps its exit code. Compared with automatic tracking, you also get:

- **A real progress bar**, when the output has a percentage (`45%`) or a step count (`[12/345]`). Use `--no-progress` to ignore those.
- **The latest output line** under the task.
- **Copy error**, with the end of the output when it fails.

Some tools only print progress in a real terminal, so ask for it: `hytte run -- git clone --progress <url>`.

## What you can do with a task

<img src="../docs/img/tasks-open.png" alt="The task list opened up" width="420">

- **Click a task** to jump to its terminal, including the right Windows Terminal tab.
- **Rerun** a failed command.
- **Copy error** from a failed `hytte run` command.

A command you stop with Ctrl+C doesn't count as failed.

## Use Hytte from your own scripts

```powershell
hytte set --progress 40 --label "Uploading" --task upload-1   # same --task updates one entry
hytte set --done --label "Uploading" --task upload-1          # green tick (--fail for a red cross)
hytte notify "Backup finished"                                # a card on the pill
hytte shelf add .\report.pdf                                  # or: git diff | hytte shelf add
hytte ports                                                   # list dev servers
hytte kill :3000                                              # stop what's on port 3000 (asks first)
```

Next: [4. AI agents](04-ai-agents.md)
