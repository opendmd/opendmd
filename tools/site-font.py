# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools==4.60.1", "brotli==1.1.0"]
# ///
"""Make the site's heading face, Fraunces, from its pinned source.

    uv run tools/site-font.py        # make site-font

Fraunces (The Fraunces Project Authors, SIL Open Font License 1.1, no
Reserved Font Name) is opendmd.dev's display face: the headings of every
page tools/site.py writes. It is the site's alone (the web app the display
serves keeps Silkscreen and its size budget), self-hosted like Silkscreen,
never linked from a font host: the site's CSP allows none.

This fetches the variable font and its OFL.txt from the google/fonts
repository at a pinned commit, checks their SHA-256, and writes:

  site/fonts/fraunces.woff2      one static cut (the axes below), printable
      ASCII and the typographic marks a heading may use, kerning and
      ligatures kept, no hinting. Published as fonts/fraunces.woff2.
  site/fonts/OFL-Fraunces.txt    the licence, published beside it.

Both are committed; run this only to change the cut or the subset.
"""
import hashlib
import io
import sys
import urllib.request
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = Path(__file__).resolve().parent.parent
# google/fonts, ofl/fraunces at this commit (the one tools/font-subset.py pins).
COMMIT = "8b0a1d0f5983c89bc2b93f1b5fb55f9e252744b5"
BASE = f"https://raw.githubusercontent.com/google/fonts/{COMMIT}/ofl/fraunces/"
SOURCES = {
    "Fraunces%5BSOFT%2CWONK%2Copsz%2Cwght%5D.ttf": "177ff6c0f14e5550a3c624247cd1189611d4eb65d000b14944c63d967958abbb",
    "OFL.txt": "bdf4c22802eaf804f998195871c6b8938aac2ac14b2d78a8bd66a6f1eced833b",
}
# The cut: the display optical size, a firm semibold, the soft terminals and
# the "wonky" alternates (the leaning n, h and m) that give it its character.
AXES = {"opsz": 144, "wght": 600, "SOFT": 100, "WONK": 1}
# Typographic marks a heading may use: ’ ‘ “ ” – — · • … ‹ › × →
MARKS = [0x2018, 0x2019, 0x201C, 0x201D, 0x2013, 0x2014, 0x00B7, 0x2022, 0x2026, 0x2039, 0x203A, 0x00D7, 0x2192]
ASCII = list(range(0x20, 0x7F))

WOFF2 = ROOT / "site/fonts/fraunces.woff2"
OFL = ROOT / "site/fonts/OFL-Fraunces.txt"
MAX = 32 * 1024  # a heading face, preloaded by every page: keep it small


def fetch(name: str) -> bytes:
    req = urllib.request.Request(BASE + name, headers={"User-Agent": "opendmd-site-font/1"})
    with urllib.request.urlopen(req, timeout=30) as r:
        data = r.read()
    got = hashlib.sha256(data).hexdigest()
    if got != SOURCES[name]:
        sys.exit(f"{name}: sha256 {got}, expected {SOURCES[name]} ({BASE + name})")
    return data


def make(ttf: bytes) -> bytes:
    # Keep the source's head.modified, so the output is byte-for-byte reproducible.
    font = TTFont(io.BytesIO(ttf), recalcTimestamp=False)
    font = instancer.instantiateVariableFont(font, AXES)
    opts = subset.Options()
    opts.flavor = "woff2"
    opts.layout_features = ["kern", "liga"]
    opts.hinting = False
    opts.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]  # copyright and licence kept
    s = subset.Subsetter(opts)
    s.populate(unicodes=ASCII + MARKS)
    s.subset(font)
    out = io.BytesIO()
    font.flavor = "woff2"
    font.save(out, reorderTables=False)
    return out.getvalue()


def main() -> int:
    names = list(SOURCES)
    woff2 = make(fetch(names[0]))
    if len(woff2) > MAX:
        sys.exit(f"the subset is {len(woff2):,} B, over {MAX:,}")
    WOFF2.parent.mkdir(parents=True, exist_ok=True)
    WOFF2.write_bytes(woff2)
    OFL.write_bytes(fetch(names[1]))
    print(f"{WOFF2.relative_to(ROOT)}: {len(woff2):,} B; {OFL.relative_to(ROOT)}: {OFL.stat().st_size:,} B")
    return 0


if __name__ == "__main__":
    sys.exit(main())
