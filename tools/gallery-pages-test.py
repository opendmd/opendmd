# /// script
# requires-python = ">=3.11"
# dependencies = ["markdown>=3.6"]
# ///
"""The formula gallery's page (/formulas/) as `make site` writes it:
tools/site.py's `write_gallery` over the web app's examples and a stand-in
for the public gallery repo's gallery/:

    uv run tools/gallery-pages-test.py [--site DIR] [--keep DIR] [-v]

1. The page has its markers, every example and every good contribution,
   and every thumbnail is lit (those over a GIF drawn over the sample
   picture). With `--site`, a real `make site` output's formulas/ is
   checked the same way first.
2. Hostile files in gallery/ (a markup name, a link out of gallery/, a
   folder) are left out, the log says so, and the rest is still published.

Nothing leaves the machine. Run by `make test`.
"""

import argparse
import contextlib
import importlib.util
import io
import os
import shutil
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GALLERY = ROOT / "opendmd-gallery"
SPIRAL = ("-- name: Spiral sweep\n-- description: A rainbow spiral turning around the middle\n-- author: @contributor\n"
          "-- licence: CC0-1.0\nloop 8\na2 = atan2(y - 0.5, (x - 0.5)*4)\nd = len((x - 0.5)*4, y - 0.5)\n"
          "hsv(frac(a2 + d*0.6 - t/8), 1, 1)\n")
RINGS = ("-- name: Rainbow rings\n-- description: Rings rippling out from the middle\n-- author: @someone\n"
         "-- licence: CC-BY-4.0\n-- values: a=0.3\nloop 4\nd = len((x - 0.5)*4, y - 0.5)\npal(rainbow, frac(d*1.5*(a + 0.5) - t/2))\n")
EVIL = SPIRAL.replace("Spiral sweep", "<script>alert(1)</script>")


def sh(*args, cwd=None, env=None, check=True) -> subprocess.CompletedProcess:
    e = {**os.environ, "GIT_AUTHOR_NAME": "Test", "GIT_AUTHOR_EMAIL": "test@example.invalid",
         "GIT_COMMITTER_NAME": "Test", "GIT_COMMITTER_EMAIL": "test@example.invalid", **(env or {})}
    r = subprocess.run([str(a) for a in args], cwd=cwd, env=e, capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"{' '.join(map(str, args))}: exit {r.returncode}\n{r.stdout}{r.stderr}")
    return r


def load_site_py():
    sys.path.insert(0, str(ROOT / "tools"))
    spec = importlib.util.spec_from_file_location("site_build", ROOT / "tools/site.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def load_site_py():
    sys.path.insert(0, str(ROOT / "tools"))
    spec = importlib.util.spec_from_file_location("site_build", ROOT / "tools/site.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def dark_thumbs(thumbs: Path) -> list[str]:
    """The thumbnails with no lit pixel (r + g + b over 90, as site-smoke
    checks the page in Chromium): opendmd-gallery's 8-bit RGB, unfiltered."""
    dark = []
    for p in sorted(thumbs.glob("*.png")):
        b, i, idat = p.read_bytes(), 8, b""
        while i < len(b):
            n = int.from_bytes(b[i:i + 4], "big")
            if b[i + 4:i + 8] == b"IDAT":
                idat += b[i + 8:i + 8 + n]
            i += 12 + n
        raw, w = zlib.decompress(idat), int.from_bytes(b[16:20], "big")
        rows = [raw[k + 1:k + 1 + w * 3] for k in range(0, len(raw), w * 3 + 1)]
        if b[24:26] != b"\x08\x02" or any(raw[k] for k in range(0, len(raw), w * 3 + 1)) or \
                not any(sum(r[j:j + 3]) > 90 for r in rows for j in range(0, len(r), 3)):
            dark.append(p.stem)
    return dark


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--site", type=Path, help="also check a real `make site` output's formulas/")
    ap.add_argument("--keep", type=Path, help="work here and keep it")
    ap.add_argument("-v", action="store_true", help="print the build's log")
    a = ap.parse_args()
    tmp = Path(a.keep or tempfile.mkdtemp(prefix="gallery-page-"))
    if a.keep and tmp.exists():
        shutil.rmtree(tmp)
    tmp.mkdir(parents=True, exist_ok=True)
    failures = 0

    def check(ok: bool, what: str, detail: str = "") -> None:
        nonlocal failures
        failures += not ok
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")
        if detail and (not ok or a.v):
            print("".join(f"       | {line}\n" for line in detail.strip().splitlines()[:40]))

    try:
        if a.site:
            dark = dark_thumbs(a.site / "formulas/examples") + dark_thumbs(a.site / "formulas/thumbs")
            n = len(list((a.site / "formulas").glob("*/*.png")))
            check(n > 10 and not dark, f"{a.site}: every thumbnail lit ({n}, dark: {dark})")
        site_py = load_site_py()
        # The release's checker, as `make gallery-tool` builds it.
        sh("cargo", "build", "-q", "--release", "--locked", "--manifest-path", GALLERY / "Cargo.toml",
           "--target-dir", tmp / "release-target", env={"RUSTUP_TOOLCHAIN": os.environ.get("RUSTUP_TOOLCHAIN", "stable")})
        tool = tmp / "release-target/release/opendmd-gallery"
        www = ROOT / "opendmd-web/www"
        wm = {"w": 64, "h": 8, "src": "wordmark.png"}

        def build(name: str, contrib: Path) -> tuple[Path, str]:
            out = tmp / name
            log = io.StringIO()
            with contextlib.redirect_stderr(log):
                site_py.write_gallery(out, www, contrib, tool, wm)
            return out, log.getvalue()

        # 1. Two good contributions.
        g1 = tmp / "gallery-1"
        g1.mkdir()
        (g1 / "spiral-sweep.psi").write_text(SPIRAL)
        (g1 / "rainbow-rings.psi").write_text(RINGS)
        out, log = build("site-1", g1)
        page = (out / "formulas/index.html").read_text()
        check(page.count("<!-- gallery:begin -->") == 1 and page.count("<!-- gallery:end -->") == 1, "the page has its markers")
        check("<h3>Spiral sweep</h3>" in page and "<h3>Rainbow rings</h3>" in page
              and (out / "formulas/thumbs/rainbow-rings.png").is_file(), "the contributions are on the page, with thumbnails", log)
        thumbs = sorted((out / "formulas").glob("*/*.png"))
        dark = dark_thumbs(out / "formulas/examples") + dark_thumbs(out / "formulas/thumbs")
        check(len(thumbs) > 10 and not dark, f"every thumbnail lit, those over a GIF over the sample picture ({len(thumbs)}, dark: {dark})")

        # 2. Hostile files: markup in a name, a link out of gallery/, a folder.
        g2 = tmp / "gallery-2"
        shutil.copytree(g1, g2)
        (g2 / "evil.psi").write_text(EVIL)
        (g2 / "passwd.psi").symlink_to("/etc/passwd")
        (g2 / "folder.psi").mkdir()
        (g2 / "folder.psi/x.psi").write_text(SPIRAL)
        out, log = build("site-2", g2)
        page = (out / "formulas/index.html").read_text()
        check("<script>" not in page and "alert(1)" not in page and not (out / "formulas/thumbs/evil.png").exists(),
              "markup in a name: left out (not on the page, no thumbnail)", log)
        check("left out 'evil.psi'" in log and "left out 'passwd.psi': not a plain file" in log
              and "left out 'folder.psi': not a plain file" in log, "the log names what was left out", log)
        check("<h3>Rainbow rings</h3>" in page and "<h3>Spiral sweep</h3>" in page, "the good contributions are still there")
    finally:
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
    print(f"gallery-pages-test: {'ok' if not failures else f'{failures} failures'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
