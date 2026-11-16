# /// script
# requires-python = ">=3.11"
# dependencies = ["qrcodegen==1.8.0", "zxing-cpp==2.3.0", "numpy>=1.26", "pillow>=10"]
# ///
"""The pairing QR code against code that is not ours (`make test`): the
core's matrices (opendmd-core/tests/connect-vectors.json `qr_code`, pinned
by tests/connect_vectors.rs) module for module against qrcodegen's for each of
the eight masks, and the panel's QR screen, as the panel lights it (one LED
a module), decoded by ZXing (zxing-cpp) from pictures of the panel: plain
squares, and LEDs as dots with dark gaps between them, as a camera sees a
HUB75 panel.

Red: a matrix with one module changed is caught by the comparison; a
screen with a fifth of its modules flipped does not decode (version 3-L
corrects about 7%); a mask other than the one asked does not match.

    uv run tools/connect/qr_check.py [--out DIR]   # DIR: the pictures, to look at
"""
from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

import numpy as np
from qrcodegen import QrCode, QrSegment
import zxingcpp
from PIL import Image

VECTORS = Path(__file__).resolve().parents[2] / "opendmd-core" / "tests" / "connect-vectors.json"
failed = 0


def check(cond, what):
    global failed
    print(("pass  " if cond else "FAIL  ") + what)
    if not cond:
        failed += 1


def other_rows(text: str, mask: int | None) -> tuple[list[str], int]:
    """Project Nayuki's QR Code generator (qrcodegen), held to 3-L, byte mode."""
    seg = QrSegment.make_bytes(text.encode())
    q = QrCode.encode_segments([seg], QrCode.Ecc.LOW, 3, 3, -1 if mask is None else mask, boostecl=False)
    assert q.get_version() == 3 and q.get_size() == 29
    return ["".join("#" if q.get_module(x, y) else "." for x in range(29)) for y in range(29)], q.get_mask()


def picture(screen: list[str], scale: int, dot: int | None, blur: bool = False) -> np.ndarray:
    """The panel as a camera might see it: each LED `scale` px, lit LEDs a
    `dot`-px square (None: the whole cell), the rest black."""
    h, w = len(screen), len(screen[0])
    img = np.zeros((h * scale, w * scale), dtype=np.uint8)
    d = dot or scale
    o = (scale - d) // 2
    for y, row in enumerate(screen):
        for x, c in enumerate(row):
            if c == "#":
                img[y * scale + o:y * scale + o + d, x * scale + o:x * scale + o + d] = 235
    # A margin of panel frame around it.
    img = np.pad(img, 6 * scale, constant_values=0)
    if blur:
        k = max(1, scale // 3)
        p = Image.fromarray(img).resize((img.shape[1] // k, img.shape[0] // k), Image.BILINEAR)
        img = np.asarray(p.resize((img.shape[1], img.shape[0]), Image.BILINEAR))
    return img


def decoded(img: np.ndarray) -> list[str]:
    return [r.text for r in zxingcpp.read_barcodes(img) if r.format == zxingcpp.BarcodeFormat.QRCode]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    v = json.loads(VECTORS.read_text())["qr_code"]
    text, masks, screen = v["text"], v["masks"], v["screen"]
    qx, qy = v["qr_x"], v["qr_y"]
    check(len(text.encode()) == 42 and text.startswith("https://opendmd.dev/c#"), f"the link is 42 bytes: {text}")

    # --- blue: another encoder, module for module ---------------------------
    for m in range(8):
        theirs, used = other_rows(text, m)
        check(used == m and theirs == masks[m], f"mask {m}: the core's matrix is qrcodegen's, module for module")
    theirs, auto = other_rows(text, None)
    check(auto == v["mask"], f"the mask the core picks ({v['mask']}) is qrcodegen's choice ({auto})")
    # The screen carries the chosen matrix at one LED a module, dark modules off.
    best = masks[v["mask"]]
    on_screen = all((screen[qy + y][qx + x] == ".") == (best[y][x] == "#") for y in range(29) for x in range(29))
    check(on_screen, "the screen holds the matrix at one LED a module (dark modules off, light ones lit)")
    zone = all(screen[y][x] == "#" for y in range(32) for x in range(qx + 29 + 4) if not (qy <= y < qy + 29 and qx <= x < qx + 29))
    check(zone, "its quiet zone is lit: 1 row above, 2 below, 4 columns either side")

    # --- blue: an independent decoder reads the panel --------------------------
    # Gating: what a camera at a normal distance sees (each LED a solid
    # cell, or LED dots softened by the lens). Information only: LED dots
    # with sharp dark gaps between them, which ZXing's binariser may read
    # as modules of their own (P1-PLAN 10.3: the rig's scan, D6, decides).
    shots = {
        "squares-6px": (picture(screen, 6, None), True),
        "leds-12px-dot8-blurred": (picture(screen, 12, 8, blur=True), True),
        "leds-9px-dot7-blurred": (picture(screen, 9, 7, blur=True), True),
        "leds-10px-dot7-sharp": (picture(screen, 10, 7), False),
        "leds-8px-dot5-sharp": (picture(screen, 8, 5), False),
    }
    for name, (img, gating) in shots.items():
        got = decoded(img)
        if gating:
            check(got == [text], f"ZXing decodes the panel ({name}): {got}")
        else:
            print(f"info  ZXing on the panel ({name}, not gating): {'decoded' if got == [text] else 'not decoded'}")
        if a.out:
            a.out.mkdir(parents=True, exist_ok=True)
            Image.fromarray(img).save(a.out / f"pair-screen-{name}.png")

    # --- red -------------------------------------------------------------------
    one_off = [list(r) for r in masks[v["mask"]]]
    one_off[14][14] = "." if one_off[14][14] == "#" else "#"
    check(["".join(r) for r in one_off] != other_rows(text, v["mask"])[0], "red: one module changed is caught by the comparison")
    check(other_rows(text, (v["mask"] + 1) % 8)[0] != masks[v["mask"]], "red: another mask's matrix does not match")
    rng = random.Random(7)
    spoilt = [list(r) for r in screen]
    cells = [(x, y) for y in range(29) for x in range(29) if not (x < 9 and y < 9) and not (x > 19 and y < 9) and not (x < 9 and y > 19)]
    for x, y in rng.sample(cells, len(cells) // 5):
        c = spoilt[qy + y][qx + x]
        spoilt[qy + y][qx + x] = "." if c == "#" else "#"
    got = decoded(picture(["".join(r) for r in spoilt], 6, None))
    check(text not in got, f"red: a fifth of the modules flipped does not decode to the link: {got}")

    print(f"QR_CHECK {'PASS' if not failed else 'FAIL'} ({failed} failed)")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
