"""Regenerates the .ico files from logo.png. Run by hand after changing the logo:

    python crates/hytte/assets/make_icons.py

logo.png is a dark glyph on a near-white tile. The glyph is lifted out by darkness, so:
  hytte.ico       dark glyph on a white rounded tile (exe, window, Start Menu)
  tray-light.ico  dark glyph, transparent (light taskbar)
  tray-dark.ico   white glyph, transparent (dark taskbar)
"""

from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageOps

HERE = Path(__file__).parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 256]
INK = (26, 27, 33)


def glyph_mask() -> Image.Image:
    """Coverage of the glyph, 0..255, cropped square around it."""
    im = Image.open(HERE / "logo.png").convert("RGBA")
    lum = ImageOps.invert(im.convert("L"))  # glyph bright, background ~0
    # The background isn't pure white; stretch so it maps to 0 and the ink to 255.
    mask = lum.point(lambda v: max(0, min(255, (v - 40) * 255 // 160)))
    mask = ImageChops.multiply(mask, im.getchannel("A"))
    # Drop faint background noise, then crop to the solid ink.
    mask = mask.point(lambda v: 0 if v < 24 else v)
    l, t, r, b = mask.point(lambda v: 255 if v > 96 else 0).getbbox()
    side = max(r - l, b - t)
    cx, cy = (l + r) // 2, (t + b) // 2
    return mask.crop((cx - side // 2, cy - side // 2, cx + side // 2, cy + side // 2))


def solid(mask: Image.Image, rgb, size: int, pad: float) -> Image.Image:
    inner = max(1, round(size * (1 - 2 * pad)))
    m = mask.resize((inner, inner), Image.LANCZOS)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    fill = Image.new("RGBA", (inner, inner), rgb + (255,))
    fill.putalpha(m)
    off = (size - inner) // 2
    out.alpha_composite(fill, (off, off))
    return out


def tile(mask: Image.Image, size: int) -> Image.Image:
    big = size * 4
    out = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(out).rounded_rectangle(
        (0, 0, big - 1, big - 1), radius=big // 5, fill=(250, 250, 250, 255)
    )
    out.alpha_composite(solid(mask, INK, big, 0.18))
    return out.resize((size, size), Image.LANCZOS)


def save(frames, name):
    frames[-1].save(HERE / name, sizes=[(f.width, f.height) for f in frames],
                    append_images=frames[:-1])


if __name__ == "__main__":
    m = glyph_mask()
    save([tile(m, s) for s in SIZES], "hytte.ico")
    # Tray glyphs use nearly the whole square: at 16 px every pixel counts.
    save([solid(m, INK, s, 0.04) for s in SIZES], "tray-light.ico")
    save([solid(m, (255, 255, 255), s, 0.04) for s in SIZES], "tray-dark.ico")
    print("wrote hytte.ico, tray-light.ico, tray-dark.ico")
