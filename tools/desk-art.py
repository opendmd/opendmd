# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.2.6", "pillow==11.3.0"]
# ///
"""Draw the demo's desk scene's den (the flat scene, opendmd-web/share/desk/flat.js): three layers of
original pixel art, from code.

    uv run tools/desk-art.py [--kit tools/site-scene-alt.py] [--out opendmd-web/share/desk/room] [--preview]

The house kit (tools/site-scene-alt.py: Art, the shelf palette and its made-up
machines) is loaded, not copied. Everything is rectangles, discs and lines: no
photograph, no model, no third-party art, no game's characters, no logos, words
or marks, and no real machine's design.

One art pixel is half an LED pitch (2 mm). The drawing is 640 x 400 art pixels;
the display stands in the middle (x 320), its feet on y 326, the desk meets the
wall on y 300. Three files, so the scene can move them apart (parallax):

  wall.webp   640 x 316, opaque   rows   0..316   the wall, posters, the shelf (far)
  desk.webp   640 x 176, alpha    rows 224..400   the desk top and what stands at its back
  near.webp   640 x  80, alpha    rows 320..400   what lies in front of the display (near)

Lossless WebP at one file pixel per art pixel; the page scales them with
image-rendering: pixelated to a whole number of device pixels.
"""
import argparse
import importlib.util
import random
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
KEY = "#ff00ff"
W, H = 640, 400
WALL_H, DESK_Y0, NEAR_Y0 = 316, 224, 320
LINE, FOOT = 300, 326  # the desk meets the wall; the display's feet


def load_kit(path: Path):
    spec = importlib.util.spec_from_file_location("site_scene_alt", path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def mix(c1, c2, t):
    a, b = (int(c1[i:i + 2], 16) for i in (1, 3, 5)), (int(c2[i:i + 2], 16) for i in (1, 3, 5))
    return "#%02x%02x%02x" % tuple(round(x + (y - x) * t) for x, y in zip(a, b))


# ---------------------------------------------------------------- the wall

def poster(a, x, y, w, h, bg, P):
    a.rect(x + 2, y + 2, x + w + 2, y + h + 2, P["ao1"])  # its shadow on the wall
    a.orect(x, y, x + w, y + h, P["label"])
    a.rect(x + 2, y + 2, x + w - 2, y + h - 2, bg)
    a.rect(x + w // 2 - 2, y - 1, x + w // 2 + 2, y + 1, P["hot"])  # a pin
    return a.box(x + 2, y + 2, x + w - 2, y + h - 2)


def poster_sun(a, x, y, P):
    """A banded sun over a grid."""
    w, h = 58, 84
    inn = poster(a, x, y, w, h, P["dark"], P)
    cx, cy, r = x + w / 2, y + 36, 18
    sun = a.disc(cx, cy, r) & inn
    for i, yy in enumerate(range(int(cy - r), int(cy + r) + 1)):
        t = i / (2 * r)
        col = P["lit"] if t < 0.35 else P["mustard"] if t < 0.6 else P["hot"] if t < 0.85 else P["red"]
        a.fill(sun & (a.yy == yy), col)
    for yy, th in ((cy + 2, 1), (cy + 6, 1), (cy + 10, 2), (cy + 14, 2)):
        a.fill(sun & (a.yy >= yy) & (a.yy < yy + th), P["dark"])
    hz = int(cy + r) + 2
    a.fill(inn & (a.yy >= hz), P["scr"])
    a.fill(inn & (a.yy == hz), P["cool"])
    step, yy = 2, hz + 2
    while yy < y + h - 2:
        a.fill(inn & (a.yy == yy), P["teal"])
        step += 1
        yy += step
    for k in range(-5, 6):
        a.fill(a.segm(cx + k * 3, hz, cx + k * 14, y + h - 2, 1) & inn & (a.yy > hz), P["teal"])
    for sx, sy in ((6, 8), (14, 15), (45, 6), (50, 18), (9, 26), (40, 12)):
        a.rect(x + sx, y + sy, x + sx + 1, y + sy + 1, P["glint"])


def poster_space(a, x, y, P):
    """A made-up rocket and a ringed planet."""
    w, h = 78, 56
    inn = poster(a, x, y, w, h, P["scr"], P)
    cx, cy = x + 54, y + 30
    a.fill(a.disc(cx, cy, 12) & inn, P["plum"])
    a.fill(a.disc(cx - 3, cy - 4, 6, 5) & inn, mix(P["plum"], P["label"], 0.3))
    ring = a.disc(cx, cy + 1, 21, 5.5) & ~a.disc(cx, cy + 1, 17, 3.2) & inn
    a.fill(ring, P["lit"])
    a.fill(a.disc(cx, cy, 12) & (a.yy < cy) & inn, P["plum"])
    a.fill(a.disc(cx - 3, cy - 4, 6, 5) & (a.yy < cy) & inn, mix(P["plum"], P["label"], 0.3))
    rx, ry = x + 16, y + 12
    a.fill(a.quad(rx + 3, rx + 4, ry, rx, rx + 7, ry + 6), P["hot"])
    a.rect(rx, ry + 6, rx + 7, ry + 22, P["grey"])
    a.rect(rx + 5, ry + 6, rx + 7, ry + 22, P["grey2"])
    a.fill(a.disc(rx + 3.5, ry + 11, 2), P["cool"])
    a.fill(a.quad(rx - 1, rx, ry + 15, rx - 4, rx, ry + 23), P["hot"])
    a.fill(a.quad(rx + 7, rx + 8, ry + 15, rx + 7, rx + 11, ry + 23), P["hot"])
    a.fill(a.quad(rx + 1, rx + 6, ry + 23, rx + 3, rx + 4, ry + 32), P["lit"])
    a.rect(rx + 3, ry + 23, rx + 4, ry + 27, P["glint"])
    rnd = random.Random(8)
    for _ in range(16):
        sx, sy = rnd.randrange(x + 4, x + w - 4), rnd.randrange(y + 4, y + h - 4)
        if not (a.disc(cx, cy, 22, 14)[sy, sx] or (rx - 5 <= sx <= rx + 12)):
            a.rect(sx, sy, sx + 1, sy + 1, P["glint"] if rnd.random() < 0.6 else P["cool"])


def poster_shapes(a, x, y, P):
    """Bold shapes: a triangle, a disc, a squiggle, dots."""
    w, h = 58, 84
    inn = poster(a, x, y, w, h, P["label"], P)
    a.fill(a.quad(x + 30, x + 31, y + 8, x + 14, x + 47, y + 36) & inn, P["teal"])
    a.fill(a.disc(x + 38, y + 40, 12) & inn, P["hot"])
    a.fill(a.disc(x + 38, y + 40, 12) & a.quad(x + 30, x + 31, y + 8, x + 14, x + 47, y + 36) & inn, P["dark"])
    pts = [(x + 8, y + 62), (x + 14, y + 54), (x + 20, y + 62), (x + 26, y + 54), (x + 32, y + 62), (x + 38, y + 54)]
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        a.fill(a.segm(x0, y0, x1, y1, 2.6) & inn, P["mustard"])
    for i in range(4):
        for j in range(3):
            a.rect(x + 8 + i * 4, y + 10 + j * 4, x + 10 + i * 4, y + 12 + j * 4, P["plum"])
    for i in range(5):
        a.fill(a.box(x + 40, y + 60 + i * 4, x + 52, y + 62 + i * 4) & inn, P["blue"])
    a.fill(a.disc(x + 14, y + 74, 4) & inn, P["blue"])


def clock(a, cx, cy, P):
    a.fill(a.disc(cx + 2, cy + 2, 15), P["ao1"])
    a.ink(a.disc(cx, cy, 15), P["teal"])
    a.ink(a.disc(cx, cy, 12), P["label"])
    for k in range(12):
        import math
        ang = k * math.pi / 6
        a.rect(round(cx + math.sin(ang) * 10 - 0.5), round(cy - math.cos(ang) * 10 - 0.5), round(cx + math.sin(ang) * 10 + 0.5), round(cy - math.cos(ang) * 10 + 0.5), P["dark"])
    a.seg(cx, cy, cx - 5, cy - 4, P["dark"], 1.8)
    a.seg(cx, cy, cx + 6, cy - 7, P["dark"], 1.2)
    a.rect(cx - 1, cy - 1, cx + 1, cy + 1, P["hot"])


def cassette_side(a, x, fy, col, P, w=20):
    a.orect(x, fy - 5, x + w, fy, col)
    a.rect(x + 2, fy - 4, x + w - 2, fy - 1, P["label"])
    a.rect(x + w // 2 - 3, fy - 3, x + w // 2 + 3, fy - 2, P["dark"])


def boombox(a, x, fy, P):
    """A made-up radio-cassette: two speakers, a door, a row of keys, a handle."""
    w, h = 58, 22
    a.seg(x + 10, fy - h, x + 12, fy - h - 5, P["ol"], 1.6)
    a.seg(x + w - 10, fy - h, x + w - 12, fy - h - 5, P["ol"], 1.6)
    a.rect(x + 12, fy - h - 6, x + w - 12, fy - h - 4, P["ol"])
    a.ink(a.rrect(x, fy - h, x + w, fy), P["grey"])
    a.rect(x + 1, fy - h, x + w - 1, fy - h + 1, P["glint"])
    a.rect(x + 1, fy - 3, x + w - 1, fy - 1, P["grey2"])
    for cx in (x + 10.5, x + w - 10.5):
        a.fill(a.disc(cx, fy - 10.5, 8), P["dark"])
        a.fill(a.disc(cx, fy - 10.5, 6) & (((a.xx + a.yy) % 2) < 1), P["grey2"])
        a.fill(a.disc(cx, fy - 10.5, 2.4), P["dark"])
    a.rect(x + 21, fy - 16, x + w - 21, fy - 7, P["dark"])
    a.rect(x + 22, fy - 15, x + w - 22, fy - 11, P["label"])
    a.rect(x + 25, fy - 14, x + 27, fy - 12, P["dark"])
    a.rect(x + w - 27, fy - 14, x + w - 25, fy - 12, P["dark"])
    for i, c in enumerate((P["hot"], P["dark"], P["dark"], P["dark"], P["cool"])):
        a.rect(x + 22 + i * 3, fy - 20, x + 24 + i * 3, fy - 18, c)
    a.rect(x + 22, fy - 5, x + w - 22, fy - 4, P["dark"])
    a.rect(x + 26, fy - 5, x + 28, fy - 4, P["hot"])


def floppy_box(a, x, fy, P):
    cols = [P["blue"], P["dark"], P["red"], P["mustard"], P["dark"], P["teal"], P["plum"]]
    for i, c in enumerate(cols):
        a.orect(x + 2 + i * 4, fy - 14 - (i % 3 == 1), x + 5 + i * 4, fy - 4, c)
        a.rect(x + 2 + i * 4, fy - 12, x + 5 + i * 4, fy - 10, P["label"])
    a.ink(a.rrect(x, fy - 8, x + 33, fy), P["grey"])
    a.rect(x + 1, fy - 8, x + 32, fy - 7, P["glint"])
    a.rect(x + 13, fy - 5, x + 20, fy - 3, P["grey2"])


def wall(K, P):
    a = K.Art(W, WALL_H, P["wall"], P["ol"])
    a.fill((a.xx % 8 == 3) & (a.yy % 8 == 3), P["dot"])  # the house pegboard
    # where the desk meets the wall: three steps of shade (ambient occlusion)
    a.fill((a.yy >= 280) & (a.yy < 288) & ((a.xx + a.yy) % 2 == 0), P["ao1"])
    a.rect(0, 288, W, 294, P["ao1"])
    a.fill((a.yy >= 294) & (a.yy < 297) & ((a.xx + a.yy) % 2 == 0), P["ao2"])
    a.rect(0, 297, W, WALL_H, P["ao2"])
    # posters
    poster_sun(a, 204, 98, P)
    poster_space(a, 281, 124, P)
    poster_shapes(a, 378, 98, P)
    # the shelf over the display
    a.rect(198, 229, 446, 232, P["ao1"])
    K.shelf_plank(a, 196, 444, 224, P)
    fy = 224
    K.cart_row(a, 201, 248, fy, P, seed=11, lean=False)
    for i, c in enumerate((P["plum"], P["teal"], P["red"])):
        cassette_side(a, 252 + (i % 2), fy - i * 5, c, P)
    boombox(a, 279, fy, P)
    K.handheld(a, 343, fy, P, P["mustard"])
    floppy_box(a, 360, fy, P)
    K.cactus(a, 408, fy, P, P["teal"])
    K.joystick(a, 422, fy, P, P["red"])
    # the sides: a clock, a board of notes, and shelves further out
    clock(a, 150, 150, P)
    bx, by = 468, 112
    a.rect(bx + 2, by + 2, bx + 60, by + 46, P["ao1"])
    a.orect(bx, by, bx + 58, by + 44, P["wood"])
    a.rect(bx + 2, by + 2, bx + 56, by + 42, mix(P["wood"], P["wood_l"], 0.6))
    for nx, ny, nw, nh, c in ((5, 5, 14, 12, P["lit"]), (22, 4, 16, 16, P["label"]), (41, 6, 12, 10, P["cool"]), (6, 21, 12, 16, P["label"]), (21, 24, 14, 12, P["hot"]), (38, 20, 15, 18, P["label"])):
        a.rect(bx + nx, by + ny, bx + nx + nw, by + ny + nh, c)
        a.rect(bx + nx + nw // 2, by + ny, bx + nx + nw // 2 + 1, by + ny + 1, P["dark"])
        if c == P["label"]:
            for ly in range(by + ny + 3, by + ny + nh - 2, 3):
                a.rect(bx + nx + 2, ly, bx + nx + nw - 2 - (ly % 2) * 3, ly + 1, P["grey2"])
    for y in (120, 176, 232):
        K.shelf_plank(a, 14, 98, y, P)
    K.cart_row(a, 18, 96, 120, P, seed=3)
    K.console_dome(a, 18, 176, 28, P, P["teal"], P["grey"])
    K.pad(a, 52, 176, P, P["plum"])
    K.snake_plant(a, 84, 176, P, P["mustard"], n=4, reach=20)
    K.handheld(a, 20, 232, P, P["red"], wide_=True)
    K.cart_row(a, 48, 96, 232, P, seed=6)
    for y in (120, 176, 232):
        K.shelf_plank(a, 548, 630, y, P)
    K.snake_plant(a, 562, 120, P, P["red"], n=5, reach=24)
    K.cart_row(a, 576, 628, 120, P, seed=9)
    K.console_wedge(a, 552, 176, 34, P, P["dark"], P["red"])
    K.cactus(a, 606, 176, P, P["mustard"])
    K.cart_row(a, 552, 600, 232, P, seed=2)
    K.joystick(a, 606, 232, P, P["teal"])
    return a


# ---------------------------------------------------------------- the desk and what stands at its back

def desk(K, P):
    a = K.Art(W, H, KEY, P["ol"])
    a.rect(0, LINE, W, H, P["wood"])
    a.rect(0, LINE, W, LINE + 1, P["ol"])
    a.rect(0, LINE + 1, W, LINE + 3, P["wood_l"])
    a.rect(0, LINE + 3, W, LINE + 6, P["sh"])  # the wall's shade on the desk
    a.fill((a.yy >= LINE + 6) & (a.yy < LINE + 9) & ((a.xx + a.yy) % 2 == 0), P["sh"])
    rnd = random.Random(5)
    for y in (324, 352, 382):  # board joints, and where boards butt
        a.rect(0, y, W, y + 1, P["wood_d"])
        a.rect(0, y + 1, W, y + 2, P["wood_l"])
    for y0, y1 in ((LINE + 3, 324), (326, 352), (354, 382), (384, H)):
        x = rnd.randrange(20, 90)
        while x < W:
            a.rect(x, y0, x + 1, y1, P["wood_d"])
            x += rnd.randrange(90, 170)
    for _ in range(90):  # grain
        x, y = rnd.randrange(0, W - 12), rnd.randrange(LINE + 10, H - 1)
        a.rect(x, y, x + rnd.randrange(4, 12), y + 1, P["grain"])

    def shadow(x0, x1, y, d=3):
        a.fill(a.quad(x0, x1, y, x0 + 4, x1 + 6, y + d) & (a.yy >= LINE), P["sh"])

    # left: a made-up console, its cartridge in; the lamp
    shadow(64, 108, 312)
    K.console_slab(a, 64, 312, 42, P, P["grey"], P["plum"], with_cart=P["mustard"])
    K.snake_plant(a, 28, 314, P, P["red"], n=5, reach=30)
    shadow(134, 160, 314)
    a.ink(a.rrect(134, 309, 160, 314), P["dark"])
    a.rect(136, 309, 158, 310, P["grey2"])
    a.seg(146, 309, 140, 266, P["ol"], 2.6)
    a.seg(140, 266, 166, 240, P["ol"], 2.6)
    a.seg(146, 308, 140, 266, P["grey2"], 1)
    a.ink(a.disc(140, 266, 2.6), P["mustard"])
    a.ink(a.disc(146, 308, 2.2), P["mustard"])
    a.ink(a.quad(160, 172, 232, 152, 186, 250), P["mustard"])  # the shade
    a.fill(a.quad(161, 164, 233, 153, 160, 250), P["lit"])
    a.rect(158, 250, 180, 252, P["lit"])  # the bulb's rim (lit by the page when the lamp is on)
    # right: a made-up computer, its tube on top
    shadow(456, 530, 312)
    a.ink(a.rrect(456, 298, 530, 312), P["grey"])
    a.rect(457, 298, 529, 299, P["glint"])
    a.rect(457, 309, 529, 311, P["grey2"])
    for sx in (462, 490):
        a.rect(sx, 302, sx + 22, 304, P["dark"])
        a.rect(sx + 8, 304, sx + 14, 306, P["grey2"])
    a.rect(520, 302, 524, 304, P["hot"])
    K.crt(a, 462, 298, 60, 46, P, P["grey"], P["grey2"], kind=3)
    a.rect(465, 255, 508, 291, P["scr"])  # the page lays the live screen here
    shadow(548, 600, 314)
    for i, col in enumerate((P["red"], P["blue"], P["mustard"], P["teal"])):  # cartridges in a pile
        a.orect(550 + (i % 2) * 2, 310 - i * 4, 566 + (i % 2) * 2, 314 - i * 4, col)
        a.rect(553 + (i % 2) * 2, 311 - i * 4, 563 + (i % 2) * 2, 313 - i * 4, P["label"])
    K.console_dome(a, 574, 314, 30, P, P["teal"], P["grey"])
    K.cactus(a, 622, 314, P, P["mustard"])
    return a


# ---------------------------------------------------------------- what lies in front of the display

def near(K, P):
    a = K.Art(W, H, KEY, P["ol"])

    def shadow(x0, x1, y, d=2):
        a.fill(a.quad(x0 + 1, x1 + 1, y, x0 + 4, x1 + 5, y + d), P["sh"])

    # a mug
    shadow(138, 150, 356)
    a.ink(a.box(150, 345, 154, 352) & ~a.box(150, 347, 152, 350), P["red"])
    a.ink(a.rrect(138, 340, 151, 356), P["red"])
    a.rect(139, 340, 150, 341, mix(P["red"], P["glint"], 0.4))
    a.rect(140, 344, 149, 346, P["label"])
    # a pad on its lead
    a.cable([(96, 346), (88, 342), (70, 338), (60, 330), (58, 320)], P["dark"])
    shadow(96, 111, 352)
    K.pad(a, 96, 352, P, P["plum"])
    # a stick, its lead running back behind the display
    a.cable([(214, 364), (204, 364), (198, 358), (198, 344), (204, 334), (208, 322)], P["dark"])
    shadow(214, 230, 368)
    K.joystick(a, 214, 368, P, P["red"])
    # two cassettes lying flat
    for x, y, c in ((258, 338, P["plum"]), (268, 346, P["dark"])):
        shadow(x, x + 30, y + 19)
        a.ink(a.rrect(x, y, x + 30, y + 19), c)
        a.rect(x + 3, y + 2, x + 27, y + 12, P["label"])
        a.rect(x + 3, y + 2, x + 27, y + 4, P["hot"] if c == P["dark"] else P["lit"])
        a.rect(x + 8, y + 6, x + 22, y + 10, P["dark"])
        a.rect(x + 10, y + 7, x + 12, y + 9, P["glint"])
        a.rect(x + 18, y + 7, x + 20, y + 9, P["glint"])
        a.fill(a.quad(x + 6, x + 24, y + 14, x + 4, x + 26, y + 19), mix(c, P["glint"], 0.18))
    # two floppies
    for x, y, c in ((316, 338, P["mustard"]), (330, 345, P["blue"])):
        shadow(x, x + 22, y + 22)
        m = a.box(x, y, x + 22, y + 22) & ~a.box(x + 21, y, x + 22, y + 1)
        a.ink(m, c)
        a.rect(x + 6, y, x + 16, y + 7, P["grey"])
        a.rect(x + 12, y + 1, x + 14, y + 6, c)
        a.rect(x + 3, y + 10, x + 19, y + 21, P["label"])
        for ly in (y + 13, y + 16):
            a.rect(x + 5, ly, x + 17 - (ly % 2) * 4, ly + 1, P["grey2"])
    # the power strip (the charger and its lead are the page's: they move)
    shadow(396, 464, 366)
    a.ink(a.rrect(396, 352, 464, 366), P["grey"])
    a.rect(397, 352, 463, 353, P["glint"])
    a.rect(397, 363, 463, 365, P["grey2"])
    for sx in (404, 422, 440):
        a.rect(sx, 356, sx + 10, 361, P["label"])
        a.rect(sx + 2, 357, sx + 4, 360, P["dark"])
        a.rect(sx + 6, 357, sx + 8, 360, P["dark"])
    a.rect(455, 356, 460, 361, P["red"])
    a.cable([(464, 360), (476, 360), (486, 364), (520, 366), (560, 368), (W, 368)], P["dark"])
    # the computer's keys
    shadow(462, 534, 350)
    a.ink(a.quad(468, 530, 334, 462, 534, 346) | a.box(462, 346, 534, 350), P["grey"])
    a.rect(463, 347, 533, 349, P["grey2"])
    for r, y in enumerate((336, 339, 342)):
        for kx in range(470 - r * 2, 528 + r * 2, 4):
            a.rect(kx, y, kx + 3, y + 2, P["label"] if (kx + r) % 28 else P["hot"])
    a.cable([(500, 334), (500, 328), (496, 322)], P["dark"])
    # a pencil and a note
    a.orect(560, 336, 590, 360, P["label"])
    for ly in range(341, 356, 4):
        a.rect(564, ly, 586 - (ly % 3) * 3, ly + 1, P["grey2"])
    a.seg(572, 364, 604, 350, P["ol"], 3.4)
    a.seg(573, 364, 603, 351, P["lit"], 1.6)
    a.rect(603, 349, 606, 351, P["hot"])
    return a


def rgba(a, y0, y1):
    px = a.px[y0:y1]
    key = np.all(px == np.array([255, 0, 255], dtype=np.uint8), axis=-1)
    out = np.dstack([px, np.where(key, 0, 255).astype(np.uint8)])
    out[key] = 0
    return Image.fromarray(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--kit", type=Path, default=HERE / "site-scene-alt.py")
    ap.add_argument("--out", type=Path, default=HERE.parent / "opendmd-web/share/desk/room")
    ap.add_argument("--preview", action="store_true", help="also write target/desk-preview.png: the three layers together, x3 (never beside the art: it would be served)")
    args = ap.parse_args()
    K = load_kit(args.kit)
    P = dict(K.SHELF)
    P.update(ao1=mix(P["wall"], P["dot"], 0.75), ao2=mix(P["dot"], P["floor"], 0.55),
             sh=mix(P["wood"], P["wood_d"], 0.62), grain=mix(P["wood"], P["wood_d"], 0.3))
    args.out.mkdir(parents=True, exist_ok=True)
    wl, dk, nr = wall(K, P), desk(K, P), near(K, P)
    layers = {"wall": Image.fromarray(wl.px), "desk": rgba(dk, DESK_Y0, H), "near": rgba(nr, NEAR_Y0, H)}
    total = 0
    for name, im in layers.items():
        f = args.out / f"{name}.webp"
        im.save(f, "WEBP", lossless=True, quality=100, method=6)
        total += f.stat().st_size
        print(f"{f.name}: {im.size[0]}x{im.size[1]}, {f.stat().st_size} bytes, {len(im.convert('RGB').getcolors(1 << 16))} colours")
    print(f"total {total} bytes")
    if args.preview:
        full = Image.new("RGBA", (W, H), P["wall"])
        full.paste(layers["wall"], (0, 0))
        full.alpha_composite(layers["desk"], (0, DESK_Y0))
        full.alpha_composite(layers["near"], (0, NEAR_Y0))
        (HERE.parent / "target").mkdir(exist_ok=True)
        full.resize((W * 3, H * 3), Image.NEAREST).save(HERE.parent / "target/desk-preview.png")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
