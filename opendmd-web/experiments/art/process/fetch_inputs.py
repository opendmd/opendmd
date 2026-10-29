# /// script
# requires-python = ">=3.11"
# ///
"""Gather the freely licensed inputs the demos use into $ART_TMP/src
(default /tmp/opendmd-art/src). `fetch_inputs.py kenney` stops after the
Kenney files (offline; all the site's three GIFs need).

  - Kenney "Pixel Platformer" 1.2, CC0 (https://kenney.nl/assets/pixel-platformer):
    the five files used are committed in kenney-pixel-platformer/ (kenney.nl
    no longer offers a stable direct download), copied to src/pp; the rest
    is downloaded and not committed
  - Public-domain (PD-textlogo) console logos from Wikimedia Commons
  - Tom Thumb 3x5 font (MIT / CC0 / CC-BY 3.0, https://robey.lag.net/2010/01/23/tiny-monospace-font.html)
  - Spleen 2.1.0 5x8 and 8x16 (BSD-2-Clause, https://github.com/fcambus/spleen)
"""
import io, os, shutil, sys, tarfile, urllib.request
from pathlib import Path

SRC = Path(os.environ.get("ART_TMP", "/tmp/opendmd-art")) / "src"
UA = {"User-Agent": "OpenDMD-art-prototype/0.1 (personal use; https://github.com/)"}

def get(url):
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA)) as r:
        return r.read()

COMMONS = "https://thumb.wikimedia.org/wikipedia/commons/thumb/"
LOGOS = {  # File:… on commons.wikimedia.org, all "Public domain" per the API's LicenseShortName
    "sega.png": "1/13/SEGA_logo.svg/960px-SEGA_logo.svg.png",
    "snes.png": "2/2c/SNES_logo.svg/960px-SNES_logo.svg.png",
    "saturn.png": "a/a0/Sega_Saturn_Japanese_logo_with_blue_planet.svg/960px-Sega_Saturn_Japanese_logo_with_blue_planet.svg.png",
    "megadrive.png": "6/63/MegaDriveJPLogo.svg/960px-MegaDriveJPLogo.svg.png",
    "3ds.png": "8/89/Nintendo_3DS_logo.svg/960px-Nintendo_3DS_logo.svg.png",
}

def main():
    SRC.mkdir(parents=True, exist_ok=True)
    if not (SRC / "pp").exists():
        shutil.copytree(Path(__file__).resolve().parent / "kenney-pixel-platformer", SRC / "pp")
    if sys.argv[1:] == ["kenney"]:  # all demos b and d (the site's GIFs) need: offline
        print("inputs in", SRC)
        return
    for name, path in LOGOS.items():
        if not (SRC / name).exists():
            (SRC / name).write_bytes(get(COMMONS + path))
    if not (SRC / "tom-thumb.bdf").exists():
        (SRC / "tom-thumb.bdf").write_bytes(get("https://robey.lag.net/downloads/tom-thumb.bdf"))
    if not (SRC / "spleen-2.1.0").exists():
        t = tarfile.open(fileobj=io.BytesIO(get("https://github.com/fcambus/spleen/releases/download/2.1.0/spleen-2.1.0.tar.gz")))
        for m in ("LICENSE", "spleen-5x8.bdf", "spleen-8x16.bdf"):
            t.extract(f"spleen-2.1.0/{m}", SRC, filter="data")
    print("inputs in", SRC)

main()
