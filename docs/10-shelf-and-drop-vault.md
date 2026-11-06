# 10. Shelf and Drop Vault

The **shelf** holds files and text snippets dropped on the pill so they can be dragged out later. The **Drop Vault** is the set of one-click actions for a selected shelf item.

Code: `crates/hytte/src/shelf.rs`, `drop.rs`, `transforms.rs`, `convert.rs`; drag-and-drop plumbing is in `window.rs` ([section 7](07-window-and-input.md#drag-and-drop)).

## Shelf items

```rust
pub struct ShelfItem {
    pub id: u64,        // unique; based on the current time in nanoseconds
    pub path: PathBuf,  // the file the tile stands for
    pub name: String,   // display name
    pub is_dir: bool,
    pub owned: bool,    // lives in Hytte's own shelf folder (removing the tile deletes it)
}
```

`Model::shelf: Vec<ShelfItem>`, newest last, at most `[shelf] max_items` (12). The panel shows 5 tiles (`SHELF_VISIBLE`); arrows or the wheel over the row scroll a tile at a time (`Model::scroll_shelf`).

| `[shelf] mode` | A dropped file becomes | Removing the tile |
|---|---|---|
| `"reference"` (default) | A tile for the **original** path | Never touches the file. |
| `"copy"` | A tile for a **copy** in `%APPDATA%\Hytte\shelf` (`owned = true`) | Deletes the copy. |

Dropped **text** is always written to a `.txt` snippet in the shelf folder (`owned = true`), named from its first 24 letters, digits and spaces, so it drags out like a file.

## Adding items: stage, then insert

Adding can mean copying gigabytes, so it is split across threads:

```text
OLE Drop ──► UiEvent::ShelfAdd(DropJob) ──► UI thread: snapshot cfg, existing paths, free room
                                                │ spawn
                                                ▼
                               worker: shelf::stage(...)   ← copies files, writes snippets
                                                │ push_event
                                                ▼
              UI thread: UiEvent::ShelfStaged(items) ──► shelf::insert(...) → save, peek Shelf 5 s
```

- **`shelf::stage(cfg, paths, text, existing, room)`** does all file I/O: skips missing and already-shelved paths, stops at `room` items; in copy mode copies to a unique name (`name.ext`, `name-1.ext`, …), recursively for folders, removing a half-finished copy on failure. A process-wide mutex serialises "pick a free name + create it". Items come back with `id = 0`.
- `copy_recursive` guards: symlinks and junctions inside a folder are skipped, never followed; one drop copies at most 500 MB (`COPY_LIMIT`), over which the half copy is removed and a card says "Too big to copy to the shelf".
- **`shelf::insert(cfg, items, staged)`** runs on the UI thread. The shelf may have changed meanwhile, so it re-checks the limit and duplicates, deletes an owned copy that no longer fits (no leak), assigns ids and appends.

`add_paths` / `add_text` are blocking wrappers kept for tests.

### Persistence

With `[shelf] persist = true` the list is saved as JSON to `%APPDATA%\Hytte\shelf.json` after every change and loaded at startup (missing files dropped). After a drag-out the shelf also prunes items whose path vanished (a "move" drop moves the real file). Startup also deletes shelf-folder files no item points to (left by a crash between copying and saving), only when `shelf.json` was read and parsed cleanly. `shelf.json` and `timer.json` are written via temp file and rename (`setup::write_file`). `shelf::remove` deletes the file or folder only when `owned`; `shelf::remove_keep_file` removes just the entry (after a successful drag-out with `remove_after_drag`).

### Screenshots

With `[shelf] screenshots` on, a Snipping Tool screenshot (Win+Shift+S) goes on the shelf as a PNG in the shelf folder. `Ui::on_clipboard` (on `WM_CLIPBOARDUPDATE`) acts only when the clipboard owner is `SnippingTool.exe` or `ScreenClippingHost.exe` and a bitmap (`CF_DIB`) is there, skips an already-shelved image (hash of the bytes) and does nothing in game mode or paused. A worker converts it (`shelf::stage_dib`) and sends `UiEvent::Screenshot`, inserted like any staged item and tracked by `Ui::track_shot`:

- **Short stay.** After `screenshot_secs` (60; `0` keeps it) the tile is removed and the PNG deleted (`T_SHOT`, `Ui::tick_shots`). Only screenshots shelved while Hytte runs are tracked.
- **Delete after paste.** With `delete_after_paste` (default on), the newest screenshot is watched for Ctrl+V (`GetAsyncKeyState` every 60 ms, ≤ 10 minutes). The watch is dropped when anything else is copied (clipboard sequence number changes; the Snipping Tool re-touching the clipboard with the same picture moves the number it compares with). A Ctrl+V already held when the screenshot arrived doesn't count until released and pressed again (`paste_step`, a plain function with tests). About two seconds after a paste the tile is removed and its PNG deleted. Only a tile still on the shelf is deleted, so one dragged out meanwhile is safe, and only Hytte's own copy, never the Snipping Tool's saved file.

## The Drop Vault

`transforms::chips_for(path)` maps an extension to buttons:

| Extension | Buttons |
|---|---|
| `png`, `jpg`, `jpeg` | **Compress** · **To PDF** · **Remove metadata** · **Read text** · **Copy as base64** · **Copy as Markdown** |
| `webp`, `bmp` | the same without **Remove metadata** |
| `json`, `yaml`, `yml` | **Format** |
| `txt` | a decode action when the snippet is one · **Copy text** · **QR code** |
| `md` | **Copy text** |
| anything else | **Copy path** |

Each button carries a `Conv`: `Under(5)`, `Pdf`, `Clean`, `Ocr`, `Decode`, `Qr`, `Base64`, `Markdown` or `Auto`. An item with more actions than fit a line (an image has six) wraps onto a second line (`transforms::chip_lines`), growing the pane by 32 px.

- **QR code** (`transforms::qr_png`, `qrcodegen`): a text snippet becomes a black-on-white PNG, 8 px per module with a 4-module quiet zone, saved beside the snippet (or in `output_folder`) with **Open**. Over ~2,300 characters: "Too long for a QR code".
- **Copy as base64** puts a `data:image/png;base64,...` URI on the clipboard; only files ≤ 2 MB (`BASE64_MAX_BYTES`).
- **Copy as Markdown** puts `![name](C:/path/to/name.png)` (forward slashes; spaces and brackets escaped).

### How a job runs

1. A click runs `Action::ShelfOp(id, conv)`; the UI shows "Working…" and sends a `DropJob { paths, text, op }` to the Drop Vault channel.
2. One of **two worker threads** (`drop::spawn_workers`) calls `handle_job`, which calls `transforms::run(op, path)` per path.
3. It sends `UiEvent::SetClipboard` if the result has text, then `UiEvent::DropDone`; the UI replaces the chip with a summary plus **Open** / **Copy** / **Dismiss** (gone after 10 s).

`transforms::place(src, suffix, ext)` writes into `[general] output_folder` if it exists, else **next to the source**, never overwriting (`-1`, `-2`, … appended). Dropped files are never executed.

### The actions

| Action | Implementation | Output |
|---|---|---|
| **Compress** (`Under(5)`) | `convert::compress` | `name-5mb.jpg`; "Already under 5 MB" if small enough. |
| **To PDF** (`Pdf`) | `convert::to_pdf` | `name.pdf`, one image per page. |
| **Remove metadata** (`Clean`) | `transforms::clean_image` | `name.clean.png` / `.jpg`; re-encoding drops EXIF and GPS. |
| **Read text** (`Ocr`) | `transforms::ocr_image` | Text copied. |
| **Format** (JSON) | `toggle_json` | `name.pretty.json` if one line, `name.min.json` if already formatted; text also copied. |
| **Format** (YAML) | `normalize_yaml` | `name.pretty.yaml`: trailing spaces trimmed, tabs expanded, blank-line runs collapsed (whitespace only; no YAML parser). |
| **Decode JWT** · **To date** · **URL decode** · **Base64 decode** · **To RGB** (`Decode`) | `transforms::decode` | Offered for a `.txt` snippet (≤ 64 KB) that is a JWT, a 10- or 13-digit Unix time (2001–2100), text with `%XX` escapes, base64 that decodes to readable UTF-8, or a `#rgb` / `#rrggbb` / `#rrggbbaa` colour. The result (pretty JWT header and payload, local date-time, `rgb(...)`) is copied; a JWT's card says when it expires or expired. |
| **Copy text** | `transform_text` | File contents copied, JSON formatted on the way. |
| **Copy path** | `transform_file` | Path copied; Explorer opens with the file selected. |

## Compress: "make it under N MB" (`convert.rs`)

1. **Refuse huge images** before decoding: over 60 megapixels (`MAX_PIXELS`) would need hundreds of MB.
2. **Flatten** onto white (`flatten`) so transparent PNGs don't turn black as JPEG; opaque images convert directly, RGBA8 is read in place (no extra full-size copy), and the decoded original is freed before encoding.
3. **Quality search** (`best_quality`): try 90; if too big try 45; if 45 fits, binary-search the highest fitting quality (≤ ~6 encodes).
4. **Downscale** (`fit_jpeg`): if even 45 is too big, scale by `sqrt(target / size) × 0.95` and repeat, up to 4 rounds.

## To PDF: a hand-written writer (`convert.rs`)

`pdf_from_pages` writes PDFs itself rather than pulling in a library: objects are 1 catalog, 1 page tree, then a page, a content stream and an image XObject per image. Each page is A4 (landscape for wide images) with the image fitted in a 24 pt margin and centred (`place`). **JPEG** files are embedded **as-is** (`/DCTDecode`: no recompression; only the header is read, `jpeg_header`); **other images** are flattened to RGB and zlib-compressed with `miniz_oxide` (`/FlateDecode`). The cross-reference table records each object's byte offset; `pdf_structure_and_xref_offsets_resolve` checks every offset.

## Read text: OCR (`transforms::ocr_windows`)

Uses the built-in `Windows.Media.Ocr` engine with the user's profile languages: no bundled model, so it needs an OCR-capable language pack or reports "OCR unavailable". It opens the file as a `StorageFile`, decodes it with `BitmapDecoder`, refuses images over the engine's `MaxImageDimension`, and runs `RecognizeAsync`, every step in `winrt::block_on` with a 15 s timeout.

## Known limits

Lossless WebP output only (`image` crate); no lossy WebP or AVIF. YAML formatting is whitespace-only. Images dragged from a browser (image data, not a file) are not accepted.

Next: [11. Configuration and files on disk](11-configuration-and-files.md)
