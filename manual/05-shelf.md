# 5. Shelf and file actions

The shelf is a place to park files and text while you switch windows.

## Using the shelf

- **Add:** drag files, folders or selected text to the top of the screen and drop them on the pill.
- **Take out:** open the Shelf pane and drag a tile into any app that accepts files.
- **Remove:** use the tile's remove button. For a normal file, this only removes the tile.

The shelf holds up to 12 items and keeps them after a restart. A tile points at your original file, so nothing is copied unless you set `mode = "copy"` ([Settings](07-settings.md#editing-configtoml)).

Images dragged straight from a browser aren't supported yet, so save them first. Dropping only works on the monitor the pill is on.

## Screenshots

A Win+Shift+S screenshot lands on the shelf automatically. It leaves after a minute, or about two seconds after you paste it with Ctrl+V. Only Hytte's own copy is deleted, never a file you saved from the Snipping Tool.

## Files you copy

Turn on **Put files I copy with Ctrl+C on the shelf** in [Settings](07-settings.md) (`[shelf] shelve_copied = true`) and files you copy in Explorer land on the shelf, as links to the originals. Cutting (Ctrl+X) doesn't count, and neither does anything an app marks private. It's off by default.

## Clipboard history

**Keep a short clipboard history** (`[clipboard] history = true`, off by default) lists the last texts you copied under the shelf tiles; click one to copy it back. It keeps 20 clips of up to 4 KB in memory only (nothing is written to disk). Each clip is forgotten after 2 minutes (`[clipboard] expire_secs`; 0 keeps it until you lock the PC or it sleeps), and all of them go when you lock the PC, when it sleeps and when you switch the setting off. Hover a clip for **Copy** and **×** (delete just that one); **Clear** in the header forgets them all. `clear_when_emptied = true` also forgets them when the Windows clipboard is emptied (off by default, since apps empty it all the time). Text that looks like a password or token, anything a password manager copied (KeePass, KeePassXC, 1Password and Bitwarden are on the `ignore_apps` list) and anything an app marks as private are never kept. A plain password copied from a program that doesn't mark it private can't be told from ordinary text, so it is kept until it expires: copy those from a password manager. Windows has its own history too, on **Win+V**.

With [Ask](09-ask.md) on, hover a clip and click the **✦** to Explain, Summarise or Shorten it with a model on your PC.

## File actions

Click a tile to see what you can do with it. Results are saved next to the original, and your original is never overwritten.

| File | Actions |
|---|---|
| PNG, JPG | **Compress**, **To PDF**, **Remove metadata**, **Read text**, **Copy as base64**, **Copy as Markdown** |
| WebP, BMP | The same, without **Remove metadata** |
| JSON, YAML | **Format** |
| Text snippet | **Copy text**, **QR code**, and a decode action if it fits (below) |
| Markdown | **Copy text** |
| Anything else | **Copy path** |

- **Compress** makes an image smaller than 5 MB.
- **To PDF** puts the image on an A4 page.
- **Remove metadata** strips camera and location data.
- **Read text** copies the text in an image. It needs a Windows language pack with OCR.
- **Format** tidies a file. On JSON, it switches between pretty and one line.
- **QR code** turns a snippet of up to about 2,300 characters into a QR image.
- **Copy as base64** works on images up to 2 MB.

### Decoding text

Drop a piece of text on the pill and select it. If it's one of these, Hytte offers to decode it and copies the result:

| Text | Action |
|---|---|
| A JWT | **Decode JWT**, which also says when it expires |
| A Unix timestamp | **To date** |
| Text with `%20`-style escapes | **URL decode** |
| Base64 | **Base64 decode** |
| A hex colour such as `#ff8800` | **To RGB** |

To save results in one folder, set `output_folder` ([Settings](07-settings.md#editing-configtoml)).

Next: [6. Everyday tools](06-everyday-tools.md)
