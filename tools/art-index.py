# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Build the "Find art for a game" name index from libretro-thumbnails.

    uv run tools/art-index.py [--out opendmd-web/www/art] [PLATFORM ...]

One JSON file per platform tag, `<out>/<PLATFORM>.json`:

    {"repo": "Nintendo_-_Game_Boy", "names": ["Tetris (World) (Rev 1)", ...],
     "kinds": "7f3..."}

`names` is every game that has a picture in any of `Named_Titles`,
`Named_Snaps`, `Named_Boxarts` or `Named_Logos` (betas, prototypes,
demos, hacks and bad dumps left out), sorted; `kinds` holds one
hex digit per name, a bit per folder that has it (1 titles, 2 snaps, 4 box
art, 8 logos). The Studio (`www/js/studio.js`) fetches the platform's file
from the card on the first search, matches titles in the browser, and loads
the pictures from raw.githubusercontent.com (which, unlike
thumbnails.libretro.com, sends `Access-Control-Allow-Origin: *`).

Why a file on the card and not the GitHub API (which pages can read): one
folder's tree is 1.3 MB of JSON (SNES titles alone; 175 KB gzipped), a
search wants three or four of them, and unauthenticated calls are capped at
60 an hour per IP, shared by everything in the house. These files are 2.4 MB
for all thirteen platforms, 530 KB gzipped (SNES: 132 KB, 32 KB gzipped),
and the Studio fetches only the platform searched. The
index is read with `git ls-tree` on a blobless, depth-1 clone — no API
calls, no images downloaded. Re-run it now and then; libretro adds games.
"""
import argparse
import gzip
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# Platform tag (the card's filename grammar) -> libretro-thumbnails repo.
REPOS = {
    "MD": "Sega_-_Mega_Drive_-_Genesis",
    "SNES": "Nintendo_-_Super_Nintendo_Entertainment_System",
    "GB": "Nintendo_-_Game_Boy",
    "GBC": "Nintendo_-_Game_Boy_Color",
    "GBA": "Nintendo_-_Game_Boy_Advance",
    "SATURN": "Sega_-_Saturn",
    "NDS": "Nintendo_-_Nintendo_DS",
    "NES": "Nintendo_-_Nintendo_Entertainment_System",
    "SMS": "Sega_-_Master_System_-_Mark_III",
    "PCE": "NEC_-_PC_Engine_-_TurboGrafx_16",
    "PS1": "Sony_-_PlayStation",
    "N64": "Nintendo_-_Nintendo_64",
    "ARCADE": "MAME",
}
KINDS = {"Named_Titles": 1, "Named_Snaps": 2, "Named_Boxarts": 4, "Named_Logos": 8}
# Left out: betas, prototypes, demos, pirate copies and hacks, and TOSEC-style
# dump flags (`[b]`, `[h]`, ...) — about a fifth of the names, none of them
# anything someone owns.
SKIP = re.compile(r"\((?:Beta|Proto|Sample|Demo|Kiosk|Pirate|Hack|Debug|Program)[^)]*\)|\[", re.I)


def listing(repo: str, tmp: Path) -> dict[str, int]:
    dest = tmp / repo
    subprocess.run(["git", "clone", "-q", "--depth", "1", "--filter=blob:none", "--no-checkout",
                    f"https://github.com/libretro-thumbnails/{repo}", str(dest)], check=True)
    out = subprocess.run(["git", "-c", "core.quotePath=false", "ls-tree", "-r", "--name-only", "-z", "HEAD"],
                         cwd=dest, check=True, capture_output=True).stdout.decode("utf-8")
    names: dict[str, int] = {}
    for path in out.split("\0"):
        folder, _, file = path.partition("/")
        bit = KINDS.get(folder)
        # Only files directly in a Named_* folder (a stray subfolder exists).
        if not bit or "/" in file or not file.lower().endswith(".png"):
            continue
        stem = file[:-4]
        if SKIP.search(stem):
            continue
        names[stem] = names.get(stem, 0) | bit
    return names


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("platforms", nargs="*", help="platform tags (default: all)")
    ap.add_argument("--out", default="opendmd-web/www/art")
    a = ap.parse_args()
    want = a.platforms or list(REPOS)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    total_raw = total_gz = 0
    with tempfile.TemporaryDirectory() as t:
        for plat in want:
            repo = REPOS[plat]
            names = listing(repo, Path(t))
            order = sorted(names, key=lambda s: (s.casefold(), s))
            doc = {"repo": repo, "names": order, "kinds": "".join(f"{names[n]:x}" for n in order)}
            data = json.dumps(doc, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            (out / f"{plat}.json").write_bytes(data)
            gz = len(gzip.compress(data, 9, mtime=0))
            total_raw += len(data)
            total_gz += gz
            print(f"{plat:<7} {repo:<50} {len(order):>6} names {len(data):>9,} B  gz {gz:>8,} B")
    print(f"{'total':<58} {total_raw:>22,} B  gz {total_gz:>8,} B")
    return 0


if __name__ == "__main__":
    sys.exit(main())
