# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the web demo's card: nothing third-party, all of it ours or CC0.

    uv run tools/demo-card.py OUT_DIR [--sim PATH]

- gifs/Demoscene/: recordings of Formulas (opendmd-web/www/formulas.json),
  drawn by the simulator's raw canvas (--screenshot-canvas) a frame at a time.
- gifs/Kenney/: the Kenney "Pixel Platformer" (CC0) animations made by the
  art pipeline prototypes (opendmd-web/experiments/art/process/out/).
- filelist.txt: the demo's "All files", live items (the clock,
  scrolling text) between the GIFs; sets/: two more playlists.
- index.json: every file and its size, which the demo reads instead of a
  directory listing.

Recordings already in OUT_DIR are kept: delete the directory to redo them
(the old FX_* ones from the built-in effects are removed).
"""

import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
KENNEY = ROOT / "opendmd-web/experiments/art/process/out"

FORMULAS = ROOT / "opendmd-web/www/formulas.json"

# (file title, Formula name in formulas.json, --formula-opts, frames at 40 ms).
# Only whole-picture Formulas: none of the looks meant to sit over a GIF.
RECORD = [
    ("Starfield", "Starfield", "", 75),
    ("Rotozoom_Floor", "Rotozoom floor", "", 75),
    ("Wireframe_Cube", "Wireframe cube", "", 75),
    ("Rotozoom", "Rotozoom", "", 75),
    ("Plasma", "Plasma", "", 75),
    ("Copper_Bars", "Copper bars", "", 75),
    ("Tunnel", "Tunnel", "", 75),
    ("Rings", "Rings", "", 75),
    ("Kaleidoscope", "Kaleidoscope", "", 75),
    ("Time_Gate", "Time gate", "", 75),
    ("Endless_Road", "Endless road", "", 75),
]
KENNEY_GIFS = {
    "b_walk.gif": "OTHER_Pixel_Platformer_Walk_01_kenney.gif",
    "b_pan.gif": "OTHER_Pixel_Platformer_Pan_01_kenney.gif",
    "d_scroll.gif": "OTHER_Pixel_Platformer_Scroll_01_kenney.gif",
}
# The demo's All files: live items, drawn by the panel, between the GIFs.
# (No Formula paths: the demo's in-browser preview cannot draw them and skips
# them; the Formulas are the Demoscene GIFs, recorded here.)
LIVE = [
    "@text?msg=OpenDMD%20-%20open%20firmware%20for%20LED%20dot-matrix%20displays&speed=30&color=ffaa00",
    "@clock?style=big&date=1",
    "@text?msg=Made%20in%20the%20browser&speed=30&color=00ccff",
    "@clock?style=small&seconds=1",
]


def formula_src(name: str) -> str:
    """The source of the example Formula called `name`; fails if there is none."""
    for e in json.loads(FORMULAS.read_text()):
        if e["name"] == name:
            return e["src"]
    raise SystemExit(f"demo-card: no Formula named {name!r} in {FORMULAS}")


def record(sim: Path, card: Path, name: str, opts: str, frames: int, out: Path) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        png = Path(tmp) / "f.png"
        psi = Path(tmp) / "f.psi"
        psi.write_text(formula_src(name))
        play = ["--formula", psi] + (["--formula-opts", opts] if opts else [])
        shots = []
        for i in range(frames):
            subprocess.run([sim, "--root", card, *play, "--screenshot-canvas", png, "--screenshot-at", str(i * 40)],
                           check=True, capture_output=True)
            shots.append(Image.open(png).convert("RGB").quantize(64, dither=Image.Dither.NONE))
    shots[0].save(out, save_all=True, append_images=shots[1:], duration=40, loop=0)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--sim", type=Path, default=ROOT / "opendmd-sim/target/release/opendmd-sim")
    a = ap.parse_args()
    card: Path = a.out
    (card / "gifs/Demoscene").mkdir(parents=True, exist_ok=True)
    (card / "gifs/Kenney").mkdir(parents=True, exist_ok=True)
    (card / "sets").mkdir(exist_ok=True)
    # The simulator wants a card root with a filelist, even to play one thing.
    if not (card / "filelist.txt").exists():
        (card / "filelist.txt").write_text("")

    fx = []
    for old in (card / "gifs/Demoscene").glob("FX_*.gif"):
        old.unlink()
    for title, name, opts, n in RECORD:
        formula_src(name)  # fail loudly on a missing name, even for a kept recording
        out = card / f"gifs/Demoscene/Formula_{title}_01.gif"
        if not out.exists():
            print(f"recording {out.name} ({n} frames)")
            record(a.sim, card, name, opts, n, out)
        fx.append(f"/gifs/Demoscene/{out.name}")
    kenney = []
    for src, name in KENNEY_GIFS.items():
        shutil.copyfile(KENNEY / src, card / f"gifs/Kenney/{name}")
        kenney.append(f"/gifs/Kenney/{name}")

    # Live items, with a GIF after every second one.
    gifs = kenney + fx[::3]
    lines = []
    for i, item in enumerate(LIVE):
        lines.append(item)
        if i >= 2 and i % 2 == 0 and gifs:
            lines.append(gifs.pop(0))
    lines += gifs
    (card / "filelist.txt").write_text("\n".join(lines) + "\n")
    (card / "sets/Demoscene.txt").write_text("\n".join(fx) + "\n")
    (card / "sets/Sprites and Clock.txt").write_text("\n".join(kenney + ["@clock?style=small&seconds=1"]) + "\n")

    files = sorted(p for p in card.rglob("*") if p.is_file() and p.name != "index.json")
    index = [{"path": "/" + p.relative_to(card).as_posix(), "size": p.stat().st_size} for p in files]
    (card / "index.json").write_text(json.dumps(index, indent=0) + "\n")
    print(f"{card}: {len(index)} files, {sum(f['size'] for f in index):,} B")


if __name__ == "__main__":
    main()
