# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build a synthetic card for `make web-smoke` where the real card dump is
absent (CI): nothing third-party, every GIF drawn here.

    uv run tools/ci-card.py OUT_DIR [--no-convert]
    DMD_CARD_ROOT=OUT_DIR make web-smoke

The layout is the real card's (gifs/<Category>/, filelist.txt, .hall, .xmas,
an obsolete filelist.odm.txt, logos/boot.gif, cache/), and the names are the
ones web-smoke looks for: Mario titles among the SNES rows of Consoles,
SNES_Mega_Man_X_08_sephirot68, SNES_Super_Metroid_01..04, a small
SNES_Asterix01, MD_ and SNES_ lines for the playlist builder, Other's
OTHER_DBZ01 (nothing to smooth) and OTHER_SCROLL_Xmen01 (a 4 px a frame
scroll, a Smooth motion candidate), MD_Demons_Of_Asteborg_01_sephirot68,
and enough Consoles rows for the Library list to scroll. Every GIF is
128x32 with a few frames at varied delays, so previews light up and the
mock's clock has real frame delays to run through.

Where web-smoke measures sizes and screens, the card is shaped like the
stock one: Consoles has ~3,400 rows (2,800 of them one-frame zz_row_*
fillers in no playlist), so a 120,000 px fling lands among its GIFs; its
GBA_* rows, the first on screen, have large copies (~60 KB: a blinking
start, then noise) with a small first frame and a small hover clip;
Arcade holds three noise GIFs whose copies are 40-60 KB (the listing
queue's); Computers and Pinball_Short a screenful or more each; Other a
1 px a frame scroll at 50 ms (Smooth motion's "already smooth").

Then `opendmd-pack convert OUT_DIR --verify` makes the prepared copies
under cache/ (skipped with --no-convert). Two Arcade GIFs past 256 colours
(ARCADE_SF2_GameOver01/02, in no playlist) then get the lossy copy and no
.failed line that earlier firmware and earlier opendmd-pack left, as on the
real card dump: web-smoke's damage checks start from that. The output is deterministic;
an existing OUT_DIR is replaced.
"""

import argparse
import os
import random
import shutil
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw

REPO = Path(__file__).resolve().parent.parent
W, H = 128, 32

# Consoles: platform -> titles (each gets numbered variants).
CONSOLES = {
    "SNES": ["Super_Mario_World", "Super_Mario_Kart", "Super_Mario_All_Stars", "Super_Mario_RPG",
             "Donkey_Kong_Country", "Zelda_A_Link_to_the_Past", "F-Zero", "Street_Fighter_II",
             "Chrono_Trigger", "Secret_of_Mana", "Contra_III", "Kirby_Super_Star",
             "Mega_Man_X2", "Mega_Man_X3", "Final_Fight", "Earthworm_Jim"],
    "MD": ["Sonic_the_Hedgehog", "Sonic_2", "Streets_of_Rage_2", "Gunstar_Heroes", "Ecco_the_Dolphin",
           "Shinobi_III", "Golden_Axe", "Castlevania_Bloodlines", "Thunder_Force_IV", "Ristar"],
    "NES": ["Super_Mario_Bros", "Super_Mario_Bros_3", "Metroid", "Mega_Man_2", "Castlevania",
            "Zelda", "Kirby_Adventure", "Duck_Tales"],
    "GBA": ["Mario_Party", "Metroid_Fusion", "Golden_Sun", "Advance_Wars", "Minish_Cap"],
    "PICO-8": ["Celeste", "Jelpi", "Hug_Fighter"],
    "PCE": ["Bonk", "Ys_Book_I_II"],
    "SMS": ["Alex_Kidd", "Wonder_Boy"],
}
MANY = ["ARCADE_SF2_GameOver01", "ARCADE_SF2_GameOver02"]  # past 256 colours, in no playlist
VARIANTS = 12  # _01.._12 of each title: ~570 Consoles rows, so the Library list scrolls well past 4000 px
# Rows like the stock card's (3,781 in Consoles): web-smoke's thumbnail fling
# scrolls 120,000 px and must still land among Consoles' GIFs. Tiny one-frame
# GIFs in no playlist, named to sort last and match no title or platform.
FILLER = 2800
# web-smoke's listing queue holds the stream queue with three prepared
# copies of 40-60 KB in Arcade (in no playlist).
ARCADE_BIG = 3
OTHER = 40
ARCADE = ["Pacman", "Galaga", "Metal_Slug", "Outrun", "Final_Fight", "Bubble_Bobble", "Gauntlet", "Rastan"]
COMPUTERS = ["AMIGA_Shadow_of_the_Beast", "C64_Turrican", "AMIGA_Lemmings", "C64_Giana_Sisters", "MSX_Penguin_Adventure"]
PINBALL = ["Addams_Family", "Twilight_Zone", "Medieval_Madness", "Attack_from_Mars", "Theatre_of_Magic"]
HALLOWEEN = ["HALLOWEEN_MEGADRIVE_McDonald_Treasure_Land", "HALLOWEEN_ARCADE_GoldenAxe16",
             "HALLOWEEN_ARCADE_SCROLL_Darkstalkers_3_01", "HALLOWEEN_MD_Splatterhouse_2",
             "HALLOWEEN_SNES_Zombies_Ate_My_Neighbors_01", "HALLOWEEN_OTHER_Pumpkin_01",
             "HALLOWEEN_OTHER_Pumpkin_02", "HALLOWEEN_NES_Castlevania_II"]
XMAS = ["XMAS_OTHER_SCROLL_UglySweatMario01", "XMAS_AMIGA_MEGADEMOS_Merry_Cmas", "XMAS_OTHER_SantaVsGift_RattenJager",
        "XMAS_SNES_Christmas_Tree_01", "XMAS_MD_Snow_01", "XMAS_OTHER_Snowman_01"]


def palette(rng: random.Random) -> list[tuple[int, int, int]]:
    return [(0, 0, 0)] + [(rng.randrange(80, 256), rng.randrange(40, 256), rng.randrange(40, 256)) for _ in range(7)]


def to_p(img: Image.Image) -> Image.Image:
    return img.convert("P", palette=Image.Palette.ADAPTIVE, colors=16)


def save(frames: list[Image.Image], delays: list[int], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    frames = [to_p(f) for f in frames]
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=delays, loop=0,
                   optimize=False, disposal=1)


def many_colours(n: int = 3) -> tuple[list[Image.Image], list[int]]:
    """Each frame a 200-colour gradient of its own hue, saved with its own
    local colour table: past 256 colours across the GIF."""
    frames = []
    for f in range(n):
        im = Image.new("RGB", (W, H))
        px = im.load()
        for x in range(W):
            for y in range(H):
                i = (x + y * W) % 200
                px[x, y] = ((i + 80 * f) % 256, (40 * f + i // 2) % 256, (255 - i + 60 * f) % 256)
        frames.append(im)
    return frames, [100] * n


def save_many(frames: list[Image.Image], delays: list[int], path: Path, shared: bool = False) -> None:
    """Each frame its own 256-colour table, or (`shared`) one table for all
    of them: the lossy, quantised version."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if shared:
        sheet = Image.new("RGB", (W, H * len(frames)))
        for i, f in enumerate(frames):
            sheet.paste(f, (0, H * i))
        pal = sheet.quantize(colors=256, method=Image.Quantize.MEDIANCUT)
        frames = [f.quantize(palette=pal, dither=Image.Dither.NONE) for f in frames]
    else:
        frames = [f.quantize(colors=256, method=Image.Quantize.MEDIANCUT) for f in frames]
    frames[0].save(path, save_all=True, append_images=frames[1:], duration=delays, loop=0,
                   optimize=False, disposal=1)


def blinking(rng: random.Random, n: int) -> tuple[list[Image.Image], list[int]]:
    """Shapes that appear and change colour in place: nothing moves, so
    nothing to smooth."""
    pal = palette(rng)
    boxes = [(rng.randrange(0, W - 20), rng.randrange(0, H - 10), rng.randrange(8, 20), rng.randrange(6, 12))
             for _ in range(6)]
    frames = []
    for i in range(n):
        im = Image.new("RGB", (W, H), pal[0])
        d = ImageDraw.Draw(im)
        for j, (x, y, w, h) in enumerate(boxes):
            d.rectangle([x, y, x + w, y + h], fill=pal[1 + (i + j) % 7])
        d.text((4 + (i % 3) * 30, 10), "ODMD", fill=pal[1 + i % 7])
        frames.append(im)
    delay = rng.choice([40, 60, 80, 100, 120])
    return frames, [delay + (20 if i % 3 == 0 else 0) for i in range(n)]


def scrolling(rng: random.Random, n: int, step: int) -> tuple[list[Image.Image], list[int]]:
    """A wide strip panned `step` px a frame: whole-frame motion, a Smooth
    motion candidate when step >= 2."""
    pal = palette(rng)
    strip_w = W + step * n
    strip = Image.new("RGB", (strip_w, H), pal[0])
    d = ImageDraw.Draw(strip)
    x = 0
    while x < strip_w:
        w = rng.randrange(6, 18)
        h = rng.randrange(8, 28)
        d.rectangle([x, H - h, x + w, H - 1], fill=pal[1 + rng.randrange(7)])
        x += w + rng.randrange(2, 10)
    frames = [strip.crop((i * step, 0, i * step + W, H)) for i in range(n)]
    return frames, [rng.choice([40, 50, 60])] * n


def noisy(rng: random.Random, n: int, first: tuple[list[Image.Image], list[int]] | None = None) -> tuple[list[Image.Image], list[int]]:
    """`n` frames of random pixels in 16 colours: nothing for LZW or a delta
    frame to save, so a prepared copy is about 3 KB a frame. After `first`
    (its frames and delays) if given: a GIF whose first frame is small and
    whose whole copy is large (web-smoke's thumbnails read only the first)."""
    pal = palette(rng) + [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(8)]
    frames, delays = first or ([], [])
    frames, delays = list(frames), list(delays)
    for _ in range(n):
        im = Image.new("RGB", (W, H))
        im.putdata([pal[rng.randrange(16)] for _ in range(W * H)])
        frames.append(im)
        delays.append(100)
    return frames, delays


def make_gif(path: Path, seed: str, kind: str = "blink", n: int = 0) -> None:
    rng = random.Random(seed)
    if kind == "noisy":
        frames, delays = noisy(rng, n or 16)
    elif kind == "heavy":
        # A blinking GIF, then noise: a small first frame and a small clip
        # (the web app's hover clip is the first 24 frames, at most 32 KB),
        # but a large whole, as the stock card's copies are.
        frames, delays = noisy(rng, n or 14, blinking(rng, 26))
    elif kind == "filler":
        frames, delays = [Image.new("RGB", (W, H))], [100]
    elif kind == "scroll1":  # 1 px a frame at 50 ms: moves, nothing to smooth
        frames, _ = scrolling(rng, n or 60, 1)
        delays = [50] * len(frames)
    elif kind == "scroll":
        frames, delays = scrolling(rng, n or 40, 4)
    elif kind == "small":
        frames, delays = blinking(rng, n or 3)
    else:
        frames, delays = blinking(rng, n or rng.randrange(4, 9))
    save(frames, delays, path)


def build(out: Path) -> None:
    if out.exists():
        shutil.rmtree(out)
    gifs = out / "gifs"
    listed: list[str] = []

    def add(cat: str, name: str, kind: str = "blink", n: int = 0, in_list: bool = True) -> str:
        p = f"/gifs/{cat}/{name}.gif"
        make_gif(gifs / cat / f"{name}.gif", p, kind, n)
        if in_list:
            listed.append(p)
        return p

    for c in ("Spectrum_Slide", "Pixel_Art"):
        for i in range(1, 4):
            add(c, f"{c.upper()}_{i:02d}")
    for plat, titles in CONSOLES.items():
        for t in titles:
            for v in range(1, VARIANTS + 1):
                # GBA's sort first in Consoles: the rows web-smoke's
                # thumbnails see, so their copies are large (a first frame,
                # then noise), as the stock card's are.
                kind = "scroll" if "SCROLL" in t else "heavy" if plat == "GBA" else "blink"
                add("Consoles", f"{plat}_{t}_{v:02d}", kind)
    for i in range(1, 9):
        add("Consoles", f"SNES_Mega_Man_X_{i:02d}" + ("_sephirot68" if i >= 5 else ""))
    for i in range(1, 5):
        add("Consoles", f"SNES_Super_Metroid_{i:02d}")
    add("Consoles", "SNES_Asterix01", "small")
    add("Consoles", "SNES_Asterix02")
    add("Consoles", "SNES_SCROLL_Mega_Man_X", "scroll")
    add("Consoles", "MD_Demons_Of_Asteborg_01_sephirot68", "scroll", 120)
    for i in range(1, OTHER + 1):
        add("Other", f"OTHER_{i:03d}")
    add("Other", "OTHER_DBZ01", "blink", 12)
    add("Other", "OTHER_SCROLL_Xmen01", "scroll", 48)
    # web-smoke's Smooth motion "already smooth" case (SMOOTH_STEADY).
    add("Other", "OTHER_SCROLL_The_Oluk_Squid_Game", "scroll1")
    for t in ARCADE:
        add("Arcade", f"ARCADE_{t}_01")
        add("Arcade", f"ARCADE_{t}_02")
    for i in range(1, ARCADE_BIG + 1):
        add("Arcade", f"ARCADE_Noise_{i:02d}", "noisy", in_list=False)
    for i in range(1, FILLER + 1):
        add("Consoles", f"zz_row_{i:04d}", "filler", in_list=False)
    # A screenful (10+ rows) in Computers, and several in Pinball_Short:
    # web-smoke's thumbnail load checks draw a whole screen of each, the
    # second after scrolling 1,440 px into it.
    for t in COMPUTERS:
        add("Computers", t)
        for v in (2, 3):
            add("Computers", f"{t}_{v:02d}")
    for t in PINBALL:
        add("Pinball_Short", f"PINBALL_{t}")
        for v in range(2, 13):
            add("Pinball_Short", f"PINBALL_{t}_{v:02d}")
    add("logos", "RePlay")
    add("logos", "RGB-Pi")
    hall = [add("Halloween", n, in_list=False) for n in HALLOWEEN]
    xmas = [add("XMAS", n, "scroll" if "SCROLL" in n else "blink", in_list=False) for n in XMAS]

    for name in MANY:
        save_many(*many_colours(), gifs / "Arcade" / f"{name}.gif")

    # One of the user's Formulas, as the web app saves it (the emulator's
    # psi-probe plays it: `refs`); comments and indentation, as written.
    (out / "formulas").mkdir(parents=True, exist_ok=True)
    (out / "formulas" / "ci-test.psi").write_text(
        'name "CI test"\n'
        'control k = 0.5\n'
        '-- a ramp, its brightness the control\n'
        'v = frac(x + t / 4)   -- moving\n'
        '  rgb(v * k, 0.2, 1 - v)\n'
    )
    # A boot logo at the card root, as on the real card.
    make_gif(out / "logos" / "boot.gif", "boot")
    (out / "filelist.txt").write_text("".join(p + "\n" for p in listed))
    (out / "filelist.hall").write_text("".join(p + "\n" for p in hall))
    (out / "filelist.xmas").write_text("".join(p + "\n" for p in xmas))
    # The retired pre-rendered format's playlist: ignored, never listed.
    (out / "filelist.odm.txt").write_text("/odm/Consoles/00/SNES_Asterix01.odm\n")
    print(f"{out}: {sum(1 for _ in gifs.rglob('*.gif'))} GIFs, {len(listed)} in filelist.txt")


def convert(root: Path) -> None:
    subprocess.run(["cargo", "run", "--release", "--quiet", "--", "convert", str(root), "--verify"],
                   cwd=REPO / "opendmd-pack", check=True, env={**os.environ, "RUSTUP_TOOLCHAIN": "stable"})


def old_lossy_copies(out: Path) -> None:
    """Give the GIFs past 256 colours what earlier converters left: a copy
    of a 256-colour quantised version, and no .failed line. The copies
    come from convert itself, run over the quantised GIFs in a scratch tree."""
    tmp = out.parent / (out.name + ".lossy")
    if tmp.exists():
        shutil.rmtree(tmp)
    for name in MANY:
        save_many(*many_colours(), tmp / "gifs" / "Arcade" / f"{name}.gif", shared=True)
    convert(tmp)
    for src in (tmp / "cache").rglob("*.gif"):
        dst = out / src.relative_to(tmp)
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dst)
    shutil.rmtree(tmp)
    failed = out / "cache" / ".failed"
    if failed.exists():
        keep = [l for l in failed.read_text().splitlines(keepends=True)
                if not any(f"/{n}.gif\t" in l for n in MANY)]
        if keep:
            failed.write_text("".join(keep))
        else:
            failed.unlink()


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", type=Path)
    ap.add_argument("--no-convert", action="store_true", help="skip opendmd-pack convert (no cache/)")
    a = ap.parse_args()
    out = a.out.resolve()
    build(out)
    if not a.no_convert:
        convert(out)
        old_lossy_copies(out)


if __name__ == "__main__":
    main()
