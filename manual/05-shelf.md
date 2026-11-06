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
