# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.60.1", "brotli==1.1.0"]
# ///
"""Make the headings' pixel font, Silkscreen, from its pinned source.

    uv run tools/font-subset.py        # make font

Silkscreen (Jason Kottke, SIL Open Font License 1.1, no Reserved Font Name)
is the web app's and the site's heading face. It is self-hosted, never
linked from Google: the device's web app has to work with no internet (the
setup network). This fetches Silkscreen Regular (400) and its OFL.txt from
the google/fonts repository at a pinned commit, checks their SHA-256, and
writes:

  opendmd-web/www/fonts/silkscreen.woff2   printable ASCII and the few
      typographic marks below: the Silkscreen headings are fixed English
      labels (a heading that shows a GIF's title or a playlist's name stays
      in the body face, so no other script is needed). Packed in the www
      pack, published with the site and the demo.
  opendmd-web/www/fonts/OFL.txt            the licence, shipped beside it.
  firmware/web/setup.html                  the embedded setup page's
      @font-face line (between the `silkscreen:` markers): the same file as
      a base64 data URI. The setup page is the fallback for a missing or
      broken www pack, so it cannot load the pack's file.

The design grid is 8 px (125 units of 1000 per pixel): the font is crisp
only at multiples of 8 px with no fractional letter-spacing. No layout
features are kept (the font has no kerning; its mark positioning is not
needed for precomposed Latin-1), and no hinting.
"""
import base64
import hashlib
import io
import re
import sys
import urllib.request
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
# google/fonts, ofl/silkscreen at this commit (2026-03-12): Silkscreen
# Version 1.001, built from googlefonts/silkscreen 206ccf3f (METADATA.pb).
COMMIT = "8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5"
BASE = f"https://raw.githubusercontent.com/google/fonts/{COMMIT}/ofl/silkscreen/"
SOURCES = {
    "Silkscreen-Regular.ttf": "c845473330b94c2079ce9af01c51ac8ba2d99c24f4d14c039843bbb8e642ebd8",
    "OFL.txt": "86c5e9c9382cdcc5948704fdfe60f2aa164a719746931219a42736ecd9cefbd3",
}
# Typographic marks a heading may use: ’ ‘ “ ” – — · • … ‹ ›
MARKS = [0x2018, 0x2019, 0x201C, 0x201D, 0x2013, 0x2014, 0x00B7, 0x2022, 0x2026, 0x2039, 0x203A]
ASCII = list(range(0x20, 0x7F))

WOFF2 = ROOT / "opendmd-web/www/fonts/silkscreen.woff2"
OFL = ROOT / "opendmd-web/www/fonts/OFL.txt"
SETUP = ROOT / "firmware/web/setup.html"
SETUP_LINE = re.compile(r"(/\* silkscreen: tools/font-subset\.py \*/).*?(/\* :silkscreen \*/)", re.S)
# The setup page's copy is inlined into the firmware image: keep it small.
SETUP_MAX_B64 = 4096


def fetch(name: str) -> bytes:
    with urllib.request.urlopen(BASE + name, timeout=30) as r:
        data = r.read()
    want = SOURCES[name]
    got = hashlib.sha256(data).hexdigest()
    if got != want:
        sys.exit(f"{name}: sha256 {got}, expected {want} ({BASE + name})")
    return data


def make(ttf: bytes, unicodes: list[int]) -> bytes:
    opts = subset.Options()
    opts.flavor = "woff2"
    opts.layout_features = []
    opts.hinting = False
    opts.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]  # copyright and licence kept
    # Keep the source's head.modified, so the output is byte-for-byte reproducible.
    font = TTFont(io.BytesIO(ttf), recalcTimestamp=False)
    s = subset.Subsetter(opts)
    s.populate(unicodes=unicodes)
    s.subset(font)
    out = io.BytesIO()
    font.flavor = "woff2"
    font.save(out, reorderTables=False)
    return out.getvalue()


def main() -> int:
    ttf = fetch("Silkscreen-Regular.ttf")
    ofl = fetch("OFL.txt")
    WOFF2.parent.mkdir(parents=True, exist_ok=True)
    woff2 = make(ttf, ASCII + MARKS)
    WOFF2.write_bytes(woff2)
    OFL.write_bytes(ofl)
    b64 = base64.b64encode(woff2).decode()
    if len(b64) > SETUP_MAX_B64:
        sys.exit(f"setup.html's subset is {len(b64)} B of base64, over {SETUP_MAX_B64}")
    html = SETUP.read_text()
    face = ('@font-face{font-family:Silkscreen;font-display:swap;'
            f'src:url(data:font/woff2;base64,{b64}) format("woff2")}}')
    new, n = SETUP_LINE.subn(lambda m: f"{m.group(1)}{face}{m.group(2)}", html)
    if n != 1:
        sys.exit(f"{SETUP.relative_to(ROOT)}: expected one silkscreen marker pair, found {n}")
    SETUP.write_text(new)
    print(f"{WOFF2.relative_to(ROOT)}: {len(woff2):,} B; {OFL.relative_to(ROOT)}: {len(ofl):,} B; "
          f"{SETUP.relative_to(ROOT)}: {len(b64):,} B of base64")
    return 0


if __name__ == "__main__":
    sys.exit(main())
