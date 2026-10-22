# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Build the web demo's card: nothing third-party, all of it ours or CC0.

    uv run tools/demo-card.py OUT_DIR [--sim PATH]

- gifs/Demoscene/: recordings of the panel's own effects, drawn by the
  simulator's raw canvas (--screenshot-canvas) a frame at a time.
- gifs/Kenney/: the Kenney "Pixel Platformer" (CC0) animations made by the
  art pipeline prototypes (opendmd-web/experiments/art/process/out/).
- filelist.txt: the demo's "All files", live items (effects, the clock,
  scrolling text) between the GIFs; sets/: two more playlists.
- index.json: every file and its size, which the demo reads instead of a
  directory listing.

Recordings already in OUT_DIR are kept: delete the directory to redo them.
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

# (file title, live item, frames at 40 ms)
RECORD = [
    ("Tunnel_Chrono", "@fx:tunnel?palette=chrono&cycle=1", 75),
    ("Tunnel_Fire_Rings", "@fx:tunnel?palette=fire&texture=rings&twist=2", 75),
    ("Tunnel_Ice", "@fx:tunnel?palette=ice&texture=checker&layers=2", 75),
    ("Rotozoom_Chrono", "@fx:rotozoom?palette=chrono&cycle=1", 75),
    ("Rotozoom_Floor_Fire", "@fx:rotozoom?floor=1&palette=fire", 75),
    ("Rotozoom_Rings_Rainbow", "@fx:rotozoom?palette=rainbow&texture=rings&wobble=2", 75),
    ("Starfield_Warp", "@fx:starfield?palette=chrono&density=3&speed=3", 75),
    ("Starfield_Dots_Amber", "@fx:starfield?palette=amber&warp=0", 75),
    ("Wire_Tesseract", "@fx:wire?shape=tesseract&color=ff66cc", 75),
    ("Wire_Morph", "@fx:wire?shape=morph", 75),
    ("Wire_Icosahedron", "@fx:wire?shape=icosahedron&color=66ff66", 75),
]
KENNEY_GIFS = {
    "b_walk.gif": "OTHER_Pixel_Platformer_Walk_01_kenney.gif",
    "b_pan.gif": "OTHER_Pixel_Platformer_Pan_01_kenney.gif",
    "d_scroll.gif": "OTHER_Pixel_Platformer_Scroll_01_kenney.gif",
}
# The demo's All files: live items, drawn by the panel, between the GIFs.
LIVE = [
    "@fx:tunnel?palette=fire&texture=rings&twist=2",
    "@text?msg=OpenDMD%20-%20open%20firmware%20for%20LED%20dot-matrix%20displays&speed=30&color=ffaa00",
    "@fx:rotozoom?floor=1&palette=chrono&cycle=1",
    "@fx:wire?shape=tesseract&color=ff66cc",
    "@clock?style=big&date=1",
    "@fx:starfield?palette=ice&density=3",
    "@fx:tunnel?palette=chrono&cycle=1&drift=1",
    "@text?msg=Made%20in%20the%20browser&speed=30&color=00ccff",
    "@fx:rotozoom?palette=rainbow&texture=rings&wobble=2",
    "@fx:wire?shape=morph",
]


def record(sim: Path, card: Path, play: str, frames: int, out: Path) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        png = Path(tmp) / "f.png"
        shots = []
        for i in range(frames):
            subprocess.run([sim, "--root", card, "--play", play, "--screenshot-canvas", png, "--screenshot-at", str(i * 40)],
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
    # The simulator wants a card root with a filelist, even to --play.
    if not (card / "filelist.txt").exists():
        (card / "filelist.txt").write_text("")

    fx = []
    for title, play, n in RECORD:
        out = card / f"gifs/Demoscene/FX_{title}_01.gif"
        if not out.exists():
            print(f"recording {out.name} ({n} frames)")
            record(a.sim, card, play, n, out)
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
        if i % 2 and gifs:
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
