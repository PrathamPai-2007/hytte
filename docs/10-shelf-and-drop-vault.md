# 10. Shelf and Drop Vault

The **shelf** holds files and text snippets the user drops on the pill, so they can be dragged out again later. The **Drop Vault** is the set of one-click actions offered for a selected shelf item.

Code: `crates/hytte/src/shelf.rs`, `drop.rs`, `transforms.rs`, `convert.rs`. The drag-and-drop plumbing itself is in `window.rs` ([section 7](07-window-and-input.md#drag-and-drop)).

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

The shelf is `Model::shelf: Vec<ShelfItem>`, newest last, at most `[shelf] max_items` (12). The expanded panel shows 5 tiles at a time (`SHELF_VISIBLE`); past that, arrows at the ends or the mouse wheel over the row scroll it a tile at a time (`Model::scroll_shelf`).

### Two modes

| `[shelf] mode` | A dropped file becomes | Removing the tile |
|---|---|---|
| `"reference"` (default) | A tile pointing at the **original** path | Never touches the file. |
| `"copy"` | A tile pointing at a **copy** in `%APPDATA%\Hytte\shelf` (`owned = true`) | Deletes the copy. |

Dropped **text** is always written to a `.txt` snippet in the shelf folder (`owned = true`). Its name comes from the first 24 letters, digits and spaces of the text, so it can be dragged out like any file.

## Adding items: stage, then insert

Adding can mean copying gigabytes, so it is split into two steps that run on different threads:

```text
OLE Drop ──► UiEvent::ShelfAdd(DropJob) ──► UI thread: snapshot cfg, existing paths, free room
                                                │ spawn
                                                ▼
                               worker: shelf::stage(...)   ← copies files, writes snippets
                                                │ push_event
                                                ▼
              UI thread: UiEvent::ShelfStaged(items) ──► shelf::insert(...) → save, peek Shelf 5 s
```

- **`shelf::stage(cfg, paths, text, existing, room)`** does all the file I/O. It skips missing paths and paths already on the shelf, and stops at `room` items. In copy mode it copies into a unique name (`name.ext`, `name-1.ext`, ...), recursively for folders, and removes a half-finished copy if copying fails. A process-wide mutex serialises "pick a free name + create it", so two drops at once can't collide. It returns items with `id = 0`.
- **`shelf::insert(cfg, items, staged)`** runs on the UI thread. The shelf may have changed while staging ran, so it checks the limit and duplicates again. An owned copy that no longer fits is deleted rather than leaked. It assigns ids and appends.

`add_paths` and `add_text` are blocking stage-and-insert wrappers, kept for tests.

### Persistence

With `[shelf] persist = true`, the list is saved as JSON to `%APPDATA%\Hytte\shelf.json` after every change and loaded at startup. On load, entries whose file has disappeared are dropped. After a tile is dragged out, the shelf also prunes items whose path no longer exists, since a "move" drop moves the real file.

### Screenshots

With `[shelf] screenshots` on, a screenshot the Snipping Tool copies (Win+Shift+S) goes on the shelf as a PNG in the shelf folder. `Ui::on_clipboard` runs on `WM_CLIPBOARDUPDATE`: it only acts when the clipboard owner is `SnippingTool.exe` or `ScreenClippingHost.exe` and a bitmap (`CF_DIB`) is there, skips an image it has already shelved (a hash of the bytes), and does nothing in game mode or while paused. A worker converts it (`shelf::stage_dib`) and sends `UiEvent::Screenshot`, which is inserted like any staged item and then tracked by `Ui::track_shot`:

- **Short stay.** After `[shelf] screenshot_secs` (60; `0` keeps it) the tile is removed and its PNG deleted (`T_SHOT`, `Ui::tick_shots`). Only screenshots shelved while Hytte is running are tracked, so ones left from before a restart stay until removed.
- **Delete after paste.** With `[shelf] delete_after_paste` on, the newest screenshot is watched for a Ctrl+V (`GetAsyncKeyState`, polled every 60 ms, for at most 10 minutes). The watch is dropped when anything else is copied (the clipboard sequence number changes). About two seconds after a paste (so the target app can read the image) the tile is removed and its PNG deleted. Only a tile still on the shelf is deleted, so a screenshot dragged out meanwhile is never touched, and only Hytte's own copy is deleted, never the Snipping Tool's own saved file.

### Removing

`shelf::remove` deletes the file or folder only when `owned`. `shelf::remove_keep_file` removes just the entry; it is used after a successful drag-out with `remove_after_drag`.

## The Drop Vault

### Which actions are offered

`transforms::chips_for(path)` maps a file extension to buttons:

| Extension | Buttons |
|---|---|
| `png`, `jpg`, `jpeg` | **Compress** · **To PDF** · **Remove metadata** · **Read text** |
| `webp`, `bmp` | **Compress** · **To PDF** · **Read text** |
| `json`, `yaml`, `yml` | **Format** |
| `txt` | A decode action when the snippet is one (see below) · **Copy text** |
| `md` | **Copy text** |
| anything else | **Copy path** |

Each button carries a `Conv` value: `Under(5)`, `Pdf`, `Clean`, `Ocr` or `Auto` (the per-type default).

### How a job runs

1. Clicking a button runs `Action::ShelfOp(id, conv)`. The UI shows a "Working…" chip and sends a `DropJob { paths, text, op }` to the Drop Vault channel.
2. One of the **two worker threads** (`drop::spawn_workers`) picks it up and calls `handle_job`, which calls `transforms::run(op, path)` for each path.
3. The worker sends `UiEvent::SetClipboard` if the result has text to copy, then `UiEvent::DropDone`. The UI replaces the chip with the result: a summary plus **Open** / **Copy** / **Dismiss** buttons. The chip disappears after 10 s.

### Where results go

`transforms::place(src, suffix, ext)` builds the output path in `[general] output_folder` if that folder exists, otherwise **next to the source**. It never overwrites: an existing name gets `-1`, `-2`, ... appended. Dropped files are never executed.

### The actions

| Action | Implementation | Output |
|---|---|---|
| **Compress** (`Conv::Under(5)`) | `convert::compress` | `name-5mb.jpg`. Skipped with "Already under 5 MB" if the file is small enough. |
| **To PDF** (`Conv::Pdf`) | `convert::to_pdf` | `name.pdf`, one image per page. |
| **Remove metadata** (`Conv::Clean`) | `transforms::clean_image` | `name.clean.png` or `.jpg`. Decoding and re-encoding drops EXIF and GPS. |
| **Read text** (`Conv::Ocr`) | `transforms::ocr_image` | Text copied to the clipboard. |
| **Format** (JSON) | `toggle_json` | `name.pretty.json` if it was one line, `name.min.json` if it was already formatted; text also copied. |
| **Format** (YAML) | `normalize_yaml` | `name.pretty.yaml` with trailing spaces trimmed, tabs expanded and blank-line runs collapsed. There is no YAML parser, so this is whitespace-only. |
| **Decode JWT** · **To date** · **URL decode** · **Base64 decode** · **To RGB** (`Conv::Decode`) | `transforms::decode` | Offered for a `.txt` snippet (64 KB at most) that is a JWT, a 10- or 13-digit Unix time (2001-2100), text with `%XX` escapes, base64 that decodes to readable UTF-8, or a `#rgb` / `#rrggbb` / `#rrggbbaa` colour. The result (JWT header and payload as pretty JSON, local date-time, `rgb(...)`) is copied; a JWT's card says when it expires or expired. |
| **Copy text** | `transform_text` | The file's contents copied, formatting JSON on the way. |
| **Copy path** | `transform_file` | The path copied, and Explorer opens with the file selected. |

## Compress: "make it under N MB" (`convert.rs`)

1. **Refuse huge images** before decoding: more than 60 megapixels (`MAX_PIXELS`) would need hundreds of MB of RAM.
2. **Flatten** onto white (`flatten`), so transparent PNGs don't turn black as JPEG. Opaque images convert directly, and RGBA8 images are read in place, so no extra full-size copy is made. The decoded original is then freed before encoding.
3. **Quality search** (`best_quality`): try quality 90. If it is still too big, try 45. If 45 fits, binary-search the highest quality in between that fits (at most about 6 encodes).
4. **Downscale** (`fit_jpeg`): if even quality 45 is too big, scale the image by `sqrt(target / size) × 0.95` (the area ratio, with headroom) and repeat. Up to 4 rounds.

## To PDF: a hand-written PDF writer (`convert.rs`)

Hytte writes PDFs itself (`pdf_from_pages`) rather than pulling in a PDF library. A PDF with one image per page is a small amount of text plus byte offsets:

- Objects: 1 catalog, 1 page tree, then a page, a content stream and an image XObject per image.
- Each page is A4, rotated to landscape for wide images, with the image fitted inside a 24 pt margin and centred (`place`).
- **JPEG** files are embedded **as-is** (`/DCTDecode`): no recompression and no quality loss. Only the header is read, for the size and colour type (`jpeg_header`).
- **Other images** are flattened to RGB and zlib-compressed with `miniz_oxide` (`/FlateDecode`).
- The cross-reference table records each object's byte offset. The test `pdf_structure_and_xref_offsets_resolve` checks every offset points at the right object.

## Read text: OCR (`transforms::ocr_windows`)

Uses the built-in `Windows.Media.Ocr` engine with the user's profile languages. There is no bundled model, so it needs an OCR-capable language pack, or it reports "OCR unavailable". It opens the file as a `StorageFile`, decodes it with `BitmapDecoder`, refuses images larger than the engine's `MaxImageDimension`, and runs `RecognizeAsync`. Every step is wrapped in `winrt::block_on` with a 15 s timeout.

## Known limits

- Lossless WebP output only (the `image` crate's encoder); lossy WebP and AVIF are not supported.
- YAML formatting is whitespace normalisation only.
- Images dragged from a browser (image data rather than a file) are not accepted.

Next: [11. Configuration and files on disk](11-configuration-and-files.md)
