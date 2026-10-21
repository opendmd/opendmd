# /// script
# requires-python = ">=3.11"
# dependencies = ["markdown>=3.6"]
# ///
"""Build the public site (opendmd.github.io) as static files.

    uv run tools/site.py OUT_DIR --www opendmd-web/www --card target/demo-card \
        --art target/site-art --dist dist

- /            the README, as the landing page
- /install/    the browser installer (docs/guide/install.md with ESP Web
               Tools' button) and, under install/firmware/, the release in
               dist/: the parts dist/manifest.json names, the merged image
               and the signed update bundle, plus install/release.json
               (what the web app's update check reads)
- /guide/      the user guide (docs/guide/)
- /dev/        the developer pages: an overview and a few clean references
               (the detailed notes in docs/ stay in the repo)
- /demo/       the web app with the demo card (tools/demo-card.py) beside it:
               with no player answering, it loads demo/index.json and plays
               that card in the browser (js/source.js, demoCard)
- /art/        the landing page's tiles and the wordmark (tools/site-art.py: recordings of the
               panel's own effects and live items, and CC0 sprites), drawn
               as LEDs by CSS

The docs mark site-only pieces with HTML comments, invisible on GitHub:
`<!-- site:hero -->` ends the README's GitHub header (the site draws its
own), `<!-- site:tiles -->` places the landing page's tiles, and
`site:features`, `site:steps` and `site:cards` ... `site:cards-end` style
the list or the h3 sections after them as cards or numbered steps.

Links between published pages are rewritten to the HTML pages; a link to
anything not published (CLAUDE.md, source files) keeps its text and loses
the link. `make site` runs this; `make site-publish` pushes it.
"""

import argparse
import html
import json
import posixpath
import re
import shutil
from pathlib import Path

import markdown

ROOT = Path(__file__).resolve().parent.parent

# Source (repo path) -> published path, in the order of the docs menu.
PAGES = {
    "README.md": "index.html",
    "docs/guide/hardware.md": "guide/hardware.html",
    "docs/guide/install.md": "install/index.html",
    "docs/guide/web-app.md": "guide/web-app.html",
    "docs/guide/create-art.md": "guide/create-art.html",
    "docs/guide/effects.md": "guide/effects.html",
    "docs/guide/card.md": "guide/card.html",
    "docs/guide/updating.md": "guide/updating.html",
    "docs/guide/troubleshooting.md": "guide/troubleshooting.html",
    "docs/developers.md": "dev/index.html",
    "docs/api.md": "dev/api.html",
    "docs/simulator.md": "dev/simulator.html",
    "docs/writing-effects.md": "dev/effects.html",
    "hardware/PINOUT.md": "dev/pinout.html",
}
# Pages that moved: old published path -> new one, kept as redirects.
MOVED = {
    "docs/web-app.html": "guide/web-app.html",
    "docs/card.html": "guide/card.html",
    "docs/deploying.html": "install/index.html",
    "docs/ota.html": "guide/updating.html",
    "docs/firmware.html": "dev/index.html",
    "docs/api.html": "dev/api.html",
    "docs/simulator.html": "dev/simulator.html",
    "docs/pinout.html": "dev/pinout.html",
    "docs/cache-format.html": "dev/index.html",
    "docs/www-pack.html": "dev/index.html",
    "docs/gifslim-experiment.html": "index.html",
    "docs/tweening-experiment.html": "index.html",
    "docs/art-pipeline.html": "index.html",
}
MENU = [
    ("Overview", "index.html"),
    ("Try the demo", "demo/"),
    (None, "Guide"),
    ("What you need", "guide/hardware.html"),
    ("Install", "install/index.html"),
    ("The web app", "guide/web-app.html"),
    ("Create Art", "guide/create-art.html"),
    ("Effects and live items", "guide/effects.html"),
    ("The SD card", "guide/card.html"),
    ("Updating", "guide/updating.html"),
    ("Troubleshooting", "guide/troubleshooting.html"),
    (None, "For developers"),
    ("Building from source", "dev/index.html"),
    ("HTTP API", "dev/api.html"),
    ("The simulator", "dev/simulator.html"),
    ("Writing an effect", "dev/effects.html"),
    ("Hardware pinout", "dev/pinout.html"),
]
# The installer page: ESP Web Tools, pinned (the flashing itself is
# esptool-js over Web Serial; Chrome, Edge, Opera or Firefox 151+, https or localhost).
INSTALL_PAGE = "install/index.html"
INSTALL_HEAD = ('<script type="module" src="https://cdn.jsdelivr.net/npm/esp-web-tools@10.4.0/dist/web/'
                'install-button.js?module"></script>')
INSTALL_BUTTON = """<div class="install">
<esp-web-install-button id="install" manifest="firmware/manifest.json">
<button slot="activate" id="install-activate">Install</button>
<span slot="unsupported">This browser can't talk to USB boards. Open this page in Chrome, Edge, Opera or Firefox on a computer.</span>
<span slot="not-allowed">This page has to be opened over https.</span>
</esp-web-install-button>
<span class="ver">OpenDMD {version}</span>
</div>"""
# Other files published as they are: repo path -> published path.
FILES = {
    "LICENSE-MIT": "LICENSE-MIT.txt",
    "LICENSE-APACHE": "LICENSE-APACHE.txt",
}

CSS = """
:root{--bg:#0d0e11;--bg2:#15171b;--bg3:#1e2025;--line:#2a2d33;--fg:#ecebe7;--mut:#a0a2a9;
--acc:#f7a531;--pink:#ff4f8b;--cyan:#33d1ff;--green:#5ef09a;--violet:#a98bff;color-scheme:dark}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.65 system-ui,-apple-system,"Segoe UI",sans-serif}
a{color:var(--acc)}
a:focus-visible,button:focus-visible{outline:2px solid var(--cyan);outline-offset:2px}
.skip{position:absolute;left:-9999px}.skip:focus{left:12px;top:12px;z-index:9;background:var(--bg3);padding:6px 10px;border-radius:6px}

/* LEDs: a 128x32 picture scaled up crisply, with a dot mask over it */
/* (the unlit LEDs show through the picture's black: it is blended "lighten" over a grid of dim dots) */
.led{position:relative;display:block;line-height:0;isolation:isolate;--cols:128;--rows:32;
background:#000 radial-gradient(closest-side,#1d1d20 50%,transparent 75%) 0 0/calc(100% / var(--cols)) calc(100% / var(--rows))}
.led img{display:block;width:100%;height:auto;image-rendering:pixelated;image-rendering:crisp-edges;border-radius:0;mix-blend-mode:lighten}
.led::after{content:"";position:absolute;inset:0;pointer-events:none;
background:radial-gradient(closest-side,transparent 62%,rgba(0,0,0,.72) 100%);
background-size:calc(100% / var(--cols)) calc(100% / var(--rows))}

.banner{padding:9px 16px;text-align:center;font-size:14px;line-height:1.45;color:#111;background:var(--acc)}
.banner strong{font-weight:800}
.wrap{display:grid;grid-template-columns:248px minmax(0,1fr);max-width:1260px;margin:0 auto}
nav{position:sticky;top:0;align-self:start;max-height:100vh;overflow:auto;padding:24px 16px;border-right:1px solid var(--line)}
nav .brand{display:inline-block;margin:0 8px 18px;padding:8px 10px;border-radius:8px;background:#000;box-shadow:0 0 0 1px var(--line),0 0 24px rgba(247,165,49,.15)}
nav .brand .led{width:140px}
nav a{display:block;padding:5px 10px;border-radius:6px;color:var(--mut);text-decoration:none;font-size:14px}
nav a:hover{color:var(--fg);background:var(--bg3)}
nav a.on{color:var(--fg);background:var(--bg3);font-weight:600;box-shadow:inset 3px 0 0 var(--acc)}
nav a.demo{color:#111;background:var(--acc);font-weight:700;margin:4px 0 8px}
nav h4{margin:18px 10px 4px;font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:var(--mut)}
main{padding:36px 44px 72px;min-width:0}
main>*{max-width:860px}
h1,h2,h3{line-height:1.25;text-wrap:balance}
h1{font-size:36px;margin:4px 0 16px;letter-spacing:-.01em}
h2{font-size:24px;margin:44px 0 12px;padding-top:12px;border-top:1px solid var(--line)}
h3{font-size:18px;margin:28px 0 8px}
h1 .headerlink{display:none}
h2 .headerlink,h3 .headerlink{opacity:0;margin-left:6px;text-decoration:none;color:var(--mut)}
h2:hover .headerlink,h3:hover .headerlink{opacity:1}
code{font:14px ui-monospace,Menlo,Consolas,monospace;background:var(--bg3);padding:1px 5px;border-radius:4px;overflow-wrap:anywhere}
pre{background:var(--bg2);border:1px solid var(--line);border-radius:8px;padding:14px 16px;overflow:auto}
pre code{background:none;padding:0;font-size:13px;line-height:1.5;overflow-wrap:normal}
table{border-collapse:collapse;display:block;overflow:auto;margin:16px 0}
th,td{border:1px solid var(--line);padding:6px 10px;text-align:left;vertical-align:top}
th{background:var(--bg2)}
img{max-width:100%;height:auto;border-radius:6px}
blockquote{margin:20px 0;padding:12px 18px;border:1px solid var(--line);border-left:4px solid var(--cyan);border-radius:8px;background:var(--bg2);color:var(--fg)}
blockquote>:first-child{margin-top:0}blockquote>:last-child{margin-bottom:0}
blockquote ul{padding-left:20px}
hr{border:0;border-top:1px solid var(--line)}

/* The installer */
.install{display:flex;flex-wrap:wrap;align-items:center;gap:16px;margin:8px 0 24px;padding:20px;border:1px solid var(--line);border-radius:12px;
background:linear-gradient(135deg,rgba(247,165,49,.12),rgba(255,79,139,.08) 60%,transparent),var(--bg2)}
.install button{font:inherit;font-weight:700;padding:11px 30px;border-radius:8px;border:0;background:var(--acc);color:#111;cursor:pointer;box-shadow:0 0 22px rgba(247,165,49,.35)}
.install .ver{color:var(--mut);font:14px ui-monospace,Menlo,Consolas,monospace}

/* Buttons, the landing page */
.cta{display:inline-block;margin:6px 8px 6px 0;padding:10px 18px;border-radius:8px;background:var(--acc);color:#111;font-weight:700;text-decoration:none}
.cta.alt{background:none;color:var(--fg);box-shadow:inset 0 0 0 2px var(--line)}
.cta.alt:hover{box-shadow:inset 0 0 0 2px var(--acc)}
.hero{margin:0 0 28px}
.hero h1{margin:0 0 18px}
.hero .plate{display:inline-block;padding:18px 22px;border-radius:14px;background:#000;
box-shadow:0 0 0 1px var(--line),0 0 60px rgba(247,165,49,.18)}
.hero .plate .led{width:420px;max-width:100%}
.hero .tag{font-size:21px;line-height:1.45;margin:0 0 12px;max-width:640px}
.chip{display:inline-block;font-size:13px;font-weight:600;color:var(--cyan);border:1px solid rgba(51,209,255,.4);border-radius:999px;padding:2px 10px;margin:0 0 14px;text-decoration:none}
.tiles{display:grid;grid-template-columns:repeat(auto-fill,minmax(250px,1fr));gap:14px;margin:28px 0 8px;padding:0}
.tiles figure{margin:0;padding:8px 8px 6px;background:#000;border:1px solid var(--line);border-radius:10px}
.tiles figcaption{font-size:13px;color:var(--mut);line-height:1.3;padding:8px 2px 2px}
.tiles-note{font-size:13px;color:var(--mut)}

/* Cards: feature lists, the hardware page */
ul.features{list-style:none;padding:0;display:grid;grid-template-columns:repeat(auto-fill,minmax(240px,1fr));gap:12px;margin:18px 0}
ul.features li{margin:0;padding:14px 16px;background:var(--bg2);border:1px solid var(--line);border-top:3px solid var(--c,var(--acc));border-radius:10px;font-size:15px}
ul.features li>strong:first-child{display:block;color:var(--fg);font-size:16px;margin-bottom:2px}
.features li:nth-child(5n+2),.cards .card:nth-child(5n+2){--c:var(--pink)}
.features li:nth-child(5n+3),.cards .card:nth-child(5n+3){--c:var(--cyan)}
.features li:nth-child(5n+4),.cards .card:nth-child(5n+4){--c:var(--green)}
.features li:nth-child(5n+5),.cards .card:nth-child(5n+5){--c:var(--violet)}
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(300px,1fr));gap:16px;margin:24px 0}
.card{padding:18px 20px;background:var(--bg2);border:1px solid var(--line);border-radius:12px;--c:var(--acc)}
.card h3{display:flex;align-items:center;gap:10px;margin:0 0 10px}
.card .num{flex:none;display:grid;place-items:center;width:32px;height:32px;border-radius:50%;background:#000;color:var(--c);
font:800 15px ui-monospace,Menlo,Consolas,monospace;box-shadow:0 0 0 2px var(--c),0 0 14px var(--c)}
.card p{margin:0 0 10px;font-size:15px}
.card p.where{margin:12px 0 0;padding-top:10px;border-top:1px dashed var(--line);font-size:14px;color:var(--mut)}
.card p.where strong{color:var(--fg)}

/* Numbered steps */
ol.steps{list-style:none;padding:0;margin:18px 0;counter-reset:step}
ol.steps>li{counter-increment:step;position:relative;margin:12px 0;padding:14px 18px 14px 66px;background:var(--bg2);border:1px solid var(--line);border-radius:12px}
ol.steps>li::before{content:counter(step);position:absolute;left:16px;top:13px;width:34px;height:34px;display:grid;place-items:center;
border-radius:50%;background:var(--acc);color:#111;font-weight:800;box-shadow:0 0 16px rgba(247,165,49,.4)}
ol.steps>li>:first-child{margin-top:0}ol.steps>li>:last-child{margin-bottom:0}

footer{grid-column:2;padding:0 44px 40px;color:var(--mut);font-size:13px}
footer p{max-width:860px;border-top:1px solid var(--line);padding-top:16px}

@media (max-width:860px){
  .wrap{display:block}
  nav{position:static;max-height:none;border-right:0;border-bottom:1px solid var(--line);padding:16px}
  nav .brand{margin:0 0 12px}
  nav .links{display:flex;flex-wrap:wrap;gap:4px}nav h4{display:none}
  nav a.on{box-shadow:inset 0 -2px 0 var(--acc)}
  main{padding:22px 16px 56px}
  footer{padding:0 16px 32px}
  h1{font-size:28px}
  .hero .plate{padding:12px 14px}
  .hero .plate .led{width:280px}
  .hero .tag{font-size:18px}
  ol.steps>li{padding:12px 14px 12px 56px}
  ol.steps>li::before{left:12px;width:30px;height:30px}
  .cards{grid-template-columns:minmax(0,1fr)}
}
"""

PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark">
<meta name="description" content="{description}">
<title>{title}</title>
<link rel="stylesheet" href="{up}site.css">{head}
</head>
<body>
<a class="skip" href="#main">Skip to the page</a>
{banner}<div class="wrap">
<nav aria-label="Site"><a class="brand" href="{up}index.html">{brand}</a><div class="links">{menu}</div></nav>
<main id="main">
{body}
</main>
<footer><p>OpenDMD is an independent, open-source project. It is not made by or affiliated with
<a href="https://dmdos.net/">DMDoS</a>: the display, its board (by <a href="https://mortaca.com">MORTACA</a>),
the build guide and the GIF pack all come from dmdos.net. The animations on this site are recordings of
OpenDMD's own effects and the CC0 <a href="https://kenney.nl/assets/pixel-platformer">Kenney</a> sprites.
Code under <a href="{up}LICENSE-MIT.txt">MIT</a> or <a href="{up}LICENSE-APACHE.txt">Apache 2.0</a>.</p></footer>
</div>
</body>
</html>
"""
# Until the public launch: a line across the top of every page. Set to "" to drop it.
BANNER = ("<strong>Preview.</strong> OpenDMD isn't publicly launched yet: what's here works, "
          "but it may change, and the source code isn't public yet.")
DESCRIPTION = "OpenDMD: open firmware and a web app for the DMDoS LED dot-matrix display."
LINK = re.compile(r'(\]\(|(?:src|href)=")([^)"\s]+)')


def rel(from_page: str, to: str) -> str:
    """A link from one published page to another published path."""
    return posixpath.relpath(to, posixpath.dirname(from_page) or ".") + ("/" if to.endswith("/") else "")


def rewrite(src: str, page: str, text: str, assets: set[str]) -> str:
    def fix(m: re.Match) -> str:
        lead, url = m.groups()
        if re.match(r"[a-z]+:|#|/", url):
            return m.group(0)
        path, _, frag = url.partition("#")
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), path))
        if target in PAGES:
            out = rel(page, PAGES[target])
        elif target in FILES:
            out = rel(page, FILES[target])
        elif (ROOT / target).is_file() and target.startswith("docs/images/"):
            assets.add(target)
            out = rel(page, target)
        else:
            return lead + "#unpublished:" + url  # unlinked below
        return lead + out + ("#" + frag if frag else "")

    # A link named after a file (`card.md`) is named after its page instead.
    names = {page: label for label, page in MENU if label}
    def title(m: re.Match) -> str:
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), m.group(2).partition("#")[0]))
        return f"[{names[PAGES[target]]}]({m.group(2)})" if PAGES.get(target) in names else m.group(0)
    text = re.sub(r"\[`([^`\]]+\.md)`\]\(([^)\s]+)\)", title, text)
    text = LINK.sub(fix, text)
    # Markdown links to unpublished files: keep the text, drop the link.
    return re.sub(r"\[([^\]]+)\]\(#unpublished:[^)]*\)", r"\1", text)


def menu(page: str) -> str:
    out = []
    for label, target in MENU:
        if label is None:
            out.append(f"<h4>{html.escape(target)}</h4>")
            continue
        cls = "demo" if target == "demo/" else "on" if target == page else ""
        out.append(f'<a href="{rel(page, target)}"{f" class={cls!r}" if cls else ""}>{html.escape(label)}</a>')
    return "".join(out)


LAZY = ' loading="lazy"'


def led(up: str, item: dict, alt: str | None = None, lazy: bool = False, size: tuple[int, int] = (128, 32)) -> str:
    """One animation drawn as LEDs: the GIF, or its still for reduced motion."""
    gif, still = html.escape(up + "art/" + item["gif"]), html.escape(up + "art/" + item["still"])
    alt = html.escape(item["alt"] if alt is None else alt)
    w, h = size
    return (f'<picture class="led"><source srcset="{still}" media="(prefers-reduced-motion: reduce)">'
            f'<img src="{gif}" data-still="{still}" alt="{alt}" width="{w}" height="{h}" decoding="async"'
            f'{LAZY if lazy else ""}></picture>')


def tiles(art: dict) -> str:
    figs = "".join(f'<figure>{led("", i, lazy=True)}<figcaption>{html.escape(i["caption"])}</figcaption></figure>'
                   for i in art["items"] if "tiles" in i["where"])
    return (f'<div class="tiles">{figs}</div>\n<p class="tiles-note">Recorded from the simulator: the panel\'s own '
            f'effects and live items, and the CC0 Kenney Pixel Platformer sprites. Bring your own GIFs, or make them '
            f'with Create Art.</p>')


def hero(body: str, art: dict) -> str:
    """The landing page's top: the README's GitHub header (its title,
    badge, hero picture and links, up to <!-- site:hero -->) becomes the
    wordmark, the tagline and the buttons."""
    m = re.search(r'<p align="center">(.*?)</p>', body, re.S)
    tag = m.group(1).strip() if m else ""
    wm = art["wordmark"]
    word = (f'<picture class="led" style="--cols:{wm["w"]};--rows:{wm["h"]}"><img src="art/{wm["src"]}" '
            f'alt="OpenDMD" width="{wm["w"]}" height="{wm["h"]}"></picture>')
    top = (f'<section class="hero">\n<h1><span class="plate">{word}</span></h1>\n'
           f'<a class="chip" href="guide/hardware.html">For the DMDoS display</a>\n'
           f'<p class="tag">{tag}</p>\n'
           f'<p><a class="cta" href="demo/">Try the demo in your browser →</a>'
           f'<a class="cta alt" href="install/">Install on your display</a>'
           f'<a class="cta alt" href="guide/hardware.html">What you need</a></p>\n</section>')
    return top + body[body.index("<!-- site:hero -->") + len("<!-- site:hero -->"):]


def decorate(body: str) -> str:
    """The docs' site markers: feature cards, numbered steps, part cards."""
    def features(m: re.Match) -> str:
        # The card's title is its bold lead-in, without the colon.
        return '<ul class="features">' + re.sub(r"<li><strong>(.*?):</strong>", r"<li><strong>\1</strong>", m.group(1)) + "</ul>"
    body = re.sub(r"<!-- site:features -->\s*<ul>(.*?)</ul>", features, body, flags=re.S)
    body = re.sub(r"<!-- site:steps -->\s*<ol>", '<ol class="steps">', body)

    def cards(m: re.Match) -> str:
        parts = re.split(r"(?=<h3)", m.group(1).strip())
        out = []
        for part in parts:
            if not part.strip():
                continue
            part = re.sub(r"(<h3[^>]*>)(\d+) · ", r'\1<span class="num" aria-hidden="true">\2</span>', part)
            part = part.replace("<p><strong>Where:</strong>", '<p class="where"><strong>Where:</strong>')
            out.append(f'<div class="card">{part}</div>')
        return '<div class="cards">' + "".join(out) + "</div>"
    return re.sub(r"<!-- site:cards -->(.*?)<!-- site:cards-end -->", cards, body, flags=re.S)


def release(dist: Path, out: Path) -> dict:
    """The current release in dist/ (fw-release keeps older versions there
    too): the parts its manifest names, the merged image and the signed
    bundle of that version only, under install/firmware/; and
    install/release.json for the web app's update check."""
    manifest = dist / "manifest.json"
    if not manifest.is_file():
        raise SystemExit(f"no {manifest}: run make fw-release first")
    m = json.loads(manifest.read_text())
    ver = m["version"]
    fw = out / "install/firmware"
    fw.mkdir(parents=True)
    names = [p["path"] for b in m["builds"] for p in b["parts"]]
    names += [f"opendmd-{ver}-4mb-full.bin", f"opendmd-{ver}.odfw"]
    for name in ["manifest.json", *names]:
        if not (dist / name).is_file():
            raise SystemExit(f"no {dist / name}: run make fw-release")
        shutil.copyfile(dist / name, fw / name)
    # The www pack's version: the header's string (docs/www-pack.md), the
    # one GET /api/www reports.
    pack = (fw / f"opendmd-{ver}-www.pack").read_bytes()[:64]
    if pack[:4] != b"ODWP":
        raise SystemExit(f"opendmd-{ver}-www.pack is not a www pack")
    www_version = pack[20:20 + pack[18]].decode()
    info = {"version": ver, "firmware": f"firmware/opendmd-{ver}.odfw",
            "www": f"firmware/opendmd-{ver}-www.pack", "www_version": www_version}
    (out / "install/release.json").write_text(json.dumps(info, indent=2) + "\n")
    return info


def markers(page: str, text: str, rel_info: dict) -> str:
    """Site-only pieces, written in the docs as HTML comments (invisible on
    GitHub): the install button and the update bundle's download link."""
    ver = html.escape(rel_info["version"])
    bundle = rel(page, "install/" + rel_info["firmware"])
    text = text.replace("<!-- site:install-button -->", INSTALL_BUTTON.format(version=ver))
    return text.replace("<!-- site:firmware-bundle -->",
                        f'<p><a href="{bundle}" download>Download the firmware update file</a> '
                        f'(OpenDMD {ver}, <code>opendmd-{ver}.odfw</code>)</p>')


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--www", type=Path, required=True)
    ap.add_argument("--card", type=Path, required=True)
    ap.add_argument("--art", type=Path, required=True, help="the artwork from tools/site-art.py")
    ap.add_argument("--dist", type=Path, required=True, help="the release from make fw-release")
    a = ap.parse_args()
    out: Path = a.out
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    # The demo: the app (the player sends the .gz copies; Pages compresses by itself) and its card.
    shutil.copytree(a.www, out / "demo", ignore=shutil.ignore_patterns("*.gz", ".gitignore"))
    shutil.copytree(a.card, out / "demo/demo")
    info = release(a.dist, out)
    art = json.loads((a.art / "art.json").read_text())
    (out / "art/strip").mkdir(parents=True)
    for i in art["items"]:
        for k in ("gif", "still"):
            shutil.copyfile(a.art / i[k], out / "art" / i[k])
    shutil.copyfile(a.art / art["wordmark"]["src"], out / "art" / art["wordmark"]["src"])

    assets: set[str] = set()
    bodies: list[str] = []
    for src, page in PAGES.items():
        text = markers(page, rewrite(src, page, (ROOT / src).read_text(), assets), info)
        md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
                               extension_configs={"toc": {"permalink": "#"}})
        body = md.convert(text)
        m = re.search(r"<h1[^>]*>(.*?)</h1>", body, re.S)
        title = re.sub(r"<[^>]+>", "", m.group(1)).strip() if m else "OpenDMD"
        title = "OpenDMD" if title == "OpenDMD" else f"{title} · OpenDMD"
        if page == "index.html":
            body = hero(body, art).replace("<!-- site:tiles -->", tiles(art))
        body = decorate(body)
        bodies.append(body)
        up = "../" * page.count("/")
        dest = out / page
        dest.parent.mkdir(parents=True, exist_ok=True)
        head = "\n" + INSTALL_HEAD if page == INSTALL_PAGE else ""
        wm = art["wordmark"]
        brand = (f'<picture class="led" style="--cols:{wm["w"]};--rows:{wm["h"]}"><img src="{up}art/{wm["src"]}" '
                 f'alt="OpenDMD" width="{wm["w"]}" height="{wm["h"]}"></picture>')
        dest.write_text(PAGE.format(title=html.escape(title), description=html.escape(DESCRIPTION), up=up, head=head,
                                    brand=brand, menu=menu(page), body=body,
                                    banner=f'<div class="banner" role="note">{BANNER}</div>\n' if BANNER else ""))
    # Pictures only the GitHub copy of a doc shows (the README's hero) stay off the site.
    assets = {a for a in assets if any(posixpath.basename(a) in b for b in bodies)}
    for target in sorted(assets):
        (out / target).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / target, out / target)
    for src, dst in FILES.items():
        shutil.copyfile(ROOT / src, out / dst)
    (out / "site.css").write_text(CSS.strip() + "\n")
    (out / ".nojekyll").write_text("")
    for src, dst in MOVED.items():
        to = html.escape(rel(src, dst))
        (out / src).parent.mkdir(parents=True, exist_ok=True)
        (out / src).write_text(f'<!doctype html><meta charset="utf-8"><title>Moved</title>'
                               f'<meta http-equiv="refresh" content="0; url={to}"><link rel="canonical" href="{to}">'
                               f'<p>This page moved: <a href="{to}">{to}</a></p>\n')
    (out / "README.md").write_text("# opendmd.github.io\n\nThe OpenDMD site: documentation and a browser demo of the web app "
                                   "(https://opendmd.github.io/). Built by `make site` in the OpenDMD repository; "
                                   "changes made here are overwritten by the next publish.\n")
    n = sum(1 for p in out.rglob("*") if p.is_file())
    size = sum(p.stat().st_size for p in out.rglob("*") if p.is_file())
    print(f"{out}: {n} files, {size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
