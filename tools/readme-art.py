# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Draw the README's header: the wordmark from tools/site-art.py as the
site shows it, lit LEDs on a black plate with an amber glow.

    uv run tools/readme-art.py [WORDMARK] [OUT]

WORDMARK defaults to target/site-art/wordmark.png, OUT to
docs/images/wordmark.png. GitHub can't use the site's CSS, so this does in
pixels what .hero .plate and .led do there.
"""

import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
CELL = 12          # pixels per LED
PAD = (4, 3)       # plate padding, in LEDs (x, y)
GLOW = 48          # room around the plate for the glow
UNLIT = (29, 29, 32)
AMBER = (247, 165, 49)


def led(colour: tuple[int, int, int]) -> Image.Image:
    """One cell: a round dot that fades to black at its edge."""
    big = CELL * 4
    dot = Image.new("RGB", (big, big))
    d = ImageDraw.Draw(dot)
    for i in range(big // 2, 0, -1):
        t = i / (big / 2)
        k = 1.0 if t < 0.62 else max(0.0, 1 - (t - 0.62) / 0.38 * 0.85)
        c = tuple(int(v * k) for v in colour)
        d.ellipse((big / 2 - i, big / 2 - i, big / 2 + i, big / 2 + i), fill=c)
    return dot.resize((CELL, CELL), Image.LANCZOS)


def main() -> None:
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target/site-art/wordmark.png"
    out = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "docs/images/wordmark.png"
    word = Image.open(src).convert("RGB")
    cols, rows = word.width + 2 * PAD[0], word.height + 2 * PAD[1]
    panel = Image.new("RGB", (cols * CELL, rows * CELL))
    lit, dark = {}, led(UNLIT)
    for y in range(rows):
        for x in range(cols):
            wx, wy = x - PAD[0], y - PAD[1]
            c = word.getpixel((wx, wy)) if 0 <= wx < word.width and 0 <= wy < word.height else (0, 0, 0)
            cell = dark if c == (0, 0, 0) else lit.setdefault(c, led(c))
            panel.paste(cell, (x * CELL, y * CELL))
    # A little bloom, as the eye sees a lit LED.
    panel = ImageChops.screen(panel, panel.filter(ImageFilter.GaussianBlur(CELL * 0.6)).point(lambda v: v * 0.55))

    w, h = panel.width + 2 * GLOW, panel.height + 2 * GLOW
    plate = (GLOW, GLOW, GLOW + panel.width, GLOW + panel.height)
    radius = CELL * 2
    glow = Image.new("L", (w, h))
    ImageDraw.Draw(glow).rounded_rectangle(plate, radius, fill=110)
    glow = glow.filter(ImageFilter.GaussianBlur(GLOW / 2.5))
    img = Image.new("RGBA", (w, h), AMBER + (0,))
    img.putalpha(glow)
    mask = Image.new("L", (w, h))
    ImageDraw.Draw(mask).rounded_rectangle(plate, radius, fill=255)
    framed = Image.new("RGBA", (w, h))
    framed.paste(panel, (GLOW, GLOW))
    img.paste(framed, (0, 0), mask)
    ImageDraw.Draw(img).rounded_rectangle(plate, radius, outline=(42, 45, 51, 255), width=2)
    img.save(out, optimize=True)
    print(f"{out}: {w}x{h}, {out.stat().st_size:,} B")


if __name__ == "__main__":
    main()
