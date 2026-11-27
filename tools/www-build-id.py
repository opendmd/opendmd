# /// script
# requires-python = ">=3.11"
# ///
"""Write the web app's build id: `<www>/js/build.js`, `export const V = '<id>';`.

    uv run tools/www-build-id.py opendmd-web/www [opendmd-web/pkg-art-panel]

The id is the first 16 hex digits of a SHA-256 over every file wasm-pack
wrote (pkg/, pkg-art/, pkg-psi/: their paths and bytes, `.gz` and dotfiles
aside), so it changes exactly when a wasm module or its glue does — two
builds of a dirty tree share a `git describe`, never this. js/wasm.js puts
it on every module's address (`?v=<id>`): wasm-pack's file names carry no
hash, and a browser that kept an earlier build's module (beta.13 served
them with `max-age=3600`) must never run it with this build's scripts
(opendmd-notes docs/notes/web-app.md "Module addresses"). `make web` runs this; the file
is gitignored, served `no-cache` like every script.

The second argument, when given, is the panel's own pkg-art (the art module
built without its site-only Studio parts, which `make web` packs in place
of www/pkg-art: tools/www-panel.py): hashed after the three, as
`pkg-art-panel/...`, so the panel's id changes whenever its module does.
"""
import hashlib
import sys
from pathlib import Path

# The site's repository lays its own wasm (pkg-psi-lang/, pkg-studio/) over
# the web app and writes the id again over these and them (its tools/site-www.py).
DIRS = ("pkg", "pkg-art", "pkg-psi")


def build_id(www: Path, panel_art: Path | None = None) -> str:
    h = hashlib.sha256()
    n = 0
    dirs = [(d, www / d) for d in DIRS] + ([("pkg-art-panel", panel_art)] if panel_art else [])
    for d, base in dirs:
        for p in sorted(base.rglob("*")):
            if not p.is_file() or p.suffix == ".gz" or p.name.startswith("."):
                continue
            rel = (d + "/" + p.relative_to(base).as_posix()).encode()
            data = p.read_bytes()
            h.update(len(rel).to_bytes(4, "little") + rel + len(data).to_bytes(8, "little") + data)
            n += 1
    if n == 0:
        raise SystemExit(f"{www}: no wasm-pack output in {', '.join(DIRS)} (run make web)")
    return h.hexdigest()[:16]


def write(www: Path, panel_art: Path | None = None) -> str:
    v = build_id(www, panel_art)
    (www / "js" / "build.js").write_text(
        "// Written by `make web` (tools/www-build-id.py): this build's id, on every\n"
        "// wasm module's address (js/wasm.js). Not committed.\n"
        f"export const V = '{v}';\n")
    return v


if __name__ == "__main__":
    if len(sys.argv) not in (2, 3):
        raise SystemExit(__doc__)
    print(f"build id {write(Path(sys.argv[1]), Path(sys.argv[2]) if len(sys.argv) == 3 else None)}")
