# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.2.6", "pillow==11.3.0"]
# ///
"""Draw alternate styles of the home page's scene, for the owner to choose from.

    uv run tools/site-scene-alt.py [--style NAME ...] [--preview DIR]

tools/site-scene.py draws the scene the site uses (the den at night, two
ramps and a Bayer dither); it is not changed and still reproduces den.webp.
This script draws the candidates beside it. Original pixel art, drawn here
from rectangles, discs and lines: no photograph, no model, no third-party
art, no game's characters or logos, and no real machine's design. Every
console, cartridge, pad, handheld, television and cabinet is made up.

  day      the same den by daylight: site-scene.py's own drawing on bright
           ramps (mint and cream, coral and yellow), lamp off, no vignette
  chunky   a 16-bit playroom: x6 art pixels, bold outlines, flat saturated
           colours, no dither. A television and console on the floor, a
           beanbag, a bookcase of cartridges
  shelf    a collector's wall by day: a long bench of made-up consoles under
           the panel, shelves of cartridges and handhelds either side
  neon     the same wall at night, lit by neon strips (shelf, repainted)
  arcade   an arcade corner: bright cabinets on a confetti carpet, a
           television and console on the floor
  synth    synthwave: a banded sun on the horizon, a perspective grid,
           ridge lines and palms either side, all in flat bands

Each style writes site/theme/den-NAME.webp (1920x1488, laid `cover` behind
the hero like den.webp) and den-NAME-phone.webp (780x720, whose top row is
the flat colour STYLES[NAME]["wall"]: the hero's background on a phone).
The live LED panel is laid over the picture's upper middle by site.css, so
that part stays bare; what the eye gets is the two sides and the foot.
Lossless WebP, every art pixel a clean square. tools/site-scene-shots.py
swaps a style into a built site and photographs the hero.
"""
import argparse
import importlib.util
import math
import random
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "site/theme"
FULL, PHONE = (1920, 1488), (780, 720)


def hx(c: str) -> tuple[int, int, int]:
    return tuple(int(c[i:i + 2], 16) for i in (1, 3, 5))


def grow(m):
    o = m.copy()
    o[1:] |= m[:-1]
    o[:-1] |= m[1:]
    o[:, 1:] |= m[:, :-1]
    o[:, :-1] |= m[:, 1:]
    return o


class Art:
    """A canvas of flat colours; ink() fills a mask and draws a one-pixel outline round it."""

    def __init__(self, w: int, h: int, bg: str, ol: str | None):
        self.w, self.h, self.ol = w, h, ol
        self.px = np.empty((h, w, 3), dtype=np.uint8)
        self.px[:] = hx(bg)
        self.yy, self.xx = np.mgrid[0:h, 0:w].astype(np.float32)

    def box(self, x0, y0, x1, y1):
        return (self.xx >= x0) & (self.xx < x1) & (self.yy >= y0) & (self.yy < y1)

    def disc(self, cx, cy, rx, ry=None):
        ry = rx if ry is None else ry
        return ((self.xx - cx) / rx) ** 2 + ((self.yy - cy) / ry) ** 2 <= 1

    def quad(self, xl0, xr0, y0, xl1, xr1, y1):
        t = np.clip((self.yy - y0) / max(y1 - y0, 1), 0, 1)
        return (self.yy >= y0) & (self.yy < y1) & (self.xx >= xl0 + (xl1 - xl0) * t) & (self.xx < xr0 + (xr1 - xr0) * t)

    def segm(self, x0, y0, x1, y1, width=1.0):
        dx, dy = x1 - x0, y1 - y0
        t = np.clip(((self.xx - x0) * dx + (self.yy - y0) * dy) / max(dx * dx + dy * dy, 1e-6), 0, 1)
        return np.hypot(self.xx - (x0 + t * dx), self.yy - (y0 + t * dy)) <= width / 2

    def fill(self, m, c: str):
        self.px[m] = hx(c)

    def ink(self, m, c: str, ol: str | None = ""):
        ol = self.ol if ol == "" else ol
        if ol:
            self.px[grow(m) & ~m] = hx(ol)
        self.px[m] = hx(c)

    def rect(self, x0, y0, x1, y1, c: str):
        self.fill(self.box(x0, y0, x1, y1), c)

    def orect(self, x0, y0, x1, y1, c: str, ol: str | None = ""):
        self.ink(self.box(x0, y0, x1, y1), c, ol)

    def rrect(self, x0, y0, x1, y1):
        """A box with its four corner pixels off."""
        m = self.box(x0, y0, x1, y1)
        for cx, cy in ((x0, y0), (x1 - 1, y0), (x0, y1 - 1), (x1 - 1, y1 - 1)):
            m &= ~self.box(cx, cy, cx + 1, cy + 1)
        return m

    def seg(self, x0, y0, x1, y1, c: str, width=1.0):
        self.fill(self.segm(x0, y0, x1, y1, width), c)

    def cable(self, pts, c: str):
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            self.seg(x0, y0, x1, y1, c)

    def image(self) -> Image.Image:
        return Image.fromarray(self.px)


# ---------------------------------------------------------------- the things (all made up)

def screen_game(a: Art, kind: int, x0, y0, x1, y1, P):
    """What a screen shows: abstract games of bars, bricks and dots, no characters."""
    w, h = x1 - x0, y1 - y0
    a.fill(a.rrect(x0, y0, x1, y1), P["scr"])
    if kind == 0:  # bat and ball
        for y in range(y0 + 1, y1 - 1, 3):
            a.rect(x0 + w // 2, y, x0 + w // 2 + 1, y + 1, P["scr2"])
        a.rect(x0 + 2, y0 + h // 3, x0 + 4, y0 + h // 3 + 6, P["lit"])
        a.rect(x1 - 4, y0 + h // 2, x1 - 2, y0 + h // 2 + 6, P["lit"])
        a.rect(x0 + w // 2 + 5, y0 + h // 2 - 2, x0 + w // 2 + 7, y0 + h // 2, P["hot"])
    elif kind == 1:  # rows of bricks, a bat under them
        for r, col in enumerate((P["hot"], P["lit"], P["cool"])):
            for bx in range(x0 + 2, x1 - 5, 6):
                a.rect(bx, y0 + 2 + r * 3, bx + 5, y0 + 4 + r * 3, col)
        a.rect(x0 + w // 2 - 4, y1 - 4, x0 + w // 2 + 4, y1 - 3, P["lit"])
        a.rect(x0 + w // 2 + 3, y1 - 9, x0 + w // 2 + 5, y1 - 7, P["lit"])
    elif kind == 2:  # a road running to the horizon
        hz = y0 + h // 3
        a.fill(a.box(x0, y0, x1, hz) & a.rrect(x0, y0, x1, y1), P["cool"])
        a.fill(a.disc(x0 + w * 0.7, hz, 4) & a.box(x0, y0, x1, hz), P["lit"])
        cx = x0 + w / 2
        a.fill(a.quad(cx - 1, cx + 1, hz, cx - w * 0.36, cx + w * 0.36, y1) & a.rrect(x0, y0, x1, y1), P["scr2"])
        for y in range(hz + 2, y1 - 1, 4):
            a.rect(cx - 0.5, y, cx + 0.5, y + 2, P["lit"])
    else:  # test bars
        cols = (P["lit"], P["hot"], P["cool"], P["scr2"], P["hot"], P["cool"])
        bw = w / len(cols)
        for i, col in enumerate(cols):
            a.fill(a.box(x0 + i * bw, y0, x0 + (i + 1) * bw, y1) & a.rrect(x0, y0, x1, y1), col)
    a.fill(a.box(x0 + 1, y0 + 1, x0 + 2, y0 + 3) | a.box(x0 + 1, y0 + 1, x0 + 4, y0 + 2), P["glint"])


def crt(a: Art, x, fy, w, h, P, body, shade, kind=0):
    """A tube television sitting on fy."""
    y = fy - h
    a.orect(x + 3, fy - 2, x + w - 3, fy, P["dark"])
    a.ink(a.rrect(x, y, x + w, fy - 2), body)
    a.rect(x + 1, y, x + w - 1, y + 1, P["glint"])
    a.rect(x + 1, fy - 5, x + w - 1, fy - 3, shade)
    side = max(7, w // 5)
    sx0, sy0, sx1, sy1 = x + 3, y + 3, x + w - side - 2, fy - 7
    a.rect(sx0 - 1, sy0 - 1, sx1 + 1, sy1 + 1, P["dark"])
    screen_game(a, kind, sx0, sy0, sx1, sy1, P)
    kx = x + w - side
    a.rect(kx + 1, y + 4, kx + 4, y + 7, P["dark"])
    a.rect(kx + 1, y + 10, kx + 4, y + 12, P["hot"])
    for gy in range(y + 17, fy - 7, 2):
        a.rect(kx, gy, x + w - 3, gy + 1, shade)


def cart(a: Art, x, fy, w, h, col, P):
    """A cartridge standing on fy: a square body, a grip notch on top, a label."""
    a.orect(x, fy - h, x + w, fy, col)
    a.rect(x + w // 2 - 1, fy - h, x + w // 2 + 1, fy - h + 1, P["dark"])
    a.rect(x + 1, fy - h + 3, x + w - 1, fy - 2, P["label"])
    a.rect(x + 1, fy - h + 5, x + w - 1, fy - h + 6, col)


def cart_row(a: Art, x0, x1, fy, P, seed, lean=True):
    rnd = random.Random(seed)
    x = x0
    while x < x1 - 3:
        w, h = rnd.choice([4, 5, 5, 6]), rnd.choice([9, 10, 10, 11])
        if x + w > x1:
            break
        cart(a, x, fy, w, h, rnd.choice(P["carts"]), P)
        x += w + 2 + (2 if lean and rnd.random() < 0.15 else 0)


def console_slab(a: Art, x, fy, w, P, body, trim, with_cart=None):
    """A flat console: a stripe, a slot on top (a cartridge in it), two ports and a light."""
    if with_cart:
        cart(a, x + w // 2 - 4, fy - 6, 8, 8, with_cart, P)
    a.ink(a.rrect(x, fy - 8, x + w, fy), body)
    a.rect(x + 1, fy - 8, x + w - 1, fy - 7, P["glint"])
    a.rect(x + 1, fy - 5, x + w - 1, fy - 3, trim)
    a.rect(x + 3, fy - 2, x + 6, fy - 1, P["dark"])
    a.rect(x + 8, fy - 2, x + 11, fy - 1, P["dark"])
    a.rect(x + w - 5, fy - 5, x + w - 3, fy - 3, P["hot"])
    a.rect(x + w - 9, fy - 5, x + w - 7, fy - 3, P["lit"])


def console_dome(a: Art, x, fy, w, P, body, lid):
    """A top-loader: a sloped body, a round lid, one big button."""
    a.ink(a.quad(x + 3, x + w - 3, fy - 11, x, x + w, fy - 6) | a.box(x, fy - 6, x + w, fy), body)
    a.ink(a.disc(x + w * 0.4, fy - 10.5, w * 0.26, 2.6), lid)
    a.rect(x + w * 0.4 - 3, fy - 12, x + w * 0.4 + 1, fy - 11, P["glint"])
    a.fill(a.disc(x + w - 6.5, fy - 8.5, 1.6), P["hot"])
    a.rect(x + 1, fy - 3, x + w - 1, fy - 2, P["dark"])
    a.rect(x + 3, fy - 5, x + 5, fy - 4, P["lit"])


def console_wedge(a: Art, x, fy, w, P, body, trim):
    """A wedge with a row of square keys along its front."""
    a.ink(a.quad(x + 6, x + w - 2, fy - 10, x, x + w, fy - 4) | a.box(x, fy - 4, x + w, fy), body)
    a.rect(x + 8, fy - 10, x + w - 3, fy - 9, P["glint"])
    for i, kx in enumerate(range(x + 4, x + w - 5, 5)):
        a.rect(kx, fy - 6, kx + 3, fy - 4, (P["lit"], P["hot"], P["cool"], trim)[i % 4])
    a.rect(x + 1, fy - 2, x + w - 1, fy - 1, trim)


def pad(a: Art, x, fy, P, body, tall=False):
    """A controller lying down: a round thumb disc and three buttons in a triangle."""
    a.ink(a.rrect(x, fy - 6, x + 15, fy), body)
    a.fill(a.disc(x + 3.5, fy - 3, 2.1), P["dark"])
    a.rect(x + 3, fy - 4, x + 4, fy - 3, P["glint"])
    a.rect(x + 9, fy - 3, x + 11, fy - 1, P["hot"])
    a.rect(x + 12, fy - 4, x + 14, fy - 2, P["lit"])
    a.rect(x + 10, fy - 6 + 1, x + 12, fy - 4, P["cool"])


def handheld(a: Art, x, fy, P, body, wide_=False):
    """A handheld standing up: a screen, a dial and buttons under it (or beside it)."""
    if wide_:
        a.ink(a.rrect(x, fy - 10, x + 22, fy), body)
        a.rect(x + 6, fy - 8, x + 16, fy - 2, P["dark"])
        a.rect(x + 7, fy - 7, x + 15, fy - 3, P["cool"])
        a.rect(x + 8, fy - 6, x + 10, fy - 5, P["glint"])
        a.fill(a.disc(x + 3, fy - 5, 1.7), P["dark"])
        a.rect(x + 18, fy - 7, x + 20, fy - 5, P["hot"])
        a.rect(x + 18, fy - 4, x + 20, fy - 2, P["lit"])
    else:
        a.ink(a.rrect(x, fy - 18, x + 12, fy), body)
        a.rect(x + 2, fy - 16, x + 10, fy - 9, P["dark"])
        a.rect(x + 3, fy - 15, x + 9, fy - 10, P["lit"])
        a.rect(x + 4, fy - 14, x + 6, fy - 13, P["glint"])
        a.rect(x + 2, fy - 7, x + 10, fy - 6, P["dark"])
        a.rect(x + 5, fy - 8, x + 7, fy - 5, P["glint"])
        for i, col in enumerate((P["hot"], P["cool"], P["dark"])):
            a.rect(x + 2 + i * 3, fy - 4, x + 4 + i * 3, fy - 2, col)


def joystick(a: Art, x, fy, P, body):
    a.ink(a.rrect(x, fy - 5, x + 16, fy), body)
    a.rect(x + 1, fy - 5, x + 15, fy - 4, P["glint"])
    a.orect(x + 4, fy - 12, x + 6, fy - 5, P["dark"], None)
    a.ink(a.disc(x + 4.5, fy - 13.5, 2.6), P["hot"])
    a.rect(x + 3, fy - 15, x + 4, fy - 14, P["glint"])
    a.rect(x + 9, fy - 4, x + 11, fy - 2, P["lit"])
    a.rect(x + 12, fy - 4, x + 14, fy - 2, P["cool"])


def cactus(a: Art, x, fy, P, pot):
    m = a.rrect(x - 3, fy - 21, x + 3, fy - 5) | a.box(x - 8, fy - 14, x - 3, fy - 11) | a.rrect(x - 9, fy - 18, x - 6, fy - 11) \
        | a.box(x + 3, fy - 11, x + 8, fy - 8) | a.rrect(x + 6, fy - 16, x + 9, fy - 8)
    a.ink(m, P["leaf"])
    a.fill(m & (a.xx == x - 1) & (a.yy % 3 == 0), P["leaf2"])
    a.fill(m & (a.xx == x + 7) & (a.yy % 3 == 1), P["leaf2"])
    a.rect(x - 1, fy - 23, x + 1, fy - 21, P["hot"])
    a.ink(a.quad(x - 5, x + 5, fy - 6, x - 4, x + 4, fy), pot)
    a.rect(x - 5, fy - 6, x + 5, fy - 5, P["glint"])


def snake_plant(a: Art, x, fy, P, pot, n=5, reach=26):
    rnd = random.Random(x)
    for i in range(n):
        dx = (i - (n - 1) / 2) * 3.4
        tip = reach * rnd.uniform(0.6, 1.0)
        m = a.quad(x + dx * 1.9 - 0.5, x + dx * 1.9 + 0.5, fy - 7 - tip, x + dx * 0.5 - 2, x + dx * 0.5 + 2, fy - 6)
        a.ink(m, P["leaf"] if i % 2 else P["leaf2"])
    a.ink(a.quad(x - 6, x + 6, fy - 8, x - 5, x + 5, fy), pot)
    a.rect(x - 6, fy - 8, x + 6, fy - 7, P["glint"])


def stool(a: Art, x, fy, P, seat):
    a.seg(x - 4, fy - 14, x - 6, fy, P["dark"], 1.6)
    a.seg(x + 4, fy - 14, x + 6, fy, P["dark"], 1.6)
    a.rect(x - 5, fy - 6, x + 6, fy - 5, P["dark"])
    a.ink(a.rrect(x - 8, fy - 18, x + 9, fy - 14), seat)
    a.rect(x - 7, fy - 18, x + 8, fy - 17, P["glint"])


def cabinet(a: Art, x, fy, P, body, side, trim, kind=0, w=44, h=118):
    """An upright games cabinet standing on fy: a lit marquee of shapes, a screen, a deck of controls."""
    top = fy - h
    a.ink(a.box(x, top + 3, x + w, fy) | a.quad(x + 3, x + w - 3, top, x, x + w, top + 3), body)
    a.rect(x, top + 3, x + 3, fy, side)
    a.rect(x + w - 3, top + 3, x + w, fy, side)
    # The marquee: lit shapes, no words.
    a.orect(x + 5, top + 4, x + w - 5, top + 16, P["lit"])
    for i, sx in enumerate(range(x + 8, x + w - 10, 9)):
        if (i + kind) % 3 == 0:
            a.fill(a.disc(sx + 3, top + 10, 3), trim)
        elif (i + kind) % 3 == 1:
            a.fill(a.quad(sx + 3, sx + 4, top + 6, sx, sx + 7, top + 14), P["hot"])
        else:
            a.rect(sx + 1, top + 7, sx + 6, top + 13, P["cool"])
    # The hood and the screen.
    a.rect(x + 3, top + 18, x + w - 3, top + 54, P["dark"])
    screen_game(a, kind, x + 7, top + 22, x + w - 7, top + 50, P)
    # The control deck, standing proud.
    cy = top + 56
    a.ink(a.quad(x - 1, x + w + 1, cy, x - 3, x + w + 3, cy + 7) | a.box(x - 3, cy + 7, x + w + 3, cy + 11), trim)
    a.rect(x - 3, cy + 8, x + w + 3, cy + 11, side)
    a.rect(x + 10, cy - 6, x + 12, cy + 2, P["dark"])
    a.ink(a.disc(x + 10.5, cy - 7, 2.6), P["hot"])
    for i, bx in enumerate((x + 22, x + 28, x + 34)):
        a.fill(a.disc(bx + 0.5, cy + 3 - (i % 2), 2, 1.5), (P["lit"], P["cool"], P["hot"])[i])
    # The body below: a band of side art, the coin door, the kick plate.
    by = cy + 12
    band = a.box(x + 3, by + 2, x + w - 3, min(by + 12, fy - 5))
    a.fill(band, side)
    a.fill(band & (((a.xx - x) + (a.yy - by)) % 8 < 3), trim)
    if h >= 110:  # room for the coin door
        a.orect(x + w // 2 - 8, by + 17, x + w // 2 + 8, by + 37, side)
        for sx in (x + w // 2 - 6, x + w // 2 - 1, x + w // 2 + 4):
            a.rect(sx, by + 20, sx + 2, by + 25, P["lit"])
        a.rect(x + w // 2 - 6, by + 29, x + w // 2 + 6, by + 30, P["dark"])
        a.rect(x + w // 2 - 6, by + 32, x + w // 2 + 6, by + 33, P["dark"])
    a.rect(x + 3, fy - 5, x + w - 3, fy, P["dark"])


def beanbag(a: Art, cx, fy, P, col, shade):
    m = a.disc(cx, fy - 11, 19, 12) | a.disc(cx - 6, fy - 19, 11, 9) | a.box(cx - 17, fy - 8, cx + 17, fy)
    m &= a.yy < fy
    a.ink(m, col)
    a.fill(m & a.disc(cx + 4, fy - 9, 9, 3), shade)
    a.fill(m & (a.yy >= fy - 3), shade)
    a.rect(cx - 10, fy - 25, cx - 5, fy - 24, P["glint"])


def shelf_plank(a: Art, x0, x1, y, P):
    a.orect(x0, y, x1, y + 3, P["wood"])
    a.rect(x0, y, x1, y + 1, P["wood_l"])
    for bx in (x0 + 4, x1 - 6):
        a.ink(a.quad(bx, bx + 2, y + 3, bx, bx + 1, y + 8), P["wood_d"])


# ---------------------------------------------------------------- shelf / neon: the collector's wall

SHELF = dict(
    wall="#f6ecd2", dot="#e6d6b0", floor="#c9b08a", ol="#3a2a32",
    wood="#d59a55", wood_l="#edc07a", wood_d="#a0683a",
    dark="#2c2a3a", glint="#ffffff", label="#fbf3dc",
    scr="#1e3247", scr2="#35607a", lit="#ffd45a", hot="#ee5d4e", cool="#3cb6b0",
    leaf="#3e9b5f", leaf2="#2c7350",
    grey="#dcd6c8", grey2="#a9a39a", teal="#2f9c95", red="#d9483f", mustard="#f0b33c", plum="#7d4f8f", blue="#3f6fb5",
    carts=["#d9483f", "#f0b33c", "#2f9c95", "#3f6fb5", "#7d4f8f", "#dcd6c8", "#5a5666"],
    doors=["#2f9c95", "#f6ecd2", "#f0b33c", "#d9483f"], tube=None,
)
NEON = dict(
    wall="#150b2e", dot="#22124a", floor="#0c061c", ol="#05020e",
    wood="#3a1d63", wood_l="#5b2f93", wood_d="#24103f",
    dark="#0b0618", glint="#ffffff", label="#f3e8ff",
    scr="#190a3a", scr2="#6a2bd9", lit="#ffe66d", hot="#ff2e97", cool="#2de2e6",
    leaf="#1fb98a", leaf2="#127a7a",
    grey="#8f86c9", grey2="#4c4387", teal="#15a3b5", red="#e0218a", mustard="#ff9a3c", plum="#7b2cd6", blue="#3a58d6",
    carts=["#ff2e97", "#2de2e6", "#ff9a3c", "#7b2cd6", "#3a58d6", "#8f86c9", "#ffe66d"],
    doors=["#24103f", "#3a1d63", "#24103f", "#3a1d63"], tube=("#2de2e6", "#ff2e97"),
)


def tube(a: Art, x0, x1, y, col, halo):
    """A neon strip: a bright line and a halo of two rows of its colour, thinned."""
    for d, step in ((1, 2), (2, 4)):
        m = a.box(x0, y + d, x1, y + d + 1) & ((a.xx + d) % step == 0)
        a.fill(m, halo)
    a.rect(x0, y, x1, y + 1, col)


def shelf_wide(P) -> Image.Image:
    a = Art(320, 248, P["wall"], P["ol"])
    a.fill((a.xx % 8 == 3) & (a.yy % 8 == 3), P["dot"])  # pegboard
    fy = 243
    a.rect(0, fy, 320, 248, P["floor"])
    a.rect(0, fy, 320, fy + 1, P["ol"])
    bench(a, 0, 320, 221, fy, P, seed=4)
    bt = 221
    # On the bench, left to right. The middle stays low.
    crt(a, 5, bt, 50, 40, P, P["grey"], P["grey2"], kind=1)
    console_slab(a, 64, bt, 34, P, P["grey"], P["plum"], with_cart=P["mustard"])
    a.cable([(72, bt - 1), (72, bt + 1), (104, bt + 1)], P["dark"])
    pad(a, 104, bt, P, P["plum"])
    for i, col in enumerate((P["red"], P["blue"], P["mustard"])):  # cartridges lying in a pile
        a.orect(132 + i, bt - 4 - i * 4, 146 + i, bt - i * 4, col)
        a.rect(135 + i, bt - 3 - i * 4, 143 + i, bt - 1 - i * 4, P["label"])
    console_dome(a, 160, bt, 30, P, P["teal"], P["grey"])
    pad(a, 196, bt, P, P["mustard"])
    joystick(a, 222, bt, P, P["red"])
    handheld(a, 246, bt, P, P["mustard"])
    console_wedge(a, 264, bt, 34, P, P["dark"], P["red"])
    snake_plant(a, 308, bt, P, P["red"], n=4, reach=24)
    # The shelves either side.
    for y in (66, 110, 156):
        shelf_plank(a, 4, 58, y, P)
    cart_row(a, 7, 56, 66, P, seed=1)
    handheld(a, 8, 110, P, P["teal"])
    cactus(a, 32, 110, P, P["mustard"])
    cart(a, 46, 110, 8, 12, P["red"], P)
    console_dome(a, 7, 156, 26, P, P["mustard"], P["grey"])
    pad(a, 38, 156, P, P["grey"])
    for y in (66, 114, 162):
        shelf_plank(a, 262, 316, y, P)
    snake_plant(a, 274, 66, P, P["teal"], n=4, reach=20)
    cart_row(a, 286, 314, 66, P, seed=5)
    cart_row(a, 265, 314, 114, P, seed=2)
    handheld(a, 265, 162, P, P["red"], wide_=True)
    joystick(a, 294, 162, P, P["teal"])
    if P["tube"]:
        c1, c2 = P["tube"]
        for y in (66, 110, 156):
            tube(a, 6, 56, y + 4, c1, P["scr2"])
        for y in (66, 114, 162):
            tube(a, 264, 314, y + 4, c2, P["plum"])
    return a.image()


def bench(a: Art, x0, x1, top, fy, P, seed):
    """A long low bench: a top, doors and open bays of cartridges, short legs."""
    rnd = random.Random(seed)
    a.orect(x0 - 2, top + 3, x1 + 2, fy - 4, P["wood_d"])
    x = x0 + 2
    i = 0
    while x < x1 - 6:
        w = rnd.choice([26, 30, 34])
        w = min(w, x1 - 2 - x)
        if i % 3 == 1:  # an open bay
            a.rect(x, top + 5, x + w, fy - 6, P["dark"])
            cart_row(a, x + 2, x + w - 1, fy - 6, P, seed=seed * 7 + i, lean=False)
        else:
            a.orect(x + 1, top + 6, x + w - 1, fy - 7, P["doors"][i % 4])
            a.rect(x + w - 5, (top + fy) // 2 - 1, x + w - 3, (top + fy) // 2 + 1, P["dark"])
        x += w + 2
        i += 1
    a.orect(x0 - 2, top, x1 + 2, top + 3, P["wood"])
    a.rect(x0 - 2, top, x1 + 2, top + 1, P["wood_l"])
    for lx in range(x0 + 6, x1 - 4, 44):
        a.orect(lx, fy - 4, lx + 3, fy, P["wood_d"])
    if P["tube"]:
        tube(a, x0, x1, top + 4, P["tube"][1], P["plum"])


def shelf_phone(P) -> Image.Image:
    a = Art(130, 120, P["wall"], P["ol"])
    a.fill((a.xx % 8 == 3) & (a.yy % 8 == 3) & (a.yy > 26), P["dot"])
    fy = 112
    a.rect(0, fy, 130, 120, P["floor"])
    a.rect(0, fy, 130, fy + 1, P["ol"])
    bench(a, 0, 130, 86, fy, P, seed=4)
    bt = 86
    crt(a, 3, bt, 44, 36, P, P["grey"], P["grey2"], kind=1)
    console_slab(a, 51, bt, 32, P, P["grey"], P["plum"], with_cart=P["mustard"])
    pad(a, 87, bt, P, P["plum"])
    snake_plant(a, 118, bt, P, P["red"], n=4, reach=22)
    shelf_plank(a, 52, 110, 50, P)
    cart_row(a, 55, 84, 50, P, seed=1)
    handheld(a, 88, 50, P, P["teal"])
    if P["tube"]:
        tube(a, 54, 108, 54, P["tube"][0], P["scr2"])
    return a.image()


# ---------------------------------------------------------------- chunky: the 16-bit playroom

CHUNKY = dict(
    wall="#1d5c6e", wall2="#184e5f", panel="#164453", skirt="#f3e3b3", ol="#0e1b26",
    floor="#d9843f", floor2="#b96a30", dark="#16212e", glint="#ffffff", label="#fbf3dc",
    scr="#152a44", scr2="#2c5d86", lit="#ffd447", hot="#ff5964", cool="#47d0c5",
    leaf="#4fbf66", leaf2="#2f8f58",
    wood="#c9793a", wood_l="#eaa35b", wood_d="#8b4a2b",
    grey="#e7e2d3", grey2="#a7a497", red="#e23d4b", pink="#ff7bac", yellow="#ffc93a", blue="#3d6fd1", purple="#7a4bc2",
    carts=["#e23d4b", "#ffc93a", "#47d0c5", "#3d6fd1", "#7a4bc2", "#e7e2d3", "#ff7bac"],
)


def chunky_room(a: Art, floor: int, P, flat: int = 0):
    """Wall, a panelled dado, the skirting and a floor of boards."""
    dado = floor - 52
    a.rect(0, dado, a.w, floor, P["wall2"])
    a.fill(a.box(0, dado + 4, a.w, floor - 4) & (a.xx % 16 == 0), P["panel"])
    a.rect(0, dado, a.w, dado + 2, P["panel"])
    if flat:
        a.rect(0, 0, a.w, flat, P["wall"])
    a.orect(-1, floor - 4, a.w + 1, floor, P["skirt"])
    a.rect(0, floor, a.w, a.h, P["floor"])
    a.rect(0, floor, a.w, floor + 1, P["ol"])
    for i, y in enumerate(range(floor + 9, a.h, 9)):
        a.rect(0, y, a.w, y + 1, P["floor2"])
        for x in range((i * 23) % 40 - 40, a.w, 40):
            a.rect(x, y - 8, x + 1, y, P["floor2"])


def rug(a: Art, cx, cy, rx, ry, P):
    m = a.disc(cx, cy, rx, ry)
    a.ink(m, P["purple"])
    a.fill(m & ~a.disc(cx, cy, rx - 4, ry - 2), P["pink"])
    a.fill(a.disc(cx, cy, rx - 12, ry - 5) & ~a.disc(cx, cy, rx - 15, ry - 6.5), P["yellow"])


def window(a: Art, x, y, w, h, P):
    """A window on an evening sky: bands of colour, a sun, a cloud."""
    a.orect(x - 2, y - 2, x + w + 2, y + h + 2, P["skirt"])
    bands = ("#3d6fd1", "#7a4bc2", "#ff7bac", "#ff9d5c", "#ffd447")
    for i, col in enumerate(bands):
        a.rect(x, y + i * h // len(bands), x + w, y + h, col)
    a.fill(a.disc(x + w * 0.68, y + h * 0.72, 7) & a.box(x, y, x + w, y + h), "#fff3b0")
    a.fill((a.disc(x + 10, y + h * 0.38, 6, 2.5) | a.disc(x + 15, y + h * 0.38 - 2, 4, 2.5)) & a.box(x, y, x + w, y + h), P["glint"])
    a.rect(x + w // 2 - 1, y, x + w // 2 + 1, y + h, P["skirt"])
    a.rect(x, y + h // 2 - 1, x + w, y + h // 2 + 1, P["skirt"])
    a.orect(x - 4, y + h + 2, x + w + 4, y + h + 5, P["skirt"])


def bookcase(a: Art, x, fy, w, rows, P, seed=3):
    h = rows * 22 + 5
    a.orect(x, fy - h, x + w, fy, P["wood"])
    a.rect(x, fy - h, x + w, fy - h + 1, P["wood_l"])
    for r in range(rows):
        y1 = fy - 4 - r * 22
        a.rect(x + 3, y1 - 18, x + w - 3, y1, P["wood_d"])
        if r == 1 and w >= 40:
            console_slab(a, x + 5, y1, 26, P, P["grey"], P["blue"])
            cart_row(a, x + 34, x + w - 3, y1, P, seed=seed + r, lean=False)
        else:
            cart_row(a, x + 4, x + w - 3, y1, P, seed=seed + r)


def chunky_wide() -> Image.Image:
    P = CHUNKY
    a = Art(320, 248, P["wall"], P["ol"])
    floor = 210
    chunky_room(a, floor, P)
    window(a, 14, 40, 42, 46, P)
    # Left: a television on a low stand, its console on the floor in front, pads trailing cables.
    a.orect(8, floor - 22, 66, floor + 2, P["red"])
    a.rect(8, floor - 22, 66, floor - 21, P["pink"])
    a.rect(12, floor - 17, 36, floor - 3, P["dark"])
    cart_row(a, 14, 35, floor - 3, P, seed=8, lean=False)
    a.orect(39, floor - 17, 62, floor - 3, P["yellow"])
    a.rect(58, floor - 11, 60, floor - 9, P["dark"])
    crt(a, 12, floor - 22, 50, 42, P, P["grey"], P["grey2"], kind=0)
    rug(a, 160, floor + 22, 92, 13, P)
    console_slab(a, 70, floor + 20, 34, P, P["grey"], P["purple"], with_cart=P["yellow"])
    a.cable([(104, floor + 18), (112, floor + 18), (118, floor + 22), (124, floor + 22)], P["dark"])
    pad(a, 122, floor + 25, P, P["red"])
    a.cable([(104, floor + 16), (140, floor + 16), (156, floor + 27), (168, floor + 28)], P["dark"])
    pad(a, 166, floor + 31, P, P["blue"])
    beanbag(a, 218, floor + 27, P, P["yellow"], "#e0a021")
    # Right: a bookcase of cartridges, a cactus on top, a snake plant beside it.
    bookcase(a, 266, floor + 3, 48, 4, P)
    cactus(a, 280, floor + 3 - 93, P, P["pink"])
    handheld(a, 296, floor + 3 - 93, P, P["cool"])
    snake_plant(a, 252, floor + 10, P, P["blue"], n=5, reach=34)
    return a.image()


def chunky_phone() -> Image.Image:
    P = CHUNKY
    a = Art(130, 120, P["wall"], P["ol"])
    floor = 92
    chunky_room(a, floor, P, flat=30)
    a.rect(0, 30, 130, 31, P["wall"])
    rug(a, 74, floor + 14, 50, 9, P)
    a.orect(3, floor - 18, 55, floor + 2, P["red"])
    a.rect(3, floor - 18, 55, floor - 17, P["pink"])
    a.rect(7, floor - 14, 30, floor - 2, P["dark"])
    cart_row(a, 9, 29, floor - 2, P, seed=8, lean=False)
    a.orect(33, floor - 14, 51, floor - 2, P["yellow"])
    crt(a, 6, floor - 18, 46, 38, P, P["grey"], P["grey2"], kind=0)
    console_slab(a, 50, floor + 14, 32, P, P["grey"], P["purple"], with_cart=P["yellow"])
    a.cable([(82, floor + 12), (86, floor + 12), (90, floor + 17)], P["dark"])
    pad(a, 88, floor + 21, P, P["red"])
    beanbag(a, 108, floor + 8, P, P["yellow"], "#e0a021")
    snake_plant(a, 70, floor + 1, P, P["blue"], n=4, reach=24)
    return a.image()


# ---------------------------------------------------------------- arcade: cabinets on a confetti carpet

ARCADE = dict(
    wall="#231942", wall2="#2d2157", ol="#0d0a1c", carpet="#1b1f5e", skirt="#0d0a1c",
    dark="#120e26", glint="#ffffff", label="#fbf3dc",
    scr="#0f1533", scr2="#2b3f9e", lit="#ffd23f", hot="#ff4d5a", cool="#3ddcff",
    leaf="#4fbf66", leaf2="#2f8f58",
    red="#e63946", red2="#9d1f3a", yellow="#ffc21a", yellow2="#c9820c", blue="#2a7de1", blue2="#1b4aa0",
    green="#35c26b", green2="#1e8a55", pink="#ff6fb5", grey="#d8d4e8", grey2="#8d87ad", white="#fbf3dc",
    carts=["#e63946", "#ffc21a", "#3ddcff", "#35c26b", "#ff6fb5", "#d8d4e8"],
)


def carpet(a: Art, floor: int, P, seed=6):
    """The carpet: scattered triangles, squiggles, rings and dots."""
    a.rect(0, floor, a.w, a.h, P["carpet"])
    a.rect(0, floor - 3, a.w, floor, P["skirt"])
    rnd = random.Random(seed)
    cols = (P["hot"], P["lit"], P["cool"], P["pink"], P["green"])
    for gx in range(4, a.w, 14):
        for gy in range(floor + 5, a.h - 2, 10):
            x, y = gx + rnd.randrange(-4, 5), gy + rnd.randrange(-2, 3)
            col = rnd.choice(cols)
            k = rnd.randrange(5)
            if k == 0:
                a.fill(a.quad(x + 2, x + 3, y - 2, x, x + 5, y + 2), col)
            elif k == 1:
                a.cable([(x, y), (x + 2, y - 2), (x + 4, y), (x + 6, y - 2)], col)
            elif k == 2:
                a.fill(a.disc(x + 2, y, 2.4, 1.8) & ~a.disc(x + 2, y, 1.1, 0.7), col)
            elif k == 3:
                a.rect(x, y, x + 2, y + 2, col)
            else:
                a.rect(x, y, x + 5, y + 1, col)


def wall_neon(a: Art, x, y, P):
    """Shapes of light on the wall: a ring, a triangle, a zigzag. No words."""
    ring = a.disc(x + 8, y + 8, 8) & ~a.disc(x + 8, y + 8, 6)
    a.fill(ring, P["hot"])
    tri = a.quad(x + 31, x + 32, y, x + 22, x + 41, y + 17)
    edge = tri & ~(a.quad(x + 31, x + 32, y + 4, x + 26, x + 37, y + 15))
    a.fill(edge, P["cool"])
    pts = [(x + 2, y + 26), (x + 9, y + 21), (x + 16, y + 26), (x + 23, y + 21), (x + 30, y + 26), (x + 37, y + 21)]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        a.seg(x0, y0, x1, y1, P["lit"], 1.8)


def arcade_wide() -> Image.Image:
    P = ARCADE
    a = Art(320, 248, P["wall"], P["ol"])
    floor = 210
    a.rect(0, floor - 40, 320, floor, P["wall2"])  # a darker band of wall above
    a.fill(a.box(0, floor - 42, 320, floor - 40) & (a.xx % 4 < 2), P["wall2"])
    carpet(a, floor, P)
    wall_neon(a, 8, 34, P)
    wall_neon(a, 270, 40, P)
    cabinet(a, -28, floor + 3, P, P["green"], P["green2"], P["yellow"], kind=2)
    cabinet(a, 18, floor + 6, P, P["red"], P["red2"], P["yellow"], kind=1)
    stool(a, 80, floor + 20, P, P["yellow"])
    crt(a, 222, floor + 8, 40, 34, P, P["grey"], P["grey2"], kind=2)
    console_wedge(a, 178, floor + 24, 30, P, P["white"], P["hot"])
    cabinet(a, 304, floor + 3, P, P["yellow"], P["yellow2"], P["red"], kind=2)
    cabinet(a, 262, floor + 6, P, P["blue"], P["blue2"], P["pink"], kind=0)
    a.cable([(208, floor + 22), (216, floor + 22), (224, floor + 9)], P["dark"])
    a.cable([(178, floor + 22), (170, floor + 22), (160, floor + 29), (150, floor + 29)], P["dark"])
    pad(a, 136, floor + 32, P, P["red"])
    return a.image()


def arcade_phone() -> Image.Image:
    P = ARCADE
    a = Art(130, 120, P["wall"], P["ol"])
    floor = 102
    a.rect(0, 40, 130, floor, P["wall2"])
    a.fill(a.box(0, 38, 130, 40) & (a.xx % 4 < 2), P["wall2"])
    carpet(a, floor, P)
    cabinet(a, 4, floor + 6, P, P["red"], P["red2"], P["yellow"], kind=1, h=74)
    cabinet(a, 100, floor + 6, P, P["blue"], P["blue2"], P["pink"], kind=0, h=74)
    crt(a, 54, floor + 5, 40, 34, P, P["grey"], P["grey2"], kind=2)
    console_wedge(a, 58, floor + 16, 30, P, P["white"], P["hot"])
    return a.image()


# ---------------------------------------------------------------- synth: sun, grid, ridges, palms

SYNTH = dict(
    sky=["#0d0221", "#130530", "#1b0840", "#260b52", "#360f66", "#4c1279", "#6a1687", "#8f1c8d", "#b8248a"],
    sun=["#fff275", "#ffd319", "#ffb11f", "#ff8a2b", "#ff5f4a", "#ff2e78", "#e4189a"],
    ground="#0f0326", grid="#ff2e97", grid2="#8f1c8d", far="#2de2e6",
    ridge="#160636", ridge2="#22094a", edge="#2de2e6", edge2="#b8248a", palm="#070114", star="#f3e8ff", star2="#9d7bd8",
)


def synth(w: int, h: int, hz: int, sun_r: int, drop: float, flat: int, palms, peak: float) -> Image.Image:
    P = SYNTH
    a = Art(w, h, P["sky"][0], None)
    # The sky: bands, wider high up, each boundary a row of alternate pixels.
    n = len(P["sky"])
    top = flat + 2
    edges = [top + (hz - top) * (1 - (1 - i / n) ** 1.9) for i in range(n + 1)]
    for i in range(1, n):
        y0 = int(edges[i])
        a.rect(0, y0, w, hz, P["sky"][i])
        a.fill(a.box(0, y0 - 1, w, y0) & ((a.xx + i) % 2 == 0), P["sky"][i])
    rnd = random.Random(21)
    for _ in range(w * hz // 420):
        x, y = rnd.randrange(w), rnd.randrange(flat + 2, max(flat + 3, int(hz * 0.7)))
        a.rect(x, y, x + 1, y + 1, P["star"] if rnd.random() < 0.4 else P["star2"])
    # The sun: bands of colour, slits that widen toward the horizon.
    cx, cy = w / 2, hz + drop  # drop: how far under the horizon its centre lies
    sun = a.disc(cx, cy, sun_r) & (a.yy < hz)
    t = np.clip((a.yy - (cy - sun_r)) / (hz - (cy - sun_r)), 0, 0.999)
    for i, col in enumerate(P["sun"]):
        a.fill(sun & (np.floor(t * len(P["sun"])) == i), col)
    y, gap = (cy - sun_r) + (hz - cy + sun_r) * 0.42, 1
    while y < hz:
        a.fill(sun & a.box(0, y, w, y + gap), P["sky"][-2])
        y += gap + max(3, sun_r // 12)
        gap += 1
    # Two ranges of ridges, high at the sides and low in the middle, edged in light.
    for depth, (fillc, edgec, amp, seed) in enumerate(((P["ridge2"], P["edge2"], 0.72, 5), (P["ridge"], P["edge"], 1.0, 9))):
        r = random.Random(seed)
        xs = list(range(0, w + 13, 12))
        side = [abs(x - w / 2) / (w / 2) for x in xs]
        ys = [hz - amp * peak * max(s - 0.3, 0) ** 1.25 * 1.9 * (0.72 + 0.2 * r.random() if k % 2 else 1.0) - (2 if depth == 0 else 0) for k, (x, s) in enumerate(zip(xs, side))]
        line = np.interp(a.xx[0], xs, ys)
        m = (a.yy >= np.floor(line)[None, :]) & (a.yy < hz)
        a.fill(m, fillc)
        inner = m.copy()
        inner[1:] &= m[:-1]
        inner[:, 1:] &= m[:, :-1]
        inner[:, :-1] &= m[:, 1:]
        a.fill(m & ~inner, edgec)
        if depth == 1:  # facets: a line down from each peak
            for k, (x, yv) in enumerate(zip(xs, ys)):
                if k % 2 == 0 and yv < hz - 6:
                    a.fill(a.segm(x, yv, x + 5, hz, 1) & m, P["ridge2"])
    # The ground: a grid running to the horizon.
    a.rect(0, hz, w, h, P["ground"])
    k = 0
    y = float(hz)
    step = 3.0
    while y < h:
        a.rect(0, int(y), w, int(y) + 1, P["grid2"] if k < 2 else P["grid"])
        y += step
        step *= 1.36
        k += 1
    for i in range(-30, 31):
        x1 = cx + i * w / 11
        m = a.segm(cx + i * w / 60, hz, x1, h, 1) & (a.yy > hz + 2)
        a.fill(m & (a.yy < hz + 5), P["grid2"])
        a.fill(m & (a.yy >= hz + 5), P["grid"])
    a.rect(0, hz, w, hz + 1, P["far"])
    for x, base, ht, lean in palms:
        palm(a, x, base, ht, lean, P)
    return a.image()


def palm(a: Art, x, base, ht, lean, P):
    """A palm in silhouette: a leaning trunk, fronds drooping from its head."""
    n = 14
    for i in range(n):
        t0, t1 = i / n, (i + 1) / n
        a.seg(x + lean * t0 ** 2, base - ht * t0, x + lean * t1 ** 2, base - ht * t1, P["palm"], 3.2 - 1.4 * t0)
    hx_, hy = x + lean, base - ht
    for ang, ln in ((-2.9, 0.34), (-2.45, 0.4), (-1.95, 0.36), (-1.2, 0.36), (-0.7, 0.4), (-0.25, 0.34), (-1.57, 0.22)):
        ln *= ht
        px, py = hx_, hy
        for s in range(1, 9):
            f = s / 8
            nx = hx_ + math.cos(ang) * ln * f
            ny = hy + math.sin(ang) * ln * f * 0.7 + ln * 0.55 * f * f
            a.seg(px, py, nx, ny, P["palm"], 2.6 - 1.4 * f)
            if s > 1:  # leaflets hanging from the rib
                a.seg(nx, ny, nx - math.cos(ang) * 2, ny + 3 + 2 * f, P["palm"], 1)
            px, py = nx, ny


def synth_wide() -> Image.Image:
    return synth(480, 372, 322, 64, 24, 0, [(34, 338, 150, 16), (86, 330, 96, -12), (448, 340, 158, -18), (398, 330, 90, 10)], 140)


def synth_phone() -> Image.Image:
    return synth(195, 180, 140, 38, -14, 44, [(18, 154, 78, 9), (180, 156, 84, -10)], 46)


# ---------------------------------------------------------------- day: site-scene.py's den on bright ramps

DAY_COOL = ["#1f2544", "#2d5773", "#3d9189", "#8ccfae", "#d8efd3", "#fdf8e7"]
DAY_WARM = ["#1f2544", "#8a3340", "#d6553f", "#f2924a", "#f9d06a", "#fdf8e7"]
DAY_WALL = 4.5  # the wall's step on the cool ramp: DAY_COOL[4] and [5], dithered evenly


def day(which: str) -> Image.Image:
    """site-scene.py's own things, on a canvas that lights the room for morning."""
    spec = importlib.util.spec_from_file_location("site_scene", Path(__file__).with_name("site-scene.py"))
    ss = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ss)
    lv = ss.lv
    shadows = {lv(0.4), lv(0.5)}  # what site-scene.py paints a cast shadow with

    class DayCanvas(ss.Canvas):
        def put(self, mask, lum, warm=0.0):
            if isinstance(lum, float) and lum in shadows:  # a shadow darkens what it falls on
                self.lum = np.where(mask, self.lum - 0.17, self.lum).astype(np.float32)
                return
            super().put(mask, lum, warm)

    def room(c, floor, flat_top=0):
        w, h = c.w, c.h
        wall = np.full_like(c.xx, lv(DAY_WALL)) - 0.04 * np.clip((c.yy - floor + 120) / 120, 0, 1)
        wall = wall - np.where((c.xx % 16 < 1), 0.05, 0) - np.where((c.xx % 16 == 8) & (c.yy % 6 < 3), 0.04, 0)
        if flat_top:
            k = np.clip((c.yy - 4) / flat_top, 0, 1)
            wall = lv(5) + (wall - lv(5)) * k
        ss.Canvas.put(c, c.yy < floor, wall, 0.0)
        ss.Canvas.put(c, c.box(0, floor - 5, w, floor), lv(5))
        ss.Canvas.put(c, c.box(0, floor - 1, w, floor), lv(1))
        vx, vy = w / 2, floor - 70
        fl = np.full_like(c.xx, lv(3.5)) - 0.05 * np.clip((c.yy - floor) / max(h - floor, 1), 0, 1)
        ang = (c.xx - vx) / np.maximum(c.yy - vy, 1)
        seam = np.abs((ang * 5.0) % 1.0 - 0.5) > 0.5 - 0.035 * (1 + 60 / np.maximum(c.yy - vy, 1))
        row = np.floor(np.log2(np.maximum(c.yy - floor + 6, 1) / 6) * 2.2)
        ends = (np.floor(ang * 5.0 + row * 0.37) % 3 == 0) & (np.abs((c.yy - floor + 6) - 6 * 2 ** ((row + 1) / 2.2)) < 0.8)
        fl = np.where(seam | ends, fl - 0.2, fl)
        ss.Canvas.put(c, c.yy >= floor, fl, 1.0)

    if which == "wide":
        c = DayCanvas(480, 372)
        floor = 310
        room(c, floor)
        ss.poster(c, 26, 84, 44, 36, 0)
        ss.poster(c, 402, 96, 30, 40, 1)
        ss.wall_shelf(c, 388, floor - 118, 80)
        ss.rug(c, 240, floor + 16, floor + 50, 78, 106)
        ss.cabinet(c, -40, floor + 2, seed=9, dim=0.5)
        ss.cabinet(c, 30, floor + 6)
        ss.stool(c, 112, floor + 22)
        ss.lamp(c, 377, floor + 3)
        ss.tv_unit(c, 384, floor + 6)
        ss.plant(c, 474, floor + 32, seed=8)
    else:
        c = DayCanvas(195, 180)
        floor = 148
        room(c, floor, flat_top=44)
        ss.rug(c, 70, floor + 12, floor + 30, 60, 74)
        ss.lamp(c, 84, floor + 2, height=96)
        ss.tv_unit(c, 98, floor + 5)
        ss.plant(c, 22, floor + 8, seed=8)
    ss.COOL, ss.WARM = DAY_COOL, DAY_WARM  # image() reads the ramps when it runs
    return c.image()


STYLES = {
    # name: (wide, phone, art-pixel scale, the flat colour at the phone picture's top)
    "day": dict(wide=lambda: day("wide"), phone=lambda: day("phone"), scale=4, wall=DAY_COOL[5]),
    "chunky": dict(wide=chunky_wide, phone=chunky_phone, scale=6, wall=CHUNKY["wall"]),
    "shelf": dict(wide=lambda: shelf_wide(SHELF), phone=lambda: shelf_phone(SHELF), scale=6, wall=SHELF["wall"]),
    "neon": dict(wide=lambda: shelf_wide(NEON), phone=lambda: shelf_phone(NEON), scale=6, wall=NEON["wall"]),
    "arcade": dict(wide=arcade_wide, phone=arcade_phone, scale=6, wall=ARCADE["wall"]),
    "synth": dict(wide=synth_wide, phone=synth_phone, scale=4, wall=SYNTH["sky"][0]),
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--style", action="append", choices=sorted(STYLES), help="draw only this style (repeatable); all of them by default")
    ap.add_argument("--preview", type=Path, help="write PNGs here instead of the site's WebP files")
    args = ap.parse_args()
    for style in args.style or list(STYLES):
        st = STYLES[style]
        for suffix, fn, size in (("", st["wide"], FULL), ("-phone", st["phone"], PHONE)):
            im = fn()
            assert (im.width * st["scale"], im.height * st["scale"]) == size, f"{style}{suffix}: {im.size} x{st['scale']} is not {size}"
            colours = im.getcolors(1 << 16)
            top = "#%02x%02x%02x" % im.getpixel((im.width // 2, 0))
            if suffix and st["wall"]:
                assert {c for _, c in im.crop((0, 0, im.width, 1)).getcolors()} == {hx(st["wall"])}, f"{style}{suffix}: the top row is not flat {st['wall']}"
            big = im.resize(size, Image.NEAREST)
            name = f"den-{style}{suffix}"
            if args.preview:
                args.preview.mkdir(parents=True, exist_ok=True)
                big.save(args.preview / f"{name}.png")
                print(args.preview / f"{name}.png", f"{len(colours)} colours, top {top}")
                continue
            out = OUT / f"{name}.webp"
            big.save(out, "WEBP", lossless=True, quality=100, method=6, exact=True)
            print(f"{out.relative_to(ROOT)}: {big.width}x{big.height} ({im.width}x{im.height} x{st['scale']}), {len(colours)} colours, top {top}, {out.stat().st_size:,} B")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
