# /// script
# requires-python = ">=3.11"
# dependencies = ["markdown>=3.6"]
# ///
"""Build the public site (https://opendmd.dev/) as static files.

    uv run tools/site.py OUT_DIR --www opendmd-web/www --card target/demo-card \
        --art target/site-art --dist dist

- /            the home page (site/pages/home.html: what OpenDMD is, the
               panel and its art, and the way to Install, the guide and the
               Formula gallery; the panel plays a bundled Formula live,
               home.js); the README stays GitHub's front page
- /install/    the browser installer (docs/guide/install.md with ESP Web
               Tools' button for the DevKit; the 8 MB build's (8 MB) is
               hidden for now, see INSTALL_BOARDS) and, under
               install/firmware/, the release in dist/: the parts
               dist/manifest-4mb.json names, the merged image, the signed
               update bundles (every layout's: release.json points at them),
               plus install/release.json (what the web app's update check
               reads)
- /guide/      the user guide (docs/guide/), with its own index
               (site/pages/guide.md) and a side menu
- /dev/        the developer pages: an overview and a few clean references
               (the detailed notes in docs/ stay in the repo), among them
               /dev/psi/, the Psi language reference (docs/psi-spec.md:
               its tables and error codes generated from the compiler's;
               live examples, search and a cheat sheet by
               opendmd-web/share/ref/psiref.js, on the demo's Psi)
- /demo/       the web app with the demo card (tools/demo-card.py) beside it:
               with no player answering, it loads demo/index.json and plays
               that card in the browser (js/source.js, demoCard)
- /f/          a Formula's page, the web app's own editor on the demo's files
               (opendmd-web/share/f/; js/formulas.js `editorPage`): a gallery
               Formula by name (`/f/#gallery=<slug>`, the gallery's tiles) or
               one the Psi reference opened (`/f/#open`); never its text in
               the link
- /formulas/   the formula gallery: OpenDMD's examples (opendmd-web/formulas/
               *.psi, CC0, copied to formulas/src/; the ten marked
               `-- bundle` are also the web app's formulas.json) and the contributions merged into the public
               gallery repo's gallery/ (`--gallery`, fetched by release.yml),
               each checked again by opendmd-gallery (`--gallery-tool`)
               before it is published; thumbnails drawn by it at build time.
               The page's body and thumbnails are tools/gallery_page.py's;
               a merged contribution appears at the next site build
- /art-index/v1/<PLATFORM>.json
               the name index of Create Art's "Find art for a game"
               (opendmd-web/art-index/, tools/art-index.py): not in the www
               pack; the web app on a player fetches it from here (_headers
               sends `Access-Control-Allow-Origin: *`; Cloudflare compresses
               JSON), the demo same-origin. `v1` is the format
               (ART_INDEX_FORMATS: each format its own source folder and
               validator, every one published, so a new format cannot drop
               an old one): each
               file is checked to be exactly its format's shape
- /art/        the landing page's tiles and the wordmark (tools/site-art.py: recordings of the
               OpenDMD's own Formulas and live items, and CC0 sprites), drawn
               as LEDs by CSS
- /dev/api/    the HTTP API as each release described it: v<version>.html, a
               copy of dev/api.html frozen when the site is built from that
               release's tag (`--api-freeze`), beside the earlier releases'
               copies from the public gallery repo's dev/api/ (`--api-archive`,
               fetched by release.yml; gallery-repo-publish.sh archives each
               release's there and never removes one).
               dev/api.html (the latest) links them all; the web app's
               Settings -> About links its firmware's
- /licenses/   the licences: a page (LICENCES below, linked from every
               page's footer) and the third-party notices from licenses/
               (tools/notices.py), the same files the release and the
               player carry
- /404.html    the page for an address with nothing there (Cloudflare's
               not_found_handling serves it)
- _headers, _redirects, .assetsignore
               for the opendmd-site Worker on Cloudflare (site/worker/, which
               serves this tree as its static assets): headers per path (CORS,
               caching, CSP), MOVED as 301s, what not to upload

Every page has the same chrome, from site/theme/: page.html around it,
header.html (the wordmark and the top bar: Guide, Formulas, Demo, For
developers and Install; a Menu button on a phone), footer.html, and
site.css (light and dark follow the system). A page in a SECTION (the guide,
the developers' pages) also gets that section's menu: down the side on a
wide screen, a folded list above the page on a phone. /f/ gets the same
header and footer (f_page), around its own f.css.

The docs mark site-only pieces with HTML comments, invisible on GitHub:
`site:features`, `site:steps` and `site:cards` ... `site:cards-end` style
the list or the h3 sections after them as cards or numbered steps.

Links between published pages are rewritten to the HTML pages; a link to
anything not published (CLAUDE.md, source files) keeps its text and loses
the link. `make site` runs this; `make site-deploy` deploys it.
"""

import argparse
import functools
import html
import json
import posixpath
import re
import shutil
import subprocess
from pathlib import Path

import markdown

import gallery_page  # tools/gallery_page.py, beside this file

ROOT = Path(__file__).resolve().parent.parent

# Source (repo path) -> published path, in the order of the docs menu.
PAGES = {
    "site/pages/home.html": "index.html",
    "site/pages/guide.md": "guide/index.html",
    "docs/guide/hardware.md": "guide/hardware.html",
    "docs/guide/install.md": "install/index.html",
    "docs/guide/web-app.md": "guide/web-app.html",
    "docs/guide/create-art.md": "guide/create-art.html",
    "docs/guide/formulas.md": "guide/formulas.html",
    "docs/guide/card.md": "guide/card.html",
    "docs/guide/updating.md": "guide/updating.html",
    "docs/guide/troubleshooting.md": "guide/troubleshooting.html",
    "docs/developers.md": "dev/index.html",
    "docs/api.md": "dev/api.html",
    "docs/psi-spec.md": "dev/psi/index.html",
    "docs/simulator.md": "dev/simulator.html",
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
    # Retired pages (the built-in effects became Formulas). These were PAGES
    # targets once; they are not any more, so the redirect pages written for
    # MOVED are the only files at these paths.
    "guide/effects.html": "guide/formulas.html",
    "dev/effects.html": "guide/formulas.html",
    # Addresses people guess: the gallery by the public repo's name, the
    # installer under the guide, the docs' old folder.
    "gallery.html": "formulas/index.html",
    "guide/install.html": "install/index.html",
    "docs/index.html": "guide/index.html",
}
# Other names a published page goes by in the docs' links.
ALIASES = {"README.md": "index.html"}
# The top bar: label, target, the section it marks. The home page is the
# wordmark; Install, last, is drawn as a button.
TOP = [
    ("Guide", "guide/index.html", "guide"),
    ("Formulas", "formulas/index.html", "formulas"),
    ("Demo", "demo/", "demo"),
    ("For developers", "dev/index.html", "dev"),
    ("Install", "install/index.html", "install"),
]
# The side menus: section -> (its heading, entries). None starts a group (a
# little space). Pages outside them are reached from the top bar, the footer
# or another page.
SECTIONS = {
    "guide": ("The guide", [
        ("Start here", "guide/index.html"),
        None,
        ("What you need", "guide/hardware.html"),
        ("Install", "install/index.html"),
        None,
        ("The web app", "guide/web-app.html"),
        ("Create Art", "guide/create-art.html"),
        ("Formulas", "guide/formulas.html"),
        ("The SD card", "guide/card.html"),
        None,
        ("Updating", "guide/updating.html"),
        ("Troubleshooting", "guide/troubleshooting.html"),
    ]),
    "dev": ("For developers", [
        ("Building from source", "dev/index.html"),
        ("HTTP API", "dev/api.html"),
        ("Psi language", "dev/psi/index.html"),
        ("The simulator", "dev/simulator.html"),
        ("Hardware pinout", "dev/pinout.html"),
        None,
        ("Licences", "licenses/index.html"),
    ]),
}
# What a link named after a file (`card.md`) is called instead.
TITLES = {e[1]: e[0] for _, entries in SECTIONS.values() for e in entries if e}


def section(page: str) -> tuple[str | None, str | None]:
    """(the top bar entry it marks, the side menu it shows) for a published page."""
    first = page.split("/")[0]
    if first == "install":
        return "install", "guide"
    if first in ("guide", "dev"):
        return first, first
    if first == "licenses":
        return None, "dev"
    if first in ("formulas", "f"):
        return "formulas", None
    return None, None
# The installer page: ESP Web Tools, pinned (the flashing itself is
# esptool-js over Web Serial; Chrome, Edge, Opera or Firefox 151+, https or localhost).
INSTALL_PAGE = "install/index.html"
# The Psi language reference: its live examples, highlighting, search and
# cheat sheet (opendmd-web/share/ref/psiref.js, copied beside the page; it
# imports the demo's Psi, so its CSP allows wasm: CSP["psi"]).
PSI_PAGE = "dev/psi/index.html"
PSI_HEAD = '<script type="module" src="psiref.js"></script>'
# The home page's panel plays a bundled Formula live once the page has
# loaded (opendmd-web/share/ref/home.js, on psiled.js, the reference's
# drawing; both copied to the root): the demo's Psi again, so CSP["home"].
HOME_HEAD = '<script type="module" src="home.js"></script>'
ROOT_SCRIPTS = ("psiled.js", "home.js")  # from opendmd-web/share/ref/
# The gallery's tiles play on hover or focus (ftiles.js, beside the page,
# on psiled.js): CSP["home"] there too. ftsort.js sorts and filters them.
GALLERY_HEAD = '<script type="module" src="ftiles.js"></script>\n<script type="module" src="ftsort.js"></script>'
INSTALL_HEAD = ('<script type="module" src="https://cdn.jsdelivr.net/npm/esp-web-tools@10.4.0/dist/web/'
                'install-button.js?module"></script>')
# Every flash layout (the firmware's `variant`): the board, the words on its
# button, and what it says about the board.
BOARDS = {
    "4mb": ("ESP32 DevKit V1", "Install",
            "The ESP32 DevKit V1: the small board with a USB socket that plugs into the DMDoS board. Most displays have this one."),
    "8mb": ("8 MB build", "Install the 8 MB build",
            "For an 8 MB board in development. Its firmware has more room."),
}
# The layouts the install page offers (a button, a manifest and its parts on
# the site). The 8 MB build's "8mb" is built and released but hidden here
# for now: add "8mb" back to bring its button back (CLAUDE.md,
# "Bringing the 8 MB option back"; flasher/index.html has the same switch).
INSTALL_BOARDS = ("4mb",)
INSTALL_ONE = """<div class="board" id="board-{v}">
<h3>{board}</h3>
<p>{about}</p>
<esp-web-install-button id="install-{v}" manifest="firmware/manifest-{v}.json">
<button slot="activate" id="install-activate-{v}">{label}</button>
<span slot="unsupported">This browser can't talk to USB boards. Open this page in Chrome, Edge, Opera or Firefox on a computer.</span>
<span slot="not-allowed">This page has to be opened over https.</span>
</esp-web-install-button>
</div>"""
INSTALL_BUTTON = ('<div class="install"><div class="boards">'
                  + "".join(INSTALL_ONE.format(v=v, board=b, label=lab, about=about) for v, (b, lab, about) in ((v, BOARDS[v]) for v in INSTALL_BOARDS))
                  + '</div><p class="ver">OpenDMD {version}.</p></div>')
# Other files published as they are: repo path -> published path.
FILES = {
    "LICENSE-MIT": "LICENSE-MIT.txt",
    "LICENSE-APACHE": "LICENSE-APACHE.txt",
    # The headings' pixel font (tools/font-subset.py), self-hosted, and its licence.
    "opendmd-web/www/fonts/silkscreen.woff2": "fonts/silkscreen.woff2",
    "opendmd-web/www/fonts/OFL.txt": "fonts/OFL.txt",
    # The third-party notices (tools/notices.py, `make notices`).
    "licenses/THIRD-PARTY-LICENSES-firmware.txt": "licenses/THIRD-PARTY-LICENSES-firmware.txt",
    "licenses/THIRD-PARTY-LICENSES-web.txt": "licenses/THIRD-PARTY-LICENSES-web.txt",
}
# The licences page (licenses/index.html), linked from every page's footer.
LICENCES_PAGE = "licenses/index.html"
# Create Art's game name index (tools/art-index.py): published at
# art-index/v<N>/<PLATFORM>.json for every format N in ART_INDEX_FORMATS.
# The web app's find.js asks for the one it reads (`ART_INDEX`). A new shape
# is a new entry here (its own source folder and validator) and a new
# `ART_INDEX` in find.js; the older entries stay, so the web apps already on
# players keep their format.
ART_INDEX_PLATFORMS = ("MD", "SNES", "GB", "GBC", "GBA", "SATURN", "NDS", "NES", "SMS", "PCE", "PS1", "N64", "ARCADE")


def art_index_v1(d) -> bool:
    """Format 1: {"repo", "names", "kinds"}, one hex digit of kinds per name."""
    return (isinstance(d, dict) and set(d) == {"repo", "names", "kinds"} and isinstance(d["repo"], str)
            and isinstance(d["names"], list) and all(isinstance(n, str) for n in d["names"])
            and isinstance(d["kinds"], str) and len(d["kinds"]) == len(d["names"])
            and re.fullmatch(r"[0-9a-f]*", d["kinds"]) is not None)


# format -> (source folder, validator)
ART_INDEX_FORMATS = {
    1: (ROOT / "opendmd-web/art-index", art_index_v1),
}
LICENCES = """# Licences

OpenDMD is open source: [MIT](../LICENSE-MIT.txt) or [Apache 2.0](../LICENSE-APACHE.txt), at your option.

It is built on other open-source software, used under the licences listed in these notices,
with each licence's full text:

- **[The firmware](THIRD-PARTY-LICENSES-firmware.txt)**: what the installer writes to the
  board (the firmware, its bootloader and the updater): Espressif's ESP-IDF (Apache 2.0, with
  FreeRTOS, lwIP, Mbed TLS, wpa_supplicant, FatFs and the C library), the HUB75 display
  driver, and the Rust crates.
- **[The web app](THIRD-PARTY-LICENSES-web.txt)**: the Rust crates in its WebAssembly, and the
  headings' font, Silkscreen ([SIL Open Font License 1.1](../fonts/OFL.txt)).
- **The installer** uses [ESP Web Tools](https://github.com/esphome/esp-web-tools) (Apache 2.0),
  which your browser loads from jsDelivr.

The player carries the firmware's and the web app's notices too: in the web app, **Settings ›
Device › About › Open-source licences**.

**Find art for a game** searches a list of game names taken from the file names of the
[libretro-thumbnails](https://github.com/libretro-thumbnails) collections (published here
as `art-index/`); the pictures come from those collections when you search, and are never
stored on this site.

The demo's and this site's animations are OpenDMD's own (its Formulas, recorded or playing
live in your browser, and its start-up show) and the
[Kenney Pixel Platformer](https://kenney.nl/assets/pixel-platformer) sprites and tiles (CC0).
"""

# The pages' chrome and styles (see the docstring).
THEME = ROOT / "site/theme"
# Until the public launch: a line across the top of every page. Set to "" to drop it.
BANNER = ("<strong>Preview.</strong> OpenDMD isn't publicly launched yet: what's here works, "
          "but it may change, and the source code isn't public yet.")
DESCRIPTION = "OpenDMD: open firmware and a web app for the DMDoS LED dot-matrix display."
# The HTTP API's frozen copies (see the docstring): dev/api/v<version>.html.
API_PAGE = "docs/api.md"
API_DIR = "dev/api"
API_FROZEN = re.compile(r"v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)\.html")
LINK = re.compile(r'(\]\(|(?:src|href)=")([^)"\s]+)')

# --- Cloudflare (site/worker/, the opendmd-site Worker's static assets) ---
#
# _headers (Cloudflare's format): every rule a path matches applies, in file
# order; a header set by two rules would be joined with a comma, so a later
# rule first drops it (`! Name`) and sets its own. One CSP per path. One
# block per pattern: a second block for the same pattern replaces the first
# (measured with wrangler dev), so headers_file merges them. No
# COOP/COEP: no wasm here uses threads, and COEP would block ESP Web Tools
# from jsDelivr and Create Art's picture fetches.
CSP_BASE = ("default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; "
            "object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'")
CSP = {
    # Every page by default (the docs, the licences, the 404 page) and the
    # home page: the home page's live Formulas run the demo's Psi, which
    # needs 'wasm-unsafe-eval'. It is in the default policy because a rule
    # for "/" alone does not detach the "/*" header on Cloudflare (both were
    # sent and the stricter won); it only lets same-origin scripts compile
    # wasm.
    "docs": CSP_BASE.replace("script-src 'self'", "script-src 'self' 'wasm-unsafe-eval'"),
    # The installer: ESP Web Tools (and the modules it imports) from jsDelivr.
    "install": CSP_BASE.replace("script-src 'self'", "script-src 'self' https://cdn.jsdelivr.net")
    + "; connect-src 'self' https://cdn.jsdelivr.net",
    # A Formula's page: the demo's wasm and the editor's GIF picker (blob:);
    # Open on my panel navigates to or opens the panel's page, never fetches.
    "f": CSP_BASE.replace("img-src 'self' data:", "img-src 'self' data: blob:")
    .replace("script-src 'self'", "script-src 'self' 'wasm-unsafe-eval'"),
    # The Psi language reference: its live examples run the demo's Psi
    # (pkg-psi, same-origin), which needs only 'wasm-unsafe-eval' on top of
    # the docs' policy: no blob:, no connect-src beyond 'self' (the wasm is
    # fetched from the site itself), no inline script.
    "psi": CSP_BASE.replace("script-src 'self'", "script-src 'self' 'wasm-unsafe-eval'"),
    # The home page and the gallery: their live Formulas run the demo's Psi,
    # as the reference's examples do (the same policy).
    "home": CSP_BASE.replace("script-src 'self'", "script-src 'self' 'wasm-unsafe-eval'"),
    # The demo: the web app's wasm, Create Art's sources (js/find.js) and the
    # update check (util.js SITE).
    "demo": CSP_BASE.replace("img-src 'self' data:", "img-src 'self' data: blob: https://blob.gifcities.org "
                             "https://raw.githubusercontent.com; media-src 'self' blob:")
    .replace("script-src 'self'", "script-src 'self' 'wasm-unsafe-eval'")
    + "; connect-src 'self' https://gifcities.archive.org https://blob.gifcities.org https://raw.githubusercontent.com "
    "https://opendmd.dev",
}
CACHE_DEFAULT = "public, max-age=600"   # what GitHub Pages sent (the art index's ETag behaviour)
CACHE_SHORT = "public, max-age=60"      # install/release.json, beta.json: the update check
CACHE_IMMUTABLE = "public, max-age=31536000, immutable"  # version-named release files
CORS_PATHS = ("/install/*", "/art-index/*", "/formulas/src/*")  # fetched from a player's http:// web app


def headers_file() -> str:
    """target/site/_headers."""
    rules: list[tuple[str, list[str]]] = [
        ("/*", ["X-Content-Type-Options: nosniff", "Referrer-Policy: strict-origin-when-cross-origin",
                "Permissions-Policy: serial=(self), usb=(self), camera=(), microphone=(), geolocation=(), payment=()",
                f"Cache-Control: {CACHE_DEFAULT}", f"Content-Security-Policy: {CSP['docs']}"]),
    ]
    for p in CORS_PATHS:
        rules.append((p, ["Access-Control-Allow-Origin: *"]))
    for path, which in (("/install/*", "install"), ("/f/*", "f"), ("/demo/*", "demo"), ("/dev/psi/*", "psi"), ("/formulas/*", "home")):
        rules.append((path, ["! Content-Security-Policy", f"Content-Security-Policy: {CSP[which]}"]))
    for name in ("release.json", "beta.json"):
        rules.append((f"/install/{name}", ["! Cache-Control", f"Cache-Control: {CACHE_SHORT}"]))
    # opendmd-<version>-…: named by the release (a published tag is never
    # rebuilt). The manifests, bootloader, table, otadata and updater keep
    # their names from release to release: the default.
    rules.append(("/install/firmware/opendmd-*", ["! Cache-Control", f"Cache-Control: {CACHE_IMMUTABLE}"]))
    # One block per path: a second block for the same path replaces the first.
    merged: dict[str, list[str]] = {}
    for path, hs in rules:
        merged.setdefault(path, []).extend(hs)
    return "".join(f"{path}\n" + "".join(f"  {h}\n" for h in hs) + "\n" for path, hs in merged.items())


def served(page: str) -> str:
    """How Cloudflare's assets serve a published page (html_handling
    auto-trailing-slash): x/index.html at /x/, x.html at /x."""
    if page == "index.html" or page.endswith("/index.html"):
        return "/" + page[:-len("index.html")]
    return "/" + page.removesuffix(".html")


def redirects_file() -> str:
    """target/site/_redirects: MOVED as real 301s (the meta-refresh
    pages stay for a plain static server), each old address with and
    without .html, straight to where its page is served (one hop). www.opendmd.dev is the Worker's
    (site/worker/src/index.ts): _redirects cannot match a host."""
    lines = []
    for src, dst in MOVED.items():
        for old in dict.fromkeys(("/" + src, "/" + src.removesuffix(".html"))):
            lines.append(f"{old} {served(dst)} 301")
    return "\n".join(lines) + "\n"


# Files in the tree not uploaded as assets.
ASSETS_IGNORE = (".assetsignore",)
# The 404 page (Cloudflare's not_found_handling "404-page"): shown at any
# depth, so its links are absolute.
NOT_FOUND_PAGE = "404.html"
NOT_FOUND = """# Not found

There's nothing at this address. It may have moved: start from the [home page](/), the
[guide](/guide/) or the [Formula gallery](/formulas/).
"""


def rel(from_page: str, to: str) -> str:
    """A link from one published page to another published path."""
    return posixpath.relpath(to, posixpath.dirname(from_page) or ".") + ("/" if to.endswith("/") else "")


def href(page: str, to: str) -> str:
    """A link from `page` to a published page, a folder's index as the folder
    (where Cloudflare serves it, and a plain static server too)."""
    return rel(page, to.removesuffix("index.html") or "./") if to.endswith("index.html") else rel(page, to)


def rewrite(src: str, page: str, text: str, assets: set[str]) -> str:
    def fix(m: re.Match) -> str:
        lead, url = m.groups()
        if re.match(r"[a-z]+:|#|/", url):
            return m.group(0)
        path, _, frag = url.partition("#")
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), path))
        target = next((s for s, d in PAGES.items() if d == ALIASES[target]), target) if target in ALIASES else target
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
    def title(m: re.Match) -> str:
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), m.group(2).partition("#")[0]))
        return f"[{TITLES[PAGES[target]]}]({m.group(2)})" if PAGES.get(target) in TITLES else m.group(0)
    text = re.sub(r"\[`([^`\]]+\.md)`\]\(([^)\s]+)\)", title, text)
    text = LINK.sub(fix, text)
    # Markdown links to unpublished files: keep the text, drop the link.
    return re.sub(r"\[([^\]]+)\]\(#unpublished:[^)]*\)", r"\1", text)


def _a(page: str, label: str, target: str, on: bool, cls: str = "") -> str:
    c = " ".join(x for x in (cls, "on" if on else "") if x)
    return (f'<a href="{href(page, target)}"{f" class={c!r}" if c else ""}{" aria-current=page" if on else ""}>'
            f'{html.escape(label)}</a>')


def top_links(page: str) -> str:
    """The top bar's links, the page's section marked."""
    mark, _ = section(page)
    return "".join(_a(page, label, t, sec == mark, "go" if sec == "install" else "") for label, t, sec in TOP)


def side_menu(page: str, on: str | None = None) -> tuple[str, str]:
    """The page's section menu, `on` (default: the page itself) marked: (the
    side one for wide screens, the folded one for phones); empty outside a section."""
    _, sec = section(on or page)
    if sec is None:
        return "", ""
    on = on or page
    head, entries = SECTIONS[sec]
    links, gap, here = [], False, ""
    for e in entries:
        if e is None:
            gap = True
            continue
        label, t = e
        here = label if t == on else here
        links.append(_a(page, label, t, t == on, "gap" if gap else ""))
        gap = False
    links_html = "".join(links)
    side = f'<nav class="side" aria-label="{html.escape(head)}"><p class="side-h">{html.escape(head)}</p>{links_html}</nav>'
    fold = (f'<details class="subnav"><summary>{html.escape(head)}{": <b>" + html.escape(here) + "</b>" if here else ""}</summary>'
            f'<nav aria-label="{html.escape(head)} (menu)">{links_html}</nav></details>\n')
    return side, fold


@functools.cache
def theme(name: str) -> str:
    return (THEME / name).read_text(encoding="utf-8")


def wordmark(up: str, wm: dict) -> str:
    return (f'<picture class="led" style="--cols:{wm["w"]};--rows:{wm["h"]}"><img src="{up}art/{wm["src"]}" '
            f'alt="OpenDMD" width="{wm["w"]}" height="{wm["h"]}"></picture>')


def header(page: str, wm: dict) -> str:
    """The banner, the wordmark and the top bar (site/theme/header.html)."""
    up = "../" * page.count("/")
    return theme("header.html").strip().format(
        banner=f'<div class="banner" role="note">{BANNER}</div>\n' if BANNER else "",
        home="/" if page == NOT_FOUND_PAGE else up or "./", wordmark=wordmark(up, wm), links=top_links(page))


def footer(page: str) -> str:
    return theme("footer.html").strip().format(up="../" * page.count("/"))


ABSOLUTE = functools.partial(re.sub, r'((?:href|src|srcset)=")(?![a-z]+:|/|#)', r"\1/")


def render(page: str, title: str, body: str, wm: dict, description: str = DESCRIPTION, head: str = "",
           on: str | None = None, kind: str = "doc") -> str:
    """A whole page: the chrome (site/theme/) around `body`. `on`: the menu
    entry to mark, when not the page itself."""
    up = "../" * page.count("/")
    side, fold = side_menu(page, on)
    doc = theme("page.html").format(
        title=html.escape(title), description=html.escape(description), up=up, head=head, kind=kind,
        header=header(page, wm), wrap="wrap sided" if side else "wrap", side=side, subnav=fold, body=body,
        footer=footer(page))
    # The 404 page answers at any depth: its links are absolute.
    return ABSOLUTE(doc) if page == NOT_FOUND_PAGE else doc


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
    return (f'<div class="tiles">{figs}</div>\n<p class="tiles-note">Recorded from OpenDMD: its own Formulas, the clock '
            f'and scrolling text, and the CC0 Kenney Pixel Platformer sprites. Your own GIFs play the same way, '
            f'and Create Art makes new ones.</p>')


# The home page (site/pages/home.html).
HOME = "site/pages/home.html"
HOME_IMAGES = re.compile(r'src="(docs/images/[\w.-]+)"')
HOME_FORMULA = re.compile(r'data-formula="formulas/src/([\w.-]+\.psi)"')


def home(art: dict, assets: set[str]) -> str:
    """The home page's body: its {{led:NAME}} (`{{led:NAME:}}`: no alt
    text) and {{tiles}} filled in, its comment dropped, its pictures noted."""
    items = {i["name"]: i for i in art["items"]}
    text = re.sub(r"<!--.*?-->\s*", "", (ROOT / HOME).read_text(encoding="utf-8"), count=1, flags=re.S)

    def fill(m: re.Match) -> str:
        what, name, alt = m.group(1), m.group(2), m.group(3)
        if what == "tiles":
            return tiles(art)
        if name not in items:
            raise SystemExit(f"{HOME}: no animation {name!r} in the site's art")
        return led("", items[name], alt=None if alt is None else "")
    text = re.sub(r"\{\{(tiles|led):?([\w-]*)(:)?\}\}", fill, text)
    # The live panel's Formula: an example, bundled (the caption says it
    # comes with OpenDMD).
    for m in HOME_FORMULA.finditer(text):
        psi = ROOT / "opendmd-web/formulas" / m.group(1)
        if not psi.is_file() or "\n-- bundle" not in "\n" + psi.read_text(encoding="utf-8"):
            raise SystemExit(f"{HOME}: data-formula {m.group(1)} is not a bundled Formula in opendmd-web/formulas/")
    for m in HOME_IMAGES.finditer(text):
        if not (ROOT / m.group(1)).is_file():
            raise SystemExit(f"{HOME}: no {m.group(1)}")
        assets.add(m.group(1))
    return text


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
    too), both flash layouts: the parts the offered layouts' manifests name, the merged
    images and the signed bundles of that version only, under
    install/firmware/; and install/release.json for the web app's update
    check: `firmware` (the 4mb bundle: what web apps from before the 8 MB
    layout read) and `variants`, each layout's bundle. (Only INSTALL_BOARDS
    get a manifest, parts and image on the site; all get their bundle.)"""
    ms = {}
    for v in BOARDS:
        manifest = dist / f"manifest-{v}.json"
        if not manifest.is_file():
            raise SystemExit(f"no {manifest}: run make fw-release first")
        ms[v] = json.loads(manifest.read_text())
    ver = ms["4mb"]["version"]
    if any(m["version"] != ver for m in ms.values()):
        raise SystemExit(f"the manifests are for different versions: {[m['version'] for m in ms.values()]}")
    fw = out / "install/firmware"
    fw.mkdir(parents=True)
    # Only the layouts the install page offers get their manifest, parts and
    # merged image on the site; every layout's signed bundle is published,
    # because release.json's `variants` point at them (the web app's update
    # check). The 8mb files otherwise stay in the GitHub release.
    shown = [v for v in ms if v in INSTALL_BOARDS]
    names = [f"manifest-{v}.json" for v in shown]
    names += sorted({p["path"] for v in shown for b in ms[v]["builds"] for p in b["parts"]})
    names += [f"opendmd-{ver}-{v}-full.bin" for v in shown]
    names += [f"opendmd-{ver}-{v}.odfw" for v in ms]
    for name in names:
        if not (dist / name).is_file():
            raise SystemExit(f"no {dist / name}: run make fw-release")
        shutil.copyfile(dist / name, fw / name)
    # The name installers before the 8 MB layout fetched: an installer tab
    # left open across a release still finds a manifest (the DevKit's; its
    # parts are named relative to it, so they are the ones published here).
    shutil.copyfile(dist / "manifest-4mb.json", fw / "manifest.json")
    # The www pack's version: the header's string (docs/www-pack.md), the
    # one GET /api/www reports.
    pack = (fw / f"opendmd-{ver}-www.pack").read_bytes()[:64]
    if pack[:4] != b"ODWP":
        raise SystemExit(f"opendmd-{ver}-www.pack is not a www pack")
    www_version = pack[20:20 + pack[18]].decode()
    info = {"version": ver, "firmware": f"firmware/opendmd-{ver}-4mb.odfw",
            "variants": {v: {"firmware": f"firmware/opendmd-{ver}-{v}.odfw"} for v in ms},
            "www": f"firmware/opendmd-{ver}-www.pack", "www_version": www_version}
    (out / "install/release.json").write_text(json.dumps(info, indent=2) + "\n")
    return info


def markers(page: str, text: str, rel_info: dict) -> str:
    """Site-only pieces, written in the docs as HTML comments (invisible on
    GitHub): the install button, the update bundle's download link."""
    ver = html.escape(rel_info["version"])
    text = text.replace("<!-- site:install-button -->", INSTALL_BUTTON.replace("{version}", ver))
    links = []
    for v in INSTALL_BOARDS:
        board = BOARDS[v][0]
        f = rel_info.get("variants", {}).get(v, {}).get("firmware")
        if f:
            links.append(f'<li><a href="{rel(page, "install/" + f)}" download>{board}</a>: '
                         f'<code>{html.escape(posixpath.basename(f))}</code></li>')
    return text.replace("<!-- site:firmware-bundle -->",
                        f"<p>The firmware update file for OpenDMD {ver}, one per board:</p><ul>{''.join(links)}</ul>")


def semver_key(v: str) -> tuple:
    """Sort key for a version: 1.0.0-beta.2 < 1.0.0-beta.10 < 1.0.0 < 1.0.1."""
    core, _, pre = v.partition("-")
    nums = tuple(int(x) for x in core.split("."))
    ids = tuple((0, int(x), "") if x.isdigit() else (1, 0, x) for x in pre.split(".")) if pre else ()
    return nums, not pre, ids


def describe() -> str:
    """This tree's `git describe`, as tools/gallery-repo-publish.sh reads it."""
    try:
        return subprocess.run(["git", "describe", "--always", "--dirty", "--tags", "--match", "v*"], cwd=ROOT, check=True,
                              capture_output=True, text=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def api_archive(archive: Path | None) -> dict[str, Path]:
    """The earlier releases' frozen API pages (the public repo's dev/api/):
    version -> file. Plain files named v<version>.html only."""
    found = {}
    for p in sorted(archive.iterdir()) if archive and archive.is_dir() else ():
        m = API_FROZEN.fullmatch(p.name)
        if m and p.is_file() and not p.is_symlink():
            found[m.group(1)] = p
    return found


def api_versions_line(page: str, versions: list[str]) -> str:
    """dev/api.html's line linking each release's frozen copy, newest first."""
    if not versions:
        return ""
    links = " · ".join(f'<a href="{rel(page, f"{API_DIR}/v{v}.html")}">{html.escape(v)}</a>' for v in versions)
    return f'<p class="vers">The latest docs. The API of a particular release: {links}</p>\n'


def api_frozen_line(page: str, version: str) -> str:
    """A frozen copy's line: which release it describes, and the latest."""
    return (f'<p class="vers">This page describes the HTTP API of <strong>OpenDMD {html.escape(version)}</strong>. '
            f'<a href="{rel(page, PAGES[API_PAGE])}">The latest version</a></p>\n')


def after_h1(body: str, extra: str) -> str:
    """`extra` right after the page's heading."""
    i = body.find("</h1>")
    return body if i < 0 or not extra else body[:i + 5] + "\n" + extra + body[i + 5:]


# The formula gallery (formulas/index.html): its body and thumbnails come
# from tools/gallery_page.py.
GALLERY_PAGE = gallery_page.PAGE


# OpenDMD's examples: every one in the gallery (the bundled ones too).
EXAMPLES = ROOT / "opendmd-web/formulas"


def write_gallery(out: Path, examples: Path, contrib: Path | None, tool: Path, wm: dict) -> None:
    """formulas/index.html: the site's page around the gallery's markers,
    and formulas/src/: the examples' files (`examples`), which
    gallery_page.py's command line reads them from."""
    up = "../" * GALLERY_PAGE.count("/")
    page_dir = (out / GALLERY_PAGE).parent
    page_dir.mkdir(parents=True, exist_ok=True)
    if examples.resolve() != (page_dir / "src").resolve():
        shutil.rmtree(page_dir / "src", ignore_errors=True)
        (page_dir / "src").mkdir()
        for p in sorted(examples.glob("*.psi")):
            shutil.copyfile(p, page_dir / "src" / p.name)
    body = gallery_page.build(page_dir, page_dir / "src", contrib, tool, gallery_page.load_added(examples))
    for js in ("ftiles.js", "ftsort.js"):
        shutil.copyfile(ROOT / "opendmd-web/share/ref" / js, page_dir / js)
    (out / GALLERY_PAGE).write_text(render(
        GALLERY_PAGE, "Formula gallery · OpenDMD", gallery_page.marked(body), wm, kind="gallery", head="\n" + GALLERY_HEAD,
        description="Formulas for the OpenDMD LED display: change one here, then open it on your panel."), encoding="utf-8")


# A Formula's page (opendmd-web/share/f/index.html): the site's header and
# footer around it, the site's styles under its own f.css.
F_PAGE = "f/index.html"


def f_page(dest: Path, wm: dict) -> None:
    text = dest.read_text(encoding="utf-8")
    subs = [
        (r'<meta name="color-scheme" content="dark">', '<meta name="color-scheme" content="light dark">'),
        (r'(<link rel="stylesheet" href="f\.css">)', '<link rel="stylesheet" href="../site.css">\n\\1'),
        (r'<body>\s*<header class="top">.*?</header>\s*', lambda m: '<body class="fpage">\n' + header(F_PAGE, wm) + '\n<div class="wrap">\n'),
        (r'</main>\s*<footer>.*?</footer>', lambda m: '</main>\n</div>\n' + footer(F_PAGE)),
    ]
    for pat, to in subs:
        text, n = re.subn(pat, to, text, count=1, flags=re.S)
        if n != 1:
            raise SystemExit(f"opendmd-web/share/f/index.html: no {pat!r} to replace (the page changed: change f_page too)")
    dest.write_text(text, encoding="utf-8")


def write_doc(out: Path, src: str, page: str, info: dict, art: dict, assets: set[str], extra: str = "",
              on: str | None = None, title_note: str = "") -> str:
    """One doc (`src`) as the published `page`, with `extra` after its
    heading; returns its body."""
    text = markers(page, rewrite(src, page, (ROOT / src).read_text(encoding="utf-8"), assets), info)
    md = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
                           extension_configs={"toc": {"permalink": "#"}})
    body = md.convert(text)
    m = re.search(r"<h1[^>]*>(.*?)</h1>", body, re.S)
    # Without the heading's permalink, whose text is "#".
    title = re.sub(r'<a class="headerlink".*?</a>|<[^>]+>', "", m.group(1)).strip() if m else "OpenDMD"
    title = "OpenDMD" if title == "OpenDMD" else f"{title}{title_note} · OpenDMD"
    if src == HOME:
        body, title = home(art, assets), "OpenDMD: open firmware and a web app for the DMDoS LED display"
    body = after_h1(decorate(body), extra)
    dest = out / page
    dest.parent.mkdir(parents=True, exist_ok=True)
    head = "\n" + {INSTALL_PAGE: INSTALL_HEAD, PSI_PAGE: PSI_HEAD}.get(page, HOME_HEAD if src == HOME else "")
    dest.write_text(render(page, title, body, art["wordmark"], head=head, on=on,
                           kind="home" if src == HOME else "doc"), encoding="utf-8")
    return body


def api_pages(out: Path, info: dict, art: dict, archive: Path | None, freeze: str, assets: set[str]) -> list[str]:
    """dev/api/: the earlier releases' frozen API pages, copied as they are,
    and, when `freeze` says so, this release's, rendered now (replacing an
    earlier publish of the same version). `freeze` "auto": only when this
    tree is the release's tag, clean (`git describe --match 'v*'` is exactly
    v<version>, the test gallery-repo-publish.sh makes); a site built from main
    between releases freezes nothing. Returns the versions, newest first."""
    ver = info["version"]
    frozen = api_archive(archive)
    if frozen:
        (out / API_DIR).mkdir(parents=True, exist_ok=True)
    for v, p in frozen.items():
        shutil.copyfile(p, out / API_DIR / p.name)
    if freeze == "yes" or (freeze == "auto" and describe() == f"v{ver}"):
        page = f"{API_DIR}/v{ver}.html"
        write_doc(out, API_PAGE, page, info, art, assets, api_frozen_line(page, ver), on=PAGES[API_PAGE],
                  title_note=f" ({ver})")
        frozen[ver] = out / page
    return sorted(frozen, key=semver_key, reverse=True)


def write_art_index(out: Path) -> None:
    """Create Art's game name index: art-index/v<N>/<PLATFORM>.json for every
    format in ART_INDEX_FORMATS, copied byte for byte (compact JSON;
    Cloudflare compresses it on the way), each file checked by its format's validator."""
    for fmt, (srcdir, valid) in ART_INDEX_FORMATS.items():
        dst = out / f"art-index/v{fmt}"
        dst.mkdir(parents=True)
        have = sorted(p.stem for p in srcdir.glob("*.json"))
        if have != sorted(ART_INDEX_PLATFORMS):
            raise SystemExit(f"{srcdir}: expected {sorted(ART_INDEX_PLATFORMS)}, found {have}")
        for plat in ART_INDEX_PLATFORMS:
            src = srcdir / f"{plat}.json"
            if not valid(json.loads(src.read_text(encoding="utf-8"))):
                raise SystemExit(f"{src}: not art index format {fmt} (a new shape is a new format: "
                                 "an entry in ART_INDEX_FORMATS and ART_INDEX in find.js)")
            shutil.copyfile(src, dst / src.name)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("out", type=Path)
    ap.add_argument("--www", type=Path)
    ap.add_argument("--card", type=Path)
    ap.add_argument("--art", type=Path, required=True, help="the artwork from tools/site-art.py")
    ap.add_argument("--dist", type=Path, required=True, help="the release from make fw-release")
    ap.add_argument("--gallery-tool", type=Path, help="opendmd-gallery, built (make gallery-tool)")
    ap.add_argument("--gallery", type=Path, help="the public gallery repo's gallery/ (contributions), if fetched")
    ap.add_argument("--api-archive", type=Path, help="the public gallery repo's dev/api/ (earlier releases' API pages), if fetched")
    ap.add_argument("--api-freeze", choices=("auto", "yes", "no"), default="auto",
                    help="freeze this release's API page as dev/api/v<version>.html (auto: when built from its tag)")
    ap.add_argument("--api-only", action="store_true",
                    help="write only dev/api.html and dev/api/, into OUT as it is (tools/site-smoke.py's publish-like check)")
    a = ap.parse_args()
    if not a.api_only and not (a.www and a.card and a.gallery_tool):
        ap.error("--www, --card and --gallery-tool are required (unless --api-only)")
    out: Path = a.out
    if a.api_only:  # into a site already built, or a new folder: only dev/api.html and dev/api/ are replaced
        shutil.rmtree(out / API_DIR, ignore_errors=True)
        out.mkdir(parents=True, exist_ok=True)
    else:
        if out.exists():
            shutil.rmtree(out)
        out.mkdir(parents=True)
    if a.api_only:
        m = json.loads((a.dist / "manifest-4mb.json").read_text())
        info = {"version": m["version"], "firmware": f"firmware/opendmd-{m['version']}-4mb.odfw"}
        art = json.loads((a.art / "art.json").read_text())
        assets: set[str] = set()
        versions = api_pages(out, info, art, a.api_archive, a.api_freeze, assets)
        write_doc(out, API_PAGE, PAGES[API_PAGE], info, art, assets, api_versions_line(PAGES[API_PAGE], versions))
        print(f"{out}: the API pages, frozen: {', '.join(versions) or 'none'}")
        return

    # The demo: the app (the player sends the .gz copies; Cloudflare compresses by itself) and its card.
    shutil.copytree(a.www, out / "demo", ignore=shutil.ignore_patterns("*.gz", ".gitignore"))
    shutil.copytree(a.card, out / "demo/demo")
    # A formula's own page; it runs on the demo's files.
    shutil.copytree(ROOT / "opendmd-web/share/f", out / "f")
    art = json.loads((a.art / "art.json").read_text())
    f_page(out / F_PAGE, art["wordmark"])
    # The Psi reference's script (the page itself is a doc, written below).
    (out / PSI_PAGE).parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(ROOT / "opendmd-web/share/ref/psiref.js", out / posixpath.dirname(PSI_PAGE) / "psiref.js")
    for name in ROOT_SCRIPTS:
        shutil.copyfile(ROOT / "opendmd-web/share/ref" / name, out / name)
    info = release(a.dist, out)
    write_art_index(out)
    (out / "art/strip").mkdir(parents=True)
    for i in art["items"]:
        for k in ("gif", "still"):
            shutil.copyfile(a.art / i[k], out / "art" / i[k])
    shutil.copyfile(a.art / art["wordmark"]["src"], out / "art" / art["wordmark"]["src"])

    assets: set[str] = set()
    bodies: list[str] = []
    versions = api_pages(out, info, art, a.api_archive, a.api_freeze, assets)
    for src, page in PAGES.items():
        extra = api_versions_line(page, versions) if src == API_PAGE else ""
        bodies.append(write_doc(out, src, page, info, art, assets, extra))
    # The formula gallery: generated, in the menu.
    write_gallery(out, EXAMPLES, a.gallery, a.gallery_tool, art["wordmark"])
    # The licences page: not a doc; the footer and the developers' menu link it.
    body = markdown.Markdown(extensions=["sane_lists"]).convert(LICENCES)
    (out / LICENCES_PAGE).parent.mkdir(parents=True, exist_ok=True)
    (out / LICENCES_PAGE).write_text(render(LICENCES_PAGE, "Licences · OpenDMD", body, art["wordmark"]), encoding="utf-8")
    # Pictures only the GitHub copy of a doc shows (the README's hero) stay off the site.
    assets = {a for a in assets if any(posixpath.basename(a) in b for b in bodies)}
    for target in sorted(assets):
        (out / target).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / target, out / target)
    for src, dst in FILES.items():
        (out / dst).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / src, out / dst)
    # The 404 page: links absolute (it answers at any depth), nothing marked.
    (out / NOT_FOUND_PAGE).write_text(render(NOT_FOUND_PAGE, "Not found · OpenDMD", markdown.Markdown().convert(NOT_FOUND),
                                             art["wordmark"]), encoding="utf-8")
    (out / "_headers").write_text(headers_file())
    (out / "_redirects").write_text(redirects_file())
    (out / ".assetsignore").write_text("".join(f"{n}\n" for n in ASSETS_IGNORE))
    shutil.copyfile(THEME / "site.css", out / "site.css")
    for src, dst in MOVED.items():
        to = html.escape(rel(src, dst))
        (out / src).parent.mkdir(parents=True, exist_ok=True)
        (out / src).write_text(f'<!doctype html><meta charset="utf-8"><title>Moved</title>'
                               f'<meta http-equiv="refresh" content="0; url={to}"><link rel="canonical" href="{to}">'
                               f'<p>This page moved: <a href="{to}">{to}</a></p>\n')
    n = sum(1 for p in out.rglob("*") if p.is_file())
    size = sum(p.stat().st_size for p in out.rglob("*") if p.is_file())
    print(f"{out}: {n} files, {size / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
