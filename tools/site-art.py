# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the public site's artwork: nothing third-party, all of it ours or CC0.

    uv run tools/site-art.py OUT_DIR [--sim PATH]

- strip/*.gif: the animations for the strip across the top of every page
  and the landing page's tiles: the panel's own effects, the clock and
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

# name, source (a live item, or kenney:FILE), ms a frame, colours,
# where it shows (strip, tiles), alt text, caption.
ART = [
    ("press-start", "@text?msg=PRESS%20START&speed=20&color=ff3366", 40, 16, "strip tiles",
     "PRESS START scrolling across the panel in pink", "Scrolling text"),
    ("tunnel-fire", "@fx:tunnel?palette=fire&texture=rings&twist=2&speed=2", 80, 32, "strip tiles",
     "A twisting tunnel of fiery rings", "Tunnel, fire palette"),
    ("kenney-walk", "kenney:b_walk.gif", 0, 32, "strip tiles",
     "Pixel-art characters walking across the panel", "Kenney Pixel Platformer sprites (CC0)"),
    ("wire-torus", "@fx:wire?shape=torus&color=00ccff&speed=3", 40, 24, "strip tiles",
     "A spinning wireframe torus in cyan", "Wireframe torus"),
    ("insert-coin", "@text?msg=INSERT%20COIN&speed=20&color=ffaa00", 40, 16, "strip",
     "INSERT COIN scrolling across the panel in amber", "Scrolling text"),
    ("starfield", "@fx:starfield?palette=chrono&density=3&speed=3", 40, 32, "strip tiles",
     "Stars flying towards the viewer", "Starfield"),
    ("clock", "@clock?style=big&date=1&color=66ff99", 0, 8, "strip tiles",
     "The panel's big clock with the date underneath", "The clock"),
    ("wire-tesseract", "@fx:wire?shape=tesseract&color=ff66cc&speed=3", 40, 24, "strip",
     "A rotating wireframe tesseract in pink", "Wireframe tesseract"),
    ("game-over", "@card:GAME OVER|thanks for playing", 0, 8, "strip",
     "A title card reading GAME OVER, thanks for playing", "Title card"),
    ("rotozoom-floor", "@fx:rotozoom?floor=1&palette=chrono&cycle=1&speed=2", 80, 32, "tiles",
     "A Mode 7 floor sweeping past under a colour-cycling sky", "Mode 7 rotozoom"),
    ("tunnel-ice", "@fx:tunnel?palette=ice&texture=checker&layers=2&speed=2", 80, 32, "tiles",
     "A checkered tunnel in icy blues", "Tunnel, ice palette"),
    ("wire-morph", "@fx:wire?shape=morph&speed=3", 40, 24, "tiles",
     "A wireframe shape morphing between solids", "Wireframe morph"),
]
WORDMARK = "@text?msg=OpenDMD&scroll=0&color=ffaa00"
MAX_FRAMES = 240  # a recording stops here if it has not looped
RECIPE = 1  # bump to redo every recording after changing how they are made


def shot(sim: Path, card: Path, play: str, ms: int, png: Path) -> Image.Image:
    subprocess.run([sim, "--root", card, "--play", play, "--screenshot-canvas", png, "--screenshot-at", str(ms)],
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
    # The simulator wants a card root with a filelist, even to --play.
    card = out / "card"
    card.mkdir(exist_ok=True)
    (card / "filelist.txt").write_text("")

    items, keep = [], set()
    for name, src, step, colours, where, alt, caption in ART:
        tag = hashlib.sha1(f"{RECIPE}|{src}|{step}|{colours}".encode()).hexdigest()[:8]
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
