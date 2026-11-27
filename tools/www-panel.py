# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Stage the panel's own web app: what the www pack holds, as files.

    uv run tools/www-panel.py opendmd-web/www-dist opendmd-web/pkg-art-panel opendmd-web/www-panel [--webmin PATH] [--no-minify]

`make web` runs this after it has minified and gzipped DIST (www-dist: the
web app with everything the site's repository mounts from it). OUT is a
copy of DIST without

- the site's own files (opendmd-pack's `SITE_ONLY`, read from
  opendmd-pack/src/www.rs so the two never differ: the demo's player, the
  site's wasm, and the converter the site's /create/ page mounts —
  js/studio.js, studio.css, js/find.js, find.css), and
- DIST's pkg-art/ (the art module with the site's Studio parts, feature
  `studio`), replaced by PANEL_ART: the same crate built without them
  (wasm-pack's output, its package.json, .d.ts, README and .gitignore left
  out), the glue minified the way tools/web-minify.py minifies it, each file
  gzipped beside it (`gzip -9 -n`, as `make web` does the rest).

The pack, the size lines, the mock and web-smoke read OUT, so what is
tested is what a panel serves; the site keeps reading DIST.
"""
import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
WWW_RS = REPO / "opendmd-pack/src/www.rs"
WASM_PACK_EXTRA = ("package.json", "README.md", ".gitignore")


def site_only() -> list[str]:
    """opendmd-pack's SITE_ONLY: files, and folders ending in `/`."""
    m = re.search(r"pub const SITE_ONLY: &\[&str\] = &\[(.*?)\];", WWW_RS.read_text(), re.S)
    if not m:
        raise SystemExit(f"www-panel: no SITE_ONLY in {WWW_RS}")
    return re.findall(r'"([^"]+)"', m.group(1))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("dist", type=Path)
    ap.add_argument("panel_art", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--webmin", default="opendmd-webmin/target/release/opendmd-webmin")
    ap.add_argument("--no-minify", action="store_true")
    a = ap.parse_args()
    dist, art, out, webmin = a.dist.resolve(), a.panel_art.resolve(), a.out.resolve(), Path(a.webmin).resolve()
    if not (dist / "index.html").is_file():
        print(f"www-panel: no {dist}/index.html (make web)", file=sys.stderr)
        return 2
    if not any(art.glob("*.wasm")):
        print(f"www-panel: no wasm in {art} (make web)", file=sys.stderr)
        return 2
    if not a.no_minify and not webmin.is_file():
        print(f"www-panel: no {webmin} (make webmin)", file=sys.stderr)
        return 2
    skip = site_only()
    files = {s for s in skip if not s.endswith("/")}
    dirs = [s for s in skip if s.endswith("/")] + ["pkg-art/"]

    def ignore(d: str, names: list[str]) -> list[str]:
        rel = Path(d).relative_to(dist).as_posix()
        rel = "" if rel == "." else rel + "/"
        return [n for n in names if (rel + n).removesuffix(".gz") in files or (rel + n + "/") in dirs]

    if out.exists():
        shutil.rmtree(out)
    shutil.copytree(dist, out, ignore=ignore)
    left = sorted(p.relative_to(out).as_posix() for p in out.rglob("*") if p.is_file()
                  and (p.relative_to(out).as_posix().removesuffix(".gz") in files
                       or any(p.relative_to(out).as_posix().startswith(d) for d in dirs)))
    if left:
        print(f"www-panel: left in {out}: {', '.join(left)}", file=sys.stderr)
        return 1
    pa = out / "pkg-art"
    shutil.copytree(art, pa, ignore=lambda _, names: [n for n in names if n in WASM_PACK_EXTRA or n.endswith(".d.ts") or n.endswith(".gz")])
    glue = sorted(p.name for p in pa.glob("*.js"))
    if not a.no_minify and glue:
        subprocess.run([str(webmin), str(art), str(pa), *glue], check=True)
    gz = [str(p) for p in pa.iterdir() if p.suffix in (".js", ".wasm")]
    subprocess.run(["gzip", "-9", "-n", "-k", "-f", *gz], check=True)
    n = sum(1 for p in out.rglob("*") if p.is_file() and p.suffix != ".gz")
    print(f"{out}: the panel's web app, {n} files ({', '.join(skip)} left to the site; pkg-art without the site's Studio parts)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
