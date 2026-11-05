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
import datetime
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
# Beside OpenDMD's examples: when each was added to OpenDMD ({"<stem>":
# "<ISO time, UTC>"}), checked in (a date git can't be trusted to keep: a
# move or a rewrite of the files would reset it). Every example has one.
ADDED = "added.json"
# Beside it: the entries as data (`write_json`).
JSON = "gallery.json"
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


# The compiled format's version the newest release the site serves plays, and
# the first release of each version (site/worker/src/gallery.ts has a copy).
# Change both when a release ships a new ABI: ABI 3 is in 1.0.0-beta.15.
ABI_SERVED = 3
ABI_FIRMWARE = {1: "1.0.0-beta.13", 2: "1.0.0-beta.14", 3: "1.0.0-beta.15"}


def cost(e: dict) -> str:
    """What a Formula costs on the panel, as the web app says it (light:
    under 5 ms, medium: under 8, else heavy; `us` is the tier figure,
    the JIT's where it takes the Formula, and `vm_us` the VM's beside it)."""
    us, vm = e.get("us"), e.get("vm_us")
    if us is None or vm is None:
        return ""
    tier = "Light" if us <= 5000 else "Medium" if us <= 8000 else "Heavy"
    without = f", {vm / 1000:.1f} ms without the JIT" if vm > us else ""
    tip = f"About {us / 1000:.1f} ms a frame on the panel{without} (light: under 5 ms; medium: under 8; the gallery takes up to 10)"
    return f'<p class="fcost" title="{html.escape(tip)}">{tier} · about {us / 1000:.1f} ms a frame</p>'


def needs(e: dict) -> str:
    """The firmware a Formula needs, when its compiled format is newer than
    the one the latest release plays (ABI_SERVED)."""
    abi = e.get("abi")
    if abi is None or abi <= ABI_SERVED:
        return ""
    v = ABI_FIRMWARE.get(abi)
    return f'<p class="fneeds">{"Needs firmware " + html.escape(v) + " or later" if v else "Needs newer firmware"}</p>'


def share_hash(e: dict) -> str:
    """`#gallery=<slug>`: a Formula goes by name, never by its text in a
    link (the web app reads the same: psikit `from`). /f/ reads it from
    formulas/src/, a panel fetches it from there (or has it built in)."""
    return "#" + urlencoded([("gallery", e["slug"])])


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


def iso(t: str | int) -> str:
    """A time (ISO 8601, or unix seconds) as the page writes it: UTC to the
    second, `YYYY-MM-DDTHH:MM:SSZ` (the Worker's `iso` writes the same)."""
    d = (datetime.datetime.fromtimestamp(t, datetime.timezone.utc) if isinstance(t, int)
         else datetime.datetime.fromisoformat(t.replace("Z", "+00:00")))
    if d.tzinfo is None:
        raise ValueError(f"{t!r}: no time zone")
    return d.astimezone(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load_added(folder: Path) -> dict[str, str]:
    """`folder`/added.json, checked against the examples in `folder`: one
    date for every .psi file and none for a file that isn't there (a new
    example has to be dated when it is added)."""
    path = folder / ADDED
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
        dates = {k: iso(v) for k, v in raw.items()}
    except (OSError, ValueError, AttributeError, TypeError) as e:
        raise SystemExit(f"{path}: {e}")
    stems = {p.stem for p in folder.glob("*.psi")}
    if set(dates) != stems:
        raise SystemExit(f"{path}: every example needs its date, and only they"
                         f" (no date: {sorted(stems - set(dates))}; no such file: {sorted(set(dates) - stems)})")
    return dates


def git_added(contrib: Path) -> dict[str, str]:
    """When each file in `contrib` was first committed, by git's commit
    time, if `contrib` is in a git checkout with its whole history; nothing
    for a shallow clone (its oldest commit would look like every file's
    first) or no checkout. Trees only (no rename detection), so a
    blob-less clone fetches nothing."""
    def git(*a: str) -> subprocess.CompletedProcess:
        return subprocess.run(["git", "-C", str(contrib), *a], capture_output=True, text=True)
    r = git("rev-parse", "--is-shallow-repository")
    if r.returncode or r.stdout.strip() != "false":
        return {}
    r = git("log", "--no-renames", "--diff-filter=A", "--relative", "--name-only", "--format=@%ct", "--", ".")
    if r.returncode:
        return {}
    out: dict[str, str] = {}
    t = 0
    for line in r.stdout.splitlines():
        if line.startswith("@"):
            t = int(line[1:])
        elif line.endswith(".psi") and "/" not in line:
            out[line[:-4]] = iso(t)  # newest first: the last seen is the first commit
    return out


def order(entries: list[dict]) -> list[dict]:
    """Newest first (by `added`), then by name; the undated last, by name.
    The page's default order (and ftsort.js's `new`)."""
    by_name = sorted(entries, key=lambda e: (e["name"].casefold(), e["slug"]))
    dated = sorted((e for e in by_name if e.get("added")), key=lambda e: e["added"], reverse=True)
    return dated + [e for e in by_name if not e.get("added")]


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


def build(formulas: Path, examples: Path, contrib: Path | None, tool: Path, added: dict[str, str],
          contrib_added: dict[str, str] | None = None) -> str:
    """Writes the thumbnails under `formulas` (formulas/examples/ and
    formulas/thumbs/, emptied first) and returns the page's body: the
    contributions in `contrib` (checked again here, so a file that should
    not have been merged is left out, not published) and OpenDMD's examples
    (the `.psi` files in `examples`, dated by `added`: `load_added`), with
    thumbnails drawn by the checker. The contributions are dated by
    `contrib_added` (slug: ISO time) or else by git (`git_added`); an
    undated one comes last. Both lists newest first."""
    for d in ("examples", "thumbs"):
        shutil.rmtree(formulas / d, ignore_errors=True)
    files = sorted(p for p in examples.glob("*.psi") if not p.is_symlink() and p.is_file())
    seeds, bad = check_gallery(tool, files, formulas / "examples", trusted=True)
    if bad or not files or len(seeds) != len(files):
        raise SystemExit(f"an example fails the gallery's checks: {bad}")
    missing = sorted({e["slug"] for e in seeds} - set(added))
    if missing:
        raise SystemExit(f"{ADDED}: no date for {missing}")
    seeds = order([{**e, "added": added[e["slug"]]} for e in seeds])
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
        dates = git_added(contrib) if contrib_added is None else {k: iso(v) for k, v in contrib_added.items()}
        entries = order([{**e, "added": dates.get(e["slug"])} for e in entries if e not in dup])
    write_json(formulas, seeds, examples, entries, contrib)

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
        # What ftsort.js sorts and filters by (its own copy of the order and the badges).
        data = (f' data-added="{t(e["added"])}"' if e.get("added") else "") + "".join(
            f' data-{k}="1"' for k, on in (("look", e.get("look")), ("panel", e.get("bundle")), ("controls", ctl)) if on)
        return (f'<li class="ftile"{data}><a class="fpic" href="../f/{t(share_hash(e))}" tabindex="-1" aria-hidden="true">'
                f'<picture class="led"><img src="{t(pic)}" alt="" width="128" height="32" loading="lazy" decoding="async"></picture></a>'
                f'<h3>{t(name)}{badges}</h3><p class="fdesc">{t(e["about"])}</p>'
                + (f'<p class="fctl">Controls: {t(", ".join(ctl))}</p>' if ctl else "")
                + cost(e) + needs(e)
                + f'<p class="fby">by {t(e["author"])} · <a href="{url}" rel="license">{lic}</a></p>'
                # Open on my panel: the Formula's page with the mark, which hands it to
                # the panel's editor at once once its address is known (else asks for it).
                f'<p class="facts"><a class="cta add" href="../f/{t(share_hash(e))}&amp;panel=1">Open on my panel</a>'
                f'<a class="cta alt" href="../f/{t(share_hash(e))}">Open</a></p></li>')
    community = "".join(tile(e, f"thumbs/{e['slug']}.png") for e in entries)
    ex = "".join(tile(e, f"examples/{e['slug']}.png") for e in seeds)
    how = (f'<a href="{REPO}/tree/main/gallery">how to add yours</a>')
    return (f'<h1>Formula gallery</h1>\n<p class="lead">Formulas are little animations and looks for your display, each '
            f'drawn by a few lines of maths. Find one you like and press <strong>Open on my panel</strong>: it opens in your '
            f'panel\'s editor, where you can change it, save it, play it, put it in a playlist or lay it over your GIFs as the look.</p>\n'
            f'<p><strong>Open</strong> plays it here in the editor, with its sliders and its code, ready to change. Made one you '
            f'like? Its page has <strong>Submit to the gallery</strong>, or read {how}.</p>\n'
            + CONTROLS
            + f'<section class="fsec" data-part="community">\n<h2 id="community">From the community</h2>\n'
            + listed("community", f'<ul class="ftiles">{community}</ul>\n' if community else EMPTY)
            + f'</section>\n<section class="fsec" data-part="examples">\n<h2 id="examples">Examples</h2>\n'
            f'<p class="tiles-note">By OpenDMD (CC0). The ones marked <em>on the panel</em> come with the panel.</p>\n'
            + listed("examples", f'<ul class="ftiles">{ex}</ul>\n') + '</section>\n'
            + NONE)


# Sorting and filtering the tiles (ftsort.js, which shows it: without a
# script the page is newest first, as built). Its state is the address's
# query (?q=…&sort=old|name&show=community|panel|looks&controls=1).
CONTROLS = ('<form class="fsort" id="fsort" role="search" aria-label="Sort and filter the Formulas" hidden>\n'
            '<label class="fs-q"><span>Search</span><input type="search" name="q" autocomplete="off" '
            'placeholder="Name, what it does, who made it"></label>\n'
            '<label><span>Sort</span><select name="sort"><option value="new">Newest</option>'
            '<option value="old">Oldest</option><option value="name">Name A–Z</option></select></label>\n'
            '<label><span>Show</span><select name="show"><option value="all">All</option>'
            '<option value="community">From the community</option><option value="panel">On the panel</option>'
            '<option value="looks">Looks</option></select></label>\n'
            '<label class="fs-ctl"><input type="checkbox" name="controls" value="1"> Has controls</label>\n'
            '<output class="fs-count" name="count" aria-live="polite"></output>\n'
            '</form>\n')
# Shown when nothing matches.
NONE = '<p class="fs-none" hidden>No Formulas match. <button type="button" class="cta alt">Show all</button></p>\n'

# The community list when it is empty.
EMPTY = ('<p class="tiles-note">Nothing here yet: be the first. Make a formula in the <a href="../demo/#formulas">demo</a>, '
         'then Submit to the gallery from its editor.</p>\n')


def listed(part: str, inner: str) -> str:
    """A list between its own markers: the site's Worker writes the two lists
    (`community`, `examples`) again from its gallery store (site/worker/src/
    gallery.ts `page`, the same tiles: site-worker-test holds them equal)
    and keeps the rest of the page as built."""
    return f"<!-- gallery:{part} -->{inner}<!-- gallery:{part}-end -->\n"


def write_json(formulas: Path, seeds: list[dict], examples: Path, entries: list[dict], contrib: Path | None) -> None:
    """formulas/gallery.json: every entry on the page as the checker read
    it, with its whole file (`file`). The site's Worker seeds its gallery
    store (D1) with the examples from it at each deploy, and serves the
    contributions it syncs from the public repo itself (site/worker/src/
    gallery.ts); site-worker-test compares the two."""
    def one(e: dict, d: Path) -> dict:
        # (`added`: an example's from added.json, a contribution's from git or null.)
        return {**e, "file": (d / f"{e['slug']}.psi").read_text(encoding="utf-8")}
    data = {"examples": [one(e, examples) for e in seeds],
            "contributions": [one(e, contrib) for e in entries] if contrib is not None else []}
    (formulas / JSON).write_text(json.dumps(data, ensure_ascii=False, separators=(",", ":")) + "\n", encoding="utf-8")


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
    ap.add_argument("--added", type=Path, default=Path(__file__).resolve().parents[1] / "opendmd-web/formulas",
                    help=f"the folder whose {ADDED} dates the examples (default: this checkout's opendmd-web/formulas)")
    a = ap.parse_args()
    page = a.site / PAGE
    if page.is_symlink() or not page.is_file():
        raise SystemExit(f"no {page}")
    text = page.read_text(encoding="utf-8")
    splice(text, "")  # the markers are there, before anything is rewritten
    body = build(page.parent, page.parent / "src", a.gallery, a.tool, load_added(a.added))
    page.write_text(splice(text, body), encoding="utf-8")
    n, ex = body.count('src="thumbs/'), body.count('src="examples/')
    print(f"{PAGE}: {n} contribution{'s' * (n != 1)}, {ex} examples")
    return 0


if __name__ == "__main__":
    sys.exit(main())
