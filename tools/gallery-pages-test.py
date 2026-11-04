# /// script
# requires-python = ">=3.11"
# dependencies = ["markdown>=3.6"]
# ///
"""The formula gallery's page (/formulas/) as `make site` writes it:
tools/site.py's `write_gallery` over OpenDMD's examples (opendmd-web/formulas/)
and a stand-in
for the public gallery repo's gallery/:

    uv run tools/gallery-pages-test.py [--site DIR] [--keep DIR] [-v]

1. The page has its markers, every example and every good contribution,
   and every thumbnail is lit (those over a GIF drawn over the sample
   picture). With `--site`, a real `make site` output's formulas/ is
   checked the same way first.
2. Hostile files in gallery/ (a markup name, a link out of gallery/, a
   folder) are left out, the log says so, and the rest is still published.
3. The order: the examples newest first by opendmd-web/formulas/added.json
   (every tile marked for ftsort.js), contributions by their first commit
   when gallery/ is a whole git checkout, undated (by name, last) in a
   shallow one; an example with no date, or a date with no example, stops
   the build.

Nothing leaves the machine. Run by `make test`.
"""

import argparse
import contextlib
import importlib.util
import io
import json
import re
import os
import shutil
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GALLERY = ROOT / "opendmd-gallery"
SPIRAL = ("-- author: @contributor\n-- licence: CC0-1.0\nname \"Spiral sweep\"\n"
          "about \"A rainbow spiral turning around the middle\"\nloop 8\na2 = atan2(y - 0.5, (x - 0.5)*4)\nd = len((x - 0.5)*4, y - 0.5)\n"
          "hsv(frac(a2 + d*0.6 - t/8), 1, 1)\n")
RINGS = ("-- author: @someone\n-- licence: CC-BY-4.0\nname \"Rainbow rings\"\nabout \"Rings rippling out from the middle\"\n"
         "control width 0..1 = 0.3 \"Width\"\nloop 4\nd = len((x - 0.5)*4, y - 0.5)\npal(rainbow, frac(d*1.5*(width + 0.5) - t/2))\n")
# (Markup can't be a name: the language refuses `<` in quotes. In a comment it
# is only the formula's text, escaped on the page; as the author, refused.)
EVIL = SPIRAL.replace("@contributor", "<script>alert(1)</script>")


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
        wm = {"w": 64, "h": 8, "src": "wordmark.png"}

        def build(name: str, contrib: Path) -> tuple[Path, str]:
            out = tmp / name
            log = io.StringIO()
            with contextlib.redirect_stderr(log):
                site_py.write_gallery(out, site_py.EXAMPLES, contrib, tool, wm)
            return out, log.getvalue()

        # 1. Two good contributions.
        g1 = tmp / "gallery-1"
        g1.mkdir()
        (g1 / "spiral-sweep.psi").write_text(SPIRAL)
        (g1 / "rainbow-rings.psi").write_text(RINGS)
        out, log = build("site-1", g1)
        page = (out / "formulas/index.html").read_text()
        check(page.count("<!-- gallery:begin -->") == 1 and page.count("<!-- gallery:end -->") == 1
              and all(page.count(f"<!-- gallery:{p}{e} -->") == 1 for p in ("community", "examples") for e in ("", "-end")),
              "the page has its markers (and its two lists', which the site's Worker rewrites from its store)")
        data = json.loads((out / "formulas/gallery.json").read_text())
        n_ex = len(list(site_py.EXAMPLES.glob("*.psi")))
        check(len(data["examples"]) == n_ex and {e["slug"] for e in data["contributions"]} == {"spiral-sweep", "rainbow-rings"}
              and data["contributions"][0]["file"] == (g1 / f"{data['contributions'][0]['slug']}.psi").read_text()
              and all(e["file"].endswith(e["src"]) for e in data["examples"]),
              f"formulas/gallery.json: the {n_ex} examples and the contributions, each with its file (the Worker seeds its store from it)")
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

        # 3. The order and the dates.
        gp = site_py.gallery_page
        added = json.loads((site_py.EXAMPLES / gp.ADDED).read_text())
        ex = [e for e in data["examples"]]
        want = sorted(sorted(ex, key=lambda e: (e["name"].casefold(), e["slug"])), key=lambda e: added[e["slug"]], reverse=True)
        tiles = re.findall(r'<li class="ftile"([^>]*)><a class="fpic" href="\.\./f/#gallery=([a-z0-9-]+)"', page.split("<!-- gallery:examples -->")[1])
        check([t[1] for t in tiles] == [e["slug"] for e in want] and all(f'data-added="{gp.iso(added[t[1]])}"' in t[0] for t in tiles)
              and all(e["added"] == gp.iso(added[e["slug"]]) for e in ex),
              "the examples newest first by added.json, each tile and gallery.json entry dated")
        marks = {t[1]: t[0] for t in tiles}
        e_look = next(e for e in ex if e["look"])
        e_ctl = next(e for e in ex if e["controls"] and not e["bundle"])
        e_panel = next(e for e in ex if e["bundle"])
        check('data-look="1"' in marks[e_look["slug"]] and 'data-controls="1"' in marks[e_ctl["slug"]] and 'data-panel="1"' not in marks[e_ctl["slug"]]
              and 'data-panel="1"' in marks[e_panel["slug"]], "each tile marked as a look, with controls, on the panel, as it is")
        check('<form class="fsort" id="fsort"' in page and " hidden>" in page.split('id="fsort"')[1][:80]
              and page.index('id="fsort"') < page.index("<!-- gallery:community -->"),
              "the sort and filter form: above the lists, hidden until its script shows it")
        repo = tmp / "repo"
        (repo / "gallery").mkdir(parents=True)
        sh("git", "init", "-q", "-b", "main", repo)
        for name, text, when in (("rainbow-rings.psi", RINGS, "2026-10-01T10:00:00Z"), ("spiral-sweep.psi", SPIRAL, "2026-10-05T12:30:00+02:00")):
            (repo / "gallery" / name).write_text(text)
            sh("git", "-C", repo, "add", "gallery")
            sh("git", "-C", repo, "commit", "-q", "-m", name, env={"GIT_COMMITTER_DATE": when, "GIT_AUTHOR_DATE": when})
        (repo / "gallery/rainbow-rings.psi").write_text(RINGS.replace("t/2", "t/3"))  # a later change: the first commit still counts
        sh("git", "-C", repo, "commit", "-q", "-am", "change", env={"GIT_COMMITTER_DATE": "2026-10-09T00:00:00Z"})
        out, log = build("site-3", repo / "gallery")
        co = re.findall(r'<li class="ftile"([^>]*)><a class="fpic" href="\.\./f/#gallery=([a-z0-9-]+)"',
                        (out / "formulas/index.html").read_text().split("<!-- gallery:community -->")[1].split("<!-- gallery:community-end -->")[0])
        check([c[1] for c in co] == ["spiral-sweep", "rainbow-rings"] and 'data-added="2026-10-05T10:30:00Z"' in co[0][0]
              and 'data-added="2026-10-01T10:00:00Z"' in co[1][0], f"contributions dated by their first commit, newest first {co}", log)
        shallow = tmp / "shallow"
        sh("git", "clone", "-q", "--depth", "1", f"file://{repo}", shallow)
        out, log = build("site-4", shallow / "gallery")
        co = re.findall(r'<li class="ftile"([^>]*)><a class="fpic" href="\.\./f/#gallery=([a-z0-9-]+)"',
                        (out / "formulas/index.html").read_text().split("<!-- gallery:community -->")[1].split("<!-- gallery:community-end -->")[0])
        check([c[1] for c in co] == ["rainbow-rings", "spiral-sweep"] and not any("data-added" in c[0] for c in co),
              f"a shallow clone: contributions undated, by name {co}", log)
        for what, change in (("an example with no date", lambda d: d.pop(next(iter(d)))), ("a date with no example", lambda d: d.update({"no-such-formula": "2026-11-01T00:00:00Z"}))):
            exdir = tmp / "examples-bad"
            shutil.rmtree(exdir, ignore_errors=True)
            shutil.copytree(site_py.EXAMPLES, exdir)
            d = dict(added)
            change(d)
            (exdir / gp.ADDED).write_text(json.dumps(d))
            try:
                site_py.write_gallery(tmp / "site-bad", exdir, None, tool, wm)
                stopped = ""
            except SystemExit as e:
                stopped = str(e)
            check("every example needs its date" in stopped, f"{what}: the build stops ({stopped[:120]})")
        # A tile says what a Formula costs and, when its compiled format is newer than the latest release plays, which firmware it needs.
        e = {"us": 4200, "vm_us": 6500, "abi": 1}
        c = gp.cost(e)
        check("Light · about 4.2 ms a frame" in c and "6.5 ms without the JIT" in c and gp.needs(e) == "", "a tile: the tier's figure, the VM's in the hover text; nothing about firmware for a format the latest release plays")
        e = {"us": 8100, "vm_us": 8100, "abi": 2}
        check("Heavy · about 8.1 ms a frame" in gp.cost(e) and "without the JIT" not in gp.cost(e), "a tile: Heavy over 8 ms; no 'without the JIT' where the JIT does not take it")
        newer = gp.ABI_SERVED + 1
        want = f'<p class="fneeds">Needs firmware {gp.ABI_FIRMWARE[newer]} or later</p>' if newer in gp.ABI_FIRMWARE else '<p class="fneeds">Needs newer firmware</p>'
        check(gp.needs(e | {"abi": gp.ABI_SERVED}) == "" and gp.needs(e | {"abi": newer}) == want, f"a tile: a format newer than the latest release plays says which firmware it needs ({want})")
        check(gp.needs({"abi": 3}) == '<p class="fneeds">Needs newer firmware</p>', "a tile: a format no release is named for says so")
        check(gp.cost({"us": 1}) == "" and gp.needs({}) == "", "a tile: a row stored before the migration says nothing")
    finally:
        if not a.keep:
            shutil.rmtree(tmp, ignore_errors=True)
    print(f"gallery-pages-test: {'ok' if not failures else f'{failures} failures'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
