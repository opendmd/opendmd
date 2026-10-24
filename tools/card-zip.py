# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""A small, synthetic card zip for testing "Set up a card" (/card/), built
the way the DMDoS pack's zip is — every entry deflated, a data descriptor
after each (general-purpose flag bit 3, so the local headers carry zero
sizes and CRCs and only the central directory has them), no zip64, no
comment, the card at the zip's root — and the same tree unzipped, for
`opendmd-pack convert`. Nothing third-party: every GIF is drawn here.

    uv run tools/card-zip.py OUT_DIR

writes OUT_DIR/card.zip and OUT_DIR/tree/ (and replaces an existing
OUT_DIR). The zip holds, under gifs/ in four categories: ordinary
animations, a 2-colour one, one of exactly 256 colours, one past 256
colours (no lighter version: it plays as it is), one with `Previous`
disposal, a larger one (~100 frames), a cut-off one and one that is not
a GIF at all past its header; a `.gif.mp4` stray; filelist.txt (with a
line naming a GIF the zip doesn't have, and one GIF in no list),
filelist.hall, filelist.xmas, logos/boot.gif, and the files the page
leaves off the card: a dummy `DMDos GUI.exe`, `icono.ico` and `scan.py`
(stored, not deflated).
"""

import argparse
import io
import random
import shutil
import zipfile
from pathlib import Path

from PIL import Image, ImageDraw

W, H = 128, 32
DATE = (2024, 1, 2, 3, 4, 6)


def blink(rng: random.Random, n: int, colours: int = 8) -> list[Image.Image]:
    pal = [(0, 0, 0)] + [(rng.randrange(60, 256), rng.randrange(40, 256), rng.randrange(40, 256)) for _ in range(colours - 1)]
    boxes = [(rng.randrange(0, W - 20), rng.randrange(0, H - 10), rng.randrange(6, 20), rng.randrange(4, 12)) for _ in range(6)]
    frames = []
    for i in range(n):
        im = Image.new("RGB", (W, H), pal[0])
        d = ImageDraw.Draw(im)
        for j, (x, y, w, h) in enumerate(boxes):
            dx = (i * (j + 1)) % 9
            d.rectangle([x + dx, y, x + w + dx, y + h], fill=pal[1 + (i + j) % (colours - 1)])
        frames.append(im)
    return frames


def gif(frames: list[Image.Image], delay=60, **kw) -> bytes:
    b = io.BytesIO()
    frames[0].save(b, "GIF", save_all=True, append_images=frames[1:], duration=delay, loop=0, optimize=False, **kw)
    return b.getvalue()


def two_colours() -> bytes:
    frames = []
    for i in range(4):
        im = Image.new("1", (W, H), 0)
        ImageDraw.Draw(im).rectangle([i * 20, 4, i * 20 + 30, 27], fill=1)
        frames.append(im.convert("P"))
    return gif(frames, 100)


def full_palette() -> bytes:
    """One frame with exactly 256 colours (every pixel of a 16x16 grid of tiles)."""
    im = Image.new("P", (W, H))
    im.putpalette([v for i in range(256) for v in (i, (i * 7) % 256, 255 - i)])
    px = im.load()
    for y in range(H):
        for x in range(W):
            px[x, y] = (x // 8) + 16 * (y // 2)
    second = im.copy()
    ImageDraw.Draw(second).rectangle([10, 10, 40, 20], fill=3)
    return gif([im, second], 200)


def too_many() -> bytes:
    """Two frames, each with its own 200-colour palette: 400 colours on the
    canvas, so the lighter version could only be lossy."""
    frames = []
    for k in range(2):
        im = Image.new("P", (W, H))
        im.putpalette([v for i in range(256) for v in ((i * 5 + k * 101) % 256, (i * 3 + k * 37) % 256, (i + k * 128) % 256)])
        px = im.load()
        for y in range(H):
            for x in range(W):
                px[x, y] = (x + y * W + k) % 200
        frames.append(im)
    # The second frame only half covers the first: both palettes stay on the canvas.
    frames[1] = frames[1].crop((0, 0, W // 2, H))
    b = io.BytesIO()
    frames[0].save(b, "GIF", save_all=True, append_images=[frames[1]], duration=150, loop=0, optimize=False, disposal=1)
    return b.getvalue()


def previous() -> bytes:
    rng = random.Random("prev")
    frames = blink(rng, 6)
    return gif(frames, 80, disposal=[1, 3, 1, 3, 2, 1])


def scroll(rng: random.Random, n: int, step: int) -> list[Image.Image]:
    strip = Image.new("RGB", (W + step * n, H))
    d = ImageDraw.Draw(strip)
    x = 0
    while x < strip.width:
        w = rng.randrange(4, 14)
        d.rectangle([x, rng.randrange(0, 20), x + w, H - 1], fill=(rng.randrange(256), rng.randrange(256), rng.randrange(256)))
        x += w + rng.randrange(1, 6)
    return [strip.crop((i * step, 0, i * step + W, H)) for i in range(n)]


def build(out: Path) -> None:
    if out.exists():
        shutil.rmtree(out)
    tree = out / "tree"
    files: dict[str, bytes] = {}
    listed, hall, xmas = [], [], []

    def put(path: str, data: bytes, into: list | None = None) -> None:
        files[path] = data
        if into is not None:
            into.append("/" + path)

    for i, name in enumerate(["SNES_Super_Metroid_01", "SNES_Super_Metroid_02", "MD_Sonic_The_Hedgehog_01",
                              "NES_Duck_Hunt_01", "GBA_Golden_Sun_01", "N64_Wave_Race_01"]):
        put(f"gifs/Consoles/{name}.gif", gif(blink(random.Random(name), 4 + i % 4), 40 + 20 * (i % 3)), listed)
    put("gifs/Consoles/SNES_SCROLL_Long_Road_01.gif", gif(scroll(random.Random("road"), 100, 3), 40), listed)
    put("gifs/Consoles/MD_Unlisted_01.gif", gif(blink(random.Random("unlisted"), 3)))  # in no playlist
    put("gifs/Arcade/ARCADE_Two_Colours.gif", two_colours(), listed)
    put("gifs/Arcade/ARCADE_Full_Palette.gif", full_palette(), listed)
    put("gifs/Arcade/ARCADE_Too_Many_Colours.gif", too_many(), listed)
    put("gifs/Arcade/ARCADE_Previous_01.gif", previous(), listed)
    for name in ["ARCADE_Galaxy_01", "ARCADE_Galaxy_02", "ARCADE_Bricks_01"]:
        put(f"gifs/Arcade/{name}.gif", gif(blink(random.Random(name), 5)), listed)
    whole = gif(blink(random.Random("cut"), 8))
    put("gifs/Pinball_Short/PINBALL_Cut_Short.gif", whole[: len(whole) // 2], listed)
    put("gifs/Pinball_Short/PINBALL_Not_A_GIF.gif", b"GIF89a" + bytes(random.Random("junk").randrange(256) for _ in range(300)), listed)
    for name in ["PINBALL_Ball_01", "PINBALL_Flipper_01", "PINBALL_Tilt_01"]:
        put(f"gifs/Pinball_Short/{name}.gif", gif(blink(random.Random(name), 4)), listed)
    for name in ["SPECTRUM_Manic_Miner_1", "SPECTRUM_Manic_Miner_2"]:
        put(f"gifs/Spectrum/{name}.gif", gif(blink(random.Random(name), 3)), listed)
    put("gifs/Spectrum/SPECTRUM_Manic_Miner_3.gif.mp4", b"\x00\x00\x00\x18ftypmp42" + bytes(64))
    for name in ["HALLOWEEN_OTHER_Pumpkin_01", "HALLOWEEN_OTHER_Bats_01"]:
        put(f"gifs/Halloween/{name}.gif", gif(blink(random.Random(name), 4)), hall)
    for name in ["XMAS_OTHER_Snowman_01", "XMAS_OTHER_Tree_01"]:
        put(f"gifs/XMAS/{name}.gif", gif(blink(random.Random(name), 4)), xmas)
    listed.insert(3, "/gifs/Consoles/SNES_Not_In_The_Zip_01.gif")  # a line naming a GIF the zip doesn't have
    put("logos/boot.gif", gif(blink(random.Random("boot"), 6)))
    put("filelist.txt", "".join(p + "\n" for p in listed).encode())
    put("filelist.hall", "".join(p + "\n" for p in hall).encode())
    put("filelist.xmas", "".join(p + "\n" for p in xmas).encode())
    put("DMDos GUI.exe", b"MZ" + bytes(random.Random("exe").randrange(256) for _ in range(5000)))
    put("icono.ico", b"\x00\x00\x01\x00" + bytes(200))
    put("scan.py", b"# not part of the card\nprint('scan')\n")

    for path, data in files.items():
        (tree / path).parent.mkdir(parents=True, exist_ok=True)
        (tree / path).write_bytes(data)

    class Unseekable(io.RawIOBase):
        """A write-only stream zipfile can't seek in, so it writes a data
        descriptor after every entry, as the pack's zip has."""
        def __init__(self, f):
            self.f, self.n = f, 0
        def writable(self):
            return True
        def write(self, b):
            self.n += len(b)
            return self.f.write(b)
        def tell(self):
            return self.n

    out.mkdir(parents=True, exist_ok=True)
    with open(out / "card.zip", "wb") as raw, zipfile.ZipFile(Unseekable(raw), "w") as z:
        z.writestr(zipfile.ZipInfo("gifs/", DATE), b"")
        for path, data in files.items():
            info = zipfile.ZipInfo(path, DATE)
            info.compress_type = zipfile.ZIP_STORED if path == "scan.py" else zipfile.ZIP_DEFLATED
            z.writestr(info, data)
    with zipfile.ZipFile(out / "card.zip") as z:
        assert all(i.flag_bits & 8 for i in z.infolist()), "every entry has a data descriptor"
        assert z.testzip() is None
    n = sum(1 for p in files if p.startswith("gifs/") and p.endswith(".gif"))
    print(f"{out}: card.zip ({(out / 'card.zip').stat().st_size} B), {n} GIFs, and tree/")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", type=Path)
    build(ap.parse_args().out.resolve())


if __name__ == "__main__":
    main()
