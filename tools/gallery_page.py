"""The formula gallery's page (/formulas/ on the site): its thumbnails and
the part of formulas/index.html between the gallery markers.

tools/site.py imports it (`build`, `marked`) and writes the page around
it. It also runs by hand over a built site, rewriting its gallery in place
from the site's formulas/src/*.psi (the examples) and DIR/*.psi (the
contributions, a checkout of the public gallery repo's gallery/):

    python3 -I gallery_page.py SITE --tool opendmd-gallery [--gallery DIR]

The Python standard library only (tools/gallery-pages-test.py checks the
page it writes).

Every entry's name, line about it, controls and look come from the checker
(`opendmd-gallery check --json`), which reads them from the formula's own
`name "…"`, `about "…"`, `control` and `look` lines with the compiler; the
file's header holds only `-- author:` and `-- licence:` (and `-- bundle` on
OpenDMD's examples, opendmd-web/formulas/, which the site build copies to
SITE/formulas/src/).
"""

from __future__ import annotations

import argparse
import html
import json
import re
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

PAGE = "formulas/index.html"
REPO = "https://github.com/opendmd/opendmd.github.io"
LICENCE_LINKS = {"CC0-1.0": ("CC0", "https://creativecommons.org/publicdomain/zero/1.0/"),
                 "CC-BY-4.0": ("CC BY 4.0", "https://creativecommons.org/licenses/by/4.0/")}
# More files than this in gallery/ and the rest are left out (a sanity limit).
MAX = 2000
# The generated part of formulas/index.html. Nothing inside can contain
# either marker: every field is escaped (no `<`), and the checker refuses
# `<` and `>` in a header anyway.
BEGIN, END = "<!-- gallery:begin -->", "<!-- gallery:end -->"


def urlencoded(pairs: list[tuple[str, str]]) -> str:
    """application/x-www-form-urlencoded exactly as the browser's
    URLSearchParams writes it (alphanumerics and *-._ kept, space as +), so
    the gallery's links are the web app's (js/psikit.js `link`)."""
    def enc(v: str) -> str:
        out = []
        for b in v.encode():
            c = chr(b)
            out.append(c if (c.isascii() and c.isalnum()) or c in "*-._" else "+" if c == " " else f"%{b:02X}")
        return "".join(out)
    return "&".join(f"{enc(k)}={enc(v)}" for k, v in pairs)


# A formula's own `name "…"` line (the declarations come first, unindented).
NAME_LINE = re.compile(r'^name "[^"\n]*"', re.M)


def share_hash(e: dict, name: str | None = None) -> str:
    """`#formula=…&name=…`, as the web app's `link` writes it for a formula
    with its controls at their defaults (which it leaves out). With `name`
    (a remix), the formula's `name` line says it too. A bundled one (not a
    remix) goes by name, `#bundled=<slug>`, as the web app's link writes it
    too: /f/ reads it from formulas/src/, a panel has it built in."""
    if name is None and e.get("bundle"):
        return "#" + urlencoded([("bundled", e["slug"])])
    src = e["src"]
    if name is not None:
        src = NAME_LINE.sub(lambda _: f'name "{name}"', src, count=1)
    return "#" + urlencoded([("formula", src), ("name", name if name is not None else e["name"])])


def recompress_png(data: bytes) -> bytes:
    """opendmd-gallery writes stored (uncompressed) PNGs: the same image,
    deflated."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    chunks, idat, i = [], b"", 8
    while i < len(data):
        n, kind = struct.unpack(">I4s", data[i:i + 8])
        body = data[i + 8:i + 8 + n]
        i += 12 + n
        if kind == b"IDAT":
            idat += body
        else:
            chunks.append((kind, body))
    out = b"\x89PNG\r\n\x1a\n"
    for kind, body in chunks:
        if kind == b"IEND":
            z = zlib.compress(zlib.decompress(idat), 9)
            out += struct.pack(">I", len(z)) + b"IDAT" + z + struct.pack(">I", zlib.crc32(b"IDAT" + z))
        out += struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    return out


def say(msg: str) -> None:
    """A line on stderr, on one line whatever a file name holds."""
    print(msg.replace("\r", " ").replace("\n", " "), file=sys.stderr)


def check_gallery(tool: Path, files: list[Path], thumbs: Path, trusted: bool = False) -> tuple[list[dict], list[dict]]:
    """opendmd-gallery check over `files`: (entries, failures). If the
    checker itself fails on a batch (not a verdict: a crash), the files are
    checked one at a time and the one it fails on is left out, so a bad
    contribution can't stop a release (or a deploy)."""
    thumbs.mkdir(parents=True, exist_ok=True)
    good, bad = [], []

    def run(batch: list[Path]) -> bool:
        r = subprocess.run([str(tool), "check", "--json", *(["--trusted"] if trusted else []), "--thumbs", str(thumbs),
                            *map(str, batch)], capture_output=True, text=True)
        if r.returncode not in (0, 1):
            return False
        for line in r.stdout.splitlines():
            e = json.loads(line)
            (bad if "problems" in e else good).append(e)
        return True

    for k in range(0, len(files), 200):
        batch = files[k:k + 200]
        if not run(batch):
            for f in batch:
                if not run([f]):
                    bad.append({"file": f.name, "problems": ["the checker failed on it"]})
    for p in sorted(thumbs.glob("*.png")):
        p.write_bytes(recompress_png(p.read_bytes()))
    return good, bad


def build(formulas: Path, examples: Path, contrib: Path | None, tool: Path) -> str:
    """Writes the thumbnails under `formulas` (formulas/examples/ and
    formulas/thumbs/, emptied first) and returns the page's body: the
    contributions in `contrib` (checked again here, so a file that should
    not have been merged is left out, not published) and OpenDMD's examples
    (the `.psi` files in `examples`), with thumbnails drawn by the checker."""
    for d in ("examples", "thumbs"):
        shutil.rmtree(formulas / d, ignore_errors=True)
    files = sorted(p for p in examples.glob("*.psi") if not p.is_symlink() and p.is_file())
    seeds, bad = check_gallery(tool, files, formulas / "examples", trusted=True)
    if bad or not files or len(seeds) != len(files):
        raise SystemExit(f"an example fails the gallery's checks: {bad}")
    seeds = sorted(seeds, key=lambda e: (e["name"].casefold(), e["slug"]))
    entries: list[dict] = []
    if contrib is not None:
        if not contrib.is_dir():
            raise SystemExit(f"--gallery {contrib}: not a folder")
        # Regular files only: a link is never followed (lstat, not stat).
        listing = sorted(contrib.iterdir())
        files = [p for p in listing if p.suffix == ".psi" and not p.is_symlink() and p.is_file()][:MAX]
        for p in listing:
            if p.suffix == ".psi" and (p.is_symlink() or not p.is_file()):
                say(f"gallery: left out {p.name!r}: not a plain file (a link or a folder)")
        entries, bad = check_gallery(tool, files, formulas / "thumbs")
        for b in bad:
            say(f"gallery: left out {b['file']!r}: {'; '.join(b['problems'])}")
        known = {e["src"].strip() for e in seeds}
        dup = [e for e in entries if e["src"].strip() in known]
        for e in dup:
            say(f"gallery: left out {e['slug']!r}: the same formula as an example")
        entries = sorted((e for e in entries if e not in dup), key=lambda e: (e["name"].casefold(), e["slug"]))

    def tile(e: dict, pic: str) -> str:
        t = html.escape
        lic, url = LICENCE_LINKS[e["licence"]]
        name = e["name"]
        # What the formula says of itself: whether it can be the panel's look
        # (its `look` line), whether the panel comes with it, its controls.
        badges = "".join(f'<span class="fbadge" title="{t(why)}">{t(b)}</span>' for b, why, on in (
            ("look", "It can be the panel's look: drawn over every GIF (Settings › Look)", e.get("look")),
            ("on the panel", "The web app comes with it: in Formulas, under Bundled", e.get("bundle"))) if on)
        ctl = e.get("controls") or []
        return (f'<li class="ftile"><a class="fpic" href="../f/{t(share_hash(e))}" tabindex="-1" aria-hidden="true">'
                f'<picture class="led"><img src="{t(pic)}" alt="" width="128" height="32" loading="lazy" decoding="async"></picture></a>'
                f'<h3>{t(name)}{badges}</h3><p class="fdesc">{t(e["about"])}</p>'
                + (f'<p class="fctl">Controls: {t(", ".join(ctl))}</p>' if ctl else "")
                + f'<p class="fby">by {t(e["author"])} · <a href="{url}" rel="license">{lic}</a></p>'
                f'<p class="facts"><a class="cta" href="../f/{t(share_hash(e))}">Open</a>'
                f'<a class="cta alt" href="../demo/{t(share_hash(e, (name + " remix")[:60]))}">Remix</a></p></li>')
    community = "".join(tile(e, f"thumbs/{e['slug']}.png") for e in entries)
    ex = "".join(tile(e, f"examples/{e['slug']}.png") for e in seeds)
    how = (f'<a href="{REPO}/tree/main/gallery">how to add yours</a>')
    return (f'<h1>Formula gallery</h1>\n<p class="lead">Formulas are a few lines of maths that draw on the display, every pixel '
            f'at once. Open one to watch it, change its sliders and send it to your display; Remix opens it in the demo\'s '
            f'editor. Made one you like? Its page has <strong>Submit to the gallery</strong>, or read {how}.</p>\n'
            f'<h2 id="community">From the community</h2>\n'
            + (f'<ul class="ftiles">{community}</ul>\n' if community else
               '<p class="tiles-note">Nothing here yet: be the first. Make a formula in the <a href="../demo/#formulas">demo</a>, '
               'then Copy share link, and on its page, Submit to the gallery.</p>\n')
            + f'<h2 id="examples">Examples</h2>\n<p class="tiles-note">By OpenDMD (CC0). The ones marked <em>on the panel</em> come with '
            f'the web app; Open any of the others and send it to your display from its page.</p>\n'
            f'<ul class="ftiles">{ex}</ul>\n')


def marked(body: str) -> str:
    """The body as the site build writes it into the page: between the markers."""
    return f"{BEGIN}\n{body}{END}"


def splice(page: str, body: str) -> str:
    """The page with what is between its markers replaced by `body`."""
    if page.count(BEGIN) != 1 or page.count(END) != 1 or page.index(BEGIN) > page.index(END):
        raise SystemExit(f"{PAGE}: not one pair of gallery markers (a page from an older site build?)")
    head, rest = page.split(BEGIN)
    _, tail = rest.split(END)
    return head + marked(body) + tail


def main() -> int:
    ap = argparse.ArgumentParser(description="Rewrite a built site's formula gallery (formulas/) in place.")
    ap.add_argument("site", type=Path, help="the built site (its formulas/src/*.psi and formulas/index.html)")
    ap.add_argument("--tool", type=Path, required=True, help="opendmd-gallery, built")
    ap.add_argument("--gallery", type=Path, help="the contributions (the public gallery repo's gallery/)")
    a = ap.parse_args()
    page = a.site / PAGE
    if page.is_symlink() or not page.is_file():
        raise SystemExit(f"no {page}")
    text = page.read_text(encoding="utf-8")
    splice(text, "")  # the markers are there, before anything is rewritten
    body = build(page.parent, page.parent / "src", a.gallery, a.tool)
    page.write_text(splice(text, body), encoding="utf-8")
    n, ex = body.count('src="thumbs/'), body.count('src="examples/')
    print(f"{PAGE}: {n} contribution{'s' * (n != 1)}, {ex} examples")
    return 0


if __name__ == "__main__":
    sys.exit(main())
