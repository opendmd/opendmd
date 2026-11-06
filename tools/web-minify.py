# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Stage the web app for shipping: a copy of `opendmd-web/www` with every
hand-written script and stylesheet minified.

    uv run tools/web-minify.py opendmd-web/www opendmd-web/www-dist [--webmin PATH] [--no-minify]

`make web` runs this after wasm-pack and the generated files (js/build.js,
formulas.json, version.txt, licenses.txt), then gzips, packs and measures
the output, never the sources: the hand-written files keep their comments
(docs/notes/web-app.md "Minified build"). Everything the pack, the mock,
web-smoke and the site's /demo/ (which /f/ and /create/ mount) read comes
from OUT.

The minifier is opendmd-webmin (a crate of its own, versions pinned in its
Cargo.lock): oxc for scripts, lightningcss for stylesheets, one file at a
time. No bundling, so the module graph, every file's path and every import
specifier stay as they are; the mangler renames nested scopes only, so
every top-level name and export keeps its spelling; ids, classes and
`data-*` are strings it never touches, and property names (wasm-bindgen's
`imports.wbg.__wbg_*` keys) are never renamed. wasm-pack's glue in `pkg*/`
is minified too: its JSDoc is most of it. The build id
(tools/www-build-id.py) hashes the glue as wasm-pack wrote it, so it still
changes exactly when the glue does. Not minified: `js/build.js`, one line,
read by tools/site-worker-test.py by pattern.

`--no-minify` (`make web WEB_MINIFY=0`) copies the files as they are, for
reading the app in a browser's debugger on a device or the mock.
"""
import argparse
import shutil
import subprocess
import sys
from pathlib import Path

SKIP_FILES = ("js/build.js",)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("src", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--webmin", default="opendmd-webmin/target/release/opendmd-webmin")
    ap.add_argument("--no-minify", action="store_true")
    a = ap.parse_args()
    src, out, webmin = a.src.resolve(), a.out.resolve(), Path(a.webmin).resolve()
    if not a.no_minify and not webmin.is_file():
        print(f"web-minify: no {webmin} (make webmin)", file=sys.stderr)
        return 2
    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(src, out, ignore=shutil.ignore_patterns("*.gz", ".gitignore"))
    todo = sorted(p.relative_to(src).as_posix() for p in src.rglob("*")
                  if p.is_file() and p.suffix in (".js", ".css")
                  and p.relative_to(src).as_posix() not in SKIP_FILES)
    if a.no_minify:
        print(f"{out}: {len(todo)} scripts and stylesheets as they are (WEB_MINIFY=0)")
        return 0
    before = sum((src / r).stat().st_size for r in todo)
    # One run for every file (each on its own: no bundling).
    r = subprocess.run([str(webmin), str(src), str(out), *todo])
    if r.returncode != 0:
        print(f"web-minify: opendmd-webmin failed ({r.returncode})", file=sys.stderr)
        return 1
    after = sum((out / r).stat().st_size for r in todo)
    print(f"{out}: {len(todo)} scripts and stylesheets minified, {before:,} -> {after:,} B raw")
    return 0


if __name__ == "__main__":
    sys.exit(main())
