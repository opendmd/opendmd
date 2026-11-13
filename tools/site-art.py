# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the public site's artwork: nothing third-party, all of it ours or CC0.

    uv run tools/site-art.py OUT_DIR [--sim PATH]

- strip/*.gif: the animations for the strip across the top of every page:
  Formulas (opendmd-web/formulas/*.psi) and a title card, recorded from the simulator's raw canvas
  (--screenshot-canvas) a frame at a time, one whole loop each; and the
  Kenney "Pixel Platformer" (CC0) sprites the demo card has too, re-encoded
  lighter.
  The home page's "Get started" panels also have the start-up show
  (`@boot`, the firmware's own frames) and a little Kenney level drawn by
  opendmd-web/experiments/art/process/kenney-pixel-platformer/scene.py
  (scene:…).
- strip/*.png: a still of each, which the site shows instead when the
  visitor asks for reduced motion.
- wordmark.png: "OpenDMD" in the panel's own font, cropped (from the
  status card with nothing to say, `@boot?len=card`: the name alone).
- art.json: what is where, with alt text and captions, for tools/site.py.

Recordings already in OUT_DIR are kept while their recipe is unchanged
(the file name carries a hash of it): delete the directory to redo them.
"""

import argparse
import hashlib
import importlib.util
import json
import subprocess
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageSequence

ROOT = Path(__file__).resolve().parent.parent
PROCESS = ROOT / "opendmd-web/experiments/art/process"
KENNEY = PROCESS / "out"

# OpenDMD's Formulas, one file each: a header of comments (author, licence,
# `-- bundle`), then the formula, whose `name "…"` line names it.
FORMULAS = ROOT / "opendmd-web/formulas"

# name, source (a virtual file, formula:NAME[?--formula-opts] -- an example in
# opendmd-web/formulas/ --, kenney:FILE, or scene:SCRIPT: a scene() in a script under
# opendmd-web/experiments/art/process/), ms a frame, colours, where it shows
# (strip, hero, doors: the home page's Get started), alt text, caption.
ART = [
    ("copper-bars", "formula:Copper bars", 40, 24, "strip",
     "Shiny Amiga-style bars bouncing up and down", "Copper bars (a Formula)"),
    ("tunnel", "formula:Tunnel", 80, 32, "strip",
     "A checkered tunnel turning and cycling colour", "Tunnel (a Formula)"),
    ("kenney-walk", "kenney:b_walk.gif", 0, 32, "strip",
     "Pixel-art characters walking across the panel", "Kenney Pixel Platformer sprites (CC0)"),
    ("wire-cube", "formula:Wireframe cube", 40, 24, "strip",
     "A turning wireframe cube", "Wireframe cube (a Formula)"),
    ("starfield", "formula:Starfield", 40, 32, "strip",
     "Stars flying towards the viewer", "Starfield (a Formula)"),
    ("clock", "formula:Big clock", 0, 16, "strip",
     "The time in big rainbow digits, a seconds bar along the bottom", "Big clock (a Formula)"),
    ("time-gate", "formula:Time gate", 80, 32, "strip",
     "Star rings rushing out of a time gate", "Time gate (a Formula)"),
    # The home page's panel: a still only (it never repeats, and the page
    # plays it live: site/pages/home.html).
    ("endless-road", "formula:Endless road", 0, 128, "hero",
     "An arcade road through a synthwave landscape, palms passing", "Endless road (a Formula)"),
    # Get started: Install and Using it.
    ("boot", "@boot?len=full&ip=opendmd.local", 40, 32, "doors",
     "OpenDMD's start-up show: the name lighting up dot by dot", "The start-up show"),
    ("kenney-run", "scene:kenney-pixel-platformer/scene.py", 0, 64, "doors",
     "A little alien running across a grassy level, picking up coins", "Kenney Pixel Platformer tiles (CC0)"),
    ("game-over", "@card:GAME OVER|thanks for playing", 0, 8, "strip",
     "A title card reading GAME OVER, thanks for playing", "Title card"),
]
WORDMARK = "@boot?len=card"
# The warm white the panel lights "Open" in, and how it is drawn here.
PANEL_WHITE, PANEL_WHITE_SHOWN = (255, 214, 150), (236, 235, 231)
# Recordings of something that plays once rather than loops (name: its
# length, then how much longer its last frame is held, in ms): the start-up
# show, whose last line, the address, would otherwise be gone in half a second.
ONCE = {"boot": (5600, 2500)}
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
    """The simulator's arguments for a source: --play for a virtual file, or
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


def scene(script: Path) -> tuple[list[Image.Image], list[int]]:
    """A drawn scene: the script's scene(), its frames and durations."""
    spec = importlib.util.spec_from_file_location(script.stem, script)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.scene()


def same(a: Image.Image, b: Image.Image) -> bool:
    return ImageChops.difference(a, b).getbbox() is None


def record(sim: Path, card: Path, play: str, step: int) -> list[Image.Image]:
    """One loop of a virtual file or a Formula, `step` ms a frame: frames until they repeat
    from the start (8 in a row, so a blank start is not mistaken for one)."""
    with tempfile.TemporaryDirectory() as tmp:
        png = Path(tmp) / "f.png"
        if step == 0:
            return [shot(sim, card, play, 1000, png)]
        frames: list[Image.Image] = []
        while len(frames) < MAX_FRAMES:
            frames.append(shot(sim, card, play, len(frames) * step, png))
            # Something that starts and ends on a blank panel: one pass is
            # a loop (a pass of an odd number of steps would otherwise take
            # two to repeat exactly).
            if len(frames) > 2 and frames[0].getbbox() is None and frames[-1].getbbox() is None:
                return frames[:-1]
            for p in range(8, len(frames) - 7):
                if all(same(frames[p + k], frames[k]) for k in range(8)):
                    return frames[:p]
        return frames


def once(sim: Path, card: Path, play: str, step: int, length: int) -> list[Image.Image]:
    """Something that plays once: its frames `step` ms apart up to `length`."""
    with tempfile.TemporaryDirectory() as tmp:
        png = Path(tmp) / "f.png"
        return [shot(sim, card, play, t, png) for t in range(0, length, step)]


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
        body = (formula_src(src.removeprefix("formula:").partition("?")[0]) if src.startswith("formula:")
                else (PROCESS / src.removeprefix("scene:")).read_text() if src.startswith("scene:") else "")
        tag = hashlib.sha1(f"{RECIPE}|{src}|{body}|{step}|{colours}|{ONCE.get(name)}".encode()).hexdigest()[:8]
        gif, still = out / f"strip/{name}-{tag}.gif", out / f"strip/{name}-{tag}.png"
        if not gif.exists():
            if src.startswith("kenney:"):
                im = Image.open(KENNEY / src.removeprefix("kenney:"))
                frames, durations = [], []
                for f in ImageSequence.Iterator(im):
                    frames.append(f.convert("RGB"))
                    durations.append(f.info.get("duration", 100))
            elif src.startswith("scene:"):
                frames, durations = scene(PROCESS / src.removeprefix("scene:"))
            else:
                length, hold = ONCE.get(name, (0, 0))
                frames = once(a.sim, card, src, step, length) if length else record(a.sim, card, src, step)
                durations = [step] * len(frames)
                durations[-1] += hold
            # A single frame is a still; the GIF is then just that still.
            save(frames, durations, colours, gif)
            # (Something that plays once: its last frame, the finished picture.)
            save([frames[-1] if name in ONCE else frames[len(frames) // 2]], [0], colours, still)
            print(f"{gif.name}: {len(frames)} frames, {gif.stat().st_size:,} B")
        keep |= {gif.name, still.name}
        items.append({"name": name, "gif": f"strip/{gif.name}", "still": f"strip/{still.name}",
                      "where": where.split(), "alt": alt, "caption": caption})
    for old in (out / "strip").iterdir():
        if old.name not in keep:
            old.unlink()

    # The wordmark: the big font, cropped to the lit pixels plus a pixel.
    with tempfile.TemporaryDirectory() as tmp:
        # (200 ms in: before the card's first glint, at 1.5 s.)
        im = shot(a.sim, card, WORDMARK, 200, Path(tmp) / "w.png")
    x0, y0, x1, y1 = im.getbbox()
    # "Open" is lit in the panel's warm white (opendmd-core text::TITLE_COLOUR),
    # which the LEDs, driven without gamma correction, show as white; drawn on
    # a screen the same numbers read as cream. So the picture uses the mark's
    # off-white for it (tools/logo.py INK), as the panel looks.
    im = im.convert("RGB")
    im.putdata([PANEL_WHITE_SHOWN if p == PANEL_WHITE else p for p in (im.get_flattened_data() if hasattr(im, "get_flattened_data") else im.getdata())])
    im.crop((x0 - 1, y0 - 1, x1 + 1, y1 + 1)).quantize(8, dither=Image.Dither.NONE).save(out / "wordmark.png")
    w, h = Image.open(out / "wordmark.png").size

    (out / "art.json").write_text(json.dumps({"items": items, "wordmark": {"src": "wordmark.png", "w": w, "h": h}},
                                             indent=1) + "\n")
    total = sum(p.stat().st_size for p in out.rglob("*") if p.suffix in (".gif", ".png"))
    strip = sum((out / i["gif"]).stat().st_size for i in items if "strip" in i["where"])
    print(f"{out}: {len(items)} animations, strip {strip:,} B, all art {total:,} B")


if __name__ == "__main__":
    main()
