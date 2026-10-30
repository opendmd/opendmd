# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""A short looping 128x32 scene from Kenney's Pixel Platformer (CC0, the
files beside this script): a green alien runs across a strip of grass,
past bushes, a mushroom and a sign, picking up spinning coins; once it is
gone the coins come back and it starts again.

    uv run scene.py OUT.gif

tools/site-art.py imports `scene()` for the site's "Using it" panel
("kenney-run"). Everything is placed by hand, no randomness: the same
pixels every time.
"""

import sys
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
W, H = 128, 32
MS = 40           # a frame
SPEED = 2         # px a frame (50 px/s)
GROUND = 26       # the top of the grass
TILE, CHAR = 18, 24


def tiles() -> tuple[Image.Image, Image.Image]:
    t = Image.open(HERE / "Tilemap/tilemap_packed.png").convert("RGBA")       # 20 x 9 tiles of 18 px
    c = Image.open(HERE / "Tilemap/tilemap-characters_packed.png").convert("RGBA")  # 9 x 3 of 24 px
    return t, c


def tile(sheet: Image.Image, n: int, size: int, cols: int) -> Image.Image:
    x, y = (n % cols) * size, (n // cols) * size
    return sheet.crop((x, y, x + size, y + size))


def scene() -> tuple[list[Image.Image], list[int]]:
    """The frames (RGB, 128x32) and their durations in ms."""
    t, c = tiles()
    T = lambda n: tile(t, n, TILE, 20)  # noqa: E731
    back = Image.new("RGBA", (W, H), (0, 0, 0, 255))
    # The grass: the top of the middle grass tile, across the panel.
    for x in range(0, W, TILE):
        back.alpha_composite(T(2).crop((0, 0, TILE, H - GROUND)), (x, GROUND))
    # Scenery standing on it (tile, x): bushes, a sprout, a mushroom, a sign.
    for n, x in ((124, 6), (125, 40), (128, 70), (124, 92), (88, 110)):
        back.alpha_composite(T(n), (x, GROUND - TILE))
    coins = [22, 52, 82]  # their left edges; at chest height
    run = [tile(c, 0, CHAR, 9), tile(c, 1, CHAR, 9)]
    frames: list[Image.Image] = []
    xs = list(range(-CHAR, W + 1, SPEED))
    for k, x in enumerate(xs + [W] * 12):  # then a moment with nobody there
        f = back.copy()
        gone = k >= len(xs)
        spin = T(151 if (k // 5) % 2 == 0 else 152)
        for cx in coins:
            # Picked up once the alien's middle passes it; back for the next run.
            if gone or x + CHAR // 2 < cx + 4:
                f.alpha_composite(spin, (cx, 8))
        if not gone:
            step = (k // 3) % 2
            f.alpha_composite(run[step], (x, GROUND - CHAR + (1 if step else 0)))
        frames.append(f.convert("RGB"))
    return frames, [MS] * len(frames)


def main() -> None:
    frames, durations = scene()
    frames[0].save(sys.argv[1], save_all=True, append_images=frames[1:], duration=durations, loop=0, optimize=True)
    print(f"{sys.argv[1]}: {len(frames)} frames at {MS} ms")


if __name__ == "__main__":
    main()
