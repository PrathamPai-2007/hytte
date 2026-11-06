# 15. Updates

Hytte can update itself. It asks GitHub for the latest release, offers it on the pill, downloads the new `hytte.exe`, checks that it was signed by the Hytte release key and swaps it into the same folder. Everything lives in `crates/hytte/src/update.rs`.

## When it checks

| Trigger | Code | Behaviour |
|---|---|---|
| Daily background check | `update::spawn_checker` | Starts 60 s after the daemon (so it never slows startup), then repeats every 24 h. The thread always runs; each daily check is skipped while `[general] check_updates` is false (`update::ENABLED`, set live from the config). Quiet: says nothing when up to date or offline, and skips a version the user already put off with **Later**. |
| Tray → **Check for updates…** | `update::check_now` (menu id 15) | Always runs. Reports "You're up to date" and errors too (as a card on the pill), and offers again even a version that was put off. |

Only one check runs at a time (`BUSY`).

## What a check does

1. **Ask GitHub.** `curl.exe` fetches `https://api.github.com/repos/PrathamPai-2007/hytte/releases/latest` (the most recent published, non-draft release). `release_from_json` takes `tag_name`, the download URL of the asset named exactly `hytte.exe` and the one of its signature, `hytte.exe.minisig`. A release without either is an error, not an update: an unsigned release can never be installed.
2. **Compare.** `parse` turns `v1.2.3` (an optional `v`, an optional `-suffix` ignored) into `(1, 2, 3)`, compared as numbers. Anything else, such as `v1.2` or `latest`, is not a version and is ignored. If the release isn't newer than `CARGO_PKG_VERSION`, the check ends.
3. **Offer it.** The release goes into `OFFER` and the pill gets `UiEvent::UpdateOffer { tag, note }`. The window (`Ui::show_update`) shows a chip, "Hytte X is available", with the first real line of the release notes under it (without the heading, bullet and "by @user" tail of GitHub's generated notes, at most 140 characters; `first_note`) and two buttons, **Update** and **Later**. There is no message box and the chip never takes focus. It waits if another card is up, and while Hytte is paused or a game is in front; it shows when that ends. The chip folds away after 45 s without an answer (the next daily check offers it again). **Later** (`ChipAction::UpdateLater`) writes the tag to `%APPDATA%\Hytte\update-skip`.
4. **Install** (`update::install_pending`, on **Update**; the work is in `install`), on a worker thread. A card says "Updating to X…", and any failure becomes a card too:
   1. Write an empty `hytte.exe.new` next to the running exe. If that fails, the folder isn't writable (for example under `Program Files`), and the card says so and points to a manual download.
   2. Download the asset into `hytte.exe.new`, and the signature (a small text file) into memory.
   3. **Verify the signature.** `verify` checks the downloaded bytes against the signature with the **public key built into this exe** (`PUBKEY`, the `minisign-verify` crate). A bad signature, a signature from another key, a tampered file or a missing signature deletes `hytte.exe.new` and changes nothing.
   4. **Verify it runs.** `hytte.exe.new --version` (killed after 10 s if it hangs) must print exactly `hytte <tag without v>`. This catches a truncated download and a release whose tag doesn't match its build, and it stops an old, correctly signed build from being passed off as a new one.
   5. **Swap.** Rename `hytte.exe` → `hytte.exe.old` (Windows lets you rename a running exe, but not overwrite it), then `hytte.exe.new` → `hytte.exe`. If the second rename fails, the first is undone.
5. **Restart.** Start the new `hytte.exe` detached (`spawn_detached`), then `window::quit()`, which is the same clean exit as the tray's Quit. The new process waits up to 5 s for the single-instance mutex while the old one finishes, then deletes `hytte.exe.old` (`update::cleanup`).

The install keeps the exe's path. So the autostart `Run` value, the Start Menu shortcut and the `PATH` entry all keep working without being rewritten.

Every step is logged to `hytte.log` with an `update:` prefix.

## Why curl.exe

Windows 10 1803 and newer ship `curl.exe` in `System32`. Using it, by absolute path built from `%SystemRoot%` so a different `curl` on `PATH` can't be picked up, means Hytte links no HTTP or TLS library at all. It runs with `CREATE_NO_WINDOW`, with a 120 s limit and a 64 MB size limit per request, and (in a release build) `--proto =https --proto-redir =https`: plain HTTP is refused, also after a redirect. HTTPS to `api.github.com` and `github.com` is the only network traffic Hytte makes, and only for this feature.

## Signing, and the key

Hytte is **not** code-signed for Windows (no Authenticode certificate), so SmartScreen may warn about a freshly downloaded `hytte.exe`. The update path doesn't depend on that: every release's `hytte.exe` is signed with [minisign](https://jedisct1.github.io/minisign/) and checked before it runs, so a hijacked download is refused.

- **The public key** is the `PUBKEY` constant in `update.rs` (and the `.pub` file's second line). It is part of every build.
- **The secret key** is only in the repository's Actions secrets, as `MINISIGN_SECRET_KEY` (the whole key file, an unencrypted `rsign generate -W` key). Keep an offline copy somewhere safe, such as a password manager.
- **The release job** (`release.yml`, "Sign hytte.exe") installs `rsign2` (pinned), signs `hytte.exe` into `hytte.exe.minisig`, and checks that the signature verifies with the `PUBKEY` found in `update.rs`, so a key mismatch fails the release before anything is published. Both files are uploaded as release assets.
- **Making a key** (once): `cargo install rsign2 --locked`, then `rsign generate -W -p hytte.pub -s hytte.key`. Copy the `RW...` line of `hytte.pub` into `PUBKEY`, and put the content of `hytte.key` into the `MINISIGN_SECRET_KEY` secret.
- **If the secret key is lost or leaks:** make a new key, change `PUBKEY`, release. Versions that have the old key can't verify the new release, so people on them update by hand once (download from the releases page); every version from then on updates itself again.

## Releasing so the updater can see it

The release workflow ([section 13](13-testing-and-release.md#how-a-release-is-built)) uploads `hytte.exe` and `hytte.exe.minisig` as their own assets next to the zip, and refuses to publish if the exe's `--version` doesn't match the tag or its signature doesn't verify. A release tagged `v1.2.0` must therefore be built from a commit whose `crates/hytte/Cargo.toml` says `1.2.0`. Drafts and pre-releases are not returned by `releases/latest`, so they are never offered.

## Testing an update

Only a **debug** build (the default `cargo build`) honours the testing variables, so no one can redirect a release build with an environment variable:

- `HYTTE_UPDATE_API`: replaces the GitHub URL. curl reads `file://` URLs (debug builds don't force HTTPS).
- `HYTTE_UPDATE_PUBKEY`: the public key to trust instead of `PUBKEY`.
- `hytte __update`: runs check, download, verification and install in the terminal, without the pill, and prints `installed vX` or `refused: why`.

A full local run, with a throwaway key:

```powershell
$t = "$env:TEMP\hytte-upd"; mkdir "$t\inst", "$t\srv" -Force | Out-Null
# 1. The "installed" build: whatever is in Cargo.toml now.
cargo build; Copy-Item target\debug\hytte.exe "$t\inst\"
# 2. The "new release": same code, version 9.9.9. Edit crates/hytte/Cargo.toml to 9.9.9, then:
cargo build; Copy-Item target\debug\hytte.exe "$t\srv\"      # ...and put the version back.
# 3. Sign it with a throwaway key, and write a fake "latest release" answer.
rsign generate -W -f -p "$t\k.pub" -s "$t\k.key"
rsign sign -W -s "$t\k.key" -x "$t\srv\hytte.exe.minisig" "$t\srv\hytte.exe"
$u = { param($f) "file:///" + ("$t\srv\$f" -replace '\\','/') }
@{ tag_name = "v9.9.9"; assets = @(
  @{ name = "hytte.exe";         browser_download_url = (& $u "hytte.exe") },
  @{ name = "hytte.exe.minisig"; browser_download_url = (& $u "hytte.exe.minisig") }) } |
  ConvertTo-Json -Depth 4 | Set-Content "$t\srv\latest.json"
# 4. Run the update from the installed copy.
$env:HYTTE_UPDATE_API = & $u "latest.json"
$env:HYTTE_UPDATE_PUBKEY = (Get-Content "$t\k.pub")[1]
& "$t\inst\hytte.exe" __update
```

Then check that:

- It prints `installed v9.9.9`; `inst\hytte.exe --version` says 9.9.9 and `hytte.exe.old` is left beside it.
- Appending a byte to `srv\hytte.exe` (so it no longer matches its signature) prints `refused: signature check failed`, and nothing changes.
- A `latest.json` without the `hytte.exe.minisig` asset prints `refused: release v9.9.9 has no hytte.exe.minisig`.
- A tag that doesn't match the exe (say `v9.9.8` pointing at the 9.9.9 build, signed) is refused with "the download doesn't look right".
- A folder you can't write to (`icacls <dir> /deny "%USERNAME%:(W,AD)"`) gives a clear error, and nothing changes.

To see the chip itself, run the daemon from `inst` with the same two variables set and use the tray's **Check for updates…**. Quit any installed Hytte first, or the test copy exits as a second instance.

The unit tests in `update.rs` cover the version parser, the release notes line, the release JSON (including a release with no signature) and `verify` with a good signature, a tampered file, another key's signature and a missing one.

Back to the [index](README.md).
