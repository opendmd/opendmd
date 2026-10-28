# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the public site's artwork: nothing third-party, all of it ours or CC0.

    uv run tools/site-art.py OUT_DIR [--sim PATH]

- strip/*.gif: the animations for the strip across the top of every page
  and the landing page's tiles: Formulas (opendmd-web/formulas/*.psi), the clock and
  scrolling text, recorded from the simulator's raw canvas
  (--screenshot-canvas) a frame at a time, one whole loop each; and the
  Kenney "Pixel Platformer" (CC0) sprites the demo card has too, re-encoded
  lighter.
- strip/*.png: a still of each, which the site shows instead when the
  visitor asks for reduced motion.
- wordmark.png: "OpenDMD" in the panel's own font, cropped.
- art.json: what is where, with alt text and captions, for tools/site.py.

Recordings already in OUT_DIR are kept while their recipe is unchanged
(the file name carries a hash of it): delete the directory to redo them.
"""

import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageSequence

ROOT = Path(__file__).resolve().parent.parent
KENNEY = ROOT / "opendmd-web/experiments/art/process/out"

# OpenDMD's Formulas, one file each: a header of comments (author, licence,
# `-- bundle`), then the formula, whose `name "…"` line names it.
FORMULAS = ROOT / "opendmd-web/formulas"

# name, source (a live item, formula:NAME[?--formula-opts] -- an example in
# opendmd-web/formulas/ --, or kenney:FILE), ms a frame, colours, where it shows
# (strip, tiles), alt text, caption.
ART = [
    ("press-start", "@text?msg=PRESS%20START&speed=20&color=ff3366", 40, 16, "strip tiles",
     "PRESS START scrolling across the panel in pink", "Scrolling text"),
    ("tunnel", "formula:Tunnel", 80, 32, "strip tiles",
     "A checkered tunnel turning and cycling colour", "Tunnel (a Formula)"),
    ("kenney-walk", "kenney:b_walk.gif", 0, 32, "strip tiles",
     "Pixel-art characters walking across the panel", "Kenney Pixel Platformer sprites (CC0)"),
    ("wire-cube", "formula:Wireframe cube", 40, 24, "strip tiles",
     "A turning wireframe cube", "Wireframe cube (a Formula)"),
    ("insert-coin", "@text?msg=INSERT%20COIN&speed=20&color=ffaa00", 40, 16, "strip",
     "INSERT COIN scrolling across the panel in amber", "Scrolling text"),
    ("starfield", "formula:Starfield", 40, 32, "strip tiles",
     "Stars flying towards the viewer", "Starfield (a Formula)"),
    ("clock", "@clock?style=big&date=1&color=66ff99", 0, 8, "strip tiles",
     "The panel's big clock with the date underneath", "The clock"),
    ("time-gate", "formula:Time gate", 80, 32, "strip",
     "Star rings rushing out of a time gate", "Time gate (a Formula)"),
    ("game-over", "@card:GAME OVER|thanks for playing", 0, 8, "strip",
     "A title card reading GAME OVER, thanks for playing", "Title card"),
    ("rotozoom-floor", "formula:Rotozoom floor", 80, 32, "tiles",
     "A Mode 7 floor running away to a foggy horizon", "Mode 7 rotozoom floor (a Formula)"),
    ("kaleidoscope", "formula:Kaleidoscope", 80, 32, "tiles",
     "Plasma folded into a mirror pattern", "Kaleidoscope (a Formula)"),
    ("plasma", "formula:Plasma", 40, 24, "tiles",
     "Three waves making every colour of a plasma", "Plasma (a Formula)"),
]
WORDMARK = "@text?msg=OpenDMD&scroll=0&color=ffaa00"
MAX_FRAMES = 240  # a recording stops here if it has not looped
RECIPE = 2  # bump to redo every recording after changing how they are made


def formula_src(name: str) -> str:
    """The source of the example Formula called `name` (without the file's
    header); fails if there is none."""
    for p in sorted(FORMULAS.glob("*.psi")):
        lines = p.read_text(encoding="utf-8").split("\n")
        while lines and lines[0].startswith(("-- author:", "-- licence:", "-- license:", "-- bundle")):
            lines.pop(0)
        if f'name "{name}"' in lines:
            return "\n".join(lines)
    raise SystemExit(f"site-art: no Formula named {name!r} in {FORMULAS}")


def play_args(play: str, tmp: Path) -> list:
    """The simulator's arguments for a source: --play for a live item, or
    --formula (the example's source in a temp .psi file) for formula:NAME[?opts]."""
    if not play.startswith("formula:"):
        return ["--play", play]
    name, _, opts = play.removeprefix("formula:").partition("?")
    psi = tmp / "f.psi"
    psi.write_text(formula_src(name))
    return ["--formula", psi] + (["--formula-opts", opts] if opts else [])


def shot(sim: Path, card: Path, play: str, ms: int, png: Path) -> Image.Image:
    subprocess.run([sim, "--root", card, *play_args(play, png.parent), "--screenshot-canvas", png, "--screenshot-at", str(ms)],
                   check=True, capture_output=True)
    return Image.open(png).convert("RGB")


def same(a: Image.Image, b: Image.Image) -> bool:
    return ImageChops.difference(a, b).getbbox() is None


def record(sim: Path, card: Path, play: str, step: int) -> list[Image.Image]:
    """One loop of a live item, `step` ms a frame: frames until they repeat
    from the start (8 in a row, so a blank start is not mistaken for one)."""
    with tempfile.TemporaryDirectory() as tmp:
        png = Path(tmp) / "f.png"
        if step == 0:
            return [shot(sim, card, play, 1000, png)]
        frames: list[Image.Image] = []
        while len(frames) < MAX_FRAMES:
            frames.append(shot(sim, card, play, len(frames) * step, png))
            # Scrolling text starts and ends on a blank panel: one pass is
            # a loop (a pass of an odd number of steps would otherwise take
            # two to repeat exactly).
            if len(frames) > 2 and frames[0].getbbox() is None and frames[-1].getbbox() is None:
                return frames[:-1]
            for p in range(8, len(frames) - 7):
                if all(same(frames[p + k], frames[k]) for k in range(8)):
                    return frames[:p]
        return frames


def save(frames: list[Image.Image], durations: list[int], colours: int, out: Path) -> None:
    # One palette for the whole loop, so no frame shifts colour.
    sheet = Image.new("RGB", (128, 32 * len(frames)))
    for i, f in enumerate(frames):
        sheet.paste(f, (0, 32 * i))
    pal = sheet.quantize(colours, dither=Image.Dither.NONE)
    q = [f.quantize(palette=pal, dither=Image.Dither.NONE) for f in frames]
    if len(q) == 1:
        q[0].save(out)
    else:
        q[0].save(out, save_all=True, append_images=q[1:], duration=durations, loop=0, optimize=True)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--sim", type=Path, default=ROOT / "opendmd-sim/target/release/opendmd-sim")
    a = ap.parse_args()
    out: Path = a.out
    (out / "strip").mkdir(parents=True, exist_ok=True)
    # The simulator wants a card root with a filelist, even to play one thing.
    card = out / "card"
    card.mkdir(exist_ok=True)
    (card / "filelist.txt").write_text("")

    items, keep = [], set()
    for name, src, step, colours, where, alt, caption in ART:
        # A Formula's tag covers its source, so editing the example re-records.
        body = formula_src(src.removeprefix("formula:").partition("?")[0]) if src.startswith("formula:") else ""
        tag = hashlib.sha1(f"{RECIPE}|{src}|{body}|{step}|{colours}".encode()).hexdigest()[:8]
        gif, still = out / f"strip/{name}-{tag}.gif", out / f"strip/{name}-{tag}.png"
        if not gif.exists():
            if src.startswith("kenney:"):
                im = Image.open(KENNEY / src.removeprefix("kenney:"))
                frames, durations = [], []
                for f in ImageSequence.Iterator(im):
                    frames.append(f.convert("RGB"))
                    durations.append(f.info.get("duration", 100))
            else:
                frames = record(a.sim, card, src, step)
                durations = [step] * len(frames)
            # A single frame is a still; the GIF is then just that still.
            save(frames, durations, colours, gif)
            save([frames[len(frames) // 2]], [0], colours, still)
            print(f"{gif.name}: {len(frames)} frames, {gif.stat().st_size:,} B")
        keep |= {gif.name, still.name}
        items.append({"name": name, "gif": f"strip/{gif.name}", "still": f"strip/{still.name}",
                      "where": where.split(), "alt": alt, "caption": caption})
    for old in (out / "strip").iterdir():
        if old.name not in keep:
            old.unlink()

    # The wordmark: the big font, cropped to the lit pixels plus a pixel.
    with tempfile.TemporaryDirectory() as tmp:
        im = shot(a.sim, card, WORDMARK, 1000, Path(tmp) / "w.png")
    x0, y0, x1, y1 = im.getbbox()
    im.crop((x0 - 1, y0 - 1, x1 + 1, y1 + 1)).quantize(8, dither=Image.Dither.NONE).save(out / "wordmark.png")
    w, h = Image.open(out / "wordmark.png").size

    (out / "art.json").write_text(json.dumps({"items": items, "wordmark": {"src": "wordmark.png", "w": w, "h": h}},
                                             indent=1) + "\n")
    total = sum(p.stat().st_size for p in out.rglob("*") if p.suffix in (".gif", ".png"))
    strip = sum((out / i["gif"]).stat().st_size for i in items if "strip" in i["where"])
    print(f"{out}: {len(items)} animations, strip {strip:,} B, all art {total:,} B")


if __name__ == "__main__":
    main()
