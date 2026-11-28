# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow==11.3.0"]
# ///
"""The animated WebP fixtures of opendmd-art's WebP reader (src/webp.rs).

    uv run opendmd-art/testdata/webp/make.py   # rewrites the files beside it

Every frame is drawn here, encoded on its own by libwebp (through Pillow),
and put into an animation by this script's own muxer, so each frame's
offset, size, duration, blending and disposal are exactly as written below
(an encoder would choose its own). What a browser shows is then worked out
here independently of the Rust reader, straight from the WebP container
spec: a transparent canvas, the previous frame's rectangle cleared when it
said "dispose", each frame alpha-blended (non-premultiplied "over", in
exact fractions, rounded once, half up) or copied, every pixel of each
frame as libwebp decodes it.

libwebp's own animation decoder (and so Chrome, and Pillow) blends in 8-bit
fixed point instead: where a see-through pixel is blended over another see-
through one it can differ from the exact result by a few levels (up to 4
in a channel seen on a 16 x 16 gradient of every alpha; this script prints
the figure for its own frames, 1 on black). Over an opaque pixel, or onto
a clear one, the two agree.

anim.webp     40 x 24, plays 3 times, 5 frames: a full frame with alpha;
              a blended lossless frame at (6, 4) that is disposed; an
              unblended lossless one at (20, 10) whose holes punch through;
              a blended lossy one with an ALPH chunk, 0 ms, disposed; an
              opaque lossy one (no alpha) at (24, 0); a blended lossless
              one at (20, 6) of every alpha over frame 0's half-alpha band
              (see-through over see-through).
anim.json     its canvas, plays and each frame's raw duration (ms).
anim.rgba     the composited canvas after each frame, RGBA, frame after frame.
"""
import io
import json
import math
from fractions import Fraction
import struct
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
W, H, PLAYS = 40, 24, 3


def chunks(b: bytes) -> list[tuple[bytes, bytes]]:
    assert b[:4] == b"RIFF" and b[8:12] == b"WEBP"
    out, i = [], 12
    while i + 8 <= len(b):
        n = struct.unpack("<I", b[i + 4:i + 8])[0]
        out.append((b[i:i + 4], b[i + 8:i + 8 + n]))
        i += 8 + n + (n & 1)
    return out


def chunk(tag: bytes, data: bytes) -> bytes:
    return tag + struct.pack("<I", len(data)) + data + (b"\0" if len(data) & 1 else b"")


def u24(v: int) -> bytes:
    return struct.pack("<I", v)[:3]


def encode(img: Image.Image, lossless: bool) -> tuple[bytes, Image.Image]:
    """The frame's bitstream chunks (ALPH + VP8, or VP8, or VP8L) and the frame as libwebp decodes them."""
    buf = io.BytesIO()
    if lossless:
        img.save(buf, "WEBP", lossless=True, exact=True)
    else:
        img.save(buf, "WEBP", quality=60, method=4, alpha_quality=100)
    b = buf.getvalue()
    body = b"".join(chunk(t, d) for t, d in chunks(b) if t in (b"ALPH", b"VP8 ", b"VP8L"))
    return body, Image.open(io.BytesIO(b)).convert("RGBA")


def draw(w: int, h: int, f) -> Image.Image:
    img = Image.new("RGBA", (w, h))
    img.putdata([f(x, y) for y in range(h) for x in range(w)])
    return img


# (x, y, image, lossless, duration ms, blend, dispose)
FRAMES = [
    # A full frame: an opaque gradient on the left, see-through on the right, a half-alpha band.
    (0, 0, draw(W, H, lambda x, y: (x * 6, y * 10, 200, 255) if x < 20 else ((255, 0, 0, 128) if 8 <= y < 14 else (0, 0, 0, 0))), True, 100, False, False),
    # Blended over it: opaque, see-through and 96-alpha pixels; disposed after.
    (6, 4, draw(12, 8, lambda x, y: (0, 255, 0, 255) if x < 4 else ((0, 0, 255, 96) if x < 8 else (255, 255, 255, 0))), True, 250, True, True),
    # Copied (no blending): its see-through pixels punch holes in the canvas.
    (20, 10, draw(10, 10, lambda x, y: (250, 200, 0, 255) if (x + y) % 3 else (0, 0, 0, 0)), True, 40, False, False),
    # Lossy with an alpha chunk, blended; 0 ms; disposed after.
    (2, 8, draw(16, 12, lambda x, y: (40, 120, 255, 255) if y < 6 else (255, 80, 40, 160)), False, 0, True, True),
    # Lossy, opaque (no alpha chunk), blended (the same as copied).
    (24, 0, draw(16, 8, lambda x, y: (200, 30, 160, 255)), False, 1000, True, False),
    # Blended, lossless, alpha 0..255 in steps over frame 0's 128-alpha band and its clear pixels.
    (20, 6, draw(16, 10, lambda x, y: ((x * 16) % 256, 255 - y * 25, 90, min(255, 17 * x + 3 * y))), True, 70, True, False),
]


def mux() -> tuple[bytes, list[Image.Image]]:
    body = chunk(b"VP8X", bytes([0x12, 0, 0, 0]) + u24(W - 1) + u24(H - 1))  # animation + alpha
    body += chunk(b"ANIM", bytes([255, 255, 255, 255]) + struct.pack("<H", PLAYS))  # (a white background: browsers ignore it)
    decoded = []
    for x, y, img, lossless, ms, blend, dispose in FRAMES:
        assert x % 2 == 0 and y % 2 == 0
        data, dec = encode(img, lossless)
        decoded.append(dec)
        flags = (0 if blend else 2) | (1 if dispose else 0)
        body += chunk(b"ANMF", u24(x // 2) + u24(y // 2) + u24(img.width - 1) + u24(img.height - 1) + u24(ms) + bytes([flags]) + data)
    return b"RIFF" + struct.pack("<I", 4 + len(body)) + b"WEBP" + body, decoded


def half_up(v: Fraction) -> int:
    return math.floor(v + Fraction(1, 2))


def over(s: tuple, d: tuple) -> tuple:
    """Non-premultiplied "over", in exact fractions, rounded once (half up)."""
    sa, da = Fraction(s[3], 255), Fraction(d[3], 255)
    a = sa + da * (1 - sa)
    if a == 0:
        return (0, 0, 0, 0)
    c = [(s[i] * sa + d[i] * da * (1 - sa)) / a for i in range(3)]
    return tuple(half_up(v) for v in c) + (half_up(a * 255),)


def composite(decoded: list[Image.Image]) -> list[bytes]:
    canvas = [(0, 0, 0, 0)] * (W * H)
    out, prev = [], None
    for (x0, y0, img, _, _, blend, dispose), dec in zip(FRAMES, decoded):
        if prev:
            px, py, pw, ph = prev
            for y in range(py, py + ph):
                for x in range(px, px + pw):
                    canvas[y * W + x] = (0, 0, 0, 0)
        src = list(dec.getdata())
        for y in range(img.height):
            for x in range(img.width):
                s, i = src[y * img.width + x], (y0 + y) * W + x0 + x
                canvas[i] = over(s, canvas[i]) if blend else s
        out.append(bytes(v for p in canvas for v in p))
        prev = (x0, y0, img.width, img.height) if dispose else None
    return out


def main() -> None:
    webp, decoded = mux()
    (HERE / "anim.webp").write_bytes(webp)
    (HERE / "anim.rgba").write_bytes(b"".join(composite(decoded)))
    lossy = [i for i, f in enumerate(FRAMES) if not f[3]]
    # How far libwebp's 8-bit blending is from the exact result, on black (lossless frames).
    im, n, exp = Image.open(io.BytesIO(webp)), W * H * 4, b"".join(composite(decoded))
    worst = 0
    for i in range(len(FRAMES)):
        im.seek(i)
        f, e = im.convert("RGBA").tobytes(), exp[i * n:(i + 1) * n]
        if i not in lossy:
            worst = max(worst, max(abs(f[j] * f[j - j % 4 + 3] // 255 - e[j] * e[j - j % 4 + 3] // 255) for j in range(n) if j % 4 != 3))
    print(f"libwebp's animation decoder differs from the exact blend by {worst} at most (on black)")
    (HERE / "anim.json").write_text(json.dumps({"width": W, "height": H, "plays": PLAYS, "ms": [f[4] for f in FRAMES], "lossy": lossy}) + "\n")
    print(f"anim.webp {len(webp)} B, {len(FRAMES)} frames")


main()
