# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.2.6", "pillow==11.3.0"]
# ///
"""Draw the home page's first scene: the games den at night (style `night`).

    uv run tools/site-scene.py [--preview DIR]      # make site-scene

Everyone's scene is now `synth`, one of the styles tools/site-scene-alt.py
draws; this one is shown to a viewer who asks for it (?scene=night,
site/theme/SCENES.md).

Original pixel art, drawn here from rectangles, lines and gradients: no
photograph, no model, no third-party art, no game's characters or logos.
Everything is painted as a brightness (0..1) on one of two colour ramps,
cool (the room) and warm (what is lit), and an ordered 4x4 Bayer dither
brings it down to the eleven colours of PALETTE. The live LED panel is not
in the picture: the page lays it over the bare middle of the wall
(site/theme/site.css `.hero`), so the middle third stays quiet.
tools/site-scene-alt.py --style day draws this same room on bright ramps.

Writes (committed; run this only to change the drawing):

  site/theme/den-night.webp        480x372 art pixels at x4 (1920x1488): the wide
                                   scene, laid `cover` behind the hero.
  site/theme/den-night-phone.webp  195x180 at x4 (780x720): the foot of the same
                                   room for a narrow screen, its top row the flat
                                   wall colour WALL (the hero's background there).

Lossless WebP, so every art pixel stays a clean square.
"""
import argparse
import random
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "site/theme"
SCALE = 4

# The two ramps, dark to light. Index 0 and 1 are shared.
COOL = ["#07080c", "#10121b", "#1b1f31", "#2e3454", "#575b8c", "#e8e1cf"]
WARM = ["#07080c", "#10121b", "#5e2c14", "#b9601a", "#f7a531", "#ffe7b0"]
PALETTE = sorted(set(COOL + WARM))
WALL = COOL[1]  # site.css [data-scene=night] --den, the flat wall above the phone's picture

BAYER = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], dtype=np.float32) / 16 + 1 / 32
N = len(COOL) - 1


def lv(k: float) -> float:
    """Ramp step k (0..5, halves dither evenly) as a brightness."""
    return k / N


class Canvas:
    def __init__(self, w: int, h: int):
        self.w, self.h = w, h
        self.lum = np.zeros((h, w), dtype=np.float32)
        self.warm = np.zeros((h, w), dtype=np.float32)  # 0 cool .. 1 warm, dithered too
        self.yy, self.xx = np.mgrid[0:h, 0:w].astype(np.float32)

    def put(self, mask, lum, warm=0.0):
        self.lum = np.where(mask, lum, self.lum).astype(np.float32)
        self.warm = np.where(mask, warm, self.warm).astype(np.float32)

    def box(self, x0, y0, x1, y1):
        """The mask of x0 <= x < x1, y0 <= y < y1."""
        return (self.xx >= x0) & (self.xx < x1) & (self.yy >= y0) & (self.yy < y1)

    def rect(self, x0, y0, x1, y1, lum, warm=0.0):
        self.put(self.box(x0, y0, x1, y1), lum, warm)

    def frame(self, x0, y0, x1, y1, lum, warm=0.0):
        self.put(self.box(x0, y0, x1, y1) & ~self.box(x0 + 1, y0 + 1, x1 - 1, y1 - 1), lum, warm)

    def disc(self, cx, cy, rx, ry=None):
        ry = rx if ry is None else ry
        return ((self.xx - cx) / rx) ** 2 + ((self.yy - cy) / ry) ** 2 <= 1

    def line(self, x0, y0, x1, y1, lum, warm=0.0, width=1.0):
        dx, dy = x1 - x0, y1 - y0
        t = np.clip(((self.xx - x0) * dx + (self.yy - y0) * dy) / (dx * dx + dy * dy), 0, 1)
        d = np.hypot(self.xx - (x0 + t * dx), self.yy - (y0 + t * dy))
        self.put(d <= width / 2, lum, warm)

    def quad(self, xl0, xr0, y0, xl1, xr1, y1):
        """A trapezoid: from xl0..xr0 at y0 to xl1..xr1 at y1."""
        t = np.clip((self.yy - y0) / max(y1 - y0, 1), 0, 1)
        return (self.yy >= y0) & (self.yy < y1) & (self.xx >= xl0 + (xl1 - xl0) * t) & (self.xx < xr0 + (xr1 - xr0) * t)

    def image(self) -> Image.Image:
        h, w = self.lum.shape
        bayer = np.tile(BAYER, (h // 4 + 1, w // 4 + 1))[:h, :w]
        t = np.clip(self.lum, 0, 1) * N
        i = np.floor(t)
        idx = np.clip(i + ((t - i) > bayer), 0, N).astype(np.int32)
        # The warm choice dithers on the same matrix, turned, so the two don't beat.
        warm = self.warm > np.tile(BAYER.T[::-1], (h // 4 + 1, w // 4 + 1))[:h, :w]
        rgb = lambda ramp: np.array([[int(c[i:i + 2], 16) for i in (1, 3, 5)] for c in ramp], dtype=np.uint8)
        out = np.where(warm[..., None], rgb(WARM)[idx], rgb(COOL)[idx])
        return Image.fromarray(out)


def glow(c: Canvas, x, y, r):
    """A soft falloff around (x, y), 1 at the centre, 0 at r."""
    return np.clip(1 - np.hypot(c.xx - x, (c.yy - y) * 1.15) / r, 0, 1) ** 2.4


def room(c: Canvas, floor: int, lamps: list[tuple[int, int]], screens: list[tuple[int, int]], flat_top: int = 0):
    """The wall and the floor, lit by the lamps (warm) and the screens (cool)."""
    w, h = c.w, c.h
    # The wall: darker to the corners, a quiet stripe paper, light pooled round the lamps.
    edge = np.abs(c.xx - w / 2) / (w / 2)
    wall = lv(1.55) - 0.10 * edge ** 2 + 0.05 * np.clip((c.yy - floor + 90) / 90, 0, 1)
    wall = wall + np.where((c.xx % 16 < 1), 0.035, 0) + np.where((c.xx % 16 == 8) & (c.yy % 6 < 3), 0.03, 0)
    warm = np.zeros_like(wall)
    for x, y in lamps:
        g = glow(c, x, y, 118)
        wall, warm = wall + 0.55 * g, np.maximum(warm, np.clip(g * 1.5, 0, 0.95))
    for x, y in screens:
        wall = wall + 0.2 * glow(c, x, y, 60)
    if flat_top:  # the phone's picture melts into the flat wall above it
        k = np.clip((c.yy - 4) / flat_top, 0, 1)
        wall, warm = lv(1) + (wall - lv(1)) * k, warm * k
    c.put(c.yy < floor, wall, warm)
    # The skirting board.
    c.rect(0, floor - 5, w, floor, lv(2.5))
    c.rect(0, floor - 5, w, floor - 4, lv(3))
    c.rect(0, floor - 1, w, floor, lv(0))
    # The floor: boards running to a vanishing point, lighter mid-room.
    vx, vy = w / 2, floor - 70
    depth = np.clip((c.yy - floor) / max(h - floor, 1), 0, 1)
    fl = lv(2.2) - 0.16 * edge ** 1.5 - 0.10 * depth
    fwarm = np.zeros_like(fl)
    for x, y in lamps:
        g = glow(c, x, floor + 10, 84)
        fl, fwarm = fl + 0.22 * g, np.maximum(fwarm, g * 0.8)
    ang = (c.xx - vx) / np.maximum(c.yy - vy, 1)
    seam = np.abs((ang * 5.0) % 1.0 - 0.5) > 0.5 - 0.035 * (1 + 60 / np.maximum(c.yy - vy, 1))
    fl = np.where(seam, fl - 0.14, fl)
    # Board ends, staggered row by row.
    row = np.floor(np.log2(np.maximum(c.yy - floor + 6, 1) / 6) * 2.2)
    ends = (np.floor(ang * 5.0 + row * 0.37) % 3 == 0) & (np.abs((c.yy - floor + 6) - 6 * 2 ** ((row + 1) / 2.2)) < 0.8)
    fl = np.where(ends, fl - 0.14, fl)
    c.put(c.yy >= floor, fl, fwarm)


def rug(c: Canvas, cx: int, y0: int, y1: int, half0: int, half1: int):
    m = c.quad(cx - half0, cx + half0, y0, cx - half1, cx + half1, y1)
    t = (c.yy - y0) / (y1 - y0)
    half = half0 + (half1 - half0) * t
    u = (c.xx - cx) / np.maximum(half, 1)  # -1..1 across the rug
    band = np.abs(u)
    lum = np.where(band > 0.9, lv(3), np.where(band > 0.8, lv(1), lv(2.5)))
    warm = np.where(band > 0.9, 1.0, np.where(band > 0.8, 0.0, 1.0))
    zig = np.abs(((u * 6) % 2) - 1)  # a zigzag along the middle
    on = (np.abs(t - 0.5 - (zig - 0.5) * 0.34) < 0.07) & (band < 0.72)
    lum, warm = np.where(on, lv(5), lum), np.where(on, 0.0, warm)
    rim = (t < 0.1) | (t > 0.9)
    lum = np.where(rim & (band <= 0.9), lv(1.5), lum)
    c.put(m, lum, warm)
    c.put(m & (band <= 0.8) & ~on & ~rim, lv(2.5) - 0.06 * np.abs(u), 0.55)


def cabinet(c: Canvas, x: int, fy: int, seed: int = 3, dim: float = 0.0):
    """An upright games cabinet, 64 wide and 160 tall, standing at x on floor line fy."""
    d = lambda k: lv(max(k - dim, 0))
    top = fy - 160
    c.put(c.disc(x + 32, fy + 3, 42, 5), lv(0.4))  # its shadow
    # The side panels and the body between them.
    c.rect(x, top + 4, x + 64, fy, d(1))
    c.put(c.quad(x + 4, x + 60, top, x, x + 64, top + 5), d(1))
    c.rect(x, top + 4, x + 3, fy, d(2.5))
    c.rect(x + 61, top + 4, x + 64, fy, d(0.5))
    c.rect(x + 3, top + 4, x + 4, fy, d(0))
    c.rect(x + 60, top + 4, x + 61, fy, d(0))
    # The marquee: lit from behind, chevrons, no words.
    mx0, mx1, my0, my1 = x + 5, x + 59, top + 3, top + 19
    chev = ((c.xx - mx0) + np.abs(c.yy - (my0 + my1) / 2)) % 12 < 5
    c.put(c.box(mx0, my0, mx1, my1), np.where(chev, lv(4), lv(3) if dim else lv(5)), 1.0)
    c.frame(mx0 - 1, my0 - 1, mx1 + 1, my1 + 1, d(0))
    # The hood and the screen: a little ship among stars, drawn here.
    c.rect(x + 4, top + 20, x + 60, top + 70, d(0.5))
    sx0, sx1, sy0, sy1 = x + 9, x + 55, top + 25, top + 64
    scr = c.box(sx0, sy0, sx1, sy1) & ~(c.box(sx0, sy0, sx0 + 1, sy0 + 1) | c.box(sx1 - 1, sy0, sx1, sy0 + 1)
                                 | c.box(sx0, sy1 - 1, sx0 + 1, sy1) | c.box(sx1 - 1, sy1 - 1, sx1, sy1))
    c.put(scr, lv(1.2) + 0.25 * (c.yy - sy0) / (sy1 - sy0) - (0.06 if dim else 0) - np.where(c.yy % 2 == 0, 0.05, 0))
    rnd = random.Random(seed)
    for _ in range(16):
        c.rect(px := rnd.randrange(sx0 + 2, sx1 - 2), py := rnd.randrange(sy0 + 2, sy1 - 12), px + 1, py + 1, lv(rnd.choice([4, 5, 5])))
    shipx, shipy = (sx0 + sx1) // 2 - 4, sy1 - 9
    for k in range(4):  # a chevron of a ship
        c.rect(shipx + 3 - k, shipy + k, shipx + 5 + k, shipy + k + 1, lv(4), 1.0)
    c.rect(shipx + 3, shipy - 5, shipx + 5, shipy - 3, lv(5))  # its shot
    for bx, by_, r in ((sx0 + 9, sy0 + 8, 3.2), (sx0 + 24, sy0 + 13, 2.2), (sx0 + 37, sy0 + 7, 2.8)):  # three rocks, drifting
        c.put(scr & (np.abs(c.xx - bx) + np.abs(c.yy - by_) <= r), lv(4))
    # The control panel, standing proud of the body.
    cy = top + 70
    c.put(c.quad(x - 1, x + 65, cy, x - 4, x + 68, cy + 10), d(3.5))
    c.rect(x - 4, cy + 10, x + 68, cy + 14, d(2))
    c.rect(x - 4, cy + 14, x + 68, cy + 15, d(0))
    c.rect(x + 16, cy - 6, x + 18, cy + 3, d(4.5))          # the stick
    c.put(c.disc(x + 17, cy - 7, 3.2), lv(4), 1.0)           # its ball
    c.put(c.disc(x + 16.5, cy + 4, 4, 1.6), d(1))
    for i, bx in enumerate((x + 33, x + 42, x + 51)):
        c.put(c.disc(bx + 0.5, cy + 4 - (i % 2), 2.6, 1.9), lv(4 if i != 1 else 5), 1.0 if i != 1 else 0.0)
    # The body below: a stripe of side art, the coin door, the kick plate.
    by = cy + 15
    stripe = ((c.xx - x) + (c.yy - by)) % 18 < 6
    c.put(c.box(x + 4, by, x + 60, fy - 7) & stripe, d(1.6))
    c.rect(x + 20, by + 12, x + 44, by + 46, d(0))
    c.rect(x + 21, by + 13, x + 43, by + 45, d(2))
    c.rect(x + 23, by + 15, x + 41, by + 43, d(1.4))
    for sxp in (x + 26, x + 35):
        c.rect(sxp, by + 20, sxp + 3, by + 26, lv(4), 1.0)
        c.rect(sxp + 1, by + 21, sxp + 2, by + 25, lv(2), 1.0)
    c.rect(x + 28, by + 33, x + 36, by + 39, d(0.4))
    c.rect(x + 4, fy - 7, x + 60, fy, d(0.3))


def tv_unit(c: Canvas, x: int, fy: int, seed: int = 7):
    """A low unit of shelves, 92 wide, with a tube television and a plant on top."""
    rnd = random.Random(seed)
    top = fy - 50
    c.put(c.disc(x + 46, fy + 3, 56, 5), lv(0.4))
    c.rect(x, top, x + 92, fy - 3, lv(1))
    c.rect(x - 2, top - 3, x + 94, top, lv(3.5))
    c.rect(x - 2, top, x + 94, top + 1, lv(0))
    for lx in (x + 3, x + 85):
        c.rect(lx, fy - 3, lx + 4, fy, lv(0.5))
    # Two shelves of spines; a console and its pads in the lower left.
    for row, sy in enumerate((top + 4, top + 26)):
        c.rect(x + 3, sy, x + 89, sy + 19, lv(0.3))
        px = x + 4
        end = x + 88
        if row == 1:
            c.rect(x + 6, sy + 11, x + 36, sy + 19, lv(3.2))
            c.rect(x + 6, sy + 11, x + 36, sy + 12, lv(4))
            c.rect(x + 10, sy + 14, x + 24, sy + 15, lv(1))
            c.rect(x + 29, sy + 14, x + 31, sy + 16, lv(4), 1.0)
            c.rect(x + 8, sy + 6, x + 18, sy + 10, lv(2.4))   # a pad lying on it
            c.rect(x + 10, sy + 7, x + 12, sy + 9, lv(4))
            c.rect(x + 15, sy + 7, x + 16, sy + 8, lv(4), 1.0)
            px = x + 40
        while px < end - 2:
            wd = rnd.choice([2, 3, 3, 4, 5])
            ht = rnd.randrange(12, 18)
            warm = 1.0 if rnd.random() < 0.22 else 0.0
            k = rnd.choice([2, 2.5, 3, 3.5, 4]) if not warm else rnd.choice([2.5, 3, 4])
            if px + wd > end:
                break
            c.rect(px, sy + 19 - ht, px + wd, sy + 19, lv(k), warm)
            if wd >= 3:
                c.rect(px + 1, sy + 21 - ht, px + wd - 1, sy + 23 - ht, lv(5 if k < 3.5 else 1), 0.0)
            px += wd + (1 if rnd.random() < 0.8 else 3)
        c.rect(x + 3, sy + 19, x + 89, sy + 21, lv(2.5))
    # The television.
    tx, ty0, ty1 = x + 8, top - 51, top - 3
    c.rect(tx, ty0, tx + 60, ty1, lv(2.2))
    c.frame(tx, ty0, tx + 60, ty1, lv(0))
    c.rect(tx + 1, ty0 + 1, tx + 59, ty0 + 2, lv(3.5))
    c.rect(tx + 6, ty1, tx + 12, ty1 + 3, lv(0.5))
    c.rect(tx + 48, ty1, tx + 54, ty1 + 3, lv(0.5))
    sx0, sx1, sy0, sy1 = tx + 5, tx + 45, ty0 + 6, ty1 - 6
    c.rect(sx0 - 2, sy0 - 2, sx1 + 2, sy1 + 2, lv(0))
    scr = c.box(sx0, sy0, sx1, sy1) & ((np.abs(c.xx - (sx0 + sx1 - 1) / 2) / 21) ** 8 + (np.abs(c.yy - (sy0 + sy1 - 1) / 2) / 19) ** 8 <= 1)
    bar = np.floor((c.xx - sx0) / 5.72)
    blum = np.choose(np.clip(bar, 0, 6).astype(int), [lv(5), lv(4.4), lv(4), lv(3.4), lv(3), lv(2.4), lv(2)])
    bwarm = np.choose(np.clip(bar, 0, 6).astype(int), [0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0])
    low = c.yy >= sy1 - 9
    blum, bwarm = np.where(low, lv(1.2) + 0.5 * (c.xx - sx0) / 40, blum), np.where(low, 0.0, bwarm)
    c.put(scr, blum - np.where(c.yy % 2 == 0, 0.07, 0), bwarm)
    for ky in (ty0 + 9, ty0 + 19):
        c.put(c.disc(tx + 53, ky + 2, 3.2), lv(4))
        c.rect(tx + 53, ky, tx + 54, ky + 2, lv(1))
    for gy in range(ty0 + 29, ty1 - 5, 2):
        c.rect(tx + 49, gy, tx + 57, gy + 1, lv(0.8))
    c.line(tx + 30, ty0, tx + 16, ty0 - 20, lv(4))
    c.line(tx + 30, ty0, tx + 46, ty0 - 16, lv(4))
    c.rect(tx + 27, ty0 - 2, tx + 34, ty0, lv(1))
    plant(c, x + 82, top - 3, small=True)


def plant(c: Canvas, x: int, fy: int, small: bool = False, seed: int = 5):
    rnd = random.Random(seed)
    ph, pw = (8, 5) if small else (15, 9)
    c.put(c.quad(x - pw, x + pw, fy - ph, x - pw + 2, x + pw - 2, fy), lv(3), 1.0)
    c.rect(x - pw - 1, fy - ph - 2, x + pw + 1, fy - ph, lv(4), 1.0)
    c.put(c.quad(x + pw - 3, x + pw, fy - ph, x + pw - 4, x + pw - 2, fy), lv(2), 1.0)
    reach = 20 if small else 46
    for i in range(7 if small else 11):
        a = -1.05 + 2.1 * (i + 0.5) / (7 if small else 11) + rnd.uniform(-0.08, 0.08)
        ln = reach * rnd.uniform(0.7, 1.0) * (1 - 0.25 * abs(a))
        x1, y1 = x + np.sin(a) * ln, fy - ph - 2 - np.cos(a) * ln
        mx, my = (x + x1) / 2 + np.sin(a) * 2, (fy - ph - 2 + y1) / 2 - 3
        k = lv(rnd.choice([2.6, 3, 3.4, 4]))
        c.line(x, fy - ph - 2, mx, my, k, width=1.6 if small else 3)
        c.line(mx, my, x1, y1 + (3 if abs(a) > 0.5 else 0), k, width=1.2 if small else 2.2)


def lamp(c: Canvas, x: int, fy: int, height: int = 132):
    c.put(c.disc(x, fy + 1, 9, 2.5), lv(1))
    c.rect(x - 4, fy - 2, x + 5, fy, lv(3))
    c.rect(x, fy - height + 14, x + 1, fy - 2, lv(4))
    top = fy - height
    shade = c.quad(x - 6, x + 7, top, x - 11, x + 12, top + 16)
    c.put(shade, lv(5) - 0.22 * np.abs(c.xx - x) / 12 - 0.1 * (top + 16 - c.yy) / 16, 1.0)
    c.rect(x - 11, top + 15, x + 12, top + 16, lv(4), 1.0)


def poster(c: Canvas, x: int, y: int, w: int, h: int, kind: int):
    c.rect(x + 2, y + 2, x + w + 2, y + h + 2, lv(0.5))  # its shadow on the wall
    c.rect(x, y, x + w, y + h, lv(0.6))
    c.frame(x, y, x + w, y + h, lv(4))
    inside = c.box(x + 2, y + 2, x + w - 2, y + h - 2)
    cx, cy = x + w / 2, y + h / 2
    if kind == 0:  # a ringed planet
        c.put(inside, lv(0.8) + 0.12 * (c.yy - y) / h)
        r = min(w, h) * 0.26
        c.put(inside & c.disc(cx, cy, r), lv(4.6) - 0.5 * np.clip(((c.xx - cx) + (c.yy - cy)) / (2 * r) + 0.5, 0, 1), 1.0)
        ring = np.abs(np.hypot((c.xx - cx) / (r * 1.9), (c.yy - cy - (c.xx - cx) * 0.25) / (r * 0.5)) - 1) < 0.14
        c.put(inside & ring & ~(c.disc(cx, cy, r) & (c.yy < cy + (c.xx - cx) * 0.25)), lv(5))
        rnd = random.Random(11)
        for _ in range(9):
            px, py = rnd.randrange(x + 3, x + w - 3), rnd.randrange(y + 3, y + h - 3)
            if not c.disc(cx, cy, r * 2.1)[py, px]:
                c.rect(px, py, px + 1, py + 1, lv(4))
    else:  # hills under a low sun
        c.put(inside, lv(1) + 0.3 * (c.yy - y) / h)
        c.put(inside & c.disc(cx, y + h * 0.5, w * 0.24), lv(4.5) - np.where((c.yy - y) % 4 < 1, 0.3, 0), 1.0)
        hill = y + h * 0.62 + 5 * np.sin((c.xx - x) / 5.0) + 3 * np.sin((c.xx - x) / 2.3 + 1)
        c.put(inside & (c.yy > hill), lv(2.2))
        c.put(inside & (c.yy > hill + 7 + 2 * np.sin((c.xx - x) / 3.1)), lv(0.6))


def wall_shelf(c: Canvas, x: int, y: int, w: int, seed: int = 2):
    rnd = random.Random(seed)
    c.rect(x, y, x + w, y + 3, lv(3.5))
    c.rect(x, y + 3, x + w, y + 4, lv(0))
    for bx in (x + 5, x + w - 7):
        c.put(c.quad(bx, bx + 2, y + 4, bx, bx + 1, y + 10), lv(2.5))
    px = x + 3
    while px < x + w - 26:  # game boxes, standing
        wd, ht = rnd.choice([3, 4, 4, 5]), rnd.randrange(13, 19)
        warm = 1.0 if rnd.random() < 0.25 else 0.0
        c.rect(px, y - ht, px + wd, y, lv(rnd.choice([2.4, 3, 3.6, 4])), warm)
        c.rect(px + 1, y - ht + 2, px + wd - 1, y - ht + 4, lv(1))
        px += wd + 1
    # A cup with a star on it.
    tx = x + w - 15
    c.rect(tx - 4, y - 3, tx + 5, y, lv(3), 1.0)
    c.rect(tx - 1, y - 7, tx + 2, y - 3, lv(4), 1.0)
    c.put(c.quad(tx - 5, tx + 6, y - 17, tx - 2, tx + 3, y - 7), lv(4.4), 1.0)
    c.rect(tx - 7, y - 16, tx - 5, y - 11, lv(4), 1.0)
    c.rect(tx + 6, y - 16, tx + 8, y - 11, lv(4), 1.0)
    c.rect(tx, y - 15, tx + 1, y - 10, lv(5))
    c.rect(tx - 2, y - 13, tx + 3, y - 12, lv(5))


def stool(c: Canvas, x: int, fy: int):
    c.put(c.disc(x, fy + 2, 12, 2.5), lv(0.5))
    for lx in (x - 8, x + 7):
        c.line(lx + (2 if lx < x else -2), fy - 24, lx, fy, lv(2.5), width=1.6)
    c.rect(x - 6, fy - 10, x + 7, fy - 9, lv(2))
    c.put(c.disc(x, fy - 26, 10.5, 3.2), lv(3), 1.0)
    c.put(c.disc(x, fy - 27, 10.5, 2.6), lv(4), 1.0)


def wide() -> Image.Image:
    """The wide scene: 480x372, a cabinet and a half to the left, the television and a lamp to the right."""
    c = Canvas(480, 372)
    floor = 310
    lamp_x = 377
    room(c, floor, lamps=[(lamp_x, floor - 112)], screens=[(52, floor - 110), (425, floor - 80)])
    poster(c, 26, 84, 44, 36, 0)
    poster(c, 402, 96, 30, 40, 1)
    wall_shelf(c, 388, floor - 118, 80)
    rug(c, 240, floor + 16, floor + 50, 78, 106)
    cabinet(c, -40, floor + 2, seed=9, dim=0.5)
    cabinet(c, 30, floor + 6)
    stool(c, 112, floor + 22)
    lamp(c, lamp_x, floor + 3)
    tv_unit(c, 384, floor + 6)
    plant(c, 474, floor + 32, seed=8)
    # The room falls away into the dark at its foot and corners.
    edge = np.abs(c.xx - 240) / 240
    c.lum = c.lum * (1 - 0.35 * np.clip((c.yy - floor - 24) / 38, 0, 1) ** 2) * (1 - 0.3 * edge ** 6)
    return c.image()


def phone() -> Image.Image:
    """The foot of the room for a narrow screen: 195x180, flat wall at its top."""
    c = Canvas(195, 180)
    floor = 148
    room(c, floor, lamps=[(84, floor - 80)], screens=[(140, floor - 70)], flat_top=44)
    rug(c, 70, floor + 12, floor + 30, 60, 74)
    lamp(c, 84, floor + 2, height=96)
    tv_unit(c, 98, floor + 5)
    plant(c, 22, floor + 8, seed=8)
    c.lum = c.lum * (1 - 0.3 * np.clip((c.yy - 164) / 16, 0, 1) ** 2)
    return c.image()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preview", type=Path, help="write PNGs here instead of the site's WebP files")
    args = ap.parse_args()
    for name, im in (("den-night", wide()), ("den-night-phone", phone())):
        colours = {c for _, c in im.getcolors(1 << 16)}
        want = {tuple(int(p[i:i + 2], 16) for i in (1, 3, 5)) for p in PALETTE}
        assert colours <= want, f"{name}: colours off the palette"
        big = im.resize((im.width * SCALE, im.height * SCALE), Image.NEAREST)
        if args.preview:
            args.preview.mkdir(parents=True, exist_ok=True)
            big.save(args.preview / f"{name}.png")
            print(args.preview / f"{name}.png")
            continue
        out = OUT / f"{name}.webp"
        big.save(out, "WEBP", lossless=True, quality=100, method=6, exact=True)
        print(f"{out.relative_to(ROOT)}: {big.width}x{big.height} ({im.width}x{im.height} x{SCALE}), {len(colours)} colours, {out.stat().st_size:,} B")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
