# /// script
# requires-python = ">=3.11"
# dependencies = ["numpy==2.2.6", "pillow==11.3.0"]
# ///
"""The 3D desk scene's sprites (opendmd-web/share/desk/hd/): one atlas of original pixel art, drawn from code.

    uv run tools/desk-art-hd.py [--kit tools/site-scene-alt.py] [--v2 tools/desk-art.py] [--out opendmd-web/share/desk/hd]

Every thing of the flat scene's drawing (tools/desk-art.py) is drawn again ON ITS OWN, on a
clear ground, at the same size: one texel is 2 mm everywhere. Painted shadows
are left out (the 3D scene casts real ones). The house kit and the flat scene's drawing
functions are loaded, not copied. Shapes only: no photograph, no third-party
art, no characters, names, logos or marks.

Writes atlas.webp (lossless, alpha) and atlas.js: `export default {name: [x, y, w, h, ax, ay]}`
where (ax, ay) is where the crop sat in the flat scene's 640 x 400 drawing (so the scene can
place it: the desk meets the wall on row 300, the drawing's middle is x 320).
"""
import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
KEY = "#ff00ff"
W, H = 640, 400
PAD = 2


def load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--kit", type=Path, default=HERE / "site-scene-alt.py")
    ap.add_argument("--v2", type=Path, default=HERE / "desk-art.py")
    ap.add_argument("--out", type=Path, default=HERE.parent / "opendmd-web/share/desk/hd")
    args = ap.parse_args()
    K = load(args.kit, "site_scene_alt")
    D = load(args.v2, "desk_art")
    P = dict(K.SHELF)
    # painted shadows fall on the clear ground: they are not drawn
    P.update(ao1=KEY, ao2=KEY, sh=KEY, grain=D.mix(P["wood"], P["wood_d"], 0.3))
    mix = D.mix
    S = {}

    def sprite(name):
        def deco(fn):
            a = K.Art(W, H, KEY, P["ol"])
            fn(a)
            S[name] = a.px.copy()
            return fn
        return deco

    # ---- the wall
    sprite("p_sun")(lambda a: D.poster_sun(a, 204, 98, P))
    sprite("p_space")(lambda a: D.poster_space(a, 281, 124, P))
    sprite("p_shapes")(lambda a: D.poster_shapes(a, 378, 98, P))
    sprite("clock")(lambda a: D.clock(a, 150, 150, P))

    @sprite("board")
    def _(a):
        bx, by = 468, 112
        a.orect(bx, by, bx + 58, by + 44, P["wood"])
        a.rect(bx + 2, by + 2, bx + 56, by + 42, mix(P["wood"], P["wood_l"], 0.6))
        for nx, ny, nw, nh, c in ((5, 5, 14, 12, P["lit"]), (22, 4, 16, 16, P["label"]), (41, 6, 12, 10, P["cool"]), (6, 21, 12, 16, P["label"]), (21, 24, 14, 12, P["hot"]), (38, 20, 15, 18, P["label"])):
            a.rect(bx + nx, by + ny, bx + nx + nw, by + ny + nh, c)
            a.rect(bx + nx + nw // 2, by + ny, bx + nx + nw // 2 + 1, by + ny + 1, P["dark"])
            if c == P["label"]:
                for ly in range(by + ny + 3, by + ny + nh - 2, 3):
                    a.rect(bx + nx + 2, ly, bx + nx + nw - 2 - (ly % 2) * 3, ly + 1, P["grey2"])

    @sprite("dial")
    def _(a):  # the dimmer's plate (its knob is the scene's)
        a.orect(486, 204, 500, 224, P["label"])
        a.rect(486, 204, 500, 205, P["glint"])

    fy = 224
    sprite("s_carts")(lambda a: K.cart_row(a, 201, 248, fy, P, seed=11, lean=False))

    @sprite("s_cass")
    def _(a):
        for i, c in enumerate((P["plum"], P["teal"], P["red"])):
            D.cassette_side(a, 252 + (i % 2), fy - i * 5, c, P)

    sprite("s_boom")(lambda a: D.boombox(a, 279, fy, P))
    sprite("s_hand")(lambda a: K.handheld(a, 343, fy, P, P["mustard"]))
    sprite("s_fbox")(lambda a: D.floppy_box(a, 360, fy, P))
    sprite("s_cactus")(lambda a: K.cactus(a, 408, fy, P, P["teal"]))
    sprite("s_stick")(lambda a: K.joystick(a, 422, fy, P, P["red"]))

    # the side shelves: what stands on each plank, as one strip
    sprite("l1")(lambda a: K.cart_row(a, 18, 96, 120, P, seed=3))

    @sprite("l2")
    def _(a):
        K.console_dome(a, 18, 176, 28, P, P["teal"], P["grey"])
        K.pad(a, 52, 176, P, P["plum"])
        K.snake_plant(a, 84, 176, P, P["mustard"], n=4, reach=20)

    @sprite("l3")
    def _(a):
        K.handheld(a, 20, 232, P, P["red"], wide_=True)
        K.cart_row(a, 48, 96, 232, P, seed=6)

    @sprite("r1")
    def _(a):
        K.snake_plant(a, 562, 120, P, P["red"], n=5, reach=24)
        K.cart_row(a, 576, 628, 120, P, seed=9)

    @sprite("r2")
    def _(a):
        K.console_wedge(a, 552, 176, 34, P, P["dark"], P["red"])
        K.cactus(a, 606, 176, P, P["mustard"])

    @sprite("r3")
    def _(a):
        K.cart_row(a, 552, 600, 232, P, seed=2)
        K.joystick(a, 606, 232, P, P["teal"])

    # ---- on the desk, at its back
    sprite("slab")(lambda a: K.console_slab(a, 64, 312, 42, P, P["grey"], P["plum"], with_cart=P["mustard"]))
    sprite("plant")(lambda a: K.snake_plant(a, 28, 314, P, P["red"], n=5, reach=30))

    @sprite("pile")
    def _(a):
        for i, col in enumerate((P["red"], P["blue"], P["mustard"], P["teal"])):
            a.orect(550 + (i % 2) * 2, 310 - i * 4, 566 + (i % 2) * 2, 314 - i * 4, col)
            a.rect(553 + (i % 2) * 2, 311 - i * 4, 563 + (i % 2) * 2, 313 - i * 4, P["label"])

    sprite("dome")(lambda a: K.console_dome(a, 574, 314, 30, P, P["teal"], P["grey"]))
    sprite("cactus")(lambda a: K.cactus(a, 622, 314, P, P["mustard"]))

    @sprite("tube")  # the computer's tube, from the front (its screen is the scene's: it glows)
    def _(a):
        K.crt(a, 462, 298, 60, 46, P, P["grey"], P["grey2"], kind=3)
        a.rect(465, 255, 508, 291, P["scr"])

    @sprite("base")
    def _(a):
        a.ink(a.rrect(456, 298, 530, 312), P["grey"])
        a.rect(457, 298, 529, 299, P["glint"])
        a.rect(457, 309, 529, 311, P["grey2"])
        for sx in (462, 490):
            a.rect(sx, 302, sx + 22, 304, P["dark"])
            a.rect(sx + 8, 304, sx + 14, 306, P["grey2"])
        a.rect(520, 302, 524, 304, P["hot"])

    # ---- in front of the display
    sprite("pad")(lambda a: K.pad(a, 96, 352, P, P["plum"]))
    sprite("stick")(lambda a: K.joystick(a, 214, 368, P, P["red"]))

    def cassette(a, x, y, c):
        a.ink(a.rrect(x, y, x + 30, y + 19), c)
        a.rect(x + 3, y + 2, x + 27, y + 12, P["label"])
        a.rect(x + 3, y + 2, x + 27, y + 4, P["hot"] if c == P["dark"] else P["lit"])
        a.rect(x + 8, y + 6, x + 22, y + 10, P["dark"])
        a.rect(x + 10, y + 7, x + 12, y + 9, P["glint"])
        a.rect(x + 18, y + 7, x + 20, y + 9, P["glint"])
        a.fill(a.quad(x + 6, x + 24, y + 14, x + 4, x + 26, y + 19), mix(c, P["glint"], 0.18))

    sprite("cas1")(lambda a: cassette(a, 258, 338, P["plum"]))
    sprite("cas2")(lambda a: cassette(a, 268, 346, P["dark"]))

    def floppy(a, x, y, c):
        m = a.box(x, y, x + 22, y + 22) & ~a.box(x + 21, y, x + 22, y + 1)
        a.ink(m, c)
        a.rect(x + 6, y, x + 16, y + 7, P["grey"])
        a.rect(x + 12, y + 1, x + 14, y + 6, c)
        a.rect(x + 3, y + 10, x + 19, y + 21, P["label"])
        for ly in (y + 13, y + 16):
            a.rect(x + 5, ly, x + 17 - (ly % 2) * 4, ly + 1, P["grey2"])

    sprite("flop1")(lambda a: floppy(a, 316, 338, P["mustard"]))
    sprite("flop2")(lambda a: floppy(a, 330, 345, P["blue"]))

    @sprite("strip")  # the power strip, from above
    def _(a):
        a.ink(a.rrect(396, 352, 464, 366), P["grey"])
        a.rect(397, 352, 463, 353, P["glint"])
        a.rect(397, 363, 463, 365, P["grey2"])
        for sx in (404, 422, 440):
            a.rect(sx, 356, sx + 10, 361, P["label"])
            a.rect(sx + 2, 357, sx + 4, 360, P["dark"])
            a.rect(sx + 6, 357, sx + 8, 360, P["dark"])
        a.rect(455, 356, 460, 361, P["dark"])  # the switch's well (the switch is the scene's: it glows)

    @sprite("keys")  # the computer's keys, from above
    def _(a):
        x, y = 462, 330
        a.ink(a.rrect(x, y, x + 72, y + 20), P["grey"])
        a.rect(x + 1, y, x + 71, y + 1, P["glint"])
        a.rect(x + 1, y + 17, x + 71, y + 19, P["grey2"])
        for r in range(4):
            for kx in range(x + 3 + (r % 2), x + 68, 4):
                wide = r == 3 and x + 20 < kx < x + 48
                a.rect(kx, y + 3 + r * 3.5, kx + (4 if wide else 3), y + 5 + r * 3.5, P["hot"] if (kx + r * 9) % 31 == 0 else P["label"])

    @sprite("note")
    def _(a):
        a.orect(560, 336, 590, 360, P["label"])
        for ly in range(341, 356, 4):
            a.rect(564, ly, 586 - (ly % 3) * 3, ly + 1, P["grey2"])

    # ---- crop, pack
    key = np.array(K.hx(KEY), dtype=np.uint8)
    crops = {}
    for name, px in S.items():
        m = ~np.all(px == key, axis=-1)
        ys, xs = np.where(m)
        x0, x1, y0, y1 = xs.min(), xs.max() + 1, ys.min(), ys.max() + 1
        rgba = np.dstack([px[y0:y1, x0:x1], np.where(m[y0:y1, x0:x1], 255, 0).astype(np.uint8)])
        rgba[~m[y0:y1, x0:x1]] = 0
        crops[name] = (Image.fromarray(rgba), int(x0), int(y0))
    AW = 256
    x = y = row = PAD
    x = PAD
    place = {}
    for name, (im, ax, ay) in sorted(crops.items(), key=lambda kv: -kv[1][0].size[1]):
        w, h = im.size
        if x + w + PAD > AW:
            x, y, row = PAD, y + row + PAD, 0
        place[name] = (x, y, w, h, ax, ay)
        x += w + PAD
        row = max(row, h)
    AH = y + row + PAD
    AH = (AH + 3) // 4 * 4
    atlas = Image.new("RGBA", (AW, AH), (0, 0, 0, 0))
    for name, (x, y, w, h, ax, ay) in place.items():
        atlas.paste(crops[name][0], (x, y))
    # bleed each sprite's edge colour one texel into its clear border (so filtering never pulls in black)
    px = np.array(atlas)
    for _ in range(1):
        a = px[..., 3] > 0
        out = px.copy()
        for dy, dx in ((0, 1), (0, -1), (1, 0), (-1, 0)):
            sh = np.roll(px, (dy, dx), axis=(0, 1))
            take = ~a & (sh[..., 3] > 0) & (out[..., 3] == 0) & ~np.any(out[..., :3] > 0, axis=-1)
            out[take, :3] = sh[take, :3]
        px = out
    atlas = Image.fromarray(px)
    args.out.mkdir(parents=True, exist_ok=True)
    f = args.out / "atlas.webp"
    atlas.save(f, "WEBP", lossless=True, quality=100, method=6, exact=True)
    (args.out / "atlas.js").write_text("// made by tools/desk-art-hd.py: name: [x, y, w, h, ax, ay]; the atlas is %d x %d\nexport const ATLAS_W = %d, ATLAS_H = %d;\nexport default %s;\n" % (AW, AH, AW, AH, json.dumps(place, separators=(",", ":"))))
    (HERE.parent / "target").mkdir(exist_ok=True)
    atlas.resize((AW * 4, AH * 4), Image.NEAREST).save(HERE.parent / "target/desk-atlas-preview.png")
    print(f"{f}: {AW}x{AH}, {f.stat().st_size} bytes, {len(place)} sprites")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
