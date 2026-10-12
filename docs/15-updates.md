# 15. Updates

Hytte can update itself. It asks GitHub for the latest release, offers it, downloads the new `hytte.exe` and swaps it into the same folder. Everything lives in `crates/hytte/src/update.rs`.

<p align="center"><img src="img/update.png" alt="The update prompt" width="420"></p>

## When it checks

| Trigger | Code | Behaviour |
|---|---|---|
| Daily background check | `update::spawn_checker` | Starts 60 s after the daemon (so it never slows startup), then repeats every 24 h. Only if `[general] check_updates = true` (the default). Quiet: says nothing when up to date or offline, and skips a version the user already declined. |
| Tray → **Check for updates…** | `update::check_now` (menu id 15) | Always runs. Reports "You're up to date" and errors too, and asks again even about a declined version. |

Only one check runs at a time (`BUSY`). A manual check while the daily one's prompt is open does nothing.

## What a check does

1. **Ask GitHub.** `curl.exe` fetches `https://api.github.com/repos/PrathamPai-2007/hytte/releases/latest` (the most recent published, non-draft release). `release_from_json` takes `tag_name` and the download URL of the asset named exactly `hytte.exe`. A release without that asset (every release before 1.0) is an error, not an update.
2. **Compare.** `parse` turns `v1.2.3` (an optional `v`, an optional `-suffix` ignored) into `(1, 2, 3)`, compared as numbers. Anything else, such as `v1.2` or `latest`, is not a version and is ignored. If the release isn't newer than `CARGO_PKG_VERSION`, the check ends.
3. **Ask the user.** A topmost message box (`setup::message`, the same helper the terminal setup uses): "Hytte X is available (you have Y). Update now?" On **No**, the tag is written to `%APPDATA%\Hytte\update-skip`.
4. **Install** (`install`), on **Yes**:
   1. Write an empty `hytte.exe.new` next to the running exe. If that fails, the folder isn't writable (for example under `Program Files`), and the user gets a message saying so and pointing to a manual download.
   2. Download the asset into `hytte.exe.new`.
   3. **Verify it.** Run `hytte.exe.new --version`. The output must be exactly `hytte <tag without v>`. This catches a truncated or wrong download, an HTML error page, and a release whose tag doesn't match its build. On mismatch the file is deleted and nothing changes.
   4. **Swap.** Rename `hytte.exe` → `hytte.exe.old` (Windows lets you rename a running exe, but not overwrite it), then `hytte.exe.new` → `hytte.exe`. If the second rename fails, the first is undone.
5. **Restart.** Start the new `hytte.exe` detached (`spawn_detached`), then `window::quit()`, which is the same clean exit as the tray's Quit. The new process waits up to 5 s for the single-instance mutex while the old one finishes, then deletes `hytte.exe.old` (`update::cleanup`).

The install keeps the exe's path. So the autostart `Run` value, the Start Menu shortcut and the `PATH` entry all keep working without being rewritten.

Every step is logged to `hytte.log` with an `update:` prefix.

## Why curl.exe

Windows 10 1803 and newer ship `curl.exe` in `System32`. Using it, by absolute path built from `%SystemRoot%` so a different `curl` on `PATH` can't be picked up, means Hytte links no HTTP or TLS library at all. It runs with `CREATE_NO_WINDOW`, with a 120 s limit per request. HTTPS to `api.github.com` and `github.com` is the only network traffic Hytte makes, and only for this feature.

## Releasing so the updater can see it

The release workflow ([section 13](13-testing-and-release.md#how-a-release-is-built)) uploads `hytte.exe` as its own asset next to the zip, and refuses to publish if the exe's `--version` doesn't match the tag. A release tagged `v1.2.0` must therefore be built from a commit whose `crates/hytte/Cargo.toml` says `1.2.0`. Drafts and pre-releases are not returned by `releases/latest`, so they are never offered.

## Testing an update

`HYTTE_UPDATE_API` replaces the GitHub URL, and curl reads `file://` URLs. A full local run:

```powershell
$t = "$env:TEMP\hytte-upd"; mkdir "$t\inst", "$t\srv", "$t\appdata\Hytte" -Force | Out-Null
# 1. The "installed" build: whatever is in Cargo.toml now.
cargo build --release; Copy-Item target\release\hytte.exe "$t\inst\"
# 2. The "new release": same code, version 9.9.9, built into its own target folder.
#    Edit crates/hytte/Cargo.toml to 9.9.9, then:
$env:CARGO_TARGET_DIR = "$t\tgt"; cargo build --release; Remove-Item Env:CARGO_TARGET_DIR
#    ...and put the version back.
Copy-Item "$t\tgt\release\hytte.exe" "$t\srv\"
# 3. A fake "latest release" answer pointing at it.
@{ tag_name = "v9.9.9"; assets = @(@{ name = "hytte.exe"; browser_download_url = "file:///$("$t\srv\hytte.exe" -replace '\\','/')" }) } |
  ConvertTo-Json -Depth 4 | Set-Content "$t\srv\latest.json"
"[general]`nadd_to_path = false`n" | Set-Content "$t\appdata\Hytte\config.toml"
# 4. Run it isolated, then use the tray's "Check for updates…".
$env:APPDATA = "$t\appdata"; $env:HYTTE_UPDATE_API = "file:///$("$t\srv\latest.json" -replace '\\','/')"
& "$t\inst\hytte.exe"
```

Then check that:

- **Yes** replaces `inst\hytte.exe` (`--version` says 9.9.9), the old process exits, the new one runs, and `hytte.exe.old` is gone.
- **No** writes `update-skip`, and the daily check won't ask about 9.9.9 again.
- A tag that doesn't match the exe (say `v9.9.8` pointing at the 9.9.9 build) is refused with "the download doesn't look right", and nothing changes.
- A folder you can't write to (`icacls <dir> /deny "%USERNAME%:(W,AD)"`) gives a clear error, and nothing changes.

Quit any installed Hytte first, or the test copy exits as a second instance.

Back to the [index](README.md).
