"""The web app's bundled Formulas: opendmd-web/www/formulas.json, written by
`make web` from the `.psi` files in opendmd-web/formulas/ that carry the
`-- bundle` line (formula-apps F3).

Every example is one file there, `<slug>.psi`: a header of comment lines
(`-- author: …`, `-- licence: …`, and `-- bundle` on the ones the firmware
ships), then the Formula itself, which says its own name, what it is, its
controls and whether it can be the panel's look (`name "…"`, `about "…"`,
`control …`, `look`). The site's gallery shows all of them (tools/site.py
hands the folder to opendmd-gallery, which reads those lines from the
compiler); the panel's web app gets only the bundled ones, as
`[{"src": …}, …]` (the Formula without the header, in file-name order):
nothing else, since the source says the rest.

    uv run tools/formulas-json.py [FORMULAS_DIR] [OUT]
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEADER = ("-- author:", "-- licence:", "-- license:", "-- bundle")


def split(text: str) -> tuple[list[str], str]:
    """A gallery file's header lines and the Formula after them."""
    lines = text.split("\n")
    n = 0
    while n < len(lines) and lines[n].startswith(HEADER):
        n += 1
    return lines[:n], "\n".join(lines[n:])


def bundled(folder: Path) -> list[dict]:
    """The `-- bundle` files' Formulas, in file-name order."""
    out = []
    files = sorted(folder.glob("*.psi"))
    if not files:
        raise SystemExit(f"formulas-json: no .psi files in {folder}")
    for p in files:
        head, src = split(p.read_text(encoding="utf-8"))
        if not any(h.startswith("-- author:") for h in head) or not any(h.startswith(("-- licence:", "-- license:")) for h in head):
            raise SystemExit(f"formulas-json: {p.name}: the header (-- author: …, -- licence: …) is missing")
        if not any(l.startswith('name "') for l in src.split("\n")):
            raise SystemExit(f'formulas-json: {p.name}: no name "…" line')
        if "-- bundle" in head:
            out.append({"src": src.rstrip("\n") + "\n"})
    if not out:
        raise SystemExit(f"formulas-json: no file in {folder} has a -- bundle line")
    return out


def main() -> int:
    folder = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "opendmd-web/formulas"
    dest = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "opendmd-web/www/formulas.json"
    entries = bundled(folder)
    text = "[\n" + ",\n".join(json.dumps(e, ensure_ascii=False, separators=(",", ":")) for e in entries) + "\n]\n"
    dest.write_text(text, encoding="utf-8")
    print(f"{dest}: {len(entries)} bundled Formulas, {len(text.encode())} B")
    return 0


if __name__ == "__main__":
    sys.exit(main())
