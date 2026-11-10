# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Draw the OpenDMD mark and everything made from it.

    uv run tools/logo.py            # (make logo) rewrite the files below
    uv run tools/logo.py --favicon  # print the web app's favicon, a data: URL

The mark: "OD" in square pixels, the O in off-white and the D in orange, a
row of lit LEDs beneath, on near-black. It is rectangles and circles in a
500-unit square, drawn twice: as SVG and, with Pillow, as PNG.

- docs/images/logo.svg, logo-512.png, logo-1024.png: the mark on a rounded tile.
- docs/images/avatar-500.png, avatar-1024.png: full bleed, for an avatar
  (GitHub crops it to a circle or a rounded square itself).
- docs/images/social-preview.png: 1280x640, the mark beside the wordmark
  (read back from docs/images/wordmark.png, tools/readme-art.py's), for the
  repository's Settings > Social preview.
- opendmd-web/share/create/icon-192.png, icon-512.png: Create Art's app
  icons, the mark drawn smaller so that a maskable crop keeps all of it.

The web app's favicon is the two letters alone (the LED row is noise at
16 px), inlined in opendmd-web/www/index.html as a data: URL: no file, no
request, nothing in the www pack.
"""

import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
IMAGES = ROOT / "docs/images"
CREATE = ROOT / "opendmd-web/share/create"
ORANGE, BLACK, INK, UNLIT = "#ffb902", "#0d0e11", "#ecebe7", "#24262b"
O = ["####", "#..#", "#..#", "#..#", "####"]
D = ["###.", "#..#", "#..#", "#..#", "###."]
U = 36  # one pixel of the letters, in the 500-unit square

FAVICON = (
    "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 11 11'>"
    f"<rect width='11' height='11' rx='2' fill='{BLACK}'/>"
    f"<path fill='{INK}' fill-rule='evenodd' d='M1 3h4v5H1zm1 1v3h2V4z'/>"
    f"<path fill='{ORANGE}' fill-rule='evenodd' d='M6 3h3v1h1v3H9v1H6zm1 1v3h2V4z'/></svg>"
)


def mark(scale: float = 1.0, radius: float = 0) -> list[tuple]:
    """The mark's shapes: ("rect", x, y, w, h, r, colour) and ("circle", x, y, r, colour)."""
    u = U * scale
    x0, y0 = 250 - 4.5 * u, 250 - 3.28 * u
    shapes = [("rect", 0, 0, 500, 500, radius, BLACK)]
    for glyph, dx, colour in ((O, 0, INK), (D, 5, ORANGE)):
        for r, row in enumerate(glyph):
            for c, v in enumerate(row):
                if v == "#":  # a hair over a pixel wide, so no seams show between them
                    shapes.append(("rect", x0 + (dx + c) * u, y0 + r * u, u + 0.5, u + 0.5, 0, colour))
    for i in range(9):
        shapes.append(("circle", x0 + (i + 0.5) * u, y0 + 6.2 * u, u * 0.36, ORANGE))
    return shapes


def svg(shapes: list[tuple]) -> str:
    out = ['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 500" width="500" height="500">',
           "<title>OpenDMD</title>"]
    for s in shapes:
        if s[0] == "rect":
            _, x, y, w, h, r, colour = s
            rx = f' rx="{r:g}"' if r else ""
            out.append(f'<rect x="{x:g}" y="{y:g}" width="{w:g}" height="{h:g}"{rx} fill="{colour}"/>')
        else:
            _, x, y, r, colour = s
            out.append(f'<circle cx="{x:g}" cy="{y:g}" r="{r:g}" fill="{colour}"/>')
    return "\n".join(out + ["</svg>\n"])


def raster(shapes: list[tuple], size: int, over: int = 4) -> Image.Image:
    """The shapes at `size` pixels square, drawn `over` times larger and scaled down."""
    k = size * over / 500
    im = Image.new("RGBA", (size * over, size * over), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    for s in shapes:
        if s[0] == "rect":
            _, x, y, w, h, r, colour = s
            d.rounded_rectangle([x * k, y * k, (x + w) * k - 1, (y + h) * k - 1], r * k, fill=colour)
        else:
            _, x, y, r, colour = s
            d.ellipse([(x - r) * k, (y - r) * k, (x + r) * k, (y + r) * k], fill=colour)
    return im.resize((size, size), Image.LANCZOS)


def save(im: Image.Image, path: Path, colours: int = 0) -> None:
    """Write a PNG; `colours` > 0 for a palette (flat art: a fraction of the size)."""
    if colours:
        im = im.convert("RGB").quantize(colours, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE)
    im.save(path, optimize=True)
    print(f"{path.relative_to(ROOT)}: {path.stat().st_size} B")


def wordmark() -> list[list[tuple[int, int, int] | None]]:
    """The panel-font "OpenDMD" as LEDs (None: unlit), read back from the
    README's wordmark.png: tools/readme-art.py draws 12-pixel cells inside a
    48-pixel margin, the word 4 cells in and 3 down."""
    im = Image.open(IMAGES / "wordmark.png").convert("RGB")
    cell, margin, pad_x, pad_y = 12, 48, 4, 3
    cols, rows = (im.width - 2 * margin) // cell - 2 * pad_x, (im.height - 2 * margin) // cell - 2 * pad_y
    grid = []
    for y in range(rows):
        row = []
        for x in range(cols):
            p = im.getpixel((margin + (x + pad_x) * cell + cell // 2, margin + (y + pad_y) * cell + cell // 2))
            row.append(p if max(p) > 100 else None)
        grid.append(row)
    return grid


def social() -> Image.Image:
    """1280x640: a dim dot matrix, the mark on its tile, the wordmark lit beside it."""
    w, h, pitch, over = 1280, 640, 10, 3
    word = wordmark()
    tile = 250
    gap = 6 * pitch
    total = tile + gap + len(word[0]) * pitch
    wx = (w - total) // 2 // pitch * pitch + tile + gap
    wy = (h - len(word) * pitch) // 2
    im = Image.new("RGB", (w * over, h * over), BLACK)
    d = ImageDraw.Draw(im)
    for gy in range(h // pitch):
        for gx in range(w // pitch):
            c = None
            if 0 <= gy - wy // pitch < len(word) and 0 <= gx - wx // pitch < len(word[0]):
                c = word[gy - wy // pitch][gx - wx // pitch]
            r = pitch * (0.44 if c else 0.27)
            cx, cy = (gx + 0.5) * pitch, (gy + 0.5) * pitch
            d.ellipse([(cx - r) * over, (cy - r) * over, (cx + r) * over, (cy + r) * over], fill=c or UNLIT)
    im = im.resize((w, h), Image.LANCZOS)
    logo = raster(mark(radius=90), tile)
    im.paste(logo, (wx - gap - tile, (h - tile) // 2), logo)
    return im


def main() -> None:
    if "--favicon" in sys.argv:
        print("data:image/svg+xml," + FAVICON.replace("#", "%23").replace("<", "%3C").replace(">", "%3E"))
        return
    (IMAGES / "logo.svg").write_text(svg(mark(radius=90)))
    print(f"docs/images/logo.svg: {(IMAGES / 'logo.svg').stat().st_size} B")
    for n in (512, 1024):
        save(raster(mark(radius=90), n), IMAGES / f"logo-{n}.png")
    for n in (500, 1024):
        save(raster(mark(), n), IMAGES / f"avatar-{n}.png", colours=32)
    save(social(), IMAGES / "social-preview.png", colours=64)
    for n in (192, 512):  # the letters inside the maskable safe zone (the middle 80%)
        save(raster(mark(scale=0.86), n), CREATE / f"icon-{n}.png", colours=24)


if __name__ == "__main__":
    main()
