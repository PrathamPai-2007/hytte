# 15. Updates

Hytte can update itself: it asks GitHub for the latest release, offers it on the pill, downloads the new `hytte.exe`, checks that it runs and reports the expected version, and swaps it into the same folder. Code: `crates/hytte/src/update.rs`.

## When it checks

| Trigger | Code | Behaviour |
|---|---|---|
| Daily background check | `update::spawn_checker` | Starts 60 s after the daemon, then every 24 h. Each check is skipped while `[general] check_updates` is false (`update::ENABLED`, live from config). Quiet when up to date or offline, and skips a version put off with **Later**. |
| Tray → **Check for updates…** | `update::check_now` (menu id 15) | Always runs. Reports "You're up to date" and errors as a card, and re-offers a version that was put off. |

One check at a time (`BUSY`).

## What a check does

1. **Ask GitHub.** `curl.exe` fetches `https://api.github.com/repos/PrathamPai-2007/hytte/releases/latest` (latest published, non-draft release). `release_from_json` takes `tag_name` and the URL of the asset named exactly `hytte.exe`; a release without it is an error.
2. **Compare.** `parse` turns `v1.2.3` (optional `v`, optional ignored `-suffix`) into `(1, 2, 3)`, compared numerically; `v1.2` or `latest` is not a version. Not newer than `CARGO_PKG_VERSION` ends the check.
3. **Offer it.** The release goes into `OFFER` and the pill gets `UiEvent::UpdateOffer { tag, note }`. `Ui::show_update` shows a chip, "Hytte X is available", with the first real line of the release notes (no heading, bullet or "by @user" tail, ≤ 140 chars; `first_note`), and **Update** / **Later**. No message box, no focus change. It waits if another card is up or Hytte is paused or a game is in front, and folds away after 45 s (the next daily check offers again). **Later** (`ChipAction::UpdateLater`) writes the tag to `%APPDATA%\Hytte\update-skip`.
4. **Install** (`update::install_pending` on **Update**; work in `install`), on a worker thread. A card says "Updating to X…"; any failure becomes a card:
   1. Write an empty `hytte.exe.new` next to the running exe. If that fails the folder isn't writable (for example `Program Files`); the card says so and points to a manual download.
   2. Download the asset into `hytte.exe.new`.
   3. **Verify it runs.** `hytte.exe.new --version` (killed after 10 s) must print exactly `hytte <tag without v>`. This catches a truncated download or a tag that doesn't match its build.
   4. **Swap.** Rename `hytte.exe` → `hytte.exe.old` (a running exe can be renamed, not overwritten), then `hytte.exe.new` → `hytte.exe`; if the second fails the first is undone.
5. **Restart.** `spawn_detached` starts the new exe, then `window::quit()` (same clean exit as tray Quit). The new process waits up to 5 s for the single-instance mutex, then `update::cleanup` deletes `hytte.exe.old`.

The exe keeps its path, so autostart, the Start Menu shortcut and the `PATH` entry keep working. Every step is logged to `hytte.log` with an `update:` prefix.

## Why curl.exe

Windows 10 1803+ ships `curl.exe` in `System32`. Using it by absolute path from `%SystemRoot%` (so another `curl` on `PATH` can't be picked up) means Hytte links no HTTP or TLS library. It runs with `CREATE_NO_WINDOW`, a 120 s limit, a 64 MB size limit, and in release builds `--proto =https --proto-redir =https` (plain HTTP refused, also after a redirect). HTTPS to `api.github.com` and `github.com`, only for this feature, is Hytte's only network traffic.

## Signing

Hytte is **not** code-signed (no Authenticode certificate), so SmartScreen may warn about a freshly downloaded `hytte.exe`. Updates are not signature-checked: trust rests on HTTPS to GitHub and the `--version` check.

Version 1.6.0 was the exception. It required a minisign signature, so releases up to this one carried `hytte.exe.minisig`. Releases no longer do, so a 1.6.0 install can't update itself and needs a manual download.

## Releasing so the updater can see it

The release workflow ([section 13](13-testing-and-release.md#how-a-release-is-built)) uploads `hytte.exe` as its own asset next to the zip and refuses to publish if the exe's `--version` doesn't match the tag. A release tagged `v1.2.0` must be built from a commit whose `crates/hytte/Cargo.toml` says `1.2.0`. Drafts and pre-releases are never returned by `releases/latest`, so never offered.

## Testing an update

Only a **debug** build honours the testing variables, so nobody can redirect a release build:

- `HYTTE_UPDATE_API`: replaces the GitHub URL; curl reads `file://` (debug builds don't force HTTPS).
- `hytte __update`: runs check, download, verification and install in the terminal, without the pill, printing `installed vX` or `refused: why`.

A full local run:

```powershell
$t = "$env:TEMP\hytte-upd"; mkdir "$t\inst", "$t\srv" -Force | Out-Null
# 1. The "installed" build: whatever is in Cargo.toml now.
cargo build; Copy-Item target\debug\hytte.exe "$t\inst\"
# 2. The "new release": same code, version 9.9.9. Edit crates/hytte/Cargo.toml to 9.9.9, then:
cargo build; Copy-Item target\debug\hytte.exe "$t\srv\"      # ...and put the version back.
# 3. A fake "latest release" answer.
$u = { param($f) "file:///" + ("$t\srv\$f" -replace '\\','/') }
@{ tag_name = "v9.9.9"; assets = @(
  @{ name = "hytte.exe"; browser_download_url = (& $u "hytte.exe") }) } |
  ConvertTo-Json -Depth 4 | Set-Content "$t\srv\latest.json"
# 4. Run the update from the installed copy.
$env:HYTTE_UPDATE_API = & $u "latest.json"
& "$t\inst\hytte.exe" __update
```

Check that:

- It prints `installed v9.9.9`; `inst\hytte.exe --version` says 9.9.9 and `hytte.exe.old` is left beside it.
- A tag that doesn't match the exe (say `v9.9.8` pointing at the 9.9.9 build) is refused with "the download doesn't look right".
- A folder you can't write to (`icacls <dir> /deny "%USERNAME%:(W,AD)"`) gives a clear error and changes nothing.

To see the chip, run the daemon from `inst` with the variable set and use the tray's **Check for updates…** (quit any installed Hytte first).

The unit tests in `update.rs` cover the version parser, the release-notes line and the release JSON.

Back to the [index](README.md).
